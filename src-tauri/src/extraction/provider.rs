use crate::knowledge::source::ProviderFuture;

use super::model::{
    ExtractionAttempt, ExtractionInputDescriptor, ExtractionProviderDescriptor, ExtractionRequest,
    ExtractionSupport,
};

pub trait DocumentExtractionProvider: Send + Sync {
    fn descriptor(&self) -> ExtractionProviderDescriptor;

    fn supports(&self, input: &ExtractionInputDescriptor) -> ExtractionSupport;

    fn extract<'a>(&'a self, request: ExtractionRequest) -> ProviderFuture<'a, ExtractionAttempt>;
}
