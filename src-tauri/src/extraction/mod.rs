pub mod coordinator;
pub mod diagnostics;
pub mod model;
pub mod projection;
pub mod provider;
pub mod quality;
pub mod router;
pub mod shadow;
pub mod store;
pub mod zotero_text;

pub use coordinator::{ExtractionCoordinator, DEFAULT_CANONICAL_BACKFILL_BATCH};

pub mod docling;
pub(crate) mod pages;

pub(crate) mod pdf_pages;
