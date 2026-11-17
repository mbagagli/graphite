use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::sync::Arc;

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use crate::agent::context::{assemble_context, select_evidence, COMPACT_EVIDENCE_BUDGET_TOKENS};
use crate::agent::model::{AgentEvidence, AgentIntent};
use crate::agent::provider::EvidenceTokenCounter;
use crate::database::lexical_query;
use crate::error::{AppError, AppResult};
use crate::model::SearchPage;
use crate::semantic::retrieval_policy::{hybrid_fusion_score, semantic_passage_score};
use crate::semantic::vector_index::{UsearchVectorIndex, VectorIndex};
use crate::semantic::EmbeddingProvider;

use super::model::{
    CharacterizationQuery, ExperimentMode, ExperimentPassage, ExperimentRetrievalKind,
    ExperimentSearchPage, ExperimentSearchResult, SourceKind,
};
use super::runtime::ExperimentRuntime;
use super::store::ExperimentStore;

const MAX_RETRIEVAL_CANDIDATES: usize = 180;
const MAX_RESULT_COUNT: usize = 50;

pub(super) struct ExperimentRetriever {
    provider: Arc<dyn EmbeddingProvider>,
}

#[derive(Clone, Debug)]
struct ChunkTarget {
    experiment_chunk_id: String,
    logical_chunk_id: String,
    item_id: String,
    document_id: String,
    document_revision_id: i64,
    text: String,
    character_start: u64,
    character_end: u64,
    page_start: Option<u32>,
    page_end: Option<u32>,
    content_fingerprint: String,
    vector_key: u64,
}

#[derive(Clone, Debug)]
struct PaperTarget {
    item_id: String,
    vector_key: u64,
}

#[derive(Clone, Debug)]
struct PassageCandidate {
    passage: ExperimentPassage,
}

#[derive(Clone, Debug)]
struct SemanticCandidate {
    item_id: String,
    score: f64,
    paper_score: Option<f64>,
    chunk_scores: Vec<f64>,
    passages: Vec<PassageCandidate>,
}

#[derive(Clone, Debug)]
struct LexicalCandidate {
    item_id: String,
    rank: u32,
    score: f64,
}

impl ExperimentRetriever {
    pub(super) fn new(provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self { provider }
    }

    pub(super) fn search(
        &self,
        runtime: &ExperimentRuntime,
        query: &CharacterizationQuery,
        retrieval_kind: ExperimentRetrievalKind,
        limit: usize,
    ) -> AppResult<ExperimentSearchPage> {
        let store = ExperimentStore::open_read_only(&runtime.database_path())?;
        if !store.ready()? {
            return Err(AppError::ApplicationData(
                "The experimental index is not ready for queries".to_owned(),
            ));
        }
        let profile_id = store.metadata_value("embedding-profile-id")?;
        let vectors: Arc<dyn VectorIndex> =
            Arc::new(UsearchVectorIndex::new(runtime.vectors_path())?);
        let scoped_ids = normalized_scope(store.connection(), &query.item_ids)?;
        let lexical = if matches!(
            retrieval_kind,
            ExperimentRetrievalKind::Lexical | ExperimentRetrievalKind::Hybrid
        ) {
            lexical_candidates(store.connection(), &query.query, &scoped_ids)?
        } else {
            Vec::new()
        };
        let semantic = if matches!(
            retrieval_kind,
            ExperimentRetrievalKind::Semantic | ExperimentRetrievalKind::Hybrid
        ) {
            semantic_candidates(
                store.connection(),
                vectors.as_ref(),
                self.provider.as_ref(),
                &profile_id,
                &query.query,
                &scoped_ids,
            )?
        } else {
            Vec::new()
        };
        let mut results =
            materialize_results(store.connection(), retrieval_kind, lexical, semantic)?;
        results.truncate(limit.clamp(1, MAX_RESULT_COUNT));
        let context_fingerprints = context_fingerprints(
            store.connection(),
            self.provider.clone(),
            &results,
            &scoped_ids,
        )?;
        Ok(ExperimentSearchPage {
            mode: ExperimentMode::ExperimentalZoteroMirror,
            retrieval_kind,
            query_id: query.id.clone(),
            results,
            context_fingerprints,
        })
    }
}

