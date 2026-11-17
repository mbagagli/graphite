ALTER TABLE agent_settings ADD COLUMN inference_backend TEXT
    NOT NULL DEFAULT 'auto' CHECK (inference_backend IN ('auto', 'cpu', 'gpu'));
ALTER TABLE agent_settings ADD COLUMN context_window TEXT
    NOT NULL DEFAULT 'medium'
    CHECK (context_window IN ('low', 'medium', 'high', 'extra-high'));
