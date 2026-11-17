use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::semantic::chunking::{scientific_chunks, DocumentSource, ScientificChunk};
use crate::semantic::vector_index::{UsearchVectorIndex, VectorIndex};
use crate::semantic::{embedding_profile_id, EmbeddingProvider};

use super::model::{
    normalized_chunk_text, sha256, ExperimentCancellationToken, ExperimentDefinition,
    ExperimentIndexSummary, ExperimentMode, ProductionSnapshot, SnapshotChunk, SnapshotProjection,
    SourceKind,
};
use super::runtime::ExperimentRuntime;
use super::store::{
    validate_ready_store, ExperimentStore, StoredChunk, StoredProvenance, SummaryRuntimeMetrics,
};

pub(super) struct ExperimentIndexer {
    provider: Arc<dyn EmbeddingProvider>,
}

#[derive(Clone, Debug)]
struct PreparedChunk {
    experiment_id: String,
    logical_chunk_id: String,
    item_id: String,
    document_id: String,
    document_revision_id: i64,
    ordinal: u32,
    text: String,
    character_start: u64,
    character_end: u64,
    page_start: Option<u32>,
    page_end: Option<u32>,
    token_count: u32,
    content_fingerprint: String,
    vector_key: u64,
    projections: Vec<SnapshotProjection>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishedManifest<'a> {
    definition: &'a ExperimentDefinition,
    execution_id: &'a str,
    mode: ExperimentMode,
    state: &'static str,
    item_count: usize,
    projection_count: usize,
    chunk_count: usize,
    paper_vector_count: usize,
    embedded_chunk_count: usize,
}

impl ExperimentIndexer {
    pub(super) fn new(provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self { provider }
    }

