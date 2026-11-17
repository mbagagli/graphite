use std::collections::BTreeSet;
use std::sync::{Arc, Mutex as StdMutex};

use futures_util::stream::{self, StreamExt};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::database::ResearchDatabase;
use crate::error::{AppError, AppResult};
use crate::extraction::model::{ExtractionAttempt, ExtractionSignal, ExtractionSupport};
use crate::extraction::projection::render_plain_text;
use crate::extraction::provider::DocumentExtractionProvider;
use crate::extraction::quality::ExtractionQualityEvaluator;

use super::config::ShadowPilotConfig;
use super::model::{
    ShadowArtifactInput, ShadowBatchSummary, ShadowCancellationToken, ShadowComparison,
    ShadowPilotDiagnostics, ShadowRequestKind, ShadowRunRequest, ShadowRuntimeMetrics,
};
use super::sidecar::ShadowSidecarProvider;
use super::store::{ShadowAttemptRecord, ShadowExtractionStore};

#[derive(Clone)]
pub struct ShadowExtractionCoordinator {
    config: ShadowPilotConfig,
    store: ShadowExtractionStore,
    quality: ExtractionQualityEvaluator,
    batch_guard: Arc<Mutex<()>>,
    active_cancellation: Arc<StdMutex<Option<ShadowCancellationToken>>>,
}

impl ShadowExtractionCoordinator {
    pub fn new(database: ResearchDatabase) -> AppResult<Self> {
        let config = ShadowPilotConfig::from_environment();
        let store = ShadowExtractionStore::new(database);
        store.configure(&config)?;
        store.recover_interrupted()?;
        Ok(Self {
            config,
            store,
            quality: ExtractionQualityEvaluator::default_v1(),
            batch_guard: Arc::new(Mutex::new(())),
            active_cancellation: Arc::new(StdMutex::new(None)),
        })
    }

    #[cfg(test)]
    pub fn with_config(database: ResearchDatabase, config: ShadowPilotConfig) -> AppResult<Self> {
        let store = ShadowExtractionStore::new(database);
        store.configure(&config)?;
        Ok(Self {
            config,
            store,
            quality: ExtractionQualityEvaluator::default_v1(),
            batch_guard: Arc::new(Mutex::new(())),
            active_cancellation: Arc::new(StdMutex::new(None)),
        })
    }

    pub async fn run_selected(&self, request: ShadowRunRequest) -> AppResult<ShadowBatchSummary> {
        if !self.config.enabled {
            return Err(AppError::ApplicationData(format!(
                "PDF shadow pilot is {}",
                self.config.availability
            )));
        }
        let document_ids = deduplicate_document_ids(request.document_ids);
        if document_ids.is_empty() {
            return Err(AppError::ApplicationData(
                "select at least one PDF for the shadow pilot".to_owned(),
            ));
        }
        if document_ids.len() > self.config.batch_limit as usize {
            return Err(AppError::ApplicationData(format!(
                "the PDF shadow pilot accepts at most {} selected documents per batch",
                self.config.batch_limit
            )));
        }
        let _guard = self.batch_guard.lock().await;
        let cancellation = ShadowCancellationToken::default();
        *self.active_cancellation.lock().map_err(|_| {
            AppError::ApplicationData("shadow cancellation state is unavailable".to_owned())
        })? = Some(cancellation.clone());
        let requested = document_ids.len() as u32;
        let summaries = stream::iter(document_ids)
            .map(|document_id| {
                let cancellation = cancellation.clone();
                async move {
                    self.run_document(&document_id, request.request_structure, cancellation)
                        .await
                }
            })
            .buffer_unordered(self.config.max_concurrency)
            .collect::<Vec<_>>()
            .await;
        self.active_cancellation
            .lock()
            .map_err(|_| {
                AppError::ApplicationData("shadow cancellation state is unavailable".to_owned())
            })?
            .take();
        let mut summary = ShadowBatchSummary {
            requested,
            ..ShadowBatchSummary::default()
        };
        for result in summaries {
            let current = result?;
            summary.queued += current.queued;
            summary.completed += current.completed;
            summary.failed += current.failed;
            summary.cancelled += current.cancelled;
            summary.unavailable += current.unavailable;
        }
        Ok(summary)
    }

