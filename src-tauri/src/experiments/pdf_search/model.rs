use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agent::context::CONTEXT_ASSEMBLER_VERSION;
use crate::semantic::chunking::{
    BOUNDARY_POLICY, CHUNKING_PROFILE_ID, CHUNKING_VERSION, MAXIMUM_CHUNKS, MAXIMUM_TOKENS,
    OVERLAP_TOKENS, TARGET_TOKENS,
};
use crate::semantic::retrieval_policy::{RRF_K, RRF_LEXICAL_WEIGHT};

pub const EXPERIMENT_SCHEMA_VERSION: u32 = 1;
pub const EXPERIMENT_PROFILE_ID: &str = "pdf-search-v1";
pub const EXPERIMENT_PROFILE_VERSION: &str = "1.0.0";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExperimentMode {
    ProductionZotero,
    ExperimentalZoteroMirror,
    ExperimentalXberg,
    ExperimentalZoteroPlusXberg,
    ExperimentalZoteroPlusXbergOcr,
    ExperimentalDoclingFlat,
    ExperimentalManualDeepReadOcr,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExperimentRetrievalKind {
    Lexical,
    Semantic,
    Hybrid,
}

impl ExperimentRetrievalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lexical => "lexical",
            Self::Semantic => "semantic",
            Self::Hybrid => "hybrid",
        }
    }
}

