CREATE TABLE active_extraction_selections (
    artifact_id TEXT PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
    extraction_id TEXT NOT NULL
        REFERENCES document_extractions(id) ON DELETE CASCADE,
    quality_report_id TEXT NOT NULL
        REFERENCES extraction_quality_reports(id) ON DELETE CASCADE,
    routing_profile_id TEXT NOT NULL
        REFERENCES extraction_routing_profiles(id),
    selection_reason TEXT NOT NULL,
    selected_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

INSERT INTO active_extraction_selections (
    artifact_id,
    extraction_id,
    quality_report_id,
    routing_profile_id,
    selection_reason,
    selected_at,
    updated_at
)
SELECT
    aes.artifact_id,
    aes.active_extraction_id,
    aes.active_quality_report_id,
    er.routing_profile_id,
    'migration-12: preserved active extraction',
    COALESCE(de.completed_at, de.created_at),
    aes.updated_at
FROM artifact_extraction_state aes
JOIN document_extractions de ON de.id = aes.active_extraction_id
JOIN extraction_runs er ON er.id = de.run_id
WHERE aes.active_extraction_id IS NOT NULL
  AND aes.active_quality_report_id IS NOT NULL;

ALTER TABLE extraction_runs
    ADD COLUMN selection_reason TEXT;

ALTER TABLE document_extractions
    ADD COLUMN attempt_outcome TEXT NOT NULL DEFAULT 'succeeded';

UPDATE document_extractions
SET attempt_outcome = CASE execution_state
    WHEN 'failed' THEN 'failed'
    ELSE 'succeeded'
END;

DROP INDEX idx_selected_document_extraction;

UPDATE document_extractions
SET selected = 0
WHERE selected <> 0;

CREATE INDEX idx_document_extractions_outcome
    ON document_extractions(artifact_id, attempt_outcome, created_at DESC);
