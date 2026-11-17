use std::path::PathBuf;

use chrono::Utc;
use rusqlite::{params, OptionalExtension};

use crate::database::{content_hash, ResearchDatabase};
use crate::error::{AppError, AppResult};
use crate::extraction::model::{
    ExtractionAttempt, ExtractionAttemptOutcome, ExtractionInputDescriptor,
    ExtractionQualityReport, ExtractionRequest, ExtractionRoutingProfile, SourceTextInput,
    CANONICAL_SCHEMA_VERSION,
};
use crate::extraction::projection::{canonical_hash, canonical_json, render_plain_text};
use crate::extraction::quality::ExtractionQualityEvaluator;
use crate::extraction::store::{rebuild_block_index, serialize, stable_id};

use super::config::ShadowPilotConfig;
use super::model::{
    ShadowArtifactInput, ShadowComparison, ShadowPilotDiagnostics, ShadowRequestKind,
    ShadowRuntimeMetrics, SHADOW_ROUTING_PROFILE_ID, SHADOW_ROUTING_PROFILE_VERSION,
};

#[derive(Clone)]
pub struct ShadowExtractionStore {
    database: ResearchDatabase,
}

pub(super) struct ShadowAttemptRecord<'a> {
    pub job_id: &'a str,
    pub request: &'a ExtractionRequest,
    pub provider_role: &'a str,
    pub routing_reason: &'a str,
    pub attempt: &'a ExtractionAttempt,
    pub quality: &'a ExtractionQualityReport,
    pub comparison: &'a ShadowComparison,
    pub metrics: &'a ShadowRuntimeMetrics,
}

impl ShadowExtractionStore {
    pub fn new(database: ResearchDatabase) -> Self {
        Self { database }
    }