fn normalized_scope(connection: &Connection, requested: &[String]) -> AppResult<Vec<String>> {
    if requested.is_empty() {
        let mut statement =
            connection.prepare("SELECT item_id FROM experiment_items ORDER BY item_id")?;
        return statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into);
    }
    let scope = requested.to_vec();
    let unique = scope.iter().collect::<HashSet<_>>();
    if unique.len() != scope.len() {
        return Err(AppError::ApplicationData(
            "The experimental query scope contains duplicate items".to_owned(),
        ));
    }
    let available = all_item_ids(connection)?;
    if scope.iter().any(|item_id| !available.contains(item_id)) {
        return Err(AppError::ApplicationData(
            "The experimental query scope contains an item outside the snapshot".to_owned(),
        ));
    }
    Ok(scope)
}

fn all_item_ids(connection: &Connection) -> AppResult<HashSet<String>> {
    let mut statement = connection.prepare("SELECT item_id FROM experiment_items")?;
    let values = statement
        .query_map([], |row| row.get(0))?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(values)
}

fn lexical_candidates(
    connection: &Connection,
    query: &str,
    scoped_ids: &[String],
) -> AppResult<Vec<LexicalCandidate>> {
    let Some(fts_query) = lexical_query(query) else {
        return Ok(Vec::new());
    };
    let scope_json = serde_json::to_string(scoped_ids)?;
    let mut statement = connection.prepare(
        "SELECT sc.item_id,
                bm25(research_fts, 8.0, 6.0, 4.0, 3.0, 2.0, 1.0)
         FROM research_fts f
         JOIN search_content sc ON sc.rowid = f.rowid
         WHERE research_fts MATCH ?1
           AND sc.item_id IN (SELECT value FROM json_each(?2))
         ORDER BY bm25(research_fts, 8.0, 6.0, 4.0, 3.0, 2.0, 1.0),
                  sc.title COLLATE NOCASE
         LIMIT 50",
    )?;
    let rows = statement
        .query_map(params![fts_query, scope_json], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .enumerate()
        .map(|(index, (item_id, bm25))| LexicalCandidate {
            item_id,
            rank: index as u32 + 1,
            score: -bm25,
        })
        .collect())
}

fn semantic_candidates(
    connection: &Connection,
    vectors: &dyn VectorIndex,
    provider: &dyn EmbeddingProvider,
    profile_id: &str,
    query: &str,
    scoped_ids: &[String],
) -> AppResult<Vec<SemanticCandidate>> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let query_vector = provider.embed_query(query.trim())?;
    let allowed = scoped_ids.iter().collect::<HashSet<_>>();
    let chunk_targets = load_chunk_targets(connection)?;
    let chunk_keys = chunk_targets
        .iter()
        .filter(|(_, target)| allowed.contains(&target.item_id))
        .map(|(key, _)| *key)
        .collect::<HashSet<_>>();
    let paper_targets = load_paper_targets(connection)?;
    let paper_keys = paper_targets
        .iter()
        .filter(|(_, target)| allowed.contains(&target.item_id))
        .map(|(key, _)| *key)
        .collect::<HashSet<_>>();
    let chunk_matches = vectors.nearest(
        profile_id,
        "chunk",
        &query_vector,
        MAX_RETRIEVAL_CANDIDATES.min(chunk_keys.len()),
        &chunk_keys,
    )?;
    let paper_matches = vectors.nearest(
        profile_id,
        "paper",
        &query_vector,
        MAX_RETRIEVAL_CANDIDATES.min(paper_keys.len()),
        &paper_keys,
    )?;
    let mut grouped = HashMap::<String, SemanticCandidate>::new();
    for matched in chunk_matches {
        let Some(target) = chunk_targets.get(&matched.key) else {
            continue;
        };
        let provenance = load_provenance(connection, &target.experiment_chunk_id)?;
        let candidate = grouped
            .entry(target.item_id.clone())
            .or_insert_with(|| SemanticCandidate::new(target.item_id.clone()));
        candidate.chunk_scores.push(matched.similarity);
        candidate.passages.push(PassageCandidate {
            passage: ExperimentPassage {
                logical_chunk_id: target.logical_chunk_id.clone(),
                item_id: target.item_id.clone(),
                document_id: target.document_id.clone(),
                source_kinds: provenance.source_kinds,
                projection_ids: provenance.projection_ids,
                extraction_ids: provenance.extraction_ids,
                page_start: target.page_start,
                page_end: target.page_end,
                content_fingerprint: target.content_fingerprint.clone(),
                semantic_score: matched.similarity,
            },
        });
    }
    for matched in paper_matches {
        let Some(target) = paper_targets.get(&matched.key) else {
            continue;
        };
        let candidate = grouped
            .entry(target.item_id.clone())
            .or_insert_with(|| SemanticCandidate::new(target.item_id.clone()));
        candidate.paper_score = Some(matched.similarity);
    }
    let mut candidates = grouped.into_values().collect::<Vec<_>>();
    for candidate in &mut candidates {
        candidate.finish();
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    Ok(candidates)
}

