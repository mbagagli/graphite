-- Inactive parser alternatives must not invalidate the active index.
DROP TRIGGER document_text_semantic_before_update;
DROP TRIGGER document_text_semantic_after_insert;
DROP TRIGGER document_text_semantic_before_delete;

CREATE TRIGGER document_text_semantic_before_update
BEFORE UPDATE OF content, source_version, is_active ON document_texts
WHEN (OLD.is_active = 1 OR NEW.is_active = 1)
 AND (OLD.content IS NOT NEW.content
      OR OLD.source_version IS NOT NEW.source_version
      OR OLD.is_active IS NOT NEW.is_active)
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
AFTER INSERT ON document_texts WHEN NEW.is_active = 1 BEGIN
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
BEFORE DELETE ON document_texts WHEN OLD.is_active = 1 BEGIN
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

