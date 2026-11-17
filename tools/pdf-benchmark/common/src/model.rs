use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const BENCHMARK_SCHEMA_VERSION: &str = "graphite-pdf-benchmark-v2";
pub const CANONICAL_SCHEMA_VERSION: &str = "canonical-document-v1";
pub const ANCHOR_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusManifest {
    pub schema_version: u32,
    pub corpus_id: String,
    pub documents: Vec<CorpusDocument>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusDocument {
    pub id: String,
    pub file: String,
    pub sha256: String,
    pub source_url: Option<String>,
    pub license: String,
    pub redistributable: bool,
    pub categories: Vec<String>,
    pub language: String,
    pub expected_pages: Option<u32>,
    pub text_layer: TextLayerKind,
    #[serde(default)]
    pub expected_ocr_pages: Vec<u32>,
    pub anchors: String,
    pub anchors_sha256: String,
    pub zotero_baseline: Option<String>,
    pub zotero_baseline_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TextLayerKind {
    Native,
    Scanned,
    Mixed,
    Broken,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnchorSet {
    pub schema_version: u32,
    pub document_id: String,
    pub anchors: Vec<Anchor>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub id: String,
    pub kind: AnchorKind,
    pub required: bool,
    pub verification_status: AnchorVerificationStatus,
    pub verification_note: String,
    #[serde(default)]
    pub expected_pages: Vec<u32>,
    pub matcher: AnchorMatcher,
    pub expected_after: Option<String>,
    pub expected_before: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AnchorVerificationStatus {
    SuggestedUnverified,
    HumanVerified,
    HumanRejected,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AnchorKind {
    TextFact,
    Heading,
    TableValue,
    Formula,
    FigureCaption,
    Reference,
    PageAssociation,
    ReadingOrderPair,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnchorMatcher {
    Exact {
        value: String,
    },
    Normalized {
        value: String,
    },
    OrderedTokens {
        tokens: Vec<String>,
        max_gap: usize,
    },
    Alternatives {
        values: Vec<String>,
    },
    NumericContext {
        value: String,
        unit: Option<String>,
        tolerance: Option<f64>,
        context: Vec<String>,
        window: Option<usize>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BenchmarkProfile {
    Native,
    RecommendedAuto,
    ForcedOcr,
}

impl BenchmarkProfile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::RecommendedAuto => "recommended-auto",
            Self::ForcedOcr => "forced-ocr",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionOutcome {
    Succeeded,
    Partial,
    Suspicious,
    EmptySuccess,
    Unsupported,
    Unavailable,
    Timeout,
    Crashed,
    Failed,
}

impl ExtractionOutcome {
    pub fn has_output(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Partial | Self::Suspicious | Self::EmptySuccess
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateDescriptor {
    pub candidate_id: String,
    pub version: String,
    pub revision: String,
    pub license: String,
    pub adapter_version: String,
    pub features: Vec<String>,
    pub supported_profiles: Vec<BenchmarkProfile>,
    pub configured_available: bool,
    pub availability_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionRequest {
    pub request_id: String,
    pub document_id: String,
    pub artifact_id: String,
    pub artifact_revision_id: i64,
    pub input_path: String,
    pub input_sha256: String,
    pub media_type: String,
    pub profile: BenchmarkProfile,
    #[serde(default)]
    pub forced_ocr_pages: Vec<u32>,
    pub zotero_baseline_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkExtractionEnvelopeV1 {
    pub schema_version: String,
    pub request_id: String,
    pub document_id: String,
    pub input_sha256: String,
    pub candidate: CandidateDescriptor,
    pub profile: BenchmarkProfile,
    pub outcome: ExtractionOutcome,
    pub outcome_reason: Option<String>,
    pub searchable_projection: Option<String>,
    pub document: Option<BenchmarkDocumentV1>,
    pub canonical_document: Option<CanonicalDocumentV1>,
    #[serde(default)]
    pub mapping_losses: Vec<MappingLoss>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub observations: ExtractionObservations,
    pub timing: TimingObservation,
    #[serde(default)]
    pub diagnostics: BTreeMap<String, serde_json::Value>,
}

impl BenchmarkExtractionEnvelopeV1 {
    pub fn terminal(
        request: &ExtractionRequest,
        candidate: CandidateDescriptor,
        outcome: ExtractionOutcome,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: BENCHMARK_SCHEMA_VERSION.to_owned(),
            request_id: request.request_id.clone(),
            document_id: request.document_id.clone(),
            input_sha256: request.input_sha256.clone(),
            candidate,
            profile: request.profile.clone(),
            outcome,
            outcome_reason: Some(reason.into()),
            searchable_projection: None,
            document: None,
            canonical_document: None,
            mapping_losses: Vec::new(),
            warnings: Vec::new(),
            observations: ExtractionObservations::default(),
            timing: TimingObservation::default(),
            diagnostics: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkDocumentV1 {
    pub title: Option<String>,
    pub language: Option<String>,
    pub reported_page_count: Option<u32>,
    #[serde(default)]
    pub pages: Vec<BenchmarkPage>,
    #[serde(default)]
    pub elements: Vec<BenchmarkElement>,
    #[serde(default)]
    pub reading_order: Vec<String>,
    #[serde(default)]
    pub relationships: Vec<BenchmarkRelationship>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkPage {
    pub page_number: u32,
    pub label: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub rotation_degrees: Option<i16>,
    pub text: String,
    #[serde(default)]
    pub regions: Vec<BenchmarkRegion>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkRegion {
    pub element_id: Option<String>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkElement {
    pub id: String,
    pub kind: BenchmarkElementKind,
    pub text: String,
    pub source_order: u32,
    pub page_number: Option<u32>,
    pub parent_id: Option<String>,
    pub heading_level: Option<u8>,
    pub region: Option<BenchmarkRegion>,
    #[serde(default)]
    pub table: Option<BenchmarkTable>,
    #[serde(default)]
    pub formula_latex: Option<String>,
    #[serde(default)]
    pub figure_asset_id: Option<String>,
    #[serde(default)]
    pub list_ordered: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BenchmarkElementKind {
    Heading,
    Paragraph,
    List,
    ListItem,
    Table,
    Formula,
    Figure,
    Caption,
    Reference,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkTable {
    pub row_count: u32,
    pub column_count: u32,
    pub cells: Vec<BenchmarkTableCell>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkTableCell {
    pub row_index: u32,
    pub column_index: u32,
    pub row_span: u32,
    pub column_span: u32,
    pub is_header: bool,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkRelationship {
    pub source_element_id: String,
    pub target_element_id: String,
    pub relation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MappingLoss {
    pub capability: String,
    pub reason: String,
    pub source_field: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionObservations {
    pub reported_page_count: Option<u32>,
    pub extracted_page_count: Option<u32>,
    #[serde(default)]
    pub pages_with_text: Vec<u32>,
    #[serde(default)]
    pub empty_pages: Vec<u32>,
    #[serde(default)]
    pub suspicious_pages: Vec<u32>,
    pub native_text_present: Option<bool>,
    pub ocr_used: Option<bool>,
    pub ocr_page_indices_reported: Option<bool>,
    #[serde(default)]
    pub ocr_pages: Vec<u32>,
    #[serde(default)]
    pub ocr_recommended_pages: Vec<u32>,
    #[serde(default)]
    pub encoding_warnings: Vec<String>,
    #[serde(default)]
    pub layout_warnings: Vec<String>,
    #[serde(default)]
    pub characters_by_page: Vec<u64>,
    pub provider_text_character_count: Option<u64>,
    #[serde(default)]
    pub structure_retention_warnings: Vec<String>,
    #[serde(default)]
    pub ocr_page_provenance: Vec<OcrPageProvenanceObservation>,
    pub provider_quality: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OcrPageProvenanceObservation {
    pub page_number: u32,
    pub source: String,
    pub model: Option<String>,
    #[serde(default)]
    pub model_revision: Option<String>,
    #[serde(default)]
    pub render_dpi: Option<f64>,
    pub confidence: Option<f64>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimingObservation {
    pub wall_time_ms: u64,
    pub provider_time_ms: Option<u64>,
    pub render_time_ms: Option<u64>,
    pub ocr_time_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalDocumentV1 {
    pub schema_version: String,
    pub artifact_id: String,
    pub artifact_revision_id: i64,
    pub extraction_profile_id: String,
    pub metadata: CanonicalDocumentMetadata,
    pub pages: Vec<CanonicalPage>,
    pub blocks: Vec<CanonicalBlock>,
    pub reading_order: Vec<String>,
    pub relationships: Vec<CanonicalBlockRelationship>,
    pub assets: Vec<CanonicalAssetReference>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalDocumentMetadata {
    pub extracted_title: Option<String>,
    pub language: Option<String>,
    pub reported_page_count_hint: Option<u32>,
    pub document_identifiers: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalPage {
    pub id: String,
    pub physical_page_number: u32,
    pub printed_label: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub rotation_degrees: Option<i16>,
    pub coordinate_space: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalBlock {
    pub id: String,
    pub parent_id: Option<String>,
    pub source_order: u32,
    pub section_path: Vec<String>,
    pub page_range: Option<PageRange>,
    pub regions: Vec<PageRegion>,
    pub content: CanonicalBlockContent,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PageRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PageRegion {
    pub page_id: String,
    pub bounding_box: NormalizedBoundingBox,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedBoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CanonicalBlockContent {
    Heading {
        text: String,
        level: u8,
    },
    Paragraph {
        text: String,
    },
    List {
        ordered: bool,
    },
    ListItem {
        text: String,
    },
    Table {
        row_count: u32,
        column_count: u32,
        cells: Vec<CanonicalTableCell>,
    },
    Formula {
        plain_text: Option<String>,
        latex: Option<String>,
        mathml: Option<String>,
        display: bool,
    },
    Figure {
        asset_id: String,
        alt_text: Option<String>,
    },
    Caption {
        text: String,
    },
    Reference {
        text: String,
        identifiers: BTreeMap<String, String>,
    },
    Other {
        label: String,
        text: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalTableCell {
    pub row_index: u32,
    pub column_index: u32,
    pub row_span: u32,
    pub column_span: u32,
    pub is_header: bool,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalBlockRelationship {
    pub source_block_id: String,
    pub target_block_id: String,
    pub relation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalAssetReference {
    pub id: String,
    pub media_type: String,
    pub cache_relative_key: Option<String>,
    pub content_hash: Option<String>,
    pub byte_count: Option<u64>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnchorResult {
    pub anchor_id: String,
    pub kind: AnchorKind,
    pub required: bool,
    pub projection_matched: bool,
    pub page_qualified_matched: Option<bool>,
    pub structural_matched: Option<bool>,
    pub matched: bool,
    pub matched_page: Option<u32>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ScoreReport {
    pub schema_version: String,
    pub document_id: String,
    pub candidate_id: String,
    pub profile: BenchmarkProfile,
    pub outcome: ExtractionOutcome,
    pub anchor_results: Vec<AnchorResult>,
    pub required_projection_recall: f64,
    pub required_page_qualified_recall: Option<f64>,
    pub required_structural_recall: Option<f64>,
    pub required_anchor_recall: f64,
    pub optional_anchor_recall: f64,
    pub byte_count: u64,
    pub unicode_scalar_count: u64,
    pub token_count: u64,
    pub expected_page_count: Option<u32>,
    pub extracted_page_count: Option<u32>,
    pub page_coverage_ratio: Option<f64>,
    pub missing_pages: Vec<u32>,
    pub characters_by_page: Vec<u64>,
    pub page_character_anomaly_pages: Vec<u32>,
    pub baseline_character_ratio: Option<f64>,
    pub baseline_token_ratio: Option<f64>,
    pub gross_coverage_warning: bool,
    pub truncation_suspected: bool,
    pub replacement_character_count: u64,
    pub control_character_count: u64,
    pub private_use_character_count: u64,
    pub duplicate_five_gram_ratio: f64,
    pub empty_pages: Vec<u32>,
    pub suspicious_pages: Vec<u32>,
    pub expected_ocr_recall: Option<f64>,
    pub false_ocr_pages: Vec<u32>,
    pub silent_loss: bool,
    pub benchmark_quality: String,
    pub quality_reasons: Vec<String>,
    pub false_good: bool,
    pub false_degraded: bool,
    pub projection_hash: Option<String>,
    pub canonical_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AggregateResult {
    pub schema_version: String,
    pub run_id: String,
    pub corpus_id: String,
    pub environment_id: String,
    pub reports: Vec<ScoreReport>,
    pub source_document_count: usize,
    pub generated_at_utc: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OperationalResultSet {
    pub schema_version: String,
    pub corpus_id: String,
    pub environment_id: String,
    pub operating_system_cache_control: String,
    pub reports: Vec<OperationalReport>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OperationalReport {
    pub document_id: String,
    pub candidate_id: String,
    pub candidate_version: String,
    pub profile: BenchmarkProfile,
    pub timeout_class: TimeoutClass,
    pub timeout_stage: TimeoutStage,
    pub timeout_ceiling_ms: u64,
    pub cold_start: OperationalPhaseMeasurement,
    pub warm_extraction: Option<OperationalPhaseMeasurement>,
    pub adapter_binary_bytes: u64,
    pub asset_logical_bytes: u64,
    pub asset_allocated_bytes: Option<u64>,
    pub measurement_notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OperationalPhaseMeasurement {
    pub outcome: ExtractionOutcome,
    pub outcome_reason: Option<String>,
    pub wall_time_ms: u64,
    pub wall_time_microseconds: u64,
    pub adapter_reported_wall_time_ms: Option<u64>,
    pub provider_time_ms: Option<u64>,
    pub render_time_ms: Option<u64>,
    pub ocr_time_ms: Option<u64>,
    pub user_cpu_time_ms: Option<u64>,
    pub system_cpu_time_ms: Option<u64>,
    pub cpu_utilization_percent: Option<f64>,
    pub peak_process_tree_rss_bytes: Option<u64>,
    pub peak_process_tree_thread_count: Option<u64>,
    pub peak_temporary_disk_bytes: u64,
    pub response_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TimeoutClass {
    Ordinary,
    ScannedOcr,
    ExtremeStress,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TimeoutStage {
    Parser,
    OcrPipeline,
    IndeterminatePipeline,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimeoutPolicy {
    pub schema_version: u32,
    pub ordinary_ms: u64,
    pub scanned_ocr_ms: u64,
    pub extreme_stress_ms: u64,
    pub extreme_page_threshold: u32,
    pub scanned_ocr_page_ratio: TimeoutRatio,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimeoutRatio {
    pub numerator: u32,
    pub denominator: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZoteroTextSidecar {
    pub schema_version: u32,
    #[serde(default)]
    pub document_id: Option<String>,
    pub provider: String,
    pub source_version: i64,
    pub content: String,
    pub indexed_pages: Option<u32>,
    pub total_pages: Option<u32>,
    pub indexed_characters: Option<u64>,
    pub total_characters: Option<u64>,
    #[serde(default)]
    pub content_sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssetLock {
    pub schema_version: u32,
    pub candidates: Vec<CandidateAssetLock>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidateAssetLock {
    pub candidate_id: String,
    pub version: String,
    pub revision: String,
    pub license: String,
    pub rust_version: Option<String>,
    #[serde(rename = "crate")]
    pub crate_name: Option<String>,
    pub features: Vec<String>,
    pub assets: Vec<ExternalAsset>,
    pub relevant_issues: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAsset {
    pub id: String,
    pub file_name: String,
    pub url: String,
    pub sha256: Option<String>,
    pub byte_count: Option<u64>,
    pub license: String,
    pub required_for: Vec<BenchmarkProfile>,
    #[serde(default)]
    pub platforms: Vec<String>,
    pub unpack_to: Option<String>,
}
