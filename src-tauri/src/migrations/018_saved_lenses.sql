-- User-authored state deliberately has no FK to rebuildable source caches.
CREATE TABLE research_lenses (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 120),
    question TEXT NOT NULL CHECK(length(question) <= 4000),
    revision INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX research_lenses_library ON research_lenses(library_id);
CREATE TABLE research_lens_members (
    lens_id TEXT NOT NULL REFERENCES research_lenses(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY(lens_id, item_id),
    UNIQUE(lens_id, position)
);