    pub fn cancel(&self) -> AppResult<u32> {
        if let Some(cancellation) = self
            .active_cancellation
            .lock()
            .map_err(|_| {
                AppError::ApplicationData("shadow cancellation state is unavailable".to_owned())
            })?
            .as_ref()
        {
            cancellation.cancel();
        }
        self.store.request_cancellation()
    }

    pub fn diagnostics(&self) -> AppResult<ShadowPilotDiagnostics> {
        self.store.diagnostics()
    }

    async fn run_document(
        &self,
        document_id: &str,
        request_structure: bool,
        cancellation: ShadowCancellationToken,
    ) -> AppResult<ShadowBatchSummary> {
        let Some(input) = self.store.artifact_input(document_id)? else {
            return Ok(ShadowBatchSummary {
                requested: 1,
                unavailable: 1,
                ..ShadowBatchSummary::default()
            });
        };
        if input.request.input.media_type != "application/pdf" {
            return Ok(ShadowBatchSummary {
                requested: 1,
                unavailable: 1,
                ..ShadowBatchSummary::default()
            });
        }
        let mut summary = ShadowBatchSummary::default();
        let native_provider = ShadowSidecarProvider::xberg_native(
            self.config.xberg_binary.clone(),
            input.local_path.clone(),
            self.config.native_timeout,
            cancellation.clone(),
        );
        let native = self
            .run_stage(
                &input,
                ShadowRequestKind::Native,
                "ordinary PDF native shadow primary",
                native_provider,
                &mut summary,
            )
            .await?;
        if cancellation.is_cancelled() {
            return Ok(summary);
        }
        if let Some(reason) = native.as_ref().and_then(ocr_fallback_reason) {
            let provider = ShadowSidecarProvider::pdf_inspector_ocr(
                self.config.pdf_inspector_binary.clone(),
                self.config.asset_root.clone(),
                input.local_path.clone(),
                self.config.ocr_timeout,
                cancellation.clone(),
            );
            self.run_stage(
                &input,
                ShadowRequestKind::OcrFallback,
                &reason,
                provider,
                &mut summary,
            )
            .await?;
        }
        if request_structure && !cancellation.is_cancelled() {
            let provider = ShadowSidecarProvider::docling_structure(
                self.config.docling_binary.clone(),
                self.config.asset_root.clone(),
                input.local_path.clone(),
                self.config.structure_timeout,
                cancellation,
            );
            self.run_stage(
                &input,
                ShadowRequestKind::StructureDemand,
                "explicit diagnostic structure-demand request",
                provider,
                &mut summary,
            )
            .await?;
        }
        Ok(summary)
    }

    async fn run_stage(
        &self,
        input: &ShadowArtifactInput,
        kind: ShadowRequestKind,
        routing_reason: &str,
        provider: ShadowSidecarProvider,
        summary: &mut ShadowBatchSummary,
    ) -> AppResult<Option<ExtractionAttempt>> {
        let (job_id, queued) = self.store.enqueue(&input.request, kind, routing_reason)?;
        if !queued || !self.store.start_job(&job_id)? {
            return Ok(None);
        }
        summary.queued += 1;
        let attempt = match provider.supports(&input.request.input) {
            ExtractionSupport::Supported | ExtractionSupport::Unavailable => {
                provider.extract(input.request.clone()).await?
            }
            ExtractionSupport::Unsupported => unavailable_attempt(
                &input.request,
                provider.descriptor(),
                "unsupported",
                "the shadow provider does not support this artifact input",
            ),
        };
        let (attempt, comparison) = compare_with_zotero(attempt, &input.request);
        let quality = self.quality.evaluate(&attempt);
        let metrics = metrics_from_attempt(&attempt);
        self.store.persist_attempt(ShadowAttemptRecord {
            job_id: &job_id,
            request: &input.request,
            provider_role: provider.role().as_str(),
            routing_reason,
            attempt: &attempt,
            quality: &quality,
            comparison: &comparison,
            metrics: &metrics,
        })?;
        match terminal_outcome(&attempt) {
            "succeeded" | "partial" | "suspicious" | "empty-success" => summary.completed += 1,
            "cancelled" => summary.cancelled += 1,
            "unavailable" | "unsupported" => summary.unavailable += 1,
            _ => summary.failed += 1,
        }
        Ok(Some(attempt))
    }
}

