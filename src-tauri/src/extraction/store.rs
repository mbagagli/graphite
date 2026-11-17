use std::collections::HashMap;

use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::database::{content_hash, select_document_text_projection, ResearchDatabase};
use crate::error::{AppError, AppResult};

use super::model::{
    CanonicalBlockContent, CanonicalDocumentV1, ExtractionAttempt, ExtractionInputDescriptor,
    ExtractionObservations, ExtractionProviderDescriptor, ExtractionQualityStatus,
    ExtractionRequest, ExtractionRoutingProfile, ExtractionSelectionDecision,
    HistoricalExtractionSelection, RoutedExtraction, SourceTextInput, CANONICAL_SCHEMA_VERSION,
};
use super::projection::{canonical_hash, canonical_json, render_plain_text};
use super::quality::ExtractionQualityEvaluator;

#[derive(Clone)]
pub struct ExtractionStore {
    database: ResearchDatabase,
}

impl ExtractionStore {
    pub fn new(database: ResearchDatabase) -> Self {
        Self { database }
    }

    pub fn active_selection(
        &self,
        artifact_id: &str,
    ) -> AppResult<Option<HistoricalExtractionSelection>> {
        let connection = self.database.connect()?;
        let selection = connection
            .query_row(
                "SELECT selection.extraction_id,
                        selection.quality_report_id,
                        report.quality_status,
                        report.usable
                 FROM active_extraction_selections selection
                 JOIN extraction_quality_reports report
                   ON report.id = selection.quality_report_id
                 WHERE selection.artifact_id = ?1",
                [artifact_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                },
            )
            .optional()?;
        selection
            .map(
                |(extraction_id, quality_report_id, quality_status, usable)| {
                    let quality_status = ExtractionQualityStatus::parse(&quality_status)
                        .ok_or_else(|| {
                            AppError::ApplicationData(format!(
                                "active extraction has unknown quality status {quality_status}"
                            ))
                        })?;
                    Ok(HistoricalExtractionSelection {
                        extraction_id,
                        quality_report_id,
                        quality_status,
                        usable,
                    })
                },
            )
            .transpose()
    }

