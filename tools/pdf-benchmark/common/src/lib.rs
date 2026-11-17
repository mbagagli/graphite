pub mod canonical;
pub mod fixtures;
pub mod metrics;
pub mod model;
pub mod protocol;
pub mod validation;

pub use canonical::{
    canonical_hash, canonical_unstructured, render_plain_text, validate_canonical_document,
};
pub use metrics::score_extraction;
pub use model::*;
pub use protocol::{run_adapter, Adapter, ProtocolRequest, ProtocolResponse};
pub use validation::{
    content_hash, read_json, validate_aggregate_privacy, validate_anchor_set, validate_extraction,
    validate_manifest, validate_operational_privacy, validate_scoring_anchors, write_json_atomic,
};