fn deduplicate_document_ids(document_ids: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    document_ids
        .into_iter()
        .filter(|document_id| !document_id.trim().is_empty())
        .filter(|document_id| seen.insert(document_id.clone()))
        .collect()
}

fn compare_with_zotero(
    mut attempt: ExtractionAttempt,
    request: &crate::extraction::model::ExtractionRequest,
) -> (ExtractionAttempt, ShadowComparison) {
    let Some(source) = request
        .source_text
        .as_ref()
        .filter(|source| source.provider == "zotero-indexed")
    else {
        return (attempt, ShadowComparison::default());
    };
    let candidate_text = attempt
        .canonical_document
        .as_ref()
        .map(render_plain_text)
        .unwrap_or_default();
    let candidate_characters = candidate_text.chars().count() as f64;
    let zotero_characters = source.content.chars().count() as f64;
    let ratio = (zotero_characters > 0.0).then_some(candidate_characters / zotero_characters);
    let page_difference = attempt
        .observations
        .reported_page_count
        .value
        .zip(source.total_pages)
        .map(|(candidate, zotero)| candidate as i64 - zotero as i64);
    let tail_overlap = (!candidate_text.is_empty() && !source.content.is_empty())
        .then(|| tail_token_overlap(&candidate_text, &source.content));
    let truncation_suspected =
        ratio.is_some_and(|value| value < 0.85) || tail_overlap.is_some_and(|value| value < 0.20);
    attempt.observations.alternative_extraction_compared =
        ExtractionSignal::observed(true, "graphite-shadow-comparison-v1");
    if let Some(ratio) = ratio {
        attempt.observations.coverage_discrepancy =
            ExtractionSignal::observed((1.0 - ratio).abs(), "graphite-shadow-comparison-v1");
        attempt
            .observations
            .provider_diagnostics
            .insert("zoteroCharacterRatio".to_owned(), Value::from(ratio));
    }
    if let Some(difference) = page_difference {
        attempt.observations.provider_diagnostics.insert(
            "zoteroPageCountDifference".to_owned(),
            Value::from(difference),
        );
    }
    if let Some(overlap) = tail_overlap {
        attempt
            .observations
            .provider_diagnostics
            .insert("zoteroTailTokenOverlap".to_owned(), Value::from(overlap));
    }
    attempt.observations.provider_diagnostics.insert(
        "shadowTruncationSuspected".to_owned(),
        Value::Bool(truncation_suspected),
    );
    (
        attempt,
        ShadowComparison {
            zotero_text_available: true,
            candidate_to_zotero_character_ratio: ratio,
            reported_page_count_difference: page_difference,
            tail_token_overlap: tail_overlap,
            truncation_suspected,
        },
    )
}

