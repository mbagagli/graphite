use std::sync::Arc;

use crate::database::ResearchDatabase;
use crate::error::AppResult;

use super::provider::DocumentExtractionProvider;
use super::router::ExtractorRouter;
use super::store::ExtractionStore;
use super::zotero_text::ZoteroTextExtractionProvider;

pub const DEFAULT_CANONICAL_BACKFILL_BATCH: u32 = 16;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractionBackfillSummary {
    pub requested: u32,
    pub processed: u32,
    pub remaining: u32,
}

#[derive(Clone)]
pub struct ExtractionCoordinator {
    store: ExtractionStore,
    router: ExtractorRouter,
}

impl ExtractionCoordinator {
    pub fn new(database: ResearchDatabase) -> Self {
        let provider: Arc<dyn DocumentExtractionProvider> = Arc::new(ZoteroTextExtractionProvider);
        Self {
            store: ExtractionStore::new(database),
            router: ExtractorRouter::default_with(vec![provider]),
        }
    }

    #[cfg(test)]
    pub fn with_router(database: ResearchDatabase, router: ExtractorRouter) -> Self {
        Self {
            store: ExtractionStore::new(database),
            router,
        }
    }

    pub async fn canonicalize_document(&self, document_id: &str) -> AppResult<bool> {
        let Some(request) = self.store.extraction_request(document_id)? else {
            return Ok(false);
        };
        let historical = self.store.active_selection(document_id)?;
        let routed = self
            .router
            .route_with_historical(request.clone(), historical)
            .await;
        self.store.persist_routed_extraction(
            &request,
            self.router.routing_profile(),
            self.router.quality_evaluator(),
            &routed,
        )?;
        Ok(true)
    }

    pub async fn canonicalize_attachment_keys(
        &self,
        library_id: &str,
        attachment_keys: &[String],
    ) -> AppResult<u32> {
        let document_ids = self
            .store
            .document_ids_for_attachment_keys(library_id, attachment_keys)?;
        let mut processed = 0_u32;
        for document_id in document_ids {
            if self.canonicalize_document(&document_id).await? {
                processed += 1;
            }
        }
        Ok(processed)
    }

    pub async fn backfill(&self, limit: u32) -> AppResult<ExtractionBackfillSummary> {
        let limit = limit.clamp(1, 100);
        let document_ids = self.store.pending_document_ids(limit)?;
        let mut processed = 0_u32;
        for document_id in &document_ids {
            if self.canonicalize_document(document_id).await? {
                processed += 1;
            }
        }
        let remaining = self.store.pending_document_ids(100)?.len() as u32;
        Ok(ExtractionBackfillSummary {
            requested: limit,
            processed,
            remaining,
        })
    }

