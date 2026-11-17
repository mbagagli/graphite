CREATE TABLE extraction_provider_profiles (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    provider_version TEXT NOT NULL,
    mapping_profile TEXT NOT NULL,
    supported_media_types_json TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    configured_available INTEGER NOT NULL DEFAULT 1
        CHECK(configured_available IN (0, 1)),
    created_at TEXT NOT NULL
);

CREATE TABLE extraction_routing_profiles (
    id TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    policy_json TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0, 1)),
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX idx_active_extraction_routing_profile
    ON extraction_routing_profiles(active) WHERE active = 1;

CREATE TABLE extraction_quality_profiles (
    id TEXT PRIMARY KEY,
    version TEXT NOT NULL,
    rules_json TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0, 1)),
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX idx_active_extraction_quality_profile
    ON extraction_quality_profiles(active) WHERE active = 1;

CREATE TABLE extraction_runs (
    id TEXT PRIMARY KEY,
    artifact_id TEXT REFERENCES documents(id) ON DELETE SET NULL,
    artifact_revision_id INTEGER
        REFERENCES document_revisions(id) ON DELETE SET NULL,
    routing_profile_id TEXT NOT NULL
        REFERENCES extraction_routing_profiles(id),
    execution_state TEXT NOT NULL,
    selected_extraction_id TEXT,
    fallback_used INTEGER NOT NULL DEFAULT 0
        CHECK(fallback_used IN (0, 1)),
    fallback_reason TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    last_error TEXT
);

CREATE INDEX idx_extraction_runs_artifact
    ON extraction_runs(artifact_id, started_at DESC);

CREATE TABLE document_extractions (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES extraction_runs(id) ON DELETE CASCADE,
    artifact_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    artifact_revision_id INTEGER NOT NULL
        REFERENCES document_revisions(id) ON DELETE CASCADE,
    provider_profile_id TEXT NOT NULL
        REFERENCES extraction_provider_profiles(id),
    canonical_schema_version TEXT NOT NULL,
    canonical_json TEXT,
    canonical_hash TEXT,
    plain_text_hash TEXT,
    execution_state TEXT NOT NULL,
    selected INTEGER NOT NULL DEFAULT 0 CHECK(selected IN (0, 1)),
    fallback_used INTEGER NOT NULL DEFAULT 0
        CHECK(fallback_used IN (0, 1)),
    fallback_reason TEXT,
    provenance_json TEXT NOT NULL DEFAULT '{}',
    observations_json TEXT NOT NULL DEFAULT '{}',
    warnings_json TEXT NOT NULL DEFAULT '[]',
    error TEXT,
    created_at TEXT NOT NULL,
    completed_at TEXT
);

CREATE INDEX idx_document_extractions_artifact
    ON document_extractions(artifact_id, created_at DESC);

CREATE UNIQUE INDEX idx_selected_document_extraction
    ON document_extractions(artifact_id) WHERE selected = 1;

CREATE TABLE extraction_quality_reports (
    id TEXT PRIMARY KEY,
    extraction_id TEXT NOT NULL
        REFERENCES document_extractions(id) ON DELETE CASCADE,
    quality_profile_id TEXT NOT NULL
        REFERENCES extraction_quality_profiles(id),
    quality_status TEXT NOT NULL
        CHECK(quality_status IN ('good', 'degraded', 'failed')),
    usable INTEGER NOT NULL CHECK(usable IN (0, 1)),
    signals_json TEXT NOT NULL,
    reasons_json TEXT NOT NULL DEFAULT '[]',
    evaluated_at TEXT NOT NULL,
    UNIQUE(extraction_id, quality_profile_id)
);

CREATE INDEX idx_extraction_quality_status
    ON extraction_quality_reports(quality_status, evaluated_at DESC);

CREATE TABLE artifact_extraction_state (
    artifact_id TEXT PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
    active_extraction_id TEXT
        REFERENCES document_extractions(id) ON DELETE SET NULL,
    active_quality_report_id TEXT
        REFERENCES extraction_quality_reports(id) ON DELETE SET NULL,
    execution_state TEXT NOT NULL DEFAULT 'pending',
    quality_status TEXT,
    updated_at TEXT NOT NULL,
    last_error TEXT
);

CREATE TABLE canonical_block_index (
    extraction_id TEXT NOT NULL
        REFERENCES document_extractions(id) ON DELETE CASCADE,
    block_id TEXT NOT NULL,
    parent_id TEXT,
    block_kind TEXT NOT NULL,
    source_order INTEGER NOT NULL,
    reading_order INTEGER,
    page_start INTEGER,
    page_end INTEGER,
    section_path_json TEXT NOT NULL DEFAULT '[]',
    plain_text TEXT,
    PRIMARY KEY(extraction_id, block_id)
);

CREATE INDEX idx_canonical_blocks_page
    ON canonical_block_index(extraction_id, page_start, page_end);

