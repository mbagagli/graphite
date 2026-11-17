CREATE TABLE semantic_model_installations (
    model_identifier TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    runtime_version TEXT NOT NULL,
    artifact_identity TEXT NOT NULL,
    cache_relative_path TEXT NOT NULL,
    installed_bytes INTEGER NOT NULL,
    license TEXT NOT NULL,
    status TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    last_checked_at TEXT NOT NULL,
    last_error TEXT
);

CREATE TABLE embedding_profiles (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    model_identifier TEXT NOT NULL,
    model_revision TEXT NOT NULL,
    artifact_identity TEXT NOT NULL,
    embedding_dimension INTEGER NOT NULL,
    similarity_metric TEXT NOT NULL,
    query_template TEXT NOT NULL,
    document_template TEXT NOT NULL,
    normalization TEXT NOT NULL,
    runtime_version TEXT NOT NULL,
    vector_backend TEXT NOT NULL,
    vector_index_format_version TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0, 1)),
    created_at TEXT NOT NULL,
    activated_at TEXT
);

CREATE UNIQUE INDEX idx_embedding_profile_active
    ON embedding_profiles(active) WHERE active = 1;

CREATE TABLE chunking_profiles (
    id TEXT PRIMARY KEY,
    strategy TEXT NOT NULL,
    version TEXT NOT NULL,
    target_tokens INTEGER NOT NULL,
    overlap_tokens INTEGER NOT NULL,
    maximum_tokens INTEGER NOT NULL,
    boundary_policy TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0, 1)),
    created_at TEXT NOT NULL
);

INSERT INTO chunking_profiles (
    id, strategy, version, target_tokens, overlap_tokens,
    maximum_tokens, boundary_policy, active, created_at
) VALUES (
    'scientific-paragraph-v1',
    'paragraph-sentence',
    '1.0.0',
    220,
    36,
    360,
    'paragraph-first; sentence fallback; no mid-word cuts',
    1,
    CURRENT_TIMESTAMP
);

