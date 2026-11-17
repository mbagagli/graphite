use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use graphite_pdf_benchmark_common::{
    score_extraction, Anchor, AnchorSet, AnchorVerificationStatus, BenchmarkExtractionEnvelopeV1,
    CorpusManifest, ExtractionObservations, ExtractionOutcome, TimingObservation,
    ANCHOR_SCHEMA_VERSION, BENCHMARK_SCHEMA_VERSION,
};
use graphite_pdf_benchmark_quality_oracle::{json_files, read_json, write_json};
use serde::Serialize;
use sha2::{Digest, Sha256};

mod error {
    pub type AppResult<T> = Result<T, String>;
}

mod semantic {
    use std::fmt::Write;

    use sha2::{Digest, Sha256};

    use crate::error::AppResult;

    pub trait EmbeddingProvider {
        fn token_count(&self, text: &str) -> AppResult<usize>;
    }

    fn text_fingerprint(parts: &[&str]) -> String {
        stable_semantic_id("semantic-text", parts)
    }

    fn stable_semantic_id(namespace: &str, parts: &[&str]) -> String {
        let mut digest = Sha256::new();
        digest.update(namespace.as_bytes());
        for part in parts {
            digest.update([0]);
            digest.update(part.as_bytes());
        }
        let mut identifier = String::with_capacity(64);
        for byte in digest.finalize() {
            write!(&mut identifier, "{byte:02x}").expect("write semantic identifier");
        }
        identifier
    }