    pub fn pending_document_ids(&self, limit: u32) -> AppResult<Vec<String>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT dt.document_id
             FROM document_texts dt
             JOIN documents d ON d.id = dt.document_id
             LEFT JOIN active_extraction_selections selection
               ON selection.artifact_id = dt.document_id
             LEFT JOIN artifact_extraction_state aes
               ON aes.artifact_id = dt.document_id
             WHERE dt.is_active = 1
               AND dt.provider NOT LIKE 'canonical:docling-local-%'
               AND dt.provider NOT IN ('canonical:pdfjs-pages-v1',
                 'canonical:demo-pdfjs-pages-v1')
               AND d.research_item_id IS NOT NULL
               AND (
                    dt.extraction_id IS NULL
                    OR selection.extraction_id IS NULL
                    OR aes.execution_state IN ('pending', 'stale')
               )
             ORDER BY dt.fetched_at, dt.document_id
             LIMIT ?1",
        )?;
        let rows = statement
            .query_map([limit.clamp(1, 100) as i64], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        Ok(rows)
    }

    pub fn document_ids_for_attachment_keys(
        &self,
        library_id: &str,
        attachment_keys: &[String],
    ) -> AppResult<Vec<String>> {
        if attachment_keys.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.database.connect()?;
        connection.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS extraction_attachment_keys (
                key TEXT PRIMARY KEY
             ) WITHOUT ROWID;
             DELETE FROM extraction_attachment_keys;",
        )?;
        for key in attachment_keys {
            connection.execute(
                "INSERT OR IGNORE INTO extraction_attachment_keys(key)
                 VALUES (?1)",
                [key],
            )?;
        }
        let mut statement = connection.prepare(
            "SELECT d.id
             FROM documents d
             JOIN document_artifact_source_bindings binding
               ON binding.artifact_id = d.id AND binding.state = 'active'
             JOIN source_instances instance
               ON instance.id = binding.source_instance_id
             JOIN extraction_attachment_keys k
               ON k.key = binding.external_artifact_id
             JOIN document_texts dt
               ON dt.document_id = d.id AND dt.is_active = 1
             WHERE instance.legacy_library_id = ?1
             ORDER BY d.id",
        )?;
        let rows = statement
            .query_map([library_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        Ok(rows)
    }

    pub fn extraction_request(&self, document_id: &str) -> AppResult<Option<ExtractionRequest>> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction()?;
        let text = transaction
            .query_row(
                "SELECT dt.id, dt.provider, dt.source_version, dt.content,
                        dt.indexed_pages, dt.total_pages, dt.indexed_chars,
                        dt.total_chars, dt.content_hash, d.media_type
                 FROM document_texts dt
                 JOIN documents d ON d.id = dt.document_id
                 WHERE dt.document_id = ?1 AND dt.is_active = 1",
                [document_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<u32>>(4)?,
                        row.get::<_, Option<u32>>(5)?,
                        row.get::<_, Option<i64>>(6)?.map(|value| value as u64),
                        row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            text_id,
            provider,
            source_version,
            content,
            indexed_pages,
            total_pages,
            indexed_characters,
            total_characters,
            stored_hash,
            media_type,
        )) = text
        else {
            return Ok(None);
        };
        if provider.starts_with("canonical:docling-local-")
            || matches!(
                provider.as_str(),
                "canonical:pdfjs-pages-v1" | "canonical:demo-pdfjs-pages-v1"
            )
        {
            return Ok(None);
        }
        let text_hash = if stored_hash.is_empty() {
            content_hash(&content)
        } else {
            stored_hash
        };
        transaction.execute(
            "UPDATE document_texts SET content_hash = ?2
             WHERE id = ?1 AND content_hash <> ?2",
            params![text_id, text_hash],
        )?;
        let revision_id =
            ensure_source_text_revision(&transaction, document_id, source_version, &text_hash)?;
        transaction.execute(
            "INSERT INTO artifact_extraction_state (
                artifact_id, execution_state, updated_at
             ) VALUES (?1, 'pending', ?2)
             ON CONFLICT(artifact_id) DO NOTHING",
            params![document_id, Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
        Ok(Some(ExtractionRequest {
            artifact_id: document_id.to_owned(),
            artifact_revision_id: revision_id,
            input_content_hash: text_hash,
            input: ExtractionInputDescriptor {
                media_type,
                has_local_file: false,
                has_bounded_reader: false,
                has_source_provided_text: true,
            },
            source_text: Some(SourceTextInput {
                text_id,
                provider,
                source_version,
                content,
                indexed_pages,
                total_pages,
                indexed_characters,
                total_characters,
            }),
        }))
    }

    pub fn persist_routed_extraction(
        &self,
        request: &ExtractionRequest,
        routing_profile: &ExtractionRoutingProfile,
        quality_evaluator: &ExtractionQualityEvaluator,
        routed: &RoutedExtraction,
    ) -> AppResult<()> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        persist_profiles(
            &transaction,
            routing_profile,
            quality_evaluator,
            routed,
            &now,
        )?;
        let provider_identity = routing_profile.provider_order.join("\u{1f}");
        let run_id = stable_id(
            "extraction-run",
            &[
                &request.artifact_id,
                &request.artifact_revision_id.to_string(),
                &request.input_content_hash,
                &routing_profile.id,
                &routing_profile.version,
                &provider_identity,
            ],
        );
        transaction.execute(
            "INSERT OR IGNORE INTO extraction_runs (
                id, artifact_id, artifact_revision_id,
                routing_profile_id, execution_state,
                fallback_used, fallback_reason, started_at
             ) VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7)",
            params![
                run_id,
                request.artifact_id,
                request.artifact_revision_id,
                routing_profile.id,
                routed.fallback_used,
                routed.fallback_reason,
                now,
            ],
        )?;
        let mut extraction_ids = Vec::with_capacity(routed.attempts.len());
        for evaluated in &routed.attempts {
            let extraction_id = stable_id(
                "extraction",
                &[
                    &request.artifact_id,
                    &request.artifact_revision_id.to_string(),
                    &request.input_content_hash,
                    &evaluated.attempt.provider.profile_id,
                    &evaluated.attempt.provider.provider_id,
                    &evaluated.attempt.provider.provider_version,
                    &evaluated.attempt.provider.mapping_profile,
                    CANONICAL_SCHEMA_VERSION,
                ],
            );
            let canonical_json_value = evaluated
                .attempt
                .canonical_document
                .as_ref()
                .map(canonical_json)
                .transpose()?;
            let canonical_hash_value = evaluated
                .attempt
                .canonical_document
                .as_ref()
                .map(canonical_hash)
                .transpose()?;
            let plain_text = evaluated
                .attempt
                .canonical_document
                .as_ref()
                .map(render_plain_text);
            let plain_text_hash = plain_text.as_deref().map(content_hash);
            transaction.execute(
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
                    ?10, 0, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17,
                    ?18
                 )",
                params![
                    extraction_id,
                    run_id,
                    request.artifact_id,
                    request.artifact_revision_id,
                    evaluated.attempt.provider.profile_id,
                    CANONICAL_SCHEMA_VERSION,
                    canonical_json_value,
                    canonical_hash_value,
                    plain_text_hash,
                    if evaluated.outcome == super::model::ExtractionAttemptOutcome::Succeeded {
                        "succeeded"
                    } else {
                        "failed"
                    },
                    routed.fallback_used,
                    routed.fallback_reason,
                    serialize(&evaluated.attempt.provenance)?,
                    serialize(&evaluated.attempt.observations)?,
                    serialize(&evaluated.attempt.warnings)?,
                    evaluated.attempt.error,
                    now,
                    evaluated.outcome.as_str(),
                ],
            )?;
            let report_id = stable_id(
                "extraction-quality",
                &[&extraction_id, &evaluated.quality.profile_id],
            );
            transaction.execute(
                "INSERT OR IGNORE INTO extraction_quality_reports (
                    id, extraction_id, quality_profile_id,
                    quality_status, usable, signals_json,
                    reasons_json, evaluated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    report_id,
                    extraction_id,
                    evaluated.quality.profile_id,
                    evaluated.quality.status.as_str(),
                    evaluated.quality.usable,
                    serialize(&evaluated.quality.signals)?,
                    serialize(&evaluated.quality.reasons)?,
                    now,
                ],
            )?;
            if let Some(document) = &evaluated.attempt.canonical_document {
                rebuild_block_index(&transaction, &extraction_id, document)?;
            }
            extraction_ids.push((extraction_id, report_id, plain_text));
        }

        if let ExtractionSelectionDecision::NewAttempt(selected_index) = routed.selection {
            let (selected_id, report_id, selected_text) =
                extraction_ids.get(selected_index).ok_or_else(|| {
                    AppError::ApplicationData(
                        "router selected an unknown extraction attempt".to_owned(),
                    )
                })?;
            let selected = &routed.attempts[selected_index];
            let selected_text = selected_text.as_deref().ok_or_else(|| {
                AppError::ApplicationData(
                    "selected extraction has no searchable projection".to_owned(),
                )
            })?;
            let selected_hash = content_hash(selected_text);
            transaction.execute(
                "INSERT INTO active_extraction_selections (
                    artifact_id, extraction_id, quality_report_id,
                    routing_profile_id, selection_reason,
                    selected_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT(artifact_id) DO UPDATE SET
                    extraction_id = excluded.extraction_id,
                    quality_report_id = excluded.quality_report_id,
                    routing_profile_id = excluded.routing_profile_id,
                    selection_reason = excluded.selection_reason,
                    selected_at = excluded.selected_at,
                    updated_at = excluded.updated_at",
                params![
                    request.artifact_id,
                    selected_id,
                    report_id,
                    routing_profile.id,
                    routed.selection_reason,
                    now,
                ],
            )?;
            select_document_text_projection(
                &transaction,
                &request.artifact_id,
                selected_id,
                "plain-text-v1",
                &format!("canonical:{}", selected.attempt.provider.profile_id),
                selected_text,
                &selected_hash,
            )?;
            transaction.execute(
                "INSERT INTO artifact_extraction_state (
                    artifact_id, active_extraction_id,
                    active_quality_report_id, execution_state,
                    quality_status, updated_at, last_error
                 ) VALUES (?1, ?2, ?3, 'succeeded', ?4, ?5, NULL)
                 ON CONFLICT(artifact_id) DO UPDATE SET
                    active_extraction_id = excluded.active_extraction_id,
                    active_quality_report_id =
                        excluded.active_quality_report_id,
                    execution_state = excluded.execution_state,
                    quality_status = excluded.quality_status,
                    updated_at = excluded.updated_at,
                    last_error = NULL",
                params![
                    request.artifact_id,
                    selected_id,
                    report_id,
                    selected.quality.status.as_str(),
                    now,
                ],
            )?;
            transaction.execute(
                "UPDATE extraction_runs SET
                    execution_state = 'succeeded',
                    selected_extraction_id = ?2,
                    fallback_used = ?3,
                    fallback_reason = ?4,
                    selection_reason = ?5,
                    completed_at = ?6,
                    last_error = NULL
                 WHERE id = ?1",
                params![
                    run_id,
                    selected_id,
                    routed.fallback_used,
                    routed.fallback_reason,
                    routed.selection_reason,
                    now,
                ],
            )?;
        } else if routed.selection == ExtractionSelectionDecision::RetainHistorical {
            let historical = routed.historical_active.as_ref().ok_or_else(|| {
                AppError::ApplicationData(
                    "router retained a historical extraction without an active selection"
                        .to_owned(),
                )
            })?;
            let current_error = routed
                .attempts
                .iter()
                .rev()
                .find_map(|attempt| attempt.attempt.error.clone());
            let current_succeeded = routed.attempts.iter().any(|attempt| {
                attempt.outcome == super::model::ExtractionAttemptOutcome::Succeeded
            });
            transaction.execute(
                "INSERT INTO artifact_extraction_state (
                    artifact_id, active_extraction_id,
                    active_quality_report_id, execution_state,
                    quality_status, updated_at, last_error
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(artifact_id) DO UPDATE SET
                    active_extraction_id = excluded.active_extraction_id,
                    active_quality_report_id =
                        excluded.active_quality_report_id,
                    execution_state = excluded.execution_state,
                    quality_status = excluded.quality_status,
                    updated_at = excluded.updated_at,
                    last_error = excluded.last_error",
                params![
                    request.artifact_id,
                    historical.extraction_id,
                    historical.quality_report_id,
                    if current_succeeded {
                        "succeeded"
                    } else {
                        "failed"
                    },
                    historical.quality_status.as_str(),
                    now,
                    current_error,
                ],
            )?;
            transaction.execute(
                "UPDATE extraction_runs SET
                    execution_state = ?2,
                    selected_extraction_id = ?3,
                    fallback_used = 1,
                    fallback_reason = ?4,
                    selection_reason = ?5,
                    completed_at = ?6,
                    last_error = ?7
                 WHERE id = ?1",
                params![
                    run_id,
                    if current_succeeded {
                        "succeeded"
                    } else {
                        "failed"
                    },
                    historical.extraction_id,
                    routed.fallback_reason,
                    routed.selection_reason,
                    now,
                    current_error,
                ],
            )?;
        } else {
            let error = routed
                .attempts
                .last()
                .and_then(|attempt| attempt.attempt.error.clone())
                .unwrap_or_else(|| "no extraction provider produced usable output".to_owned());
            transaction.execute(
                "INSERT INTO artifact_extraction_state (
                    artifact_id, execution_state, quality_status,
                    updated_at, last_error
                 ) VALUES (?1, 'failed', 'failed', ?2, ?3)
                 ON CONFLICT(artifact_id) DO UPDATE SET
                    execution_state = 'failed',
                    quality_status = CASE
                        WHEN artifact_extraction_state.active_extraction_id IS NULL
                        THEN 'failed'
                        ELSE artifact_extraction_state.quality_status
                    END,
                    updated_at = excluded.updated_at,
                    last_error = excluded.last_error",
                params![request.artifact_id, now, error],
            )?;
            transaction.execute(
                "UPDATE extraction_runs SET
                    execution_state = 'failed', completed_at = ?2,
                    last_error = ?3, selection_reason = ?4
                 WHERE id = ?1",
                params![run_id, now, error, routed.selection_reason],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn recompute_quality_reports(
        &self,
        evaluator: &ExtractionQualityEvaluator,
    ) -> AppResult<u32> {
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction()?;
        let rows = {
            let mut statement = transaction.prepare(
                "SELECT de.id, de.canonical_json, de.provenance_json,
                        de.observations_json, de.warnings_json, de.error,
                        epp.capabilities_json
                 FROM document_extractions de
                 JOIN extraction_provider_profiles epp
                   ON epp.id = de.provider_profile_id
                 ORDER BY de.created_at, de.id",
            )?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "UPDATE extraction_quality_profiles SET active = 0
             WHERE active = 1 AND id <> ?1",
            [&evaluator.profile().id],
        )?;
        transaction.execute(
            "INSERT INTO extraction_quality_profiles (
                id, version, rules_json, active, created_at
             ) VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT(id) DO UPDATE SET active = 1",
            params![
                evaluator.profile().id,
                evaluator.profile().version,
                serialize(evaluator.profile())?,
                now,
            ],
        )?;
        let mut inserted = 0_u32;
        for (
            extraction_id,
            canonical_json_value,
            provenance_json,
            observations_json,
            warnings_json,
            error,
            descriptor_json,
        ) in rows
        {
            let attempt = ExtractionAttempt {
                provider: deserialize::<ExtractionProviderDescriptor>(&descriptor_json)?,
                canonical_document: canonical_json_value
                    .as_deref()
                    .map(deserialize::<CanonicalDocumentV1>)
                    .transpose()?,
                provenance: deserialize(&provenance_json)?,
                observations: deserialize::<ExtractionObservations>(&observations_json)?,
                warnings: deserialize(&warnings_json)?,
                error,
            };
            let report = evaluator.evaluate(&attempt);
            let report_id = stable_id("extraction-quality", &[&extraction_id, &report.profile_id]);
            let changed = transaction.execute(
                "INSERT OR IGNORE INTO extraction_quality_reports (
                    id, extraction_id, quality_profile_id,
                    quality_status, usable, signals_json,
                    reasons_json, evaluated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    report_id,
                    extraction_id,
                    report.profile_id,
                    report.status.as_str(),
                    report.usable,
                    serialize(&report.signals)?,
                    serialize(&report.reasons)?,
                    now,
                ],
            )?;
            inserted += changed as u32;
            transaction.execute(
                "UPDATE active_extraction_selections SET
                    quality_report_id = ?2,
                    selection_reason = 'quality profile recomputed for active extraction',
                    updated_at = ?3
                 WHERE extraction_id = ?1",
                params![extraction_id, report_id, now],
            )?;
            transaction.execute(
                "UPDATE artifact_extraction_state SET
                    active_quality_report_id = ?2,
                    quality_status = ?3,
                    updated_at = ?4
                 WHERE active_extraction_id = ?1",
                params![extraction_id, report_id, report.status.as_str(), now,],
            )?;
        }
        transaction.commit()?;
        Ok(inserted)
    }
}

