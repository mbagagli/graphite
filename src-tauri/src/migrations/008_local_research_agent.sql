CREATE TABLE agent_settings (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    provider_kind TEXT NOT NULL DEFAULT 'graphy-managed'
        CHECK(provider_kind IN ('graphy-managed', 'openai-compatible')),
    model_profile TEXT NOT NULL DEFAULT 'lightweight'
        CHECK(model_profile IN ('lightweight', 'balanced')),
    external_endpoint TEXT,
    external_model TEXT,
    external_context_tokens INTEGER,
    answer_detail TEXT NOT NULL DEFAULT 'concise'
        CHECK(answer_detail IN ('concise', 'detailed')),
    updated_at TEXT NOT NULL
);

INSERT INTO agent_settings(id, updated_at) VALUES (1, CURRENT_TIMESTAMP);

CREATE TABLE agent_model_installations (
    profile_id TEXT PRIMARY KEY,
    provider_kind TEXT NOT NULL,
    model_identifier TEXT NOT NULL,
    model_revision TEXT NOT NULL,
    artifact_identity TEXT NOT NULL,
    artifact_filename TEXT NOT NULL,
    quantization TEXT NOT NULL,
    expected_bytes INTEGER NOT NULL,
    installed_bytes INTEGER NOT NULL,
    sha256 TEXT NOT NULL,
    context_capacity INTEGER NOT NULL,
    cache_relative_path TEXT NOT NULL,
    license TEXT NOT NULL,
    status TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    last_checked_at TEXT NOT NULL,
    last_error TEXT
);

CREATE TABLE agent_conversations (
    id TEXT PRIMARY KEY,
    library_id TEXT REFERENCES libraries(id) ON DELETE SET NULL,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_agent_conversations_updated
    ON agent_conversations(updated_at DESC);

CREATE TABLE agent_runs (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL
        REFERENCES agent_conversations(id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    intent TEXT NOT NULL,
    scope_json TEXT NOT NULL,
    priority_context_json TEXT NOT NULL,
    provider_json TEXT NOT NULL,
    generation_json TEXT NOT NULL,
    prompt_versions_json TEXT NOT NULL,
    tool_calls_json TEXT NOT NULL,
    retrieval_json TEXT NOT NULL,
    context_json TEXT NOT NULL,
    answer_markdown TEXT,
    citations_json TEXT NOT NULL DEFAULT '[]',
    limitations_json TEXT NOT NULL DEFAULT '[]',
    actions_json TEXT NOT NULL DEFAULT '[]',
    timings_json TEXT NOT NULL DEFAULT '{}',
    result_json TEXT,
    completion_state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    completed_at TEXT
);

CREATE INDEX idx_agent_runs_conversation
    ON agent_runs(conversation_id, created_at);

CREATE TABLE agent_turns (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL
        REFERENCES agent_conversations(id) ON DELETE CASCADE,
    agent_run_id TEXT REFERENCES agent_runs(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK(role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_agent_turns_conversation
    ON agent_turns(conversation_id, created_at);

CREATE TABLE agent_sources (
    run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL,
    evidence_kind TEXT NOT NULL,
    item_id TEXT,
    item_title TEXT NOT NULL,
    document_id TEXT,
    document_revision_id INTEGER,
    chunk_id TEXT,
    passage_text TEXT,
    character_start INTEGER,
    character_end INTEGER,
    page_start INTEGER,
    page_end INTEGER,
    retrieval_modes_json TEXT NOT NULL DEFAULT '[]',
    rank_json TEXT NOT NULL DEFAULT '{}',
    provider TEXT NOT NULL,
    source_fingerprint TEXT NOT NULL,
    eligible_for_scientific_claim INTEGER NOT NULL
        CHECK(eligible_for_scientific_claim IN (0, 1)),
    PRIMARY KEY(run_id, source_id)
);

CREATE INDEX idx_agent_sources_item
    ON agent_sources(item_id, run_id);