CREATE TABLE semantic_vector_entries (
    vector_key INTEGER PRIMARY KEY AUTOINCREMENT,
    embedding_profile_id TEXT NOT NULL
        REFERENCES embedding_profiles(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    index_kind TEXT NOT NULL CHECK(index_kind IN ('paper', 'chunk')),
    target_id TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0 CHECK(active IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(embedding_profile_id, index_kind, target_id)
);

CREATE INDEX idx_semantic_vector_lookup
    ON semantic_vector_entries(
        embedding_profile_id, library_id, index_kind, active
    );

CREATE TABLE paper_semantic_representations (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL REFERENCES research_items(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    embedding_profile_id TEXT NOT NULL
        REFERENCES embedding_profiles(id) ON DELETE CASCADE,
    representation_version TEXT NOT NULL,
    representation_kind TEXT NOT NULL,
    source_fields TEXT NOT NULL,
    document_id TEXT REFERENCES documents(id) ON DELETE SET NULL,
    document_revision_id INTEGER
        REFERENCES document_revisions(id) ON DELETE SET NULL,
    source_fingerprint TEXT NOT NULL,
    coverage TEXT NOT NULL,
    vector_key INTEGER NOT NULL UNIQUE
        REFERENCES semantic_vector_entries(vector_key),
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(item_id, embedding_profile_id, representation_version)
);

CREATE INDEX idx_paper_semantic_status
    ON paper_semantic_representations(
        embedding_profile_id, library_id, status
    );

CREATE TABLE semantic_chunks (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL REFERENCES research_items(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    document_revision_id INTEGER NOT NULL
        REFERENCES document_revisions(id) ON DELETE CASCADE,
    embedding_profile_id TEXT NOT NULL
        REFERENCES embedding_profiles(id) ON DELETE CASCADE,
    chunking_profile_id TEXT NOT NULL
        REFERENCES chunking_profiles(id),
    source_provider TEXT NOT NULL,
    character_start INTEGER NOT NULL,
    character_end INTEGER NOT NULL,
    page_start INTEGER,
    page_end INTEGER,
    section_heading TEXT,
    ordinal INTEGER NOT NULL,
    token_count INTEGER NOT NULL,
    text TEXT NOT NULL,
    text_fingerprint TEXT NOT NULL,
    vector_key INTEGER NOT NULL UNIQUE
        REFERENCES semantic_vector_entries(vector_key),
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(
        document_revision_id,
        embedding_profile_id,
        chunking_profile_id,
        ordinal
    )
);

CREATE INDEX idx_semantic_chunks_search
    ON semantic_chunks(
        embedding_profile_id, library_id, status, item_id
    );
CREATE INDEX idx_semantic_chunks_document
    ON semantic_chunks(document_id, document_revision_id);

CREATE TABLE semantic_document_states (
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    document_revision_id INTEGER NOT NULL
        REFERENCES document_revisions(id) ON DELETE CASCADE,
    embedding_profile_id TEXT NOT NULL
        REFERENCES embedding_profiles(id) ON DELETE CASCADE,
    chunking_profile_id TEXT NOT NULL
        REFERENCES chunking_profiles(id),
    status TEXT NOT NULL,
    chunk_count INTEGER NOT NULL DEFAULT 0,
    bibliography_excluded INTEGER NOT NULL DEFAULT 0
        CHECK(bibliography_excluded IN (0, 1)),
    updated_at TEXT NOT NULL,
    last_error TEXT,
    PRIMARY KEY(document_id, embedding_profile_id, chunking_profile_id)
);

CREATE TABLE semantic_neighbor_cache (
    edge_id TEXT PRIMARY KEY,
    embedding_profile_id TEXT NOT NULL
        REFERENCES embedding_profiles(id) ON DELETE CASCADE,
    source_item_id TEXT NOT NULL
        REFERENCES research_items(id) ON DELETE CASCADE,
    target_item_id TEXT NOT NULL
        REFERENCES research_items(id) ON DELETE CASCADE,
    representation_version TEXT NOT NULL,
    source_representation TEXT NOT NULL,
    target_representation TEXT NOT NULL,
    similarity_metric TEXT NOT NULL,
    similarity_score REAL NOT NULL,
    mutual INTEGER NOT NULL DEFAULT 0 CHECK(mutual IN (0, 1)),
    created_at TEXT NOT NULL,
    UNIQUE(embedding_profile_id, source_item_id, target_item_id),
    CHECK(source_item_id < target_item_id)
);

CREATE INDEX idx_semantic_neighbor_source
    ON semantic_neighbor_cache(embedding_profile_id, source_item_id);
CREATE INDEX idx_semantic_neighbor_target
    ON semantic_neighbor_cache(embedding_profile_id, target_item_id);

CREATE TABLE semantic_index_state (
    library_id TEXT PRIMARY KEY REFERENCES libraries(id) ON DELETE CASCADE,
    embedding_profile_id TEXT
        REFERENCES embedding_profiles(id) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'model-missing',
    eligible_papers INTEGER NOT NULL DEFAULT 0,
    indexed_papers INTEGER NOT NULL DEFAULT 0,
    stale_papers INTEGER NOT NULL DEFAULT 0,
    eligible_documents INTEGER NOT NULL DEFAULT 0,
    chunked_documents INTEGER NOT NULL DEFAULT 0,
    generated_chunks INTEGER NOT NULL DEFAULT 0,
    indexed_chunks INTEGER NOT NULL DEFAULT 0,
    stale_chunks INTEGER NOT NULL DEFAULT 0,
    metadata_only_papers INTEGER NOT NULL DEFAULT 0,
    vector_index_bytes INTEGER NOT NULL DEFAULT 0,
    chunk_metadata_bytes INTEGER NOT NULL DEFAULT 0,
    last_indexed_at TEXT,
    last_error TEXT
);

CREATE TABLE semantic_index_errors (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    embedding_profile_id TEXT
        REFERENCES embedding_profiles(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL,
    target_id TEXT,
    message TEXT NOT NULL,
    recorded_at TEXT NOT NULL
);

CREATE INDEX idx_semantic_errors_library
    ON semantic_index_errors(library_id, recorded_at DESC);

CREATE TRIGGER paper_semantic_before_delete
BEFORE DELETE ON paper_semantic_representations BEGIN
    UPDATE semantic_vector_entries
    SET active = 0, updated_at = CURRENT_TIMESTAMP
    WHERE vector_key = OLD.vector_key;
END;

CREATE TRIGGER semantic_chunk_before_delete
BEFORE DELETE ON semantic_chunks BEGIN
    UPDATE semantic_vector_entries
    SET active = 0, updated_at = CURRENT_TIMESTAMP
    WHERE vector_key = OLD.vector_key;
END;

CREATE TRIGGER research_item_semantic_metadata_changed
AFTER UPDATE OF title, abstract_text ON research_items
WHEN OLD.title IS NOT NEW.title OR OLD.abstract_text IS NOT NEW.abstract_text
BEGIN
    UPDATE semantic_vector_entries
    SET active = 0, updated_at = CURRENT_TIMESTAMP
    WHERE vector_key IN (
        SELECT vector_key FROM paper_semantic_representations
        WHERE item_id = NEW.id
    );

    UPDATE paper_semantic_representations
    SET status = 'stale', updated_at = CURRENT_TIMESTAMP
    WHERE item_id = NEW.id;

    DELETE FROM semantic_neighbor_cache
    WHERE source_item_id = NEW.id OR target_item_id = NEW.id;
END;

CREATE TRIGGER document_text_semantic_before_update
BEFORE UPDATE OF content, source_version ON document_texts
WHEN OLD.content IS NOT NEW.content OR OLD.source_version IS NOT NEW.source_version
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
        SELECT research_item_id FROM documents WHERE id = OLD.document_id
    ) OR target_item_id IN (
        SELECT research_item_id FROM documents WHERE id = OLD.document_id
    );
END;

CREATE TRIGGER document_text_semantic_after_insert
AFTER INSERT ON document_texts BEGIN
    UPDATE semantic_vector_entries
    SET active = 0, updated_at = CURRENT_TIMESTAMP
    WHERE vector_key IN (
        SELECT psr.vector_key
        FROM paper_semantic_representations psr
        JOIN documents d ON d.research_item_id = psr.item_id
        WHERE d.id = NEW.document_id
          AND psr.representation_kind IN ('title-body', 'title-only')
    );

    UPDATE paper_semantic_representations
    SET status = 'stale', updated_at = CURRENT_TIMESTAMP
    WHERE item_id IN (
        SELECT research_item_id FROM documents WHERE id = NEW.document_id
    ) AND representation_kind IN ('title-body', 'title-only');
END;

CREATE TRIGGER document_text_semantic_before_delete
BEFORE DELETE ON document_texts BEGIN
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
        SELECT research_item_id FROM documents WHERE id = OLD.document_id
    ) OR target_item_id IN (
        SELECT research_item_id FROM documents WHERE id = OLD.document_id
    );
END;

CREATE TRIGGER reference_extraction_semantic_after_insert
AFTER INSERT ON reference_extractions
WHEN NEW.is_current = 1 AND NEW.status = 'references-extracted'
BEGIN
    UPDATE semantic_vector_entries
    SET active = 0, updated_at = CURRENT_TIMESTAMP
    WHERE vector_key IN (
        SELECT vector_key FROM semantic_chunks
        WHERE document_id = NEW.document_id
    );
    DELETE FROM semantic_chunks WHERE document_id = NEW.document_id;
    DELETE FROM semantic_document_states WHERE document_id = NEW.document_id;
END;

CREATE TRIGGER reference_extraction_semantic_before_update
BEFORE UPDATE OF status, section_start, is_current ON reference_extractions
WHEN OLD.status IS NOT NEW.status
  OR OLD.section_start IS NOT NEW.section_start
  OR OLD.is_current IS NOT NEW.is_current
BEGIN
    UPDATE semantic_vector_entries
    SET active = 0, updated_at = CURRENT_TIMESTAMP
    WHERE vector_key IN (
        SELECT vector_key FROM semantic_chunks
        WHERE document_id = OLD.document_id
    );
    DELETE FROM semantic_chunks WHERE document_id = OLD.document_id;
    DELETE FROM semantic_document_states WHERE document_id = OLD.document_id;
END;

CREATE TRIGGER reference_extraction_semantic_before_delete
BEFORE DELETE ON reference_extractions
BEGIN
    UPDATE semantic_vector_entries
    SET active = 0, updated_at = CURRENT_TIMESTAMP
    WHERE vector_key IN (
        SELECT vector_key FROM semantic_chunks
        WHERE document_id = OLD.document_id
    );
    DELETE FROM semantic_chunks WHERE document_id = OLD.document_id;
    DELETE FROM semantic_document_states WHERE document_id = OLD.document_id;
END;