impl SemanticCandidate {
    fn new(item_id: String) -> Self {
        Self {
            item_id,
            score: 0.0,
            paper_score: None,
            chunk_scores: Vec::new(),
            passages: Vec::new(),
        }
    }

    fn finish(&mut self) {
        self.chunk_scores
            .sort_by(|left, right| right.total_cmp(left));
        self.passages.sort_by(|left, right| {
            right
                .passage
                .semantic_score
                .total_cmp(&left.passage.semantic_score)
                .then_with(|| {
                    left.passage
                        .logical_chunk_id
                        .cmp(&right.passage.logical_chunk_id)
                })
        });
        self.passages.truncate(3);
        let passage_score = semantic_passage_score(&self.chunk_scores);
        self.score = match (passage_score, self.paper_score) {
            (Some(passage), Some(paper)) => passage.max((passage * 0.78) + (paper * 0.22)),
            (Some(passage), None) => passage,
            (None, Some(paper)) => paper,
            (None, None) => 0.0,
        };
    }
}

fn materialize_results(
    connection: &Connection,
    kind: ExperimentRetrievalKind,
    lexical: Vec<LexicalCandidate>,
    semantic: Vec<SemanticCandidate>,
) -> AppResult<Vec<ExperimentSearchResult>> {
    let lexical_by_id = lexical
        .into_iter()
        .map(|candidate| (candidate.item_id.clone(), candidate))
        .collect::<HashMap<_, _>>();
    let semantic_by_id = semantic
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.item_id.clone(), (index as u32 + 1, candidate)))
        .collect::<HashMap<_, _>>();
    let mut item_ids = match kind {
        ExperimentRetrievalKind::Lexical => lexical_by_id.keys().cloned().collect(),
        ExperimentRetrievalKind::Semantic => semantic_by_id.keys().cloned().collect(),
        ExperimentRetrievalKind::Hybrid => {
            let mut ids = lexical_by_id.keys().cloned().collect::<HashSet<_>>();
            ids.extend(semantic_by_id.keys().cloned());
            ids
        }
    };
    let mut results = Vec::with_capacity(item_ids.len());
    for item_id in item_ids.drain() {
        let title = connection.query_row(
            "SELECT title FROM experiment_items WHERE item_id = ?1",
            [&item_id],
            |row| row.get::<_, String>(0),
        )?;
        let lexical = lexical_by_id.get(&item_id);
        let semantic = semantic_by_id.get(&item_id);
        let lexical_rank = match kind {
            ExperimentRetrievalKind::Lexical => None,
            ExperimentRetrievalKind::Semantic | ExperimentRetrievalKind::Hybrid => {
                lexical.map(|candidate| candidate.rank)
            }
        };
        let semantic_rank = semantic.map(|(rank, _)| *rank);
        let fusion_score = match kind {
            ExperimentRetrievalKind::Lexical => {
                lexical.map(|candidate| candidate.score).unwrap_or_default()
            }
            ExperimentRetrievalKind::Semantic => semantic
                .map(|(_, candidate)| candidate.score)
                .unwrap_or_default(),
            ExperimentRetrievalKind::Hybrid => hybrid_fusion_score(lexical_rank, semantic_rank),
        };
        results.push(ExperimentSearchResult {
            item_id,
            title,
            lexical_rank,
            semantic_rank,
            fusion_score,
            passages: semantic
                .map(|(_, candidate)| {
                    candidate
                        .passages
                        .iter()
                        .map(|candidate| candidate.passage.clone())
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    results.sort_by(|left, right| result_order(kind, left, right));
    Ok(results)
}

fn result_order(
    kind: ExperimentRetrievalKind,
    left: &ExperimentSearchResult,
    right: &ExperimentSearchResult,
) -> Ordering {
    match kind {
        ExperimentRetrievalKind::Lexical => right
            .fusion_score
            .total_cmp(&left.fusion_score)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.item_id.cmp(&right.item_id)),
        ExperimentRetrievalKind::Semantic | ExperimentRetrievalKind::Hybrid => right
            .fusion_score
            .total_cmp(&left.fusion_score)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.item_id.cmp(&right.item_id)),
    }
}

struct ProvenanceSet {
    source_kinds: Vec<SourceKind>,
    projection_ids: Vec<String>,
    extraction_ids: Vec<String>,
}

fn load_provenance(connection: &Connection, chunk_id: &str) -> AppResult<ProvenanceSet> {
    let mut statement = connection.prepare(
        "SELECT source_kind, projection_id, extraction_id
         FROM experiment_chunk_provenance
         WHERE chunk_id = ?1 ORDER BY source_kind, projection_id",
    )?;
    let rows = statement
        .query_map([chunk_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut source_kinds = Vec::new();
    let mut projection_ids = Vec::new();
    let mut extraction_ids = Vec::new();
    for (source, projection, extraction) in rows {
        source_kinds.push(parse_source_kind(&source)?);
        projection_ids.push(projection);
        if let Some(extraction) = extraction {
            extraction_ids.push(extraction);
        }
    }
    source_kinds.sort();
    source_kinds.dedup();
    extraction_ids.sort();
    extraction_ids.dedup();
    Ok(ProvenanceSet {
        source_kinds,
        projection_ids,
        extraction_ids,
    })
}

fn parse_source_kind(value: &str) -> AppResult<SourceKind> {
    match value {
        "zotero" => Ok(SourceKind::Zotero),
        "xberg" => Ok(SourceKind::Xberg),
        "pdf-inspector-ocr" => Ok(SourceKind::PdfInspectorOcr),
        "docling" => Ok(SourceKind::Docling),
        _ => Err(AppError::ApplicationData(format!(
            "The experimental source kind is invalid: {value}"
        ))),
    }
}

fn load_chunk_targets(connection: &Connection) -> AppResult<HashMap<u64, ChunkTarget>> {
    let mut statement = connection.prepare(
        "SELECT vector_key, id, logical_chunk_id, item_id, document_id,
                document_revision_id, text, character_start, character_end,
                page_start, page_end, content_fingerprint
         FROM experiment_chunks",
    )?;
    let targets = statement
        .query_map([], |row| {
            Ok(ChunkTarget {
                vector_key: row.get::<_, i64>(0)? as u64,
                experiment_chunk_id: row.get(1)?,
                logical_chunk_id: row.get(2)?,
                item_id: row.get(3)?,
                document_id: row.get(4)?,
                document_revision_id: row.get(5)?,
                text: row.get(6)?,
                character_start: row.get::<_, i64>(7)? as u64,
                character_end: row.get::<_, i64>(8)? as u64,
                page_start: row.get(9)?,
                page_end: row.get(10)?,
                content_fingerprint: row.get(11)?,
            })
        })?
        .map(|row| row.map(|target| (target.vector_key, target)))
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(targets)
}

fn load_paper_targets(connection: &Connection) -> AppResult<HashMap<u64, PaperTarget>> {
    let mut statement =
        connection.prepare("SELECT vector_key, item_id FROM experiment_paper_representations")?;
    let targets = statement
        .query_map([], |row| {
            Ok(PaperTarget {
                vector_key: row.get::<_, i64>(0)? as u64,
                item_id: row.get(1)?,
            })
        })?
        .map(|row| row.map(|target| (target.vector_key, target)))
        .collect::<Result<HashMap<_, _>, _>>()?;
    Ok(targets)
}

fn context_fingerprints(
    connection: &Connection,
    provider: Arc<dyn EmbeddingProvider>,
    results: &[ExperimentSearchResult],
    scope: &[String],
) -> AppResult<Vec<String>> {
    let chunks = load_context_chunk_details(connection)?;
    let mut evidence = Vec::new();
    for result in results {
        for passage in &result.passages {
            let Some(detail) = chunks.get(&passage.logical_chunk_id) else {
                continue;
            };
            evidence.push(AgentEvidence {
                source_id: String::new(),
                kind: "document-passage".to_owned(),
                item_id: Some(result.item_id.clone()),
                item_title: result.title.clone(),
                document_id: Some(passage.document_id.clone()),
                document_revision_id: Some(detail.document_revision_id),
                chunk_id: Some(passage.logical_chunk_id.clone()),
                excerpt: Some(bounded_chars(&detail.text, 420)),
                character_start: Some(detail.character_start),
                character_end: Some(detail.character_end),
                page_start: passage.page_start,
                page_end: passage.page_end,
                retrieval_modes: vec!["hybrid".to_owned()],
                lexical_rank: result.lexical_rank,
                semantic_rank: result.semantic_rank,
                provider: format!(
                    "M4 Hybrid / {}",
                    connection.query_row(
                        "SELECT value FROM experiment_metadata
                         WHERE key = 'embedding-profile-id'",
                        [],
                        |row| row.get::<_, String>(0),
                    )?
                ),
                source_fingerprint: evidence_fingerprint(&[
                    &result.item_id,
                    &detail.document_revision_id.to_string(),
                    &detail.character_start.to_string(),
                    &bounded_chars(&detail.text, 420),
                ]),
                eligible_for_scientific_claim: true,
                stale: false,
            });
        }
    }
    let selection = select_evidence(evidence, &AgentIntent::LibrarySynthesis, scope, &[]);
    let counter = EmbeddingTokenCounter { provider };
    let assembly = assemble_context(selection, COMPACT_EVIDENCE_BUDGET_TOKENS, &counter)?;
    Ok(assembly
        .included_evidence
        .into_iter()
        .map(|source| source.source_fingerprint)
        .collect())
}

pub(super) fn production_context_fingerprints(
    page: &SearchPage,
    scope: &[String],
    provider: Arc<dyn EmbeddingProvider>,
) -> AppResult<Vec<String>> {
    let mut evidence = Vec::new();
    for result in &page.results {
        for passage in &result.passages {
            evidence.push(AgentEvidence {
                source_id: String::new(),
                kind: "document-passage".to_owned(),
                item_id: Some(result.item.id.clone()),
                item_title: result.item.title.clone(),
                document_id: Some(passage.document_id.clone()),
                document_revision_id: Some(passage.document_revision_id),
                chunk_id: Some(passage.chunk_id.clone()),
                excerpt: Some(passage.snippet.clone()),
                character_start: Some(passage.character_start),
                character_end: Some(passage.character_end),
                page_start: passage.page_start,
                page_end: passage.page_end,
                retrieval_modes: result.retrieval_modes.clone(),
                lexical_rank: result.ranking.lexical_rank,
                semantic_rank: result.ranking.semantic_rank,
                provider: format!("M4 Hybrid / {}", passage.embedding_profile_id),
                source_fingerprint: evidence_fingerprint(&[
                    &result.item.id,
                    &passage.document_revision_id.to_string(),
                    &passage.character_start.to_string(),
                    &passage.snippet,
                ]),
                eligible_for_scientific_claim: true,
                stale: false,
            });
        }
    }
    let selection = select_evidence(evidence, &AgentIntent::LibrarySynthesis, scope, &[]);
    let counter = EmbeddingTokenCounter { provider };
    let assembly = assemble_context(selection, COMPACT_EVIDENCE_BUDGET_TOKENS, &counter)?;
    Ok(assembly
        .included_evidence
        .into_iter()
        .map(|source| source.source_fingerprint)
        .collect())
}

fn load_context_chunk_details(connection: &Connection) -> AppResult<HashMap<String, ChunkTarget>> {
    Ok(load_chunk_targets(connection)?
        .into_values()
        .map(|target| (target.logical_chunk_id.clone(), target))
        .collect())
}

struct EmbeddingTokenCounter {
    provider: Arc<dyn EmbeddingProvider>,
}

impl EvidenceTokenCounter for EmbeddingTokenCounter {
    fn count_tokens(&self, text: &str) -> AppResult<u32> {
        Ok(self.provider.token_count(text)?.min(u32::MAX as usize) as u32)
    }

    fn token_accounting_method(&self) -> &'static str {
        "bge-tokenizer-equivalence-v1"
    }
}

fn evidence_fingerprint(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut output, "{byte:02x}").expect("write evidence fingerprint");
    }
    output
}

fn bounded_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_fingerprints_are_delimited_and_deterministic() {
        assert_eq!(
            evidence_fingerprint(&["ab", "c"]),
            evidence_fingerprint(&["ab", "c"])
        );
        assert_ne!(
            evidence_fingerprint(&["ab", "c"]),
            evidence_fingerprint(&["a", "bc"])
        );
    }
}
