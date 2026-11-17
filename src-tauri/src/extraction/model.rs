use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const CANONICAL_SCHEMA_VERSION: &str = "canonical-document-v1";

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

impl CanonicalBlockContent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Heading { .. } => "heading",
            Self::Paragraph { .. } => "paragraph",
            Self::List { .. } => "list",
            Self::ListItem { .. } => "list-item",
            Self::Table { .. } => "table",
            Self::Formula { .. } => "formula",
            Self::Figure { .. } => "figure",
            Self::Caption { .. } => "caption",
            Self::Reference { .. } => "reference",
            Self::Other { .. } => "other",
        }
    }

    pub fn plain_text(&self) -> Option<&str> {
        match self {
            Self::Heading { text, .. }
            | Self::Paragraph { text }
            | Self::ListItem { text }
            | Self::Caption { text }
            | Self::Reference { text, .. }
            | Self::Other { text, .. } => Some(text),
            Self::Formula {
                plain_text,
                latex,
                mathml,
                ..
            } => plain_text
                .as_deref()
                .or(latex.as_deref())
                .or(mathml.as_deref()),
            Self::Figure { alt_text, .. } => alt_text.as_deref(),
            Self::List { .. } | Self::Table { .. } => None,
        }
    }
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionProviderDescriptor {
    pub profile_id: String,
    pub provider_id: String,
    pub provider_version: String,
    pub mapping_profile: String,
    pub supported_media_types: Vec<String>,
    pub required_input_forms: Vec<String>,
    pub native_text_capability: bool,
    pub layout_capability: bool,
    pub table_capability: bool,
    pub formula_capability: bool,
    pub figure_capability: bool,
    pub ocr_capability: bool,
    pub page_provenance_capability: bool,
    pub bounding_box_capability: bool,
    pub configured_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionInputDescriptor {
    pub media_type: String,
    pub has_local_file: bool,
    pub has_bounded_reader: bool,
    pub has_source_provided_text: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExtractionSupport {
    Supported,
    Unsupported,
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct ExtractionRequest {
    pub artifact_id: String,
    pub artifact_revision_id: i64,
    pub input_content_hash: String,
    pub input: ExtractionInputDescriptor,
    pub source_text: Option<SourceTextInput>,
}

#[derive(Clone, Debug)]
pub struct SourceTextInput {
    pub text_id: String,
    pub provider: String,
    pub source_version: i64,
    pub content: String,
    pub indexed_pages: Option<u32>,
    pub total_pages: Option<u32>,
    pub indexed_characters: Option<u64>,
    pub total_characters: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SignalAvailability {
    Observed,
    Unsupported,
    NotReported,
    NotApplicable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionSignal<T> {
    pub availability: SignalAvailability,
    pub value: Option<T>,
    pub provenance: String,
    pub explanation: Option<String>,
}

impl<T> ExtractionSignal<T> {
    pub fn observed(value: T, provenance: impl Into<String>) -> Self {
        Self {
            availability: SignalAvailability::Observed,
            value: Some(value),
            provenance: provenance.into(),
            explanation: None,
        }
    }

    pub fn unavailable(
        availability: SignalAvailability,
        provenance: impl Into<String>,
        explanation: impl Into<String>,
    ) -> Self {
        Self {
            availability,
            value: None,
            provenance: provenance.into(),
            explanation: Some(explanation.into()),
        }
    }
}

impl<T> Default for ExtractionSignal<T> {
    fn default() -> Self {
        Self::unavailable(
            SignalAvailability::NotReported,
            "graphite",
            "signal was not reported",
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionObservations {
    pub provider_profile_id: String,
    pub canonical_schema_version: String,
    pub artifact_id: String,
    pub artifact_revision_id: i64,
    pub input_content_hash: String,
    pub reported_page_count: ExtractionSignal<u32>,
    pub extracted_page_count: ExtractionSignal<u32>,
    pub pages_with_text: ExtractionSignal<u32>,
    pub empty_page_indices: ExtractionSignal<Vec<u32>>,
    pub suspicious_page_indices: ExtractionSignal<Vec<u32>>,
    #[serde(default)]
    pub page_text_density_anomaly_indices: ExtractionSignal<Vec<u32>>,
    pub native_text_presence: ExtractionSignal<bool>,
    pub ocr_used: ExtractionSignal<bool>,
    pub ocr_page_indices: ExtractionSignal<Vec<u32>>,
    #[serde(default)]
    pub ocr_mean_confidence: ExtractionSignal<f64>,
    pub encoding_warnings: ExtractionSignal<Vec<String>>,
    pub layout_warnings: ExtractionSignal<Vec<String>>,
    #[serde(default)]
    pub structure_retention_warnings: ExtractionSignal<Vec<String>>,
    #[serde(default)]
    pub replacement_character_count: ExtractionSignal<u64>,
    #[serde(default)]
    pub projection_truncated: ExtractionSignal<bool>,
    #[serde(default)]
    pub native_to_projection_retention_ratio: ExtractionSignal<f64>,
    pub characters_by_page: ExtractionSignal<Vec<u64>>,
    pub tokens_by_page: ExtractionSignal<Vec<u64>>,
    pub token_accounting_method: Option<String>,
    pub heading_count: ExtractionSignal<u32>,
    pub table_count: ExtractionSignal<u32>,
    pub formula_count: ExtractionSignal<u32>,
    pub figure_count: ExtractionSignal<u32>,
    pub caption_count: ExtractionSignal<u32>,
    pub reference_count: ExtractionSignal<u32>,
    pub plain_text_character_count: ExtractionSignal<u64>,
    pub plain_text_hash: ExtractionSignal<String>,
    pub fallback_used: ExtractionSignal<bool>,
    pub fallback_reason: ExtractionSignal<String>,
    pub alternative_extraction_compared: ExtractionSignal<bool>,
    pub coverage_discrepancy: ExtractionSignal<f64>,
    pub provider_diagnostics: BTreeMap<String, serde_json::Value>,
}

impl ExtractionObservations {
    pub fn unreported(
        provider_profile_id: impl Into<String>,
        artifact_id: impl Into<String>,
        artifact_revision_id: i64,
        input_content_hash: impl Into<String>,
    ) -> Self {
        let provider_profile_id = provider_profile_id.into();
        let unavailable_u32 = || {
            ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id.clone(),
                "provider did not report this signal",
            )
        };
        let unavailable_bool = || {
            ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id.clone(),
                "provider did not report this signal",
            )
        };
        let unavailable_u32_list = || {
            ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id.clone(),
                "provider did not report this signal",
            )
        };
        let unavailable_u64_list = || {
            ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id.clone(),
                "provider did not report this signal",
            )
        };
        let unavailable_string_list = || {
            ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id.clone(),
                "provider did not report this signal",
            )
        };
        Self {
            provider_profile_id: provider_profile_id.clone(),
            canonical_schema_version: CANONICAL_SCHEMA_VERSION.to_owned(),
            artifact_id: artifact_id.into(),
            artifact_revision_id,
            input_content_hash: input_content_hash.into(),
            reported_page_count: unavailable_u32(),
            extracted_page_count: unavailable_u32(),
            pages_with_text: unavailable_u32(),
            empty_page_indices: unavailable_u32_list(),
            suspicious_page_indices: unavailable_u32_list(),
            page_text_density_anomaly_indices: unavailable_u32_list(),
            native_text_presence: unavailable_bool(),
            ocr_used: unavailable_bool(),
            ocr_page_indices: unavailable_u32_list(),
            ocr_mean_confidence: ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id.clone(),
                "provider did not report OCR confidence",
            ),
            encoding_warnings: unavailable_string_list(),
            layout_warnings: unavailable_string_list(),
            structure_retention_warnings: unavailable_string_list(),
            replacement_character_count: ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id.clone(),
                "provider did not report replacement character count",
            ),
            projection_truncated: unavailable_bool(),
            native_to_projection_retention_ratio: ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id.clone(),
                "provider did not report native-to-projection retention",
            ),
            characters_by_page: unavailable_u64_list(),
            tokens_by_page: unavailable_u64_list(),
            token_accounting_method: None,
            heading_count: unavailable_u32(),
            table_count: unavailable_u32(),
            formula_count: unavailable_u32(),
            figure_count: unavailable_u32(),
            caption_count: unavailable_u32(),
            reference_count: unavailable_u32(),
            plain_text_character_count: ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id.clone(),
                "provider did not report this signal",
            ),
            plain_text_hash: ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id.clone(),
                "provider did not report this signal",
            ),
            fallback_used: unavailable_bool(),
            fallback_reason: ExtractionSignal::unavailable(
                SignalAvailability::NotApplicable,
                provider_profile_id.clone(),
                "no fallback decision has been made",
            ),
            alternative_extraction_compared: unavailable_bool(),
            coverage_discrepancy: ExtractionSignal::unavailable(
                SignalAvailability::NotReported,
                provider_profile_id,
                "provider did not report this signal",
            ),
            provider_diagnostics: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ExtractionAttempt {
    pub provider: ExtractionProviderDescriptor,
    pub canonical_document: Option<CanonicalDocumentV1>,
    pub provenance: BTreeMap<String, serde_json::Value>,
    pub observations: ExtractionObservations,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExtractionAttemptOutcome {
    Succeeded,
    Failed,
    Unsupported,
    Unavailable,
}

impl ExtractionAttemptOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExtractionQualityStatus {
    Good,
    Degraded,
    Failed,
}

impl ExtractionQualityStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "good" => Some(Self::Good),
            "degraded" => Some(Self::Degraded),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionQualityReport {
    pub profile_id: String,
    pub status: ExtractionQualityStatus,
    pub usable: bool,
    pub reasons: Vec<String>,
    pub signals: ExtractionObservations,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionQualityProfile {
    pub id: String,
    pub version: String,
    pub require_searchable_text: bool,
    pub flat_pdf_text_is_degraded: bool,
    pub maximum_coverage_discrepancy: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionRoutingProfile {
    pub id: String,
    pub version: String,
    pub provider_order: Vec<String>,
    pub compare_degraded: bool,
    pub allow_usable_degraded: bool,
}

#[derive(Clone, Debug)]
pub struct EvaluatedExtractionAttempt {
    pub attempt: ExtractionAttempt,
    pub quality: ExtractionQualityReport,
    pub outcome: ExtractionAttemptOutcome,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoricalExtractionSelection {
    pub extraction_id: String,
    pub quality_report_id: String,
    pub quality_status: ExtractionQualityStatus,
    pub usable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractionSelectionDecision {
    NewAttempt(usize),
    RetainHistorical,
    None,
}

#[derive(Clone, Debug)]
pub struct RoutedExtraction {
    pub attempts: Vec<EvaluatedExtractionAttempt>,
    pub historical_active: Option<HistoricalExtractionSelection>,
    pub selection: ExtractionSelectionDecision,
    pub selection_reason: String,
    pub fallback_used: bool,
    pub fallback_reason: Option<String>,
}
