use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags};

use crate::error::{AppError, AppResult};

use super::model::{
    ExperimentDefinition, ExperimentIndexSummary, ExperimentMode, SnapshotItem, SnapshotProjection,
};

const SCHEMA: &str = include_str!("schema_v1.sql");

pub(super) struct ExperimentStore {
    connection: Connection,
}

pub(super) struct StoredChunk<'a> {
    pub id: &'a str,
    pub logical_chunk_id: &'a str,
    pub item_id: &'a str,
    pub document_id: &'a str,
    pub document_revision_id: i64,
    pub ordinal: u32,
    pub text: &'a str,
    pub character_start: u64,
    pub character_end: u64,
    pub page_start: Option<u32>,
    pub page_end: Option<u32>,
    pub token_count: u32,
    pub content_fingerprint: &'a str,
    pub vector_key: u64,
}

pub(super) struct StoredProvenance<'a> {
    pub chunk_id: &'a str,
    pub projection: &'a SnapshotProjection,
    pub page_start: Option<u32>,
    pub page_end: Option<u32>,
}

pub(super) struct SummaryRuntimeMetrics {
    pub sqlite_bytes: u64,
    pub vector_bytes: u64,
    pub embedded_chunk_count: u64,
    pub reused_duplicate_count: u64,
    pub idempotent_reuse: bool,
}

impl ExperimentStore {
    pub(super) fn initialize(
        path: &Path,
        definition: &ExperimentDefinition,
        execution_id: &str,
        mode: ExperimentMode,
    ) -> AppResult<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(SCHEMA)?;
        connection.execute(
            "INSERT INTO experiment_metadata(key, value) VALUES
                ('schema-version', ?1),
                ('definition-id', ?2),
                ('experiment-profile', ?3),
                ('embedding-profile-id', ?4),
                ('chunking-profile-id', ?5),
                ('retrieval-profile', ?6),
                ('context-profile', ?7)",
            params![
                definition.schema_version.to_string(),
                definition.definition_id,
                definition.experiment_profile,
                definition.embedding_profile_id,
                definition.chunking_profile,
                definition.retrieval_profile,
                definition.context_profile,
            ],
        )?;
        connection.execute(
            "INSERT INTO experiment_executions(
                id, definition_id, mode, state, started_at
             ) VALUES (?1, ?2, ?3, 'staging', ?4)",
            params![
                execution_id,
                definition.definition_id,
                mode.as_str(),
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(Self { connection })
    }

