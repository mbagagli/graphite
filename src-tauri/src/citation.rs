mod read_model;

pub use read_model::{
    cited_by_page, co_cited_works, connection_details, load_citation_diagnostics, missing_works,
    neighborhood, reference_page, relationship_evidence, shared_reference_papers,
};

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use tokio::sync::Mutex;

use crate::bibliography::{
    normalize_title, title_token_similarity, ConservativeTextExtractor, ExtractedReference,
    ExtractionInput, ReferenceExtractionProvider, EXTRACTOR_PROVIDER, EXTRACTOR_VERSION,
    RESOLVER_VERSION,
};
use crate::database::{content_hash, stable_id, ResearchDatabase};
use crate::error::{AppError, AppResult};
use crate::model::{ResolutionOverrideRequest, WorkspacePayload};

const DEFAULT_INDEX_BATCH: u32 = 32;
const MAX_INDEX_BATCH: u32 = 100;

#[derive(Clone)]
pub struct CitationCoordinator {
    database: ResearchDatabase,
    guard: Arc<Mutex<()>>,
}

impl CitationCoordinator {
    pub fn new(database: ResearchDatabase, guard: Arc<Mutex<()>>) -> Self {
        Self { database, guard }
    }

    pub async fn index(
        &self,
        priority_item_id: Option<&str>,
        max_documents: Option<u32>,
    ) -> AppResult<WorkspacePayload> {
        let _single_flight = self.guard.lock().await;
        let Some((source_id, _, library)) = self.database.active_library_context()? else {
            return self.database.load_workspace();
        };
        index_document_batch(
            &self.database,
            &source_id,
            &library.id,
            priority_item_id,
            max_documents
                .unwrap_or(DEFAULT_INDEX_BATCH)
                .clamp(1, MAX_INDEX_BATCH),
        )?;
        self.database.load_workspace()
    }

    pub async fn rebuild(&self) -> AppResult<WorkspacePayload> {
        let _single_flight = self.guard.lock().await;
        let Some((_, _, library)) = self.database.active_library_context()? else {
            return self.database.load_workspace();
        };
        rebuild_index(&self.database, &library.id)?;
        self.database.load_workspace()
    }

    pub async fn reresolve(&self) -> AppResult<WorkspacePayload> {
        let _single_flight = self.guard.lock().await;
        let Some((source_id, _, library)) = self.database.active_library_context()? else {
            return self.database.load_workspace();
        };
        reresolve_batch(&self.database, &source_id, &library.id, 5_000)?;
        self.database.load_workspace()
    }
}

#[derive(Clone)]
struct IndexDocument {
    document_id: String,
    source_id: String,
    library_id: String,
    source_item_id: String,
    text_provider: String,
    text_version: i64,
    text: String,
    completeness: String,
}

fn index_document_batch(
    database: &ResearchDatabase,
    source_id: &str,
    library_id: &str,
    priority_item_id: Option<&str>,
    max_documents: u32,
) -> AppResult<()> {
    let mut connection = database.connect()?;
    set_index_status(&connection, library_id, "indexing", None)?;
    let result = (|| {
        let documents =
            load_index_documents(&connection, library_id, priority_item_id, max_documents)?;
        let extractor = ConservativeTextExtractor;
        for document in documents {
            index_document(&mut connection, &extractor, &document)?;
        }
        reresolve_batch_with_connection(&mut connection, source_id, library_id, 2_000)
    })();
    if let Err(error) = result {
        refresh_index_state(&connection, library_id, Some(&error.to_string()))?;
        return Err(error);
    }
    refresh_index_state(&connection, library_id, None)
}

fn load_index_documents(
    connection: &Connection,
    library_id: &str,
    priority_item_id: Option<&str>,
    limit: u32,
) -> AppResult<Vec<IndexDocument>> {
    let mut statement = connection.prepare(
        "SELECT d.id, d.source_id, d.library_id, d.research_item_id,
                dt.provider, dt.source_version, dt.content, dt.completeness
         FROM documents d
         JOIN document_texts dt
           ON dt.document_id = d.id AND dt.is_active = 1
         LEFT JOIN reference_extractions e
           ON e.document_id = d.id AND e.is_current = 1
          AND e.extractor_version = ?1
         WHERE d.library_id = ?2 AND d.research_item_id IS NOT NULL
           AND e.id IS NULL
         ORDER BY CASE WHEN d.research_item_id = ?3 THEN 0 ELSE 1 END,
                  dt.fetched_at DESC, d.id
         LIMIT ?4",
    )?;
    let rows = statement.query_map(
        params![EXTRACTOR_VERSION, library_id, priority_item_id, limit],
        |row| {
            Ok(IndexDocument {
                document_id: row.get(0)?,
                source_id: row.get(1)?,
                library_id: row.get(2)?,
                source_item_id: row.get(3)?,
                text_provider: row.get(4)?,
                text_version: row.get(5)?,
                text: row.get(6)?,
                completeness: row.get(7)?,
            })
        },
    )?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn index_document(
    connection: &mut Connection,
    extractor: &dyn ReferenceExtractionProvider,
    document: &IndexDocument,
) -> AppResult<()> {
    let extraction = extractor.extract(ExtractionInput {
        text: &document.text,
        completeness: &document.completeness,
    });
    let text_hash = content_hash(&document.text);
    let transaction = connection.transaction()?;
    remove_current_document_evidence(&transaction, &document.document_id)?;
    let revision_id = ensure_text_revision(&transaction, document, &text_hash)?;
    let extraction_id = stable_id(
        "reference-extraction",
        &[
            &document.document_id,
            extractor.version(),
            &document.text_version.to_string(),
            &text_hash,
        ],
    );
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "INSERT INTO reference_extractions (
            id, source_id, library_id, source_item_id, document_id,
            document_revision_id, provider, extractor_version,
            source_text_provider, source_text_version, source_text_hash,
            source_completeness, status, section_heading, section_start,
            section_end, reference_count, diagnostic, is_current, indexed_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, 1, ?19
         )
         ON CONFLICT(id) DO UPDATE SET
            source_item_id = excluded.source_item_id,
            document_revision_id = excluded.document_revision_id,
            source_completeness = excluded.source_completeness,
            status = excluded.status,
            section_heading = excluded.section_heading,
            section_start = excluded.section_start,
            section_end = excluded.section_end,
            reference_count = excluded.reference_count,
            diagnostic = excluded.diagnostic,
            is_current = 1,
            indexed_at = excluded.indexed_at",
        params![
            extraction_id,
            document.source_id,
            document.library_id,
            document.source_item_id,
            document.document_id,
            revision_id,
            extractor.provider_id(),
            extractor.version(),
            document.text_provider,
            document.text_version,
            text_hash,
            document.completeness,
            extraction.status,
            extraction.section_heading,
            extraction.section_start.map(|value| value as i64),
            extraction.section_end.map(|value| value as i64),
            extraction.references.len() as i64,
            extraction.diagnostic,
            now,
        ],
    )?;
    transaction.execute(
        "UPDATE bibliographic_references SET is_current = 0
         WHERE extraction_id = ?1",
        [&extraction_id],
    )?;
    for reference in &extraction.references {
        store_reference(
            &transaction,
            document,
            revision_id,
            &extraction_id,
            reference,
            &extraction.status,
        )?;
    }
    transaction.execute(
        "UPDATE reference_resolution_overrides
         SET stale = 1, updated_at = ?2
         WHERE reference_id IN (
             SELECT id FROM bibliographic_references
             WHERE extraction_id = ?1 AND is_current = 0
         )",
        params![extraction_id, now],
    )?;
    cleanup_citation_edges(&transaction)?;
    transaction.commit()?;
    Ok(())
}