    #[allow(dead_code)]
    mod chunking {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../src-tauri/src/semantic/chunking.rs"
        ));
    }

    #[derive(Clone, Debug)]
    pub struct ChunkView {
        pub ordinal: usize,
        pub text: String,
        pub character_start: usize,
        pub character_end: usize,
    }

    struct BenchmarkTokenCounter;

    impl EmbeddingProvider for BenchmarkTokenCounter {
        fn token_count(&self, text: &str) -> AppResult<usize> {
            Ok(text.split_whitespace().count())
        }
    }

    fn source(document_id: &str, text: &str) -> chunking::DocumentSource {
        chunking::DocumentSource {
            item_id: format!("benchmark-item-{document_id}"),
            library_id: "benchmark".to_owned(),
            document_id: document_id.to_owned(),
            document_revision_id: 1,
            provider: "benchmark-projection".to_owned(),
            text: text.to_owned(),
            body_end_byte: text.len(),
            bibliography_excluded: false,
        }
    }

    pub fn chunks(document_id: &str, text: &str, capped: bool) -> AppResult<Vec<ChunkView>> {
        let source = source(document_id, text);
        let result = if capped {
            chunking::scientific_chunks(&BenchmarkTokenCounter, &source)
        } else {
            chunking::scientific_chunks_with_maximum(&BenchmarkTokenCounter, &source, None)
        };
        result.map(|chunks| {
            chunks
                .into_iter()
                .map(|chunk| ChunkView {
                    ordinal: chunk.ordinal,
                    text: chunk.text,
                    character_start: chunk.character_start,
                    character_end: chunk.character_end,
                })
                .collect()
        })
    }

    pub fn profile_id() -> &'static str {
        chunking::CHUNKING_PROFILE_ID
    }

    pub fn maximum_chunks() -> usize {
        chunking::MAXIMUM_CHUNKS
    }

    pub fn projected_token_count(text: &str) -> usize {
        text.split_whitespace().count()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RagAggregate {
    schema_version: &'static str,
    chunking_profile_id: &'static str,
    token_accounting: &'static str,
    chunker_source_sha256: String,
    reports: Vec<RagReport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RagReport {
    document_id: String,
    candidate_id: String,
    profile: String,
    chunk_count: usize,
    projected_token_count: usize,
    expected_chunk_count_without_cap: usize,
    chunk_truncation_begins_at_ordinal: Option<usize>,
    first_excluded_character: Option<usize>,
    chunk_cap_reached: bool,
    projection_truncated_by_cap: bool,
    projection_anchor_count: usize,
    chunk_anchor_count: usize,
    structured_only_anchor_count: usize,
    anchors_after_truncation: Vec<String>,
    anchors: Vec<RagAnchorResult>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RagAnchorResult {
    anchor_id: String,
    projection_present: bool,
    survives_chunking: bool,
    first_chunk_ordinal: Option<usize>,
    context_co_located: Option<bool>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("rag-oracle: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() != 3 {
        return Err("usage: rag <manifest> <run-directory> <privacy-safe-output>".to_owned());
    }
    let manifest_path = PathBuf::from(&arguments[0]);
    let run_directory = PathBuf::from(&arguments[1]);
    let output = PathBuf::from(&arguments[2]);
    let manifest: CorpusManifest = read_json(&manifest_path)?;
    let corpus_root = manifest_path
        .parent()
        .ok_or_else(|| "manifest has no parent directory".to_owned())?;
    let documents = manifest
        .documents
        .iter()
        .map(|document| (document.id.clone(), document))
        .collect::<BTreeMap<_, _>>();
    let mut reports = Vec::new();
    for path in json_files(&run_directory)? {
        let extraction: BenchmarkExtractionEnvelopeV1 = read_json(&path)?;
        let document = documents
            .get(&extraction.document_id)
            .ok_or_else(|| format!("unknown document {}", extraction.document_id))?;
        let anchors: AnchorSet = read_json(&corpus_root.join(&document.anchors))?;
        reports.push(evaluate_projection(document, &anchors, extraction)?);
    }
    reports.sort_by(|left, right| {
        (
            left.candidate_id.as_str(),
            left.profile.as_str(),
            left.document_id.as_str(),
        )
            .cmp(&(
                right.candidate_id.as_str(),
                right.profile.as_str(),
                right.document_id.as_str(),
            ))
    });
    let chunker_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../src-tauri/src/semantic/chunking.rs");
    let source = std::fs::read(chunker_path)
        .map_err(|error| format!("cannot read production chunker: {error}"))?;
    write_json(
        &output,
        &RagAggregate {
            schema_version: "graphite-m5-rag-oracle-v1",
            chunking_profile_id: semantic::profile_id(),
            token_accounting: "production-test-whitespace-token-counter",
            chunker_source_sha256: sha256(&source),
            reports,
        },
    )
}

fn evaluate_projection(
    document: &graphite_pdf_benchmark_common::CorpusDocument,
    anchors: &AnchorSet,
    extraction: BenchmarkExtractionEnvelopeV1,
) -> Result<RagReport, String> {
    let projection = extraction.searchable_projection.as_deref().unwrap_or("");
    let chunks = semantic::chunks(&document.id, projection, true)?;
    let uncapped_chunks = semantic::chunks(&document.id, projection, false)?;
    let verified = anchors
        .anchors
        .iter()
        .filter(|anchor| anchor.verification_status == AnchorVerificationStatus::HumanVerified)
        .collect::<Vec<_>>();
    let mut chunk_matches = BTreeMap::<String, BTreeSet<usize>>::new();
    let mut projection_matches = BTreeMap::<String, bool>::new();
    for anchor in &verified {
        projection_matches.insert(
            anchor.id.clone(),
            anchor_matches(document, &extraction, anchor, projection),
        );
        let ordinals = chunks
            .iter()
            .filter(|chunk| anchor_matches(document, &extraction, anchor, &chunk.text))
            .map(|chunk| chunk.ordinal)
            .collect::<BTreeSet<_>>();
        chunk_matches.insert(anchor.id.clone(), ordinals);
    }
    let anchor_results = verified
        .iter()
        .map(|anchor| {
            let ordinals = &chunk_matches[&anchor.id];
            let related = anchor
                .expected_after
                .as_ref()
                .or(anchor.expected_before.as_ref());
            let context_co_located = related.map(|related| {
                chunk_matches
                    .get(related)
                    .is_some_and(|other| !ordinals.is_disjoint(other))
            });
            RagAnchorResult {
                anchor_id: anchor.id.clone(),
                projection_present: projection_matches[&anchor.id],
                survives_chunking: !ordinals.is_empty(),
                first_chunk_ordinal: ordinals.first().copied(),
                context_co_located,
            }
        })
        .collect::<Vec<_>>();
    let anchors_after_truncation = verified
        .iter()
        .filter(|anchor| {
            projection_matches[&anchor.id]
                && chunk_matches[&anchor.id].is_empty()
                && uncapped_chunks
                    .iter()
                    .any(|chunk| anchor_matches(document, &extraction, anchor, &chunk.text))
        })
        .map(|anchor| anchor.id.clone())
        .collect::<Vec<_>>();
    let projection_characters = projection.chars().count();
    let last_character = chunks
        .last()
        .map(|chunk| chunk.character_end)
        .unwrap_or_default();
    let structured_only_anchor_count = extraction
        .canonical_document
        .as_ref()
        .map(|canonical| {
            let canonical_text = graphite_pdf_benchmark_common::render_plain_text(canonical);
            verified
                .iter()
                .filter(|anchor| {
                    !projection_matches[&anchor.id]
                        && anchor_matches(document, &extraction, anchor, &canonical_text)
                })
                .count()
        })
        .unwrap_or_default();
    Ok(RagReport {
        document_id: extraction.document_id,
        candidate_id: extraction.candidate.candidate_id,
        profile: extraction.profile.as_str().to_owned(),
        chunk_count: chunks.len(),
        projected_token_count: semantic::projected_token_count(projection),
        expected_chunk_count_without_cap: uncapped_chunks.len(),
        chunk_truncation_begins_at_ordinal: (uncapped_chunks.len() > chunks.len())
            .then_some(chunks.len()),
        first_excluded_character: uncapped_chunks
            .get(chunks.len())
            .map(|chunk| chunk.character_start),
        chunk_cap_reached: chunks.len() == semantic::maximum_chunks(),
        projection_truncated_by_cap: chunks.len() == semantic::maximum_chunks()
            && last_character < projection_characters,
        projection_anchor_count: anchor_results
            .iter()
            .filter(|result| result.projection_present)
            .count(),
        chunk_anchor_count: anchor_results
            .iter()
            .filter(|result| result.survives_chunking)
            .count(),
        structured_only_anchor_count,
        anchors_after_truncation,
        anchors: anchor_results,
    })
}

fn anchor_matches(
    document: &graphite_pdf_benchmark_common::CorpusDocument,
    extraction: &BenchmarkExtractionEnvelopeV1,
    anchor: &Anchor,
    text: &str,
) -> bool {
    let mut anchor = anchor.clone();
    anchor.expected_pages.clear();
    anchor.expected_after = None;
    anchor.expected_before = None;
    let anchor_set = AnchorSet {
        schema_version: ANCHOR_SCHEMA_VERSION,
        document_id: document.id.clone(),
        anchors: vec![anchor],
    };
    let candidate = BenchmarkExtractionEnvelopeV1 {
        schema_version: BENCHMARK_SCHEMA_VERSION.to_owned(),
        request_id: "rag-oracle".to_owned(),
        document_id: document.id.clone(),
        input_sha256: extraction.input_sha256.clone(),
        candidate: extraction.candidate.clone(),
        profile: extraction.profile.clone(),
        outcome: ExtractionOutcome::Succeeded,
        outcome_reason: None,
        searchable_projection: Some(text.to_owned()),
        document: None,
        canonical_document: None,
        mapping_losses: Vec::new(),
        warnings: Vec::new(),
        observations: ExtractionObservations::default(),
        timing: TimingObservation::default(),
        diagnostics: BTreeMap::new(),
    };
    score_extraction(document, &anchor_set, &candidate, None)
        .anchor_results
        .first()
        .is_some_and(|result| result.matched)
}

fn sha256(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut output, "{byte:02x}").expect("write SHA-256");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{semantic, sha256};

    #[test]
    fn oracle_includes_the_frozen_production_chunker_profile() {
        assert_eq!(semantic::profile_id(), "scientific-paragraph-v1");
        assert_eq!(semantic::maximum_chunks(), 512);
    }

    #[test]
    fn chunker_source_identity_is_stable_sha256() {
        assert_eq!(
            sha256(b"graphite"),
            "6575522ed8660310608630ebfc8e8ef2e8ec2bc0ee55d232a34428cee95a9837"
        );
    }
}
