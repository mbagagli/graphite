CREATE TABLE shadow_extraction_pilot_state (
    singleton_id INTEGER PRIMARY KEY CHECK(singleton_id = 1),
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0, 1)),
    availability TEXT NOT NULL,
    routing_profile_id TEXT NOT NULL,
    max_concurrency INTEGER NOT NULL CHECK(max_concurrency BETWEEN 1 AND 2),
    updated_at TEXT NOT NULL,
    last_error TEXT
);

INSERT INTO shadow_extraction_pilot_state (
    singleton_id, enabled, availability, routing_profile_id,
    max_concurrency, updated_at
) VALUES (
    1, 0, 'disabled', 'pdf-shadow-strategy-v1', 1,
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
);

CREATE TABLE shadow_extraction_jobs (
    id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    artifact_revision_id INTEGER NOT NULL
        REFERENCES document_revisions(id) ON DELETE CASCADE,
    routing_profile_id TEXT NOT NULL
        REFERENCES extraction_routing_profiles(id),
    request_kind TEXT NOT NULL
        CHECK(request_kind IN ('native', 'ocr-fallback', 'structure-demand')),
    execution_state TEXT NOT NULL
        CHECK(execution_state IN (
            'pending', 'running', 'completed', 'failed',
            'cancelled', 'unavailable'
        )),
    routing_reason TEXT NOT NULL,
    cancellation_requested INTEGER NOT NULL DEFAULT 0
        CHECK(cancellation_requested IN (0, 1)),
    created_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    last_error TEXT,
    UNIQUE(artifact_revision_id, routing_profile_id, request_kind)
);

CREATE INDEX idx_shadow_extraction_jobs_state
    ON shadow_extraction_jobs(execution_state, created_at, id);

CREATE TABLE shadow_extraction_attempts (
    job_id TEXT NOT NULL
        REFERENCES shadow_extraction_jobs(id) ON DELETE CASCADE,
    extraction_id TEXT NOT NULL
        REFERENCES document_extractions(id) ON DELETE CASCADE,
    quality_report_id TEXT NOT NULL
        REFERENCES extraction_quality_reports(id) ON DELETE CASCADE,
    provider_role TEXT NOT NULL,
    attempt_outcome TEXT NOT NULL,
    routing_reason TEXT NOT NULL,
    duration_ms INTEGER,
    peak_rss_bytes INTEGER,
    peak_thread_count INTEGER,
    comparison_json TEXT NOT NULL DEFAULT '{}',
    recorded_at TEXT NOT NULL,
    PRIMARY KEY(job_id, extraction_id),
    UNIQUE(extraction_id)
);

CREATE INDEX idx_shadow_extraction_attempts_outcome
    ON shadow_extraction_attempts(attempt_outcome, recorded_at DESC);
