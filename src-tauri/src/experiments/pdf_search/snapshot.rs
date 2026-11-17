use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use crate::error::{AppError, AppResult};
use crate::extraction::model::CanonicalDocumentV1;
use crate::extraction::projection::render_plain_text;
use crate::semantic::EMBEDDING_DIMENSION;

use super::model::{
    sha256, ProductionSnapshot, SnapshotChunk, SnapshotEmbeddingProfile, SnapshotItem,
    SnapshotPaperVector, SnapshotProjection, SourceKind,
};

pub struct ReadOnlyProductionSnapshot;

impl ReadOnlyProductionSnapshot {
    pub fn capture(
        database_path: &Path,
        production_vector_root: &Path,
        requested_item_ids: &[String],
    ) -> AppResult<ProductionSnapshot> {
        if requested_item_ids.is_empty() || requested_item_ids.len() > 30 {
            return Err(AppError::ApplicationData(
                "The PDF search experiment requires between 1 and 30 selected items".to_owned(),
            ));
        }
        let requested = requested_item_ids.iter().collect::<HashSet<_>>();
        if requested.len() != requested_item_ids.len() {
            return Err(AppError::ApplicationData(
                "The PDF search experiment item scope contains duplicates".to_owned(),
            ));
        }
        let connection = open_read_only(database_path)?;
        let embedding_profile = active_embedding_profile(&connection)?;
        let items = load_items(&connection, requested_item_ids)?;
        if items.len() != requested_item_ids.len() {
            return Err(AppError::ApplicationData(format!(
                "The read-only production snapshot found {} of {} requested Zotero items",
                items.len(),
                requested_item_ids.len()
            )));
        }
        let chunk_rows = load_chunk_rows(&connection, &embedding_profile.id, &requested)?;
        let paper_rows = load_paper_rows(&connection, &embedding_profile.id, &requested)?;
        let chunk_vectors = read_vectors(
            &production_vector_root.join(format!("{}-chunk.usearch", embedding_profile.id)),
            chunk_rows.iter().map(|row| row.vector_key),
        )?;
        let paper_vectors = read_vectors(
            &production_vector_root.join(format!("{}-paper.usearch", embedding_profile.id)),
            paper_rows.iter().map(|row| row.vector_key),
        )?;
        let chunks = chunk_rows
            .into_iter()
            .map(|row| {
                let vector = chunk_vectors.get(&row.vector_key).cloned().ok_or_else(|| {
                    AppError::Semantic(format!(
                        "The production chunk vector {} is missing",
                        row.vector_key
                    ))
                })?;
                Ok(SnapshotChunk {
                    id: row.id,
                    item_id: row.item_id,
                    document_id: row.document_id,
                    document_revision_id: row.document_revision_id,
                    ordinal: row.ordinal,
                    text: row.text,
                    character_start: row.character_start,
                    character_end: row.character_end,
                    page_start: row.page_start,
                    page_end: row.page_end,
                    fingerprint: row.fingerprint,
                    vector_key: row.vector_key,
                    vector,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        let paper_vectors = paper_rows
            .into_iter()
            .map(|row| {
                let vector = paper_vectors.get(&row.vector_key).cloned().ok_or_else(|| {
                    AppError::Semantic(format!(
                        "The production paper vector {} is missing",
                        row.vector_key
                    ))
                })?;
                Ok(SnapshotPaperVector {
                    item_id: row.item_id,
                    representation_kind: row.representation_kind,
                    vector_key: row.vector_key,
                    vector,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        let shadow_projections = load_shadow_projections(&connection, &requested)?;
        assert_query_only(&connection)?;
        Ok(ProductionSnapshot {
            embedding_profile,
            items,
            chunks,
            paper_vectors,
            shadow_projections,
        })
    }
}

fn open_read_only(path: &Path) -> AppResult<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
    connection.execute_batch("PRAGMA query_only = ON; PRAGMA foreign_keys = ON;")?;
    Ok(connection)
}

fn assert_query_only(connection: &Connection) -> AppResult<()> {
    let query_only: bool = connection.query_row("PRAGMA query_only", [], |row| row.get(0))?;
    if !query_only {
        return Err(AppError::ApplicationData(
            "The production snapshot connection is not query-only".to_owned(),
        ));
    }
    let write_attempt = connection.execute(
        "CREATE TABLE graphite_experiment_write_probe(id INTEGER)",
        [],
    );
    if write_attempt.is_ok() {
        return Err(AppError::ApplicationData(
            "The production snapshot unexpectedly accepted a write".to_owned(),
        ));
    }
    Ok(())
}

fn active_embedding_profile(connection: &Connection) -> AppResult<SnapshotEmbeddingProfile> {
    connection
        .query_row(
            "SELECT id, artifact_identity, embedding_dimension
             FROM embedding_profiles WHERE active = 1",
            [],
            |row| {
                Ok(SnapshotEmbeddingProfile {
                    id: row.get(0)?,
                    artifact_identity: row.get(1)?,
                    dimension: row.get::<_, i64>(2)? as usize,
                })
            },
        )
        .optional()?
        .ok_or_else(|| {
            AppError::Semantic(
                "The production embedding profile is unavailable for the experiment".to_owned(),
            )
        })
}

fn load_items(connection: &Connection, item_ids: &[String]) -> AppResult<Vec<SnapshotItem>> {
    let mut items = Vec::with_capacity(item_ids.len());
    for item_id in item_ids {
        let row = connection
            .query_row(
                "SELECT sc.item_id, sc.library_id, sc.title, sc.creators,
                        sc.tags, sc.abstract_text, sc.venue,
                        d.id, dr.id, dt.source_version, dt.content,
                        CASE WHEN refs.status = 'references-extracted'
                                  AND refs.section_start IS NOT NULL
                             THEN refs.section_start ELSE length(dt.content) END
                 FROM search_content sc
                 JOIN documents d ON d.research_item_id = sc.item_id
                 JOIN document_texts dt
                   ON dt.document_id = d.id AND dt.is_active = 1
                  AND dt.provider = 'zotero-indexed'
                 JOIN document_revisions dr ON dr.id = (
                     SELECT revision.id FROM document_revisions revision
                     WHERE revision.document_id = d.id
                       AND revision.source_version = dt.source_version
                       AND revision.revision_key LIKE 'text:zotero-indexed:%'
                     ORDER BY revision.id DESC LIMIT 1
                 )
                 LEFT JOIN reference_extractions refs
                   ON refs.document_id = d.id
                  AND refs.document_revision_id = dr.id
                  AND refs.is_current = 1
                 WHERE sc.item_id = ?1
                 ORDER BY CASE WHEN dt.completeness = 'full' THEN 0 ELSE 1 END,
                          d.id LIMIT 1",
                [item_id],
                |row| {
                    let body: String = row.get(10)?;
                    let requested_end = row.get::<_, i64>(11)?.max(0) as usize;
                    Ok(SnapshotItem {
                        item_id: row.get(0)?,
                        library_id: row.get(1)?,
                        title: row.get(2)?,
                        creators: row.get(3)?,
                        tags: row.get(4)?,
                        abstract_text: row.get(5)?,
                        venue: row.get(6)?,
                        document_id: row.get(7)?,
                        document_revision_id: row.get(8)?,
                        source_version: row.get(9)?,
                        body_end_byte: safe_byte_boundary(&body, requested_end),
                        body,
                    })
                },
            )
            .optional()?;
        if let Some(row) = row {
            items.push(row);
        }
    }
    Ok(items)
}

struct ChunkRow {
    id: String,
    item_id: String,
    document_id: String,
    document_revision_id: i64,
    ordinal: u32,
    text: String,
    character_start: u64,
    character_end: u64,
    page_start: Option<u32>,
    page_end: Option<u32>,
    fingerprint: String,
    vector_key: u64,
}

fn load_chunk_rows(
    connection: &Connection,
    profile_id: &str,
    requested: &HashSet<&String>,
) -> AppResult<Vec<ChunkRow>> {
    let mut statement = connection.prepare(
        "SELECT sc.id, sc.item_id, sc.document_id,
                sc.document_revision_id, sc.ordinal, sc.text,
                sc.character_start, sc.character_end,
                sc.page_start, sc.page_end, sc.text_fingerprint,
                sc.vector_key
         FROM semantic_chunks sc
         JOIN semantic_vector_entries vectors
           ON vectors.vector_key = sc.vector_key
         WHERE sc.embedding_profile_id = ?1
           AND sc.chunking_profile_id = 'scientific-paragraph-v1'
           AND sc.status = 'active' AND vectors.active = 1
         ORDER BY sc.item_id, sc.ordinal",
    )?;
    let rows = statement
        .query_map([profile_id], |row| {
            Ok(ChunkRow {
                id: row.get(0)?,
                item_id: row.get(1)?,
                document_id: row.get(2)?,
                document_revision_id: row.get(3)?,
                ordinal: row.get::<_, i64>(4)? as u32,
                text: row.get(5)?,
                character_start: row.get::<_, i64>(6)? as u64,
                character_end: row.get::<_, i64>(7)? as u64,
                page_start: row.get(8)?,
                page_end: row.get(9)?,
                fingerprint: row.get(10)?,
                vector_key: row.get::<_, i64>(11)? as u64,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|row| requested.contains(&row.item_id))
        .collect())
}

struct PaperRow {
    item_id: String,
    representation_kind: String,
    vector_key: u64,
}

fn load_paper_rows(
    connection: &Connection,
    profile_id: &str,
    requested: &HashSet<&String>,
) -> AppResult<Vec<PaperRow>> {
    let mut statement = connection.prepare(
        "SELECT representation.item_id,
                representation.representation_kind,
                representation.vector_key
         FROM paper_semantic_representations representation
         JOIN semantic_vector_entries vectors
           ON vectors.vector_key = representation.vector_key
         WHERE representation.embedding_profile_id = ?1
           AND representation.status = 'active' AND vectors.active = 1
         ORDER BY representation.item_id",
    )?;
    let rows = statement
        .query_map([profile_id], |row| {
            Ok(PaperRow {
                item_id: row.get(0)?,
                representation_kind: row.get(1)?,
                vector_key: row.get::<_, i64>(2)? as u64,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|row| requested.contains(&row.item_id))
        .collect())
}

fn load_shadow_projections(
    connection: &Connection,
    requested: &HashSet<&String>,
) -> AppResult<Vec<SnapshotProjection>> {
    let mut statement = connection.prepare(
        "SELECT document.research_item_id, extraction.artifact_id,
                extraction.artifact_revision_id, extraction.id,
                extraction.provider_profile_id, extraction.canonical_json,
                attempt.provider_role
         FROM shadow_extraction_attempts attempt
         JOIN document_extractions extraction
           ON extraction.id = attempt.extraction_id
         JOIN documents document ON document.id = extraction.artifact_id
         WHERE extraction.canonical_json IS NOT NULL
         ORDER BY extraction.id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut projections = Vec::new();
    for (item_id, document_id, revision_id, extraction_id, profile, json, role) in rows {
        if !requested.contains(&item_id) {
            continue;
        }
        let source_kind = match role.as_str() {
            "native-primary" => SourceKind::Xberg,
            "ocr-fallback" => SourceKind::PdfInspectorOcr,
            "structure-demand" => SourceKind::Docling,
            _ => continue,
        };
        let canonical: CanonicalDocumentV1 = serde_json::from_str(&json)?;
        let content = render_plain_text(&canonical);
        let content_fingerprint = sha256(content.as_bytes());
        projections.push(SnapshotProjection {
            projection_id: sha256(
                format!("{item_id}|{extraction_id}|{}", source_kind.as_str()).as_bytes(),
            ),
            item_id,
            document_id,
            document_revision_id: revision_id,
            source_kind,
            extraction_id: Some(extraction_id),
            provider_profile: profile,
            content,
            content_fingerprint,
        });
    }
    Ok(projections)
}

fn read_vectors(
    path: &Path,
    keys: impl IntoIterator<Item = u64>,
) -> AppResult<HashMap<u64, Vec<f32>>> {
    if !path.is_file() {
        return Err(AppError::Semantic(
            "The production vector index required by the read-only snapshot is missing".to_owned(),
        ));
    }
    let options = IndexOptions {
        dimensions: EMBEDDING_DIMENSION,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        connectivity: 16,
        expansion_add: 64,
        expansion_search: 48,
        ..Default::default()
    };
    let index = Index::new(&options).map_err(vector_error)?;
    index
        .load(path.to_string_lossy().as_ref())
        .map_err(vector_error)?;
    let mut vectors = HashMap::new();
    for key in keys {
        if !index.contains(key) {
            continue;
        }
        let mut vector = vec![0.0_f32; EMBEDDING_DIMENSION];
        if index.get(key, &mut vector).map_err(vector_error)? > 0 {
            vectors.insert(key, vector);
        }
    }
    Ok(vectors)
}

fn safe_byte_boundary(value: &str, requested: usize) -> usize {
    let mut boundary = requested.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn vector_error(error: impl std::fmt::Display) -> AppError {
    AppError::Semantic(format!(
        "The read-only production vector snapshot failed: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ResearchDatabase;

    #[test]
    fn production_snapshot_connection_rejects_writes() {
        let path = std::env::temp_dir().join(format!(
            "graphite-read-only-snapshot-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        drop(ResearchDatabase::initialize(path.clone()).expect("production fixture"));
        let connection = open_read_only(&path).expect("read-only connection");
        assert_query_only(&connection).expect("query-only enforcement");
        assert!(connection
            .execute(
                "INSERT INTO app_meta(key, value) VALUES ('probe', 'write')",
                []
            )
            .is_err());
        std::fs::remove_file(path).expect("remove production fixture");
    }
}