    pub fn recompute_quality(
        &self,
        evaluator: &super::quality::ExtractionQualityEvaluator,
    ) -> AppResult<u32> {
        self.store.recompute_quality_reports(evaluator)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;
    use std::sync::Arc;

    use crate::database::ResearchDatabase;
    use crate::extraction::model::{
        ExtractionAttempt, ExtractionInputDescriptor, ExtractionProviderDescriptor,
        ExtractionQualityProfile, ExtractionRequest, ExtractionRoutingProfile, ExtractionSupport,
    };
    use crate::extraction::provider::DocumentExtractionProvider;
    use crate::extraction::quality::{tests::fixture_attempt, ExtractionQualityEvaluator};
    use crate::knowledge::source::ProviderFuture;
    use crate::model::{
        FullTextRecord, FullTextSyncDelta, LibraryDelta, RawObject, RemoteLibrary, SourceProbe,
    };

    use super::*;

    #[derive(Clone)]
    struct FixtureExtractionProvider {
        descriptor: ExtractionProviderDescriptor,
        structured: bool,
        text: String,
        support: ExtractionSupport,
        failure: Option<String>,
    }

    impl DocumentExtractionProvider for FixtureExtractionProvider {
        fn descriptor(&self) -> ExtractionProviderDescriptor {
            self.descriptor.clone()
        }

        fn supports(&self, _input: &ExtractionInputDescriptor) -> ExtractionSupport {
            self.support.clone()
        }

        fn extract<'a>(
            &'a self,
            request: ExtractionRequest,
        ) -> ProviderFuture<'a, ExtractionAttempt> {
            Box::pin(async move {
                let mut attempt = fixture_attempt(self.structured, &self.text);
                attempt.provider = self.descriptor.clone();
                attempt.observations.provider_profile_id = self.descriptor.profile_id.clone();
                attempt.observations.artifact_id = request.artifact_id.clone();
                attempt.observations.artifact_revision_id = request.artifact_revision_id;
                attempt.observations.input_content_hash = request.input_content_hash;
                if let Some(document) = &mut attempt.canonical_document {
                    document.artifact_id = request.artifact_id;
                    document.artifact_revision_id = request.artifact_revision_id;
                    document.extraction_profile_id = self.descriptor.profile_id.clone();
                }
                if let Some(error) = &self.failure {
                    attempt.canonical_document = None;
                    attempt.error = Some(error.clone());
                }
                Ok(attempt)
            })
        }
    }

    #[tokio::test]
    async fn resumes_bounded_backfill_without_changing_active_text() {
        let (database, path, source_id, library_id) = prepared_database();
        database
            .apply_fulltext_delta(
                &source_id,
                &library_id,
                &FullTextSyncDelta {
                    successful_version: 3,
                    records: vec![
                        full_text("PDF1", "first α text"),
                        full_text("PDF2", "second  1.2e-3\n\ntext"),
                    ],
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("full text");
        let before = active_texts(&database);
        let coordinator = ExtractionCoordinator::new(database.clone());
        let first = coordinator.backfill(1).await.expect("first backfill");
        assert_eq!(first.processed, 1);
        assert_eq!(first.remaining, 1);
        let second = coordinator.backfill(1).await.expect("resume backfill");
        assert_eq!(second.processed, 1);
        assert_eq!(second.remaining, 0);
        let repeated = coordinator.backfill(1).await.expect("idempotent backfill");
        assert_eq!(repeated.processed, 0);
        assert_eq!(active_texts(&database), before);
        let connection = database.connect().expect("connection");
        let canonical_count: i64 = connection
            .query_row(
                "SELECT COUNT(*)
                 FROM active_extraction_selections selection
                 JOIN document_extractions extraction
                   ON extraction.id = selection.extraction_id
                 WHERE extraction.canonical_json IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .expect("canonical count");
        let block_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM canonical_block_index", [], |row| {
                row.get(0)
            })
            .expect("block count");
        assert_eq!(canonical_count, 2);
        assert_eq!(block_count, 2);
        let extraction_count_before: i64 = connection
            .query_row("SELECT COUNT(*) FROM document_extractions", [], |row| {
                row.get(0)
            })
            .expect("extraction count");
        drop(connection);
        let revised_quality = ExtractionQualityEvaluator::new(ExtractionQualityProfile {
            id: "extraction-quality-test-v2".to_owned(),
            version: "2.0.0".to_owned(),
            require_searchable_text: true,
            flat_pdf_text_is_degraded: true,
            maximum_coverage_discrepancy: 0.05,
        });
        assert_eq!(
            coordinator
                .recompute_quality(&revised_quality)
                .expect("quality recomputation"),
            2
        );
        assert_eq!(
            coordinator
                .recompute_quality(&revised_quality)
                .expect("idempotent quality recomputation"),
            0
        );
        let connection = database.connect().expect("connection");
        let extraction_count_after: i64 = connection
            .query_row("SELECT COUNT(*) FROM document_extractions", [], |row| {
                row.get(0)
            })
            .expect("extraction count after quality change");
        assert_eq!(extraction_count_after, extraction_count_before);
        fs::remove_file(path).expect("remove database");
    }

    #[tokio::test]
    async fn profile_change_preserves_attempts_and_never_downgrades_historical_good() {
        let (database, path, source_id, library_id) = prepared_database();
        database
            .apply_fulltext_delta(
                &source_id,
                &library_id,
                &FullTextSyncDelta {
                    successful_version: 3,
                    records: vec![full_text("PDF1", "stable source text")],
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("full text");
        let document_id = document_id(&database, "PDF1");
        let good = fixture_provider("structured-profile-v1", "1.0.0", true);
        ExtractionCoordinator::with_router(database.clone(), router_for(good))
            .canonicalize_document(&document_id)
            .await
            .expect("good extraction");
        let connection = database.connect().expect("connection");
        let first_selection: String = connection
            .query_row(
                "SELECT extraction_id FROM active_extraction_selections
                 WHERE artifact_id = ?1",
                [&document_id],
                |row| row.get(0),
            )
            .expect("first selection");
        let immutable_before = extraction_snapshot(&connection, &first_selection);
        drop(connection);

        let degraded = fixture_provider("flat-profile-v2", "2.0.0", false);
        ExtractionCoordinator::with_router(database.clone(), router_for(degraded))
            .canonicalize_document(&document_id)
            .await
            .expect("degraded extraction");
        let connection = database.connect().expect("connection");
        let active_after: String = connection
            .query_row(
                "SELECT extraction_id FROM active_extraction_selections
                 WHERE artifact_id = ?1",
                [&document_id],
                |row| row.get(0),
            )
            .expect("active selection after profile change");
        let attempts: Vec<(String, String)> = {
            let mut statement = connection
                .prepare(
                    "SELECT id, provider_profile_id FROM document_extractions
                     WHERE artifact_id = ?1 ORDER BY created_at, id",
                )
                .expect("attempts");
            statement
                .query_map([&document_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("attempt rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("attempt history")
        };
        let retained_reason: String = connection
            .query_row(
                "SELECT selection_reason FROM extraction_runs
                 WHERE artifact_id = ?1
                 ORDER BY started_at DESC, id DESC LIMIT 1",
                [&document_id],
                |row| row.get(0),
            )
            .expect("retained reason");
        assert_eq!(active_after, first_selection);
        assert_eq!(attempts.len(), 2);
        assert_ne!(attempts[0].0, attempts[1].0);
        assert!(attempts
            .iter()
            .any(|(_, profile)| profile == "structured-profile-v1"));
        assert!(attempts
            .iter()
            .any(|(_, profile)| profile == "flat-profile-v2"));
        assert_eq!(
            extraction_snapshot(&connection, &first_selection),
            immutable_before
        );
        assert!(retained_reason.contains("historical good"));
        assert!(attempts.iter().all(|(id, _)| {
            connection
                .query_row(
                    "SELECT selected = 0 FROM document_extractions WHERE id = ?1",
                    [id],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap_or(false)
        }));
        drop(connection);

        let mut failed = fixture_provider("failed-profile-v3", "3.0.0", true);
        failed.failure = Some("new extractor profile failed".to_owned());
        ExtractionCoordinator::with_router(database.clone(), router_for(failed))
            .canonicalize_document(&document_id)
            .await
            .expect("failed extraction persistence");
        let connection = database.connect().expect("connection");
        let active_after_failure: String = connection
            .query_row(
                "SELECT extraction_id FROM active_extraction_selections
                 WHERE artifact_id = ?1",
                [&document_id],
                |row| row.get(0),
            )
            .expect("active selection after failed profile");
        let failed_attempts: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM document_extractions
                 WHERE artifact_id = ?1 AND attempt_outcome = 'failed'",
                [&document_id],
                |row| row.get(0),
            )
            .expect("failed attempt count");
        let attempt_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM document_extractions WHERE artifact_id = ?1",
                [&document_id],
                |row| row.get(0),
            )
            .expect("attempt count after failed profile");
        assert_eq!(active_after_failure, first_selection);
        assert_eq!(failed_attempts, 1);
        assert_eq!(attempt_count, 3);
        assert_eq!(
            extraction_snapshot(&connection, &first_selection),
            immutable_before
        );
        drop(connection);
        fs::remove_file(path).expect("remove database");
    }

    #[tokio::test]
    async fn better_profile_switches_only_mutable_selection_state() {
        let (database, path, source_id, library_id) = prepared_database();
        database
            .apply_fulltext_delta(
                &source_id,
                &library_id,
                &FullTextSyncDelta {
                    successful_version: 3,
                    records: vec![full_text("PDF1", "stable source text")],
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("full text");
        let document_id = document_id(&database, "PDF1");
        let degraded = fixture_provider("flat-profile-v1", "1.0.0", false);
        ExtractionCoordinator::with_router(database.clone(), router_for(degraded))
            .canonicalize_document(&document_id)
            .await
            .expect("degraded extraction");
        let connection = database.connect().expect("connection");
        let degraded_id: String = connection
            .query_row(
                "SELECT extraction_id FROM active_extraction_selections
                 WHERE artifact_id = ?1",
                [&document_id],
                |row| row.get(0),
            )
            .expect("degraded selection");
        let degraded_snapshot = extraction_snapshot(&connection, &degraded_id);
        drop(connection);

        let good = fixture_provider("structured-profile-v2", "2.0.0", true);
        ExtractionCoordinator::with_router(database.clone(), router_for(good))
            .canonicalize_document(&document_id)
            .await
            .expect("good extraction");
        let connection = database.connect().expect("connection");
        let good_id: String = connection
            .query_row(
                "SELECT extraction_id FROM active_extraction_selections
                 WHERE artifact_id = ?1",
                [&document_id],
                |row| row.get(0),
            )
            .expect("good selection");
        assert_ne!(good_id, degraded_id);
        assert_eq!(
            extraction_snapshot(&connection, &degraded_id),
            degraded_snapshot
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT report.quality_status
                     FROM active_extraction_selections selection
                     JOIN extraction_quality_reports report
                       ON report.id = selection.quality_report_id
                     WHERE selection.artifact_id = ?1",
                    [&document_id],
                    |row| row.get::<_, String>(0),
                )
                .expect("active quality"),
            "good"
        );
        fs::remove_file(path).expect("remove database");
    }

    #[tokio::test]
    async fn unsupported_and_unavailable_routes_are_persisted_without_selection() {
        let (database, path, source_id, library_id) = prepared_database();
        database
            .apply_fulltext_delta(
                &source_id,
                &library_id,
                &FullTextSyncDelta {
                    successful_version: 3,
                    records: vec![full_text("PDF1", "stable source text")],
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("full text");
        let document_id = document_id(&database, "PDF1");
        let mut unsupported = fixture_provider("unsupported-profile-v1", "1.0.0", true);
        unsupported.support = ExtractionSupport::Unsupported;
        let mut unavailable = fixture_provider("unavailable-profile-v1", "1.0.0", true);
        unavailable.support = ExtractionSupport::Unavailable;
        let router = ExtractorRouter::new(
            vec![Arc::new(unsupported), Arc::new(unavailable)],
            ExtractionRoutingProfile {
                id: "unsupported-routing-v1".to_owned(),
                version: "1.0.0".to_owned(),
                provider_order: vec![
                    "unsupported-profile-v1".to_owned(),
                    "unavailable-profile-v1".to_owned(),
                ],
                compare_degraded: true,
                allow_usable_degraded: true,
            },
            ExtractionQualityEvaluator::default_v1(),
        );
        ExtractionCoordinator::with_router(database.clone(), router)
            .canonicalize_document(&document_id)
            .await
            .expect("unsupported route persistence");
        let connection = database.connect().expect("connection");
        let persisted = {
            let mut statement = connection
                .prepare(
                    "SELECT extraction.attempt_outcome,
                            report.quality_status,
                            extraction.canonical_json IS NULL,
                            EXISTS(
                                SELECT 1 FROM active_extraction_selections selection
                                WHERE selection.artifact_id = extraction.artifact_id
                            )
                     FROM document_extractions extraction
                     JOIN extraction_quality_reports report
                       ON report.extraction_id = extraction.id
                     WHERE extraction.artifact_id = ?1
                     ORDER BY extraction.attempt_outcome",
                )
                .expect("outcome statement");
            statement
                .query_map([&document_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                })
                .expect("outcome rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("persisted outcomes")
        };
        assert_eq!(
            persisted,
            vec![
                ("unavailable".to_owned(), "failed".to_owned(), true, false),
                ("unsupported".to_owned(), "failed".to_owned(), true, false),
            ]
        );
        drop(connection);
        let diagnostics = crate::extraction::diagnostics::load_extraction_diagnostics(&database)
            .expect("diagnostics");
        assert_eq!(diagnostics.unsupported, 1);
        assert_eq!(diagnostics.unavailable, 1);
        fs::remove_file(path).expect("remove database");
    }

    pub(crate) fn prepared_database() -> (ResearchDatabase, std::path::PathBuf, String, String) {
        let path = std::env::temp_dir().join(format!(
            "graphite-extraction-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let database = ResearchDatabase::initialize(path.clone()).expect("database");
        let source = database
            .register_source(&SourceProbe {
                server_id: "extraction-source".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("source");
        let library = database
            .register_libraries(
                &source.id,
                &[RemoteLibrary {
                    kind: "user".to_owned(),
                    zotero_id: "0".to_owned(),
                    name: "My Library".to_owned(),
                }],
            )
            .expect("library")
            .remove(0);
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects: vec![
                        item("PAPER1", None),
                        item("PDF1", Some("PAPER1")),
                        item("PAPER2", None),
                        item("PDF2", Some("PAPER2")),
                    ],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("metadata");
        (database, path, source.id, library.id)
    }

    fn fixture_provider(
        profile_id: &str,
        provider_version: &str,
        structured: bool,
    ) -> FixtureExtractionProvider {
        let mut descriptor = fixture_attempt(structured, "stable source text").provider;
        descriptor.profile_id = profile_id.to_owned();
        descriptor.provider_id = "fixture-extractor".to_owned();
        descriptor.provider_version = provider_version.to_owned();
        descriptor.mapping_profile = format!("fixture-mapping-{provider_version}");
        FixtureExtractionProvider {
            descriptor,
            structured,
            text: "stable source text".to_owned(),
            support: ExtractionSupport::Supported,
            failure: None,
        }
    }

    fn router_for(provider: FixtureExtractionProvider) -> ExtractorRouter {
        let profile_id = provider.descriptor.profile_id.clone();
        ExtractorRouter::new(
            vec![Arc::new(provider)],
            ExtractionRoutingProfile {
                id: format!("fixture-routing-{profile_id}"),
                version: "1.0.0".to_owned(),
                provider_order: vec![profile_id],
                compare_degraded: true,
                allow_usable_degraded: true,
            },
            ExtractionQualityEvaluator::default_v1(),
        )
    }

    pub(crate) fn document_id(database: &ResearchDatabase, attachment_key: &str) -> String {
        database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT id FROM documents WHERE zotero_attachment_key = ?1",
                [attachment_key],
                |row| row.get(0),
            )
            .expect("document")
    }

    fn extraction_snapshot(
        connection: &rusqlite::Connection,
        extraction_id: &str,
    ) -> (String, String, String, String, Option<String>, String, i64) {
        connection
            .query_row(
                "SELECT canonical_json, provenance_json, observations_json,
                        warnings_json, error, attempt_outcome, selected
                 FROM document_extractions WHERE id = ?1",
                [extraction_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .expect("extraction snapshot")
    }

    fn item(key: &str, parent: Option<&str>) -> RawObject {
        let data = if let Some(parent) = parent {
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
        } else {
            serde_json::json!({
                "key": key,
                "version": 1,
                "itemType": "journalArticle",
                "title": format!("Paper {key}"),
                "creators": [],
                "tags": [],
                "collections": []
            })
        };
        RawObject {
            key: key.to_owned(),
            version: 1,
            object_type: "items".to_owned(),
            json: serde_json::json!({
                "key": key,
                "version": 1,
                "data": data
            }),
        }
    }

    pub(crate) fn full_text(key: &str, content: &str) -> FullTextRecord {
        FullTextRecord {
            attachment_key: key.to_owned(),
            version: 3,
            content: content.to_owned(),
            indexed_pages: Some(2),
            total_pages: Some(2),
            indexed_chars: Some(content.len() as u64),
            total_chars: Some(content.len() as u64),
        }
    }

    fn active_texts(database: &ResearchDatabase) -> Vec<(String, String)> {
        let connection = database.connect().expect("connection");
        let mut statement = connection
            .prepare(
                "SELECT document_id, content FROM document_texts
                 WHERE is_active = 1 ORDER BY document_id",
            )
            .expect("text statement");
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("text rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("active texts");
        rows
    }
}
