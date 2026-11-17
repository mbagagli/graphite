use std::env;
use std::path::PathBuf;
use std::time::Duration;

use super::model::SHADOW_BATCH_LIMIT;

const ENABLE_ENV: &str = "GRAPHITE_PDF_SHADOW_ENABLED";
const XBERG_BIN_ENV: &str = "GRAPHITE_PDF_SHADOW_XBERG_BIN";
const PDF_INSPECTOR_BIN_ENV: &str = "GRAPHITE_PDF_SHADOW_PDF_INSPECTOR_BIN";
const DOCLING_BIN_ENV: &str = "GRAPHITE_PDF_SHADOW_DOCLING_BIN";
const ASSET_ROOT_ENV: &str = "GRAPHITE_PDF_SHADOW_ASSET_ROOT";
const CONCURRENCY_ENV: &str = "GRAPHITE_PDF_SHADOW_CONCURRENCY";

#[derive(Clone, Debug)]
pub struct ShadowPilotConfig {
    pub enabled: bool,
    pub availability: String,
    pub xberg_binary: Option<PathBuf>,
    pub pdf_inspector_binary: Option<PathBuf>,
    pub docling_binary: Option<PathBuf>,
    pub asset_root: Option<PathBuf>,
    pub max_concurrency: usize,
    pub batch_limit: u32,
    pub native_timeout: Duration,
    pub ocr_timeout: Duration,
    pub structure_timeout: Duration,
}

impl ShadowPilotConfig {
    pub fn from_environment() -> Self {
        let requested = env::var(ENABLE_ENV)
            .ok()
            .is_some_and(|value| matches!(value.as_str(), "1" | "true"));
        let compiled = cfg!(feature = "pdf-shadow-pilot");
        let supported = cfg!(all(target_os = "linux", target_arch = "x86_64"));
        let enabled = requested && compiled && supported;
        let availability = if !requested {
            "disabled".to_owned()
        } else if !compiled {
            "unavailable: Graphite was built without pdf-shadow-pilot".to_owned()
        } else if !supported {
            "unavailable: Milestone 6-A supports Linux x64 only".to_owned()
        } else {
            "available".to_owned()
        };
        Self {
            enabled,
            availability,
            xberg_binary: env::var_os(XBERG_BIN_ENV).map(PathBuf::from),
            pdf_inspector_binary: env::var_os(PDF_INSPECTOR_BIN_ENV).map(PathBuf::from),
            docling_binary: env::var_os(DOCLING_BIN_ENV).map(PathBuf::from),
            asset_root: env::var_os(ASSET_ROOT_ENV).map(PathBuf::from),
            max_concurrency: env::var(CONCURRENCY_ENV)
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 2),
            batch_limit: SHADOW_BATCH_LIMIT,
            native_timeout: Duration::from_secs(90),
            ocr_timeout: Duration::from_secs(300),
            structure_timeout: Duration::from_secs(300),
        }
    }

    #[cfg(test)]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            availability: "disabled".to_owned(),
            xberg_binary: None,
            pdf_inspector_binary: None,
            docling_binary: None,
            asset_root: None,
            max_concurrency: 1,
            batch_limit: SHADOW_BATCH_LIMIT,
            native_timeout: Duration::from_secs(1),
            ocr_timeout: Duration::from_secs(1),
            structure_timeout: Duration::from_secs(1),
        }
    }
}