    pub(super) fn open_read_only(path: &Path) -> AppResult<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.execute_batch("PRAGMA query_only = ON; PRAGMA foreign_keys = ON;")?;
        Ok(Self { connection })
    }

    pub(super) fn open(path: &Path) -> AppResult<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self { connection })
    }

    pub(super) fn connection(&self) -> &Connection {
        &self.connection
    }

    pub(super) fn insert_item(&self, item: &SnapshotItem) -> AppResult<()> {
        self.connection.execute(
            "INSERT INTO experiment_items(
                item_id, library_id, title, creators, tags,
                abstract_text, venue, document_id, document_revision_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                item.item_id,
                item.library_id,
                item.title,
                item.creators,
                item.tags,
                item.abstract_text,
                item.venue,
                item.document_id,
                item.document_revision_id,
            ],
        )?;
        self.connection.execute(
            "INSERT INTO search_content(
                item_id, library_id, title, creators, tags,
                abstract_text, venue, body
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                item.item_id,
                item.library_id,
                item.title,
                item.creators,
                item.tags,
                item.abstract_text,
                item.venue,
                item.body,
            ],
        )?;
        Ok(())
    }

    pub(super) fn insert_projection(&self, projection: &SnapshotProjection) -> AppResult<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO experiment_projections(
                id, item_id, document_id, document_revision_id,
                source_kind, extraction_id, provider_profile,
                content, content_fingerprint
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                projection.projection_id,
                projection.item_id,
                projection.document_id,
                projection.document_revision_id,
                projection.source_kind.as_str(),
                projection.extraction_id,
                projection.provider_profile,
                projection.content,
                projection.content_fingerprint,
            ],
        )?;
        Ok(())
    }

    pub(super) fn insert_chunk(&self, chunk: StoredChunk<'_>) -> AppResult<()> {
        self.connection.execute(
            "INSERT INTO experiment_vector_entries(
                vector_key, kind, target_id, embedding_profile_id
             ) VALUES (?1, 'chunk', ?2,
                (SELECT value FROM experiment_metadata
                 WHERE key = 'embedding-profile-id'))",
            params![chunk.vector_key as i64, chunk.id],
        )?;
        self.connection.execute(
            "INSERT INTO experiment_chunks(
                id, logical_chunk_id, item_id, document_id,
                document_revision_id, ordinal, text,
                character_start, character_end, page_start, page_end,
                token_count, content_fingerprint, vector_key
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7,
                ?8, ?9, ?10, ?11, ?12, ?13, ?14
             )",
            params![
                chunk.id,
                chunk.logical_chunk_id,
                chunk.item_id,
                chunk.document_id,
                chunk.document_revision_id,
                chunk.ordinal,
                chunk.text,
                chunk.character_start as i64,
                chunk.character_end as i64,
                chunk.page_start,
                chunk.page_end,
                chunk.token_count,
                chunk.content_fingerprint,
                chunk.vector_key as i64,
            ],
        )?;
        Ok(())
    }

    pub(super) fn insert_provenance(&self, provenance: StoredProvenance<'_>) -> AppResult<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO experiment_chunk_provenance(
                chunk_id, projection_id, source_kind,
                extraction_id, provider_profile, page_start, page_end
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                provenance.chunk_id,
                provenance.projection.projection_id,
                provenance.projection.source_kind.as_str(),
                provenance.projection.extraction_id,
                provenance.projection.provider_profile,
                provenance.page_start,
                provenance.page_end,
            ],
        )?;
        Ok(())
    }

    pub(super) fn insert_paper_vector(
        &self,
        item_id: &str,
        representation_kind: &str,
        vector_key: u64,
    ) -> AppResult<()> {
        self.connection.execute(
            "INSERT INTO experiment_vector_entries(
                vector_key, kind, target_id, embedding_profile_id
             ) VALUES (?1, 'paper', ?2,
                (SELECT value FROM experiment_metadata
                 WHERE key = 'embedding-profile-id'))",
            params![vector_key as i64, item_id],
        )?;
        self.connection.execute(
            "INSERT INTO experiment_paper_representations(
                item_id, representation_kind, vector_key
             ) VALUES (?1, ?2, ?3)",
            params![item_id, representation_kind, vector_key as i64],
        )?;
        Ok(())
    }

    pub(super) fn finish(&self, execution_id: &str) -> AppResult<()> {
        self.connection.execute(
            "UPDATE experiment_executions SET
                state = 'ready', completed_at = ?2, last_error = NULL
             WHERE id = ?1 AND state = 'staging'",
            params![execution_id, Utc::now().to_rfc3339()],
        )?;
        self.connection.execute_batch(
            "INSERT INTO research_fts(research_fts) VALUES('optimize');
             PRAGMA optimize;",
        )?;
        Ok(())
    }

    pub(super) fn record_idempotent_execution(
        &self,
        execution_id: &str,
        definition_id: &str,
        mode: ExperimentMode,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        self.connection.execute(
            "INSERT INTO experiment_executions(
                id, definition_id, mode, state,
                started_at, completed_at, last_error
             ) VALUES (?1, ?2, ?3, 'ready', ?4, ?4, NULL)",
            params![execution_id, definition_id, mode.as_str(), now],
        )?;
        Ok(())
    }

    pub(super) fn cancel(&self, execution_id: &str) -> AppResult<()> {
        self.connection.execute(
            "UPDATE experiment_executions SET
                state = 'cancelled', completed_at = ?2,
                last_error = 'cancelled before publication'
             WHERE id = ?1 AND state = 'staging'",
            params![execution_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub(super) fn definition_id(&self) -> AppResult<String> {
        self.connection
            .query_row(
                "SELECT value FROM experiment_metadata WHERE key = 'definition-id'",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub(super) fn metadata_value(&self, key: &str) -> AppResult<String> {
        self.connection
            .query_row(
                "SELECT value FROM experiment_metadata WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub(super) fn ready(&self) -> AppResult<bool> {
        self.connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM experiment_executions WHERE state = 'ready'
                 )",
                [],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub(super) fn summary(
        &self,
        execution_id: String,
        mode: ExperimentMode,
        metrics: SummaryRuntimeMetrics,
    ) -> AppResult<ExperimentIndexSummary> {
        let (items, projections, chunks, provenance, papers) = self.connection.query_row(
            "SELECT
                (SELECT COUNT(*) FROM experiment_items),
                (SELECT COUNT(*) FROM experiment_projections),
                (SELECT COUNT(*) FROM experiment_chunks),
                (SELECT COUNT(*) FROM experiment_chunk_provenance),
                (SELECT COUNT(*) FROM experiment_paper_representations)",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                ))
            },
        )?;
        Ok(ExperimentIndexSummary {
            definition_id: self.definition_id()?,
            execution_id,
            mode,
            item_count: items,
            projection_count: projections,
            chunk_count: chunks,
            provenance_count: provenance,
            embedded_chunk_count: metrics.embedded_chunk_count,
            reused_duplicate_count: metrics.reused_duplicate_count,
            paper_vector_count: papers,
            sqlite_bytes: metrics.sqlite_bytes,
            vector_bytes: metrics.vector_bytes,
            idempotent_reuse: metrics.idempotent_reuse,
        })
    }
}

pub(super) fn validate_ready_store(
    path: &Path,
    definition: &ExperimentDefinition,
) -> AppResult<Option<ExperimentStore>> {
    if !path.is_file() {
        return Ok(None);
    }
    let store = ExperimentStore::open_read_only(path)?;
    if store.definition_id()? != definition.definition_id || !store.ready()? {
        return Err(AppError::ApplicationData(
            "The published experimental index does not match its definition".to_owned(),
        ));
    }
    Ok(Some(store))
}
