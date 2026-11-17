CREATE TABLE app_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE sources (
    id TEXT PRIMARY KEY,
    server_id TEXT NOT NULL UNIQUE,
    api_version TEXT NOT NULL,
    schema_version TEXT,
    compatibility TEXT NOT NULL,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    connection_status TEXT NOT NULL,
    last_error TEXT
);

CREATE TABLE libraries (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('user', 'group')),
    zotero_library_id TEXT NOT NULL,
    name TEXT NOT NULL,
    successful_version INTEGER NOT NULL DEFAULT 0,
    last_synced_at TEXT,
    last_error TEXT,
    UNIQUE(source_id, kind, zotero_library_id)
);

CREATE TABLE source_objects (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    object_type TEXT NOT NULL,
    object_key TEXT NOT NULL,
    object_version INTEGER NOT NULL,
    raw_json TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0, 1)),
    PRIMARY KEY(library_id, object_type, object_key)
);

CREATE TABLE research_items (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    zotero_key TEXT NOT NULL,
    item_type TEXT NOT NULL,
    title TEXT NOT NULL,
    abstract_text TEXT,
    date_text TEXT,
    publication_year INTEGER,
    venue TEXT,
    volume TEXT,
    issue TEXT,
    pages TEXT,
    publisher TEXT,
    doi TEXT,
    isbn TEXT,
    issn TEXT,
    url TEXT,
    date_added TEXT,
    date_modified TEXT,
    source_version INTEGER NOT NULL,
    is_trashed INTEGER NOT NULL DEFAULT 0 CHECK(is_trashed IN (0, 1)),
    UNIQUE(library_id, zotero_key)
);

CREATE TABLE creators (
    item_id TEXT NOT NULL REFERENCES research_items(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    creator_type TEXT NOT NULL,
    first_name TEXT,
    last_name TEXT,
    field_mode_name TEXT,
    display_name TEXT NOT NULL,
    normalized_display_name TEXT NOT NULL,
    PRIMARY KEY(item_id, position)
);

CREATE TABLE collections (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    zotero_key TEXT NOT NULL,
    name TEXT NOT NULL,
    parent_id TEXT,
    source_version INTEGER NOT NULL,
    UNIQUE(library_id, zotero_key)
);

CREATE TABLE item_collections (
    item_id TEXT NOT NULL REFERENCES research_items(id) ON DELETE CASCADE,
    collection_id TEXT NOT NULL,
    collection_key TEXT NOT NULL,
    PRIMARY KEY(item_id, collection_id)
);

CREATE TABLE item_tags (
    item_id TEXT NOT NULL REFERENCES research_items(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    normalized_tag TEXT NOT NULL,
    tag_type INTEGER,
    PRIMARY KEY(item_id, normalized_tag)
);

CREATE TABLE notes (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    zotero_key TEXT NOT NULL,
    parent_item_id TEXT,
    title TEXT NOT NULL,
    content TEXT,
    source_version INTEGER NOT NULL,
    UNIQUE(library_id, zotero_key)
);

CREATE TABLE attachments (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    zotero_key TEXT NOT NULL,
    parent_item_id TEXT,
    title TEXT NOT NULL,
    content_type TEXT,
    filename TEXT,
    link_mode TEXT,
    available_locally INTEGER NOT NULL DEFAULT 0 CHECK(available_locally IN (0, 1)),
    source_version INTEGER NOT NULL,
    UNIQUE(library_id, zotero_key)
);

CREATE TABLE item_relations (
    item_id TEXT NOT NULL REFERENCES research_items(id) ON DELETE CASCADE,
    predicate TEXT NOT NULL,
    target TEXT NOT NULL,
    PRIMARY KEY(item_id, predicate, target)
);

CREATE TABLE sync_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    libraries_scanned INTEGER NOT NULL,
    objects_changed INTEGER NOT NULL,
    objects_deleted INTEGER NOT NULL,
    error TEXT
);