    pub fn configure(&self, config: &ShadowPilotConfig) -> AppResult<()> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        persist_routing_profile(&transaction, &now)?;
        transaction.execute(
            "UPDATE shadow_extraction_pilot_state SET
                enabled = ?1, availability = ?2,
                routing_profile_id = ?3, max_concurrency = ?4,
                updated_at = ?5, last_error = NULL
             WHERE singleton_id = 1",
            params![
                config.enabled,
                config.availability,
                SHADOW_ROUTING_PROFILE_ID,
                config.max_concurrency as i64,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn recover_interrupted(&self) -> AppResult<u32> {
        let connection = self.database.connect()?;
        let changed = connection.execute(
            "UPDATE shadow_extraction_jobs SET
                execution_state = CASE
                    WHEN cancellation_requested = 1 THEN 'cancelled'
                    ELSE 'pending'
                END,
                started_at = NULL,
                completed_at = CASE
                    WHEN cancellation_requested = 1 THEN ?1 ELSE NULL
                END,
                last_error = CASE
                    WHEN cancellation_requested = 1
                    THEN 'cancelled while Graphite was not running'
                    ELSE NULL
                END
             WHERE execution_state = 'running'",
            [Utc::now().to_rfc3339()],
        )?;
        Ok(changed as u32)
    }

    pub fn artifact_input(&self, document_id: &str) -> AppResult<Option<ShadowArtifactInput>> {
        let connection = self.database.connect()?;
        let row = connection
            .query_row(
                "SELECT revision.id, revision.content_hash,
                        locator.canonical_path, document.media_type,
                        text.id, text.provider, text.source_version,
                        text.content, text.indexed_pages, text.total_pages,
                        text.indexed_chars, text.total_chars
                 FROM documents document
                 JOIN document_locators locator
                   ON locator.document_id = document.id
                  AND locator.locator_kind = 'validated-local-file'
                 JOIN document_revisions revision
                   ON revision.document_id = document.id
                  AND revision.revision_key = locator.revision_key
                  AND revision.revision_kind = 'artifact-content'
                 LEFT JOIN document_texts text
                   ON text.document_id = document.id AND text.is_active = 1
                 WHERE document.id = ?1
                 ORDER BY revision.id DESC LIMIT 1",
                [document_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<u32>>(8)?,
                        row.get::<_, Option<u32>>(9)?,
                        row.get::<_, Option<i64>>(10)?,
                        row.get::<_, Option<i64>>(11)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            revision_id,
            input_hash,
            path,
            media_type,
            text_id,
            text_provider,
            text_version,
            text_content,
            indexed_pages,
            total_pages,
            indexed_characters,
            total_characters,
        )) = row
        else {
            return Ok(None);
        };
        let source_text = match (text_id, text_provider, text_version, text_content) {
            (Some(text_id), Some(provider), Some(source_version), Some(content)) => {
                Some(SourceTextInput {
                    text_id,
                    provider,
                    source_version,
                    content,
                    indexed_pages,
                    total_pages,
                    indexed_characters: indexed_characters.map(|value| value as u64),
                    total_characters: total_characters.map(|value| value as u64),
                })
            }
            _ => None,
        };
        Ok(Some(ShadowArtifactInput {
            request: ExtractionRequest {
                artifact_id: document_id.to_owned(),
                artifact_revision_id: revision_id,
                input_content_hash: input_hash,
                input: ExtractionInputDescriptor {
                    media_type,
                    has_local_file: true,
                    has_bounded_reader: false,
                    has_source_provided_text: source_text.is_some(),
                },
                source_text,
            },
            local_path: PathBuf::from(path),
        }))
    }

    pub fn enqueue(
        &self,
        request: &ExtractionRequest,
        request_kind: ShadowRequestKind,
        routing_reason: &str,
    ) -> AppResult<(String, bool)> {
        let connection = self.database.connect()?;
        let job_id = stable_id(
            "shadow-job",
            &[
                &request.artifact_id,
                &request.artifact_revision_id.to_string(),
                SHADOW_ROUTING_PROFILE_ID,
                request_kind.as_str(),
            ],
        );
        let changed = connection.execute(
            "INSERT OR IGNORE INTO shadow_extraction_jobs (
                id, artifact_id, artifact_revision_id,
                routing_profile_id, request_kind, execution_state,
                routing_reason, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending', ?6, ?7)",
            params![
                job_id,
                request.artifact_id,
                request.artifact_revision_id,
                SHADOW_ROUTING_PROFILE_ID,
                request_kind.as_str(),
                routing_reason,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok((job_id, changed == 1))
    }

    pub fn start_job(&self, job_id: &str) -> AppResult<bool> {
        let connection = self.database.connect()?;
        let changed = connection.execute(
            "UPDATE shadow_extraction_jobs SET
                execution_state = 'running', started_at = ?2,
                completed_at = NULL, last_error = NULL
             WHERE id = ?1 AND execution_state = 'pending'
               AND cancellation_requested = 0",
            params![job_id, Utc::now().to_rfc3339()],
        )?;
        Ok(changed == 1)
    }

    pub fn request_cancellation(&self) -> AppResult<u32> {
        let connection = self.database.connect()?;
        let changed = connection.execute(
            "UPDATE shadow_extraction_jobs SET
                cancellation_requested = 1,
                execution_state = CASE
                    WHEN execution_state = 'pending' THEN 'cancelled'
                    ELSE execution_state
                END,
                completed_at = CASE
                    WHEN execution_state = 'pending' THEN ?1
                    ELSE completed_at
                END,
                last_error = CASE
                    WHEN execution_state = 'pending'
                    THEN 'shadow extraction was cancelled before start'
                    ELSE last_error
                END
             WHERE execution_state IN ('pending', 'running')",
            [Utc::now().to_rfc3339()],
        )?;
        Ok(changed as u32)
    }

    pub fn persist_attempt(&self, record: ShadowAttemptRecord<'_>) -> AppResult<String> {
        let ShadowAttemptRecord {
            job_id,
            request,
            provider_role,
            routing_reason,
            attempt,
            quality,
            comparison,
            metrics,
        } = record;
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        let quality_evaluator = ExtractionQualityEvaluator::default_v1();
        persist_provider_profiles(&transaction, attempt, &quality_evaluator, &now)?;
        let run_id = stable_id("shadow-run", &[job_id]);
        transaction.execute(
            "INSERT OR IGNORE INTO extraction_runs (
                id, artifact_id, artifact_revision_id,
                routing_profile_id, execution_state,
                fallback_used, fallback_reason, started_at,
                selection_reason
             ) VALUES (?1, ?2, ?3, ?4, 'running', 0, NULL, ?5,
                'shadow-only: active selection is prohibited')",
            params![
                run_id,
                request.artifact_id,
                request.artifact_revision_id,
                SHADOW_ROUTING_PROFILE_ID,
                now,
            ],
        )?;
        let extraction_id = stable_id(
            "extraction",
            &[
                &request.artifact_id,
                &request.artifact_revision_id.to_string(),
                &request.input_content_hash,
                &attempt.provider.profile_id,
                &attempt.provider.provider_id,
                &attempt.provider.provider_version,
                &attempt.provider.mapping_profile,
                CANONICAL_SCHEMA_VERSION,
            ],
        );
        let canonical_json_value = attempt
            .canonical_document
            .as_ref()
            .map(canonical_json)
            .transpose()?;
        let canonical_hash_value = attempt
            .canonical_document
            .as_ref()
            .map(canonical_hash)
            .transpose()?;
        let plain_text = attempt.canonical_document.as_ref().map(render_plain_text);
        let plain_text_hash = plain_text.as_deref().map(content_hash);
        let attempt_outcome = terminal_outcome(attempt);
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO document_extractions (
                id, run_id, artifact_id, artifact_revision_id,
                provider_profile_id, canonical_schema_version,
                canonical_json, canonical_hash, plain_text_hash,
                execution_state, selected, fallback_used,
                fallback_reason, provenance_json, observations_json,
                warnings_json, error, created_at, completed_at,
                attempt_outcome
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, 0, 0, NULL, ?11, ?12, ?13, ?14, ?15, ?15, ?16
             )",
            params![
                extraction_id,
                run_id,
                request.artifact_id,
                request.artifact_revision_id,
                attempt.provider.profile_id,
                CANONICAL_SCHEMA_VERSION,
                canonical_json_value,
                canonical_hash_value,
                plain_text_hash,
                if attempt.canonical_document.is_some() {
                    "succeeded"
                } else {
                    "failed"
                },
                serialize(&attempt.provenance)?,
                serialize(&attempt.observations)?,
                serialize(&attempt.warnings)?,
                attempt.error,
                now,
                attempt_outcome,
            ],
        )?;
        if inserted == 0 {
            verify_immutable_attempt(
                &transaction,
                &extraction_id,
                &attempt.provider.profile_id,
                canonical_hash_value.as_deref(),
                plain_text_hash.as_deref(),
                attempt_outcome,
            )?;
        } else if let Some(document) = &attempt.canonical_document {
            rebuild_block_index(&transaction, &extraction_id, document)?;
        }
        let quality_report_id =
            stable_id("extraction-quality", &[&extraction_id, &quality.profile_id]);
        transaction.execute(
            "INSERT OR IGNORE INTO extraction_quality_reports (
                id, extraction_id, quality_profile_id,
                quality_status, usable, signals_json,
                reasons_json, evaluated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                quality_report_id,
                extraction_id,
                quality.profile_id,
                quality.status.as_str(),
                quality.usable,
                serialize(&quality.signals)?,
                serialize(&quality.reasons)?,
                now,
            ],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO shadow_extraction_attempts (
                job_id, extraction_id, quality_report_id,
                provider_role, attempt_outcome, routing_reason,
                duration_ms, peak_rss_bytes, peak_thread_count,
                comparison_json, recorded_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                job_id,
                extraction_id,
                quality_report_id,
                provider_role,
                attempt_outcome,
                routing_reason,
                metrics.duration_ms.map(|value| value as i64),
                metrics.peak_rss_bytes.map(|value| value as i64),
                metrics.peak_thread_count.map(|value| value as i64),
                serialize(comparison)?,
                now,
            ],
        )?;
        let state = job_state(attempt_outcome);
        transaction.execute(
            "UPDATE shadow_extraction_jobs SET
                execution_state = ?2, completed_at = ?3,
                last_error = ?4
             WHERE id = ?1",
            params![
                job_id,
                state,
                now,
                if state == "completed" {
                    None::<String>
                } else {
                    attempt.error.clone()
                },
            ],
        )?;
        transaction.execute(
            "UPDATE extraction_runs SET
                execution_state = ?2, completed_at = ?3,
                last_error = ?4,
                selected_extraction_id = NULL,
                selection_reason = 'shadow-only: active selection was not changed'
             WHERE id = ?1",
            params![
                run_id,
                if state == "completed" {
                    "succeeded"
                } else {
                    "failed"
                },
                now,
                attempt.error,
            ],
        )?;
        transaction.commit()?;
        Ok(extraction_id)
    }

    pub fn diagnostics(&self) -> AppResult<ShadowPilotDiagnostics> {
        let connection = self.database.connect()?;
        let (enabled, availability, routing_profile) = connection.query_row(
            "SELECT enabled, availability, routing_profile_id
             FROM shadow_extraction_pilot_state WHERE singleton_id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, bool>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        let job_counts = connection.query_row(
            "SELECT
                COUNT(CASE WHEN execution_state = 'pending' THEN 1 END),
                COUNT(CASE WHEN execution_state = 'running' THEN 1 END),
                COUNT(CASE WHEN execution_state = 'completed' THEN 1 END),
                COUNT(CASE WHEN execution_state = 'failed' THEN 1 END),
                COUNT(CASE WHEN execution_state = 'cancelled' THEN 1 END),
                COUNT(CASE WHEN execution_state = 'unavailable' THEN 1 END)
             FROM shadow_extraction_jobs",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, i64>(5)? as u64,
                ))
            },
        )?;
        let quality_counts = connection.query_row(
            "SELECT
                COUNT(CASE WHEN report.quality_status = 'good' THEN 1 END),
                COUNT(CASE WHEN report.quality_status = 'degraded' THEN 1 END),
                COUNT(CASE WHEN report.quality_status = 'failed' THEN 1 END),
                COUNT(CASE WHEN attempt.provider_role = 'ocr-fallback' THEN 1 END),
                COUNT(CASE
                    WHEN json_extract(attempt.comparison_json,
                        '$.truncationSuspected') = 1 THEN 1 END)
             FROM shadow_extraction_attempts attempt
             JOIN extraction_quality_reports report
               ON report.id = attempt.quality_report_id",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                ))
            },
        )?;
        let mut statement = connection.prepare(
            "SELECT DISTINCT extraction.provider_profile_id
             FROM shadow_extraction_attempts attempt
             JOIN document_extractions extraction
               ON extraction.id = attempt.extraction_id
             ORDER BY extraction.provider_profile_id",
        )?;
        let providers = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let latest = connection
            .query_row(
                "SELECT extraction.provider_profile_id,
                        attempt.routing_reason,
                        attempt.duration_ms,
                        attempt.peak_rss_bytes
                 FROM shadow_extraction_attempts attempt
                 JOIN document_extractions extraction
                   ON extraction.id = attempt.extraction_id
                 ORDER BY attempt.recorded_at DESC,
                          attempt.extraction_id DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?.map(|value| value as u64),
                        row.get::<_, Option<i64>>(3)?.map(|value| value as u64),
                    ))
                },
            )
            .optional()?;
        Ok(ShadowPilotDiagnostics {
            enabled,
            availability,
            routing_profile,
            pending: job_counts.0,
            running: job_counts.1,
            completed: job_counts.2,
            failed: job_counts.3,
            cancelled: job_counts.4,
            unavailable: job_counts.5,
            good: quality_counts.0,
            degraded: quality_counts.1,
            quality_failed: quality_counts.2,
            providers,
            last_provider_profile: latest.as_ref().map(|value| value.0.clone()),
            last_routing_reason: latest.as_ref().map(|value| value.1.clone()),
            last_duration_ms: latest.as_ref().and_then(|value| value.2),
            last_peak_rss_bytes: latest.as_ref().and_then(|value| value.3),
            fallback_routes: quality_counts.3,
            comparison_discrepancies: quality_counts.4,
        })
    }
}