fn ensure_source_text_revision(
    transaction: &rusqlite::Transaction<'_>,
    document_id: &str,
    source_version: i64,
    text_hash: &str,
) -> AppResult<i64> {
    let existing = transaction
        .query_row(
            "SELECT id
             FROM document_revisions
             WHERE document_id = ?1 AND source_version = ?2
               AND revision_key LIKE 'text:%'
             ORDER BY id DESC LIMIT 1",
            params![document_id, source_version],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(revision_id) = existing {
        transaction.execute(
            "UPDATE document_revisions SET
                revision_kind = 'source-provided-text',
                content_hash = ?2,
                artifact_source_binding_id = COALESCE(
                    artifact_source_binding_id,
                    (SELECT b.id
                     FROM document_artifact_source_bindings b
                     WHERE b.artifact_id = ?3 AND b.state = 'active'
                     ORDER BY b.id LIMIT 1)
                )
             WHERE id = ?1",
            params![revision_id, text_hash, document_id],
        )?;
        return Ok(revision_id);
    }
    let revision_key = format!(
        "text:active-projection:{source_version}:{}",
        &text_hash[..16]
    );
    transaction.execute(
        "INSERT INTO document_revisions (
            document_id, source_version, revision_key,
            discovered_at, revision_kind, content_hash,
            artifact_source_binding_id
         ) VALUES (
            ?1, ?2, ?3, ?4, 'source-provided-text', ?5,
            (SELECT b.id
             FROM document_artifact_source_bindings b
             WHERE b.artifact_id = ?1 AND b.state = 'active'
             ORDER BY b.id LIMIT 1)
         )",
        params![
            document_id,
            source_version,
            revision_key,
            Utc::now().to_rfc3339(),
            text_hash,
        ],
    )?;
    Ok(transaction.last_insert_rowid())
}

