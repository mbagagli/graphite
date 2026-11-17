use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentProviderKind {
    #[serde(alias = "graphy-managed")]
    GraphiteManaged,
    OpenaiCompatible,
}

impl AgentProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GraphiteManaged => "graphite-managed",
            Self::OpenaiCompatible => "openai-compatible",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentDetail {
    Concise,
    Detailed,
}

impl AgentDetail {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Concise => "concise",
            Self::Detailed => "detailed",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AgentIntent {
    PaperDiscovery,
    CurrentPaperQa,
    MultiPaperComparison,
    StructuredExtraction,
    CitationStructure,
    LibrarySynthesis,
}

impl AgentIntent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PaperDiscovery => "paper-discovery",
            Self::CurrentPaperQa => "current-paper-qa",
            Self::MultiPaperComparison => "multi-paper-comparison",
            Self::StructuredExtraction => "structured-extraction",
            Self::CitationStructure => "citation-structure",
            Self::LibrarySynthesis => "library-synthesis",
        }
    }
}

/// Choose automatic fallback or an explicit inference device class.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InferenceBackend {
    #[default]
    Auto,
    Cpu,
    Gpu,
}

impl InferenceBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
}

/// Scale the managed context within the selected profile's supported limit.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ContextWindow {
    Low,
    #[default]
    Medium,
    High,
    ExtraHigh,
}