fn tail_token_overlap(candidate: &str, source: &str) -> f64 {
    fn tokens(value: &str) -> BTreeSet<String> {
        value
            .chars()
            .rev()
            .take(4096)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
            .split_whitespace()
            .map(|token| {
                token
                    .chars()
                    .filter(|character| character.is_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            })
            .filter(|token| token.len() >= 3)
            .collect()
    }
    let candidate = tokens(candidate);
    let source = tokens(source);
    if source.is_empty() {
        return 1.0;
    }
    candidate.intersection(&source).count() as f64 / source.len() as f64
}

fn ocr_fallback_reason(attempt: &ExtractionAttempt) -> Option<String> {
    if attempt.canonical_document.is_none() || attempt.error.is_some() {
        return Some("Xberg native output was unusable".to_owned());
    }
    let text = attempt
        .canonical_document
        .as_ref()
        .map(render_plain_text)
        .unwrap_or_default();
    if text.is_empty() || terminal_outcome(attempt) == "empty-success" {
        return Some("Xberg native output was empty".to_owned());
    }
    if attempt
        .observations
        .reported_page_count
        .value
        .zip(attempt.observations.extracted_page_count.value)
        .is_some_and(|(reported, extracted)| extracted < reported)
    {
        return Some("Xberg native page coverage was incomplete".to_owned());
    }
    if attempt
        .observations
        .empty_page_indices
        .value
        .as_ref()
        .is_some_and(|pages| !pages.is_empty())
    {
        return Some("Xberg native output contained empty pages".to_owned());
    }
    if attempt
        .observations
        .suspicious_page_indices
        .value
        .as_ref()
        .is_some_and(|pages| !pages.is_empty())
    {
        return Some("Xberg native output contained suspicious sparse pages".to_owned());
    }
    None
}

fn unavailable_attempt(
    request: &crate::extraction::model::ExtractionRequest,
    provider: crate::extraction::model::ExtractionProviderDescriptor,
    outcome: &str,
    reason: &str,
) -> ExtractionAttempt {
    let mut observations = crate::extraction::model::ExtractionObservations::unreported(
        &provider.profile_id,
        &request.artifact_id,
        request.artifact_revision_id,
        &request.input_content_hash,
    );
    observations.provider_diagnostics.insert(
        "shadowTerminalOutcome".to_owned(),
        Value::String(outcome.to_owned()),
    );
    ExtractionAttempt {
        provider,
        canonical_document: None,
        provenance: Default::default(),
        observations,
        warnings: Vec::new(),
        error: Some(reason.to_owned()),
    }
}

fn terminal_outcome(attempt: &ExtractionAttempt) -> &str {
    attempt
        .observations
        .provider_diagnostics
        .get("shadowTerminalOutcome")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if attempt.error.is_some() {
                "failed"
            } else {
                "succeeded"
            }
        })
}

fn metrics_from_attempt(attempt: &ExtractionAttempt) -> ShadowRuntimeMetrics {
    ShadowRuntimeMetrics {
        duration_ms: metric(
            &attempt.observations.provider_diagnostics,
            "shadowDurationMs",
        ),
        peak_rss_bytes: metric(
            &attempt.observations.provider_diagnostics,
            "shadowPeakRssBytes",
        ),
        peak_thread_count: metric(
            &attempt.observations.provider_diagnostics,
            "shadowPeakThreadCount",
        ),
    }
}

fn metric(values: &std::collections::BTreeMap<String, Value>, key: &str) -> Option<u64> {
    values.get(key).and_then(Value::as_u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_routing_uses_only_extraction_observations() {
        let mut attempt = crate::extraction::quality::tests::fixture_attempt(true, "clean text");
        attempt.observations.reported_page_count = ExtractionSignal::observed(2, "fixture");
        attempt.observations.extracted_page_count = ExtractionSignal::observed(2, "fixture");
        attempt.observations.empty_page_indices =
            ExtractionSignal::observed(Vec::<u32>::new(), "fixture");
        attempt.observations.suspicious_page_indices =
            ExtractionSignal::observed(Vec::<u32>::new(), "fixture");
        assert_eq!(ocr_fallback_reason(&attempt), None);
        attempt.observations.empty_page_indices = ExtractionSignal::observed(vec![2], "fixture");
        assert_eq!(
            ocr_fallback_reason(&attempt).as_deref(),
            Some("Xberg native output contained empty pages")
        );
    }

    #[test]
    fn comparison_exposes_metrics_without_source_text() {
        let overlap = tail_token_overlap(
            "alpha beta common tail statement",
            "other common tail statement",
        );
        assert!(overlap > 0.5);
    }
}

#[cfg(all(
    test,
    feature = "pdf-shadow-pilot",
    target_os = "linux",
    target_arch = "x86_64"
))]
mod local_pilot_tests {
    use std::fs;
    use std::path::Path;

    use serde::Deserialize;
    use sha2::{Digest, Sha256};

    use crate::database::ResearchDatabase;
    use crate::extraction::shadow::{ShadowRunRequest, SHADOW_ROUTING_PROFILE_ID};
    use crate::extraction::ExtractionCoordinator;
    use crate::model::{
        FullTextRecord, FullTextSyncDelta, LibraryDelta, RawObject, RemoteLibrary, SourceProbe,
    };

    use super::ShadowExtractionCoordinator;