fn persist_profiles(
    transaction: &rusqlite::Transaction<'_>,
    routing_profile: &ExtractionRoutingProfile,
    quality_evaluator: &ExtractionQualityEvaluator,
    routed: &RoutedExtraction,
    now: &str,
) -> AppResult<()> {
    transaction.execute(
        "INSERT OR IGNORE INTO extraction_routing_profiles (
            id, version, policy_json, active, created_at
         ) VALUES (?1, ?2, ?3, 0, ?4)",
        params![
            routing_profile.id,
            routing_profile.version,
            serialize(routing_profile)?,
            now,
        ],
    )?;
    let quality_profile = quality_evaluator.profile();
    transaction.execute(
        "INSERT OR IGNORE INTO extraction_quality_profiles (
            id, version, rules_json, active, created_at
         ) VALUES (?1, ?2, ?3, 0, ?4)",
        params![
            quality_profile.id,
            quality_profile.version,
            serialize(quality_profile)?,
            now,
        ],
    )?;
    for evaluated in &routed.attempts {
        let descriptor = &evaluated.attempt.provider;
        transaction.execute(
            "INSERT OR IGNORE INTO extraction_provider_profiles (
                id, provider_id, provider_version, mapping_profile,
                supported_media_types_json, capabilities_json,
                configured_available, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                descriptor.profile_id,
                descriptor.provider_id,
                descriptor.provider_version,
                descriptor.mapping_profile,
                serialize(&descriptor.supported_media_types)?,
                serialize(descriptor)?,
                descriptor.configured_available,
                now,
            ],
        )?;
        let stored_identity = transaction.query_row(
            "SELECT provider_id, provider_version, mapping_profile
             FROM extraction_provider_profiles WHERE id = ?1",
            [&descriptor.profile_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        if stored_identity
            != (
                descriptor.provider_id.clone(),
                descriptor.provider_version.clone(),
                descriptor.mapping_profile.clone(),
            )
        {
            return Err(AppError::ApplicationData(format!(
                "extraction provider profile {} was reused with a different provider, version, or mapping profile",
                descriptor.profile_id
            )));
        }
    }
    Ok(())
}

