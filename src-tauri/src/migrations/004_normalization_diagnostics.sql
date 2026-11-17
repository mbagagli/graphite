ALTER TABLE sync_runs
    ADD COLUMN normalization_warnings INTEGER NOT NULL DEFAULT 0;

CREATE TABLE normalization_diagnostics (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    object_type TEXT NOT NULL,
    object_key TEXT NOT NULL,
    object_version INTEGER NOT NULL,
    severity TEXT NOT NULL,
    message TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    PRIMARY KEY(library_id, object_type, object_key)
);

CREATE INDEX normalization_diagnostics_library_idx
    ON normalization_diagnostics(library_id, severity);