fn ensure_text_revision(
    transaction: &Transaction<'_>,
    document: &IndexDocument,
    text_hash: &str,
) -> AppResult<i64> {
    let revision_key = format!(
        "text:{}:{}:{}",
        document.text_provider,
        document.text_version,
        &text_hash[..16]
    );
    transaction.execute(
        "INSERT OR IGNORE INTO document_revisions (
            document_id, source_version, revision_key,
            file_size, modified_ms, discovered_at
         ) VALUES (?1, ?2, ?3, NULL, NULL, ?4)",
        params![
            document.document_id,
            document.text_version,
            revision_key,
            Utc::now().to_rfc3339(),
        ],
    )?;
    transaction
        .query_row(
            "SELECT id FROM document_revisions
             WHERE document_id = ?1 AND revision_key = ?2",
            params![document.document_id, revision_key],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn remove_current_document_evidence(
    transaction: &Transaction<'_>,
    document_id: &str,
) -> AppResult<()> {
    transaction.execute(
        "DELETE FROM citation_evidence
         WHERE reference_id IN (
             SELECT r.id FROM bibliographic_references r
             JOIN reference_extractions e ON e.id = r.extraction_id
             WHERE e.document_id = ?1 AND e.is_current = 1
         )",
        [document_id],
    )?;
    transaction.execute(
        "UPDATE reference_extractions
         SET is_current = 0, status = 'stale'
         WHERE document_id = ?1 AND is_current = 1",
        [document_id],
    )?;
    transaction.execute(
        "UPDATE bibliographic_references SET is_current = 0
         WHERE extraction_id IN (
             SELECT id FROM reference_extractions
             WHERE document_id = ?1 AND is_current = 0
         )",
        [document_id],
    )?;
    cleanup_citation_edges(transaction)
}

fn store_reference(
    transaction: &Transaction<'_>,
    document: &IndexDocument,
    revision_id: i64,
    extraction_id: &str,
    reference: &ExtractedReference,
    extraction_status: &str,
) -> AppResult<()> {
    let fingerprint = content_hash(&normalize_title(&reference.raw_text));
    let reference_id = stable_id(
        "reference",
        &[
            &document.document_id,
            &fingerprint,
            &reference.ordinal.to_string(),
        ],
    );
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "INSERT INTO bibliographic_references (
            id, extraction_id, source_id, library_id, source_item_id,
            source_document_id, source_document_revision_id, ordinal,
            raw_text, original_doi, normalized_doi, parsed_title,
            parsed_year, first_author, parsed_creators_json, parsed_venue,
            identifiers_json, parser_provider, parser_version,
            extraction_confidence, page_number, source_start, source_end,
            reference_fingerprint, is_current, created_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, NULL, ?21,
            ?22, ?23, 1, ?24
         )
         ON CONFLICT(id) DO UPDATE SET
            extraction_id = excluded.extraction_id,
            source_document_revision_id = excluded.source_document_revision_id,
            ordinal = excluded.ordinal,
            raw_text = excluded.raw_text,
            original_doi = excluded.original_doi,
            normalized_doi = excluded.normalized_doi,
            parsed_title = excluded.parsed_title,
            parsed_year = excluded.parsed_year,
            first_author = excluded.first_author,
            parsed_creators_json = excluded.parsed_creators_json,
            parsed_venue = excluded.parsed_venue,
            identifiers_json = excluded.identifiers_json,
            extraction_confidence = excluded.extraction_confidence,
            source_start = excluded.source_start,
            source_end = excluded.source_end,
            reference_fingerprint = excluded.reference_fingerprint,
            is_current = 1",
        params![
            reference_id,
            extraction_id,
            document.source_id,
            document.library_id,
            document.source_item_id,
            document.document_id,
            revision_id,
            reference.ordinal,
            reference.raw_text,
            reference.original_doi,
            reference.normalized_doi,
            reference.title,
            reference.year,
            reference.first_author,
            serde_json::to_string(&reference.creators)?,
            reference.venue,
            serde_json::to_string(&HashMap::<String, String>::new())?,
            EXTRACTOR_PROVIDER,
            EXTRACTOR_VERSION,
            reference.confidence,
            reference.start.map(|value| value as i64),
            reference.end.map(|value| value as i64),
            fingerprint,
            now,
        ],
    )?;
    transaction.execute(
        "DELETE FROM reference_identifiers WHERE reference_id = ?1",
        [&reference_id],
    )?;
    transaction.execute(
        "UPDATE reference_resolution_overrides
         SET stale = 0, updated_at = ?3
         WHERE reference_id = ?1 AND source_reference_fingerprint = ?2",
        params![reference_id, fingerprint, now],
    )?;
    if let (Some(original), Some(normalized)) = (&reference.original_doi, &reference.normalized_doi)
    {
        transaction.execute(
            "INSERT INTO reference_identifiers (
                reference_id, kind, original_value, normalized_value
             ) VALUES (?1, 'doi', ?2, ?3)",
            params![reference_id, original, normalized],
        )?;
    }
    let external_work_id = upsert_external_work(transaction, document, reference, &fingerprint)?;
    resolve_reference(
        transaction,
        &reference_id,
        &document.source_id,
        &document.source_item_id,
        reference,
        &external_work_id,
        extraction_status == "references-extracted",
    )
}

fn upsert_external_work(
    transaction: &Transaction<'_>,
    document: &IndexDocument,
    reference: &ExtractedReference,
    reference_fingerprint: &str,
) -> AppResult<String> {
    let (identity_kind, canonical_fingerprint) = if let Some(doi) = &reference.normalized_doi {
        ("doi", format!("doi:{doi}"))
    } else if let (Some(title), Some(year), Some(author)) =
        (&reference.title, reference.year, &reference.first_author)
    {
        (
            "strong-metadata",
            format!("metadata:{author}:{year}:{}", normalize_title(title)),
        )
    } else {
        (
            "source-reference",
            format!("reference:{reference_fingerprint}"),
        )
    };
    let work_id = stable_id(
        "external-work",
        &[&document.source_id, &canonical_fingerprint],
    );
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "INSERT INTO referenced_works (
            id, source_id, identity_kind, canonical_fingerprint,
            normalized_doi, title, publication_year, first_author,
            venue, created_at, last_seen_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
         ON CONFLICT(source_id, canonical_fingerprint) DO UPDATE SET
            normalized_doi = COALESCE(referenced_works.normalized_doi,
                                      excluded.normalized_doi),
            title = COALESCE(referenced_works.title, excluded.title),
            publication_year = COALESCE(referenced_works.publication_year,
                                        excluded.publication_year),
            first_author = COALESCE(referenced_works.first_author,
                                    excluded.first_author),
            venue = COALESCE(referenced_works.venue, excluded.venue),
            last_seen_at = excluded.last_seen_at",
        params![
            work_id,
            document.source_id,
            identity_kind,
            canonical_fingerprint,
            reference.normalized_doi,
            reference.title,
            reference.year,
            reference.first_author,
            reference.venue,
            now,
        ],
    )?;
    Ok(work_id)
}

#[derive(Clone)]
struct Candidate {
    item_id: String,
    title: String,
    score: f64,
    category: String,
    evidence: Vec<String>,
}

fn resolve_reference(
    transaction: &Transaction<'_>,
    reference_id: &str,
    source_id: &str,
    source_item_id: &str,
    reference: &ExtractedReference,
    external_work_id: &str,
    authoritative_extraction: bool,
) -> AppResult<()> {
    transaction.execute(
        "DELETE FROM reference_resolution_candidates WHERE reference_id = ?1",
        [reference_id],
    )?;
    let fingerprint = content_hash(&normalize_title(&reference.raw_text));
    let manual = transaction
        .query_row(
            "SELECT decision, target_item_id, source_reference_fingerprint,
                    stale
             FROM reference_resolution_overrides WHERE reference_id = ?1",
            [reference_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            },
        )
        .optional()?;
    if let Some((decision, target, override_fingerprint, stale)) = manual {
        if !stale && override_fingerprint == fingerprint {
            let matched = if decision == "accept" { target } else { None };
            let status = if matched.is_some() {
                "manual-resolved"
            } else if decision == "reject" {
                "external"
            } else {
                "unresolved"
            };
            let category = if matched.is_some() {
                "manual decision"
            } else if decision == "reject" {
                "external work"
            } else {
                "unresolved"
            };
            store_resolution(
                transaction,
                reference_id,
                status,
                matched.as_deref(),
                Some(external_work_id),
                &format!("manual-{decision}"),
                category,
                None,
                &["Graphite-owned researcher decision".to_owned()],
            )?;
            if let Some(target_id) = matched {
                if authoritative_extraction && target_id != source_item_id {
                    attach_citation_evidence(
                        transaction,
                        source_item_id,
                        &target_id,
                        reference_id,
                        "manual decision",
                    )?;
                }
            }
            return Ok(());
        }
    }

    let mut candidates = if let Some(doi) = &reference.normalized_doi {
        doi_candidates(transaction, source_id, doi)?
    } else {
        metadata_candidates(transaction, source_id, reference)?
    };
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.title.cmp(&right.title))
    });
    for (rank, candidate) in candidates.iter().enumerate() {
        transaction.execute(
            "INSERT INTO reference_resolution_candidates (
                reference_id, candidate_item_id, category, score,
                evidence_json, rank
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                reference_id,
                candidate.item_id,
                candidate.category,
                candidate.score,
                serde_json::to_string(&candidate.evidence)?,
                rank as i64 + 1,
            ],
        )?;
    }

    let exact = candidates
        .iter()
        .filter(|candidate| candidate.category == "exact-identifier")
        .collect::<Vec<_>>();
    let strong = candidates
        .iter()
        .filter(|candidate| candidate.category == "strong-metadata")
        .collect::<Vec<_>>();
    let probable = candidates
        .iter()
        .filter(|candidate| candidate.category == "probable")
        .collect::<Vec<_>>();
    let resolved = if exact.len() == 1 {
        Some((exact[0], "exact-identifier", "exact DOI match"))
    } else if exact.is_empty() && strong.len() == 1 {
        Some((strong[0], "strong-metadata", "strong metadata match"))
    } else {
        None
    };
    if let Some((candidate, status, category)) = resolved {
        store_resolution(
            transaction,
            reference_id,
            status,
            Some(&candidate.item_id),
            Some(external_work_id),
            status,
            category,
            Some(candidate.score),
            &candidate.evidence,
        )?;
        if authoritative_extraction && candidate.item_id != source_item_id {
            attach_citation_evidence(
                transaction,
                source_item_id,
                &candidate.item_id,
                reference_id,
                category,
            )?;
        }
    } else if exact.len() > 1 || strong.len() > 1 || probable.len() > 1 {
        store_resolution(
            transaction,
            reference_id,
            "ambiguous",
            None,
            Some(external_work_id),
            "multiple-local-candidates",
            "ambiguous",
            candidates.first().map(|candidate| candidate.score),
            &["Several local records plausibly identify this work".to_owned()],
        )?;
    } else if probable.len() == 1 {
        store_resolution(
            transaction,
            reference_id,
            "probable",
            None,
            Some(external_work_id),
            "probable-metadata",
            "probable",
            Some(probable[0].score),
            &probable[0].evidence,
        )?;
    } else {
        let external = reference.normalized_doi.is_some()
            || (reference.title.is_some()
                && reference.year.is_some()
                && reference.first_author.is_some());
        store_resolution(
            transaction,
            reference_id,
            if external { "external" } else { "unresolved" },
            None,
            Some(external_work_id),
            if external {
                "no-local-match"
            } else {
                "insufficient-bibliographic-metadata"
            },
            if external {
                "external work"
            } else {
                "unresolved"
            },
            None,
            &["No sufficiently trustworthy local identity match".to_owned()],
        )?;
    }
    Ok(())
}

