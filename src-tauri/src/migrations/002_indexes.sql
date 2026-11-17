CREATE INDEX source_objects_version_idx
    ON source_objects(library_id, object_type, object_version);
CREATE INDEX research_items_library_year_idx
    ON research_items(library_id, is_trashed, publication_year);
CREATE INDEX research_items_title_idx
    ON research_items(library_id, title);
CREATE INDEX creators_display_idx
    ON creators(normalized_display_name, item_id);
CREATE INDEX item_tags_value_idx
    ON item_tags(normalized_tag, item_id);
CREATE INDEX item_collections_collection_idx
    ON item_collections(collection_id, item_id);
CREATE INDEX attachments_parent_idx
    ON attachments(parent_item_id);
CREATE INDEX notes_parent_idx
    ON notes(parent_item_id);
CREATE INDEX sync_runs_source_idx
    ON sync_runs(source_id, id DESC);