    pub(super) fn index_zotero_mirror(
        &self,
        runtime: &ExperimentRuntime,
        definition: &ExperimentDefinition,
        snapshot: &ProductionSnapshot,
        cancellation: &ExperimentCancellationToken,
    ) -> AppResult<ExperimentIndexSummary> {
        let mode = ExperimentMode::ExperimentalZoteroMirror;
        let execution_id = execution_id(&definition.definition_id);
        let published = runtime.recover_incomplete_publication(definition)?;
        if published {
            let existing =
                validate_ready_store(&runtime.database_path(), definition)?.ok_or_else(|| {
                    AppError::ApplicationData(
                        "The published experiment database is unavailable".to_owned(),
                    )
                })?;
            drop(existing);
            let store = ExperimentStore::open(&runtime.database_path())?;
            store.record_idempotent_execution(&execution_id, &definition.definition_id, mode)?;
            return store.summary(
                execution_id,
                mode,
                SummaryRuntimeMetrics {
                    sqlite_bytes: file_bytes(&runtime.database_path()),
                    vector_bytes: directory_bytes(&runtime.vectors_path()),
                    embedded_chunk_count: 0,
                    reused_duplicate_count: 0,
                    idempotent_reuse: true,
                },
            );
        }
        validate_embedding_profile(self.provider.as_ref(), definition, snapshot)?;
        let staging = runtime.reset_staging()?;
        let staged_database = staging.join("experiment.sqlite3");
        let vectors: Arc<dyn VectorIndex> =
            Arc::new(UsearchVectorIndex::new(staging.join("vectors"))?);
        let store = ExperimentStore::initialize(&staged_database, definition, &execution_id, mode)?;
        for item in &snapshot.items {
            if cancellation.is_cancelled() {
                store.cancel(&execution_id)?;
                return Err(cancelled_error());
            }
            store.insert_item(item)?;
        }
        let projections = snapshot
            .items
            .iter()
            .map(zotero_projection)
            .collect::<Vec<_>>();
        for projection in &projections {
            store.insert_projection(projection)?;
        }
        let (prepared, reused_duplicate_count) =
            prepare_mirror_chunks(snapshot, &projections, self.provider.as_ref())?;
        let mut chunks_by_document = HashMap::<String, Vec<(u64, Vec<f32>)>>::new();
        for chunk in &prepared {
            if cancellation.is_cancelled() {
                store.cancel(&execution_id)?;
                return Err(cancelled_error());
            }
            let production = snapshot
                .chunks
                .iter()
                .find(|candidate| candidate.id == chunk.logical_chunk_id)
                .ok_or_else(|| {
                    AppError::Semantic(format!(
                        "The Zotero mirror chunk '{}' has no production counterpart",
                        chunk.logical_chunk_id
                    ))
                })?;
            chunks_by_document
                .entry(chunk.document_id.clone())
                .or_default()
                .push((chunk.vector_key, production.vector.clone()));
        }
        let mut chunk_batches = chunks_by_document.into_values().collect::<Vec<_>>();
        for batch in &mut chunk_batches {
            batch.sort_by_key(|(key, _)| *key);
        }
        chunk_batches.sort_by_key(|batch| batch.first().map(|(key, _)| *key).unwrap_or_default());
        for batch in &chunk_batches {
            vectors.upsert(&definition.embedding_profile_id, "chunk", batch)?;
        }
        for chunk in &prepared {
            store.insert_chunk(StoredChunk {
                id: &chunk.experiment_id,
                logical_chunk_id: &chunk.logical_chunk_id,
                item_id: &chunk.item_id,
                document_id: &chunk.document_id,
                document_revision_id: chunk.document_revision_id,
                ordinal: chunk.ordinal,
                text: &chunk.text,
                character_start: chunk.character_start,
                character_end: chunk.character_end,
                page_start: chunk.page_start,
                page_end: chunk.page_end,
                token_count: chunk.token_count,
                content_fingerprint: &chunk.content_fingerprint,
                vector_key: chunk.vector_key,
            })?;
            for projection in &chunk.projections {
                store.insert_provenance(StoredProvenance {
                    chunk_id: &chunk.experiment_id,
                    projection,
                    page_start: chunk.page_start,
                    page_end: chunk.page_end,
                })?;
            }
        }
        let mut paper_vectors = snapshot
            .paper_vectors
            .iter()
            .map(|paper| (paper.vector_key, paper.vector.clone()))
            .collect::<Vec<_>>();
        paper_vectors.sort_by_key(|(key, _)| *key);
        vectors.upsert(&definition.embedding_profile_id, "paper", &paper_vectors)?;
        for paper in &snapshot.paper_vectors {
            store.insert_paper_vector(
                &paper.item_id,
                &paper.representation_kind,
                paper.vector_key,
            )?;
        }
        store.finish(&execution_id)?;
        let manifest = PublishedManifest {
            definition,
            execution_id: &execution_id,
            mode,
            state: "ready",
            item_count: snapshot.items.len(),
            projection_count: projections.len(),
            chunk_count: prepared.len(),
            paper_vector_count: snapshot.paper_vectors.len(),
            embedded_chunk_count: 0,
        };
        fs::write(
            staging.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest)?,
        )
        .map_err(runtime_error)?;
        drop(store);
        drop(vectors);
        if cancellation.is_cancelled() {
            return Err(cancelled_error());
        }
        runtime.publish_staging()?;
        let store = ExperimentStore::open_read_only(&runtime.database_path())?;
        store.summary(
            execution_id,
            mode,
            SummaryRuntimeMetrics {
                sqlite_bytes: file_bytes(&runtime.database_path()),
                vector_bytes: directory_bytes(&runtime.vectors_path()),
                embedded_chunk_count: 0,
                reused_duplicate_count,
                idempotent_reuse: false,
            },
        )
    }
}

fn validate_embedding_profile(
    provider: &dyn EmbeddingProvider,
    definition: &ExperimentDefinition,
    snapshot: &ProductionSnapshot,
) -> AppResult<()> {
    let metadata = provider.installed_metadata()?.ok_or_else(|| {
        AppError::Semantic(
            "The frozen production BGE model must already be installed for the experiment"
                .to_owned(),
        )
    })?;
    let profile_id = embedding_profile_id(&metadata);
    if profile_id != definition.embedding_profile_id
        || profile_id != snapshot.embedding_profile.id
        || metadata.artifact_identity != definition.embedding_artifact_identity
        || metadata.dimension != snapshot.embedding_profile.dimension
    {
        return Err(AppError::Semantic(
            "The experimental embedding profile differs from production".to_owned(),
        ));
    }
    Ok(())
}