fn doi_candidates(
    transaction: &Transaction<'_>,
    source_id: &str,
    doi: &str,
) -> AppResult<Vec<Candidate>> {
    let mut statement = transaction.prepare(
        "SELECT i.id, i.title
         FROM research_item_match_keys k
         JOIN research_items i ON i.id = k.item_id
         WHERE k.source_id = ?1 AND k.normalized_doi = ?2
           AND i.is_trashed = 0
         ORDER BY i.title, i.id
         LIMIT 25",
    )?;
    let rows = statement.query_map(params![source_id, doi], |row| {
        Ok(Candidate {
            item_id: row.get(0)?,
            title: row.get(1)?,
            score: 1.0,
            category: "exact-identifier".to_owned(),
            evidence: vec![format!("Normalized DOI exactly matches {doi}")],
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn metadata_candidates(
    transaction: &Transaction<'_>,
    source_id: &str,
    reference: &ExtractedReference,
) -> AppResult<Vec<Candidate>> {
    let (Some(year), Some(author)) = (reference.year, reference.first_author.as_deref()) else {
        return Ok(Vec::new());
    };
    let mut statement = transaction.prepare(
        "SELECT i.id, i.title, i.venue
         FROM research_item_match_keys k
         JOIN research_items i ON i.id = k.item_id
         WHERE k.source_id = ?1 AND k.publication_year = ?2
           AND k.first_author_surname = ?3 AND i.is_trashed = 0
         ORDER BY i.title, i.id
         LIMIT 100",
    )?;
    let rows = statement.query_map(params![source_id, year, author], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut candidates = Vec::new();
    for row in rows {
        let (item_id, title, venue) = row?;
        let title_score = title_token_similarity(&reference.raw_text, &title);
        let venue_compatible = reference
            .venue
            .as_deref()
            .zip(venue.as_deref())
            .is_some_and(|(reference_venue, candidate_venue)| {
                let left = normalize_title(reference_venue);
                let right = normalize_title(candidate_venue);
                !left.is_empty() && (left.contains(&right) || right.contains(&left))
            });
        let score = (title_score * 0.78) + 0.12 + 0.08 + f64::from(venue_compatible) * 0.02;
        let category = if title_score >= 0.78 {
            "strong-metadata"
        } else if title_score >= 0.55 {
            "probable"
        } else {
            continue;
        };
        let mut evidence = vec![
            "Publication year matches exactly".to_owned(),
            "First-author surname matches exactly".to_owned(),
            format!("Title token agreement {:.0}%", title_score * 100.0),
        ];
        if venue_compatible {
            evidence.push("Venue text is compatible".to_owned());
        }
        candidates.push(Candidate {
            item_id,
            title,
            score,
            category: category.to_owned(),
            evidence,
        });
    }
    Ok(candidates)
}

#[allow(clippy::too_many_arguments)]
fn store_resolution(
    transaction: &Transaction<'_>,
    reference_id: &str,
    status: &str,
    matched_item_id: Option<&str>,
    external_work_id: Option<&str>,
    method: &str,
    confidence_category: &str,
    score: Option<f64>,
    evidence: &[String],
) -> AppResult<()> {
    transaction.execute(
        "INSERT INTO reference_resolutions (
            reference_id, status, matched_item_id, external_work_id,
            method, confidence_category, score, evidence_json,
            resolver_version, resolved_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(reference_id) DO UPDATE SET
            status = excluded.status,
            matched_item_id = excluded.matched_item_id,
            external_work_id = excluded.external_work_id,
            method = excluded.method,
            confidence_category = excluded.confidence_category,
            score = excluded.score,
            evidence_json = excluded.evidence_json,
            resolver_version = excluded.resolver_version,
            resolved_at = excluded.resolved_at",
        params![
            reference_id,
            status,
            matched_item_id,
            external_work_id,
            method,
            confidence_category,
            score,
            serde_json::to_string(evidence)?,
            RESOLVER_VERSION,
            Utc::now().to_rfc3339(),
        ],
    )?;
    transaction.execute(
        "DELETE FROM citation_resolution_queue WHERE reference_id = ?1",
        [reference_id],
    )?;
    Ok(())
}

fn attach_citation_evidence(
    transaction: &Transaction<'_>,
    source_item_id: &str,
    target_item_id: &str,
    reference_id: &str,
    category: &str,
) -> AppResult<()> {
    let edge_id = stable_id("citation-edge", &[source_item_id, target_item_id]);
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "INSERT INTO citation_edges (
            id, source_item_id, target_item_id, evidence_count,
            strongest_category, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 1, ?4, ?5, ?5)
         ON CONFLICT(source_item_id, target_item_id) DO UPDATE SET
            strongest_category = CASE
                WHEN citation_edges.strongest_category = 'exact DOI match'
                THEN citation_edges.strongest_category
                ELSE excluded.strongest_category
            END,
            updated_at = excluded.updated_at",
        params![edge_id, source_item_id, target_item_id, category, now],
    )?;
    transaction.execute(
        "INSERT OR REPLACE INTO citation_evidence (
            citation_edge_id, reference_id
         ) VALUES (?1, ?2)",
        params![edge_id, reference_id],
    )?;
    transaction.execute(
        "UPDATE citation_edges
         SET evidence_count = (
             SELECT COUNT(*) FROM citation_evidence ce
             WHERE ce.citation_edge_id = citation_edges.id
         )
         WHERE id = ?1",
        [&edge_id],
    )?;
    Ok(())
}

fn cleanup_citation_edges(transaction: &Transaction<'_>) -> AppResult<()> {
    transaction.execute(
        "DELETE FROM citation_edges
         WHERE NOT EXISTS (
             SELECT 1 FROM citation_evidence ce
             WHERE ce.citation_edge_id = citation_edges.id
         )",
        [],
    )?;
    transaction.execute(
        "UPDATE citation_edges
         SET evidence_count = (
             SELECT COUNT(*) FROM citation_evidence ce
             WHERE ce.citation_edge_id = citation_edges.id
         )",
        [],
    )?;
    Ok(())
}

fn reresolve_batch(
    database: &ResearchDatabase,
    source_id: &str,
    library_id: &str,
    limit: u32,
) -> AppResult<()> {
    let mut connection = database.connect()?;
    reresolve_batch_with_connection(&mut connection, source_id, library_id, limit)?;
    refresh_index_state(&connection, library_id, None)
}

fn reresolve_batch_with_connection(
    connection: &mut Connection,
    source_id: &str,
    library_id: &str,
    limit: u32,
) -> AppResult<()> {
    let reference_ids = {
        let mut statement = connection.prepare(
            "SELECT r.id
             FROM bibliographic_references r
             JOIN reference_extractions e ON e.id = r.extraction_id
             LEFT JOIN reference_resolutions rr ON rr.reference_id = r.id
             LEFT JOIN citation_resolution_queue q ON q.reference_id = r.id
             WHERE r.library_id = ?1 AND r.is_current = 1 AND e.is_current = 1
               AND (q.reference_id IS NOT NULL OR rr.reference_id IS NULL
                    OR rr.resolver_version <> ?2)
             ORDER BY CASE WHEN q.reference_id IS NULL THEN 1 ELSE 0 END,
                      r.source_item_id, r.ordinal
             LIMIT ?3",
        )?;
        let rows = statement
            .query_map(params![library_id, RESOLVER_VERSION, limit], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for reference_id in reference_ids {
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM citation_evidence WHERE reference_id = ?1",
            [&reference_id],
        )?;
        let reference = load_reference_for_resolution(&transaction, &reference_id)?;
        if let Some((source_item_id, external_work_id, authoritative, reference)) = reference {
            resolve_reference(
                &transaction,
                &reference_id,
                source_id,
                &source_item_id,
                &reference,
                &external_work_id,
                authoritative,
            )?;
        }
        cleanup_citation_edges(&transaction)?;
        transaction.commit()?;
    }
    Ok(())
}

fn load_reference_for_resolution(
    transaction: &Transaction<'_>,
    reference_id: &str,
) -> AppResult<Option<(String, String, bool, ExtractedReference)>> {
    transaction
        .query_row(
            "SELECT r.source_item_id, rr.external_work_id,
                    e.status, r.ordinal, r.raw_text, r.original_doi,
                    r.normalized_doi, r.parsed_title, r.parsed_year,
                    r.first_author, r.parsed_creators_json, r.parsed_venue,
                    r.source_start, r.source_end, r.extraction_confidence
             FROM bibliographic_references r
             JOIN reference_extractions e ON e.id = r.extraction_id
             JOIN reference_resolutions rr ON rr.reference_id = r.id
             WHERE r.id = ?1 AND r.is_current = 1 AND e.is_current = 1",
            [reference_id],
            |row| {
                let creators_json: String = row.get(10)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)? == "references-extracted",
                    ExtractedReference {
                        ordinal: row.get(3)?,
                        raw_text: row.get(4)?,
                        original_doi: row.get(5)?,
                        normalized_doi: row.get(6)?,
                        title: row.get(7)?,
                        year: row.get(8)?,
                        first_author: row.get(9)?,
                        creators: serde_json::from_str(&creators_json).unwrap_or_default(),
                        venue: row.get(11)?,
                        start: row.get::<_, Option<i64>>(12)?.map(|value| value as u64),
                        end: row.get::<_, Option<i64>>(13)?.map(|value| value as u64),
                        confidence: row.get(14)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(Into::into)
}

fn rebuild_index(database: &ResearchDatabase, library_id: &str) -> AppResult<()> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "DELETE FROM citation_evidence
         WHERE reference_id IN (
             SELECT id FROM bibliographic_references WHERE library_id = ?1
         )",
        [library_id],
    )?;
    transaction.execute(
        "UPDATE reference_extractions
         SET is_current = 0, status = 'stale'
         WHERE library_id = ?1",
        [library_id],
    )?;
    transaction.execute(
        "UPDATE bibliographic_references SET is_current = 0
         WHERE library_id = ?1",
        [library_id],
    )?;
    transaction.execute(
        "DELETE FROM reference_resolution_candidates
         WHERE reference_id IN (
             SELECT id FROM bibliographic_references WHERE library_id = ?1
         )",
        [library_id],
    )?;
    transaction.execute(
        "DELETE FROM reference_resolutions
         WHERE reference_id IN (
             SELECT id FROM bibliographic_references WHERE library_id = ?1
         )",
        [library_id],
    )?;
    cleanup_citation_edges(&transaction)?;
    transaction.execute(
        "INSERT INTO citation_index_state (
            library_id, status, extractor_version, resolver_version
         ) VALUES (?1, 'queued', ?2, ?3)
         ON CONFLICT(library_id) DO UPDATE SET
            status = 'queued', extractor_version = excluded.extractor_version,
            resolver_version = excluded.resolver_version, last_error = NULL",
        params![library_id, EXTRACTOR_VERSION, RESOLVER_VERSION],
    )?;
    transaction.commit()?;
    refresh_index_state(&connection, library_id, None)
}

fn set_index_status(
    connection: &Connection,
    library_id: &str,
    status: &str,
    error: Option<&str>,
) -> AppResult<()> {
    connection.execute(
        "INSERT INTO citation_index_state (
            library_id, status, extractor_version, resolver_version,
            last_error
         ) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(library_id) DO UPDATE SET
            status = excluded.status,
            extractor_version = excluded.extractor_version,
            resolver_version = excluded.resolver_version,
            last_error = excluded.last_error",
        params![
            library_id,
            status,
            EXTRACTOR_VERSION,
            RESOLVER_VERSION,
            error
        ],
    )?;
    Ok(())
}

fn refresh_index_state(
    connection: &Connection,
    library_id: &str,
    error: Option<&str>,
) -> AppResult<()> {
    let diagnostics = load_citation_diagnostics(connection, Some(library_id))?;
    let status = if error.is_some() {
        "failed"
    } else if diagnostics.queued_documents > 0 {
        "queued"
    } else if diagnostics.eligible_documents == 0 {
        "not-indexed"
    } else {
        "ready"
    };
    connection.execute(
        "INSERT INTO citation_index_state (
            library_id, status, extractor_version, resolver_version,
            eligible_documents, indexed_documents, no_reference_section,
            partial_documents, failed_documents, references_extracted,
            references_with_doi, resolved_local, external_works,
            ambiguous_references, unresolved_references, citation_edges,
            queued_documents, last_indexed_at, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19
         )
         ON CONFLICT(library_id) DO UPDATE SET
            status = excluded.status,
            extractor_version = excluded.extractor_version,
            resolver_version = excluded.resolver_version,
            eligible_documents = excluded.eligible_documents,
            indexed_documents = excluded.indexed_documents,
            no_reference_section = excluded.no_reference_section,
            partial_documents = excluded.partial_documents,
            failed_documents = excluded.failed_documents,
            references_extracted = excluded.references_extracted,
            references_with_doi = excluded.references_with_doi,
            resolved_local = excluded.resolved_local,
            external_works = excluded.external_works,
            ambiguous_references = excluded.ambiguous_references,
            unresolved_references = excluded.unresolved_references,
            citation_edges = excluded.citation_edges,
            queued_documents = excluded.queued_documents,
            last_indexed_at = excluded.last_indexed_at,
            last_error = excluded.last_error",
        params![
            library_id,
            status,
            EXTRACTOR_VERSION,
            RESOLVER_VERSION,
            diagnostics.eligible_documents as i64,
            diagnostics.indexed_documents as i64,
            diagnostics.no_reference_section as i64,
            diagnostics.partial_documents as i64,
            diagnostics.failed_documents as i64,
            diagnostics.references_extracted as i64,
            diagnostics.references_with_doi as i64,
            diagnostics.resolved_local as i64,
            diagnostics.external_works as i64,
            diagnostics.ambiguous_references as i64,
            diagnostics.unresolved_references as i64,
            diagnostics.citation_edges as i64,
            diagnostics.queued_documents as i64,
            Utc::now().to_rfc3339(),
            error,
        ],
    )?;
    Ok(())
}

pub fn apply_override(
    database: &ResearchDatabase,
    request: &ResolutionOverrideRequest,
) -> AppResult<()> {
    if !["accept", "reject", "unresolved"].contains(&request.decision.as_str()) {
        return Err(AppError::ApplicationData(
            "The citation review decision is not supported".to_owned(),
        ));
    }
    if request.decision == "accept" && request.target_item_id.is_none() {
        return Err(AppError::ApplicationData(
            "Accepting a reference candidate requires a target paper".to_owned(),
        ));
    }
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let (source_id, source_item_id, fingerprint) = transaction
        .query_row(
            "SELECT r.source_id, r.source_item_id, r.reference_fingerprint
             FROM bibliographic_references r
             JOIN reference_extractions e ON e.id = r.extraction_id
             WHERE r.id = ?1 AND r.is_current = 1 AND e.is_current = 1",
            [&request.reference_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            AppError::ApplicationData(
                "The bibliography entry is no longer current; refresh the citation view".to_owned(),
            )
        })?;
    if let Some(target_id) = &request.target_item_id {
        let valid = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM research_items
                            WHERE id = ?1 AND source_id = ?2 AND is_trashed = 0)",
            params![target_id, source_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !valid {
            return Err(AppError::ApplicationData(
                "The selected resolution target is unavailable in this Graphite source".to_owned(),
            ));
        }
    }
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "INSERT INTO reference_resolution_overrides (
            reference_id, decision, target_item_id,
            source_reference_fingerprint, stale, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)
         ON CONFLICT(reference_id) DO UPDATE SET
            decision = excluded.decision,
            target_item_id = excluded.target_item_id,
            source_reference_fingerprint = excluded.source_reference_fingerprint,
            stale = 0,
            updated_at = excluded.updated_at",
        params![
            request.reference_id,
            request.decision,
            request.target_item_id,
            fingerprint,
            now,
        ],
    )?;
    transaction.execute(
        "DELETE FROM citation_evidence WHERE reference_id = ?1",
        [&request.reference_id],
    )?;
    let (_, external_work_id, authoritative, reference) =
        load_reference_for_resolution(&transaction, &request.reference_id)?
            .ok_or_else(|| AppError::ApplicationData("Reference became stale".to_owned()))?;
    resolve_reference(
        &transaction,
        &request.reference_id,
        &source_id,
        &source_item_id,
        &reference,
        &external_work_id,
        authoritative,
    )?;
    cleanup_citation_edges(&transaction)?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use crate::model::{
        CitationListRequest, EdgeEvidenceRequest, FullTextRecord, FullTextSyncDelta, LibraryDelta,
        MissingWorksRequest, RawObject, ReferenceQueryRequest, RemoteLibrary,
        ResearchNeighborhoodRequest, ResolutionOverrideRequest, SearchRequest, SourceProbe,
    };

    use super::{
        apply_override, cited_by_page, co_cited_works, index_document_batch,
        load_citation_diagnostics, missing_works, neighborhood, rebuild_index, reference_page,
        relationship_evidence, reresolve_batch, shared_reference_papers, stable_id,
        ResearchDatabase,
    };

    struct TestCorpus {
        database: ResearchDatabase,
        path: PathBuf,
        source_id: String,
        library_id: String,
    }

    impl Drop for TestCorpus {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn fixture_item(
        key: &str,
        title: &str,
        author: &str,
        year: i32,
        doi: Option<&str>,
    ) -> RawObject {
        RawObject {
            key: key.to_owned(),
            version: 1,
            object_type: "items".to_owned(),
            json: serde_json::json!({
                "key": key,
                "version": 1,
                "data": {
                    "key": key,
                    "version": 1,
                    "itemType": "journalArticle",
                    "title": title,
                    "date": year.to_string(),
                    "DOI": doi.unwrap_or_default(),
                    "publicationTitle": "Journal of Synthetic Geophysics",
                    "creators": [{
                        "creatorType": "author",
                        "firstName": "A.",
                        "lastName": author
                    }],
                    "tags": [],
                    "collections": []
                }
            }),
        }
    }

    fn fixture_attachment(key: &str, parent_key: &str) -> RawObject {
        RawObject {
            key: key.to_owned(),
            version: 1,
            object_type: "items".to_owned(),
            json: serde_json::json!({
                "key": key,
                "version": 1,
                "data": {
                    "key": key,
                    "version": 1,
                    "itemType": "attachment",
                    "parentItem": parent_key,
                    "title": "Indexed full text",
                    "contentType": "application/pdf",
                    "filename": format!("{key}.pdf"),
                    "linkMode": "imported_file"
                }
            }),
        }
    }

    fn test_corpus(name: &str, items: Vec<RawObject>, documents: Vec<(&str, &str)>) -> TestCorpus {
        let path = std::env::temp_dir().join(format!(
            "graphite-citation-{name}-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let database = ResearchDatabase::initialize(path.clone()).expect("database");
        let source = database
            .register_source(&SourceProbe {
                server_id: format!("citation-test-{name}"),
                api_version: "3".to_owned(),
                schema_version: Some("37".to_owned()),
            })
            .expect("source");
        let library = database
            .register_libraries(
                &source.id,
                &[RemoteLibrary {
                    kind: "user".to_owned(),
                    zotero_id: "0".to_owned(),
                    name: "Test library".to_owned(),
                }],
            )
            .expect("library")
            .remove(0);
        let mut objects = items;
        for (attachment_key, parent_key) in &documents {
            objects.push(fixture_attachment(attachment_key, parent_key));
        }
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects,
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("metadata delta");
        TestCorpus {
            database,
            path,
            source_id: source.id,
            library_id: library.id,
        }
    }

    fn apply_texts(corpus: &TestCorpus, version: i64, records: Vec<(&str, &str)>) {
        corpus
            .database
            .apply_fulltext_delta(
                &corpus.source_id,
                &corpus.library_id,
                &FullTextSyncDelta {
                    successful_version: version,
                    records: records
                        .into_iter()
                        .map(|(attachment_key, content)| FullTextRecord {
                            attachment_key: attachment_key.to_owned(),
                            version,
                            content: content.to_owned(),
                            indexed_pages: Some(8),
                            total_pages: Some(8),
                            indexed_chars: Some(content.len() as u64),
                            total_chars: Some(content.len() as u64),
                        })
                        .collect(),
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("full-text delta");
    }

    fn item_id(corpus: &TestCorpus, key: &str) -> String {
        stable_id("item", &[&corpus.source_id, &corpus.library_id, key])
    }

    const READ_MODEL_TEXT_A: &str = "Methods and observations.\n\nReferences\n\
[1] Lovelace, A. (2020). Target waves in crustal imaging. Journal. \
https://doi.org/10.1000/target\n\
[2] Curie, M. (2018). Missing attenuation structure. Journal. \
doi:10.1000/external\n\
[3] Noether, A. (2019). Robust inversion of surface dispersion. \
Journal of Synthetic Geophysics.\n\
[4] Faraday, M. (2015). External field observations. Journal. \
doi:10.1000/field\n\
[5] Curie, M. (2018). Target waves in crustal imaging. Another Journal.\n\
[6] Curie, M. (2018). Missing attenuation structure. Journal. \
https://doi.org/10.1000/external";

    const READ_MODEL_TEXT_D: &str = "Discussion.\n\nBibliography\n\
[1] Lovelace, A. (2020). Target waves in crustal imaging. Journal. \
doi.org/10.1000/TARGET.\n\
[2] Curie, M. (2018). Missing attenuation structure. Journal. \
https://doi.org/10.1000/external\n\
[3] Maxwell, J. (2011). Peripheral wavefield study. Journal. \
doi:10.1000/peripheral\n\
[4] Faraday, M. (2015). External field observations. Journal. \
doi:10.1000/field";

    fn indexed_read_model_corpus(name: &str) -> TestCorpus {
        let corpus = test_corpus(
            name,
            vec![
                fixture_item("SOURCEA", "Source paper A", "Euler", 2024, None),
                fixture_item("SOURCED", "Source paper D", "Gauss", 2023, None),
                fixture_item(
                    "TARGETB",
                    "Target waves in crustal imaging",
                    "Lovelace",
                    2020,
                    Some("10.1000/target"),
                ),
                fixture_item(
                    "TARGETC",
                    "Robust inversion of surface dispersion",
                    "Noether",
                    2019,
                    None,
                ),
            ],
            vec![("ATTACHA", "SOURCEA"), ("ATTACHD", "SOURCED")],
        );
        apply_texts(
            &corpus,
            1,
            vec![
                ("ATTACHA", READ_MODEL_TEXT_A),
                ("ATTACHD", READ_MODEL_TEXT_D),
            ],
        );
        index_document_batch(
            &corpus.database,
            &corpus.source_id,
            &corpus.library_id,
            None,
            10,
        )
        .expect("citation index");
        corpus
    }

    #[test]
    fn resolves_local_and_external_works_with_provenance() {
        let corpus = indexed_read_model_corpus("resolution");

        let connection = corpus.database.connect().expect("connection");
        let diagnostics =
            load_citation_diagnostics(&connection, Some(&corpus.library_id)).expect("diagnostics");
        assert_eq!(diagnostics.indexed_documents, 2);
        assert_eq!(diagnostics.references_extracted, 10);
        assert_eq!(diagnostics.resolved_local, 3);
        assert_eq!(diagnostics.citation_edges, 3);

        let citation_filtered = corpus
            .database
            .search(&SearchRequest {
                query: String::new(),
                mode: None,
                limit: Some(20),
                offset: Some(0),
                item_ids: Vec::new(),
                collection_id: None,
                citation_filter: Some("has-external".to_owned()),
                year_from: None,
                year_to: None,
                item_type: None,
            })
            .expect("citation-filtered search");
        assert_eq!(citation_filtered.total, 2);
        assert!(citation_filtered.results.iter().all(|result| result
            .reasons
            .contains(&"contains external references".to_owned())));

        let source_a = item_id(&corpus, "SOURCEA");
        let scoped_citations = corpus
            .database
            .search(&SearchRequest {
                query: String::new(),
                mode: None,
                limit: Some(20),
                offset: Some(0),
                item_ids: vec![source_a.clone()],
                collection_id: None,
                citation_filter: Some("has-external".to_owned()),
                year_from: None,
                year_to: None,
                item_type: None,
            })
            .expect("item and citation filtered search");
        assert_eq!(scoped_citations.total, 1);
        assert_eq!(scoped_citations.results[0].item.id, source_a);
        let unknown_citations = corpus
            .database
            .search(&SearchRequest {
                query: String::new(),
                mode: None,
                limit: Some(20),
                offset: Some(0),
                item_ids: vec!["foreign-or-missing-item".to_owned()],
                collection_id: None,
                citation_filter: Some("has-external".to_owned()),
                year_from: None,
                year_to: None,
                item_type: None,
            })
            .expect("unknown citation scope");
        assert_eq!(unknown_citations.total, 0);
        let target_b = item_id(&corpus, "TARGETB");
        let page = reference_page(
            &corpus.database,
            &ReferenceQueryRequest {
                item_id: source_a.clone(),
                filter: None,
                limit: Some(20),
                offset: Some(0),
            },
        )
        .expect("reference page");
        assert_eq!(page.references.len(), 6);
        assert!(page.references.iter().any(|reference| {
            reference.normalized_doi.as_deref() == Some("10.1000/target")
                && reference
                    .matched_item
                    .as_ref()
                    .is_some_and(|item| item.id == target_b)
        }));
        assert!(page.references.iter().any(|reference| {
            reference.resolution_status == "external" && reference.external_work.is_some()
        }));
        assert!(page.references.iter().any(|reference| {
            reference.first_author.as_deref() == Some("curie")
                && reference.parsed_title.as_deref() == Some("Target waves in crustal imaging")
                && reference.matched_item.is_none()
        }));

        let external_instances = connection
            .query_row(
                "SELECT COUNT(*) FROM referenced_works
                 WHERE normalized_doi = '10.1000/external'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("external works");
        assert_eq!(external_instances, 1);

        let edge_id = stable_id("citation-edge", &[&source_a, &target_b]);
        let evidence = relationship_evidence(
            &corpus.database,
            &EdgeEvidenceRequest {
                edge_id: edge_id.clone(),
            },
        )
        .expect("evidence")
        .expect("citation edge");
        assert_eq!(evidence.edge_id, edge_id);
        assert_eq!(evidence.source.id, source_a);
        assert_eq!(evidence.target.id, target_b);
        assert_eq!(evidence.evidence.len(), 1);
        assert!(evidence.evidence[0].raw_text.contains("10.1000/target"));

        let incoming = cited_by_page(
            &corpus.database,
            &crate::model::CitationListRequest {
                item_id: target_b.clone(),
                limit: Some(20),
                offset: Some(0),
            },
        )
        .expect("reverse citation query");
        assert_eq!(incoming.total, 2);
        assert!(incoming.items.iter().all(|item| item.id != target_b));

        let shared = shared_reference_papers(&corpus.database, &source_a, 20)
            .expect("bibliographic coupling");
        assert_eq!(shared.links.len(), 1);
        assert_eq!(shared.links[0].count, 3);

        let co_cited = co_cited_works(&corpus.database, &target_b, 20).expect("co-citation query");
        let external_co_citation = co_cited
            .links
            .iter()
            .find(|link| {
                link.external_work
                    .as_ref()
                    .and_then(|work| work.doi.as_deref())
                    == Some("10.1000/external")
            })
            .expect("co-cited external work");
        assert_eq!(external_co_citation.count, 2);

        let focused = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec![source_a],
                mode: "references".to_owned(),
                max_nodes: Some(4),
                max_edges: Some(3),
                offset: Some(0),
            },
        )
        .expect("bounded neighborhood");
        assert!(focused.projection.graph.nodes.len() <= 4);
        assert!(focused.projection.graph.edges.len() <= 3);
        assert!(focused
            .projection
            .graph
            .nodes
            .iter()
            .any(|node| node.entity_kind == "external-work"));
        let unique_node_count = focused
            .projection
            .graph
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(unique_node_count, focused.projection.graph.nodes.len());
        assert!(focused
            .projection
            .graph
            .edges
            .iter()
            .any(|edge| edge.evidence_count == 2));

        let before = diagnostics.indexed_documents;
        index_document_batch(
            &corpus.database,
            &corpus.source_id,
            &corpus.library_id,
            None,
            10,
        )
        .expect("unchanged re-index");
        let unchanged = load_citation_diagnostics(
            &corpus.database.connect().expect("connection"),
            Some(&corpus.library_id),
        )
        .expect("diagnostics");
        assert_eq!(unchanged.indexed_documents, before);

        apply_texts(
            &corpus,
            2,
            vec![
                ("ATTACHA", READ_MODEL_TEXT_A),
                ("ATTACHD", READ_MODEL_TEXT_D),
            ],
        );
        index_document_batch(
            &corpus.database,
            &corpus.source_id,
            &corpus.library_id,
            None,
            10,
        )
        .expect("content-identical source version");
        let extraction_rows = corpus
            .database
            .connect()
            .expect("connection")
            .query_row("SELECT COUNT(*) FROM reference_extractions", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("extraction rows");
        assert_eq!(extraction_rows, 2);

        let connection = corpus.database.connect().expect("connection");
        connection
            .execute(
                "UPDATE reference_resolutions SET resolver_version = '0.0.0'",
                [],
            )
            .expect("simulate resolver upgrade");
        drop(connection);
        reresolve_batch(&corpus.database, &corpus.source_id, &corpus.library_id, 100)
            .expect("version-triggered re-resolution");
        let stale_versions = corpus
            .database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT COUNT(*) FROM reference_resolutions
                 WHERE resolver_version <> ?1",
                [crate::bibliography::RESOLVER_VERSION],
                |row| row.get::<_, i64>(0),
            )
            .expect("resolver versions");
        assert_eq!(stale_versions, 0);
    }

    #[test]
    fn read_models_preserve_filters_pagination_and_missing_work_order() {
        let corpus = indexed_read_model_corpus("read-model-pages");
        let source_a = item_id(&corpus, "SOURCEA");
        let source_d = item_id(&corpus, "SOURCED");
        let target_b = item_id(&corpus, "TARGETB");

        let all = reference_page(
            &corpus.database,
            &ReferenceQueryRequest {
                item_id: source_a.clone(),
                filter: None,
                limit: Some(100),
                offset: Some(0),
            },
        )
        .expect("complete reference page");
        assert_eq!(all.total, 6);
        assert_eq!(all.limit, 100);
        assert_eq!(all.offset, 0);
        assert_eq!(
            all.references
                .iter()
                .map(|reference| reference.ordinal)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6]
        );
        assert_eq!(all.coverage.item_id, source_a);
        assert_eq!(all.coverage.extraction_status, "references-extracted");
        assert_eq!(all.coverage.total_references, 6);
        assert_eq!(all.coverage.resolved_local, 2);
        assert_eq!(all.coverage.external, 4);
        assert_eq!(all.coverage.ambiguous, 0);
        assert_eq!(all.coverage.unresolved, 0);
        assert_eq!(all.coverage.cited_by_local, 0);

        for (filter, expected) in [("local", 2), ("external", 4), ("unresolved", 0)] {
            let filtered = reference_page(
                &corpus.database,
                &ReferenceQueryRequest {
                    item_id: source_a.clone(),
                    filter: Some(filter.to_owned()),
                    limit: Some(100),
                    offset: Some(0),
                },
            )
            .expect("filtered reference page");
            assert_eq!(filtered.total, expected);
            assert_eq!(filtered.references.len() as u64, expected);
        }
        let ambiguous = reference_page(
            &corpus.database,
            &ReferenceQueryRequest {
                item_id: source_a.clone(),
                filter: Some("ambiguous".to_owned()),
                limit: Some(100),
                offset: Some(0),
            },
        )
        .expect("empty ambiguous page");
        assert_eq!(ambiguous.total, 0);
        assert!(ambiguous.references.is_empty());

        let paged = reference_page(
            &corpus.database,
            &ReferenceQueryRequest {
                item_id: source_a,
                filter: None,
                limit: Some(2),
                offset: Some(2),
            },
        )
        .expect("paginated references");
        assert_eq!(paged.limit, 2);
        assert_eq!(paged.offset, 2);
        assert_eq!(paged.total, 6);
        assert_eq!(
            paged
                .references
                .iter()
                .map(|reference| &reference.id)
                .collect::<Vec<_>>(),
            all.references[2..4]
                .iter()
                .map(|reference| &reference.id)
                .collect::<Vec<_>>()
        );

        let first_citing = cited_by_page(
            &corpus.database,
            &CitationListRequest {
                item_id: target_b.clone(),
                limit: Some(1),
                offset: Some(0),
            },
        )
        .expect("first citing paper");
        assert_eq!(first_citing.total, 2);
        assert_eq!(first_citing.items[0].id, item_id(&corpus, "SOURCEA"));
        let second_citing = cited_by_page(
            &corpus.database,
            &CitationListRequest {
                item_id: target_b,
                limit: Some(1),
                offset: Some(1),
            },
        )
        .expect("second citing paper");
        assert_eq!(second_citing.items[0].id, source_d);

        let first_missing = missing_works(
            &corpus.database,
            &MissingWorksRequest {
                limit: Some(2),
                offset: Some(0),
            },
        )
        .expect("first missing works page");
        assert_eq!(first_missing.total, 4);
        assert_eq!(
            first_missing
                .works
                .iter()
                .map(|work| work.doi.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("10.1000/field"), Some("10.1000/external")]
        );
        assert!(first_missing
            .works
            .iter()
            .all(|work| work.referenced_by_count == 2));
        let second_missing = missing_works(
            &corpus.database,
            &MissingWorksRequest {
                limit: Some(2),
                offset: Some(2),
            },
        )
        .expect("second missing works page");
        assert_eq!(second_missing.works.len(), 2);
        assert_eq!(
            second_missing.works[0].doi.as_deref(),
            Some("10.1000/peripheral")
        );
        assert_eq!(second_missing.works[0].referenced_by_count, 1);
        assert_eq!(second_missing.works[1].doi, None);
        assert_eq!(
            second_missing.works[1].title.as_deref(),
            Some("Target waves in crustal imaging")
        );
        assert_eq!(second_missing.works[1].referenced_by_count, 1);

        assert!(relationship_evidence(
            &corpus.database,
            &EdgeEvidenceRequest {
                edge_id: "missing-citation-edge".to_owned(),
            },
        )
        .expect("unknown edge query")
        .is_none());
    }

    #[test]
    fn neighborhoods_preserve_direction_deduplication_limits_and_empty_cases() {
        let corpus = indexed_read_model_corpus("read-model-neighborhoods");
        let source_a = item_id(&corpus, "SOURCEA");
        let source_d = item_id(&corpus, "SOURCED");
        let target_b = item_id(&corpus, "TARGETB");
        let target_c = item_id(&corpus, "TARGETC");

        let references = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec![source_a.clone()],
                mode: "references".to_owned(),
                max_nodes: Some(20),
                max_edges: Some(20),
                offset: Some(0),
            },
        )
        .expect("reference neighborhood");
        assert_eq!(references.mode, "references");
        assert_eq!(references.total_relationships, 5);
        assert_eq!(references.hidden_relationships, 0);
        assert_eq!(references.projection.graph.nodes.len(), 6);
        assert_eq!(references.projection.graph.edges.len(), 5);
        assert_eq!(references.projection.graph.total_node_count, 6);
        assert!(!references.projection.graph.has_more);
        assert_eq!(
            references.projection.graph.available_relations,
            vec!["citation"]
        );
        assert!(references.projection.graph.edges.iter().all(|edge| {
            edge.source == source_a && edge.relation == "citation" && edge.directed
        }));
        let mut reference_targets = references
            .projection
            .graph
            .edges
            .iter()
            .map(|edge| edge.target.clone())
            .collect::<Vec<_>>();
        reference_targets.sort();
        reference_targets.dedup();
        assert_eq!(reference_targets.len(), 5);
        assert!(reference_targets.contains(&target_b));
        assert!(reference_targets.contains(&target_c));
        let mut evidence_counts = references
            .projection
            .graph
            .edges
            .iter()
            .map(|edge| edge.evidence_count)
            .collect::<Vec<_>>();
        evidence_counts.sort();
        assert_eq!(evidence_counts, vec![1, 1, 1, 1, 2]);
        let unique_nodes = references
            .projection
            .graph
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique_nodes.len(), references.projection.graph.nodes.len());
        let seed_node = references
            .projection
            .graph
            .nodes
            .iter()
            .find(|node| node.id == source_a)
            .expect("seed node");
        assert_eq!((seed_node.x, seed_node.y), (0.0, 0.0));
        assert!(seed_node.importance >= 1.0);

        let bounded = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec![source_a.clone()],
                mode: "references".to_owned(),
                max_nodes: Some(3),
                max_edges: Some(1),
                offset: Some(1),
            },
        )
        .expect("bounded reference neighborhood");
        assert_eq!(bounded.total_relationships, 5);
        assert_eq!(bounded.hidden_relationships, 4);
        assert_eq!(bounded.projection.graph.nodes.len(), 3);
        assert_eq!(bounded.projection.graph.edges.len(), 1);
        assert_eq!(bounded.projection.graph.edges[0].target, target_c);
        assert!(bounded.projection.graph.has_more);

        let cited_by = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec![target_b.clone()],
                mode: "cited-by".to_owned(),
                max_nodes: Some(20),
                max_edges: Some(20),
                offset: Some(0),
            },
        )
        .expect("cited-by neighborhood");
        assert_eq!(cited_by.total_relationships, 2);
        assert_eq!(cited_by.projection.graph.edges.len(), 2);
        assert!(cited_by.projection.graph.edges.iter().all(|edge| {
            edge.target == target_b && edge.relation == "citation" && edge.directed
        }));
        let citing_sources = cited_by
            .projection
            .graph
            .edges
            .iter()
            .map(|edge| edge.source.clone())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            citing_sources,
            std::collections::HashSet::from([source_a.clone(), source_d])
        );

        let shared = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec![source_a.clone()],
                mode: "shared-references".to_owned(),
                max_nodes: Some(20),
                max_edges: Some(20),
                offset: Some(0),
            },
        )
        .expect("shared-reference neighborhood");
        assert_eq!(shared.total_relationships, 1);
        assert_eq!(shared.projection.graph.edges.len(), 1);
        assert_eq!(
            shared.projection.graph.edges[0].relation,
            "bibliographic-coupling"
        );
        assert!(!shared.projection.graph.edges[0].directed);
        assert_eq!(shared.projection.graph.edges[0].evidence_count, 3);
        let details = super::connection_details(
            &corpus.database,
            &source_a,
            &shared.projection.graph.edges[0].target,
            "bibliographic-coupling",
        )
        .expect("shared reference titles");
        assert_eq!(details.len(), 3);
        assert!(details.iter().all(|title| !title.is_empty()));

        let co_cited = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec![target_b],
                mode: "co-cited".to_owned(),
                max_nodes: Some(20),
                max_edges: Some(20),
                offset: Some(0),
            },
        )
        .expect("co-cited neighborhood");
        assert_eq!(co_cited.total_relationships, 5);
        assert_eq!(co_cited.projection.graph.edges.len(), 5);
        assert!(co_cited
            .projection
            .graph
            .edges
            .iter()
            .all(|edge| edge.relation == "co-cited" && !edge.directed));
        let co_cited_targets = co_cited
            .projection
            .graph
            .edges
            .iter()
            .map(|edge| &edge.target)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(co_cited_targets.len(), 5);
        for edge in &co_cited.projection.graph.edges {
            let details =
                super::connection_details(&corpus.database, &edge.source, &edge.target, "co-cited")
                    .expect("co-citing paper titles");
            assert_eq!(details.len(), edge.evidence_count as usize);
        }

        let mut co_cited_counts = co_cited
            .projection
            .graph
            .edges
            .iter()
            .map(|edge| edge.evidence_count)
            .collect::<Vec<_>>();
        co_cited_counts.sort();
        assert_eq!(co_cited_counts, vec![1, 1, 1, 2, 2]);

        let first_seed_only = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec![source_a.clone(), item_id(&corpus, "SOURCED")],
                mode: "citation".to_owned(),
                max_nodes: Some(20),
                max_edges: Some(20),
                offset: Some(0),
            },
        )
        .expect("multi-seed citation neighborhood");
        let single_seed = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec![source_a],
                mode: "citation".to_owned(),
                max_nodes: Some(20),
                max_edges: Some(20),
                offset: Some(0),
            },
        )
        .expect("single-seed citation neighborhood");
        assert_eq!(
            serde_json::to_value(first_seed_only).expect("serialize multi-seed neighborhood"),
            serde_json::to_value(single_seed).expect("serialize single-seed neighborhood")
        );

        let empty = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec![item_id(&corpus, "TARGETC")],
                mode: "references".to_owned(),
                max_nodes: Some(20),
                max_edges: Some(20),
                offset: Some(0),
            },
        )
        .expect("empty reference neighborhood");
        assert_eq!(empty.total_relationships, 0);
        assert_eq!(empty.hidden_relationships, 0);
        assert_eq!(empty.projection.graph.nodes.len(), 1);
        assert!(empty.projection.graph.edges.is_empty());
        assert_eq!(empty.projection.graph.total_node_count, 1);
        assert!(!empty.projection.graph.has_more);

        let no_seed = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: Vec::new(),
                mode: "citation".to_owned(),
                max_nodes: None,
                max_edges: None,
                offset: None,
            },
        )
        .expect_err("missing seed must fail");
        assert!(no_seed
            .to_string()
            .contains("A citation neighborhood requires a seed paper"));
        let missing_seed = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec!["missing-paper".to_owned()],
                mode: "citation".to_owned(),
                max_nodes: None,
                max_edges: None,
                offset: None,
            },
        )
        .expect_err("unknown seed must fail");
        assert!(missing_seed
            .to_string()
            .contains("The research item is unavailable"));
    }

    #[test]
    fn preserves_ambiguity_until_a_researcher_accepts_a_candidate() {
        let corpus = test_corpus(
            "ambiguity",
            vec![
                fixture_item("SOURCE", "Source paper", "Euler", 2024, None),
                fixture_item(
                    "DUPA",
                    "Duplicate record A",
                    "Lovelace",
                    2020,
                    Some("10.1000/duplicate"),
                ),
                fixture_item(
                    "DUPB",
                    "Duplicate record B",
                    "Lovelace",
                    2020,
                    Some("https://doi.org/10.1000/DUPLICATE"),
                ),
            ],
            vec![("ATTACH", "SOURCE")],
        );
        let text = "Results.\n\nReferences\n\
[1] Lovelace, A. (2020). Duplicate work. doi:10.1000/duplicate\n\
[2] Curie, M. (2017). External one. doi:10.1000/ext-one\n\
[3] Maxwell, J. (2016). External two. doi:10.1000/ext-two\n\
[4] Faraday, M. (2015). External three. doi:10.1000/ext-three";
        apply_texts(&corpus, 1, vec![("ATTACH", text)]);
        index_document_batch(
            &corpus.database,
            &corpus.source_id,
            &corpus.library_id,
            None,
            10,
        )
        .expect("citation index");

        let source = item_id(&corpus, "SOURCE");
        let duplicate_a = item_id(&corpus, "DUPA");
        let page = reference_page(
            &corpus.database,
            &ReferenceQueryRequest {
                item_id: source.clone(),
                filter: Some("ambiguous".to_owned()),
                limit: Some(10),
                offset: Some(0),
            },
        )
        .expect("ambiguous references");
        assert_eq!(page.references.len(), 1);
        assert_eq!(page.references[0].candidates.len(), 2);
        let reference_id = page.references[0].id.clone();
        let edge_count = corpus
            .database
            .connect()
            .expect("connection")
            .query_row("SELECT COUNT(*) FROM citation_edges", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("edge count");
        assert_eq!(edge_count, 0);

        apply_override(
            &corpus.database,
            &ResolutionOverrideRequest {
                reference_id: reference_id.clone(),
                decision: "reject".to_owned(),
                target_item_id: None,
            },
        )
        .expect("keep external decision");
        let kept_external = reference_page(
            &corpus.database,
            &ReferenceQueryRequest {
                item_id: source.clone(),
                filter: Some("external".to_owned()),
                limit: Some(10),
                offset: Some(0),
            },
        )
        .expect("manual external references");
        let rejected = kept_external
            .references
            .iter()
            .find(|reference| reference.id == reference_id)
            .expect("rejected reference read state");
        assert_eq!(rejected.resolution_status, "external");
        assert_eq!(rejected.resolution_method, "manual-reject");
        assert_eq!(rejected.confidence_category, "external work");
        assert!(rejected.matched_item.is_none());
        assert!(rejected.external_work.is_some());
        assert!(!rejected.override_stale);

        apply_override(
            &corpus.database,
            &ResolutionOverrideRequest {
                reference_id: reference_id.clone(),
                decision: "accept".to_owned(),
                target_item_id: Some(duplicate_a.clone()),
            },
        )
        .expect("manual resolution");
        let accepted = reference_page(
            &corpus.database,
            &ReferenceQueryRequest {
                item_id: source.clone(),
                filter: Some("local".to_owned()),
                limit: Some(10),
                offset: Some(0),
            },
        )
        .expect("manual local references");
        let accepted = accepted
            .references
            .iter()
            .find(|reference| reference.id == reference_id)
            .expect("accepted reference read state");
        assert_eq!(accepted.resolution_status, "manual-resolved");
        assert_eq!(accepted.resolution_method, "manual-accept");
        assert_eq!(accepted.confidence_category, "manual decision");
        assert_eq!(
            accepted.matched_item.as_ref().map(|item| &item.id),
            Some(&duplicate_a)
        );
        assert!(!accepted.override_stale);
        let edge_id = stable_id("citation-edge", &[&source, &duplicate_a]);
        assert!(relationship_evidence(
            &corpus.database,
            &EdgeEvidenceRequest {
                edge_id: edge_id.clone()
            }
        )
        .expect("evidence query")
        .is_some());

        rebuild_index(&corpus.database, &corpus.library_id).expect("rebuild");
        index_document_batch(
            &corpus.database,
            &corpus.source_id,
            &corpus.library_id,
            None,
            10,
        )
        .expect("re-index");
        assert!(
            relationship_evidence(&corpus.database, &EdgeEvidenceRequest { edge_id })
                .expect("preserved override evidence")
                .is_some()
        );
        let override_stale = corpus
            .database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT stale FROM reference_resolution_overrides
                 WHERE reference_id = ?1",
                [&reference_id],
                |row| row.get::<_, bool>(0),
            )
            .expect("override state");
        assert!(!override_stale);

        let same_reference_text = text.replacen("Results.", "Expanded results.", 1);
        apply_texts(&corpus, 2, vec![("ATTACH", &same_reference_text)]);
        index_document_batch(
            &corpus.database,
            &corpus.source_id,
            &corpus.library_id,
            None,
            10,
        )
        .expect("re-index stable source reference");
        let stable_override_stale = corpus
            .database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT stale FROM reference_resolution_overrides
                 WHERE reference_id = ?1",
                [&reference_id],
                |row| row.get::<_, bool>(0),
            )
            .expect("stable override state");
        assert!(!stable_override_stale);
        assert!(relationship_evidence(
            &corpus.database,
            &EdgeEvidenceRequest {
                edge_id: stable_id("citation-edge", &[&source, &duplicate_a])
            }
        )
        .expect("stable override edge")
        .is_some());

        let changed_text = "Revised results.\n\nReferences\n\
[1] Lovelace, A. (2020). Different duplicate work. doi:10.1000/changed\n\
[2] Curie, M. (2017). External one. doi:10.1000/ext-one\n\
[3] Maxwell, J. (2016). External two. doi:10.1000/ext-two\n\
[4] Faraday, M. (2015). External three. doi:10.1000/ext-three";
        apply_texts(&corpus, 3, vec![("ATTACH", changed_text)]);
        let changed_override_stale = corpus
            .database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT stale FROM reference_resolution_overrides
                 WHERE reference_id = ?1",
                [&reference_id],
                |row| row.get::<_, bool>(0),
            )
            .expect("changed override state");
        assert!(changed_override_stale);
    }

    #[test]
    fn invalidates_edges_when_the_source_text_revision_changes() {
        let corpus = test_corpus(
            "revision",
            vec![
                fixture_item("SOURCE", "Changing source", "Euler", 2024, None),
                fixture_item(
                    "TARGETA",
                    "First local target",
                    "Lovelace",
                    2020,
                    Some("10.1000/first"),
                ),
                fixture_item(
                    "TARGETB",
                    "Second local target",
                    "Noether",
                    2021,
                    Some("10.1000/second"),
                ),
            ],
            vec![("ATTACH", "SOURCE")],
        );
        let first_text = "Body.\n\nReferences\n\
[1] Lovelace, A. (2020). First local target. doi:10.1000/first\n\
[2] Curie, M. (2018). External. doi:10.1000/external\n\
[3] Maxwell, J. (2017). External. doi:10.1000/peripheral\n\
[4] Faraday, M. (2016). External. doi:10.1000/field";
        apply_texts(&corpus, 1, vec![("ATTACH", first_text)]);
        index_document_batch(
            &corpus.database,
            &corpus.source_id,
            &corpus.library_id,
            None,
            10,
        )
        .expect("initial index");

        let source = item_id(&corpus, "SOURCE");
        let target_a = item_id(&corpus, "TARGETA");
        let target_b = item_id(&corpus, "TARGETB");
        let first_edge = stable_id("citation-edge", &[&source, &target_a]);
        assert!(relationship_evidence(
            &corpus.database,
            &EdgeEvidenceRequest {
                edge_id: first_edge.clone()
            }
        )
        .expect("initial edge")
        .is_some());

        let second_text = "Revised body.\n\nReferences\n\
[1] Noether, A. (2021). Second local target. doi:10.1000/second\n\
[2] Curie, M. (2018). External. doi:10.1000/external\n\
[3] Maxwell, J. (2017). External. doi:10.1000/peripheral\n\
[4] Faraday, M. (2016). External. doi:10.1000/field";
        apply_texts(&corpus, 2, vec![("ATTACH", second_text)]);
        assert!(relationship_evidence(
            &corpus.database,
            &EdgeEvidenceRequest {
                edge_id: first_edge
            }
        )
        .expect("invalidated edge")
        .is_none());

        index_document_batch(
            &corpus.database,
            &corpus.source_id,
            &corpus.library_id,
            None,
            10,
        )
        .expect("re-index revision");
        let second_edge = stable_id("citation-edge", &[&source, &target_b]);
        assert!(relationship_evidence(
            &corpus.database,
            &EdgeEvidenceRequest {
                edge_id: second_edge
            }
        )
        .expect("replacement edge")
        .is_some());

        corpus
            .database
            .apply_fulltext_delta(
                &corpus.source_id,
                &corpus.library_id,
                &FullTextSyncDelta {
                    successful_version: 3,
                    records: Vec::new(),
                    removed_attachment_keys: vec!["ATTACH".to_owned()],
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("remove source text");
        assert!(relationship_evidence(
            &corpus.database,
            &EdgeEvidenceRequest {
                edge_id: stable_id("citation-edge", &[&source, &target_b])
            }
        )
        .expect("removed source evidence")
        .is_none());
    }

    #[test]
    fn selectively_reresolves_when_a_local_target_appears_or_disappears() {
        let corpus = test_corpus(
            "target-lifecycle",
            vec![fixture_item(
                "SOURCE",
                "Source awaiting a local target",
                "Euler",
                2024,
                None,
            )],
            vec![("ATTACH", "SOURCE")],
        );
        let text = "Body.\n\nReferences\n\
[1] Lovelace, A. (2020). Target arrives later. doi:10.1000/future\n\
[2] Curie, M. (2018). External one. doi:10.1000/ext-one\n\
[3] Maxwell, J. (2017). External two. doi:10.1000/ext-two\n\
[4] Faraday, M. (2016). External three. doi:10.1000/ext-three";
        apply_texts(&corpus, 1, vec![("ATTACH", text)]);
        index_document_batch(
            &corpus.database,
            &corpus.source_id,
            &corpus.library_id,
            None,
            10,
        )
        .expect("initial external index");
        let source = item_id(&corpus, "SOURCE");
        let initial = reference_page(
            &corpus.database,
            &ReferenceQueryRequest {
                item_id: source.clone(),
                filter: None,
                limit: Some(10),
                offset: Some(0),
            },
        )
        .expect("initial references");
        assert_eq!(initial.references[0].resolution_status, "external");

        corpus
            .database
            .apply_delta(
                &corpus.source_id,
                &corpus.library_id,
                &LibraryDelta {
                    target_version: 2,
                    objects: vec![fixture_item(
                        "TARGET",
                        "Target arrives later",
                        "Lovelace",
                        2020,
                        Some("10.1000/future"),
                    )],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("new local target");
        reresolve_batch(&corpus.database, &corpus.source_id, &corpus.library_id, 100)
            .expect("selective re-resolution");
        let target = item_id(&corpus, "TARGET");
        let edge_id = stable_id("citation-edge", &[&source, &target]);
        assert!(relationship_evidence(
            &corpus.database,
            &EdgeEvidenceRequest {
                edge_id: edge_id.clone()
            }
        )
        .expect("new local edge")
        .is_some());

        corpus
            .database
            .apply_delta(
                &corpus.source_id,
                &corpus.library_id,
                &LibraryDelta {
                    target_version: 3,
                    objects: Vec::new(),
                    deleted_items: vec!["TARGET".to_owned()],
                    deleted_collections: Vec::new(),
                },
            )
            .expect("delete target");
        assert!(
            relationship_evidence(&corpus.database, &EdgeEvidenceRequest { edge_id })
                .expect("removed target edge")
                .is_none()
        );
        reresolve_batch(&corpus.database, &corpus.source_id, &corpus.library_id, 100)
            .expect("reclassify deleted target");
        let reclassified = reference_page(
            &corpus.database,
            &ReferenceQueryRequest {
                item_id: source,
                filter: None,
                limit: Some(10),
                offset: Some(0),
            },
        )
        .expect("reclassified references");
        assert_eq!(reclassified.references[0].resolution_status, "external");
    }

    #[test]
    fn keeps_dense_citation_queries_incremental_and_bounded() {
        const TARGETS: usize = 3_000;
        const SOURCES: usize = 20;
        const REFERENCES_PER_SOURCE: usize = 30;

        let mut items = Vec::with_capacity(TARGETS + SOURCES);
        for index in 0..TARGETS {
            items.push(fixture_item(
                &format!("TARGET{index:04}"),
                &format!("Indexed target work {index}"),
                &format!("Author{index}"),
                2000 + (index % 20) as i32,
                Some(&format!("10.5000/target-{index}")),
            ));
        }
        let source_keys = (0..SOURCES)
            .map(|index| format!("SOURCE{index:03}"))
            .collect::<Vec<_>>();
        let attachment_keys = (0..SOURCES)
            .map(|index| format!("ATTACH{index:03}"))
            .collect::<Vec<_>>();
        for (index, source_key) in source_keys.iter().enumerate() {
            items.push(fixture_item(
                source_key,
                &format!("Dense citing paper {index}"),
                "Source",
                2024,
                None,
            ));
        }
        let documents = attachment_keys
            .iter()
            .zip(source_keys.iter())
            .map(|(attachment, source)| (attachment.as_str(), source.as_str()))
            .collect::<Vec<_>>();
        let corpus = test_corpus("scale", items, documents);
        let bibliography = (0..REFERENCES_PER_SOURCE)
            .map(|index| {
                format!(
                    "[{}] Author{index}, A. ({}). Indexed target work {index}. \
                     Journal. doi:10.5000/target-{index}",
                    index + 1,
                    2000 + (index % 20),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let texts = (0..SOURCES)
            .map(|_| format!("Local corpus body.\n\nReferences\n{bibliography}"))
            .collect::<Vec<_>>();
        let records = attachment_keys
            .iter()
            .zip(texts.iter())
            .map(|(attachment, text)| (attachment.as_str(), text.as_str()))
            .collect::<Vec<_>>();
        apply_texts(&corpus, 1, records);

        index_document_batch(
            &corpus.database,
            &corpus.source_id,
            &corpus.library_id,
            None,
            SOURCES as u32,
        )
        .expect("bounded dense index");
        let diagnostics = load_citation_diagnostics(
            &corpus.database.connect().expect("connection"),
            Some(&corpus.library_id),
        )
        .expect("dense diagnostics");
        assert_eq!(diagnostics.indexed_documents, SOURCES as u64);
        assert_eq!(
            diagnostics.references_extracted,
            (SOURCES * REFERENCES_PER_SOURCE) as u64
        );

        let seed = item_id(&corpus, &source_keys[0]);
        let page = reference_page(
            &corpus.database,
            &ReferenceQueryRequest {
                item_id: seed.clone(),
                filter: None,
                limit: Some(10),
                offset: Some(0),
            },
        )
        .expect("paged high-degree references");
        assert_eq!(page.total, REFERENCES_PER_SOURCE as u64);
        assert_eq!(page.references.len(), 10);

        let focused = neighborhood(
            &corpus.database,
            &ResearchNeighborhoodRequest {
                seed_item_ids: vec![seed.clone()],
                mode: "references".to_owned(),
                max_nodes: Some(12),
                max_edges: Some(10),
                offset: Some(0),
            },
        )
        .expect("bounded dense neighborhood");
        assert_eq!(focused.projection.graph.nodes.len(), 12);
        assert_eq!(focused.projection.graph.edges.len(), 10);
        assert!(focused.hidden_relationships > 0);

        let coupling =
            shared_reference_papers(&corpus.database, &seed, 5).expect("bounded inverted coupling");
        assert_eq!(coupling.links.len(), 5);
        assert!(coupling
            .links
            .iter()
            .all(|link| link.count == REFERENCES_PER_SOURCE as u64));
        let materialized_structural_tables = corpus
            .database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table'
                   AND name IN ('bibliographic_coupling', 'co_citation')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("structural storage audit");
        assert_eq!(materialized_structural_tables, 0);
    }
}