impl ContextWindow {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::ExtraHigh => "extra-high",
        }
    }

    pub fn tokens(self, capacity: u32) -> u32 {
        let quarters = match self {
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
            Self::ExtraHigh => 4,
        };
        (capacity / 4 * quarters).max(2_048).min(capacity)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentContext {
    #[serde(default)]
    pub selected_only: bool,
    #[serde(default)]
    pub lens_only: bool,
    pub current_item_id: Option<String>,
    pub current_document_id: Option<String>,
    pub current_page: Option<u32>,
    #[serde(default)]
    pub selected_item_ids: Vec<String>,
    pub active_collection_id: Option<String>,
    pub active_lens_id: Option<String>,
    #[serde(default)]
    pub active_lens_item_ids: Vec<String>,
    #[serde(default)]
    pub graph_neighborhood_item_ids: Vec<String>,
    #[serde(default)]
    pub removed_context_kinds: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalProviderConfig {
    pub endpoint: String,
    pub model: String,
    pub context_tokens: u32,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRequest {
    pub question: String,
    pub conversation_id: Option<String>,
    pub provider_kind: AgentProviderKind,
    pub model_profile: String,
    #[serde(default)]
    pub inference_backend: InferenceBackend,
    #[serde(default)]
    pub context_window: ContextWindow,
    pub detail: AgentDetail,
    pub context: AgentContext,
    pub external_provider: Option<ExternalProviderConfig>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSettings {
    pub provider_kind: AgentProviderKind,
    pub model_profile: String,
    #[serde(default)]
    pub inference_backend: InferenceBackend,
    #[serde(default)]
    pub context_window: ContextWindow,
    pub external_endpoint: Option<String>,
    pub external_model: Option<String>,
    pub external_context_tokens: Option<u32>,
    pub answer_detail: AgentDetail,
}

/// Describe a GPU that the linked inference runtime initialized.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputeDevice {
    pub index: usize,
    pub name: String,
    pub backend: String,
    pub kind: String,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub cpu_name: String,
    pub compiled_backend: String,
    pub execution_status: String,
    pub gpu_devices: Vec<ComputeDevice>,
    pub graphics_adapters: Vec<String>,
    pub logical_cpus: usize,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub operating_system: String,
    pub architecture: String,
    pub acceleration_backend: String,
    pub acceleration_available: bool,
    pub recommended_profile: String,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProfile {
    pub id: String,
    pub label: String,
    pub model_identifier: String,
    pub model_revision: String,
    pub artifact_filename: String,
    pub artifact_identity: String,
    pub quantization: String,
    pub expected_bytes: u64,
    pub sha256: String,
    pub context_capacity: u32,
    pub license: String,
    pub installed: bool,
    pub status: String,
    pub installed_bytes: u64,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEnvironment {
    pub settings: AgentSettings,
    pub hardware: HardwareProfile,
    pub model_profiles: Vec<ModelProfile>,
    pub runtime_name: String,
    pub runtime_version: String,
    pub trust_statement: String,
    pub tool_budget: u32,
    pub context_budget_tokens: u32,
    pub reranker_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvidence {
    pub source_id: String,
    pub kind: String,
    pub item_id: Option<String>,
    pub item_title: String,
    pub document_id: Option<String>,
    pub document_revision_id: Option<i64>,
    pub chunk_id: Option<String>,
    pub excerpt: Option<String>,
    pub character_start: Option<u64>,
    pub character_end: Option<u64>,
    pub page_start: Option<u32>,
    pub page_end: Option<u32>,
    pub retrieval_modes: Vec<String>,
    pub lexical_rank: Option<u32>,
    pub semantic_rank: Option<u32>,
    pub provider: String,
    pub source_fingerprint: String,
    pub eligible_for_scientific_claim: bool,
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCitation {
    pub source_id: String,
    pub marker: String,
    pub claim_index: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAction {
    pub kind: String,
    pub label: String,
    pub query: Option<String>,
    pub item_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolCall {
    pub tool: String,
    pub purpose: String,
    pub result_count: u32,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTimings {
    pub retrieval_ms: u64,
    pub context_ms: u64,
    pub time_to_first_token_ms: Option<u64>,
    pub generation_ms: u64,
    pub total_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalTrace {
    pub profile: String,
    pub requested_mode: String,
    pub effective_mode: String,
    pub retrieval_path: String,
    pub scope_kind: String,
    pub scope_item_count: u32,
    pub scoped_chunk_limit_per_paper: Option<u32>,
    pub loaded_chunk_count: u32,
    pub highest_loaded_chunk_ordinal: Option<u32>,
    pub search_result_count: u32,
    pub lexical_result_count: u32,
    pub semantic_result_count: u32,
    pub fused_result_count: u32,
    pub evidence_candidate_count: u32,
    pub eligible_passage_count: u32,
    pub distinct_paper_count: u32,
    pub metadata_fallback_count: u32,
    pub fallback_reason: Option<String>,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextAssemblyStats {
    pub profile: String,
    pub token_accounting: String,
    pub budget_tokens: u32,
    pub used_tokens: u32,
    pub unused_tokens: u32,
    pub input_evidence_count: u32,
    pub scope_filtered_count: u32,
    pub exact_duplicates_omitted: u32,
    pub redundancy_policy: String,
    pub redundant_evidence_omitted: u32,
    pub selection_limit_omitted: u32,
    pub candidate_source_count: u32,
    pub included_source_count: u32,
    pub oversized_evidence_omitted: u32,
    pub omitted_for_budget: u32,
    pub stopped_on_non_fitting_source: bool,
    pub first_non_fitting_source_id: Option<String>,
    pub first_non_fitting_source_tokens: Option<u32>,
    pub first_non_fitting_source_oversized: bool,
    pub unevaluated_after_budget_stop: u32,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunResult {
    pub run_id: String,
    pub conversation_id: String,
    pub question: String,
    pub intent: AgentIntent,
    pub answer_markdown: String,
    pub status: String,
    pub provider_label: String,
    pub model_label: String,
    pub context_label: String,
    pub context_tokens: u32,
    pub generation_tokens: u32,
    pub sources: Vec<AgentEvidence>,
    pub citations: Vec<AgentCitation>,
    pub limitations: Vec<String>,
    pub actions: Vec<AgentAction>,
    pub tool_calls: Vec<AgentToolCall>,
    pub prompt_versions: Vec<String>,
    pub timings: AgentTimings,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStreamEvent {
    pub kind: String,
    pub stage: Option<String>,
    pub message: Option<String>,
    pub delta: Option<String>,
    pub result: Option<AgentRunResult>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub turn_count: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTurn {
    pub id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub content: String,
    pub status: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDetail {
    pub summary: ConversationSummary,
    pub turns: Vec<ConversationTurn>,
    pub runs: Vec<AgentRunResult>,
}

#[cfg(test)]
mod tests {
    use super::AgentProviderKind;

    #[test]
    fn accepts_the_previous_managed_provider_identifier() {
        let provider: AgentProviderKind =
            serde_json::from_str("\"graphy-managed\"").expect("legacy provider");

        assert_eq!(provider, AgentProviderKind::GraphiteManaged);
        assert_eq!(
            serde_json::to_string(&provider).expect("current provider"),
            "\"graphite-managed\""
        );
    }

    #[test]
    fn selected_only_is_explicit_and_old_contexts_keep_broader_search() {
        let old: super::AgentContext = serde_json::from_str("{}").expect("old context");
        assert!(!old.selected_only);
        let selected: super::AgentContext =
            serde_json::from_str(r#"{"selectedOnly":true,"selectedItemIds":["paper"]}"#)
                .expect("explicit selection");
        assert!(selected.selected_only);
        assert_eq!(selected.selected_item_ids, ["paper"]);
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInstallEvent {
    pub kind: String,
    pub profile_id: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct GenerationRequest {
    pub system_prompt: String,
    pub user_prompt: String,
    pub max_tokens: u32,
    pub context_tokens: u32,
    pub temperature: f32,
}

#[derive(Clone, Debug)]
pub struct GenerationResult {
    pub text: String,
    pub generated_tokens: u32,
    pub time_to_first_token_ms: Option<u64>,
    pub generation_ms: u64,
}

#[cfg(test)]
mod preference_tests {
    use super::*;

    #[test]
    fn context_presets_are_distinct_bounded_and_medium_by_default() {
        let levels = [
            ContextWindow::Low,
            ContextWindow::Medium,
            ContextWindow::High,
            ContextWindow::ExtraHigh,
        ];
        assert_eq!(
            levels.map(|level| level.tokens(8_192)),
            [2_048, 4_096, 6_144, 8_192]
        );
        assert_eq!(
            levels.map(|level| level.tokens(16_384)),
            [4_096, 8_192, 12_288, 16_384]
        );
        let old = serde_json::json!({
            "providerKind": "graphite-managed", "modelProfile": "lightweight",
            "answerDetail": "concise"
        });
        let settings: AgentSettings = serde_json::from_value(old).unwrap();
        assert_eq!(settings.context_window, ContextWindow::Medium);
        assert_eq!(settings.inference_backend, InferenceBackend::Auto);
        assert!(serde_json::from_value::<ContextWindow>(serde_json::json!("unlimited")).is_err());
        assert!(serde_json::from_value::<InferenceBackend>(serde_json::json!("remote")).is_err());
    }
}