fn persist_routing_profile(transaction: &rusqlite::Transaction<'_>, now: &str) -> AppResult<()> {
    let profile = ExtractionRoutingProfile {
        id: SHADOW_ROUTING_PROFILE_ID.to_owned(),
        version: SHADOW_ROUTING_PROFILE_VERSION.to_owned(),
        provider_order: vec![
            "native-primary".to_owned(),
            "ocr-fallback-if-observable".to_owned(),
            "structure-demand-if-explicit".to_owned(),
        ],
        compare_degraded: true,
        allow_usable_degraded: true,
    };
    transaction.execute(
        "INSERT OR IGNORE INTO extraction_routing_profiles (
            id, version, policy_json, active, created_at
         ) VALUES (?1, ?2, ?3, 0, ?4)",
        params![profile.id, profile.version, serialize(&profile)?, now],
    )?;
    Ok(())
}

fn persist_provider_profiles(
    transaction: &rusqlite::Transaction<'_>,
    attempt: &ExtractionAttempt,
    evaluator: &ExtractionQualityEvaluator,
    now: &str,
) -> AppResult<()> {
    persist_routing_profile(transaction, now)?;
    let quality = evaluator.profile();
    transaction.execute(
        "INSERT OR IGNORE INTO extraction_quality_profiles (
            id, version, rules_json, active, created_at
         ) VALUES (?1, ?2, ?3, 0, ?4)",
        params![quality.id, quality.version, serialize(quality)?, now],
    )?;
    let provider = &attempt.provider;
    transaction.execute(
        "INSERT OR IGNORE INTO extraction_provider_profiles (
            id, provider_id, provider_version, mapping_profile,
            supported_media_types_json, capabilities_json,
            configured_available, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            provider.profile_id,
            provider.provider_id,
            provider.provider_version,
            provider.mapping_profile,
            serialize(&provider.supported_media_types)?,
            serialize(provider)?,
            provider.configured_available,
            now,
        ],
    )?;
    Ok(())
}

