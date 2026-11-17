mod config;
mod coordinator;
mod model;
#[cfg(all(
    test,
    feature = "pdf-shadow-pilot",
    target_os = "linux",
    target_arch = "x86_64"
))]
mod observation_tests;
mod sidecar;
mod store;

pub use coordinator::ShadowExtractionCoordinator;
pub use model::{
    ShadowBatchSummary, ShadowPilotDiagnostics, ShadowRunRequest, SHADOW_ROUTING_PROFILE_ID,
};

pub fn load_shadow_diagnostics(
    database: &crate::database::ResearchDatabase,
) -> crate::error::AppResult<ShadowPilotDiagnostics> {
    store::ShadowExtractionStore::new(database.clone()).diagnostics()
}

#[cfg(test)]
pub(crate) fn persist_fixture_shadow_attempt(
    database: crate::database::ResearchDatabase,
    request: &crate::extraction::model::ExtractionRequest,
    attempt: &crate::extraction::model::ExtractionAttempt,
) -> crate::error::AppResult<String> {
    let store = store::ShadowExtractionStore::new(database);
    store.configure(&config::ShadowPilotConfig::disabled())?;
    let (job_id, _) = store.enqueue(
        request,
        model::ShadowRequestKind::Native,
        "test-only shadow isolation fixture",
    )?;
    store.start_job(&job_id)?;
    let quality =
        crate::extraction::quality::ExtractionQualityEvaluator::default_v1().evaluate(attempt);
    let comparison = model::ShadowComparison::default();
    let metrics = model::ShadowRuntimeMetrics::default();
    store.persist_attempt(store::ShadowAttemptRecord {
        job_id: &job_id,
        request,
        provider_role: "native-primary",
        routing_reason: "test-only shadow isolation fixture",
        attempt,
        quality: &quality,
        comparison: &comparison,
        metrics: &metrics,
    })
}
