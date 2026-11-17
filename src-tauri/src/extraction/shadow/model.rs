use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::extraction::model::ExtractionRequest;

pub const SHADOW_ROUTING_PROFILE_ID: &str = "pdf-shadow-strategy-v1";
pub const SHADOW_ROUTING_PROFILE_VERSION: &str = "1.0.0";
pub const SHADOW_BATCH_LIMIT: u32 = 10;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ShadowRequestKind {
    Native,
    OcrFallback,
    StructureDemand,
}

impl ShadowRequestKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::OcrFallback => "ocr-fallback",
            Self::StructureDemand => "structure-demand",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ShadowArtifactInput {
    pub request: ExtractionRequest,
    pub local_path: PathBuf,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ShadowComparison {
    pub zotero_text_available: bool,
    pub candidate_to_zotero_character_ratio: Option<f64>,
    pub reported_page_count_difference: Option<i64>,
    pub tail_token_overlap: Option<f64>,
    pub truncation_suspected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShadowRuntimeMetrics {
    pub duration_ms: Option<u64>,
    pub peak_rss_bytes: Option<u64>,
    pub peak_thread_count: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct ShadowCancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl ShadowCancellationToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShadowBatchSummary {
    pub requested: u32,
    pub queued: u32,
    pub completed: u32,
    pub failed: u32,
    pub cancelled: u32,
    pub unavailable: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShadowRunRequest {
    pub document_ids: Vec<String>,
    pub request_structure: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShadowPilotDiagnostics {
    pub enabled: bool,
    pub availability: String,
    pub routing_profile: String,
    pub pending: u64,
    pub running: u64,
    pub completed: u64,
    pub failed: u64,
    pub cancelled: u64,
    pub unavailable: u64,
    pub good: u64,
    pub degraded: u64,
    pub quality_failed: u64,
    pub providers: Vec<String>,
    pub last_provider_profile: Option<String>,
    pub last_routing_reason: Option<String>,
    pub last_duration_ms: Option<u64>,
    pub last_peak_rss_bytes: Option<u64>,
    pub fallback_routes: u64,
    pub comparison_discrepancies: u64,
}

impl ShadowPilotDiagnostics {
    pub fn disabled(availability: impl Into<String>) -> Self {
        Self {
            availability: availability.into(),
            routing_profile: SHADOW_ROUTING_PROFILE_ID.to_owned(),
            ..Self::default()
        }
    }
}