impl ExperimentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProductionZotero => "production-zotero",
            Self::ExperimentalZoteroMirror => "experimental-zotero-mirror",
            Self::ExperimentalXberg => "experimental-xberg",
            Self::ExperimentalZoteroPlusXberg => "experimental-zotero-plus-xberg",
            Self::ExperimentalZoteroPlusXbergOcr => "experimental-zotero-plus-xberg-ocr",
            Self::ExperimentalDoclingFlat => "experimental-docling-flat",
            Self::ExperimentalManualDeepReadOcr => "experimental-manual-deep-read-ocr",
        }
    }

    pub fn is_experimental(self) -> bool {
        !matches!(self, Self::ProductionZotero)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    Zotero,
    Xberg,
    PdfInspectorOcr,
    Docling,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zotero => "zotero",
            Self::Xberg => "xberg",
            Self::PdfInspectorOcr => "pdf-inspector-ocr",
            Self::Docling => "docling",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentDefinition {
    pub definition_id: String,
    pub schema_version: u32,
    pub experiment_profile: String,
    pub corpus_manifest_sha256: String,
    pub candidate_profiles: BTreeMap<String, String>,
    pub chunking_profile: String,
    pub chunking_contract_sha256: String,
    pub embedding_profile_id: String,
    pub embedding_artifact_identity: String,
    pub retrieval_profile: String,
    pub context_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_bundle_sha256: Option<String>,
}

impl ExperimentDefinition {
    pub fn frozen(
        corpus_manifest_sha256: impl Into<String>,
        embedding_profile_id: impl Into<String>,
        embedding_artifact_identity: impl Into<String>,
    ) -> Self {
        let mut candidate_profiles = BTreeMap::new();
        candidate_profiles.insert(
            "xberg".to_owned(),
            "1.0.14/native/b6660a759d31089075dd89112b77b918f872879a".to_owned(),
        );
        candidate_profiles.insert(
            "pdf-inspector-ocr".to_owned(),
            "1.15.0/recommended-auto/ca6d66714683f71d1b1cd447f766a2b287bea751".to_owned(),
        );
        candidate_profiles.insert(
            "docling".to_owned(),
            "1.16.0/recommended-auto/7f8de88e860e289c42873922bd888cd50815ef26".to_owned(),
        );
        let corpus_manifest_sha256 = corpus_manifest_sha256.into();
        let embedding_profile_id = embedding_profile_id.into();
        let embedding_artifact_identity = embedding_artifact_identity.into();
        let chunking_contract = format!(
            "{CHUNKING_PROFILE_ID}|{CHUNKING_VERSION}|{TARGET_TOKENS}|{OVERLAP_TOKENS}|{MAXIMUM_TOKENS}|{MAXIMUM_CHUNKS}|{BOUNDARY_POLICY}"
        );
        let chunking_contract_sha256 = sha256(chunking_contract.as_bytes());
        let definition_material = serde_json::to_vec(&(
            EXPERIMENT_SCHEMA_VERSION,
            EXPERIMENT_PROFILE_ID,
            EXPERIMENT_PROFILE_VERSION,
            &corpus_manifest_sha256,
            &candidate_profiles,
            CHUNKING_PROFILE_ID,
            &chunking_contract_sha256,
            &embedding_profile_id,
            &embedding_artifact_identity,
            "scoped-hybrid-v1",
            RRF_K.to_bits(),
            RRF_LEXICAL_WEIGHT.to_bits(),
            CONTEXT_ASSEMBLER_VERSION,
        ))
        .expect("experiment definition is serializable");
        Self {
            definition_id: sha256(&definition_material),
            schema_version: EXPERIMENT_SCHEMA_VERSION,
            experiment_profile: format!("{EXPERIMENT_PROFILE_ID}/{EXPERIMENT_PROFILE_VERSION}"),
            corpus_manifest_sha256,
            candidate_profiles,
            chunking_profile: CHUNKING_PROFILE_ID.to_owned(),
            chunking_contract_sha256,
            embedding_profile_id,
            embedding_artifact_identity,
            retrieval_profile: format!(
                "scoped-hybrid-v1;rrf-k={RRF_K};lexical-weight={RRF_LEXICAL_WEIGHT}"
            ),
            context_profile: CONTEXT_ASSEMBLER_VERSION.to_owned(),
            evaluation_bundle_sha256: None,
        }
    }

    pub fn bind_evaluation_bundle(mut self, checksum: &str) -> Result<Self, String> {
        if checksum.len() != 64
            || !checksum
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("The evaluation bundle checksum must be a complete SHA-256".to_owned());
        }
        let material = serde_json::to_vec(&("evaluation-inputs-v1", &self.definition_id, checksum))
            .expect("evaluation-bound experiment definition is serializable");
        self.definition_id = sha256(&material);
        self.evaluation_bundle_sha256 = Some(checksum.to_owned());
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotEmbeddingProfile {
    pub id: String,
    pub artifact_identity: String,
    pub dimension: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotItem {
    pub item_id: String,
    pub library_id: String,
    pub title: String,
    pub creators: String,
    pub tags: String,
    pub abstract_text: String,
    pub venue: String,
    pub document_id: String,
    pub document_revision_id: i64,
    pub source_version: i64,
    pub body: String,
    pub body_end_byte: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotChunk {
    pub id: String,
    pub item_id: String,
    pub document_id: String,
    pub document_revision_id: i64,
    pub ordinal: u32,
    pub text: String,
    pub character_start: u64,
    pub character_end: u64,
    pub page_start: Option<u32>,
    pub page_end: Option<u32>,
    pub fingerprint: String,
    pub vector_key: u64,
    pub vector: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotPaperVector {
    pub item_id: String,
    pub representation_kind: String,
    pub vector_key: u64,
    pub vector: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotProjection {
    pub projection_id: String,
    pub item_id: String,
    pub document_id: String,
    pub document_revision_id: i64,
    pub source_kind: SourceKind,
    pub extraction_id: Option<String>,
    pub provider_profile: String,
    pub content: String,
    pub content_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProductionSnapshot {
    pub embedding_profile: SnapshotEmbeddingProfile,
    pub items: Vec<SnapshotItem>,
    pub chunks: Vec<SnapshotChunk>,
    pub paper_vectors: Vec<SnapshotPaperVector>,
    pub shadow_projections: Vec<SnapshotProjection>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CharacterizationQuery {
    pub id: String,
    pub query: String,
    #[serde(default)]
    pub item_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentPassage {
    pub logical_chunk_id: String,
    pub item_id: String,
    pub document_id: String,
    pub source_kinds: Vec<SourceKind>,
    pub projection_ids: Vec<String>,
    pub extraction_ids: Vec<String>,
    pub page_start: Option<u32>,
    pub page_end: Option<u32>,
    pub content_fingerprint: String,
    pub semantic_score: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentSearchResult {
    pub item_id: String,
    pub title: String,
    pub lexical_rank: Option<u32>,
    pub semantic_rank: Option<u32>,
    pub fusion_score: f64,
    pub passages: Vec<ExperimentPassage>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentSearchPage {
    pub mode: ExperimentMode,
    pub retrieval_kind: ExperimentRetrievalKind,
    pub query_id: String,
    pub results: Vec<ExperimentSearchResult>,
    pub context_fingerprints: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MirrorEquivalenceDifference {
    pub query_id: String,
    pub retrieval_kind: ExperimentRetrievalKind,
    pub field: String,
    pub production: String,
    pub experimental: String,
    pub explanation: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MirrorEquivalenceReport {
    pub definition_id: String,
    pub query_count: u32,
    pub comparison_count: u32,
    pub equivalent: bool,
    pub differences: Vec<MirrorEquivalenceDifference>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentIndexSummary {
    pub definition_id: String,
    pub execution_id: String,
    pub mode: ExperimentMode,
    pub item_count: u64,
    pub projection_count: u64,
    pub chunk_count: u64,
    pub provenance_count: u64,
    pub embedded_chunk_count: u64,
    pub reused_duplicate_count: u64,
    pub paper_vector_count: u64,
    pub sqlite_bytes: u64,
    pub vector_bytes: u64,
    pub idempotent_reuse: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ExperimentCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl ExperimentCancellationToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

pub fn sha256(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut output, "{byte:02x}").expect("write SHA-256");
    }
    output
}

pub fn normalized_chunk_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluation_bundle_changes_only_the_scored_definition_identity() {
        let base = ExperimentDefinition::frozen("corpus", "embedding", "artifact");
        let first = base
            .clone()
            .bind_evaluation_bundle(&"a".repeat(64))
            .expect("first evaluation definition");
        let repeated = base
            .clone()
            .bind_evaluation_bundle(&"a".repeat(64))
            .expect("repeated evaluation definition");
        let second = base
            .clone()
            .bind_evaluation_bundle(&"b".repeat(64))
            .expect("second evaluation definition");
        assert_eq!(first, repeated);
        assert_ne!(base.definition_id, first.definition_id);
        assert_ne!(first.definition_id, second.definition_id);
        assert_eq!(base.evaluation_bundle_sha256, None);
        assert_eq!(first.evaluation_bundle_sha256, Some("a".repeat(64)));
    }

    #[test]
    fn evaluation_bundle_rejects_partial_or_uppercase_hashes() {
        let base = ExperimentDefinition::frozen("corpus", "embedding", "artifact");
        assert!(base.clone().bind_evaluation_bundle("abc").is_err());
        assert!(base.bind_evaluation_bundle(&"A".repeat(64)).is_err());
    }
}