fn verify_immutable_attempt(
    transaction: &rusqlite::Transaction<'_>,
    extraction_id: &str,
    provider_profile_id: &str,
    canonical_hash: Option<&str>,
    plain_text_hash: Option<&str>,
    attempt_outcome: &str,
) -> AppResult<()> {
    let stored = transaction.query_row(
        "SELECT provider_profile_id, canonical_hash,
                plain_text_hash, attempt_outcome
         FROM document_extractions WHERE id = ?1",
        [extraction_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )?;
    if stored
        != (
            provider_profile_id.to_owned(),
            canonical_hash.map(str::to_owned),
            plain_text_hash.map(str::to_owned),
            attempt_outcome.to_owned(),
        )
    {
        return Err(AppError::ApplicationData(format!(
            "immutable shadow extraction identity {extraction_id} produced different evidence"
        )));
    }
    Ok(())
}

fn terminal_outcome(attempt: &ExtractionAttempt) -> &str {
    attempt
        .observations
        .provider_diagnostics
        .get("shadowTerminalOutcome")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| {
            if attempt.error.is_some() {
                ExtractionAttemptOutcome::Failed.as_str()
            } else {
                ExtractionAttemptOutcome::Succeeded.as_str()
            }
        })
}

fn job_state(outcome: &str) -> &str {
    match outcome {
        "succeeded" | "partial" | "suspicious" | "empty-success" => "completed",
        "unavailable" | "unsupported" => "unavailable",
        "cancelled" => "cancelled",
        _ => "failed",
    }
}

