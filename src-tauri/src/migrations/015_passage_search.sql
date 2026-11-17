-- Index existing chunk text without rebuilding embeddings on selection changes.
CREATE VIRTUAL TABLE semantic_chunk_fts USING fts5(
    text, content='semantic_chunks', content_rowid='rowid',
    tokenize='porter unicode61'
);
CREATE TRIGGER semantic_chunk_fts_insert AFTER INSERT ON semantic_chunks BEGIN
    INSERT INTO semantic_chunk_fts(rowid, text) VALUES (new.rowid, new.text);
END;
CREATE TRIGGER semantic_chunk_fts_delete AFTER DELETE ON semantic_chunks BEGIN
    INSERT INTO semantic_chunk_fts(semantic_chunk_fts, rowid, text)
    VALUES ('delete', old.rowid, old.text);
END;
CREATE TRIGGER semantic_chunk_fts_update AFTER UPDATE OF text ON semantic_chunks
BEGIN
    INSERT INTO semantic_chunk_fts(semantic_chunk_fts, rowid, text)
    VALUES ('delete', old.rowid, old.text);
    INSERT INTO semantic_chunk_fts(rowid, text) VALUES (new.rowid, new.text);
END;
INSERT INTO semantic_chunk_fts(semantic_chunk_fts) VALUES ('rebuild');
