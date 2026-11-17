use std::sync::Arc;

use super::model::{
    EvaluatedExtractionAttempt, ExtractionAttempt, ExtractionAttemptOutcome,
    ExtractionQualityStatus, ExtractionRequest, ExtractionRoutingProfile,
    ExtractionSelectionDecision, ExtractionSignal, ExtractionSupport,
    HistoricalExtractionSelection, RoutedExtraction,
};
use super::provider::DocumentExtractionProvider;
use super::quality::ExtractionQualityEvaluator;

#[derive(Clone)]
pub struct ExtractorRouter {
    providers: Vec<Arc<dyn DocumentExtractionProvider>>,
    routing_profile: ExtractionRoutingProfile,
    quality_evaluator: ExtractionQualityEvaluator,
}

impl ExtractorRouter {
    pub fn new(
        providers: Vec<Arc<dyn DocumentExtractionProvider>>,
        routing_profile: ExtractionRoutingProfile,
        quality_evaluator: ExtractionQualityEvaluator,
    ) -> Self {
        Self {
            providers,
            routing_profile,
            quality_evaluator,
        }
    }

    pub fn default_with(providers: Vec<Arc<dyn DocumentExtractionProvider>>) -> Self {
        let provider_order = providers
            .iter()
            .map(|provider| provider.descriptor().profile_id)
            .collect();
        Self::new(
            providers,
            ExtractionRoutingProfile {
                id: "source-text-fallback-v1".to_owned(),
                version: "1.0.0".to_owned(),
                provider_order,
                compare_degraded: true,
                allow_usable_degraded: true,
            },
            ExtractionQualityEvaluator::default_v1(),
        )
    }

    pub fn routing_profile(&self) -> &ExtractionRoutingProfile {
        &self.routing_profile
    }

    pub fn quality_evaluator(&self) -> &ExtractionQualityEvaluator {
        &self.quality_evaluator
    }

    pub async fn route(&self, request: ExtractionRequest) -> RoutedExtraction {
        self.route_with_historical(request, None).await
    }

    pub async fn route_with_historical(
        &self,
        request: ExtractionRequest,
        historical_active: Option<HistoricalExtractionSelection>,
    ) -> RoutedExtraction {
        let mut attempts = Vec::new();
        for provider in self.ordered_providers() {
            let descriptor = provider.descriptor();
            let support = provider.supports(&request.input);
            let (attempt, outcome) = match support {
                ExtractionSupport::Unsupported => (
                    unsupported_attempt(
                        descriptor,
                        &request,
                        "provider does not support the artifact media type and available input forms",
                    ),
                    ExtractionAttemptOutcome::Unsupported,
                ),
                ExtractionSupport::Unavailable => (
                    unsupported_attempt(
                        descriptor,
                        &request,
                        "provider is unavailable in the current configuration",
                    ),
                    ExtractionAttemptOutcome::Unavailable,
                ),
                ExtractionSupport::Supported => match provider.extract(request.clone()).await {
                    Ok(attempt) => {
                        let outcome = if attempt.error.is_some() {
                            ExtractionAttemptOutcome::Failed
                        } else {
                            ExtractionAttemptOutcome::Succeeded
                        };
                        (attempt, outcome)
                    }
                    Err(error) => (ExtractionAttempt {
                    provider: descriptor.clone(),
                    canonical_document: None,
                    provenance: Default::default(),
                    observations: super::model::ExtractionObservations::unreported(
                        descriptor.profile_id,
                        request.artifact_id.clone(),
                        request.artifact_revision_id,
                        request.input_content_hash.clone(),
                    ),
                    warnings: Vec::new(),
                    error: Some(error.to_string()),
                    }, ExtractionAttemptOutcome::Failed),
                },
            };
            let quality = self.quality_evaluator.evaluate(&attempt);
            let should_continue = match quality.status {
                ExtractionQualityStatus::Good => false,
                ExtractionQualityStatus::Degraded => self.routing_profile.compare_degraded,
                ExtractionQualityStatus::Failed => true,
            };
            attempts.push(EvaluatedExtractionAttempt {
                attempt,
                quality,
                outcome,
            });
            if !should_continue {
                break;
            }
        }
        let current_selection =
            select_attempt(&attempts, self.routing_profile.allow_usable_degraded);
        let (selection, selection_reason) =
            select_with_history(&attempts, current_selection, historical_active.as_ref());
        let fallback_used = match selection {
            ExtractionSelectionDecision::NewAttempt(index) => index > 0,
            ExtractionSelectionDecision::RetainHistorical => true,
            ExtractionSelectionDecision::None => false,
        };
        let fallback_reason = fallback_used.then(|| selection_reason.clone());
        let compared = attempts.len() > 1;
        let coverage_discrepancy = extraction_coverage_discrepancy(&attempts);
        if let ExtractionSelectionDecision::NewAttempt(index) = selection {
            let selected = &mut attempts[index];
            selected.attempt.observations.fallback_used =
                ExtractionSignal::observed(fallback_used, "extractor-router");
            selected.attempt.observations.fallback_reason = if let Some(reason) = &fallback_reason {
                ExtractionSignal::observed(reason.clone(), "extractor-router")
            } else {
                ExtractionSignal::unavailable(
                    super::model::SignalAvailability::NotApplicable,
                    "extractor-router",
                    "the selected provider was the primary candidate",
                )
            };
            selected
                .attempt
                .observations
                .alternative_extraction_compared =
                ExtractionSignal::observed(compared, "extractor-router");
            selected.attempt.observations.coverage_discrepancy =
                if let Some(discrepancy) = coverage_discrepancy {
                    ExtractionSignal::observed(discrepancy, "extractor-router")
                } else {
                    ExtractionSignal::unavailable(
                        super::model::SignalAvailability::NotApplicable,
                        "extractor-router",
                        "no alternative usable extraction was compared",
                    )
                };
            selected.quality = self.quality_evaluator.evaluate(&selected.attempt);
        }
        RoutedExtraction {
            attempts,
            historical_active,
            selection,
            selection_reason,
            fallback_used,
            fallback_reason,
        }
    }

