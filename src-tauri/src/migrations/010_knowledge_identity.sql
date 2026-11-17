CREATE TABLE source_instances (
    id TEXT PRIMARY KEY,
    provider_kind TEXT NOT NULL,
    display_name TEXT NOT NULL,
    external_identity TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active',
    last_successful_cursor TEXT,
    last_synced_at TEXT,
    last_error TEXT,
    legacy_source_id TEXT REFERENCES sources(id) ON DELETE SET NULL,
    legacy_library_id TEXT UNIQUE
        REFERENCES libraries(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(provider_kind, external_identity)
);

CREATE INDEX idx_source_instances_provider_state
    ON source_instances(provider_kind, state);

CREATE TABLE knowledge_item_source_bindings (
    id TEXT PRIMARY KEY,
    knowledge_item_id TEXT
        REFERENCES research_items(id) ON DELETE SET NULL,
    source_instance_id TEXT NOT NULL
        REFERENCES source_instances(id) ON DELETE CASCADE,
    external_item_id TEXT NOT NULL,
    external_item_kind TEXT NOT NULL,
    source_revision TEXT,
    normalized_source_metadata_json TEXT NOT NULL DEFAULT '{}',
    metadata_hash TEXT NOT NULL,
    is_primary INTEGER NOT NULL DEFAULT 1 CHECK(is_primary IN (0, 1)),
    state TEXT NOT NULL DEFAULT 'active',
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    deleted_at TEXT,
    UNIQUE(source_instance_id, external_item_id)
);

CREATE INDEX idx_knowledge_bindings_item_state
    ON knowledge_item_source_bindings(knowledge_item_id, state);

CREATE TABLE document_artifact_source_bindings (
    id TEXT PRIMARY KEY,
    artifact_id TEXT REFERENCES documents(id) ON DELETE SET NULL,
    source_instance_id TEXT NOT NULL
        REFERENCES source_instances(id) ON DELETE CASCADE,
    knowledge_item_source_binding_id TEXT
        REFERENCES knowledge_item_source_bindings(id) ON DELETE SET NULL,
    external_artifact_id TEXT NOT NULL,
    source_revision TEXT,
    source_metadata_json TEXT NOT NULL DEFAULT '{}',
    metadata_hash TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'active',
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    deleted_at TEXT,
    UNIQUE(source_instance_id, external_artifact_id)
);

CREATE INDEX idx_artifact_bindings_artifact_state
    ON document_artifact_source_bindings(artifact_id, state);

ALTER TABLE research_items
    ADD COLUMN knowledge_kind TEXT NOT NULL DEFAULT 'other';

UPDATE research_items
SET knowledge_kind = CASE item_type
    WHEN 'journalArticle' THEN 'paper'
    WHEN 'conferencePaper' THEN 'paper'
    WHEN 'preprint' THEN 'paper'
    WHEN 'book' THEN 'book'
    WHEN 'bookSection' THEN 'book'
    WHEN 'report' THEN 'report'
    WHEN 'thesis' THEN 'report'
    WHEN 'manuscript' THEN 'report'
    WHEN 'presentation' THEN 'presentation'
    WHEN 'dataset' THEN 'dataset'
    ELSE 'other'
END;

ALTER TABLE documents
    ADD COLUMN artifact_role TEXT NOT NULL DEFAULT 'attachment';

ALTER TABLE document_revisions
    ADD COLUMN revision_kind TEXT NOT NULL DEFAULT 'legacy-unknown';

ALTER TABLE document_revisions
    ADD COLUMN content_hash TEXT;

ALTER TABLE document_revisions
    ADD COLUMN artifact_source_binding_id TEXT
        REFERENCES document_artifact_source_bindings(id) ON DELETE SET NULL;

ALTER TABLE document_locators
    ADD COLUMN artifact_source_binding_id TEXT
        REFERENCES document_artifact_source_bindings(id) ON DELETE SET NULL;

ALTER TABLE document_locators
    ADD COLUMN locator_kind TEXT NOT NULL DEFAULT 'validated-local-file';

INSERT INTO source_instances (
    id,
    provider_kind,
    display_name,
    external_identity,
    state,
    last_successful_cursor,
    last_synced_at,
    last_error,
    legacy_source_id,
    legacy_library_id,
    created_at,
    updated_at
)
SELECT
    'source-instance:' || l.id,
    'zotero',
    l.name,
    s.server_id || ':' || l.kind || ':' || l.zotero_library_id,
    CASE WHEN l.available = 1 THEN 'active' ELSE 'unavailable' END,
    CAST(l.successful_version AS TEXT),
    l.last_synced_at,
    l.last_error,
    l.source_id,
    l.id,
    COALESCE(l.last_synced_at, CURRENT_TIMESTAMP),
    CURRENT_TIMESTAMP
FROM libraries l
JOIN sources s ON s.id = l.source_id;

INSERT INTO knowledge_item_source_bindings (
    id,
    knowledge_item_id,
    source_instance_id,
    external_item_id,
    external_item_kind,
    source_revision,
    normalized_source_metadata_json,
    metadata_hash,
    is_primary,
    state,
    first_seen_at,
    last_seen_at
)
SELECT
    'knowledge-binding:' || i.id,
    i.id,
    si.id,
    i.zotero_key,
    i.item_type,
    CAST(i.source_version AS TEXT),
    json_object(
        'itemKind', i.item_type,
        'dateAdded', i.date_added,
        'dateModified', i.date_modified
    ),
    '',
    1,
    CASE WHEN i.is_trashed = 1 THEN 'deleted' ELSE 'active' END,
    COALESCE(i.date_added, CURRENT_TIMESTAMP),
    COALESCE(i.date_modified, CURRENT_TIMESTAMP)
FROM research_items i
JOIN source_instances si ON si.legacy_library_id = i.library_id;

INSERT INTO document_artifact_source_bindings (
    id,
    artifact_id,
    source_instance_id,
    knowledge_item_source_binding_id,
    external_artifact_id,
    source_revision,
    source_metadata_json,
    metadata_hash,
    state,
    first_seen_at,
    last_seen_at
)
SELECT
    'artifact-binding:' || d.id,
    d.id,
    si.id,
    kib.id,
    d.zotero_attachment_key,
    CAST(d.source_version AS TEXT),
    json_object(
        'mediaType', d.media_type,
        'linkMode', d.link_mode,
        'filename', d.filename
    ),
    '',
    'active',
    d.created_at,
    d.updated_at
FROM documents d
JOIN source_instances si ON si.legacy_library_id = d.library_id
LEFT JOIN knowledge_item_source_bindings kib
    ON kib.knowledge_item_id = d.research_item_id
   AND kib.source_instance_id = si.id;

UPDATE document_revisions
SET revision_kind = 'source-provided-text'
WHERE revision_key LIKE 'text:zotero-indexed:%';

UPDATE document_revisions
SET revision_kind = 'artifact-content'
WHERE EXISTS (
    SELECT 1
    FROM document_locators dl
    WHERE dl.document_id = document_revisions.document_id
      AND dl.revision_key = document_revisions.revision_key
);

UPDATE document_revisions
SET artifact_source_binding_id = (
    SELECT b.id
    FROM document_artifact_source_bindings b
    WHERE b.artifact_id = document_revisions.document_id
    ORDER BY b.id
    LIMIT 1
);

UPDATE document_locators
SET artifact_source_binding_id = (
    SELECT b.id
    FROM document_artifact_source_bindings b
    WHERE b.artifact_id = document_locators.document_id
    ORDER BY b.id
    LIMIT 1
);
