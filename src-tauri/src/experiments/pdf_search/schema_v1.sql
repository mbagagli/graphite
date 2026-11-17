PRAGMA foreign_keys = ON;

CREATE TABLE experiment_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE experiment_executions (
    id TEXT PRIMARY KEY,
    definition_id TEXT NOT NULL,
    mode TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN (
        'staging', 'ready', 'cancelled', 'failed'
    )),
    started_at TEXT NOT NULL,
    completed_at TEXT,
    last_error TEXT
);

CREATE TABLE experiment_items (
    item_id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    title TEXT NOT NULL,
    creators TEXT NOT NULL,
    tags TEXT NOT NULL,
    abstract_text TEXT NOT NULL,
    venue TEXT NOT NULL,
    document_id TEXT NOT NULL,
    document_revision_id INTEGER NOT NULL
);

CREATE TABLE experiment_projections (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL REFERENCES experiment_items(item_id) ON DELETE CASCADE,
    document_id TEXT NOT NULL,
    document_revision_id INTEGER NOT NULL,
    source_kind TEXT NOT NULL CHECK(source_kind IN (
        'zotero', 'xberg', 'pdf-inspector-ocr', 'docling'
    )),
    extraction_id TEXT,
    provider_profile TEXT NOT NULL,
    content TEXT NOT NULL,
    content_fingerprint TEXT NOT NULL,
    UNIQUE(item_id, source_kind, content_fingerprint)
);

CREATE TABLE search_content (
    rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT NOT NULL UNIQUE REFERENCES experiment_items(item_id) ON DELETE CASCADE,
    library_id TEXT NOT NULL,
    title TEXT NOT NULL,
    creators TEXT NOT NULL,
    tags TEXT NOT NULL,
    abstract_text TEXT NOT NULL,
    venue TEXT NOT NULL,
    body TEXT NOT NULL
);

CREATE VIRTUAL TABLE research_fts USING fts5(
    title,
    creators,
    tags,
    abstract_text,
    venue,
    body,
    content='search_content',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER search_content_ai AFTER INSERT ON search_content BEGIN
    INSERT INTO research_fts(
        rowid, title, creators, tags, abstract_text, venue, body
    ) VALUES (
        new.rowid, new.title, new.creators, new.tags,
        new.abstract_text, new.venue, new.body
    );
END;

CREATE TABLE experiment_chunks (
    id TEXT PRIMARY KEY,
    logical_chunk_id TEXT NOT NULL,
    item_id TEXT NOT NULL REFERENCES experiment_items(item_id) ON DELETE CASCADE,
    document_id TEXT NOT NULL,
    document_revision_id INTEGER NOT NULL,
    ordinal INTEGER NOT NULL,
    text TEXT NOT NULL,
    character_start INTEGER NOT NULL,
    character_end INTEGER NOT NULL,
    page_start INTEGER,
    page_end INTEGER,
    token_count INTEGER NOT NULL,
    content_fingerprint TEXT NOT NULL,
    vector_key INTEGER NOT NULL UNIQUE,
    UNIQUE(item_id, content_fingerprint)
);

CREATE TABLE experiment_chunk_provenance (
    chunk_id TEXT NOT NULL REFERENCES experiment_chunks(id) ON DELETE CASCADE,
    projection_id TEXT NOT NULL REFERENCES experiment_projections(id) ON DELETE CASCADE,
    source_kind TEXT NOT NULL,
    extraction_id TEXT,
    provider_profile TEXT NOT NULL,
    page_start INTEGER,
    page_end INTEGER,
    PRIMARY KEY(chunk_id, projection_id)
);

CREATE TABLE experiment_vector_entries (
    vector_key INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('paper', 'chunk')),
    target_id TEXT NOT NULL,
    embedding_profile_id TEXT NOT NULL,
    UNIQUE(kind, target_id)
);

CREATE TABLE experiment_paper_representations (
    item_id TEXT PRIMARY KEY REFERENCES experiment_items(item_id) ON DELETE CASCADE,
    representation_kind TEXT NOT NULL,
    vector_key INTEGER NOT NULL UNIQUE REFERENCES experiment_vector_entries(vector_key)
);