    fn ordered_providers(&self) -> Vec<&Arc<dyn DocumentExtractionProvider>> {
        let mut ordered = Vec::new();
        for profile_id in &self.routing_profile.provider_order {
            if let Some(provider) = self
                .providers
                .iter()
                .find(|provider| provider.descriptor().profile_id == *profile_id)
            {
                ordered.push(provider);
            }
        }
        for provider in &self.providers {
            if !ordered.iter().any(|candidate| {
                candidate.descriptor().profile_id == provider.descriptor().profile_id
            }) {
                ordered.push(provider);
            }
        }
        ordered
    }
}

fn unsupported_attempt(
    descriptor: super::model::ExtractionProviderDescriptor,
    request: &ExtractionRequest,
    reason: &str,
) -> ExtractionAttempt {
    ExtractionAttempt {
        observations: super::model::ExtractionObservations::unreported(
            descriptor.profile_id.clone(),
            request.artifact_id.clone(),
            request.artifact_revision_id,
            request.input_content_hash.clone(),
        ),
        provider: descriptor,
        canonical_document: None,
        provenance: Default::default(),
        warnings: Vec::new(),
        error: Some(format!("{reason}: {}", request.input.media_type)),
    }
}

fn select_attempt(attempts: &[EvaluatedExtractionAttempt], allow_degraded: bool) -> Option<usize> {
    attempts
        .iter()
        .position(|attempt| attempt.quality.status == ExtractionQualityStatus::Good)
        .or_else(|| {
            allow_degraded.then(|| {
                attempts.iter().position(|attempt| {
                    attempt.quality.status == ExtractionQualityStatus::Degraded
                        && attempt.quality.usable
                })
            })?
        })
}

fn select_with_history(
    attempts: &[EvaluatedExtractionAttempt],
    current_selection: Option<usize>,
    historical: Option<&HistoricalExtractionSelection>,
) -> (ExtractionSelectionDecision, String) {
    let Some(historical) = historical.filter(|selection| selection.usable) else {
        return match current_selection {
            Some(index) => (
                ExtractionSelectionDecision::NewAttempt(index),
                format!(
                    "selected new {} extraction because no usable historical extraction exists",
                    attempts[index].quality.status.as_str()
                ),
            ),
            None => (
                ExtractionSelectionDecision::None,
                "no historical or new usable extraction exists".to_owned(),
            ),
        };
    };
    let Some(index) = current_selection else {
        return (
            ExtractionSelectionDecision::RetainHistorical,
            format!(
                "retained historical {} extraction because the current route produced no usable extraction",
                historical.quality_status.as_str()
            ),
        );
    };
    let current_status = &attempts[index].quality.status;
    match (&historical.quality_status, current_status) {
        (ExtractionQualityStatus::Good, ExtractionQualityStatus::Good) => (
            ExtractionSelectionDecision::RetainHistorical,
            "retained historical good extraction over an equal-quality new extraction".to_owned(),
        ),
        (ExtractionQualityStatus::Good, _) => (
            ExtractionSelectionDecision::RetainHistorical,
            format!(
                "retained historical good extraction over new {} extraction",
                current_status.as_str()
            ),
        ),
        (ExtractionQualityStatus::Degraded, ExtractionQualityStatus::Good)
        | (ExtractionQualityStatus::Failed, ExtractionQualityStatus::Good) => (
            ExtractionSelectionDecision::NewAttempt(index),
            format!(
                "selected new good extraction over historical {} extraction",
                historical.quality_status.as_str()
            ),
        ),
        (ExtractionQualityStatus::Degraded, ExtractionQualityStatus::Degraded) => (
            ExtractionSelectionDecision::RetainHistorical,
            "retained historical degraded extraction over an equal-quality new extraction"
                .to_owned(),
        ),
        (ExtractionQualityStatus::Failed, ExtractionQualityStatus::Degraded) => (
            ExtractionSelectionDecision::NewAttempt(index),
            "selected new degraded extraction because the historical extraction is failed"
                .to_owned(),
        ),
        (_, ExtractionQualityStatus::Failed) => (
            ExtractionSelectionDecision::RetainHistorical,
            format!(
                "retained historical {} extraction over new failed extraction",
                historical.quality_status.as_str()
            ),
        ),
    }
}