fn zotero_projection(item: &super::model::SnapshotItem) -> SnapshotProjection {
    let content_fingerprint = sha256(item.body.as_bytes());
    SnapshotProjection {
        projection_id: sha256(
            format!(
                "{}|{}|{}|zotero|{}",
                item.item_id, item.document_id, item.document_revision_id, content_fingerprint
            )
            .as_bytes(),
        ),
        item_id: item.item_id.clone(),
        document_id: item.document_id.clone(),
        document_revision_id: item.document_revision_id,
        source_kind: SourceKind::Zotero,
        extraction_id: None,
        provider_profile: "zotero-indexed/source-text-v1".to_owned(),
        content: item.body.clone(),
        content_fingerprint,
    }
}

fn prepare_mirror_chunks(
    snapshot: &ProductionSnapshot,
    projections: &[SnapshotProjection],
    provider: &dyn EmbeddingProvider,
) -> AppResult<(Vec<PreparedChunk>, u64)> {
    let mut prepared: Vec<PreparedChunk> = Vec::new();
    let mut dedup = HashMap::<(String, String), usize>::new();
    let mut reused_duplicate_count = 0_u64;
    for projection in projections {
        let item = snapshot
            .items
            .iter()
            .find(|candidate| candidate.item_id == projection.item_id)
            .ok_or_else(|| {
                AppError::ApplicationData(
                    "The experimental projection has no snapshot item".to_owned(),
                )
            })?;
        let source = DocumentSource {
            item_id: item.item_id.clone(),
            library_id: item.library_id.clone(),
            document_id: item.document_id.clone(),
            document_revision_id: item.document_revision_id,
            provider: "zotero-indexed".to_owned(),
            text: projection.content.clone(),
            body_end_byte: item.body_end_byte,
            bibliography_excluded: item.body_end_byte < item.body.len(),
        };
        let chunks = scientific_chunks(provider, &source)?;
        validate_logical_chunks(snapshot, item, &chunks)?;
        for chunk in chunks {
            let normalized = normalized_chunk_text(&chunk.text);
            let content_fingerprint = sha256(normalized.as_bytes());
            let key = (item.item_id.clone(), content_fingerprint.clone());
            if let Some(existing) = dedup.get(&key).copied() {
                let existing_chunk = &prepared[existing];
                if existing_chunk.text != chunk.text
                    && normalized_chunk_text(&existing_chunk.text) != normalized
                {
                    return Err(AppError::ApplicationData(
                        "The exact experimental deduplication key is inconsistent".to_owned(),
                    ));
                }
                retain_projection(&mut prepared[existing].projections, projection);
                reused_duplicate_count = reused_duplicate_count.saturating_add(1);
                continue;
            }
            let production = snapshot
                .chunks
                .iter()
                .find(|candidate| candidate.id == chunk.id)
                .ok_or_else(|| {
                    AppError::Semantic(format!(
                        "The generated Zotero chunk '{}' is absent from production",
                        chunk.id
                    ))
                })?;
            let experiment_id =
                sha256(format!("{}|{}", item.item_id, content_fingerprint).as_bytes());
            dedup.insert(key, prepared.len());
            prepared.push(prepared_chunk(
                experiment_id,
                projection.clone(),
                chunk,
                production,
                content_fingerprint,
            ));
        }
    }
    Ok((prepared, reused_duplicate_count))
}

fn retain_projection(
    projections: &mut Vec<SnapshotProjection>,
    projection: &SnapshotProjection,
) -> bool {
    if projections
        .iter()
        .any(|candidate| candidate.projection_id == projection.projection_id)
    {
        return false;
    }
    projections.push(projection.clone());
    true
}

