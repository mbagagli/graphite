CREATE TABLE documents (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    attachment_id TEXT NOT NULL UNIQUE
        REFERENCES attachments(id) ON DELETE CASCADE,
    research_item_id TEXT
        REFERENCES research_items(id) ON DELETE CASCADE
        DEFERRABLE INITIALLY DEFERRED,
    zotero_attachment_key TEXT NOT NULL,
    media_type TEXT NOT NULL,
    title TEXT NOT NULL,
    filename TEXT,
    link_mode TEXT,
    source_version INTEGER NOT NULL,
    availability TEXT NOT NULL,
    last_error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_documents_library_item
    ON documents(library_id, research_item_id);
CREATE INDEX idx_documents_attachment_key
    ON documents(library_id, zotero_attachment_key);

CREATE TABLE document_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    source_version INTEGER NOT NULL,
    revision_key TEXT NOT NULL,
    file_size INTEGER,
    modified_ms INTEGER,
    discovered_at TEXT NOT NULL,
    UNIQUE(document_id, revision_key)
);

CREATE TABLE document_locators (
    document_id TEXT PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
    canonical_path TEXT NOT NULL,
    revision_key TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    modified_ms INTEGER,
    resolved_at TEXT NOT NULL,
    last_checked_at TEXT NOT NULL
);

CREATE TABLE document_texts (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    zotero_attachment_key TEXT NOT NULL,
    provider TEXT NOT NULL,
    source_version INTEGER NOT NULL,
    content TEXT NOT NULL,
    indexed_pages INTEGER,
    total_pages INTEGER,
    indexed_chars INTEGER,
    total_chars INTEGER,
    completeness TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    UNIQUE(document_id, provider)
);

CREATE INDEX idx_document_texts_library
    ON document_texts(library_id, provider);

CREATE TABLE fulltext_sync_state (
    library_id TEXT PRIMARY KEY REFERENCES libraries(id) ON DELETE CASCADE,
    successful_version INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'not-indexed',
    last_synced_at TEXT,
    changed_documents INTEGER NOT NULL DEFAULT 0,
    warning_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT
);

CREATE TABLE reader_states (
    document_id TEXT PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
    page_number INTEGER NOT NULL DEFAULT 1,
    zoom REAL NOT NULL DEFAULT 1.0,
    fit_mode TEXT NOT NULL DEFAULT 'width',
    outline_open INTEGER NOT NULL DEFAULT 1
        CHECK(outline_open IN (0, 1)),
    context_open INTEGER NOT NULL DEFAULT 1
        CHECK(context_open IN (0, 1)),
    updated_at TEXT NOT NULL
);

CREATE TABLE search_content (
    rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id TEXT NOT NULL UNIQUE
        REFERENCES research_items(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL,
    title TEXT NOT NULL,
    creators TEXT NOT NULL,
    tags TEXT NOT NULL,
    abstract_text TEXT NOT NULL,
    venue TEXT NOT NULL,
    body TEXT NOT NULL
);

CREATE INDEX idx_search_content_library ON search_content(library_id);

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

CREATE TRIGGER search_content_ad AFTER DELETE ON search_content BEGIN
    INSERT INTO research_fts(
        research_fts, rowid, title, creators, tags,
        abstract_text, venue, body
    ) VALUES (
        'delete', old.rowid, old.title, old.creators, old.tags,
        old.abstract_text, old.venue, old.body
    );
END;

CREATE TRIGGER search_content_au AFTER UPDATE ON search_content BEGIN
    INSERT INTO research_fts(
        research_fts, rowid, title, creators, tags,
        abstract_text, venue, body
    ) VALUES (
        'delete', old.rowid, old.title, old.creators, old.tags,
        old.abstract_text, old.venue, old.body
    );
    INSERT INTO research_fts(
        rowid, title, creators, tags, abstract_text, venue, body
    ) VALUES (
        new.rowid, new.title, new.creators, new.tags,
        new.abstract_text, new.venue, new.body
    );
END;

INSERT INTO documents (
    id, source_id, library_id, attachment_id, research_item_id,
    zotero_attachment_key, media_type, title, filename, link_mode,
    source_version, availability, created_at, updated_at
)
SELECT
    'document:' || a.id,
    a.source_id,
    a.library_id,
    a.id,
    a.parent_item_id,
    a.zotero_key,
    COALESCE(a.content_type, 'application/octet-stream'),
    a.title,
    a.filename,
    a.link_mode,
    a.source_version,
    CASE
        WHEN a.content_type = 'application/pdf' THEN 'metadata-only'
        ELSE 'unsupported'
    END,
    CURRENT_TIMESTAMP,
    CURRENT_TIMESTAMP
FROM attachments a;

INSERT INTO search_content (
    item_id, library_id, title, creators, tags,
    abstract_text, venue, body
)
SELECT
    i.id,
    i.library_id,
    i.title,
    COALESCE((
        SELECT group_concat(c.display_name, ' ')
        FROM creators c WHERE c.item_id = i.id
    ), ''),
    COALESCE((
        SELECT group_concat(t.tag, ' ')
        FROM item_tags t WHERE t.item_id = i.id
    ), ''),
    COALESCE(i.abstract_text, ''),
    COALESCE(i.venue, ''),
    ''
FROM research_items i
WHERE i.is_trashed = 0;
