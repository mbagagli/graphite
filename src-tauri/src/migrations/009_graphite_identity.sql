ALTER TABLE agent_settings RENAME TO agent_settings_before_graphite;

CREATE TABLE agent_settings (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    provider_kind TEXT NOT NULL DEFAULT 'graphite-managed'
        CHECK(provider_kind IN ('graphite-managed', 'openai-compatible')),
    model_profile TEXT NOT NULL DEFAULT 'lightweight'
        CHECK(model_profile IN ('lightweight', 'balanced')),
    external_endpoint TEXT,
    external_model TEXT,
    external_context_tokens INTEGER,
    answer_detail TEXT NOT NULL DEFAULT 'concise'
        CHECK(answer_detail IN ('concise', 'detailed')),
    updated_at TEXT NOT NULL
);

INSERT INTO agent_settings(
    id,
    provider_kind,
    model_profile,
    external_endpoint,
    external_model,
    external_context_tokens,
    answer_detail,
    updated_at
)
SELECT
    id,
    CASE provider_kind
        WHEN 'graphy-managed' THEN 'graphite-managed'
        ELSE provider_kind
    END,
    model_profile,
    external_endpoint,
    external_model,
    external_context_tokens,
    answer_detail,
    updated_at
FROM agent_settings_before_graphite;

DROP TABLE agent_settings_before_graphite;

UPDATE agent_model_installations
SET provider_kind = 'graphite-managed'
WHERE provider_kind = 'graphy-managed';

UPDATE agent_runs
SET provider_json = replace(
        replace(
            provider_json,
            'graphy-managed',
            'graphite-managed'
        ),
        'Graphy',
        'Graphite'
    ),
    prompt_versions_json = replace(
        prompt_versions_json,
        'graphy-grounding-policy-v1',
        'graphite-grounding-policy-v1'
    ),
    result_json = replace(
        replace(
            result_json,
            'graphy-managed',
            'graphite-managed'
        ),
        'Graphy',
        'Graphite'
    )
WHERE provider_json LIKE '%graphy-managed%'
   OR provider_json LIKE '%Graphy%'
   OR prompt_versions_json LIKE '%graphy-grounding-policy-v1%'
   OR result_json LIKE '%graphy-managed%'
   OR result_json LIKE '%Graphy%';

UPDATE agent_sources
SET provider = replace(provider, 'Graphy', 'Graphite')
WHERE provider LIKE '%Graphy%';

UPDATE reference_extractions
SET provider = 'graphite-conservative-text'
WHERE provider = 'graphy-conservative-text';

UPDATE bibliographic_references
SET parser_provider = 'graphite-conservative-text'
WHERE parser_provider = 'graphy-conservative-text';

UPDATE reference_resolutions
SET evidence_json = replace(evidence_json, 'Graphy-owned', 'Graphite-owned')
WHERE evidence_json LIKE '%Graphy-owned%';