fn validate_logical_chunks(
    snapshot: &ProductionSnapshot,
    item: &super::model::SnapshotItem,
    generated: &[ScientificChunk],
) -> AppResult<()> {
    let production = snapshot
        .chunks
        .iter()
        .filter(|chunk| chunk.item_id == item.item_id)
        .collect::<Vec<_>>();
    if production.len() != generated.len() {
        return Err(AppError::Semantic(format!(
            "The Zotero mirror generated {} chunks for '{}' but production has {}",
            generated.len(),
            item.item_id,
            production.len()
        )));
    }
    for (generated, production) in generated.iter().zip(production) {
        if generated.id != production.id
            || generated.ordinal as u32 != production.ordinal
            || generated.text != production.text
            || generated.character_start as u64 != production.character_start
            || generated.character_end as u64 != production.character_end
            || generated.fingerprint != production.fingerprint
        {
            return Err(AppError::Semantic(format!(
                "The Zotero mirror chunk contract differs for item '{}' at ordinal {}",
                item.item_id, generated.ordinal
            )));
        }
    }
    Ok(())
}

fn prepared_chunk(
    experiment_id: String,
    projection: SnapshotProjection,
    chunk: ScientificChunk,
    production: &SnapshotChunk,
    content_fingerprint: String,
) -> PreparedChunk {
    PreparedChunk {
        experiment_id,
        logical_chunk_id: chunk.id,
        item_id: projection.item_id.clone(),
        document_id: projection.document_id.clone(),
        document_revision_id: projection.document_revision_id,
        ordinal: chunk.ordinal as u32,
        text: chunk.text,
        character_start: chunk.character_start as u64,
        character_end: chunk.character_end as u64,
        page_start: production.page_start,
        page_end: production.page_end,
        token_count: chunk.token_count as u32,
        content_fingerprint,
        vector_key: production.vector_key,
        projections: vec![projection],
    }
}

#[cfg(test)]
pub(super) fn dedup_identity(item_id: &str, text: &str) -> String {
    sha256(format!("{item_id}|{}", normalized_chunk_text(text)).as_bytes())
}

fn execution_id(definition_id: &str) -> String {
    sha256(
        format!(
            "{definition_id}|{}|{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )
        .as_bytes(),
    )
}

fn file_bytes(path: &Path) -> u64 {
    path.metadata().map(|metadata| metadata.len()).unwrap_or(0)
}

fn directory_bytes(path: &Path) -> u64 {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                directory_bytes(&path)
            } else {
                file_bytes(&path)
            }
        })
        .sum()
}

fn cancelled_error() -> AppError {
    AppError::Semantic("The experimental index build was cancelled".to_owned())
}

fn runtime_error(error: std::io::Error) -> AppError {
    AppError::ApplicationData(format!(
        "The experiment manifest could not be written: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_deduplication_normalizes_whitespace_but_not_words() {
        assert_eq!(
            dedup_identity("paper", "alpha  beta\ngamma"),
            dedup_identity("paper", "alpha beta gamma")
        );
        assert_ne!(
            dedup_identity("paper", "alpha beta gamma"),
            dedup_identity("paper", "alpha beta delta")
        );
        assert_ne!(
            dedup_identity("paper-a", "alpha beta"),
            dedup_identity("paper-b", "alpha beta")
        );
    }

    #[test]
    fn duplicate_chunks_retain_each_source_projection() {
        let mut projections = vec![projection("zotero", SourceKind::Zotero)];
        assert!(retain_projection(
            &mut projections,
            &projection("xberg", SourceKind::Xberg)
        ));
        assert!(!retain_projection(
            &mut projections,
            &projection("xberg", SourceKind::Xberg)
        ));
        assert_eq!(projections.len(), 2);
        assert_eq!(projections[0].source_kind, SourceKind::Zotero);
        assert_eq!(projections[1].source_kind, SourceKind::Xberg);
    }

    fn projection(id: &str, source_kind: SourceKind) -> SnapshotProjection {
        SnapshotProjection {
            projection_id: id.to_owned(),
            item_id: "item".to_owned(),
            document_id: "document".to_owned(),
            document_revision_id: 1,
            source_kind,
            extraction_id: None,
            provider_profile: id.to_owned(),
            content: "identical text".to_owned(),
            content_fingerprint: "fingerprint".to_owned(),
        }
    }
}