fn extraction_coverage_discrepancy(attempts: &[EvaluatedExtractionAttempt]) -> Option<f64> {
    let counts = attempts
        .iter()
        .filter(|attempt| attempt.quality.usable)
        .filter_map(|attempt| {
            attempt
                .attempt
                .observations
                .plain_text_character_count
                .value
        })
        .collect::<Vec<_>>();
    if counts.len() < 2 {
        return None;
    }
    let minimum = *counts.iter().min()? as f64;
    let maximum = *counts.iter().max()? as f64;
    if maximum == 0.0 {
        Some(0.0)
    } else {
        Some((maximum - minimum) / maximum)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::database::content_hash;
    use crate::error::AppResult;
    use crate::extraction::provider::DocumentExtractionProvider;
    use crate::knowledge::source::ProviderFuture;

    use super::*;
    use crate::extraction::model::{
        ExtractionInputDescriptor, ExtractionProviderDescriptor, ExtractionSupport,
    };
    use crate::extraction::quality::tests::fixture_attempt;

    #[derive(Clone)]
    struct FakeProvider {
        attempt: ExtractionAttempt,
        transport_error: Option<String>,
        support: ExtractionSupport,
    }

    impl DocumentExtractionProvider for FakeProvider {
        fn descriptor(&self) -> ExtractionProviderDescriptor {
            self.attempt.provider.clone()
        }

        fn supports(&self, _input: &ExtractionInputDescriptor) -> ExtractionSupport {
            self.support.clone()
        }

        fn extract<'a>(
            &'a self,
            _request: ExtractionRequest,
        ) -> ProviderFuture<'a, ExtractionAttempt> {
            Box::pin(async move {
                if let Some(error) = &self.transport_error {
                    return Err(crate::error::AppError::ApplicationData(error.clone()));
                }
                Ok(self.attempt.clone())
            })
        }
    }

    #[tokio::test]
    async fn falls_back_after_primary_failure() -> AppResult<()> {
        let mut failed = fixture_attempt(true, "primary");
        failed.canonical_document = None;
        failed.error = Some("primary failed".to_owned());
        let mut fallback = fixture_attempt(true, "fallback result");
        fallback.provider.profile_id = "fallback-fixture-v1".to_owned();
        if let Some(document) = &mut fallback.canonical_document {
            document.extraction_profile_id = "fallback-fixture-v1".to_owned();
        }
        let router = router(vec![failed, fallback]);
        let routed = router.route(request()).await;
        assert_eq!(routed.selection, ExtractionSelectionDecision::NewAttempt(1));
        assert!(routed.fallback_used);
        assert_eq!(
            routed.attempts[1].quality.status,
            ExtractionQualityStatus::Good
        );
        Ok(())
    }

    #[tokio::test]
    async fn selects_better_fallback_after_degraded_primary() {
        let primary = fixture_attempt(false, "flat primary");
        let fallback = fixture_attempt(true, "structured fallback");
        let routed = router(vec![primary, fallback]).route(request()).await;
        assert_eq!(routed.selection, ExtractionSelectionDecision::NewAttempt(1));
        assert!(routed.fallback_used);
    }

    #[tokio::test]
    async fn selects_first_of_two_degraded_attempts_deterministically() {
        let first = fixture_attempt(false, "first degraded");
        let second = fixture_attempt(false, "second degraded with more text");
        let routed = router(vec![first, second]).route(request()).await;
        assert_eq!(routed.selection, ExtractionSelectionDecision::NewAttempt(0));
        assert!(!routed.fallback_used);
        assert_eq!(routed.attempts.len(), 2);
    }

    #[tokio::test]
    async fn historical_good_is_never_replaced_by_degraded_or_failed_output() {
        let degraded = router(vec![fixture_attempt(false, "new degraded")])
            .route_with_historical(request(), Some(historical(ExtractionQualityStatus::Good)))
            .await;
        assert_eq!(
            degraded.selection,
            ExtractionSelectionDecision::RetainHistorical
        );

        let mut failed_attempt = fixture_attempt(true, "failed");
        failed_attempt.canonical_document = None;
        failed_attempt.error = Some("replacement failed".to_owned());
        let failed = router(vec![failed_attempt])
            .route_with_historical(
                request(),
                Some(historical(ExtractionQualityStatus::Degraded)),
            )
            .await;
        assert_eq!(
            failed.selection,
            ExtractionSelectionDecision::RetainHistorical
        );
        assert!(degraded.selection_reason.contains("historical good"));

        let mut failed_attempt = fixture_attempt(true, "failed");
        failed_attempt.canonical_document = None;
        failed_attempt.error = Some("new profile failed".to_owned());
        let failed = router(vec![failed_attempt])
            .route_with_historical(request(), Some(historical(ExtractionQualityStatus::Good)))
            .await;
        assert_eq!(
            failed.selection,
            ExtractionSelectionDecision::RetainHistorical
        );
    }

    #[tokio::test]
    async fn historical_degraded_switches_only_for_good_new_output() {
        let good = router(vec![fixture_attempt(true, "new good")])
            .route_with_historical(
                request(),
                Some(historical(ExtractionQualityStatus::Degraded)),
            )
            .await;
        assert_eq!(good.selection, ExtractionSelectionDecision::NewAttempt(0));

        let degraded = router(vec![fixture_attempt(false, "new degraded")])
            .route_with_historical(
                request(),
                Some(historical(ExtractionQualityStatus::Degraded)),
            )
            .await;
        assert_eq!(
            degraded.selection,
            ExtractionSelectionDecision::RetainHistorical
        );
    }

    #[tokio::test]
    async fn no_history_selects_good_or_allowed_degraded_deterministically() {
        let good = router(vec![fixture_attempt(true, "good")])
            .route(request())
            .await;
        assert_eq!(good.selection, ExtractionSelectionDecision::NewAttempt(0));
        let degraded = router(vec![fixture_attempt(false, "degraded")])
            .route(request())
            .await;
        assert_eq!(
            degraded.selection,
            ExtractionSelectionDecision::NewAttempt(0)
        );
    }

    #[tokio::test]
    async fn unsupported_outcome_is_explicit_and_not_selected() {
        let attempt = fixture_attempt(true, "unused");
        let provider_order = vec![attempt.provider.profile_id.clone()];
        let provider = Arc::new(FakeProvider {
            attempt,
            transport_error: None,
            support: ExtractionSupport::Unsupported,
        }) as Arc<dyn DocumentExtractionProvider>;
        let router = ExtractorRouter::new(
            vec![provider],
            ExtractionRoutingProfile {
                id: "unsupported-routing-v1".to_owned(),
                version: "1.0.0".to_owned(),
                provider_order,
                compare_degraded: true,
                allow_usable_degraded: true,
            },
            ExtractionQualityEvaluator::default_v1(),
        );
        let routed = router.route(request()).await;
        assert_eq!(routed.selection, ExtractionSelectionDecision::None);
        assert_eq!(routed.attempts.len(), 1);
        assert_eq!(
            routed.attempts[0].outcome,
            ExtractionAttemptOutcome::Unsupported
        );
        assert!(routed.attempts[0]
            .attempt
            .error
            .as_deref()
            .is_some_and(|reason| reason.contains("does not support")));
    }

    fn router(attempts: Vec<ExtractionAttempt>) -> ExtractorRouter {
        let provider_order = attempts
            .iter()
            .map(|attempt| attempt.provider.profile_id.clone())
            .collect();
        let providers = attempts
            .into_iter()
            .map(|attempt| {
                Arc::new(FakeProvider {
                    attempt,
                    transport_error: None,
                    support: ExtractionSupport::Supported,
                }) as Arc<dyn DocumentExtractionProvider>
            })
            .collect();
        ExtractorRouter::new(
            providers,
            ExtractionRoutingProfile {
                id: "test-routing-v1".to_owned(),
                version: "1.0.0".to_owned(),
                provider_order,
                compare_degraded: true,
                allow_usable_degraded: true,
            },
            ExtractionQualityEvaluator::default_v1(),
        )
    }

    fn request() -> ExtractionRequest {
        ExtractionRequest {
            artifact_id: "artifact-1".to_owned(),
            artifact_revision_id: 1,
            input_content_hash: content_hash("input"),
            input: ExtractionInputDescriptor {
                media_type: "application/pdf".to_owned(),
                has_local_file: true,
                has_bounded_reader: true,
                has_source_provided_text: true,
            },
            source_text: None,
        }
    }

    fn historical(status: ExtractionQualityStatus) -> HistoricalExtractionSelection {
        HistoricalExtractionSelection {
            extraction_id: "historical-extraction".to_owned(),
            quality_report_id: "historical-report".to_owned(),
            usable: status != ExtractionQualityStatus::Failed,
            quality_status: status,
        }
    }
}