pub(super) fn rebuild_block_index(
    transaction: &rusqlite::Transaction<'_>,
    extraction_id: &str,
    document: &super::model::CanonicalDocumentV1,
) -> AppResult<()> {
    transaction.execute(
        "DELETE FROM canonical_block_index WHERE extraction_id = ?1",
        [extraction_id],
    )?;
    let reading_order = document
        .reading_order
        .iter()
        .enumerate()
        .map(|(index, block_id)| (block_id.as_str(), index as i64))
        .collect::<HashMap<_, _>>();
    for block in &document.blocks {
        let plain_text = match &block.content {
            CanonicalBlockContent::Table { cells, .. } => Some(
                cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\t"),
            ),
            content => content.plain_text().map(str::to_owned),
        };
        transaction.execute(
            "INSERT INTO canonical_block_index (
                extraction_id, block_id, parent_id, block_kind,
                source_order, reading_order, page_start, page_end,
                section_path_json, plain_text
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                extraction_id,
                block.id,
                block.parent_id,
                block.content.kind(),
                block.source_order,
                reading_order.get(block.id.as_str()),
                block.page_range.as_ref().map(|range| range.start),
                block.page_range.as_ref().map(|range| range.end),
                serialize(&block.section_path)?,
                plain_text,
            ],
        )?;
    }
    transaction.execute(
        "DELETE FROM canonical_assets WHERE extraction_id = ?1",
        [extraction_id],
    )?;
    for asset in &document.assets {
        transaction.execute(
            "INSERT INTO canonical_assets (
                extraction_id, asset_id, media_type,
                cache_relative_key, content_hash, byte_count,
                metadata_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                extraction_id,
                asset.id,
                asset.media_type,
                asset.cache_relative_key,
                asset.content_hash,
                asset.byte_count.map(|value| value as i64),
                serialize(&asset.metadata)?,
            ],
        )?;
    }
    Ok(())
}

pub(super) fn serialize<T: serde::Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|error| AppError::ApplicationData(error.to_string()))
}

fn deserialize<T: serde::de::DeserializeOwned>(value: &str) -> AppResult<T> {
    serde_json::from_str(value).map_err(|error| AppError::ApplicationData(error.to_string()))
}

pub(super) fn stable_id(kind: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    for part in parts {
        hasher.update([0x1f]);
        hasher.update(part.as_bytes());
    }
    let suffix = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{kind}_{}", &suffix[..20])
}