    const PILOT_DOCUMENTS: &[(&str, &str)] = &[
        (
            "paper05",
            "5f7fbabbd3b61c40046e53e66071c84e14afa9e91064a98acb6e6888606c124f",
        ),
        (
            "paper11",
            "c75dd3d3f2528044a6335d2bd02a37ce866856d86c8ec156c48773cc1b77eaf1",
        ),
        (
            "paper12",
            "a33444f68c12b24bdd04c0361d73152d181c9adf8610d0d1cbd3791335280fea",
        ),
        (
            "paper13",
            "0214f36f1b7dbeb104a0cfe87927ed8f90ef61e9aef37a325910ba0e05e60b1c",
        ),
        (
            "paper14",
            "0f8b1ec902a20c7615d622fb609fa3469615c422426d19e8557d6df01100d72e",
        ),
    ];

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ZoteroSidecar {
        source_version: i64,
        content: String,
        indexed_pages: Option<u32>,
        total_pages: Option<u32>,
        indexed_characters: Option<u64>,
        total_characters: Option<u64>,
    }

    #[tokio::test]
    #[ignore = "requires explicitly configured release sidecars and the local benchmark corpus"]
    async fn runs_bounded_real_linux_shadow_pilot_without_active_state_changes() {
        let corpus_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("tools/pdf-benchmark/corpus/local");
        let database_path = std::env::temp_dir().join(format!(
            "graphite-m6-shadow-pilot-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let database = ResearchDatabase::initialize(database_path.clone()).expect("pilot database");
        let source = database
            .register_source(&SourceProbe {
                server_id: "m6-shadow-pilot".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("pilot source");
        let library = database
            .register_libraries(
                &source.id,
                &[RemoteLibrary {
                    kind: "user".to_owned(),
                    zotero_id: "0".to_owned(),
                    name: "M6 local pilot".to_owned(),
                }],
            )
            .expect("pilot library")
            .remove(0);
        let objects = PILOT_DOCUMENTS
            .iter()
            .flat_map(|(id, _)| {
                [
                    pilot_item(&format!("{id}-item"), None),
                    pilot_item(&format!("{id}-pdf"), Some(&format!("{id}-item"))),
                ]
            })
            .collect::<Vec<_>>();
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects,
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("pilot metadata");
        let mut full_text_records = Vec::new();
        for (document_id, expected_hash) in PILOT_DOCUMENTS {
            let pdf = corpus_root.join(format!("{document_id}.pdf"));
            let bytes = fs::read(&pdf).expect("pilot PDF");
            assert_eq!(hex_sha256(&bytes), *expected_hash, "{document_id} checksum");
            let graphite_document_id = document_for_key(&database, &format!("{document_id}-pdf"));
            database
                .store_document_locator(
                    &graphite_document_id,
                    &pdf,
                    &format!("pilot:{document_id}"),
                    expected_hash,
                    bytes.len() as u64,
                    None,
                )
                .expect("pilot locator");
            let sidecar_path = corpus_root.join(format!("{document_id}.zotero-fulltext.json"));
            if sidecar_path.is_file() {
                let sidecar: ZoteroSidecar =
                    serde_json::from_slice(&fs::read(sidecar_path).expect("Zotero sidecar"))
                        .expect("Zotero sidecar JSON");
                full_text_records.push(FullTextRecord {
                    attachment_key: format!("{document_id}-pdf"),
                    version: sidecar.source_version,
                    content: sidecar.content,
                    indexed_pages: sidecar.indexed_pages,
                    total_pages: sidecar.total_pages,
                    indexed_chars: sidecar.indexed_characters,
                    total_chars: sidecar.total_characters,
                });
            }
        }
        database
            .apply_fulltext_delta(
                &source.id,
                &library.id,
                &FullTextSyncDelta {
                    successful_version: 1,
                    records: full_text_records,
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("pilot Zotero text");
        ExtractionCoordinator::new(database.clone())
            .backfill(10)
            .await
            .expect("pilot canonical Zotero baseline");
        let before = active_state_snapshot(&database);
        let coordinator =
            ShadowExtractionCoordinator::new(database.clone()).expect("shadow coordinator");
        let document_ids = PILOT_DOCUMENTS
            .iter()
            .map(|(id, _)| document_for_key(&database, &format!("{id}-pdf")))
            .collect::<Vec<_>>();
        let content_summary = coordinator
            .run_selected(ShadowRunRequest {
                document_ids: document_ids.clone(),
                request_structure: false,
            })
            .await
            .expect("bounded shadow content pilot");
        let structure_summary = coordinator
            .run_selected(ShadowRunRequest {
                document_ids: vec![document_ids[1].clone()],
                request_structure: true,
            })
            .await
            .expect("explicit structure pilot");
        assert_eq!(active_state_snapshot(&database), before);
        let diagnostics = coordinator.diagnostics().expect("shadow diagnostics");
        assert_eq!(diagnostics.routing_profile, SHADOW_ROUTING_PROFILE_ID);
        assert_eq!(diagnostics.running, 0);
        assert!(diagnostics.completed > 0);
        let attempt_rows = pilot_attempt_rows(&database);
        eprintln!(
            "M6 pilot: content={content_summary:?}; structure={structure_summary:?}; diagnostics={diagnostics:?}; attempts={attempt_rows:?}"
        );
        fs::remove_file(database_path).expect("remove pilot database");
    }

    fn pilot_item(key: &str, parent: Option<&str>) -> RawObject {
        let data = parent.map_or_else(
            || {
                serde_json::json!({
                    "key": key,
                    "version": 1,
                    "itemType": "journalArticle",
                    "title": format!("Pilot {key}"),
                    "creators": [],
                    "tags": [],
                    "collections": []
                })
            },
            |parent| {
                serde_json::json!({
                    "key": key,
                    "version": 1,
                    "itemType": "attachment",
                    "parentItem": parent,
                    "title": "PDF",
                    "contentType": "application/pdf",
                    "filename": format!("{key}.pdf"),
                    "linkMode": "imported_file"
                })
            },
        );
        RawObject {
            key: key.to_owned(),
            version: 1,
            object_type: "items".to_owned(),
            json: serde_json::json!({"key": key, "version": 1, "data": data}),
        }
    }

    fn document_for_key(database: &ResearchDatabase, attachment_key: &str) -> String {
        database
            .connect()
            .expect("pilot connection")
            .query_row(
                "SELECT id FROM documents WHERE zotero_attachment_key = ?1",
                [attachment_key],
                |row| row.get(0),
            )
            .expect("pilot document")
    }

    fn active_state_snapshot(database: &ResearchDatabase) -> Vec<(String, String, String, String)> {
        let connection = database.connect().expect("snapshot connection");
        let mut statement = connection
            .prepare(
                "SELECT text.document_id, text.id, text.content_hash,
                        COALESCE(selection.extraction_id, '')
                 FROM document_texts text
                 LEFT JOIN active_extraction_selections selection
                   ON selection.artifact_id = text.document_id
                 WHERE text.is_active = 1
                 ORDER BY text.document_id",
            )
            .expect("active state statement");
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .expect("active state rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("active state snapshot")
    }

    fn pilot_attempt_rows(
        database: &ResearchDatabase,
    ) -> Vec<(String, String, String, String, i64, i64, i64)> {
        let connection = database.connect().expect("pilot metrics connection");
        let mut statement = connection
            .prepare(
                "SELECT binding.external_artifact_id,
                        extraction.provider_profile_id,
                        attempt.attempt_outcome,
                        quality.quality_status,
                        COALESCE(attempt.duration_ms, 0),
                        COALESCE(attempt.peak_rss_bytes, 0),
                        COALESCE(attempt.peak_thread_count, 0)
                 FROM shadow_extraction_attempts attempt
                 JOIN document_extractions extraction
                   ON extraction.id = attempt.extraction_id
                 JOIN extraction_quality_reports quality
                   ON quality.id = attempt.quality_report_id
                 JOIN document_artifact_source_bindings binding
                   ON binding.artifact_id = extraction.artifact_id
                  AND binding.state = 'active'
                 ORDER BY binding.external_artifact_id,
                          extraction.provider_profile_id",
            )
            .expect("pilot metrics statement");
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })
            .expect("pilot metrics rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("pilot metrics")
    }

    fn hex_sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
