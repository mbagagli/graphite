use rusqlite::OptionalExtension;

use crate::database::ResearchDatabase;
use crate::error::AppResult;
use crate::model::ExtractionDiagnostics;

pub fn load_extraction_diagnostics(
    database: &ResearchDatabase,
) -> AppResult<ExtractionDiagnostics> {
    let connection = database.connect()?;
    let active_library_id = connection
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'active_library_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(library_id) = active_library_id else {
        let mut diagnostics = ExtractionDiagnostics::empty();
        diagnostics.shadow = super::shadow::load_shadow_diagnostics(database)?;
        return Ok(diagnostics);
    };
    let counts = connection.query_row(
        "SELECT
            COUNT(DISTINCT CASE
                WHEN aes.execution_state IS NULL
                  OR aes.execution_state IN ('pending', 'stale')
                THEN d.id END),
            COUNT(DISTINCT CASE
                WHEN aes.quality_status = 'good' THEN d.id END),
            COUNT(DISTINCT CASE
                WHEN aes.quality_status = 'degraded' THEN d.id END),
            COUNT(DISTINCT CASE
                WHEN aes.quality_status = 'failed' THEN d.id END),
            COUNT(DISTINCT CASE
                WHEN de.fallback_used = 1
                THEN d.id END)
         FROM documents d
         JOIN document_texts dt
           ON dt.document_id = d.id AND dt.is_active = 1
         LEFT JOIN artifact_extraction_state aes
           ON aes.artifact_id = d.id
         LEFT JOIN active_extraction_selections selection
           ON selection.artifact_id = d.id
         LEFT JOIN document_extractions de
           ON de.id = selection.extraction_id
         WHERE d.library_id = ?1",
        [&library_id],
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
    let routing_outcomes = connection.query_row(
        "SELECT
            COUNT(CASE WHEN extraction.attempt_outcome = 'unsupported' THEN 1 END),
            COUNT(CASE WHEN extraction.attempt_outcome = 'unavailable' THEN 1 END)
         FROM document_extractions extraction
         JOIN documents document ON document.id = extraction.artifact_id
         WHERE document.library_id = ?1",
        [&library_id],
        |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)),
    )?;
    let active_provider_profile = connection
        .query_row(
            "SELECT epp.id
             FROM extraction_provider_profiles epp
             JOIN document_extractions de
               ON de.provider_profile_id = epp.id
             JOIN active_extraction_selections selection
               ON selection.extraction_id = de.id
             JOIN documents d ON d.id = de.artifact_id
             WHERE d.library_id = ?1
             ORDER BY de.completed_at DESC LIMIT 1",
            [&library_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let routing_profile = connection.query_row(
        "SELECT id FROM extraction_routing_profiles WHERE active = 1",
        [],
        |row| row.get::<_, String>(0),
    )?;
    let quality_profile = connection.query_row(
        "SELECT id FROM extraction_quality_profiles WHERE active = 1",
        [],
        |row| row.get::<_, String>(0),
    )?;
    let (pending, good, degraded, failed, fallback) = counts;
    Ok(ExtractionDiagnostics {
        status: if failed > 0 {
            "attention".to_owned()
        } else if degraded > 0 || pending > 0 {
            "degraded".to_owned()
        } else {
            "ready".to_owned()
        },
        pending,
        good,
        degraded,
        failed,
        fallback,
        unsupported: routing_outcomes.0,
        unavailable: routing_outcomes.1,
        active_provider_profile,
        active_routing_profile: routing_profile,
        active_quality_profile: quality_profile,
        shadow: super::shadow::load_shadow_diagnostics(database)?,
    })
}

#[cfg(test)]
mod tests {
    use crate::model::ExtractionDiagnostics;

    #[test]
    fn diagnostics_contract_exposes_only_counts_status_and_profile_ids() {
        let diagnostics = ExtractionDiagnostics {
            status: "degraded".to_owned(),
            pending: 1,
            good: 2,
            degraded: 3,
            failed: 4,
            fallback: 5,
            unsupported: 6,
            unavailable: 7,
            active_provider_profile: Some("provider-profile-v1".to_owned()),
            active_routing_profile: "routing-profile-v1".to_owned(),
            active_quality_profile: "quality-profile-v1".to_owned(),
            shadow: crate::extraction::shadow::ShadowPilotDiagnostics::disabled("disabled"),
        };
        let value = serde_json::to_value(diagnostics).expect("diagnostics JSON");
        let object = value.as_object().expect("diagnostics object");
        assert!(object.keys().all(|key| !matches!(
            key.as_str(),
            "canonicalJson"
                | "canonicalDocument"
                | "content"
                | "extractedText"
                | "path"
                | "canonicalPath"
                | "provenance"
                | "observations"
        )));
        assert_eq!(object.len(), 12);
        let shadow = object["shadow"].as_object().expect("shadow diagnostics");
        assert!(shadow.keys().all(|key| !matches!(
            key.as_str(),
            "canonicalJson" | "content" | "path" | "provenance" | "observations"
        )));
    }
}