CREATE TABLE canonical_assets (
    extraction_id TEXT NOT NULL
        REFERENCES document_extractions(id) ON DELETE CASCADE,
    asset_id TEXT NOT NULL,
    media_type TEXT NOT NULL,
    cache_relative_key TEXT,
    content_hash TEXT,
    byte_count INTEGER,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY(extraction_id, asset_id)
);

ALTER TABLE document_texts
    ADD COLUMN extraction_id TEXT
        REFERENCES document_extractions(id) ON DELETE SET NULL;

ALTER TABLE document_texts
    ADD COLUMN projection_profile_id TEXT;

ALTER TABLE document_texts
    ADD COLUMN content_hash TEXT NOT NULL DEFAULT '';

ALTER TABLE document_texts
    ADD COLUMN is_active INTEGER NOT NULL DEFAULT 1
        CHECK(is_active IN (0, 1));

UPDATE document_texts
SET is_active = 0;

UPDATE document_texts
SET is_active = 1
WHERE id IN (
    SELECT selected.id
    FROM document_texts selected
    WHERE selected.id = (
        SELECT candidate.id
        FROM document_texts candidate
        WHERE candidate.document_id = selected.document_id
        ORDER BY
            CASE candidate.provider
                WHEN 'zotero-indexed' THEN 0
                ELSE 1
            END,
            candidate.fetched_at DESC,
            candidate.id
        LIMIT 1
    )
);

UPDATE document_texts
SET projection_profile_id = 'plain-text-v1'
WHERE is_active = 1;

CREATE UNIQUE INDEX idx_active_document_text_projection
    ON document_texts(document_id) WHERE is_active = 1;

CREATE TRIGGER active_document_text_before_deactivate
BEFORE UPDATE OF is_active ON document_texts
WHEN OLD.is_active = 1 AND NEW.is_active = 0
BEGIN
    UPDATE semantic_vector_entries
    SET active = 0, updated_at = CURRENT_TIMESTAMP
    WHERE vector_key IN (
        SELECT vector_key FROM semantic_chunks
        WHERE document_id = OLD.document_id
        UNION ALL
        SELECT vector_key FROM paper_semantic_representations
        WHERE document_id = OLD.document_id
    );

    DELETE FROM semantic_chunks WHERE document_id = OLD.document_id;
    DELETE FROM semantic_document_states WHERE document_id = OLD.document_id;

    UPDATE paper_semantic_representations
    SET status = 'stale', updated_at = CURRENT_TIMESTAMP
    WHERE document_id = OLD.document_id;

    DELETE FROM semantic_neighbor_cache
    WHERE source_item_id IN (
        SELECT research_item_id FROM documents
        WHERE id = OLD.document_id
    ) OR target_item_id IN (
        SELECT research_item_id FROM documents
        WHERE id = OLD.document_id
    );
END;

INSERT INTO extraction_provider_profiles (
    id,
    provider_id,
    provider_version,
    mapping_profile,
    supported_media_types_json,
    capabilities_json,
    configured_available,
    created_at
) VALUES (
    'source-provided-unstructured-text-v1',
    'zotero-indexed-text',
    '1.0.0',
    'canonical-unstructured-text-v1',
    '["application/pdf","text/plain"]',
    '{"profileId":"source-provided-unstructured-text-v1","providerId":"zotero-indexed-text","providerVersion":"1.0.0","mappingProfile":"canonical-unstructured-text-v1","supportedMediaTypes":["application/pdf","text/plain"],"requiredInputForms":["source-provided-text"],"nativeTextCapability":false,"layoutCapability":false,"tableCapability":false,"formulaCapability":false,"figureCapability":false,"ocrCapability":false,"pageProvenanceCapability":false,"boundingBoxCapability":false,"configuredAvailable":true}',
    1,
    CURRENT_TIMESTAMP
);

INSERT INTO extraction_routing_profiles (
    id,
    version,
    policy_json,
    active,
    created_at
) VALUES (
    'source-text-fallback-v1',
    '1.0.0',
    '{"compareDegraded":true,"allowUsableDegraded":true,"mergeOutputs":false}',
    1,
    CURRENT_TIMESTAMP
);

INSERT INTO extraction_quality_profiles (
    id,
    version,
    rules_json,
    active,
    created_at
) VALUES (
    'extraction-quality-v1',
    '1.0.0',
    '{"requireSearchableText":true,"flatPdfTextIsDegraded":true,"maximumCoverageDiscrepancy":0.15}',
    1,
    CURRENT_TIMESTAMP
);

INSERT INTO artifact_extraction_state (
    artifact_id,
    execution_state,
    updated_at
)
SELECT d.id, 'pending', CURRENT_TIMESTAMP
FROM documents d
WHERE EXISTS (
    SELECT 1
    FROM document_texts dt
    WHERE dt.document_id = d.id AND dt.is_active = 1
);
