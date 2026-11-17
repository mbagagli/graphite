use std::future::Future;
use std::pin::Pin;

use crate::error::AppResult;

use super::model::{SourceDelta, SourceDiscoveryRequest, SourceProviderDescriptor};

pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

pub trait SourceProvider: Send + Sync {
    fn descriptor(&self) -> SourceProviderDescriptor;

    fn discover<'a>(&'a self, request: SourceDiscoveryRequest) -> ProviderFuture<'a, SourceDelta>;
}
