pub(crate) mod lenses;
mod read_model;

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::PathBuf;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};

use crate::bibliography::{normalize_author_surname, normalize_doi, normalize_title};
use crate::error::{AppError, AppResult};
use crate::knowledge::model::{
    DocumentArtifactRecord, KnowledgeItemRecord, SourceDelta, SourceGroupRecord,
};
use crate::model::{
    Author, CorpusDiagnostics, DocumentAttachment, DocumentTextPayload, DocumentTextSummary,
    FullTextSyncDelta, ItemPage, ItemProvenance, ItemQueryRequest, LibraryDelta, LibraryDescriptor,
    RawObject, ReaderState, RemoteLibrary, ResearchCollection, ResearchItem, ResearchNote,
    SearchPage, SearchRequest, SearchResult, SourceDiagnostics, SourceProbe, SyncSummary,
};
use crate::normalize::{normalize_collection, normalize_item, NormalizedItem};

const MIGRATION_1: &str = include_str!("migrations/001_initial.sql");
const MIGRATION_2: &str = include_str!("migrations/002_indexes.sql");
const MIGRATION_3: &str = include_str!("migrations/003_library_availability.sql");
const MIGRATION_4: &str = include_str!("migrations/004_normalization_diagnostics.sql");
const MIGRATION_5: &str = include_str!("migrations/005_document_corpus.sql");
const MIGRATION_6: &str = include_str!("migrations/006_citation_intelligence.sql");
const MIGRATION_7: &str = include_str!("migrations/007_semantic_retrieval.sql");
const MIGRATION_8: &str = include_str!("migrations/008_local_research_agent.sql");
const MIGRATION_9: &str = include_str!("migrations/009_graphite_identity.sql");
const MIGRATION_10: &str = include_str!("migrations/010_knowledge_identity.sql");
const MIGRATION_11: &str = include_str!("migrations/011_canonical_extraction.sql");
const MIGRATION_12: &str = include_str!("migrations/012_extraction_selection_hardening.sql");
const MIGRATION_13: &str = include_str!("migrations/013_extraction_quality_hardening.sql");
const MIGRATION_17: &str = include_str!("migrations/017_inference_preferences.sql");
const MIGRATION_16: &str = include_str!("migrations/016_active_extraction_indexing.sql");
const MIGRATION_15: &str = include_str!("migrations/015_passage_search.sql");
const MIGRATION_14: &str = include_str!("migrations/014_pdf_shadow_pilot.sql");
const MIGRATION_18: &str = include_str!("migrations/018_saved_lenses.sql");
const MAX_PAGE_LIMIT: u32 = 200;

#[derive(Clone)]
pub struct ResearchDatabase {
    path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct StoredSource {
    pub id: String,
    pub changed: bool,
}

#[derive(Clone, Debug)]
pub struct StoredLibrary {
    pub id: String,
    pub remote: RemoteLibrary,
    pub version: i64,
}

#[derive(Clone, Debug)]
pub struct DocumentResolutionContext {
    pub document_id: String,
    pub provider_kind: String,
    pub remote: RemoteLibrary,
    pub server_id: String,
    pub attachment_key: String,
    pub media_type: String,
    pub canonical_path: Option<PathBuf>,
    pub revision_key: Option<String>,
}

pub struct SyncRunCompletion<'a> {
    pub status: &'a str,
    pub libraries_scanned: u32,
    pub objects_changed: u64,
    pub objects_deleted: u64,
    pub normalization_warnings: u64,
    pub error: Option<&'a str>,
}

struct CompatibilityDelta<'a> {
    target_version: i64,
    objects: &'a [RawObject],
    deleted_items: &'a [String],
    deleted_collections: &'a [String],
    neutral_delta: Option<&'a SourceDelta>,
}