#[cfg(test)]
mod tests {
    use crate::extraction::coordinator::tests::{document_id, full_text, prepared_database};
    use crate::extraction::store::ExtractionStore;
    use crate::model::FullTextSyncDelta;

    use super::*;

    #[test]
    fn shadow_jobs_are_idempotent_restartable_cancellable_and_revision_recoverable() {
        let (database, path, source_id, library_id) = prepared_database();
        database
            .apply_fulltext_delta(
                &source_id,
                &library_id,
                &FullTextSyncDelta {
                    successful_version: 1,
                    records: vec![full_text("PDF1", "stable Zotero text")],
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("full text");
        let document_id = document_id(&database, "PDF1");
        let request = ExtractionStore::new(database.clone())
            .extraction_request(&document_id)
            .expect("request")
            .expect("document request");
        let store = ShadowExtractionStore::new(database.clone());
        store
            .configure(&ShadowPilotConfig::disabled())
            .expect("configure");
        let (job_id, inserted) = store
            .enqueue(&request, ShadowRequestKind::Native, "fixture native route")
            .expect("enqueue");
        assert!(inserted);
        let (same_job_id, duplicate) = store
            .enqueue(&request, ShadowRequestKind::Native, "fixture native route")
            .expect("duplicate enqueue");
        assert_eq!(same_job_id, job_id);
        assert!(!duplicate);
        assert!(store.start_job(&job_id).expect("start"));
        assert_eq!(store.recover_interrupted().expect("recover"), 1);
        assert_eq!(store.request_cancellation().expect("cancel"), 1);
        let state: (String, bool) = database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT execution_state, cancellation_requested
                 FROM shadow_extraction_jobs WHERE id = ?1",
                [&job_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("job state");
        assert_eq!(state, ("cancelled".to_owned(), true));
        let failed_connection = database.connect().expect("failed job connection");
        failed_connection
            .execute(
                "UPDATE shadow_extraction_jobs SET
                    execution_state = 'failed', cancellation_requested = 0,
                    last_error = 'fixture provider failure'
                 WHERE id = ?1",
                [&job_id],
            )
            .expect("mark failed fixture");
        failed_connection
            .execute(
                "INSERT INTO document_revisions (
                    document_id, source_version, revision_key,
                    file_size, modified_ms, discovered_at,
                    revision_kind, content_hash
                 )
                 SELECT document_id, source_version, 'fixture-recovery-revision',
                        file_size, modified_ms, CURRENT_TIMESTAMP,
                        revision_kind, content_hash
                 FROM document_revisions WHERE id = ?1",
                [request.artifact_revision_id],
            )
            .expect("new artifact revision");
        let mut recovery_request = request.clone();
        recovery_request.artifact_revision_id = failed_connection.last_insert_rowid();
        assert_ne!(
            recovery_request.artifact_revision_id,
            request.artifact_revision_id
        );
        let (recovery_job_id, recovery_inserted) = store
            .enqueue(
                &recovery_request,
                ShadowRequestKind::Native,
                "fixture recovery route",
            )
            .expect("enqueue recovery revision");
        assert!(recovery_inserted);
        assert_ne!(recovery_job_id, job_id);
        std::fs::remove_file(path).expect("remove database");
    }
}
