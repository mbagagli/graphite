use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Zotero is not reachable at 127.0.0.1:23119. Start Zotero and enable its local API.")]
    ZoteroUnavailable,
    #[error("Zotero local API access is disabled. Enable 'Allow other applications on this computer to communicate with Zotero' in Zotero advanced settings.")]
    ZoteroDisabled,
    #[error("The Zotero source changed during synchronization. The cache was not modified.")]
    SourceChanged,
    #[error("Zotero changed while Graphite was synchronizing. Retrying is required.")]
    ConcurrentChange,
    #[error("This Zotero instance does not expose Zotero-Server-ID. Graphite requires modern Zotero for safe cache partitioning.")]
    MissingServerIdentity,
    #[error("Zotero returned an unsupported response: {0}")]
    InvalidZoteroResponse(String),
    #[error("Zotero does not currently expose the requested local resource: {0}")]
    ZoteroResourceUnavailable(String),
    #[error("The requested document cannot be opened safely: {0}")]
    DocumentUnavailable(String),
    #[error("The local Zotero request failed: {0}")]
    Http(String),
    #[error("Graphite's application database failed: {0}")]
    Database(String),
    #[error("Graphite's application data directory is unavailable: {0}")]
    ApplicationData(String),
    #[error("Local PDF extraction: {0}")]
    Extraction(String),
    #[error("Graphite's local semantic subsystem failed: {0}")]
    Semantic(String),
    #[error("Graphite's local research agent failed: {0}")]
    Agent(String),
    #[error("Graphite's local model runtime failed: {0}")]
    LocalModel(String),
}

impl From<rusqlite::Error> for AppError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Database(value.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        if value.is_connect() || value.is_timeout() {
            Self::ZoteroUnavailable
        } else {
            Self::Http(value.to_string())
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self::ApplicationData(format!("Citation provenance serialization failed: {value}"))
    }
}

pub type AppResult<T> = Result<T, AppError>;