impl ResearchDatabase {
    pub fn initialize(path: PathBuf) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| AppError::ApplicationData(error.to_string()))?;
        }
        let database = Self { path };
        let mut connection = database.connect()?;
        migrate(&mut connection)?;
        Ok(database)
    }

    pub fn register_source(&self, probe: &SourceProbe) -> AppResult<StoredSource> {
        let connection = self.connect()?;
        let source_id = stable_id("source", &[&probe.server_id]);
        let previous_active = meta_value(&connection, "active_source_id")?;
        let changed = previous_active
            .as_deref()
            .is_some_and(|value| value != source_id);
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "INSERT INTO sources (
                id, server_id, api_version, schema_version,
                compatibility, first_seen_at, last_seen_at,
                connection_status, last_error
             ) VALUES (?1, ?2, ?3, ?4, 'modern', ?5, ?5, 'online', NULL)
             ON CONFLICT(id) DO UPDATE SET
                api_version = excluded.api_version,
                schema_version = excluded.schema_version,
                last_seen_at = excluded.last_seen_at,
                connection_status = 'online',
                last_error = NULL",
            params![
                source_id,
                probe.server_id,
                probe.api_version,
                probe.schema_version,
                now,
            ],
        )?;
        set_meta(&connection, "active_source_id", &source_id)?;
        if changed {
            connection.execute("DELETE FROM app_meta WHERE key = 'active_library_id'", [])?;
        }
        Ok(StoredSource {
            id: source_id,
            changed,
        })
    }

    pub fn register_libraries(
        &self,
        source_id: &str,
        libraries: &[RemoteLibrary],
    ) -> AppResult<Vec<StoredLibrary>> {
        let connection = self.connect()?;
        let mut stored = Vec::with_capacity(libraries.len());
        connection.execute(
            "UPDATE libraries SET available = 0 WHERE source_id = ?1",
            [source_id],
        )?;
        for library in libraries {
            let id = stable_id("library", &[source_id, &library.kind, &library.zotero_id]);
            let source_instance_id = format!("source-instance:{id}");
            let now = Utc::now().to_rfc3339();
            connection.execute(
                "INSERT INTO libraries (
                    id, source_id, kind, zotero_library_id, name,
                    successful_version, available
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 0, 1)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    available = 1",
                params![id, source_id, library.kind, library.zotero_id, library.name,],
            )?;
            connection.execute(
                "INSERT INTO source_instances (
                    id, provider_kind, display_name, external_identity,
                    state, last_successful_cursor, legacy_source_id,
                    legacy_library_id, created_at, updated_at
                 )
                 SELECT
                    ?1, 'zotero', ?2,
                    s.server_id || ':' || ?3 || ':' || ?4,
                    'active', '0', ?5, ?6, ?7, ?7
                 FROM sources s WHERE s.id = ?5
                 ON CONFLICT(id) DO UPDATE SET
                    display_name = excluded.display_name,
                    external_identity = excluded.external_identity,
                    state = 'active',
                    legacy_source_id = excluded.legacy_source_id,
                    legacy_library_id = excluded.legacy_library_id,
                    updated_at = excluded.updated_at",
                params![
                    source_instance_id,
                    library.name,
                    library.kind,
                    library.zotero_id,
                    source_id,
                    id,
                    now,
                ],
            )?;
            let version = connection.query_row(
                "SELECT successful_version FROM libraries WHERE id = ?1",
                [&id],
                |row| row.get(0),
            )?;
            stored.push(StoredLibrary {
                id,
                remote: library.clone(),
                version,
            });
        }
        let active_library_id = meta_value(&connection, "active_library_id")?;
        let active_is_available = active_library_id
            .as_deref()
            .is_some_and(|active| stored.iter().any(|item| item.id == active));
        if !active_is_available {
            if let Some(personal) = stored.iter().find(|item| item.remote.kind == "user") {
                set_meta(&connection, "active_library_id", &personal.id)?;
            }
        }
        Ok(stored)
    }

    pub fn apply_delta(
        &self,
        source_id: &str,
        library_id: &str,
        delta: &LibraryDelta,
    ) -> AppResult<(u64, u64, u64)> {
        self.apply_compatibility_delta(
            source_id,
            library_id,
            CompatibilityDelta {
                target_version: delta.target_version,
                objects: &delta.objects,
                deleted_items: &delta.deleted_items,
                deleted_collections: &delta.deleted_collections,
                neutral_delta: None,
            },
        )
    }

    pub fn apply_source_delta(
        &self,
        source_id: &str,
        library_id: &str,
        delta: &SourceDelta,
    ) -> AppResult<(u64, u64, u64)> {
        if !delta.snapshot_complete
            && (!delta.item_tombstones.is_empty()
                || !delta.artifact_tombstones.is_empty()
                || !delta.group_tombstones.is_empty())
        {
            return Err(AppError::InvalidZoteroResponse(
                "a partial source snapshot cannot infer deletions".to_owned(),
            ));
        }
        let objects: Vec<RawObject> = delta
            .source_objects
            .iter()
            .map(|object| RawObject {
                key: object.external_id.clone(),
                version: object.source_revision,
                object_type: object.object_kind.clone(),
                json: object.payload.clone(),
            })
            .collect();
        let mut deleted_items = delta.item_tombstones.clone();
        deleted_items.extend(delta.artifact_tombstones.iter().cloned());
        deleted_items.sort();
        deleted_items.dedup();
        self.apply_compatibility_delta(
            source_id,
            library_id,
            CompatibilityDelta {
                target_version: delta.checkpoint_after,
                objects: &objects,
                deleted_items: &deleted_items,
                deleted_collections: &delta.group_tombstones,
                neutral_delta: Some(delta),
            },
        )
    }

    fn apply_compatibility_delta(
        &self,
        source_id: &str,
        library_id: &str,
        delta: CompatibilityDelta<'_>,
    ) -> AppResult<(u64, u64, u64)> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let fetched_at = Utc::now().to_rfc3339();
        let mut changed = 0_u64;
        let mut warnings = delta
            .neutral_delta
            .map(|source_delta| source_delta.warnings.len() as u64)
            .unwrap_or(0);
        let mut affected_item_ids = HashSet::new();

        if let Some(source_delta) = delta.neutral_delta {
            persist_neutral_source_records(
                &transaction,
                source_id,
                library_id,
                source_delta,
                &fetched_at,
                &mut affected_item_ids,
            )?;
        }

        for object in delta.objects {
            let raw_json = serde_json::to_string(&object.json)
                .map_err(|error| AppError::InvalidZoteroResponse(error.to_string()))?;
            let content_hash = content_hash(&raw_json);
            transaction.execute(
                "INSERT INTO source_objects (
                    source_id, library_id, object_type, object_key,
                    object_version, raw_json, content_hash, fetched_at,
                    deleted
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)
                 ON CONFLICT(library_id, object_type, object_key) DO UPDATE SET
                    object_version = excluded.object_version,
                    raw_json = excluded.raw_json,
                    content_hash = excluded.content_hash,
                    fetched_at = excluded.fetched_at,
                    deleted = 0",
                params![
                    source_id,
                    library_id,
                    object.object_type,
                    object.key,
                    object.version,
                    raw_json,
                    content_hash,
                    fetched_at,
                ],
            )?;
            transaction.execute(
                "DELETE FROM normalization_diagnostics
                 WHERE library_id = ?1 AND object_type = ?2 AND object_key = ?3",
                params![library_id, object.object_type, object.key],
            )?;
            let normalization_error = match object.object_type.as_str() {
                "collections" => match normalize_collection(object) {
                    Ok(collection) => {
                        upsert_collection(&transaction, source_id, library_id, &collection)?;
                        None
                    }
                    Err(error) => Some(error),
                },
                "items" => match normalize_item(object) {
                    Ok(item) => {
                        if let Some(item_id) =
                            upsert_item(&transaction, source_id, library_id, &item)?
                        {
                            affected_item_ids.insert(item_id);
                        }
                        None
                    }
                    Err(error) => Some(error),
                },
                _ => None,
            };
            if let Some(error) = normalization_error {
                transaction.execute(
                    "INSERT INTO normalization_diagnostics (
                        source_id, library_id, object_type, object_key,
                        object_version, severity, message, recorded_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, 'warning', ?6, ?7)",
                    params![
                        source_id,
                        library_id,
                        object.object_type,
                        object.key,
                        object.version,
                        error.to_string(),
                        fetched_at,
                    ],
                )?;
                warnings += 1;
            }
            changed += 1;
        }

        if let Some(source_delta) = delta.neutral_delta {
            changed = changed.max(
                (source_delta.knowledge_items.len()
                    + source_delta.artifacts.len()
                    + source_delta.source_groups.len()) as u64,
            );
        }

        transaction.execute(
            "UPDATE document_artifact_source_bindings
             SET knowledge_item_source_binding_id = (
                 SELECT ib.id
                 FROM documents d
                 JOIN knowledge_item_source_bindings ib
                   ON ib.knowledge_item_id = d.research_item_id
                  AND ib.source_instance_id =
                      document_artifact_source_bindings.source_instance_id
                 WHERE d.id =
                     document_artifact_source_bindings.artifact_id
                   AND ib.state = 'active'
                 ORDER BY ib.id LIMIT 1
             )
             WHERE source_instance_id = (
                 SELECT id FROM source_instances
                 WHERE legacy_library_id = ?1
             )",
            [library_id],
        )?;

        for key in delta.deleted_items {
            affected_item_ids.extend(affected_research_items(&transaction, library_id, key)?);
            tombstone_object(
                &transaction,
                source_id,
                library_id,
                "items",
                key,
                delta.target_version,
                &fetched_at,
            )?;
            match tombstone_entity_binding(&transaction, library_id, key, &fetched_at)? {
                BindingDeletion::Preserved => {}
                BindingDeletion::Artifact(artifact_id) => {
                    transaction.execute(
                        "DELETE FROM attachments
                         WHERE id = (
                             SELECT attachment_id FROM documents WHERE id = ?1
                         )",
                        [&artifact_id],
                    )?;
                }
                BindingDeletion::KnowledgeItem(parent_id) => {
                    transaction.execute(
                        "UPDATE document_artifact_source_bindings
                         SET state = 'deleted', deleted_at = ?2,
                             last_seen_at = ?2
                         WHERE artifact_id IN (
                             SELECT d.id
                             FROM documents d
                             WHERE d.research_item_id = ?1
                         )",
                        params![parent_id, fetched_at],
                    )?;
                    transaction
                        .execute("DELETE FROM research_items WHERE id = ?1", [&parent_id])?;
                    transaction.execute(
                        "DELETE FROM attachments WHERE parent_item_id = ?1",
                        [&parent_id],
                    )?;
                    transaction
                        .execute("DELETE FROM notes WHERE parent_item_id = ?1", [&parent_id])?;
                }
                BindingDeletion::LegacyOnly => {
                    delete_item(&transaction, library_id, key)?;
                }
            }
        }
        for key in delta.deleted_collections {
            tombstone_object(
                &transaction,
                source_id,
                library_id,
                "collections",
                key,
                delta.target_version,
                &fetched_at,
            )?;
            let collection_id = stable_id("collection", &[source_id, library_id, key]);
            transaction.execute(
                "DELETE FROM item_collections WHERE collection_id = ?1",
                [&collection_id],
            )?;
            transaction.execute(
                "DELETE FROM collections WHERE library_id = ?1 AND zotero_key = ?2",
                params![library_id, key],
            )?;
        }
        for item_id in affected_item_ids {
            refresh_search_content(&transaction, &item_id)?;
        }
        let deleted = (delta.deleted_items.len() + delta.deleted_collections.len()) as u64;
        transaction.execute(
            "UPDATE libraries SET
                successful_version = ?2,
                last_synced_at = ?3,
                last_error = NULL
             WHERE id = ?1",
            params![library_id, delta.target_version, fetched_at],
        )?;
        transaction.execute(
            "UPDATE source_instances SET
                last_successful_cursor = ?2,
                last_synced_at = ?3,
                last_error = NULL,
                state = 'active',
                updated_at = ?3
             WHERE legacy_library_id = ?1",
            params![library_id, delta.target_version.to_string(), fetched_at],
        )?;
        transaction.commit()?;
        Ok((changed, deleted, warnings))
    }

    pub fn object_keys(&self, library_id: &str, object_type: &str) -> AppResult<HashSet<String>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT object_key
             FROM source_objects
             WHERE library_id = ?1 AND object_type = ?2 AND deleted = 0",
        )?;
        let rows = statement.query_map(params![library_id, object_type], |row| row.get(0))?;
        rows.collect::<Result<HashSet<String>, _>>()
            .map_err(AppError::from)
    }

    pub fn begin_sync(&self, source_id: &str) -> AppResult<i64> {
        let connection = self.connect()?;
        let started_at = Utc::now().to_rfc3339();
        connection.execute(
            "INSERT INTO sync_runs (
                source_id, status, started_at,
                libraries_scanned, objects_changed, objects_deleted
             ) VALUES (?1, 'running', ?2, 0, 0, 0)",
            params![source_id, started_at],
        )?;
        Ok(connection.last_insert_rowid())
    }

    pub fn finish_sync(&self, run_id: i64, completion: SyncRunCompletion<'_>) -> AppResult<()> {
        let connection = self.connect()?;
        connection.execute(
            "UPDATE sync_runs SET
                status = ?2,
                finished_at = ?3,
                libraries_scanned = ?4,
                objects_changed = ?5,
                objects_deleted = ?6,
                normalization_warnings = ?7,
                error = ?8
             WHERE id = ?1",
            params![
                run_id,
                completion.status,
                Utc::now().to_rfc3339(),
                completion.libraries_scanned,
                completion.objects_changed as i64,
                completion.objects_deleted as i64,
                completion.normalization_warnings as i64,
                completion.error,
            ],
        )?;
        Ok(())
    }

    pub fn record_connection_error(&self, error: &str) -> AppResult<()> {
        let connection = self.connect()?;
        if let Some(source_id) = meta_value(&connection, "active_source_id")? {
            connection.execute(
                "UPDATE sources SET connection_status = 'offline', last_error = ?2
                 WHERE id = ?1",
                params![source_id, error],
            )?;
        }
        Ok(())
    }

    pub fn set_active_library(&self, library_id: &str) -> AppResult<()> {
        let connection = self.connect()?;
        let exists = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM libraries
                WHERE id = ?1 AND source_id = (
                    SELECT value FROM app_meta WHERE key = 'active_source_id'
                )
             )",
            [library_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(AppError::Database(
                "The selected library does not belong to the active Zotero source".to_owned(),
            ));
        }
        set_meta(&connection, "active_library_id", library_id)
    }

    pub fn clear_active_partition(&self) -> AppResult<()> {
        let connection = self.connect()?;
        let Some(source_id) = meta_value(&connection, "active_source_id")? else {
            return Ok(());
        };
        connection.execute("DELETE FROM sources WHERE id = ?1", [&source_id])?;
        connection.execute(
            "DELETE FROM app_meta WHERE key IN ('active_source_id', 'active_library_id')",
            [],
        )?;
        Ok(())
    }

    pub fn search(&self, request: &SearchRequest) -> AppResult<SearchPage> {
        let connection = self.connect()?;
        let Some(library_id) = meta_value(&connection, "active_library_id")? else {
            return Ok(SearchPage {
                results: Vec::new(),
                total: 0,
                offset: 0,
                limit: request.limit.unwrap_or(20).min(50),
                corpus_status: "not-indexed".to_owned(),
                retrieval_mode: "keyword".to_owned(),
                semantic_indexed_papers: 0,
                semantic_eligible_papers: 0,
            });
        };
        let limit = request.limit.unwrap_or(20).clamp(1, 50);
        let offset = request.offset.unwrap_or(0);
        let item_scope_requested =
            prepare_search_item_scope(&connection, &library_id, &request.item_ids)?;
        let item_scope_condition = if item_scope_requested {
            " AND EXISTS (
                SELECT 1 FROM search_scope_item_ids scope
                WHERE scope.id = i.id
              )"
        } else {
            ""
        };
        let citation_filter = citation_search_filter(request.citation_filter.as_deref());
        let Some(fts_query) = lexical_query(&request.query) else {
            if citation_filter != "all" {
                let condition = citation_search_condition(citation_filter, "i.id");
                let total = connection.query_row(
                    &format!(
                        "SELECT COUNT(*) FROM research_items i
                         WHERE i.library_id = ?1 AND i.is_trashed = 0
                           AND (?3 IS NULL OR i.publication_year >= ?3)
                           AND (?4 IS NULL OR i.publication_year <= ?4)
                           AND (?5 IS NULL OR i.item_type = ?5)
                           AND (?2 IS NULL OR EXISTS (
                                SELECT 1 FROM item_collections ic
                                WHERE ic.item_id = i.id
                                  AND ic.collection_id = ?2
                           )){condition}{item_scope_condition}"
                    ),
                    params![
                        library_id,
                        request.collection_id,
                        request.year_from,
                        request.year_to,
                        request.item_type,
                    ],
                    |row| row.get::<_, i64>(0),
                )? as u64;
                let mut statement = connection.prepare(&format!(
                    "SELECT i.id FROM research_items i
                     WHERE i.library_id = ?1 AND i.is_trashed = 0
                       AND (?3 IS NULL OR i.publication_year >= ?3)
                       AND (?4 IS NULL OR i.publication_year <= ?4)
                       AND (?5 IS NULL OR i.item_type = ?5)
                       AND (?2 IS NULL OR EXISTS (
                            SELECT 1 FROM item_collections ic
                            WHERE ic.item_id = i.id AND ic.collection_id = ?2
                       )){condition}{item_scope_condition}
                     ORDER BY i.title COLLATE NOCASE, i.id
                     LIMIT ?6 OFFSET ?7"
                ))?;
                let ids = statement
                    .query_map(
                        params![
                            library_id,
                            request.collection_id,
                            request.year_from,
                            request.year_to,
                            request.item_type,
                            limit,
                            offset,
                        ],
                        |row| row.get::<_, String>(0),
                    )?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(statement);
                connection.execute_batch(
                    "CREATE TEMP TABLE IF NOT EXISTS selected_item_ids (
                        id TEXT PRIMARY KEY
                     ) WITHOUT ROWID;
                     DELETE FROM selected_item_ids;",
                )?;
                for item_id in &ids {
                    connection.execute(
                        "INSERT OR IGNORE INTO selected_item_ids(id) VALUES (?1)",
                        [item_id],
                    )?;
                }
                let items_by_id: HashMap<_, _> = load_selected_items(&connection)?
                    .into_iter()
                    .map(|item| (item.id.clone(), item))
                    .collect();
                let reason = citation_search_label(citation_filter).to_owned();
                let results = ids
                    .into_iter()
                    .filter_map(|item_id| {
                        items_by_id.get(&item_id).cloned().map(|item| SearchResult {
                            item,
                            score: 1.0,
                            reasons: vec![reason.clone()],
                            snippet: None,
                            match_source: "metadata".to_owned(),
                            retrieval_modes: vec!["keyword".to_owned()],
                            ranking: crate::model::RetrievalScoreBreakdown::default(),
                            passages: Vec::new(),
                        })
                    })
                    .collect();
                return Ok(SearchPage {
                    results,
                    total,
                    offset,
                    limit,
                    corpus_status: load_corpus_diagnostics(&connection, Some(&library_id))?.status,
                    retrieval_mode: "keyword".to_owned(),
                    semantic_indexed_papers: 0,
                    semantic_eligible_papers: 0,
                });
            }
            return Ok(SearchPage {
                results: Vec::new(),
                total: 0,
                offset,
                limit,
                corpus_status: load_corpus_diagnostics(&connection, Some(&library_id))?.status,
                retrieval_mode: "keyword".to_owned(),
                semantic_indexed_papers: 0,
                semantic_eligible_papers: 0,
            });
        };
        let citation_condition = citation_search_condition(citation_filter, "sc.item_id");
        let total = connection.query_row(
            &format!(
                "SELECT COUNT(*)
             FROM research_fts f
             JOIN search_content sc ON sc.rowid = f.rowid
             JOIN research_items i ON i.id = sc.item_id
             WHERE research_fts MATCH ?1 AND sc.library_id = ?2
               AND (?4 IS NULL OR i.publication_year >= ?4)
               AND (?5 IS NULL OR i.publication_year <= ?5)
               AND (?6 IS NULL OR i.item_type = ?6)
               AND (?3 IS NULL OR EXISTS (
                    SELECT 1 FROM item_collections ic
                    WHERE ic.item_id = sc.item_id AND ic.collection_id = ?3
               )){citation_condition}{item_scope_condition}"
            ),
            params![
                fts_query,
                library_id,
                request.collection_id,
                request.year_from,
                request.year_to,
                request.item_type,
            ],
            |row| row.get::<_, i64>(0),
        )? as u64;
        let mut statement = connection.prepare(&format!(
            "SELECT sc.item_id,
                    bm25(research_fts, 8.0, 6.0, 4.0, 3.0, 2.0, 1.0),
                    snippet(research_fts, 5, '[', ']', ' ... ', 24),
                    CASE
                        WHEN instr(
                            highlight(research_fts, 5, char(1), char(2)),
                            char(1)
                        ) > 0 THEN 'metadata-and-full-text'
                        ELSE 'metadata'
                    END
             FROM research_fts f
             JOIN search_content sc ON sc.rowid = f.rowid
             JOIN research_items i ON i.id = sc.item_id
             WHERE research_fts MATCH ?1 AND sc.library_id = ?2
               AND (?4 IS NULL OR i.publication_year >= ?4)
               AND (?5 IS NULL OR i.publication_year <= ?5)
               AND (?6 IS NULL OR i.item_type = ?6)
               AND (?3 IS NULL OR EXISTS (
                    SELECT 1 FROM item_collections ic
                    WHERE ic.item_id = sc.item_id AND ic.collection_id = ?3
               )){citation_condition}{item_scope_condition}
             ORDER BY bm25(research_fts, 8.0, 6.0, 4.0, 3.0, 2.0, 1.0),
                      sc.title COLLATE NOCASE
             LIMIT ?7 OFFSET ?8"
        ))?;
        let hits = statement
            .query_map(
                params![
                    fts_query,
                    library_id,
                    request.collection_id,
                    request.year_from,
                    request.year_to,
                    request.item_type,
                    limit,
                    offset,
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        connection.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS selected_item_ids (
                id TEXT PRIMARY KEY
             ) WITHOUT ROWID;
             DELETE FROM selected_item_ids;",
        )?;
        for (item_id, _, _, _) in &hits {
            connection.execute(
                "INSERT OR IGNORE INTO selected_item_ids(id) VALUES (?1)",
                [item_id],
            )?;
        }
        let items_by_id: HashMap<_, _> = load_selected_items(&connection)?
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect();
        let results = hits
            .into_iter()
            .filter_map(|(item_id, rank, snippet, match_source)| {
                items_by_id.get(&item_id).cloned().map(|item| {
                    let mut reasons = if match_source == "metadata-and-full-text" {
                        vec!["lexical metadata or indexed text".to_owned()]
                    } else {
                        vec!["lexical metadata".to_owned()]
                    };
                    if citation_filter != "all" {
                        reasons.push(citation_search_label(citation_filter).to_owned());
                    }
                    SearchResult {
                        item,
                        score: -rank,
                        reasons,
                        snippet: snippet.filter(|value| !value.is_empty()),
                        match_source,
                        retrieval_modes: vec!["keyword".to_owned()],
                        ranking: crate::model::RetrievalScoreBreakdown {
                            lexical_score: Some(-rank),
                            ..Default::default()
                        },
                        passages: Vec::new(),
                    }
                })
            })
            .collect();
        Ok(SearchPage {
            results,
            total,
            offset,
            limit,
            corpus_status: load_corpus_diagnostics(&connection, Some(&library_id))?.status,
            retrieval_mode: "keyword".to_owned(),
            semantic_indexed_papers: 0,
            semantic_eligible_papers: 0,
        })
    }

    pub fn query_items(&self, request: &ItemQueryRequest) -> AppResult<ItemPage> {
        let connection = self.connect()?;
        let Some(library_id) = meta_value(&connection, "active_library_id")? else {
            return Ok(ItemPage {
                items: Vec::new(),
                total: 0,
                offset: 0,
                limit: request.limit.unwrap_or(50).min(MAX_PAGE_LIMIT),
            });
        };
        let limit = request.limit.unwrap_or(50).clamp(1, MAX_PAGE_LIMIT);
        let offset = request.offset.unwrap_or(0);
        connection.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS selected_item_ids (
                id TEXT PRIMARY KEY
             ) WITHOUT ROWID;
             DELETE FROM selected_item_ids;",
        )?;
        let normalized_tag = request.tag.as_deref().map(normalize_facet);
        let total = connection.query_row(
            "SELECT COUNT(*) FROM research_items i
             WHERE i.library_id = ?1 AND i.is_trashed = 0
               AND (?2 IS NULL OR EXISTS (
                    SELECT 1 FROM item_collections ic
                    WHERE ic.item_id = i.id AND ic.collection_id = ?2
               ))
               AND (?3 IS NULL OR i.item_type = ?3)
               AND (?4 IS NULL OR i.publication_year >= ?4)
               AND (?5 IS NULL OR i.publication_year <= ?5)
               AND (?6 IS NULL OR EXISTS (
                    SELECT 1 FROM item_tags it
                    WHERE it.item_id = i.id AND it.normalized_tag = ?6
               ))",
            params![
                library_id,
                request.collection_id,
                request.item_type,
                request.year_from,
                request.year_to,
                normalized_tag,
            ],
            |row| row.get::<_, i64>(0),
        )? as u64;
        connection.execute(
            "INSERT INTO selected_item_ids(id)
             SELECT i.id FROM research_items i
             WHERE i.library_id = ?1 AND i.is_trashed = 0
               AND (?2 IS NULL OR EXISTS (
                    SELECT 1 FROM item_collections ic
                    WHERE ic.item_id = i.id AND ic.collection_id = ?2
               ))
               AND (?3 IS NULL OR i.item_type = ?3)
               AND (?4 IS NULL OR i.publication_year >= ?4)
               AND (?5 IS NULL OR i.publication_year <= ?5)
               AND (?6 IS NULL OR EXISTS (
                    SELECT 1 FROM item_tags it
                    WHERE it.item_id = i.id AND it.normalized_tag = ?6
               ))
             ORDER BY
                CASE WHEN ?7 = 'year-desc' THEN i.publication_year END DESC,
                i.title COLLATE NOCASE,
                i.id
             LIMIT ?8 OFFSET ?9",
            params![
                library_id,
                request.collection_id,
                request.item_type,
                request.year_from,
                request.year_to,
                normalized_tag,
                request.sort,
                limit,
                offset,
            ],
        )?;
        let items = load_selected_items(&connection)?;
        Ok(ItemPage {
            items,
            total,
            offset,
            limit,
        })
    }

    pub fn active_library_context(&self) -> AppResult<Option<(String, String, StoredLibrary)>> {
        let connection = self.connect()?;
        let Some(library_id) = meta_value(&connection, "active_library_id")? else {
            return Ok(None);
        };
        connection
            .query_row(
                "SELECT l.source_id, s.server_id, l.kind,
                        l.zotero_library_id, l.name, l.successful_version
                 FROM libraries l
                 JOIN sources s ON s.id = l.source_id
                 WHERE l.id = ?1",
                [&library_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        StoredLibrary {
                            id: library_id.clone(),
                            remote: RemoteLibrary {
                                kind: row.get(2)?,
                                zotero_id: row.get(3)?,
                                name: row.get(4)?,
                            },
                            version: row.get(5)?,
                        },
                    ))
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn fulltext_cursor(&self, library_id: &str) -> AppResult<i64> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT successful_version FROM fulltext_sync_state
                 WHERE library_id = ?1",
                [library_id],
                |row| row.get(0),
            )
            .optional()
            .map(|value| value.unwrap_or(0))
            .map_err(Into::into)
    }

    pub fn fulltext_keys(&self, library_id: &str) -> AppResult<HashSet<String>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "SELECT binding.external_artifact_id
             FROM document_texts text
             JOIN document_artifact_source_bindings binding
               ON binding.artifact_id = text.document_id
              AND binding.state = 'active'
             JOIN source_instances instance
               ON instance.id = binding.source_instance_id
             WHERE instance.legacy_library_id = ?1
               AND text.provider = 'zotero-indexed'",
        )?;
        let rows = statement.query_map([library_id], |row| row.get(0))?;
        rows.collect::<Result<HashSet<_>, _>>().map_err(Into::into)
    }

    pub fn apply_fulltext_delta(
        &self,
        source_id: &str,
        library_id: &str,
        delta: &FullTextSyncDelta,
    ) -> AppResult<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        let mut affected_item_ids = HashSet::new();
        for attachment_key in &delta.removed_attachment_keys {
            if let Some((document_id, item_id)) = transaction
                .query_row(
                    "SELECT d.id, d.research_item_id
                     FROM document_artifact_source_bindings binding
                     JOIN source_instances instance
                       ON instance.id = binding.source_instance_id
                     JOIN documents d ON d.id = binding.artifact_id
                     WHERE instance.legacy_library_id = ?1
                       AND binding.external_artifact_id = ?2
                       AND binding.state = 'active'",
                    params![library_id, attachment_key],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()?
            {
                if !retains_local_extraction(&transaction, &document_id)? {
                    invalidate_document_citations(&transaction, &document_id, None)?;
                }
                if let Some(item_id) = item_id {
                    affected_item_ids.insert(item_id);
                }
            }
            transaction.execute(
                "DELETE FROM document_texts
                 WHERE document_id IN (
                    SELECT binding.artifact_id
                    FROM document_artifact_source_bindings binding
                    JOIN source_instances instance
                      ON instance.id = binding.source_instance_id
                    WHERE instance.legacy_library_id = ?1
                      AND binding.external_artifact_id = ?2
                 ) AND provider = 'zotero-indexed'",
                params![library_id, attachment_key],
            )?;
        }
        for record in &delta.records {
            let document = transaction
                .query_row(
                    "SELECT id, research_item_id FROM documents
                     WHERE id = (
                        SELECT binding.artifact_id
                        FROM document_artifact_source_bindings binding
                        JOIN source_instances instance
                          ON instance.id = binding.source_instance_id
                        WHERE instance.legacy_library_id = ?1
                          AND binding.external_artifact_id = ?2
                          AND binding.state = 'active'
                        ORDER BY binding.id LIMIT 1
                     )",
                    params![library_id, record.attachment_key],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                )
                .optional()?;
            let Some((document_id, item_id)) = document else {
                continue;
            };
            let completeness = text_completeness(
                record.indexed_pages,
                record.total_pages,
                record.indexed_chars,
                record.total_chars,
            );
            let text_hash = content_hash(&record.content);
            let retain_local = retains_local_extraction(&transaction, &document_id)?;
            if !retain_local {
                invalidate_document_citations(&transaction, &document_id, Some(&text_hash))?;
            }
            let revision_key = format!(
                "text:zotero-indexed:{}:{}",
                record.version,
                &text_hash[..16]
            );
            transaction.execute(
                "INSERT OR IGNORE INTO document_revisions (
                    document_id, source_version, revision_key,
                    file_size, modified_ms, discovered_at, revision_kind,
                    content_hash, artifact_source_binding_id
                 ) VALUES (
                    ?1, ?2, ?3, NULL, NULL, ?4,
                    'source-provided-text', ?5,
                    (SELECT b.id
                     FROM document_artifact_source_bindings b
                     WHERE b.artifact_id = ?1 AND b.state = 'active'
                     ORDER BY b.id LIMIT 1)
                 )",
                params![document_id, record.version, revision_key, now, text_hash,],
            )?;
            let text_id = stable_id("text", &[&document_id, "zotero-indexed"]);
            transaction.execute(
                "UPDATE document_texts
                 SET is_active = 0
                 WHERE document_id = ?1 AND provider <> 'zotero-indexed' AND ?2=0",
                params![document_id, retain_local],
            )?;
            transaction.execute(
                "INSERT INTO document_texts (
                    id, document_id, source_id, library_id,
                    zotero_attachment_key, provider, source_version,
                    content, indexed_pages, total_pages, indexed_chars,
                    total_chars, completeness, fetched_at,
                    projection_profile_id, content_hash, is_active
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, 'zotero-indexed', ?6,
                    ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                    'plain-text-v1', ?14, ?15
                 )
                 ON CONFLICT(document_id, provider) DO UPDATE SET
                    source_version = excluded.source_version,
                    content = excluded.content,
                    indexed_pages = excluded.indexed_pages,
                    total_pages = excluded.total_pages,
                    indexed_chars = excluded.indexed_chars,
                    total_chars = excluded.total_chars,
                    completeness = excluded.completeness,
                    fetched_at = excluded.fetched_at,
                    projection_profile_id = excluded.projection_profile_id,
                    content_hash = excluded.content_hash,
                    is_active = excluded.is_active",
                params![
                    text_id,
                    document_id,
                    source_id,
                    library_id,
                    record.attachment_key,
                    record.version,
                    record.content,
                    record.indexed_pages,
                    record.total_pages,
                    record.indexed_chars.map(|value| value as i64),
                    record.total_chars.map(|value| value as i64),
                    completeness,
                    now,
                    text_hash,
                    !retain_local,
                ],
            )?;
            if let Some(item_id) = item_id {
                affected_item_ids.insert(item_id);
            }
        }
        for item_id in affected_item_ids {
            refresh_search_content(&transaction, &item_id)?;
        }
        transaction.execute(
            "INSERT INTO fulltext_sync_state (
                library_id, successful_version, status, last_synced_at,
                changed_documents, warning_count, last_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(library_id) DO UPDATE SET
                successful_version = excluded.successful_version,
                status = excluded.status,
                last_synced_at = excluded.last_synced_at,
                changed_documents = excluded.changed_documents,
                warning_count = excluded.warning_count,
                last_error = excluded.last_error",
            params![
                library_id,
                delta.successful_version,
                delta.status,
                now,
                delta.records.len() as i64,
                delta.warning_count as i64,
                delta.last_error,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn rebuild_document_index(&self) -> AppResult<CorpusDiagnostics> {
        let mut connection = self.connect()?;
        let Some(library_id) = meta_value(&connection, "active_library_id")? else {
            return load_corpus_diagnostics(&connection, None);
        };
        let transaction = connection.transaction()?;
        let mut statement = transaction.prepare(
            "SELECT DISTINCT research_item_id FROM documents
             WHERE library_id = ?1 AND research_item_id IS NOT NULL",
        )?;
        let item_ids = statement
            .query_map([&library_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut document_statement =
            transaction.prepare("SELECT id FROM documents WHERE library_id = ?1")?;
        let document_ids = document_statement
            .query_map([&library_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(document_statement);
        for document_id in document_ids {
            invalidate_document_citations(&transaction, &document_id, None)?;
        }
        transaction.execute(
            "DELETE FROM artifact_extraction_state
             WHERE artifact_id IN (
                 SELECT id FROM documents WHERE library_id = ?1
             )",
            [&library_id],
        )?;
        transaction.execute(
            "DELETE FROM extraction_runs
             WHERE artifact_id IN (
                 SELECT id FROM documents WHERE library_id = ?1
             )",
            [&library_id],
        )?;
        transaction.execute(
            "DELETE FROM document_texts WHERE library_id = ?1",
            [&library_id],
        )?;
        for item_id in item_ids {
            refresh_search_content(&transaction, &item_id)?;
        }
        transaction.execute(
            "INSERT INTO fulltext_sync_state (
                library_id, successful_version, status,
                changed_documents, warning_count, last_error
             ) VALUES (?1, 0, 'not-indexed', 0, 0, NULL)
             ON CONFLICT(library_id) DO UPDATE SET
                successful_version = 0,
                status = 'not-indexed',
                last_synced_at = NULL,
                changed_documents = 0,
                warning_count = 0,
                last_error = NULL",
            [&library_id],
        )?;
        transaction.commit()?;
        load_corpus_diagnostics(&connection, Some(&library_id))
    }

    pub fn document_text(&self, document_id: &str) -> AppResult<Option<DocumentTextPayload>> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT document_id, provider, content, indexed_pages,
                        total_pages, indexed_chars, total_chars, completeness
                 FROM document_texts
                 WHERE document_id = ?1 AND is_active = 1",
                [document_id],
                |row| {
                    Ok(DocumentTextPayload {
                        document_id: row.get(0)?,
                        provider: row.get(1)?,
                        content: row.get(2)?,
                        indexed_pages: row.get(3)?,
                        total_pages: row.get(4)?,
                        indexed_chars: row.get::<_, Option<i64>>(5)?.map(|value| value as u64),
                        total_chars: row.get::<_, Option<i64>>(6)?.map(|value| value as u64),
                        completeness: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn document_resolution_context(
        &self,
        document_id: &str,
    ) -> AppResult<DocumentResolutionContext> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT d.id, l.kind, l.zotero_library_id, l.name,
                        s.server_id, binding.external_artifact_id, d.media_type,
                        dl.canonical_path, dl.revision_key, instance.provider_kind
                 FROM documents d
                 JOIN document_artifact_source_bindings binding
                   ON binding.artifact_id = d.id AND binding.state = 'active'
                 JOIN source_instances instance
                   ON instance.id = binding.source_instance_id
                  AND instance.provider_kind IN ('zotero', 'real-demo')
                 JOIN libraries l ON l.id = instance.legacy_library_id
                 JOIN sources s ON s.id = instance.legacy_source_id
                 LEFT JOIN document_locators dl ON dl.document_id = d.id
                 WHERE d.id = ?1
                 ORDER BY binding.id LIMIT 1",
                [document_id],
                |row| {
                    Ok(DocumentResolutionContext {
                        document_id: row.get(0)?,
                        remote: RemoteLibrary {
                            kind: row.get(1)?,
                            zotero_id: row.get(2)?,
                            name: row.get(3)?,
                        },
                        server_id: row.get(4)?,
                        attachment_key: row.get(5)?,
                        media_type: row.get(6)?,
                        canonical_path: row.get::<_, Option<String>>(7)?.map(PathBuf::from),
                        revision_key: row.get(8)?,
                        provider_kind: row.get(9)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| {
                AppError::DocumentUnavailable(format!("unknown document identifier {document_id}"))
            })
    }

    pub fn store_document_locator(
        &self,
        document_id: &str,
        path: &std::path::Path,
        revision_key: &str,
        content_hash: &str,
        file_size: u64,
        modified_ms: Option<i64>,
    ) -> AppResult<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        let path_text = path.to_string_lossy();
        transaction.execute(
            "INSERT INTO document_locators (
                document_id, canonical_path, revision_key, file_size,
                modified_ms, resolved_at, last_checked_at,
                artifact_source_binding_id, locator_kind
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?6,
                (SELECT b.id
                 FROM document_artifact_source_bindings b
                 WHERE b.artifact_id = ?1 AND b.state = 'active'
                 ORDER BY b.id LIMIT 1),
                'validated-local-file'
             )
             ON CONFLICT(document_id) DO UPDATE SET
                canonical_path = excluded.canonical_path,
                revision_key = excluded.revision_key,
                file_size = excluded.file_size,
                modified_ms = excluded.modified_ms,
                artifact_source_binding_id =
                    excluded.artifact_source_binding_id,
                locator_kind = excluded.locator_kind,
                resolved_at = excluded.resolved_at,
                last_checked_at = excluded.last_checked_at",
            params![
                document_id,
                path_text,
                revision_key,
                file_size as i64,
                modified_ms,
                now,
            ],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO document_revisions (
                document_id, source_version, revision_key, file_size,
                modified_ms, discovered_at, revision_kind, content_hash,
                artifact_source_binding_id
             ) SELECT id, source_version, ?2, ?4, ?5, ?6,
                      'artifact-content', ?3,
                      (SELECT b.id
                       FROM document_artifact_source_bindings b
                       WHERE b.artifact_id = documents.id
                         AND b.state = 'active'
                       ORDER BY b.id LIMIT 1)
               FROM documents WHERE id = ?1",
            params![
                document_id,
                revision_key,
                content_hash,
                file_size as i64,
                modified_ms,
                now
            ],
        )?;
        transaction.execute(
            "UPDATE documents SET availability = 'ready',
                    last_error = NULL, updated_at = ?2
             WHERE id = ?1",
            params![document_id, now],
        )?;
        let invalidated = transaction.execute(
            "UPDATE document_texts SET is_active = 0
             WHERE document_id = ?1 AND is_active = 1
               AND extraction_id IN (
                 SELECT e.id FROM document_extractions e
                 JOIN document_revisions r ON r.id = e.artifact_revision_id
                 WHERE e.artifact_id = ?1
                   AND r.revision_kind = 'artifact-content'
                   AND r.content_hash <> ?2
               )",
            params![document_id, content_hash],
        )?;
        if invalidated > 0 {
            transaction.execute(
                "UPDATE artifact_extraction_state
                 SET execution_state = 'stale', updated_at = ?2,
                     last_error = 'The original PDF changed. Prepare its text again.'
                 WHERE artifact_id = ?1",
                params![document_id, now],
            )?;
            invalidate_document_citations(&transaction, document_id, None)?;
            let item_id: String = transaction.query_row(
                "SELECT research_item_id FROM documents WHERE id = ?1",
                [document_id],
                |row| row.get(0),
            )?;
            refresh_search_content(&transaction, &item_id)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn source_document_paths(&self) -> AppResult<Vec<PathBuf>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare("SELECT canonical_path FROM document_locators")?;
        let paths = statement.query_map([], |row| row.get::<_, String>(0).map(PathBuf::from))?;
        Ok(paths.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn document_path(&self, document_id: &str) -> AppResult<PathBuf> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT canonical_path FROM document_locators
                 WHERE document_id = ?1",
                [document_id],
                |row| row.get::<_, String>(0).map(PathBuf::from),
            )
            .optional()?
            .ok_or_else(|| {
                AppError::DocumentUnavailable(
                    "resolve the document before requesting its bytes".to_owned(),
                )
            })
    }

    pub fn reader_state(&self, document_id: &str) -> AppResult<Option<ReaderState>> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT document_id, page_number, zoom, fit_mode,
                        outline_open, context_open
                 FROM reader_states WHERE document_id = ?1",
                [document_id],
                |row| {
                    Ok(ReaderState {
                        document_id: row.get(0)?,
                        page_number: row.get(1)?,
                        zoom: row.get(2)?,
                        fit_mode: row.get(3)?,
                        outline_open: row.get(4)?,
                        context_open: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn save_reader_state(&self, state: &ReaderState) -> AppResult<()> {
        if state.page_number == 0 || !(0.25..=5.0).contains(&state.zoom) {
            return Err(AppError::DocumentUnavailable(
                "reader page or zoom is outside the supported range".to_owned(),
            ));
        }
        let connection = self.connect()?;
        connection.execute(
            "INSERT INTO reader_states (
                document_id, page_number, zoom, fit_mode,
                outline_open, context_open, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(document_id) DO UPDATE SET
                page_number = excluded.page_number,
                zoom = excluded.zoom,
                fit_mode = excluded.fit_mode,
                outline_open = excluded.outline_open,
                context_open = excluded.context_open,
                updated_at = excluded.updated_at",
            params![
                state.document_id,
                state.page_number,
                state.zoom,
                state.fit_mode,
                state.outline_open,
                state.context_open,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub(crate) fn items_by_ids(&self, item_ids: &[String]) -> AppResult<Vec<ResearchItem>> {
        if item_ids.is_empty() {
            return Ok(Vec::new());
        }
        let connection = self.connect()?;
        connection.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS selected_item_ids (
                id TEXT PRIMARY KEY
             ) WITHOUT ROWID;
             DELETE FROM selected_item_ids;",
        )?;
        for item_id in item_ids.iter().take(MAX_PAGE_LIMIT as usize) {
            connection.execute(
                "INSERT OR IGNORE INTO selected_item_ids(id)
                 SELECT id FROM research_items
                 WHERE id = ?1 AND is_trashed = 0",
                [item_id],
            )?;
        }
        load_selected_items(&connection)
    }

    pub(crate) fn connect(&self) -> AppResult<Connection> {
        let connection = Connection::open(&self.path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(connection)
    }
}

fn migrate(connection: &mut Connection) -> AppResult<()> {
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY,
             name TEXT NOT NULL,
             applied_at TEXT NOT NULL
         );",
    )?;
    let migrations = [
        (1_i64, "initial source and normalized schema", MIGRATION_1),
        (2_i64, "lookup and synchronization indexes", MIGRATION_2),
        (3_i64, "track currently discoverable libraries", MIGRATION_3),
        (4_i64, "record normalization diagnostics", MIGRATION_4),
        (5_i64, "add document corpus and lexical search", MIGRATION_5),
        (
            6_i64,
            "add evidence-first citation intelligence",
            MIGRATION_6,
        ),
        (
            7_i64,
            "add local semantic retrieval and vector provenance",
            MIGRATION_7,
        ),
        (
            8_i64,
            "add grounded local research agent and conversations",
            MIGRATION_8,
        ),
        (9_i64, "adopt Graphite application identity", MIGRATION_9),
        (
            10_i64,
            "add neutral knowledge identity and source bindings",
            MIGRATION_10,
        ),
        (
            11_i64,
            "add canonical document extraction persistence",
            MIGRATION_11,
        ),
        (
            12_i64,
            "separate immutable extraction attempts from active selection",
            MIGRATION_12,
        ),
        (
            13_i64,
            "activate parser-neutral extraction quality hardening",
            MIGRATION_13,
        ),
        (
            14_i64,
            "add isolated PDF shadow extraction pilot state",
            MIGRATION_14,
        ),
        (
            15_i64,
            "index scientific passages for lexical retrieval",
            MIGRATION_15,
        ),
        (16_i64, "index only the active extraction", MIGRATION_16),
        (
            17_i64,
            "save inference backend and context preferences",
            MIGRATION_17,
        ),
        (
            18_i64,
            "persist library-scoped research lenses",
            MIGRATION_18,
        ),
    ];
    for (version, name, sql) in migrations {
        let applied = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
            [version],
            |row| row.get::<_, bool>(0),
        )?;
        if applied {
            continue;
        }
        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        match version {
            10 => validate_knowledge_identity_migration(&transaction)?,
            11 => backfill_extraction_projection_hashes(&transaction)?,
            12 => validate_extraction_selection_migration(&transaction)?,
            _ => {}
        }
        transaction.execute(
            "INSERT INTO schema_migrations (version, name, applied_at)
             VALUES (?1, ?2, ?3)",
            params![version, name, Utc::now().to_rfc3339()],
        )?;
        transaction.commit()?;
    }
    backfill_match_keys(connection)?;
    Ok(())
}

fn validate_knowledge_identity_migration(transaction: &Transaction<'_>) -> AppResult<()> {
    backfill_binding_hashes(transaction, "knowledge_item_source_bindings")?;
    backfill_binding_hashes(transaction, "document_artifact_source_bindings")?;
    let text_revisions = {
        let mut statement = transaction.prepare(
            "SELECT dr.id, dt.content
             FROM document_revisions dr
             JOIN document_texts dt
               ON dt.document_id = dr.document_id
              AND dt.source_version = dr.source_version
             WHERE dr.revision_kind = 'source-provided-text'",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for (revision_id, text) in text_revisions {
        transaction.execute(
            "UPDATE document_revisions SET content_hash = ?2 WHERE id = ?1",
            params![revision_id, content_hash(&text)],
        )?;
    }
    let invalid_items: i64 = transaction.query_row(
        "SELECT COUNT(*)
         FROM research_items i
         WHERE (
             SELECT COUNT(*)
             FROM knowledge_item_source_bindings b
             WHERE b.knowledge_item_id = i.id
         ) <> 1",
        [],
        |row| row.get(0),
    )?;
    let invalid_artifacts: i64 = transaction.query_row(
        "SELECT COUNT(*)
         FROM documents d
         WHERE (
             SELECT COUNT(*)
             FROM document_artifact_source_bindings b
             WHERE b.artifact_id = d.id
         ) <> 1",
        [],
        |row| row.get(0),
    )?;
    let mismatched_parents: i64 = transaction.query_row(
        "SELECT COUNT(*)
         FROM documents d
         JOIN document_artifact_source_bindings ab
           ON ab.artifact_id = d.id
         LEFT JOIN knowledge_item_source_bindings ib
           ON ib.id = ab.knowledge_item_source_binding_id
         WHERE d.research_item_id IS NOT ib.knowledge_item_id",
        [],
        |row| row.get(0),
    )?;
    let foreign_key_failures = {
        let mut statement = transaction.prepare("PRAGMA foreign_key_check")?;
        let count = statement
            .query([])?
            .mapped(|row| row.get::<_, String>(0))
            .count();
        count
    };
    if invalid_items != 0
        || invalid_artifacts != 0
        || mismatched_parents != 0
        || foreign_key_failures != 0
    {
        return Err(AppError::ApplicationData(format!(
            "knowledge identity migration validation failed: {invalid_items} item bindings, {invalid_artifacts} artifact bindings, {mismatched_parents} parent links, {foreign_key_failures} foreign keys"
        )));
    }
    Ok(())
}

fn backfill_binding_hashes(transaction: &Transaction<'_>, table: &str) -> AppResult<()> {
    let metadata_column = if table == "knowledge_item_source_bindings" {
        "normalized_source_metadata_json"
    } else {
        "source_metadata_json"
    };
    let sql = format!("SELECT id, {metadata_column} FROM {table}");
    let rows = {
        let mut statement = transaction.prepare(&sql)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    let update_sql = format!("UPDATE {table} SET metadata_hash = ?2 WHERE id = ?1");
    for (binding_id, metadata) in rows {
        transaction.execute(&update_sql, params![binding_id, content_hash(&metadata)])?;
    }
    Ok(())
}

fn backfill_extraction_projection_hashes(transaction: &Transaction<'_>) -> AppResult<()> {
    let rows = {
        let mut statement = transaction
            .prepare("SELECT id, content FROM document_texts WHERE content_hash = ''")?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for (text_id, text) in rows {
        transaction.execute(
            "UPDATE document_texts SET content_hash = ?2 WHERE id = ?1",
            params![text_id, content_hash(&text)],
        )?;
    }
    let foreign_key_failures = {
        let mut statement = transaction.prepare("PRAGMA foreign_key_check")?;
        let count = statement
            .query([])?
            .mapped(|row| row.get::<_, String>(0))
            .count();
        count
    };
    if foreign_key_failures != 0 {
        return Err(AppError::ApplicationData(format!(
            "canonical extraction migration validation failed: {foreign_key_failures} foreign keys"
        )));
    }
    Ok(())
}

fn validate_extraction_selection_migration(transaction: &Transaction<'_>) -> AppResult<()> {
    let inconsistent_selections: i64 = transaction.query_row(
        "SELECT COUNT(*)
         FROM active_extraction_selections selection
         JOIN document_extractions extraction
           ON extraction.id = selection.extraction_id
         JOIN extraction_quality_reports report
           ON report.id = selection.quality_report_id
         WHERE extraction.artifact_id <> selection.artifact_id
            OR report.extraction_id <> selection.extraction_id",
        [],
        |row| row.get(0),
    )?;
    let mutable_attempt_markers: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM document_extractions WHERE selected <> 0",
        [],
        |row| row.get(0),
    )?;
    let foreign_key_failures = {
        let mut statement = transaction.prepare("PRAGMA foreign_key_check")?;
        let count = statement
            .query([])?
            .mapped(|row| row.get::<_, String>(0))
            .count();
        count
    };
    if inconsistent_selections != 0 || mutable_attempt_markers != 0 || foreign_key_failures != 0 {
        return Err(AppError::ApplicationData(format!(
            "extraction selection migration validation failed: {inconsistent_selections} inconsistent selections, {mutable_attempt_markers} mutable attempt markers, {foreign_key_failures} foreign keys"
        )));
    }
    Ok(())
}

pub(crate) fn backfill_match_keys(connection: &Connection) -> AppResult<()> {
    let mut statement = connection.prepare(
        "SELECT i.id, i.source_id, i.library_id, i.title, i.doi,
                i.publication_year,
                (SELECT COALESCE(c.last_name, c.field_mode_name, c.display_name)
                 FROM creators c
                 WHERE c.item_id = i.id AND c.creator_type = 'author'
                 ORDER BY c.position LIMIT 1)
         FROM research_items i
         WHERE NOT EXISTS (
             SELECT 1 FROM research_item_match_keys k WHERE k.item_id = i.id
         )",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<i32>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (item_id, source_id, library_id, title, doi, year, first_author) in rows {
        connection.execute(
            "INSERT INTO research_item_match_keys (
                item_id, source_id, library_id, normalized_doi,
                normalized_title, publication_year, first_author_surname,
                updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                item_id,
                source_id,
                library_id,
                doi.as_deref().and_then(normalize_doi),
                normalize_title(&title),
                year,
                first_author.as_deref().map(normalize_author_surname),
                Utc::now().to_rfc3339(),
            ],
        )?;
    }
    Ok(())
}

fn persist_neutral_source_records(
    transaction: &Transaction<'_>,
    source_id: &str,
    library_id: &str,
    delta: &SourceDelta,
    observed_at: &str,
    affected_item_ids: &mut HashSet<String>,
) -> AppResult<()> {
    let instance_id = delta.source_instance_id.clone();
    let valid_instance: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM source_instances
            WHERE id = ?1 AND legacy_source_id = ?2
              AND legacy_library_id = ?3
         )",
        params![instance_id, source_id, library_id],
        |row| row.get(0),
    )?;
    if !valid_instance {
        return Err(AppError::ApplicationData(format!(
            "source delta instance {} does not match its compatibility source and library",
            delta.source_instance_id
        )));
    }
    transaction.execute(
        "UPDATE source_instances SET
            provider_kind = ?2,
            display_name = ?3,
            state = 'active',
            updated_at = ?4
         WHERE id = ?1",
        params![
            instance_id,
            delta.descriptor.provider_kind,
            delta.descriptor.display_name,
            observed_at,
        ],
    )?;
    for group in &delta.source_groups {
        persist_neutral_source_group(transaction, source_id, library_id, group)?;
    }
    for item in &delta.knowledge_items {
        let item_id = persist_neutral_knowledge_item(
            transaction,
            source_id,
            library_id,
            &instance_id,
            item,
            observed_at,
        )?;
        affected_item_ids.insert(item_id);
    }
    for artifact in &delta.artifacts {
        if let Some(item_id) = persist_neutral_document_artifact(
            transaction,
            source_id,
            library_id,
            &instance_id,
            artifact,
            observed_at,
        )? {
            affected_item_ids.insert(item_id);
        }
    }
    Ok(())
}

fn persist_neutral_source_group(
    transaction: &Transaction<'_>,
    source_id: &str,
    library_id: &str,
    group: &SourceGroupRecord,
) -> AppResult<()> {
    let id = stable_id("collection", &[source_id, library_id, &group.external_id]);
    let parent_id = group
        .parent_external_id
        .as_deref()
        .map(|key| stable_id("collection", &[source_id, library_id, key]));
    transaction.execute(
        "INSERT INTO collections (
            id, source_id, library_id, zotero_key, name,
            parent_id, source_version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            parent_id = excluded.parent_id,
            source_version = excluded.source_version",
        params![
            id,
            source_id,
            library_id,
            group.external_id,
            group.name,
            parent_id,
            group.source_revision,
        ],
    )?;
    Ok(())
}

fn persist_neutral_knowledge_item(
    transaction: &Transaction<'_>,
    source_id: &str,
    library_id: &str,
    instance_id: &str,
    item: &KnowledgeItemRecord,
    observed_at: &str,
) -> AppResult<String> {
    let existing_id = transaction
        .query_row(
            "SELECT knowledge_item_id
             FROM knowledge_item_source_bindings
             WHERE source_instance_id = ?1 AND external_item_id = ?2",
            params![instance_id, item.external_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let item_id = existing_id
        .unwrap_or_else(|| stable_id("item", &[source_id, library_id, &item.external_id]));
    transaction.execute(
        "INSERT INTO research_items (
            id, source_id, library_id, zotero_key, item_type,
            title, abstract_text, publication_year, source_version,
            is_trashed, knowledge_kind
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10)
         ON CONFLICT(id) DO UPDATE SET
            title = excluded.title,
            abstract_text = excluded.abstract_text,
            publication_year = excluded.publication_year,
            knowledge_kind = excluded.knowledge_kind,
            is_trashed = 0",
        params![
            item_id,
            source_id,
            library_id,
            item.external_id,
            item.external_kind,
            item.title,
            item.abstract_text,
            item.publication_year,
            item.source_revision,
            item.knowledge_kind.as_str(),
        ],
    )?;
    let metadata = serde_json::to_string(&item.source_metadata)
        .map_err(|error| AppError::ApplicationData(error.to_string()))?;
    let binding_id = source_qualified_binding_id(
        "knowledge-binding",
        instance_id,
        &item.external_id,
        &item_id,
        "knowledge-item",
    );
    let is_primary: bool = !transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM knowledge_item_source_bindings
            WHERE knowledge_item_id = ?1 AND state = 'active'
         )",
        [&item_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO knowledge_item_source_bindings (
            id, knowledge_item_id, source_instance_id,
            external_item_id, external_item_kind, source_revision,
            normalized_source_metadata_json, metadata_hash,
            is_primary, state, first_seen_at, last_seen_at, deleted_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            ?9, 'active', ?10, ?10, NULL
         )
         ON CONFLICT(source_instance_id, external_item_id) DO UPDATE SET
            knowledge_item_id = excluded.knowledge_item_id,
            external_item_kind = excluded.external_item_kind,
            source_revision = excluded.source_revision,
            normalized_source_metadata_json =
                excluded.normalized_source_metadata_json,
            metadata_hash = excluded.metadata_hash,
            state = 'active',
            last_seen_at = excluded.last_seen_at,
            deleted_at = NULL",
        params![
            binding_id,
            item_id,
            instance_id,
            item.external_id,
            item.external_kind,
            item.source_revision.to_string(),
            metadata,
            content_hash(&metadata),
            is_primary,
            observed_at,
        ],
    )?;
    Ok(item_id)
}

fn persist_neutral_document_artifact(
    transaction: &Transaction<'_>,
    source_id: &str,
    library_id: &str,
    instance_id: &str,
    artifact: &DocumentArtifactRecord,
    observed_at: &str,
) -> AppResult<Option<String>> {
    let existing_id = transaction
        .query_row(
            "SELECT artifact_id
             FROM document_artifact_source_bindings
             WHERE source_instance_id = ?1 AND external_artifact_id = ?2",
            params![instance_id, artifact.external_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let attachment_id = stable_id("item", &[source_id, library_id, &artifact.external_id]);
    let artifact_id = existing_id.unwrap_or_else(|| format!("document:{attachment_id}"));
    let parent_item_id = artifact
        .parent_external_item_id
        .as_deref()
        .map(|external_id| bound_knowledge_item_id(transaction, instance_id, external_id))
        .transpose()?
        .flatten();
    let parent_binding_id = artifact
        .parent_external_item_id
        .as_deref()
        .map(|external_id| {
            transaction
                .query_row(
                    "SELECT id FROM knowledge_item_source_bindings
                     WHERE source_instance_id = ?1
                       AND external_item_id = ?2 AND state = 'active'",
                    params![instance_id, external_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
        })
        .transpose()?
        .flatten();
    let link_mode = artifact
        .source_metadata
        .get("linkMode")
        .and_then(serde_json::Value::as_str);
    transaction.execute(
        "INSERT INTO attachments (
            id, source_id, library_id, zotero_key, parent_item_id,
            title, content_type, filename, link_mode,
            available_locally, source_version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10)
         ON CONFLICT(id) DO UPDATE SET
            parent_item_id = excluded.parent_item_id,
            title = excluded.title,
            content_type = excluded.content_type,
            filename = excluded.filename,
            link_mode = excluded.link_mode,
            source_version = excluded.source_version",
        params![
            attachment_id,
            source_id,
            library_id,
            artifact.external_id,
            parent_item_id,
            artifact.title,
            artifact.media_type,
            artifact.filename,
            link_mode,
            artifact.source_revision,
        ],
    )?;
    let availability = if artifact.media_type == "application/pdf" {
        "metadata-only"
    } else {
        "unsupported"
    };
    transaction.execute(
        "INSERT INTO documents (
            id, source_id, library_id, attachment_id,
            research_item_id, zotero_attachment_key, media_type,
            title, filename, link_mode, source_version,
            availability, created_at, updated_at, artifact_role
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?13, ?14
         )
         ON CONFLICT(id) DO UPDATE SET
            research_item_id = excluded.research_item_id,
            media_type = excluded.media_type,
            title = excluded.title,
            filename = excluded.filename,
            source_version = excluded.source_version,
            artifact_role = excluded.artifact_role,
            updated_at = excluded.updated_at",
        params![
            artifact_id,
            source_id,
            library_id,
            attachment_id,
            parent_item_id,
            artifact.external_id,
            artifact.media_type,
            artifact.title,
            artifact.filename,
            link_mode,
            artifact.source_revision,
            availability,
            observed_at,
            artifact.artifact_role.as_str(),
        ],
    )?;
    let metadata = serde_json::to_string(&artifact.source_metadata)
        .map_err(|error| AppError::ApplicationData(error.to_string()))?;
    let binding_id = source_qualified_binding_id(
        "artifact-binding",
        instance_id,
        &artifact.external_id,
        &artifact_id,
        "document-artifact",
    );
    transaction.execute(
        "INSERT INTO document_artifact_source_bindings (
            id, artifact_id, source_instance_id,
            knowledge_item_source_binding_id, external_artifact_id,
            source_revision, source_metadata_json, metadata_hash,
            state, first_seen_at, last_seen_at, deleted_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            'active', ?9, ?9, NULL
         )
         ON CONFLICT(source_instance_id, external_artifact_id) DO UPDATE SET
            artifact_id = excluded.artifact_id,
            knowledge_item_source_binding_id =
                excluded.knowledge_item_source_binding_id,
            source_revision = excluded.source_revision,
            source_metadata_json = excluded.source_metadata_json,
            metadata_hash = excluded.metadata_hash,
            state = 'active',
            last_seen_at = excluded.last_seen_at,
            deleted_at = NULL",
        params![
            binding_id,
            artifact_id,
            instance_id,
            parent_binding_id,
            artifact.external_id,
            artifact.source_revision.to_string(),
            metadata,
            content_hash(&metadata),
            observed_at,
        ],
    )?;
    Ok(parent_item_id)
}

fn upsert_collection(
    transaction: &Transaction<'_>,
    source_id: &str,
    library_id: &str,
    collection: &crate::normalize::NormalizedCollection,
) -> AppResult<()> {
    let id = stable_id("collection", &[source_id, library_id, &collection.key]);
    let parent_id = collection
        .parent_key
        .as_deref()
        .map(|key| stable_id("collection", &[source_id, library_id, key]));
    transaction.execute(
        "INSERT INTO collections (
            id, source_id, library_id, zotero_key, name,
            parent_id, source_version
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            parent_id = excluded.parent_id,
            source_version = excluded.source_version",
        params![
            id,
            source_id,
            library_id,
            collection.key,
            collection.name,
            parent_id,
            collection.version,
        ],
    )?;
    Ok(())
}

fn upsert_item(
    transaction: &Transaction<'_>,
    source_id: &str,
    library_id: &str,
    item: &NormalizedItem,
) -> AppResult<Option<String>> {
    let instance_id = source_instance_id(transaction, library_id)?;
    let id = bound_knowledge_item_id(transaction, &instance_id, &item.key)?
        .unwrap_or_else(|| stable_id("item", &[source_id, library_id, &item.key]));
    let parent_id = item
        .parent_key
        .as_deref()
        .map(|key| bound_knowledge_item_id(transaction, &instance_id, key))
        .transpose()?
        .flatten()
        .or_else(|| {
            item.parent_key
                .as_deref()
                .map(|key| stable_id("item", &[source_id, library_id, key]))
        });
    match item.item_type.as_str() {
        "attachment" => {
            let document_id = bound_artifact_id(transaction, &instance_id, &item.key)?
                .unwrap_or_else(|| format!("document:{id}"));
            let previous_document_version = transaction
                .query_row(
                    "SELECT source_version FROM documents WHERE id = ?1",
                    [&document_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            transaction.execute(
                "DELETE FROM research_items WHERE library_id = ?1 AND zotero_key = ?2",
                params![library_id, item.key],
            )?;
            transaction.execute(
                "DELETE FROM notes WHERE library_id = ?1 AND zotero_key = ?2",
                params![library_id, item.key],
            )?;
            transaction.execute(
                "INSERT INTO attachments (
                    id, source_id, library_id, zotero_key, parent_item_id,
                    title, content_type, filename, link_mode,
                    available_locally, source_version
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET
                    parent_item_id = excluded.parent_item_id,
                    title = excluded.title,
                    content_type = excluded.content_type,
                    filename = excluded.filename,
                    link_mode = excluded.link_mode,
                    available_locally = excluded.available_locally,
                    source_version = excluded.source_version",
                params![
                    id,
                    source_id,
                    library_id,
                    item.key,
                    parent_id,
                    item.title,
                    item.content_type,
                    item.filename,
                    item.link_mode,
                    item.attachment_available,
                    item.version,
                ],
            )?;
            let media_type = item
                .content_type
                .as_deref()
                .unwrap_or("application/octet-stream");
            let availability = if media_type == "application/pdf" {
                "metadata-only"
            } else {
                "unsupported"
            };
            let now = Utc::now().to_rfc3339();
            transaction.execute(
                "INSERT INTO documents (
                    id, source_id, library_id, attachment_id,
                    research_item_id, zotero_attachment_key, media_type,
                    title, filename, link_mode, source_version,
                    availability, created_at, updated_at, artifact_role
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?13, 'attachment'
                 )
                 ON CONFLICT(id) DO UPDATE SET
                    research_item_id = excluded.research_item_id,
                    media_type = excluded.media_type,
                    title = excluded.title,
                    filename = excluded.filename,
                    link_mode = excluded.link_mode,
                    source_version = excluded.source_version,
                    availability = CASE
                        WHEN documents.availability = 'ready'
                             AND excluded.media_type = 'application/pdf'
                        THEN documents.availability
                        ELSE excluded.availability
                    END,
                    updated_at = excluded.updated_at",
                params![
                    document_id,
                    source_id,
                    library_id,
                    id,
                    parent_id,
                    item.key,
                    media_type,
                    item.title,
                    item.filename,
                    item.link_mode,
                    item.version,
                    availability,
                    now,
                ],
            )?;
            upsert_artifact_source_binding(
                transaction,
                library_id,
                &document_id,
                parent_id.as_deref(),
                item,
                &now,
            )?;
            if previous_document_version.is_some_and(|version| version != item.version) {
                invalidate_document_citations(transaction, &document_id, None)?;
            }
            return Ok(parent_id);
        }
        "note" => {
            transaction.execute(
                "DELETE FROM research_items WHERE library_id = ?1 AND zotero_key = ?2",
                params![library_id, item.key],
            )?;
            transaction.execute(
                "DELETE FROM attachments WHERE library_id = ?1 AND zotero_key = ?2",
                params![library_id, item.key],
            )?;
            transaction.execute(
                "INSERT INTO notes (
                    id, source_id, library_id, zotero_key, parent_item_id,
                    title, content, source_version
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                    parent_item_id = excluded.parent_item_id,
                    title = excluded.title,
                    content = excluded.content,
                    source_version = excluded.source_version",
                params![
                    id,
                    source_id,
                    library_id,
                    item.key,
                    parent_id,
                    item.title,
                    item.note_content,
                    item.version,
                ],
            )?;
            return Ok(parent_id);
        }
        "annotation" => return Ok(parent_id),
        _ if item.is_top_level_research_item() => {
            transaction.execute(
                "DELETE FROM attachments WHERE library_id = ?1 AND zotero_key = ?2",
                params![library_id, item.key],
            )?;
            transaction.execute(
                "DELETE FROM notes WHERE library_id = ?1 AND zotero_key = ?2",
                params![library_id, item.key],
            )?;
            transaction.execute(
                "INSERT INTO research_items (
                    id, source_id, library_id, zotero_key, item_type,
                    title, abstract_text, date_text, publication_year,
                    venue, volume, issue, pages, publisher, doi, isbn,
                    issn, url, date_added, date_modified, source_version,
                    is_trashed, knowledge_kind
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                    ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                    ?21, ?22, ?23
                 )
                 ON CONFLICT(id) DO UPDATE SET
                    item_type = excluded.item_type,
                    knowledge_kind = excluded.knowledge_kind,
                    title = excluded.title,
                    abstract_text = excluded.abstract_text,
                    date_text = excluded.date_text,
                    publication_year = excluded.publication_year,
                    venue = excluded.venue,
                    volume = excluded.volume,
                    issue = excluded.issue,
                    pages = excluded.pages,
                    publisher = excluded.publisher,
                    doi = excluded.doi,
                    isbn = excluded.isbn,
                    issn = excluded.issn,
                    url = excluded.url,
                    date_added = excluded.date_added,
                    date_modified = excluded.date_modified,
                    source_version = excluded.source_version,
                    is_trashed = excluded.is_trashed",
                params![
                    id,
                    source_id,
                    library_id,
                    item.key,
                    item.item_type,
                    item.title,
                    item.abstract_text,
                    item.date,
                    item.year,
                    item.venue,
                    item.volume,
                    item.issue,
                    item.pages,
                    item.publisher,
                    item.doi,
                    item.isbn,
                    item.issn,
                    item.url,
                    item.date_added,
                    item.date_modified,
                    item.version,
                    item.is_trashed,
                    knowledge_kind(item.item_type.as_str()),
                ],
            )?;
            transaction.execute("DELETE FROM creators WHERE item_id = ?1", [&id])?;
            transaction.execute("DELETE FROM item_tags WHERE item_id = ?1", [&id])?;
            transaction.execute("DELETE FROM item_collections WHERE item_id = ?1", [&id])?;
            transaction.execute("DELETE FROM item_relations WHERE item_id = ?1", [&id])?;
            for creator in &item.creators {
                transaction.execute(
                    "INSERT INTO creators (
                        item_id, position, creator_type, first_name,
                        last_name, field_mode_name, display_name,
                        normalized_display_name
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        id,
                        creator.position as i64,
                        creator.creator_type,
                        creator.first_name,
                        creator.last_name,
                        creator.field_mode_name,
                        creator.display_name,
                        normalize_facet(&creator.display_name),
                    ],
                )?;
            }
            for tag in &item.tags {
                transaction.execute(
                    "INSERT OR REPLACE INTO item_tags (
                        item_id, tag, normalized_tag, tag_type
                     )
                     VALUES (?1, ?2, ?3, ?4)",
                    params![id, tag.value, normalize_facet(&tag.value), tag.tag_type],
                )?;
            }
            for collection_key in &item.collection_keys {
                let collection_id =
                    stable_id("collection", &[source_id, library_id, collection_key]);
                transaction.execute(
                    "INSERT OR IGNORE INTO item_collections (
                        item_id, collection_id, collection_key
                     ) VALUES (?1, ?2, ?3)",
                    params![id, collection_id, collection_key],
                )?;
            }
            for relation in &item.relations {
                transaction.execute(
                    "INSERT INTO item_relations (item_id, predicate, target)
                     VALUES (?1, ?2, ?3)",
                    params![id, relation.predicate, relation.target],
                )?;
            }
            refresh_item_match_key(transaction, source_id, library_id, item, &id)?;
            upsert_knowledge_item_source_binding(transaction, library_id, &id, item)?;
            return Ok(Some(id));
        }
        _ => {}
    }
    Ok(parent_id)
}

fn knowledge_kind(item_type: &str) -> &'static str {
    match item_type {
        "journalArticle" | "conferencePaper" | "preprint" => "paper",
        "book" | "bookSection" => "book",
        "report" | "thesis" | "manuscript" => "report",
        "presentation" => "presentation",
        "dataset" => "dataset",
        _ => "other",
    }
}

fn source_instance_id(transaction: &Transaction<'_>, library_id: &str) -> AppResult<String> {
    transaction
        .query_row(
            "SELECT id FROM source_instances WHERE legacy_library_id = ?1",
            [library_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn bound_knowledge_item_id(
    transaction: &Transaction<'_>,
    source_instance_id: &str,
    external_item_id: &str,
) -> AppResult<Option<String>> {
    transaction
        .query_row(
            "SELECT knowledge_item_id
             FROM knowledge_item_source_bindings
             WHERE source_instance_id = ?1 AND external_item_id = ?2",
            params![source_instance_id, external_item_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(|value| value.flatten())
        .map_err(Into::into)
}

fn bound_artifact_id(
    transaction: &Transaction<'_>,
    source_instance_id: &str,
    external_artifact_id: &str,
) -> AppResult<Option<String>> {
    transaction
        .query_row(
            "SELECT artifact_id
             FROM document_artifact_source_bindings
             WHERE source_instance_id = ?1 AND external_artifact_id = ?2",
            params![source_instance_id, external_artifact_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(|value| value.flatten())
        .map_err(Into::into)
}

fn source_qualified_binding_id(
    prefix: &str,
    source_instance_id: &str,
    external_id: &str,
    entity_id: &str,
    binding_kind: &str,
) -> String {
    stable_id(
        prefix,
        &[source_instance_id, external_id, entity_id, binding_kind],
    )
}

fn upsert_knowledge_item_source_binding(
    transaction: &Transaction<'_>,
    library_id: &str,
    item_id: &str,
    item: &NormalizedItem,
) -> AppResult<()> {
    let instance_id = source_instance_id(transaction, library_id)?;
    let metadata = serde_json::json!({
        "itemKind": item.item_type,
        "dateAdded": item.date_added,
        "dateModified": item.date_modified,
    })
    .to_string();
    let now = Utc::now().to_rfc3339();
    let binding_id = source_qualified_binding_id(
        "knowledge-binding",
        &instance_id,
        &item.key,
        item_id,
        "knowledge-item",
    );
    transaction.execute(
        "INSERT INTO knowledge_item_source_bindings (
            id, knowledge_item_id, source_instance_id,
            external_item_id, external_item_kind, source_revision,
            normalized_source_metadata_json, metadata_hash,
            is_primary, state, first_seen_at, last_seen_at, deleted_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            1, 'active', ?9, ?9, NULL
         )
         ON CONFLICT(source_instance_id, external_item_id) DO UPDATE SET
            knowledge_item_id = excluded.knowledge_item_id,
            external_item_kind = excluded.external_item_kind,
            source_revision = excluded.source_revision,
            normalized_source_metadata_json =
                excluded.normalized_source_metadata_json,
            metadata_hash = excluded.metadata_hash,
            state = 'active',
            last_seen_at = excluded.last_seen_at,
            deleted_at = NULL",
        params![
            binding_id,
            item_id,
            instance_id,
            item.key,
            item.item_type,
            item.version.to_string(),
            metadata,
            content_hash(&metadata),
            now,
        ],
    )?;
    Ok(())
}

fn upsert_artifact_source_binding(
    transaction: &Transaction<'_>,
    library_id: &str,
    document_id: &str,
    parent_item_id: Option<&str>,
    item: &NormalizedItem,
    now: &str,
) -> AppResult<()> {
    let instance_id = source_instance_id(transaction, library_id)?;
    let parent_binding_id = parent_item_id
        .map(|parent_id| {
            transaction
                .query_row(
                    "SELECT id
                     FROM knowledge_item_source_bindings
                     WHERE knowledge_item_id = ?1
                       AND source_instance_id = ?2
                       AND state = 'active'",
                    params![parent_id, instance_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
        })
        .transpose()?
        .flatten();
    let metadata = serde_json::json!({
        "mediaType": item.content_type,
        "linkMode": item.link_mode,
        "filename": item.filename,
    })
    .to_string();
    let binding_id = source_qualified_binding_id(
        "artifact-binding",
        &instance_id,
        &item.key,
        document_id,
        "document-artifact",
    );
    transaction.execute(
        "INSERT INTO document_artifact_source_bindings (
            id, artifact_id, source_instance_id,
            knowledge_item_source_binding_id, external_artifact_id,
            source_revision, source_metadata_json, metadata_hash,
            state, first_seen_at, last_seen_at, deleted_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            'active', ?9, ?9, NULL
         )
         ON CONFLICT(source_instance_id, external_artifact_id) DO UPDATE SET
            artifact_id = excluded.artifact_id,
            knowledge_item_source_binding_id =
                excluded.knowledge_item_source_binding_id,
            source_revision = excluded.source_revision,
            source_metadata_json = excluded.source_metadata_json,
            metadata_hash = excluded.metadata_hash,
            state = 'active',
            last_seen_at = excluded.last_seen_at,
            deleted_at = NULL",
        params![
            binding_id,
            document_id,
            instance_id,
            parent_binding_id,
            item.key,
            item.version.to_string(),
            metadata,
            content_hash(&metadata),
            now,
        ],
    )?;
    Ok(())
}

fn affected_research_items(
    transaction: &Transaction<'_>,
    library_id: &str,
    key: &str,
) -> AppResult<Vec<String>> {
    let mut statement = transaction.prepare(
        "SELECT id FROM research_items
         WHERE library_id = ?1 AND zotero_key = ?2
         UNION
         SELECT parent_item_id FROM attachments
         WHERE library_id = ?1 AND zotero_key = ?2
           AND parent_item_id IS NOT NULL
         UNION
         SELECT parent_item_id FROM notes
         WHERE library_id = ?1 AND zotero_key = ?2
           AND parent_item_id IS NOT NULL",
    )?;
    let rows = statement.query_map(params![library_id, key], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn refresh_item_match_key(
    transaction: &Transaction<'_>,
    source_id: &str,
    library_id: &str,
    item: &NormalizedItem,
    item_id: &str,
) -> AppResult<()> {
    let first_author = item
        .creators
        .iter()
        .find(|creator| creator.creator_type == "author")
        .and_then(|creator| {
            creator
                .last_name
                .as_deref()
                .or(creator.field_mode_name.as_deref())
                .or(Some(creator.display_name.as_str()))
        })
        .map(normalize_author_surname)
        .filter(|value| !value.is_empty());
    let normalized_doi = item.doi.as_deref().and_then(normalize_doi);
    transaction.execute(
        "INSERT INTO research_item_match_keys (
            item_id, source_id, library_id, normalized_doi,
            normalized_title, publication_year, first_author_surname,
            updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(item_id) DO UPDATE SET
            source_id = excluded.source_id,
            library_id = excluded.library_id,
            normalized_doi = excluded.normalized_doi,
            normalized_title = excluded.normalized_title,
            publication_year = excluded.publication_year,
            first_author_surname = excluded.first_author_surname,
            updated_at = excluded.updated_at",
        params![
            item_id,
            source_id,
            library_id,
            normalized_doi,
            normalize_title(&item.title),
            item.year,
            first_author,
            Utc::now().to_rfc3339(),
        ],
    )?;
    transaction.execute(
        "INSERT OR REPLACE INTO citation_resolution_queue (
            reference_id, reason, queued_at
         )
         SELECT r.id, 'local-metadata-changed', ?5
         FROM bibliographic_references r
         JOIN reference_extractions e ON e.id = r.extraction_id
         WHERE e.is_current = 1 AND r.source_id = ?1
           AND (
               (?2 IS NOT NULL AND r.normalized_doi = ?2)
               OR (
                   ?3 IS NOT NULL AND ?4 IS NOT NULL
                   AND r.parsed_year = ?3 AND r.first_author = ?4
               )
           )",
        params![
            source_id,
            normalized_doi,
            item.year,
            first_author,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn invalidate_document_citations(
    transaction: &Transaction<'_>,
    document_id: &str,
    replacement_text_hash: Option<&str>,
) -> AppResult<()> {
    let hash_condition = replacement_text_hash
        .map(|_| " AND source_text_hash <> ?2")
        .unwrap_or("");
    let stale_query = format!(
        "SELECT id FROM reference_extractions
         WHERE document_id = ?1 AND is_current = 1{hash_condition}"
    );
    let extraction_ids = if let Some(hash) = replacement_text_hash {
        let mut statement = transaction.prepare(&stale_query)?;
        let rows = statement
            .query_map(params![document_id, hash], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    } else {
        let mut statement = transaction.prepare(&stale_query)?;
        let rows = statement
            .query_map([document_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for extraction_id in extraction_ids {
        transaction.execute(
            "UPDATE reference_resolution_overrides
             SET stale = 1, updated_at = ?2
             WHERE reference_id IN (
                 SELECT id FROM bibliographic_references
                 WHERE extraction_id = ?1
             )",
            params![extraction_id, Utc::now().to_rfc3339()],
        )?;
        transaction.execute(
            "DELETE FROM citation_evidence
             WHERE reference_id IN (
                 SELECT id FROM bibliographic_references
                 WHERE extraction_id = ?1
             )",
            [&extraction_id],
        )?;
        transaction.execute(
            "UPDATE reference_extractions
             SET is_current = 0, status = 'stale'
             WHERE id = ?1",
            [&extraction_id],
        )?;
    }
    transaction.execute(
        "DELETE FROM citation_edges
         WHERE NOT EXISTS (
             SELECT 1 FROM citation_evidence ce
             WHERE ce.citation_edge_id = citation_edges.id
         )",
        [],
    )?;
    transaction.execute(
        "UPDATE citation_edges
         SET evidence_count = (
             SELECT COUNT(*) FROM citation_evidence ce
             WHERE ce.citation_edge_id = citation_edges.id
         ), updated_at = ?1",
        [Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

pub(crate) fn refresh_search_content(
    transaction: &Transaction<'_>,
    item_id: &str,
) -> AppResult<()> {
    transaction.execute("DELETE FROM search_content WHERE item_id = ?1", [item_id])?;
    transaction.execute(
        "INSERT INTO search_content (
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
            COALESCE((
                SELECT group_concat(dt.content, char(10))
                FROM document_texts dt
                JOIN documents d ON d.id = dt.document_id
                WHERE d.research_item_id = i.id AND dt.is_active = 1
            ), '')
         FROM research_items i
         WHERE i.id = ?1 AND i.is_trashed = 0",
        [item_id],
    )?;
    Ok(())
}

enum BindingDeletion {
    Preserved,
    Artifact(String),
    KnowledgeItem(String),
    LegacyOnly,
}

fn tombstone_entity_binding(
    transaction: &Transaction<'_>,
    library_id: &str,
    external_id: &str,
    deleted_at: &str,
) -> AppResult<BindingDeletion> {
    let instance_id = source_instance_id(transaction, library_id)?;
    let artifact_id = transaction
        .query_row(
            "SELECT artifact_id
             FROM document_artifact_source_bindings
             WHERE source_instance_id = ?1 AND external_artifact_id = ?2",
            params![instance_id, external_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    if let Some(artifact_id) = artifact_id {
        transaction.execute(
            "UPDATE document_artifact_source_bindings
             SET state = 'deleted', last_seen_at = ?3, deleted_at = ?3
             WHERE source_instance_id = ?1 AND external_artifact_id = ?2",
            params![instance_id, external_id, deleted_at],
        )?;
        let active_bindings: i64 = transaction.query_row(
            "SELECT COUNT(*)
             FROM document_artifact_source_bindings
             WHERE artifact_id = ?1 AND state = 'active'",
            [&artifact_id],
            |row| row.get(0),
        )?;
        return Ok(if active_bindings > 0 {
            BindingDeletion::Preserved
        } else {
            BindingDeletion::Artifact(artifact_id)
        });
    }
    let item_id = transaction
        .query_row(
            "SELECT knowledge_item_id
             FROM knowledge_item_source_bindings
             WHERE source_instance_id = ?1 AND external_item_id = ?2",
            params![instance_id, external_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    if let Some(item_id) = item_id {
        transaction.execute(
            "UPDATE knowledge_item_source_bindings
             SET state = 'deleted', last_seen_at = ?3, deleted_at = ?3
             WHERE source_instance_id = ?1 AND external_item_id = ?2",
            params![instance_id, external_id, deleted_at],
        )?;
        let active_bindings: i64 = transaction.query_row(
            "SELECT COUNT(*)
             FROM knowledge_item_source_bindings
             WHERE knowledge_item_id = ?1 AND state = 'active'",
            [&item_id],
            |row| row.get(0),
        )?;
        return Ok(if active_bindings > 0 {
            BindingDeletion::Preserved
        } else {
            BindingDeletion::KnowledgeItem(item_id)
        });
    }
    Ok(BindingDeletion::LegacyOnly)
}

fn delete_item(transaction: &Transaction<'_>, library_id: &str, key: &str) -> AppResult<()> {
    transaction.execute(
        "DELETE FROM research_items WHERE library_id = ?1 AND zotero_key = ?2",
        params![library_id, key],
    )?;
    transaction.execute(
        "DELETE FROM attachments WHERE library_id = ?1 AND zotero_key = ?2",
        params![library_id, key],
    )?;
    transaction.execute(
        "DELETE FROM notes WHERE library_id = ?1 AND zotero_key = ?2",
        params![library_id, key],
    )?;
    Ok(())
}

fn tombstone_object(
    transaction: &Transaction<'_>,
    source_id: &str,
    library_id: &str,
    object_type: &str,
    key: &str,
    version: i64,
    fetched_at: &str,
) -> AppResult<()> {
    transaction.execute(
        "INSERT INTO source_objects (
            source_id, library_id, object_type, object_key,
            object_version, raw_json, content_hash, fetched_at, deleted
         ) VALUES (?1, ?2, ?3, ?4, ?5, '{}', '', ?6, 1)
         ON CONFLICT(library_id, object_type, object_key) DO UPDATE SET
            object_version = excluded.object_version,
            fetched_at = excluded.fetched_at,
            deleted = 1",
        params![source_id, library_id, object_type, key, version, fetched_at],
    )?;
    transaction.execute(
        "DELETE FROM normalization_diagnostics
         WHERE library_id = ?1 AND object_type = ?2 AND object_key = ?3",
        params![library_id, object_type, key],
    )?;
    Ok(())
}

fn load_libraries(connection: &Connection, source_id: &str) -> AppResult<Vec<LibraryDescriptor>> {
    let mut statement = connection.prepare(
        "SELECT l.id, l.name, l.kind, l.last_synced_at,
                COUNT(i.id)
         FROM libraries l
         LEFT JOIN research_items i
           ON i.library_id = l.id AND i.is_trashed = 0
         WHERE l.source_id = ?1 AND l.available = 1
         GROUP BY l.id
         ORDER BY CASE l.kind WHEN 'user' THEN 0 ELSE 1 END, l.name",
    )?;
    let rows = statement.query_map([source_id], |row| {
        Ok(LibraryDescriptor {
            id: row.get(0)?,
            name: row.get(1)?,
            kind: row.get(2)?,
            last_synced_at: row.get(3)?,
            item_count: row.get::<_, i64>(4)? as u64,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_collections(
    connection: &Connection,
    library_id: &str,
) -> AppResult<Vec<ResearchCollection>> {
    let mut statement = connection.prepare(
        "SELECT id, name, parent_id, library_id
         FROM collections WHERE library_id = ?1 ORDER BY name",
    )?;
    let rows = statement.query_map([library_id], |row| {
        Ok(ResearchCollection {
            id: row.get(0)?,
            name: row.get(1)?,
            description: String::new(),
            parent_id: row.get(2)?,
            library_id: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn prepare_search_item_scope(
    connection: &Connection,
    library_id: &str,
    requested_item_ids: &[String],
) -> AppResult<bool> {
    connection.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS search_scope_item_ids (
            id TEXT PRIMARY KEY
         ) WITHOUT ROWID;
         DELETE FROM search_scope_item_ids;",
    )?;
    if requested_item_ids.is_empty() {
        return Ok(false);
    }
    for item_id in requested_item_ids {
        connection.execute(
            "INSERT OR IGNORE INTO search_scope_item_ids(id)
             SELECT id FROM research_items
             WHERE id = ?1 AND library_id = ?2 AND is_trashed = 0",
            params![item_id, library_id],
        )?;
    }
    Ok(true)
}

fn load_selected_items(connection: &Connection) -> AppResult<Vec<ResearchItem>> {
    let mut creators: HashMap<String, Vec<Author>> = HashMap::new();
    let mut creator_statement = connection.prepare(
        "SELECT c.item_id, c.position, c.creator_type, c.display_name
         FROM creators c
         JOIN selected_item_ids s ON s.id = c.item_id
         ORDER BY c.item_id, c.position",
    )?;
    for row in creator_statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })? {
        let (item_id, position, creator_type, name) = row?;
        creators.entry(item_id.clone()).or_default().push(Author {
            id: format!("{item_id}:creator:{position}"),
            name,
            creator_type,
        });
    }

    let mut tags: HashMap<String, Vec<String>> = HashMap::new();
    let mut tag_statement = connection.prepare(
        "SELECT t.item_id, t.tag FROM item_tags t
         JOIN selected_item_ids s ON s.id = t.item_id
         ORDER BY t.item_id, t.tag",
    )?;
    for row in tag_statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (item_id, tag) = row?;
        tags.entry(item_id).or_default().push(tag);
    }

    let mut collection_ids: HashMap<String, Vec<String>> = HashMap::new();
    let mut membership_statement = connection.prepare(
        "SELECT m.item_id, m.collection_id FROM item_collections m
         JOIN selected_item_ids s ON s.id = m.item_id
         ORDER BY m.item_id, m.collection_id",
    )?;
    for row in membership_statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (item_id, collection_id) = row?;
        collection_ids
            .entry(item_id)
            .or_default()
            .push(collection_id);
    }

    let mut attachments: HashMap<String, Vec<DocumentAttachment>> = HashMap::new();
    let mut attachment_statement = connection.prepare(
        "SELECT a.parent_item_id, a.id, d.id, d.media_type,
                d.availability, d.title, d.filename, d.link_mode,
                d.source_version, dt.provider, dt.completeness,
                dt.indexed_pages, dt.total_pages, dt.indexed_chars,
                dt.total_chars, dt.source_version
         FROM attachments a
         JOIN selected_item_ids s ON s.id = a.parent_item_id
         JOIN documents d ON d.attachment_id = a.id
         LEFT JOIN document_texts dt
           ON dt.document_id = d.id
          AND dt.is_active = 1
         ORDER BY d.title",
    )?;
    for row in attachment_statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            DocumentAttachment {
                id: row.get(1)?,
                document_id: row.get(2)?,
                media_type: row.get(3)?,
                availability: row.get(4)?,
                title: row.get(5)?,
                filename: row.get(6)?,
                link_mode: row.get(7)?,
                source_version: row.get(8)?,
                page_count: row.get(12)?,
                text: row
                    .get::<_, Option<String>>(9)?
                    .map(|provider| DocumentTextSummary {
                        provider,
                        status: row.get(10).unwrap_or_else(|_| "unknown".to_owned()),
                        indexed_pages: row.get(11).ok().flatten(),
                        total_pages: row.get(12).ok().flatten(),
                        indexed_chars: row
                            .get::<_, Option<i64>>(13)
                            .ok()
                            .flatten()
                            .map(|v| v as u64),
                        total_chars: row
                            .get::<_, Option<i64>>(14)
                            .ok()
                            .flatten()
                            .map(|v| v as u64),
                        source_version: row.get(15).unwrap_or_default(),
                    }),
            },
        ))
    })? {
        let (parent_id, attachment) = row?;
        attachments.entry(parent_id).or_default().push(attachment);
    }

    let mut notes: HashMap<String, Vec<ResearchNote>> = HashMap::new();
    let mut note_statement = connection.prepare(
        "SELECT notes.parent_item_id, notes.id, notes.title, notes.content
         FROM notes
         JOIN selected_item_ids s ON s.id = notes.parent_item_id
         ORDER BY notes.title",
    )?;
    for row in note_statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            ResearchNote {
                id: row.get(1)?,
                title: row.get(2)?,
                content: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            },
        ))
    })? {
        let (parent_id, note) = row?;
        notes.entry(parent_id).or_default().push(note);
    }

    let mut statement = connection.prepare(
        "SELECT i.id, i.source_id, i.library_id, i.zotero_key,
                i.item_type, i.title, i.abstract_text, i.date_text,
                i.publication_year, i.venue, i.volume, i.issue,
                i.pages, i.publisher, i.doi, i.isbn, i.issn, i.url,
                i.date_added, i.date_modified, i.source_version,
                (SELECT COUNT(*) FROM citation_edges ce
                 WHERE ce.target_item_id = i.id)
         FROM research_items i
         JOIN selected_item_ids s ON s.id = i.id
         ORDER BY i.title",
    )?;
    let rows = statement.query_map([], |row| {
        let id: String = row.get(0)?;
        Ok(ResearchItem {
            authors: creators.remove(&id).unwrap_or_default(),
            tags: tags.remove(&id).unwrap_or_default(),
            collection_ids: collection_ids.remove(&id).unwrap_or_default(),
            attachments: attachments.remove(&id).unwrap_or_default(),
            notes: notes.remove(&id).unwrap_or_default(),
            id,
            provenance: ItemProvenance {
                source_id: row.get(1)?,
                library_id: row.get(2)?,
                zotero_key: row.get(3)?,
                zotero_item_type: row.get(4)?,
                source_version: row.get(20)?,
                date_added: row.get(18)?,
                date_modified: row.get(19)?,
            },
            title: row.get(5)?,
            abstract_text: row.get(6)?,
            date: row.get(7)?,
            year: row.get(8)?,
            venue: row.get(9)?,
            volume: row.get(10)?,
            issue: row.get(11)?,
            pages: row.get(12)?,
            publisher: row.get(13)?,
            doi: row.get(14)?,
            isbn: row.get(15)?,
            issn: row.get(16)?,
            url: row.get(17)?,
            topics: Vec::new(),
            methods: Vec::new(),
            citation_count: row.get::<_, u32>(21)?,
            importance: 0.45 + (row.get::<_, u32>(21)?.min(25) as f64 * 0.012),
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn load_diagnostics(connection: &Connection) -> AppResult<SourceDiagnostics> {
    let Some(source_id) = meta_value(connection, "active_source_id")? else {
        return Ok(SourceDiagnostics::empty());
    };
    let mut diagnostics = connection.query_row(
        "SELECT server_id, api_version, schema_version, connection_status,
                last_error
         FROM sources WHERE id = ?1",
        [&source_id],
        |row| {
            Ok(SourceDiagnostics {
                mode: if source_id == crate::demo::catalog::SOURCE_ID {
                    "demo"
                } else {
                    "zotero"
                }
                .to_owned(),
                health: "ready".to_owned(),
                connection_status: row.get(3)?,
                server_id: row.get(0)?,
                api_version: row.get(1)?,
                schema_version: row.get(2)?,
                last_synced_at: None,
                last_error: row.get(4)?,
                active_library_id: None,
                source_changed: false,
                sync_summary: None,
            })
        },
    )?;
    diagnostics.active_library_id = meta_value(connection, "active_library_id")?;
    diagnostics.last_synced_at = connection
        .query_row(
            "SELECT MAX(last_synced_at) FROM libraries WHERE source_id = ?1",
            [&source_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    diagnostics.health = if diagnostics.last_synced_at.is_some() {
        if diagnostics.connection_status == "online" {
            "ready".to_owned()
        } else {
            "offline-cache".to_owned()
        }
    } else {
        "not-synced".to_owned()
    };
    diagnostics.sync_summary = connection
        .query_row(
            "SELECT status, libraries_scanned, objects_changed,
                    objects_deleted, normalization_warnings,
                    started_at, finished_at
             FROM sync_runs WHERE source_id = ?1 ORDER BY id DESC LIMIT 1",
            [&source_id],
            |row| {
                Ok(SyncSummary {
                    status: row.get(0)?,
                    libraries_scanned: row.get(1)?,
                    objects_changed: row.get::<_, i64>(2)? as u64,
                    objects_deleted: row.get::<_, i64>(3)? as u64,
                    normalization_warnings: row.get::<_, i64>(4)? as u64,
                    started_at: row.get(5)?,
                    finished_at: row.get(6)?,
                })
            },
        )
        .optional()?;
    Ok(diagnostics)
}

fn load_corpus_diagnostics(
    connection: &Connection,
    library_id: Option<&str>,
) -> AppResult<CorpusDiagnostics> {
    let Some(library_id) = library_id else {
        return Ok(CorpusDiagnostics {
            status: "not-indexed".to_owned(),
            document_count: 0,
            pdf_count: 0,
            resolved_pdf_count: 0,
            full_text_count: 0,
            partial_text_count: 0,
            indexed_pages: 0,
            successful_version: 0,
            last_synced_at: None,
            changed_documents: 0,
            warning_count: 0,
            last_error: None,
        });
    };
    let (
        document_count,
        pdf_count,
        resolved_pdf_count,
        full_text_count,
        partial_text_count,
        indexed_pages,
    ) = connection.query_row(
        "SELECT
                COUNT(DISTINCT d.id),
                COUNT(DISTINCT CASE WHEN d.media_type = 'application/pdf' THEN d.id END),
                COUNT(DISTINCT CASE WHEN d.availability = 'ready' THEN d.id END),
                COUNT(DISTINCT CASE WHEN dt.completeness = 'full' THEN d.id END),
                COUNT(DISTINCT CASE WHEN dt.completeness = 'partial' THEN d.id END),
                COALESCE(SUM(dt.indexed_pages), 0)
             FROM documents d
             LEFT JOIN document_texts dt
               ON dt.document_id = d.id
                  AND dt.is_active = 1
             WHERE d.library_id = ?1",
        [library_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as u64,
                row.get::<_, i64>(3)? as u64,
                row.get::<_, i64>(4)? as u64,
                row.get::<_, i64>(5)? as u64,
            ))
        },
    )?;
    let state = connection
        .query_row(
            "SELECT successful_version, status, last_synced_at,
                    changed_documents, warning_count, last_error
             FROM fulltext_sync_state WHERE library_id = ?1",
            [library_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .optional()?;
    let (successful_version, status, last_synced_at, changed_documents, warning_count, last_error) =
        state.unwrap_or((0, "not-indexed".to_owned(), None, 0, 0, None));
    Ok(CorpusDiagnostics {
        status,
        document_count,
        pdf_count,
        resolved_pdf_count,
        full_text_count,
        partial_text_count,
        indexed_pages,
        successful_version,
        last_synced_at,
        changed_documents,
        warning_count,
        last_error,
    })
}

fn meta_value(connection: &Connection, key: &str) -> AppResult<Option<String>> {
    connection
        .query_row("SELECT value FROM app_meta WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(Into::into)
}

fn set_meta(connection: &Connection, key: &str, value: &str) -> AppResult<()> {
    connection.execute(
        "INSERT INTO app_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub(crate) fn stable_id(kind: &str, parts: &[&str]) -> String {
    format!("{kind}_{}", stable_suffix(&parts.join("\u{1f}")))
}

fn stable_suffix(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex_digest(hasher.finalize())[..20].to_owned()
}

pub(crate) fn content_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex_digest(hasher.finalize())
}

fn retains_local_extraction(connection: &Connection, document_id: &str) -> AppResult<bool> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM document_texts WHERE document_id=?1
         AND is_active=1 AND (provider LIKE 'canonical:docling-local-%'
           OR provider IN ('canonical:pdfjs-pages-v1',
             'canonical:demo-pdfjs-pages-v1')))",
            [document_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

pub(crate) fn select_document_text_projection(
    transaction: &Transaction<'_>,
    document_id: &str,
    extraction_id: &str,
    projection_profile_id: &str,
    provider: &str,
    content: &str,
    selected_content_hash: &str,
) -> AppResult<bool> {
    let active = transaction
        .query_row(
            "SELECT id, source_id, library_id, zotero_attachment_key,
                    source_version, content_hash, provider, extraction_id
             FROM document_texts
             WHERE document_id = ?1 AND is_active = 1",
            [document_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .optional()?;
    if let Some((text_id, _, _, _, _, active_hash, active_provider, active_extraction)) = &active {
        let has_page_provenance = |provider: &str| {
            provider.starts_with("canonical:docling-local-")
                || matches!(
                    provider,
                    "canonical:pdfjs-pages-v1" | "canonical:demo-pdfjs-pages-v1"
                )
        };
        let page_provenance_changes = active_extraction.as_deref() != Some(extraction_id)
            && (has_page_provenance(provider) || has_page_provenance(active_provider));
        if active_hash == selected_content_hash && !page_provenance_changes {
            transaction.execute(
                "UPDATE document_texts SET
                    extraction_id = ?2,
                    projection_profile_id = ?3,
                    content_hash = ?4
                 WHERE id = ?1",
                params![
                    text_id,
                    extraction_id,
                    projection_profile_id,
                    selected_content_hash,
                ],
            )?;
            return Ok(false);
        }
    }
    let (source_id, library_id, attachment_key): (String, String, String) = transaction.query_row(
        "SELECT l.source_id, d.library_id, d.zotero_attachment_key
             FROM documents d JOIN libraries l ON l.id=d.library_id WHERE d.id=?1",
        [document_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let source_version: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(source_version),0)+1 FROM document_revisions WHERE document_id=?1",
        [document_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO document_revisions (document_id, source_version, revision_key,
         discovered_at, revision_kind, content_hash)
         VALUES (?1,?2,?3,?4,'derived-text',?5)",
        params![
            document_id,
            source_version,
            format!("text:canonical:{source_version}:{selected_content_hash}"),
            Utc::now().to_rfc3339(),
            selected_content_hash
        ],
    )?;
    invalidate_document_citations(transaction, document_id, Some(selected_content_hash))?;
    transaction.execute(
        "UPDATE document_texts SET is_active = 0
         WHERE document_id = ?1 AND is_active = 1",
        [document_id],
    )?;
    let text_id = stable_id("text", &[document_id, provider]);
    transaction.execute(
        "INSERT INTO document_texts (
            id, document_id, source_id, library_id,
            zotero_attachment_key, provider, source_version,
            content, completeness, fetched_at, extraction_id,
            projection_profile_id, content_hash, is_active
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            'derived', ?9, ?10, ?11, ?12, 1
         )
         ON CONFLICT(document_id, provider) DO UPDATE SET
            source_version = excluded.source_version,
            content = excluded.content,
            completeness = excluded.completeness,
            fetched_at = excluded.fetched_at,
            extraction_id = excluded.extraction_id,
            projection_profile_id = excluded.projection_profile_id,
            content_hash = excluded.content_hash,
            is_active = 1",
        params![
            text_id,
            document_id,
            source_id,
            library_id,
            attachment_key,
            provider,
            source_version,
            content,
            Utc::now().to_rfc3339(),
            extraction_id,
            projection_profile_id,
            selected_content_hash,
        ],
    )?;
    if let Some(item_id) = transaction
        .query_row(
            "SELECT research_item_id FROM documents WHERE id = ?1",
            [document_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
    {
        refresh_search_content(transaction, &item_id)?;
    }
    Ok(true)
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    let mut output = String::with_capacity(bytes.as_ref().len() * 2);
    for byte in bytes.as_ref() {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn normalize_facet(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn citation_search_filter(value: Option<&str>) -> &'static str {
    match value {
        Some("references-indexed") => "references-indexed",
        Some("cited-by-local") => "cited-by-local",
        Some("has-external") => "has-external",
        Some("needs-review") => "needs-review",
        _ => "all",
    }
}

fn citation_search_label(filter: &str) -> &'static str {
    match filter {
        "references-indexed" => "references indexed locally",
        "cited-by-local" => "cited by the local corpus",
        "has-external" => "contains external references",
        "needs-review" => "contains unresolved or ambiguous references",
        _ => "all papers",
    }
}

fn citation_search_condition(filter: &str, item_expression: &str) -> String {
    match filter {
        "references-indexed" => format!(
            " AND EXISTS (
                SELECT 1 FROM bibliographic_references br
                JOIN reference_extractions be ON be.id = br.extraction_id
                WHERE br.source_item_id = {item_expression}
                  AND br.is_current = 1 AND be.is_current = 1
            )"
        ),
        "cited-by-local" => format!(
            " AND EXISTS (
                SELECT 1 FROM citation_edges ce
                WHERE ce.target_item_id = {item_expression}
            )"
        ),
        "has-external" => format!(
            " AND EXISTS (
                SELECT 1 FROM bibliographic_references br
                JOIN reference_extractions be ON be.id = br.extraction_id
                JOIN reference_resolutions rr ON rr.reference_id = br.id
                WHERE br.source_item_id = {item_expression}
                  AND br.is_current = 1 AND be.is_current = 1
                  AND rr.status = 'external'
            )"
        ),
        "needs-review" => format!(
            " AND EXISTS (
                SELECT 1 FROM bibliographic_references br
                JOIN reference_extractions be ON be.id = br.extraction_id
                JOIN reference_resolutions rr ON rr.reference_id = br.id
                WHERE br.source_item_id = {item_expression}
                  AND br.is_current = 1 AND be.is_current = 1
                  AND rr.status IN ('ambiguous', 'probable', 'unresolved')
            )"
        ),
        _ => String::new(),
    }
}

pub(crate) fn lexical_query(value: &str) -> Option<String> {
    let terms = value
        .split_whitespace()
        .map(|term| term.trim_matches(|character: char| !character.is_alphanumeric()))
        .filter(|term| !term.is_empty())
        .take(12)
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    (!terms.is_empty()).then(|| terms.join(" AND "))
}

fn text_completeness(
    indexed_pages: Option<u32>,
    total_pages: Option<u32>,
    indexed_chars: Option<u64>,
    total_chars: Option<u64>,
) -> &'static str {
    let page_complete = indexed_pages
        .zip(total_pages)
        .is_some_and(|(indexed, total)| total > 0 && indexed >= total);
    let character_complete = indexed_chars
        .zip(total_chars)
        .is_some_and(|(indexed, total)| total > 0 && indexed >= total);
    if page_complete || character_complete {
        "full"
    } else {
        "partial"
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::params;

    use crate::model::{
        FullTextRecord, FullTextSyncDelta, GraphProjectionRequest, LibraryDelta, RawObject,
        RemoteLibrary, SearchRequest, SourceProbe,
    };

    use super::{stable_id, ResearchDatabase};

    fn temporary_database(name: &str) -> (ResearchDatabase, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "graphite-{name}-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let database = ResearchDatabase::initialize(path.clone()).expect("database");
        (database, path)
    }

    fn fixture_item(key: &str, title: &str, version: i64) -> RawObject {
        RawObject {
            key: key.to_owned(),
            version,
            object_type: "items".to_owned(),
            json: serde_json::json!({
                "key": key,
                "version": version,
                "data": {
                    "key": key,
                    "version": version,
                    "itemType": "journalArticle",
                    "title": title,
                    "date": "2022",
                    "creators": [{
                        "creatorType": "author",
                        "firstName": "Ada",
                        "lastName": "Lovelace"
                    }],
                    "tags": [{"tag": "ambient noise"}],
                    "collections": []
                }
            }),
        }
    }

    fn fixture_attachment(key: &str, parent_key: &str, version: i64) -> RawObject {
        RawObject {
            key: key.to_owned(),
            version,
            object_type: "items".to_owned(),
            json: serde_json::json!({
                "key": key,
                "version": version,
                "data": {
                    "key": key,
                    "version": version,
                    "itemType": "attachment",
                    "parentItem": parent_key,
                    "title": "Full text PDF",
                    "contentType": "application/pdf",
                    "filename": "paper.pdf",
                    "linkMode": "imported_file"
                }
            }),
        }
    }

    fn fixture_collection(key: &str, name: &str) -> RawObject {
        RawObject {
            key: key.to_owned(),
            version: 1,
            object_type: "collections".to_owned(),
            json: serde_json::json!({
                "key": key,
                "version": 1,
                "data": {
                    "key": key,
                    "version": 1,
                    "name": name
                }
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fixture_projection_item(
        key: &str,
        title: &str,
        year: Option<i32>,
        author: Option<&str>,
        tags: &[&str],
        collections: &[&str],
        related_targets: &[&str],
    ) -> RawObject {
        let creators = author
            .into_iter()
            .map(|last_name| {
                serde_json::json!({
                    "creatorType": "author",
                    "firstName": "A.",
                    "lastName": last_name
                })
            })
            .collect::<Vec<_>>();
        let tags = tags
            .iter()
            .map(|tag| serde_json::json!({"tag": tag}))
            .collect::<Vec<_>>();
        RawObject {
            key: key.to_owned(),
            version: 1,
            object_type: "items".to_owned(),
            json: serde_json::json!({
                "key": key,
                "version": 1,
                "data": {
                    "key": key,
                    "version": 1,
                    "itemType": "journalArticle",
                    "title": title,
                    "date": year.map(|value| value.to_string()).unwrap_or_default(),
                    "publicationTitle": "Journal of Projection Tests",
                    "creators": creators,
                    "tags": tags,
                    "collections": collections,
                    "relations": {"dc:relation": related_targets}
                }
            }),
        }
    }

    #[test]
    fn empty_workspace_graph_and_analytics_are_stable() {
        let (database, path) = temporary_database("empty-read-model");

        let virgin_workspace = database
            .load_workspace()
            .expect("workspace without an active library");
        assert!(virgin_workspace.snapshot.items.is_empty());
        assert!(virgin_workspace.snapshot.libraries.is_empty());
        assert_eq!(virgin_workspace.snapshot.active_library_id, None);
        assert_eq!(virgin_workspace.snapshot.total_item_count, 0);
        assert!(virgin_workspace.graph.nodes.is_empty());
        assert!(virgin_workspace.graph.edges.is_empty());
        assert_eq!(virgin_workspace.graph.total_node_count, 0);
        let no_active_projection = database
            .graph_projection(&GraphProjectionRequest {
                collection_id: None,
                seed_item_ids: Vec::new(),
                limit: None,
                offset: None,
            })
            .expect("graph projection without an active library");
        assert!(no_active_projection.items.is_empty());
        assert!(no_active_projection.topics.is_empty());
        assert!(no_active_projection.graph.nodes.is_empty());

        let source = database
            .register_source(&SourceProbe {
                server_id: "empty-projection-server".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("source");
        let library = database
            .register_libraries(
                &source.id,
                &[RemoteLibrary {
                    kind: "user".to_owned(),
                    zotero_id: "0".to_owned(),
                    name: "Empty library".to_owned(),
                }],
            )
            .expect("library")
            .remove(0);
        let workspace = database.load_workspace().expect("empty workspace");
        assert!(workspace.snapshot.items.is_empty());
        assert_eq!(workspace.snapshot.total_item_count, 0);
        assert_eq!(workspace.snapshot.loaded_item_count, 0);
        assert_eq!(workspace.snapshot.active_library_id, Some(library.id));
        assert_eq!(workspace.snapshot.topics.len(), 1);
        assert_eq!(workspace.snapshot.topics[0].id, "uncategorized");
        assert!(workspace.graph.nodes.is_empty());
        assert!(workspace.graph.edges.is_empty());
        assert!(workspace.graph.available_relations.is_empty());
        assert_eq!(workspace.graph.total_node_count, 0);
        assert!(!workspace.graph.has_more);
        assert_eq!(workspace.analytics.by_topic.len(), 1);
        assert_eq!(workspace.analytics.by_topic[0].topic_id, "uncategorized");
        assert_eq!(workspace.analytics.by_topic[0].count, 0);
        assert!(workspace.analytics.by_year.is_empty());
        assert!(workspace.analytics.most_connected.is_empty());
        assert_eq!(workspace.analytics.pdf_coverage, 0.0);
        assert_eq!(workspace.analytics.document_count, 0);
        assert_eq!(workspace.analytics.indexed_pages, 0);
        assert!(workspace.analytics.citation.most_cited_local.is_empty());
        assert_eq!(workspace.analytics.citation.resolution_coverage, 0.0);

        let projection = database
            .graph_projection(&GraphProjectionRequest {
                collection_id: None,
                seed_item_ids: Vec::new(),
                limit: None,
                offset: None,
            })
            .expect("empty graph projection");
        assert!(projection.items.is_empty());
        assert_eq!(projection.topics.len(), 1);
        assert_eq!(projection.topics[0].id, "uncategorized");
        assert!(projection.graph.nodes.is_empty());
        assert_eq!(projection.graph.total_node_count, 0);
        assert!(!projection.graph.has_more);

        fs::remove_file(path).expect("remove temporary database");
    }

    #[test]
    fn graph_and_analytics_preserve_scoping_edges_topics_and_counts() {
        let (database, path) = temporary_database("graph-analytics-read-model");
        let source = database
            .register_source(&SourceProbe {
                server_id: "projection-server".to_owned(),
                api_version: "3".to_owned(),
                schema_version: Some("37".to_owned()),
            })
            .expect("source");
        let libraries = database
            .register_libraries(
                &source.id,
                &[
                    RemoteLibrary {
                        kind: "user".to_owned(),
                        zotero_id: "0".to_owned(),
                        name: "Personal library".to_owned(),
                    },
                    RemoteLibrary {
                        kind: "group".to_owned(),
                        zotero_id: "42".to_owned(),
                        name: "Group library".to_owned(),
                    },
                ],
            )
            .expect("libraries");
        let personal = libraries
            .iter()
            .find(|library| library.remote.kind == "user")
            .expect("personal library");
        let group = libraries
            .iter()
            .find(|library| library.remote.kind == "group")
            .expect("group library");
        let relation_target = "https://api.zotero.org/users/0/items/GAMMA";
        database
            .apply_delta(
                &source.id,
                &personal.id,
                &LibraryDelta {
                    target_version: 1,
                    objects: vec![
                        fixture_collection("FOCUS", "Focus collection"),
                        fixture_projection_item(
                            "ALPHA",
                            "Alpha paper",
                            Some(2020),
                            Some("Lovelace"),
                            &["Wave", "Shared"],
                            &["FOCUS"],
                            &[relation_target],
                        ),
                        fixture_projection_item(
                            "BETA",
                            "Beta paper",
                            Some(2021),
                            Some("Lovelace"),
                            &["Wave"],
                            &["FOCUS"],
                            &[],
                        ),
                        fixture_projection_item(
                            "GAMMA",
                            "Gamma paper",
                            Some(2021),
                            Some("Hopper"),
                            &["Shared"],
                            &[],
                            &[],
                        ),
                        fixture_projection_item(
                            "ORPHAN",
                            "Orphan paper",
                            None,
                            None,
                            &[],
                            &[],
                            &[],
                        ),
                        fixture_attachment("PDFALPHA", "ALPHA", 1),
                    ],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("personal metadata");
        database
            .apply_delta(
                &source.id,
                &group.id,
                &LibraryDelta {
                    target_version: 1,
                    objects: vec![fixture_projection_item(
                        "FOREIGN",
                        "Foreign group paper",
                        Some(1999),
                        Some("Foreign"),
                        &["Wave"],
                        &[],
                        &[],
                    )],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("group metadata");
        database
            .apply_fulltext_delta(
                &source.id,
                &personal.id,
                &FullTextSyncDelta {
                    successful_version: 2,
                    records: vec![FullTextRecord {
                        attachment_key: "PDFALPHA".to_owned(),
                        version: 2,
                        content: "Complete local projection test text".to_owned(),
                        indexed_pages: Some(12),
                        total_pages: Some(12),
                        indexed_chars: Some(35),
                        total_chars: Some(35),
                    }],
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("full text");

        let alpha = stable_id("item", &[&source.id, &personal.id, "ALPHA"]);
        let beta = stable_id("item", &[&source.id, &personal.id, "BETA"]);
        let gamma = stable_id("item", &[&source.id, &personal.id, "GAMMA"]);
        let orphan = stable_id("item", &[&source.id, &personal.id, "ORPHAN"]);
        let foreign = stable_id("item", &[&source.id, &group.id, "FOREIGN"]);
        let citation_edge = stable_id("citation-edge", &[&beta, &gamma]);
        database
            .connect()
            .expect("connection")
            .execute(
                "INSERT INTO citation_edges (
                    id, source_item_id, target_item_id, evidence_count,
                    strongest_category, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 2, 'exact identifier', ?4, ?4)",
                params![citation_edge, beta, gamma, "2026-08-25T00:00:00Z"],
            )
            .expect("citation edge");

        let workspace = database.load_workspace().expect("workspace");
        assert_eq!(
            workspace.snapshot.active_library_id,
            Some(personal.id.clone())
        );
        assert_eq!(workspace.snapshot.total_item_count, 4);
        assert_eq!(workspace.snapshot.loaded_item_count, 4);
        assert_eq!(
            workspace
                .snapshot
                .items
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Alpha paper", "Beta paper", "Gamma paper", "Orphan paper"]
        );
        assert!(!workspace
            .snapshot
            .items
            .iter()
            .any(|item| item.id == foreign));
        assert_eq!(workspace.snapshot.libraries.len(), 2);
        assert_eq!(workspace.snapshot.libraries[0].kind, "user");
        assert_eq!(workspace.snapshot.libraries[0].item_count, 4);
        assert_eq!(workspace.snapshot.libraries[1].kind, "group");
        assert_eq!(workspace.snapshot.libraries[1].item_count, 1);
        assert_eq!(
            workspace
                .snapshot
                .topics
                .iter()
                .map(|topic| topic.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Not analyzed / no clear topic"]
        );
        assert_eq!(workspace.graph.nodes.len(), 4);
        assert_eq!(workspace.graph.total_node_count, 4);
        assert!(!workspace.graph.has_more);
        assert_eq!(
            workspace.graph.available_relations,
            vec![
                "citation",
                "shared-collection",
                "shared-creator",
                "shared-tag",
                "zotero-related"
            ]
        );
        assert_eq!(workspace.graph.edges.len(), 6);
        let citation = workspace
            .graph
            .edges
            .iter()
            .find(|edge| edge.id == citation_edge)
            .expect("directed citation");
        assert_eq!(citation.source, beta);
        assert_eq!(citation.target, gamma);
        assert_eq!(citation.relation, "citation");
        assert!(citation.directed);
        assert_eq!(citation.evidence_count, 2);
        assert_eq!(citation.weight, 1.06);
        assert_eq!(
            citation.evidence_label.as_deref(),
            Some("2 bibliography evidence / exact identifier")
        );
        assert!(
            workspace
                .graph
                .edges
                .iter()
                .filter(|edge| edge.relation == "zotero-related")
                .count()
                == 1
        );
        assert!(workspace.graph.edges.iter().all(|edge| {
            edge.relation == "citation"
                || (!edge.directed
                    && edge.source < edge.target
                    && edge.id == stable_id("edge", &[&edge.relation, &edge.source, &edge.target]))
        }));
        assert!(!workspace
            .graph
            .edges
            .iter()
            .any(|edge| edge.source == orphan || edge.target == orphan));

        assert_eq!(
            workspace
                .analytics
                .by_year
                .iter()
                .map(|metric| (metric.year, metric.count))
                .collect::<Vec<_>>(),
            vec![(2020, 1), (2021, 2)]
        );
        assert_eq!(
            workspace
                .analytics
                .by_topic
                .iter()
                .map(|metric| (metric.topic_id.as_str(), metric.count))
                .collect::<Vec<_>>(),
            vec![("uncategorized", 4)]
        );
        assert_eq!(
            workspace
                .analytics
                .most_connected
                .iter()
                .map(|item| (item.id.as_str(), item.citation_count))
                .collect::<Vec<_>>(),
            vec![
                (alpha.as_str(), 5),
                (beta.as_str(), 4),
                (gamma.as_str(), 3),
                (orphan.as_str(), 0),
            ]
        );
        assert_eq!(workspace.analytics.pdf_coverage, 0.25);
        assert_eq!(workspace.analytics.document_count, 1);
        assert_eq!(workspace.analytics.resolved_pdf_count, 0);
        assert_eq!(workspace.analytics.full_text_count, 1);
        assert_eq!(workspace.analytics.partial_text_count, 0);
        assert_eq!(workspace.analytics.indexed_pages, 12);
        assert_eq!(workspace.citations.citation_edges, 1);
        assert_eq!(workspace.citations.references_extracted, 0);
        assert_eq!(workspace.analytics.citation.most_cited_local.len(), 1);
        assert_eq!(
            workspace.analytics.citation.most_cited_local[0].item_id,
            gamma
        );
        assert_eq!(workspace.analytics.citation.most_cited_local[0].count, 1);
        assert_eq!(
            workspace.analytics.citation.most_referencing[0].item_id,
            beta
        );
        assert_eq!(workspace.analytics.citation.resolution_coverage, 0.0);
        assert!(workspace
            .analytics
            .citation
            .frequently_referenced_external
            .is_empty());

        let repeated = database.load_workspace().expect("repeated workspace");
        assert_eq!(
            serde_json::to_value(&workspace.graph).expect("serialize graph"),
            serde_json::to_value(&repeated.graph).expect("serialize repeated graph")
        );
        assert_eq!(
            serde_json::to_value(&workspace.analytics).expect("serialize analytics"),
            serde_json::to_value(&repeated.analytics).expect("serialize repeated analytics")
        );

        let focus_collection = stable_id("collection", &[&source.id, &personal.id, "FOCUS"]);
        let first_page = database
            .graph_projection(&GraphProjectionRequest {
                collection_id: Some(focus_collection.clone()),
                seed_item_ids: Vec::new(),
                limit: Some(1),
                offset: Some(0),
            })
            .expect("first collection page");
        assert_eq!(first_page.items.len(), 1);
        assert_eq!(first_page.items[0].id, alpha);
        assert_eq!(first_page.graph.total_node_count, 2);
        assert!(first_page.graph.has_more);
        let second_page = database
            .graph_projection(&GraphProjectionRequest {
                collection_id: Some(focus_collection.clone()),
                seed_item_ids: Vec::new(),
                limit: Some(1),
                offset: Some(1),
            })
            .expect("second collection page");
        assert_eq!(second_page.items.len(), 1);
        assert_eq!(second_page.items[0].id, beta);
        assert_eq!(second_page.graph.total_node_count, 2);
        assert!(!second_page.graph.has_more);

        let seeded = database
            .graph_projection(&GraphProjectionRequest {
                collection_id: Some(focus_collection),
                seed_item_ids: vec![orphan.clone(), orphan.clone(), "missing".to_owned()],
                limit: Some(2),
                offset: Some(0),
            })
            .expect("seeded collection projection");
        assert_eq!(seeded.graph.total_node_count, 2);
        assert!(!seeded.graph.has_more);
        assert_eq!(
            seeded
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([alpha.as_str(), orphan.as_str()])
        );

        let explicit_foreign = database
            .projection_for_item_ids(&[foreign.clone(), foreign.clone(), "missing".to_owned()])
            .expect("explicit cross-library projection");
        assert_eq!(explicit_foreign.items.len(), 1);
        assert_eq!(explicit_foreign.items[0].id, foreign);

        fs::remove_file(path).expect("remove temporary database");
    }

    #[test]
    fn saved_lenses_survive_restart_and_protect_membership() {
        use super::lenses::SaveLensRequest;
        if let Some(path) = std::env::var_os("GRAPHITE_LENS_RESTART_TEST_DB") {
            let database = ResearchDatabase::initialize(path.into()).expect("fresh process");
            let workspace = database.load_workspace().expect("restored workspace");
            if let Ok(deleted) = std::env::var("GRAPHITE_LENS_RESTART_TEST_DELETED_ID") {
                assert_eq!(workspace.snapshot.lenses.len(), 1);
                assert_eq!(workspace.snapshot.lenses[0].name, "Recovery");
                assert_eq!(workspace.snapshot.lenses[0].paper_ids.len(), 1);
                assert_eq!(workspace.snapshot.items.len(), 4);
                let members: i64 = database
                    .connect()
                    .unwrap()
                    .query_row(
                        "SELECT COUNT(*) FROM research_lens_members WHERE lens_id = ?1",
                        [deleted],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(members, 0);
                return;
            }
            let lens = &workspace.snapshot.lenses[0];
            assert_eq!(lens.name, "Network transfer");
            assert_eq!(lens.description, "Which methods generalize?");
            assert_eq!(lens.paper_ids.len(), 5);
            assert!(lens.unavailable_paper_ids.is_empty());
            return;
        }
        let directory = tempfile::tempdir().expect("temporary app data");
        let path = directory.path().join("research.sqlite");
        let database = ResearchDatabase::initialize(path.clone()).expect("database");
        let source = database
            .register_source(&SourceProbe {
                server_id: "lens-test".into(),
                api_version: "3".into(),
                schema_version: None,
            })
            .expect("source");
        let libraries = database
            .register_libraries(
                &source.id,
                &[
                    RemoteLibrary {
                        kind: "user".into(),
                        zotero_id: "0".into(),
                        name: "One".into(),
                    },
                    RemoteLibrary {
                        kind: "group".into(),
                        zotero_id: "2".into(),
                        name: "Two".into(),
                    },
                ],
            )
            .expect("libraries");
        let library = &libraries[0];
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects: (0..5)
                        .map(|i| fixture_item(&format!("P{i}"), &format!("Paper {i}"), 1))
                        .collect(),
                    deleted_items: vec![],
                    deleted_collections: vec![],
                },
            )
            .expect("papers");
        database
            .set_active_library(&library.id)
            .expect("active library");
        let ids: Vec<_> = database
            .load_workspace()
            .unwrap()
            .snapshot
            .items
            .iter()
            .map(|p| p.id.clone())
            .collect();
        let mut request = SaveLensRequest {
            id: None,
            revision: None,
            library_id: library.id.clone(),
            name: "  Network transfer  ".into(),
            description: "Which methods generalize?".into(),
            paper_ids: ids.clone(),
        };
        let saved = database.save_lens(&request).expect("save five papers");
        assert_eq!(saved.name, "Network transfer");
        drop(database);
        let restart = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "database::tests::saved_lenses_survive_restart_and_protect_membership",
                "--nocapture",
            ])
            .env("GRAPHITE_LENS_RESTART_TEST_DB", &path)
            .output()
            .expect("restart in a fresh process");
        assert!(
            restart.status.success(),
            "Fresh-process Lens check failed: {} {}",
            String::from_utf8_lossy(&restart.stdout),
            String::from_utf8_lossy(&restart.stderr)
        );
        let reopened = ResearchDatabase::initialize(path.clone()).expect("restart database");
        let restored = reopened.load_workspace().unwrap().snapshot.lenses.remove(0);
        assert_eq!(restored.id, saved.id);
        assert_eq!(restored.paper_ids, ids);
        assert_eq!(restored.description, request.description);
        request.id = Some(saved.id.clone());
        request.revision = Some(saved.revision);
        request.name = "Renamed investigation".into();
        request.paper_ids.pop();
        let edited = reopened.save_lens(&request).expect("edit membership");
        assert_eq!(edited.paper_ids.len(), 4);
        assert!(reopened.save_lens(&request).is_err());
        assert!(reopened
            .delete_lens(&library.id, &saved.id, saved.revision)
            .is_err());
        request.revision = Some(edited.revision);
        request.paper_ids.push("foreign-paper".into());
        assert!(reopened.save_lens(&request).is_err());
        assert_eq!(
            reopened.list_lenses(&library.id).unwrap()[0].revision,
            edited.revision
        );
        request.paper_ids.pop();
        request.paper_ids.push(ids[4].clone());
        let edited = reopened.save_lens(&request).expect("add member");
        reopened.set_active_library(&libraries[1].id).unwrap();
        assert!(reopened.list_lenses(&libraries[1].id).unwrap().is_empty());
        assert!(reopened.save_lens(&request).is_err());
        assert!(reopened
            .delete_lens(&libraries[1].id, &saved.id, edited.revision)
            .is_err());
        reopened.set_active_library(&library.id).unwrap();
        reopened
            .connect()
            .unwrap()
            .execute("DELETE FROM research_items WHERE id = ?1", [&ids[0]])
            .unwrap();
        let missing = reopened.list_lenses(&library.id).unwrap().remove(0);
        assert_eq!(missing.paper_ids.len(), 5);
        assert_eq!(missing.unavailable_paper_ids, vec![ids[0].clone()]);
        request.revision = Some(missing.revision);
        request.description = "Retain missing sources".into();
        let retained = reopened
            .save_lens(&request)
            .expect("retain unavailable paper");
        assert_eq!(
            retained.unavailable_paper_ids,
            missing.unavailable_paper_ids
        );
        request.revision = Some(retained.revision);
        request.paper_ids.clear();
        let empty = reopened.save_lens(&request).expect("empty saved set");
        assert!(empty.paper_ids.is_empty());
        reopened
            .delete_lens(&library.id, &empty.id, empty.revision)
            .expect("delete");
        assert!(reopened.list_lenses(&library.id).unwrap().is_empty());
        request.id = None;
        request.revision = None;
        request.name = " ".into();
        assert!(reopened.save_lens(&request).is_err());
        request.name = "x".repeat(121);
        assert!(reopened.save_lens(&request).is_err());
        request.name = "Recovery".into();
        request.description = "x".repeat(4001);
        assert!(reopened.save_lens(&request).is_err());
        request.description.clear();
        request.paper_ids = vec![ids[1].clone(); 1201];
        assert!(reopened.save_lens(&request).is_err());
        request.paper_ids = vec![ids[1].clone(); 2];
        let recovery = reopened.save_lens(&request).unwrap();
        assert_eq!(recovery.paper_ids, [ids[1].clone()]);
        request.name = "Delete populated Lens".into();
        request.paper_ids = ids[1..].to_vec();
        let disposable = reopened.save_lens(&request).unwrap();
        reopened
            .delete_lens(&library.id, &disposable.id, disposable.revision)
            .expect("delete populated Lens");
        let restart = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "database::tests::saved_lenses_survive_restart_and_protect_membership",
                "--nocapture",
            ])
            .env("GRAPHITE_LENS_RESTART_TEST_DB", &path)
            .env("GRAPHITE_LENS_RESTART_TEST_DELETED_ID", &disposable.id)
            .output()
            .expect("restart after deleting populated Lens");
        assert!(
            restart.status.success(),
            "Deletion restart check failed: {} {}",
            String::from_utf8_lossy(&restart.stdout),
            String::from_utf8_lossy(&restart.stderr)
        );
        reopened
            .connect()
            .unwrap()
            .execute("DELETE FROM sources WHERE id = ?1", [&source.id])
            .unwrap();
        assert_eq!(
            super::lenses::load_lenses(&reopened.connect().unwrap(), &library.id).unwrap()[0].id,
            recovery.id
        );
    }

    #[test]
    fn migrations_are_idempotent_and_persistence_survives_reopen() {
        let (database, path) = temporary_database("migration");
        let probe = SourceProbe {
            server_id: "server-a".to_owned(),
            api_version: "3".to_owned(),
            schema_version: Some("37".to_owned()),
        };
        let source = database.register_source(&probe).expect("source");
        let remote = RemoteLibrary {
            kind: "user".to_owned(),
            zotero_id: "0".to_owned(),
            name: "My Library".to_owned(),
        };
        let library = database
            .register_libraries(&source.id, std::slice::from_ref(&remote))
            .expect("library")
            .remove(0);
        let delta = LibraryDelta {
            target_version: 4,
            objects: vec![
                fixture_item("ITEMA", "Persistent paper", 4),
                fixture_attachment("PDFA", "ITEMA", 4),
            ],
            deleted_items: Vec::new(),
            deleted_collections: Vec::new(),
        };
        database
            .apply_delta(&source.id, &library.id, &delta)
            .expect("delta");

        let reopened = ResearchDatabase::initialize(path.clone()).expect("reopen");
        let workspace = reopened.load_workspace().expect("workspace");
        assert_eq!(workspace.snapshot.items.len(), 1);
        assert_eq!(workspace.snapshot.items[0].title, "Persistent paper");
        let connection = reopened.connect().expect("migration connection");
        let item_binding_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM knowledge_item_source_bindings",
                [],
                |row| row.get(0),
            )
            .expect("item binding count");
        let artifact_binding_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM document_artifact_source_bindings",
                [],
                |row| row.get(0),
            )
            .expect("artifact binding count");
        assert_eq!(item_binding_count, 1);
        assert_eq!(artifact_binding_count, 1);
        let active_quality_profile: String = connection
            .query_row(
                "SELECT id FROM extraction_quality_profiles WHERE active = 1",
                [],
                |row| row.get(0),
            )
            .expect("active extraction quality profile");
        let migration_thirteen_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 13",
                [],
                |row| row.get(0),
            )
            .expect("migration thirteen count");
        let migration_fourteen_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = 14",
                [],
                |row| row.get(0),
            )
            .expect("migration fourteen count");
        let shadow_state: (bool, String, String) = connection
            .query_row(
                "SELECT enabled, availability, routing_profile_id
                 FROM shadow_extraction_pilot_state WHERE singleton_id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("shadow pilot state");
        assert_eq!(active_quality_profile, "extraction-quality-v1.1");
        assert_eq!(migration_thirteen_count, 1);
        assert_eq!(migration_fourteen_count, 1);
        assert_eq!(
            shadow_state,
            (
                false,
                "disabled".to_owned(),
                "pdf-shadow-strategy-v1".to_owned()
            )
        );
        let provider_kind = reopened
            .connect()
            .expect("connection")
            .query_row(
                "SELECT provider_kind FROM agent_settings WHERE id = 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("agent provider setting");
        assert_eq!(provider_kind, "graphite-managed");
        fs::remove_file(path).expect("remove temporary database");
    }

    #[test]
    fn migration_eleven_upgrades_multiple_legacy_text_providers_without_identity_changes() {
        let path = std::env::temp_dir().join(format!(
            "graphite-schema-nine-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let connection = rusqlite::Connection::open(&path).expect("schema-nine connection");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                 );",
            )
            .expect("migration table");
        for (version, sql) in [
            (1_i64, super::MIGRATION_1),
            (2, super::MIGRATION_2),
            (3, super::MIGRATION_3),
            (4, super::MIGRATION_4),
            (5, super::MIGRATION_5),
            (6, super::MIGRATION_6),
            (7, super::MIGRATION_7),
            (8, super::MIGRATION_8),
            (9, super::MIGRATION_9),
        ] {
            connection
                .execute_batch(sql)
                .expect("schema-nine migration");
            connection
                .execute(
                    "INSERT INTO schema_migrations(version, name, applied_at)
                     VALUES (?1, 'fixture', CURRENT_TIMESTAMP)",
                    [version],
                )
                .expect("record schema-nine migration");
        }
        connection
            .execute_batch(
                "INSERT INTO sources (
                    id, server_id, api_version, compatibility,
                    first_seen_at, last_seen_at, connection_status
                 ) VALUES (
                    'source-keep', 'server-keep', '3', 'modern',
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, 'online'
                 );
                 INSERT INTO libraries (
                    id, source_id, kind, zotero_library_id, name,
                    successful_version, available
                 ) VALUES (
                    'library-keep', 'source-keep', 'user', '0',
                    'Library', 1, 1
                 );
                 INSERT INTO research_items (
                    id, source_id, library_id, zotero_key, item_type,
                    title, source_version
                 ) VALUES (
                    'knowledge-keep', 'source-keep', 'library-keep',
                    'ITEM1', 'journalArticle', 'Persistent item', 1
                 );
                 INSERT INTO attachments (
                    id, source_id, library_id, zotero_key,
                    parent_item_id, title, content_type,
                    available_locally, source_version
                 ) VALUES (
                    'attachment-keep', 'source-keep', 'library-keep',
                    'PDF1', 'knowledge-keep', 'PDF', 'application/pdf',
                    1, 1
                 );
                 INSERT INTO documents (
                    id, source_id, library_id, attachment_id,
                    research_item_id, zotero_attachment_key, media_type,
                    title, source_version, availability, created_at,
                    updated_at
                 ) VALUES (
                    'artifact-keep', 'source-keep', 'library-keep',
                    'attachment-keep', 'knowledge-keep', 'PDF1',
                    'application/pdf', 'PDF', 1, 'metadata-only',
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                 );
                 INSERT INTO document_revisions (
                    document_id, source_version, revision_key, discovered_at
                 ) VALUES (
                    'artifact-keep', 1, 'text:zotero-indexed:1:fixture',
                    CURRENT_TIMESTAMP
                 );
                 INSERT INTO document_texts (
                    id, document_id, source_id, library_id,
                    zotero_attachment_key, provider, source_version,
                    content, completeness, fetched_at
                 ) VALUES (
                    'text-keep', 'artifact-keep', 'source-keep',
                    'library-keep', 'PDF1', 'zotero-indexed', 1,
                    'persistent searchable body', 'full',
                    CURRENT_TIMESTAMP
                 );
                 INSERT INTO document_texts (
                    id, document_id, source_id, library_id,
                    zotero_attachment_key, provider, source_version,
                    content, completeness, fetched_at
                 ) VALUES (
                    'text-legacy-alternative', 'artifact-keep',
                    'source-keep', 'library-keep', 'PDF1',
                    'legacy-alternative', 1, 'older alternate body',
                    'partial', CURRENT_TIMESTAMP
                 );
                 INSERT INTO search_content (
                    item_id, library_id, title, creators, tags,
                    abstract_text, venue, body
                 ) VALUES (
                    'knowledge-keep', 'library-keep', 'Persistent item',
                    '', '', '', '', 'persistent searchable body'
                 );
                 INSERT INTO reader_states (
                    document_id, page_number, zoom, fit_mode,
                    outline_open, context_open, updated_at
                 ) VALUES (
                    'artifact-keep', 4, 1.25, 'width', 1, 1,
                    CURRENT_TIMESTAMP
                 );
                 INSERT INTO embedding_profiles (
                    id, provider, model_identifier, model_revision,
                    artifact_identity, embedding_dimension,
                    similarity_metric, query_template, document_template,
                    normalization, runtime_version, vector_backend,
                    vector_index_format_version, active, created_at
                 ) VALUES (
                    'embedding-keep', 'fixture', 'fixture', '1', '1',
                    384, 'cosine', '{query}', '{document}', 'l2', '1',
                    'fixture', '1', 1, CURRENT_TIMESTAMP
                 );
                 INSERT INTO semantic_vector_entries (
                    vector_key, embedding_profile_id, library_id,
                    index_kind, target_id, active, created_at, updated_at
                 ) VALUES (
                    120, 'embedding-keep', 'library-keep', 'chunk',
                    'chunk-keep', 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                 );
                 INSERT INTO semantic_chunks (
                    id, item_id, library_id, document_id,
                    document_revision_id, embedding_profile_id,
                    chunking_profile_id, source_provider,
                    character_start, character_end, ordinal, token_count,
                    text, text_fingerprint, vector_key, status,
                    created_at, updated_at
                 ) VALUES (
                    'chunk-keep', 'knowledge-keep', 'library-keep',
                    'artifact-keep', 1, 'embedding-keep',
                    'scientific-paragraph-v1', 'zotero-indexed',
                    0, 26, 120, 4, 'persistent searchable body',
                    'chunk-hash', 120, 'active', CURRENT_TIMESTAMP,
                    CURRENT_TIMESTAMP
                 );
                 INSERT INTO agent_conversations (
                    id, library_id, title, created_at, updated_at
                 ) VALUES (
                    'conversation-keep', 'library-keep', 'Saved',
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                 );
                 INSERT INTO agent_runs (
                    id, conversation_id, question, intent, scope_json,
                    priority_context_json, provider_json, generation_json,
                    prompt_versions_json, tool_calls_json, retrieval_json,
                    context_json, completion_state, created_at
                 ) VALUES (
                    'run-keep', 'conversation-keep', 'Question',
                    'current-paper-qa', '{}', '{}', '{}', '{}', '{}',
                    '[]', '{}', '{}', 'complete', CURRENT_TIMESTAMP
                 );
                 INSERT INTO agent_sources (
                    run_id, source_id, evidence_kind, item_id, item_title,
                    document_id, document_revision_id, chunk_id,
                    retrieval_modes_json, rank_json, provider,
                    source_fingerprint, eligible_for_scientific_claim
                 ) VALUES (
                    'run-keep', 'S120', 'document-passage',
                    'knowledge-keep', 'Persistent item', 'artifact-keep',
                    1, 'chunk-keep', '[\"hybrid\"]', '{}', 'fixture',
                    'source-hash', 1
                 );
                 INSERT INTO app_meta(key, value)
                 VALUES ('active_source_id', 'source-keep');
                 INSERT INTO app_meta(key, value)
                 VALUES ('active_library_id', 'library-keep');",
            )
            .expect("schema-nine fixture");
        drop(connection);

        let database = ResearchDatabase::initialize(path.clone()).expect("upgrade schema nine");
        let connection = database.connect().expect("upgraded connection");
        let identities = connection
            .query_row(
                "SELECT i.id, d.id, rs.document_id, ase.source_id
                 FROM research_items i
                 JOIN documents d ON d.research_item_id = i.id
                 JOIN reader_states rs ON rs.document_id = d.id
                 JOIN agent_sources ase ON ase.item_id = i.id
                 WHERE i.id = 'knowledge-keep'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .expect("preserved identities");
        assert_eq!(
            identities,
            (
                "knowledge-keep".to_owned(),
                "artifact-keep".to_owned(),
                "artifact-keep".to_owned(),
                "S120".to_owned(),
            )
        );
        let active_vectors: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM semantic_vector_entries
                 WHERE active = 1 AND vector_key = 120",
                [],
                |row| row.get(0),
            )
            .expect("active vectors");
        let fts_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM research_fts
                 WHERE research_fts MATCH 'persistent'",
                [],
                |row| row.get(0),
            )
            .expect("FTS rows");
        let binding_counts = connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*)
                     FROM knowledge_item_source_bindings),
                    (SELECT COUNT(*)
                     FROM document_artifact_source_bindings)",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("binding counts");
        let active_projection = connection
            .query_row(
                "SELECT COUNT(*), MIN(provider)
                 FROM document_texts WHERE document_id = 'artifact-keep'
                   AND is_active = 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .expect("active projection");
        let legacy_projections = {
            let mut statement = connection
                .prepare(
                    "SELECT provider, content, is_active
                     FROM document_texts
                     WHERE document_id = 'artifact-keep'
                     ORDER BY provider",
                )
                .expect("legacy provider statement");
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })
                .expect("legacy provider rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("legacy providers")
        };
        let migration_eleven_applied: bool = connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM schema_migrations WHERE version = 11
                 )",
                [],
                |row| row.get(0),
            )
            .expect("migration eleven record");
        assert_eq!(active_vectors, 1);
        assert_eq!(fts_rows, 1);
        assert_eq!(binding_counts, (1, 1));
        assert_eq!(active_projection, (1, "zotero-indexed".to_owned()));
        assert!(migration_eleven_applied);
        assert_eq!(
            legacy_projections,
            vec![
                (
                    "legacy-alternative".to_owned(),
                    "older alternate body".to_owned(),
                    0,
                ),
                (
                    "zotero-indexed".to_owned(),
                    "persistent searchable body".to_owned(),
                    1,
                ),
            ]
        );
        fs::remove_file(path).expect("remove schema-nine database");
    }

    #[test]
    fn migration_twelve_moves_selection_without_rewriting_extraction_evidence() {
        let path = std::env::temp_dir().join(format!(
            "graphite-schema-eleven-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let connection = rusqlite::Connection::open(&path).expect("schema-eleven connection");
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                 );",
            )
            .expect("migration table");
        for (version, sql) in [
            (1_i64, super::MIGRATION_1),
            (2, super::MIGRATION_2),
            (3, super::MIGRATION_3),
            (4, super::MIGRATION_4),
            (5, super::MIGRATION_5),
            (6, super::MIGRATION_6),
            (7, super::MIGRATION_7),
            (8, super::MIGRATION_8),
            (9, super::MIGRATION_9),
            (10, super::MIGRATION_10),
            (11, super::MIGRATION_11),
        ] {
            connection
                .execute_batch(sql)
                .expect("schema-eleven migration");
            connection
                .execute(
                    "INSERT INTO schema_migrations(version, name, applied_at)
                     VALUES (?1, 'fixture', CURRENT_TIMESTAMP)",
                    [version],
                )
                .expect("record schema-eleven migration");
        }
        connection
            .execute_batch(
                "INSERT INTO sources (
                    id, server_id, api_version, compatibility,
                    first_seen_at, last_seen_at, connection_status
                 ) VALUES (
                    'source', 'server', '3', 'modern',
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, 'online'
                 );
                 INSERT INTO libraries (
                    id, source_id, kind, zotero_library_id, name,
                    successful_version, available
                 ) VALUES ('library', 'source', 'user', '0', 'Library', 1, 1);
                 INSERT INTO research_items (
                    id, source_id, library_id, zotero_key, item_type,
                    title, source_version, knowledge_kind
                 ) VALUES (
                    'item', 'source', 'library', 'ITEM', 'journalArticle',
                    'Item', 1, 'paper'
                 );
                 INSERT INTO attachments (
                    id, source_id, library_id, zotero_key, parent_item_id,
                    title, content_type, available_locally, source_version
                 ) VALUES (
                    'attachment', 'source', 'library', 'PDF', 'item',
                    'PDF', 'application/pdf', 1, 1
                 );
                 INSERT INTO documents (
                    id, source_id, library_id, attachment_id,
                    research_item_id, zotero_attachment_key, media_type,
                    title, source_version, availability, created_at,
                    updated_at, artifact_role
                 ) VALUES (
                    'artifact', 'source', 'library', 'attachment', 'item',
                    'PDF', 'application/pdf', 'PDF', 1, 'ready',
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP, 'attachment'
                 );
                 INSERT INTO document_revisions (
                    id, document_id, source_version, revision_key,
                    discovered_at, revision_kind, content_hash
                 ) VALUES (
                    1, 'artifact', 1, 'text:zotero-indexed:1',
                    CURRENT_TIMESTAMP, 'source-provided-text', 'input-hash'
                 );
                 INSERT INTO extraction_provider_profiles (
                    id, provider_id, provider_version, mapping_profile,
                    supported_media_types_json, capabilities_json,
                    configured_available, created_at
                 ) VALUES (
                    'provider-profile', 'provider', '1.0.0', 'mapping-v1',
                    '[]', '{}', 1, CURRENT_TIMESTAMP
                 );
                 INSERT INTO extraction_runs (
                    id, artifact_id, artifact_revision_id,
                    routing_profile_id, execution_state, selected_extraction_id,
                    fallback_used, started_at, completed_at
                 ) VALUES (
                    'run', 'artifact', 1, 'source-text-fallback-v1',
                    'succeeded', 'extraction', 0,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                 );
                 INSERT INTO document_extractions (
                    id, run_id, artifact_id, artifact_revision_id,
                    provider_profile_id, canonical_schema_version,
                    canonical_json, canonical_hash, plain_text_hash,
                    execution_state, selected, fallback_used,
                    provenance_json, observations_json, warnings_json,
                    error, created_at, completed_at
                 ) VALUES (
                    'extraction', 'run', 'artifact', 1,
                    'provider-profile', 'canonical-document-v1',
                    '{\"evidence\":\"preserve\"}', 'canonical-hash',
                    'text-hash', 'succeeded', 1, 0,
                    '{\"provider\":\"fixture\"}',
                    '{\"signal\":\"observed\"}',
                    '[\"warning\"]', NULL,
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                 );
                 INSERT INTO extraction_quality_reports (
                    id, extraction_id, quality_profile_id,
                    quality_status, usable, signals_json,
                    reasons_json, evaluated_at
                 ) VALUES (
                    'report', 'extraction', 'extraction-quality-v1',
                    'good', 1, '{\"quality\":\"good\"}', '[]',
                    CURRENT_TIMESTAMP
                 );
                 INSERT INTO artifact_extraction_state (
                    artifact_id, active_extraction_id,
                    active_quality_report_id, execution_state,
                    quality_status, updated_at
                 ) VALUES (
                    'artifact', 'extraction', 'report', 'succeeded',
                    'good', CURRENT_TIMESTAMP
                 );",
            )
            .expect("schema-eleven extraction fixture");
        let evidence_before = connection
            .query_row(
                "SELECT canonical_json, canonical_hash, plain_text_hash,
                        provenance_json, observations_json, warnings_json, error
                 FROM document_extractions WHERE id = 'extraction'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .expect("evidence before migration");
        drop(connection);

        let database = ResearchDatabase::initialize(path.clone()).expect("upgrade schema eleven");
        let connection = database.connect().expect("upgraded connection");
        let evidence_after = connection
            .query_row(
                "SELECT canonical_json, canonical_hash, plain_text_hash,
                        provenance_json, observations_json, warnings_json, error
                 FROM document_extractions WHERE id = 'extraction'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .expect("evidence after migration");
        let selection = connection
            .query_row(
                "SELECT extraction_id, quality_report_id, routing_profile_id
                 FROM active_extraction_selections WHERE artifact_id = 'artifact'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .expect("migrated selection");
        let selected_marker: i64 = connection
            .query_row(
                "SELECT selected FROM document_extractions WHERE id = 'extraction'",
                [],
                |row| row.get(0),
            )
            .expect("legacy selected marker");
        assert_eq!(evidence_after, evidence_before);
        assert_eq!(
            selection,
            (
                "extraction".to_owned(),
                "report".to_owned(),
                "source-text-fallback-v1".to_owned(),
            )
        );
        assert_eq!(selected_marker, 0);
        fs::remove_file(path).expect("remove schema-eleven database");
    }

    #[test]
    fn preserves_entities_until_their_final_source_binding_is_deleted() {
        let (database, path) = temporary_database("binding-deletion");
        let first_source = database
            .register_source(&SourceProbe {
                server_id: "binding-source-a".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("first source");
        let first_library = database
            .register_libraries(
                &first_source.id,
                &[RemoteLibrary {
                    kind: "user".to_owned(),
                    zotero_id: "0".to_owned(),
                    name: "First library".to_owned(),
                }],
            )
            .expect("first library")
            .remove(0);
        database
            .apply_delta(
                &first_source.id,
                &first_library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects: vec![fixture_item("ITEMA", "Linked paper", 1)],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("first item");
        let item_id = stable_id("item", &[&first_source.id, &first_library.id, "ITEMA"]);
        let second_source = database
            .register_source(&SourceProbe {
                server_id: "binding-source-b".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("second source");
        let second_library = database
            .register_libraries(
                &second_source.id,
                &[RemoteLibrary {
                    kind: "user".to_owned(),
                    zotero_id: "0".to_owned(),
                    name: "Second library".to_owned(),
                }],
            )
            .expect("second library")
            .remove(0);
        let connection = database.connect().expect("connection");
        let second_instance: String = connection
            .query_row(
                "SELECT id FROM source_instances
                 WHERE legacy_library_id = ?1",
                [&second_library.id],
                |row| row.get(0),
            )
            .expect("second source instance");
        let first_binding_id: String = connection
            .query_row(
                "SELECT id FROM knowledge_item_source_bindings
                 WHERE knowledge_item_id = ?1",
                [&item_id],
                |row| row.get(0),
            )
            .expect("first source-qualified binding");
        let second_binding_id = super::source_qualified_binding_id(
            "knowledge-binding",
            &second_instance,
            "LINKED",
            &item_id,
            "knowledge-item",
        );
        for _ in 0..2 {
            connection
                .execute(
                    "INSERT INTO knowledge_item_source_bindings (
                    id, knowledge_item_id, source_instance_id,
                    external_item_id, external_item_kind, source_revision,
                    normalized_source_metadata_json, metadata_hash,
                    is_primary, state, first_seen_at, last_seen_at
                 ) VALUES (
                    ?1, ?2, ?3, 'LINKED',
                    'paper', '1', '{}', 'linked-hash', 0, 'active',
                    CURRENT_TIMESTAMP, CURRENT_TIMESTAMP
                 )
                 ON CONFLICT(source_instance_id, external_item_id) DO UPDATE SET
                    last_seen_at = excluded.last_seen_at",
                    params![second_binding_id, item_id, second_instance],
                )
                .expect("idempotent second binding");
        }
        let binding_ids = {
            let mut statement = connection
                .prepare(
                    "SELECT id FROM knowledge_item_source_bindings
                     WHERE knowledge_item_id = ?1 ORDER BY id",
                )
                .expect("binding IDs");
            statement
                .query_map([&item_id], |row| row.get::<_, String>(0))
                .expect("binding rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("bindings")
        };
        assert_eq!(binding_ids.len(), 2);
        assert_ne!(first_binding_id, second_binding_id);
        assert!(binding_ids.contains(&first_binding_id));
        assert!(binding_ids.contains(&second_binding_id));
        drop(connection);
        database
            .apply_delta(
                &first_source.id,
                &first_library.id,
                &LibraryDelta {
                    target_version: 2,
                    objects: Vec::new(),
                    deleted_items: vec!["ITEMA".to_owned()],
                    deleted_collections: Vec::new(),
                },
            )
            .expect("delete first binding");
        let connection = database.connect().expect("connection");
        let item_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM research_items WHERE id = ?1)",
                [&item_id],
                |row| row.get(0),
            )
            .expect("preserved item");
        assert!(item_exists);
        drop(connection);
        database
            .apply_delta(
                &second_source.id,
                &second_library.id,
                &LibraryDelta {
                    target_version: 2,
                    objects: Vec::new(),
                    deleted_items: vec!["LINKED".to_owned()],
                    deleted_collections: Vec::new(),
                },
            )
            .expect("delete final binding");
        let connection = database.connect().expect("connection");
        let item_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM research_items WHERE id = ?1)",
                [&item_id],
                |row| row.get(0),
            )
            .expect("deleted item");
        let detached_tombstones: i64 = connection
            .query_row(
                "SELECT COUNT(*)
                 FROM knowledge_item_source_bindings
                 WHERE knowledge_item_id IS NULL AND state = 'deleted'",
                [],
                |row| row.get(0),
            )
            .expect("detached tombstones");
        assert!(!item_exists);
        assert_eq!(detached_tombstones, 2);
        fs::remove_file(path).expect("remove temporary database");
    }

    #[test]
    fn partitions_sources_and_applies_explicit_deletions() {
        let (database, path) = temporary_database("partitions");
        for server_id in ["server-a", "server-b"] {
            let source = database
                .register_source(&SourceProbe {
                    server_id: server_id.to_owned(),
                    api_version: "3".to_owned(),
                    schema_version: None,
                })
                .expect("source");
            let remote = RemoteLibrary {
                kind: "user".to_owned(),
                zotero_id: "0".to_owned(),
                name: "My Library".to_owned(),
            };
            let library = database
                .register_libraries(&source.id, std::slice::from_ref(&remote))
                .expect("library")
                .remove(0);
            database
                .apply_delta(
                    &source.id,
                    &library.id,
                    &LibraryDelta {
                        target_version: 1,
                        objects: vec![fixture_item(
                            "SHAREDKEY",
                            &format!("Paper from {server_id}"),
                            1,
                        )],
                        deleted_items: Vec::new(),
                        deleted_collections: Vec::new(),
                    },
                )
                .expect("insert");
            if server_id == "server-b" {
                database
                    .apply_delta(
                        &source.id,
                        &library.id,
                        &LibraryDelta {
                            target_version: 2,
                            objects: Vec::new(),
                            deleted_items: vec!["SHAREDKEY".to_owned()],
                            deleted_collections: Vec::new(),
                        },
                    )
                    .expect("delete");
            }
        }
        let workspace = database.load_workspace().expect("workspace");
        assert!(workspace.snapshot.items.is_empty());
        fs::remove_file(path).expect("remove temporary database");
    }

    #[test]
    fn handles_thousands_of_records_with_bounded_graph_edges() {
        let (database, path) = temporary_database("stress");
        let source = database
            .register_source(&SourceProbe {
                server_id: "stress-server".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("source");
        let remote = RemoteLibrary {
            kind: "user".to_owned(),
            zotero_id: "0".to_owned(),
            name: "My Library".to_owned(),
        };
        let library = database
            .register_libraries(&source.id, std::slice::from_ref(&remote))
            .expect("library")
            .remove(0);
        let mut objects: Vec<_> = (0..10_000)
            .map(|index| {
                let mut item = fixture_item(
                    &format!("ITEM{index:05}"),
                    &format!("Research paper {index}"),
                    1,
                );
                if index < 1_500 {
                    item.json["data"]["collections"] = serde_json::json!(["LARGE"]);
                }
                item
            })
            .collect();
        objects.push(fixture_collection("LARGE", "Large collection"));
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects,
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("bulk delta");
        let workspace = database.load_workspace().expect("workspace");
        assert_eq!(workspace.snapshot.items.len(), 10_000);
        assert_eq!(workspace.snapshot.total_item_count, 10_000);
        assert_eq!(workspace.graph.nodes.len(), 10_000);
        assert!(!workspace.graph.has_more);
        assert!(workspace.graph.edges.len() < 2_000);
        for (collection_id, expected) in [
            (None, 10_000),
            (
                Some(stable_id("collection", &[&source.id, &library.id, "LARGE"])),
                1_500,
            ),
        ] {
            let projection = database
                .graph_projection(&GraphProjectionRequest {
                    collection_id,
                    seed_item_ids: Vec::new(),
                    limit: None,
                    offset: None,
                })
                .expect("complete graph projection");
            assert_eq!(projection.items.len(), expected);
            assert_eq!(projection.graph.nodes.len(), expected);
            assert_eq!(projection.graph.total_node_count, expected as u64);
            assert!(!projection.graph.has_more);
        }
        let seeded_ids = (9_900..10_000)
            .map(|index| {
                stable_id(
                    "item",
                    &[&source.id, &library.id, &format!("ITEM{index:05}")],
                )
            })
            .collect::<Vec<_>>();
        let projection = database
            .graph_projection(&GraphProjectionRequest {
                collection_id: None,
                seed_item_ids: seeded_ids.clone(),
                limit: Some(600),
                offset: Some(0),
            })
            .expect("seeded graph projection");
        assert_eq!(projection.items.len(), 600);
        assert!(projection.items.iter().any(|item| item.id == seeded_ids[0]));
        let lens_ids = workspace
            .snapshot
            .items
            .iter()
            .rev()
            .take(1_200)
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        let lens = database
            .graph_projection(&GraphProjectionRequest {
                collection_id: None,
                seed_item_ids: lens_ids.clone(),
                limit: Some(1_200),
                offset: None,
            })
            .expect("complete Lens membership beyond the query page limit");
        assert_eq!(
            lens.items
                .iter()
                .map(|item| &item.id)
                .collect::<std::collections::HashSet<_>>(),
            lens_ids.iter().collect::<std::collections::HashSet<_>>(),
        );
        fs::remove_file(path).expect("remove temporary database");
    }

    #[test]
    fn indexes_document_text_and_preserves_it_across_metadata_updates() {
        let (database, path) = temporary_database("fulltext");
        let source = database
            .register_source(&SourceProbe {
                server_id: "corpus-server".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("source");
        let library = database
            .register_libraries(
                &source.id,
                &[RemoteLibrary {
                    kind: "user".to_owned(),
                    zotero_id: "0".to_owned(),
                    name: "My Library".to_owned(),
                }],
            )
            .expect("library")
            .remove(0);
        let mut paper_one = fixture_item("PAPER1", "Crustal observations", 1);
        paper_one.json["data"]["collections"] = serde_json::json!(["COLL1"]);
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 1,
                    objects: vec![
                        fixture_collection("COLL1", "Scoped collection"),
                        paper_one,
                        fixture_attachment("PDF1", "PAPER1", 1),
                        fixture_item("PAPER2", "Distractor observations", 1),
                        fixture_attachment("PDF2", "PAPER2", 1),
                    ],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("metadata");
        database
            .apply_fulltext_delta(
                &source.id,
                &library.id,
                &FullTextSyncDelta {
                    successful_version: 3,
                    records: vec![
                        FullTextRecord {
                            attachment_key: "PDF1".to_owned(),
                            version: 3,
                            content: "A distinctive lithospheric dispersion signal".to_owned(),
                            indexed_pages: Some(12),
                            total_pages: Some(12),
                            indexed_chars: None,
                            total_chars: None,
                        },
                        FullTextRecord {
                            attachment_key: "PDF2".to_owned(),
                            version: 3,
                            content: "A stronger lithospheric dispersion distractor".to_owned(),
                            indexed_pages: Some(4),
                            total_pages: Some(4),
                            indexed_chars: None,
                            total_chars: None,
                        },
                    ],
                    removed_attachment_keys: Vec::new(),
                    warning_count: 0,
                    status: "ready".to_owned(),
                    last_error: None,
                },
            )
            .expect("full text");
        let page = database
            .search(&SearchRequest {
                query: "lithospheric dispersion".to_owned(),
                mode: None,
                limit: Some(10),
                offset: Some(0),
                item_ids: Vec::new(),
                collection_id: None,
                citation_filter: None,
                year_from: None,
                year_to: None,
                item_type: None,
            })
            .expect("search");
        assert_eq!(page.total, 2);
        assert_eq!(page.results[0].match_source, "metadata-and-full-text");
        let paper_one_id = database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT id FROM research_items WHERE zotero_key = 'PAPER1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("first paper ID");
        let collection_id = database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT id FROM collections WHERE zotero_key = 'COLL1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("collection ID");
        let scoped_page = database
            .search(&SearchRequest {
                query: "lithospheric dispersion".to_owned(),
                mode: None,
                limit: Some(10),
                offset: Some(0),
                item_ids: vec![paper_one_id.clone(), paper_one_id.clone()],
                collection_id: Some(collection_id),
                citation_filter: None,
                year_from: Some(2022),
                year_to: Some(2022),
                item_type: Some("journalArticle".to_owned()),
            })
            .expect("item-scoped search");
        assert_eq!(scoped_page.total, 1);
        assert_eq!(scoped_page.results[0].item.title, "Crustal observations");
        let unknown_scope = database
            .search(&SearchRequest {
                query: "lithospheric dispersion".to_owned(),
                mode: None,
                limit: Some(10),
                offset: Some(0),
                item_ids: vec!["foreign-or-missing-item".to_owned()],
                collection_id: None,
                citation_filter: None,
                year_from: None,
                year_to: None,
                item_type: None,
            })
            .expect("unknown item scope");
        assert_eq!(unknown_scope.total, 0);
        let metadata_page = database
            .search(&SearchRequest {
                query: "crustal observations".to_owned(),
                mode: None,
                limit: Some(10),
                offset: Some(0),
                item_ids: Vec::new(),
                collection_id: None,
                citation_filter: None,
                year_from: None,
                year_to: None,
                item_type: None,
            })
            .expect("metadata search");
        assert_eq!(metadata_page.results[0].match_source, "metadata");

        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 2,
                    objects: vec![fixture_item("PAPER1", "Updated observations", 2)],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("metadata update");
        let workspace = database.load_workspace().expect("workspace");
        assert_eq!(workspace.corpus.full_text_count, 2);
        assert_eq!(workspace.snapshot.items[0].attachments.len(), 1);
        database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 3,
                    objects: Vec::new(),
                    deleted_items: vec!["PDF1".to_owned()],
                    deleted_collections: Vec::new(),
                },
            )
            .expect("attachment deletion");
        let deleted_page = database
            .search(&SearchRequest {
                query: "lithospheric dispersion".to_owned(),
                mode: None,
                limit: Some(10),
                offset: Some(0),
                item_ids: vec![paper_one_id],
                collection_id: None,
                citation_filter: None,
                year_from: None,
                year_to: None,
                item_type: None,
            })
            .expect("search after deletion");
        assert_eq!(deleted_page.total, 0);
        fs::remove_file(path).expect("remove temporary database");
    }

    #[test]
    fn records_malformed_objects_without_aborting_safe_normalization() {
        let (database, path) = temporary_database("diagnostics");
        let source = database
            .register_source(&SourceProbe {
                server_id: "rollback-server".to_owned(),
                api_version: "3".to_owned(),
                schema_version: None,
            })
            .expect("source");
        let remote = RemoteLibrary {
            kind: "user".to_owned(),
            zotero_id: "0".to_owned(),
            name: "My Library".to_owned(),
        };
        let library = database
            .register_libraries(&source.id, std::slice::from_ref(&remote))
            .expect("library")
            .remove(0);
        let malformed = RawObject {
            key: "BROKEN".to_owned(),
            version: 8,
            object_type: "collections".to_owned(),
            json: serde_json::json!({"key": "BROKEN", "version": 8}),
        };
        let result = database
            .apply_delta(
                &source.id,
                &library.id,
                &LibraryDelta {
                    target_version: 8,
                    objects: vec![fixture_item("VALID", "Safe paper", 8), malformed],
                    deleted_items: Vec::new(),
                    deleted_collections: Vec::new(),
                },
            )
            .expect("delta with warning");

        assert_eq!(result, (2, 0, 1));
        let workspace = database.load_workspace().expect("workspace");
        assert_eq!(workspace.snapshot.items.len(), 1);
        assert_eq!(workspace.snapshot.items[0].title, "Safe paper");
        let connection = database.connect().expect("connection");
        let diagnostics: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM normalization_diagnostics",
                [],
                |row| row.get(0),
            )
            .expect("diagnostics");
        assert_eq!(diagnostics, 1);
        let stored = database
            .register_libraries(&source.id, &[remote])
            .expect("library state")
            .remove(0);
        assert_eq!(stored.version, 8);
        fs::remove_file(path).expect("remove temporary database");
    }
}
