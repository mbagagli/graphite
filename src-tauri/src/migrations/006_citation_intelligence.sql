CREATE TABLE research_item_match_keys (
    item_id TEXT PRIMARY KEY
        REFERENCES research_items(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    normalized_doi TEXT,
    normalized_title TEXT NOT NULL,
    publication_year INTEGER,
    first_author_surname TEXT,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_match_keys_doi
    ON research_item_match_keys(source_id, normalized_doi)
    WHERE normalized_doi IS NOT NULL;
CREATE INDEX idx_match_keys_author_year
    ON research_item_match_keys(
        source_id, publication_year, first_author_surname
    );
CREATE INDEX idx_match_keys_title
    ON research_item_match_keys(source_id, normalized_title);

CREATE TABLE reference_extractions (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    source_item_id TEXT NOT NULL
        REFERENCES research_items(id) ON DELETE CASCADE,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    document_revision_id INTEGER NOT NULL
        REFERENCES document_revisions(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    extractor_version TEXT NOT NULL,
    source_text_provider TEXT NOT NULL,
    source_text_version INTEGER NOT NULL,
    source_text_hash TEXT NOT NULL,
    source_completeness TEXT NOT NULL,
    status TEXT NOT NULL,
    section_heading TEXT,
    section_start INTEGER,
    section_end INTEGER,
    reference_count INTEGER NOT NULL DEFAULT 0,
    diagnostic TEXT,
    is_current INTEGER NOT NULL DEFAULT 1
        CHECK(is_current IN (0, 1)),
    indexed_at TEXT NOT NULL,
    UNIQUE(
        document_id, extractor_version,
        source_text_version, source_text_hash
    )
);

CREATE UNIQUE INDEX idx_current_reference_extraction
    ON reference_extractions(document_id)
    WHERE is_current = 1;
CREATE INDEX idx_reference_extractions_library_status
    ON reference_extractions(library_id, status, is_current);
CREATE INDEX idx_reference_extractions_item
    ON reference_extractions(source_item_id, is_current);

CREATE TABLE bibliographic_references (
    id TEXT PRIMARY KEY,
    extraction_id TEXT NOT NULL
        REFERENCES reference_extractions(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    source_item_id TEXT NOT NULL
        REFERENCES research_items(id) ON DELETE CASCADE,
    source_document_id TEXT NOT NULL
        REFERENCES documents(id) ON DELETE CASCADE,
    source_document_revision_id INTEGER NOT NULL
        REFERENCES document_revisions(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    raw_text TEXT NOT NULL,
    original_doi TEXT,
    normalized_doi TEXT,
    parsed_title TEXT,
    parsed_year INTEGER,
    first_author TEXT,
    parsed_creators_json TEXT NOT NULL DEFAULT '[]',
    parsed_venue TEXT,
    identifiers_json TEXT NOT NULL DEFAULT '{}',
    parser_provider TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    extraction_confidence TEXT NOT NULL,
    page_number INTEGER,
    source_start INTEGER,
    source_end INTEGER,
    reference_fingerprint TEXT NOT NULL,
    is_current INTEGER NOT NULL DEFAULT 1 CHECK(is_current IN (0, 1)),
    created_at TEXT NOT NULL,
    UNIQUE(extraction_id, ordinal)
);

CREATE INDEX idx_references_source_item
    ON bibliographic_references(source_item_id, extraction_id, ordinal);
CREATE INDEX idx_references_doi
    ON bibliographic_references(source_id, normalized_doi)
    WHERE normalized_doi IS NOT NULL;
CREATE INDEX idx_references_metadata
    ON bibliographic_references(source_id, parsed_year, first_author);

CREATE TABLE reference_identifiers (
    reference_id TEXT NOT NULL
        REFERENCES bibliographic_references(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    original_value TEXT NOT NULL,
    normalized_value TEXT NOT NULL,
    PRIMARY KEY(reference_id, kind, normalized_value)
);

CREATE INDEX idx_reference_identifier_lookup
    ON reference_identifiers(kind, normalized_value);

CREATE TABLE referenced_works (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    identity_kind TEXT NOT NULL,
    canonical_fingerprint TEXT NOT NULL,
    normalized_doi TEXT,
    title TEXT,
    publication_year INTEGER,
    first_author TEXT,
    venue TEXT,
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    UNIQUE(source_id, canonical_fingerprint)
);

CREATE INDEX idx_referenced_works_doi
    ON referenced_works(source_id, normalized_doi)
    WHERE normalized_doi IS NOT NULL;

CREATE TABLE reference_resolutions (
    reference_id TEXT PRIMARY KEY
        REFERENCES bibliographic_references(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    matched_item_id TEXT
        REFERENCES research_items(id) ON DELETE SET NULL,
    external_work_id TEXT
        REFERENCES referenced_works(id) ON DELETE SET NULL,
    method TEXT NOT NULL,
    confidence_category TEXT NOT NULL,
    score REAL,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    resolver_version TEXT NOT NULL,
    resolved_at TEXT NOT NULL
);

CREATE INDEX idx_reference_resolutions_item
    ON reference_resolutions(matched_item_id, status);
CREATE INDEX idx_reference_resolutions_external
    ON reference_resolutions(external_work_id, status);

CREATE TABLE reference_resolution_candidates (
    reference_id TEXT NOT NULL
        REFERENCES bibliographic_references(id) ON DELETE CASCADE,
    candidate_item_id TEXT NOT NULL
        REFERENCES research_items(id) ON DELETE CASCADE,
    category TEXT NOT NULL,
    score REAL NOT NULL,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    rank INTEGER NOT NULL,
    PRIMARY KEY(reference_id, candidate_item_id)
);

CREATE INDEX idx_resolution_candidates_reference
    ON reference_resolution_candidates(reference_id, rank);

CREATE TABLE reference_resolution_overrides (
    reference_id TEXT PRIMARY KEY
        REFERENCES bibliographic_references(id) ON DELETE CASCADE,
    decision TEXT NOT NULL
        CHECK(decision IN ('accept', 'reject', 'unresolved')),
    target_item_id TEXT
        REFERENCES research_items(id) ON DELETE SET NULL,
    source_reference_fingerprint TEXT NOT NULL,
    stale INTEGER NOT NULL DEFAULT 0 CHECK(stale IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE citation_edges (
    id TEXT PRIMARY KEY,
    source_item_id TEXT NOT NULL
        REFERENCES research_items(id) ON DELETE CASCADE,
    target_item_id TEXT NOT NULL
        REFERENCES research_items(id) ON DELETE CASCADE,
    evidence_count INTEGER NOT NULL,
    strongest_category TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(source_item_id, target_item_id),
    CHECK(source_item_id <> target_item_id)
);

CREATE INDEX idx_citation_edges_source
    ON citation_edges(source_item_id, target_item_id);
CREATE INDEX idx_citation_edges_target
    ON citation_edges(target_item_id, source_item_id);

CREATE TABLE citation_evidence (
    citation_edge_id TEXT NOT NULL
        REFERENCES citation_edges(id) ON DELETE CASCADE,
    reference_id TEXT NOT NULL UNIQUE
        REFERENCES bibliographic_references(id) ON DELETE CASCADE,
    PRIMARY KEY(citation_edge_id, reference_id)
);

CREATE TABLE citation_resolution_queue (
    reference_id TEXT PRIMARY KEY
        REFERENCES bibliographic_references(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    queued_at TEXT NOT NULL
);

CREATE TABLE citation_index_state (
    library_id TEXT PRIMARY KEY REFERENCES libraries(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'not-indexed',
    extractor_version TEXT NOT NULL,
    resolver_version TEXT NOT NULL,
    eligible_documents INTEGER NOT NULL DEFAULT 0,
    indexed_documents INTEGER NOT NULL DEFAULT 0,
    no_reference_section INTEGER NOT NULL DEFAULT 0,
    partial_documents INTEGER NOT NULL DEFAULT 0,
    failed_documents INTEGER NOT NULL DEFAULT 0,
    references_extracted INTEGER NOT NULL DEFAULT 0,
    references_with_doi INTEGER NOT NULL DEFAULT 0,
    resolved_local INTEGER NOT NULL DEFAULT 0,
    external_works INTEGER NOT NULL DEFAULT 0,
    ambiguous_references INTEGER NOT NULL DEFAULT 0,
    unresolved_references INTEGER NOT NULL DEFAULT 0,
    citation_edges INTEGER NOT NULL DEFAULT 0,
    queued_documents INTEGER NOT NULL DEFAULT 0,
    last_indexed_at TEXT,
    last_error TEXT
);

CREATE TRIGGER research_item_target_before_delete
BEFORE DELETE ON research_items BEGIN
    INSERT OR REPLACE INTO citation_resolution_queue(
        reference_id, reason, queued_at
    )
    SELECT reference_id, 'local-target-deleted', CURRENT_TIMESTAMP
    FROM reference_resolutions
    WHERE matched_item_id = OLD.id;

    UPDATE reference_resolutions
    SET status = 'unresolved',
        matched_item_id = NULL,
        method = 'local-target-deleted',
        confidence_category = 'unresolved',
        resolved_at = CURRENT_TIMESTAMP
    WHERE matched_item_id = OLD.id;
END;
