use std::cmp::Ordering as CompareOrdering;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use chrono::Utc;
use fastembed::{
    EmbeddingModel, InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding,
    TextInitOptions, TokenizerFiles, UserDefinedEmbeddingModel,
};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

pub(crate) mod chunking;
mod model_lifecycle;
mod passage_search;
pub(crate) mod retrieval_policy;
pub(crate) mod topics;
pub(crate) mod vector_index;

use chunking::{
    scientific_chunks, DocumentSource, ScientificChunk, BOUNDARY_POLICY, CHUNKING_PROFILE_ID,
    CHUNKING_STRATEGY, CHUNKING_VERSION, MAXIMUM_CHUNKS, MAXIMUM_TOKENS, OVERLAP_TOKENS,
    TARGET_TOKENS,
};
use model_lifecycle::{ModelInstallationMarker, ModelLifecycle, MODEL_FILE};
use retrieval_policy::{hybrid_fusion_score, semantic_passage_score, RRF_K, RRF_LEXICAL_WEIGHT};
use vector_index::{UsearchVectorIndex, VectorIndex};

use crate::agent::answer_context::{
    COMPACT_EVIDENCE_BUDGET_TOKENS, CONTEXT_ASSEMBLER_VERSION, LARGE_EVIDENCE_BUDGET_TOKENS,
    MAX_EVIDENCE, MAX_PASSAGES_PER_PAPER, TOKEN_ACCOUNTING_PROFILE,
};
use crate::database::ResearchDatabase;
use crate::error::{AppError, AppResult};
use crate::model::{
    ChunkingProfileSummary, CitationItemSummary, EmbeddingProfileSummary, GraphEdge,
    PaperSemanticCoverage, ResearchItem, ResearchNeighborhood, RetrievalProfileSummary, SearchPage,
    SearchRequest, SearchResult, SemanticDiagnostics, SemanticIndexRequest, SemanticPassage,
    SemanticRelationshipEvidence, SimilarPaper, SimilarPaperPage, SimilarPapersRequest,
};

pub const DEFAULT_MODEL_ID: &str = "Qdrant/bge-small-en-v1.5-onnx-Q";
pub const DEFAULT_MODEL_NAME: &str = "BGE small English v1.5 (quantized)";
pub const DEFAULT_MODEL_SOURCE: &str = "Hugging Face / Qdrant ONNX artifact";
pub const DEFAULT_MODEL_LICENSE: &str = "MIT model / Apache-2.0 runtime";
pub const DEFAULT_MODEL_DOWNLOAD_BYTES: u64 = 35_000_000;
pub const EMBEDDING_DIMENSION: usize = 384;
pub const EMBEDDING_RUNTIME_VERSION: &str = "fastembed-5.17.4/ort-2.0.0-rc.13";
pub const VECTOR_BACKEND: &str = "usearch-hnsw-2.26.0";
pub const VECTOR_FORMAT_VERSION: &str = "usearch-dense-v1";
pub const PAPER_REPRESENTATION_VERSION: &str = "paper-title-abstract-v1";
pub const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";
pub const DOCUMENT_PREFIX: &str = "";
const DEFAULT_SIMILARITY_THRESHOLD: f64 = 0.52;
const MAX_RETRIEVAL_CANDIDATES: usize = 180;

#[derive(Clone, Debug)]
pub struct EmbeddingProviderMetadata {
    pub provider: String,
    pub model_identifier: String,
    pub model_revision: String,
    pub artifact_identity: String,
    pub dimension: usize,
    pub metric: String,
    pub query_template: String,
    pub document_template: String,
    pub normalization: String,
    pub runtime_version: String,
    pub installed_bytes: u64,
}

pub trait EmbeddingProvider: Send + Sync {
    fn installed_metadata(&self) -> AppResult<Option<EmbeddingProviderMetadata>>;
    fn install(&self) -> AppResult<EmbeddingProviderMetadata>;
    fn embed_documents(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>>;
    fn embed_query(&self, query: &str) -> AppResult<Vec<f32>>;
    fn token_count(&self, text: &str) -> AppResult<usize>;
}

#[derive(Clone)]
pub struct LocalFastEmbedProvider {
    lifecycle: ModelLifecycle,
    model: Arc<StdMutex<Option<TextEmbedding>>>,
    tokenizer: Arc<StdMutex<Option<Tokenizer>>>,
}

impl LocalFastEmbedProvider {
    pub fn new(cache_dir: PathBuf) -> AppResult<Self> {
        Ok(Self {
            lifecycle: ModelLifecycle::initialize(cache_dir)?,
            model: Arc::new(StdMutex::new(None)),
            tokenizer: Arc::new(StdMutex::new(None)),
        })
    }

    fn load_local_assets(&self, marker: &ModelInstallationMarker) -> AppResult<()> {
        let mut model_guard = self
            .model
            .lock()
            .map_err(|_| AppError::Semantic("The embedding model lock was poisoned".to_owned()))?;
        if model_guard.is_some() {
            return Ok(());
        }
        let snapshot = self.lifecycle.validate_marker(marker)?;
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: ModelLifecycle::read_asset(&snapshot, "tokenizer.json")?,
            config_file: ModelLifecycle::read_asset(&snapshot, "config.json")?,
            special_tokens_map_file: ModelLifecycle::read_asset(
                &snapshot,
                "special_tokens_map.json",
            )?,
            tokenizer_config_file: ModelLifecycle::read_asset(&snapshot, "tokenizer_config.json")?,
        };
        let onnx = ModelLifecycle::read_asset(&snapshot, MODEL_FILE)?;
        let local_model = UserDefinedEmbeddingModel::new(onnx, tokenizer_files)
            .with_pooling(Pooling::Cls)
            .with_quantization(QuantizationMode::Static);
        let model = TextEmbedding::try_new_from_user_defined(
            local_model,
            InitOptionsUserDefined::new()
                .with_max_length(512)
                .with_intra_threads(2),
        )
        .map_err(|error| {
            AppError::Semantic(format!(
                "The local ONNX embedding model could not load: {error}"
            ))
        })?;
        let tokenizer = Tokenizer::from_file(snapshot.join("tokenizer.json")).map_err(|error| {
            AppError::Semantic(format!("The model tokenizer could not load: {error}"))
        })?;
        *model_guard = Some(model);
        *self
            .tokenizer
            .lock()
            .map_err(|_| AppError::Semantic("The tokenizer lock was poisoned".to_owned()))? =
            Some(tokenizer);
        Ok(())
    }

    fn marker_metadata(marker: &ModelInstallationMarker) -> EmbeddingProviderMetadata {
        EmbeddingProviderMetadata {
            provider: "fastembed-onnx-cpu".to_owned(),
            model_identifier: marker.model_identifier.clone(),
            model_revision: marker.artifact_identity.clone(),
            artifact_identity: marker.artifact_identity.clone(),
            dimension: EMBEDDING_DIMENSION,
            metric: "cosine".to_owned(),
            query_template: format!("{QUERY_PREFIX}{{query}}"),
            document_template: "{document}".to_owned(),
            normalization: "l2-unit".to_owned(),
            runtime_version: EMBEDDING_RUNTIME_VERSION.to_owned(),
            installed_bytes: marker.installed_bytes,
        }
    }
}

impl EmbeddingProvider for LocalFastEmbedProvider {
    fn installed_metadata(&self) -> AppResult<Option<EmbeddingProviderMetadata>> {
        self.lifecycle
            .read_marker()
            .map(|value| value.as_ref().map(Self::marker_metadata))
    }

    fn install(&self) -> AppResult<EmbeddingProviderMetadata> {
        if let Some(metadata) = self.installed_metadata()? {
            return Ok(metadata);
        }
        let options = TextInitOptions::new(EmbeddingModel::BGESmallENV15Q)
            .with_cache_dir(self.lifecycle.root().to_path_buf())
            .with_show_download_progress(true)
            .with_max_length(512)
            .with_intra_threads(2);
        let model = TextEmbedding::try_new(options).map_err(|error| {
            AppError::Semantic(format!(
                "The explicit model installation failed. Check the internet connection and try again: {error}"
            ))
        })?;
        let snapshot = self.lifecycle.discover_snapshot()?;
        let marker = self.lifecycle.finalize_downloaded_snapshot(&snapshot)?;
        *self.model.lock().map_err(|_| {
            AppError::Semantic("The embedding model lock was poisoned".to_owned())
        })? = Some(model);
        let tokenizer = Tokenizer::from_file(snapshot.join("tokenizer.json")).map_err(|error| {
            AppError::Semantic(format!("The model tokenizer could not load: {error}"))
        })?;
        *self
            .tokenizer
            .lock()
            .map_err(|_| AppError::Semantic("The tokenizer lock was poisoned".to_owned()))? =
            Some(tokenizer);
        Ok(Self::marker_metadata(&marker))
    }

    fn embed_documents(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        let marker = self
            .lifecycle
            .read_marker()?
            .ok_or_else(model_missing_error)?;
        self.load_local_assets(&marker)?;
        let inputs = texts
            .iter()
            .map(|text| format!("{DOCUMENT_PREFIX}{text}"))
            .collect::<Vec<_>>();
        let mut guard = self
            .model
            .lock()
            .map_err(|_| AppError::Semantic("The embedding model lock was poisoned".to_owned()))?;
        let model = guard.as_mut().ok_or_else(model_missing_error)?;
        let vectors = model.embed(inputs, Some(16)).map_err(|error| {
            AppError::Semantic(format!("Local document embedding failed: {error}"))
        })?;
        normalize_vectors(vectors)
    }

    fn embed_query(&self, query: &str) -> AppResult<Vec<f32>> {
        let vectors = self.embed_documents(&[format!("{QUERY_PREFIX}{}", query.trim())])?;
        vectors.into_iter().next().ok_or_else(|| {
            AppError::Semantic("The local model returned no query vector".to_owned())
        })
    }

    fn token_count(&self, text: &str) -> AppResult<usize> {
        let marker = self
            .lifecycle
            .read_marker()?
            .ok_or_else(model_missing_error)?;
        self.load_local_assets(&marker)?;
        let guard = self
            .tokenizer
            .lock()
            .map_err(|_| AppError::Semantic("The tokenizer lock was poisoned".to_owned()))?;
        let tokenizer = guard.as_ref().ok_or_else(model_missing_error)?;
        tokenizer
            .encode(text, true)
            .map(|encoding| encoding.len())
            .map_err(|error| AppError::Semantic(format!("Token counting failed: {error}")))
    }
}

#[derive(Clone)]
pub struct SemanticCoordinator {
    database: ResearchDatabase,
    provider: Arc<dyn EmbeddingProvider>,
    vectors: Arc<dyn VectorIndex>,
    operation_guard: Arc<StdMutex<()>>,
    cancel_requested: Arc<AtomicBool>,
}

impl SemanticCoordinator {
    pub fn new(
        database: ResearchDatabase,
        model_cache_dir: PathBuf,
        vector_index_dir: PathBuf,
    ) -> AppResult<Self> {
        let coordinator = Self {
            database,
            provider: Arc::new(LocalFastEmbedProvider::new(model_cache_dir)?),
            vectors: Arc::new(UsearchVectorIndex::new(vector_index_dir)?),
            operation_guard: Arc::new(StdMutex::new(())),
            cancel_requested: Arc::new(AtomicBool::new(false)),
        };
        coordinator.recover_interrupted_indexing()?;
        Ok(coordinator)
    }

    /// Refresh interrupted work before this coordinator accepts operations.
    fn recover_interrupted_indexing(&self) -> AppResult<()> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT library_id, embedding_profile_id FROM semantic_index_state
             WHERE status = 'indexing' AND embedding_profile_id IS NOT NULL",
        )?;
        let interrupted = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (library_id, profile_id) in interrupted {
            refresh_semantic_state(
                &connection,
                &library_id,
                &profile_id,
                self.vectors.storage_bytes(&profile_id),
                true,
                Some("Indexing was interrupted. Saved passages were retained. Continue the next semantic batch in library setup."),
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn with_components(
        database: ResearchDatabase,
        provider: Arc<dyn EmbeddingProvider>,
        vectors: Arc<dyn VectorIndex>,
    ) -> Self {
        Self {
            database,
            provider,
            vectors,
            operation_guard: Arc::new(StdMutex::new(())),
            cancel_requested: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn load_workspace(&self) -> AppResult<crate::model::WorkspacePayload> {
        let workspace = self.database.load_workspace()?;
        self.enrich_workspace(workspace)
    }

    pub(crate) fn enrich_workspace(
        &self,
        mut workspace: crate::model::WorkspacePayload,
    ) -> AppResult<crate::model::WorkspacePayload> {
        let Some(profile) = workspace.semantic.active_profile.as_ref() else {
            return Ok(workspace);
        };
        let profile_id = profile.id.clone();
        if workspace.semantic.indexed_papers > 0 {
            if let Ok(_guard) = self.operation_guard.try_lock() {
                if let Some(library) = workspace.snapshot.active_library_id.as_deref() {
                    if topics::refresh(
                        &self.database.connect()?,
                        library,
                        &profile_id,
                        self.vectors.as_ref(),
                        &self.cancel_requested,
                    )? {
                        workspace = self.database.load_workspace()?;
                    }
                }
            }
        }
        let profile = workspace
            .semantic
            .active_profile
            .as_ref()
            .expect("active profile retained");
        let physical_paper_vectors = self.vectors.entry_count(&profile.id, "paper")?;
        let physical_chunk_vectors = self.vectors.entry_count(&profile.id, "chunk")?;
        if physical_paper_vectors < workspace.semantic.registered_paper_vectors {
            if let Some(library) = workspace.snapshot.active_library_id.as_deref() {
                self.database.connect()?.execute(
                    "DELETE FROM app_meta WHERE key=?1",
                    [format!("semantic-topics-v1:{library}")],
                )?;
                workspace = self.database.load_workspace()?;
            }
        }

        workspace.semantic.physical_paper_vectors = Some(physical_paper_vectors);
        workspace.semantic.paper_vector_count_matches =
            Some(workspace.semantic.registered_paper_vectors == physical_paper_vectors);
        workspace.semantic.physical_chunk_vectors = Some(physical_chunk_vectors);
        workspace.semantic.chunk_vector_count_matches =
            Some(workspace.semantic.registered_chunk_vectors == physical_chunk_vectors);
        workspace.semantic.vector_index_bytes = self.vectors.storage_bytes(&profile_id);
        Ok(workspace)
    }

    pub fn install_model(&self) -> AppResult<crate::model::WorkspacePayload> {
        let metadata = self.provider.install()?;
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        let profile_id = embedding_profile_id(&metadata);
        let previous_active = active_profile(&transaction)?.map(|profile| profile.id);
        transaction.execute(
            "INSERT INTO semantic_model_installations (
                model_identifier, provider, runtime_version,
                artifact_identity, cache_relative_path, installed_bytes,
                license, status, installed_at, last_checked_at, last_error
             ) VALUES (?1, ?2, ?3, ?4, 'semantic-models', ?5, ?6,
                       'installed', ?7, ?7, NULL)
             ON CONFLICT(model_identifier) DO UPDATE SET
                provider = excluded.provider,
                runtime_version = excluded.runtime_version,
                artifact_identity = excluded.artifact_identity,
                installed_bytes = excluded.installed_bytes,
                license = excluded.license,
                status = 'installed',
                last_checked_at = excluded.last_checked_at,
                last_error = NULL",
            params![
                metadata.model_identifier,
                metadata.provider,
                metadata.runtime_version,
                metadata.artifact_identity,
                metadata.installed_bytes as i64,
                DEFAULT_MODEL_LICENSE,
                now,
            ],
        )?;
        transaction.execute(
            "INSERT INTO embedding_profiles (
                id, provider, model_identifier, model_revision,
                artifact_identity, embedding_dimension, similarity_metric,
                query_template, document_template, normalization,
                runtime_version, vector_backend,
                vector_index_format_version, active, created_at, activated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, 0, ?14, NULL
             ) ON CONFLICT(id) DO NOTHING",
            params![
                profile_id,
                metadata.provider,
                metadata.model_identifier,
                metadata.model_revision,
                metadata.artifact_identity,
                metadata.dimension as i64,
                metadata.metric,
                metadata.query_template,
                metadata.document_template,
                metadata.normalization,
                metadata.runtime_version,
                VECTOR_BACKEND,
                VECTOR_FORMAT_VERSION,
                now,
            ],
        )?;
        transaction.execute("UPDATE embedding_profiles SET active = 0", [])?;
        transaction.execute(
            "UPDATE embedding_profiles
             SET active = 1, activated_at = ?2 WHERE id = ?1",
            params![profile_id, now],
        )?;
        if let Some(library_id) = active_library_id(&transaction)? {
            if previous_active.as_deref() == Some(profile_id.as_str()) {
                transaction.execute(
                    "INSERT INTO semantic_index_state (
                        library_id, embedding_profile_id, status
                     ) VALUES (?1, ?2, 'ready')
                     ON CONFLICT(library_id) DO UPDATE SET
                        embedding_profile_id = excluded.embedding_profile_id,
                        last_error = NULL",
                    params![library_id, profile_id],
                )?;
            } else {
                transaction.execute(
                    "INSERT INTO semantic_index_state (
                        library_id, embedding_profile_id, status
                     ) VALUES (?1, ?2, 'ready')
                     ON CONFLICT(library_id) DO UPDATE SET
                        embedding_profile_id = excluded.embedding_profile_id,
                        status = 'ready', eligible_papers = 0,
                        indexed_papers = 0, stale_papers = 0,
                        eligible_documents = 0, chunked_documents = 0,
                        generated_chunks = 0, indexed_chunks = 0,
                        stale_chunks = 0, metadata_only_papers = 0,
                        vector_index_bytes = 0, chunk_metadata_bytes = 0,
                        last_indexed_at = NULL, last_error = NULL",
                    params![library_id, profile_id],
                )?;
            }
        }
        transaction.commit()?;
        self.load_workspace()
    }

    pub fn cancel_indexing(&self) {
        self.cancel_requested.store(true, Ordering::Release);
    }

    pub fn rebuild(&self) -> AppResult<crate::model::WorkspacePayload> {
        let connection = self.database.connect()?;
        let profile = active_profile(&connection)?.ok_or_else(model_missing_error)?;
        let Some(library_id) = active_library_id(&connection)? else {
            return self.load_workspace();
        };
        connection.execute(
            "UPDATE semantic_vector_entries SET active = 0, updated_at = ?3
             WHERE embedding_profile_id = ?1 AND library_id = ?2",
            params![profile.id, library_id, Utc::now().to_rfc3339()],
        )?;
        self.vectors.rebuild(&profile.id)?;
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM paper_semantic_representations
             WHERE embedding_profile_id = ?1 AND library_id = ?2",
            params![profile.id, library_id],
        )?;
        transaction.execute(
            "DELETE FROM semantic_chunks
             WHERE embedding_profile_id = ?1 AND library_id = ?2",
            params![profile.id, library_id],
        )?;
        transaction.execute(
            "DELETE FROM semantic_neighbor_cache
             WHERE embedding_profile_id = ?1
               AND (source_item_id IN (
                    SELECT id FROM research_items WHERE library_id = ?2
               ) OR target_item_id IN (
                    SELECT id FROM research_items WHERE library_id = ?2
               ))",
            params![profile.id, library_id],
        )?;
        transaction.execute(
            "DELETE FROM semantic_vector_entries
             WHERE embedding_profile_id = ?1 AND library_id = ?2",
            params![profile.id, library_id],
        )?;
        transaction.execute(
            "UPDATE semantic_index_state SET
                status = 'ready', indexed_papers = 0, stale_papers = 0,
                chunked_documents = 0, generated_chunks = 0,
                indexed_chunks = 0, stale_chunks = 0,
                vector_index_bytes = 0, chunk_metadata_bytes = 0,
                last_indexed_at = NULL, last_error = NULL
             WHERE library_id = ?1",
            [&library_id],
        )?;
        transaction.commit()?;
        self.load_workspace()
    }

    pub fn index(
        &self,
        request: &SemanticIndexRequest,
    ) -> AppResult<crate::model::WorkspacePayload> {
        let _guard = self
            .operation_guard
            .try_lock()
            .map_err(|_| AppError::Semantic("Semantic indexing is already running".to_owned()))?;
        self.cancel_requested.store(false, Ordering::Release);
        let connection = self.database.connect()?;
        let profile = active_profile(&connection)?.ok_or_else(model_missing_error)?;
        let Some(library_id) = active_library_id(&connection)? else {
            return self.load_workspace();
        };
        if self.provider.installed_metadata()?.is_none() {
            return Err(model_missing_error());
        }
        connection.execute(
            "INSERT INTO semantic_index_state (
                library_id, embedding_profile_id, status
             ) VALUES (?1, ?2, 'indexing')
             ON CONFLICT(library_id) DO UPDATE SET
                embedding_profile_id = excluded.embedding_profile_id,
                status = 'indexing', last_error = NULL",
            params![library_id, profile.id],
        )?;

        let outcome = self.run_index_batch(
            &profile,
            &library_id,
            request.max_papers.unwrap_or(24).clamp(1, 64) as usize,
            request.max_documents.unwrap_or(6).clamp(1, 16) as usize,
        );
        if let Err(error) = &outcome {
            record_semantic_error(
                &self.database.connect()?,
                &library_id,
                Some(&profile.id),
                "index-run",
                None,
                &error.to_string(),
            )?;
        }
        refresh_semantic_state(
            &self.database.connect()?,
            &library_id,
            &profile.id,
            self.vectors.storage_bytes(&profile.id),
            self.cancel_requested.load(Ordering::Acquire),
            outcome.as_ref().err().map(ToString::to_string).as_deref(),
        )?;
        outcome?;
        if !self.cancel_requested.load(Ordering::Acquire) {
            topics::refresh(
                &connection,
                &library_id,
                &profile.id,
                self.vectors.as_ref(),
                &self.cancel_requested,
            )?;
        }
        self.load_workspace()
    }

    pub fn search(&self, request: &SearchRequest) -> AppResult<SearchPage> {
        self.search_with_passage_order(request, false)
    }

    /// Retrieve keyword-led complements without changing ordinary hybrid search.
    pub(crate) fn search_complement(&self, request: &SearchRequest) -> AppResult<SearchPage> {
        self.search_with_passage_order(request, true)
    }

    fn search_with_passage_order(
        &self,
        request: &SearchRequest,
        prefer_lexical: bool,
    ) -> AppResult<SearchPage> {
        let mode = retrieval_mode(request.mode.as_deref());
        if mode == "keyword" {
            return self.database.search(request);
        }
        if request.query.trim().is_empty() {
            return empty_semantic_page(&self.database, request, mode);
        }
        let connection = self.database.connect()?;
        let profile = active_profile(&connection)?.ok_or_else(model_missing_error)?;
        let library_id = active_library_id(&connection)?.ok_or_else(|| {
            AppError::Semantic("Select a Zotero library before semantic search".to_owned())
        })?;
        if self.provider.installed_metadata()?.is_none() {
            return Err(model_missing_error());
        }
        let query_vector = self.provider.embed_query(request.query.trim())?;
        let semantic_candidates = self.semantic_candidates(
            &connection,
            &profile,
            &library_id,
            request,
            &query_vector,
            prefer_lexical,
        )?;
        let diagnostics = load_semantic_diagnostics(&connection, Some(&library_id), None)?;
        let mut page = if mode == "hybrid" {
            self.hybrid_page(request, semantic_candidates, &diagnostics)?
        } else {
            self.semantic_page(request, semantic_candidates, &diagnostics)?
        };
        page.corpus_status = connection
            .query_row(
                "SELECT status FROM fulltext_sync_state WHERE library_id = ?1",
                [&library_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_else(|| "not-indexed".to_owned());
        Ok(page)
    }

    pub fn similar_papers(&self, request: &SimilarPapersRequest) -> AppResult<SimilarPaperPage> {
        if request.seed_item_ids.is_empty() {
            return Err(AppError::Semantic(
                "Select at least one indexed paper before finding similar papers".to_owned(),
            ));
        }
        let connection = self.database.connect()?;
        let profile = active_profile(&connection)?.ok_or_else(model_missing_error)?;
        let library_id = active_library_id(&connection)?.ok_or_else(|| {
            AppError::Semantic("Select a Zotero library before finding similar papers".to_owned())
        })?;
        let paper_targets = load_paper_targets(&connection, &profile.id, &library_id)?;
        let mut seed_vectors = Vec::new();
        for seed_id in &request.seed_item_ids {
            let Some(target) = paper_targets
                .values()
                .find(|target| &target.item_id == seed_id)
            else {
                continue;
            };
            if let Some(vector) = self
                .vectors
                .vector(&profile.id, "paper", target.vector_key)?
            {
                seed_vectors.push(vector);
            }
        }
        if seed_vectors.is_empty() {
            return Err(AppError::Semantic(
                "The selected papers have not been indexed semantically yet".to_owned(),
            ));
        }
        let centroid = vector_centroid(&seed_vectors)?;
        let allowed_item_ids = filtered_item_ids(
            &connection,
            &library_id,
            ItemFilters {
                requested_item_ids: &[],
                collection_id: request.collection_id.as_deref(),
                ..ItemFilters::default()
            },
        )?;
        let seed_ids = request
            .seed_item_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        let allowed_keys = paper_targets
            .iter()
            .filter(|(_, target)| {
                allowed_item_ids.contains(&target.item_id) && !seed_ids.contains(&target.item_id)
            })
            .map(|(key, _)| *key)
            .collect::<HashSet<_>>();
        let limit = request.limit.unwrap_or(24).clamp(1, 80) as usize;
        let threshold = request
            .minimum_similarity
            .unwrap_or(DEFAULT_SIMILARITY_THRESHOLD)
            .clamp(-1.0, 1.0);
        let matches = self.vectors.nearest(
            &profile.id,
            "paper",
            &centroid,
            limit.saturating_mul(3),
            &allowed_keys,
        )?;
        let chosen = matches
            .into_iter()
            .filter(|matched| matched.similarity >= threshold)
            .take(limit)
            .collect::<Vec<_>>();
        let item_ids = chosen
            .iter()
            .filter_map(|matched| paper_targets.get(&matched.key))
            .map(|target| target.item_id.clone())
            .collect::<Vec<_>>();
        let items = self
            .database
            .items_by_ids(&item_ids)?
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect::<HashMap<_, _>>();
        let mut results = Vec::new();
        for matched in chosen {
            let Some(target) = paper_targets.get(&matched.key) else {
                continue;
            };
            let Some(item) = items.get(&target.item_id).cloned() else {
                continue;
            };
            results.push(SimilarPaper {
                item,
                similarity_score: matched.similarity,
                similarity_label: similarity_label(matched.similarity).to_owned(),
                source_representation: if seed_vectors.len() > 1 {
                    "centroid of selected paper representations".to_owned()
                } else {
                    "selected paper representation".to_owned()
                },
                target_representation: target.representation_kind.clone(),
                embedding_profile: profile.clone(),
            });
        }
        let diagnostics = load_semantic_diagnostics(&connection, Some(&library_id), None)?;
        Ok(SimilarPaperPage {
            results,
            seed_item_ids: request.seed_item_ids.clone(),
            indexed_papers: diagnostics.indexed_papers,
            eligible_papers: diagnostics.eligible_papers,
        })
    }

    pub fn neighborhood_scoped(
        &self,
        seed_item_ids: &[String],
        max_nodes: Option<u32>,
        max_edges: Option<u32>,
        collection_id: Option<&str>,
    ) -> AppResult<ResearchNeighborhood> {
        let node_limit = max_nodes.unwrap_or(48).clamp(2, 120) as usize;
        let similar = self.similar_papers(&SimilarPapersRequest {
            seed_item_ids: seed_item_ids.to_vec(),
            collection_id: collection_id.map(str::to_owned),
            limit: Some(node_limit.saturating_sub(seed_item_ids.len()) as u32),
            minimum_similarity: Some(DEFAULT_SIMILARITY_THRESHOLD),
        })?;
        let mut item_ids = seed_item_ids.to_vec();
        item_ids.extend(similar.results.iter().map(|result| result.item.id.clone()));
        item_ids.sort();
        item_ids.dedup();
        let mut projection = self.database.projection_for_item_ids(&item_ids)?;
        let profile = self.active_profile_required()?;
        let connection = self.database.connect()?;
        let targets = load_paper_targets(
            &connection,
            &profile.id,
            &active_library_id(&connection)?.unwrap_or_default(),
        )?;
        let edge_limit = max_edges.unwrap_or(100).clamp(1, 300) as usize;
        let mut semantic_edges = Vec::new();
        for seed_id in seed_item_ids {
            let Some(seed) = targets.values().find(|target| &target.item_id == seed_id) else {
                continue;
            };
            let Some(seed_vector) = self.vectors.vector(&profile.id, "paper", seed.vector_key)?
            else {
                continue;
            };
            for result in &similar.results {
                if semantic_edges.len() >= edge_limit {
                    break;
                }
                let Some(target) = targets
                    .values()
                    .find(|target| target.item_id == result.item.id)
                else {
                    continue;
                };
                let Some(target_vector) =
                    self.vectors
                        .vector(&profile.id, "paper", target.vector_key)?
                else {
                    continue;
                };
                let score = cosine_similarity(&seed_vector, &target_vector)?;
                if score < DEFAULT_SIMILARITY_THRESHOLD {
                    continue;
                }
                let edge_id =
                    persist_semantic_edge(&connection, &profile, seed, target, score, false)?;
                semantic_edges.push(GraphEdge {
                    id: edge_id,
                    source: seed.item_id.clone(),
                    target: target.item_id.clone(),
                    relation: "semantically-similar".to_owned(),
                    weight: score,
                    directed: false,
                    evidence_count: 1,
                    evidence_label: Some(format!(
                        "{} / cosine {:.3}",
                        similarity_label(score),
                        score
                    )),
                });
            }
        }
        let total_relationships = semantic_edges.len() as u64;
        projection.graph.edges.extend(semantic_edges);
        if total_relationships > 0
            && !projection
                .graph
                .available_relations
                .iter()
                .any(|relation| relation == "semantically-similar")
        {
            projection
                .graph
                .available_relations
                .push("semantically-similar".to_owned());
        }
        Ok(ResearchNeighborhood {
            projection,
            mode: "semantic".to_owned(),
            total_relationships,
            hidden_relationships: 0,
        })
    }

    pub fn paper_coverage(&self, item_id: &str) -> AppResult<PaperSemanticCoverage> {
        let connection = self.database.connect()?;
        let profile = active_profile(&connection)?;
        let (paper_status, representation, coverage) = if let Some(profile) = &profile {
            connection
                .query_row(
                    "SELECT status, representation_kind, coverage
                     FROM paper_semantic_representations
                     WHERE item_id = ?1 AND embedding_profile_id = ?2",
                    params![item_id, profile.id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?
                .map(|row| (row.0, Some(row.1), Some(row.2)))
                .unwrap_or_else(|| ("not-indexed".to_owned(), None, None))
        } else {
            ("model-missing".to_owned(), None, None)
        };
        let (document_count, chunk_count, indexed_chunk_count) = connection.query_row(
            "SELECT
                (SELECT COUNT(*) FROM documents WHERE research_item_id = ?1),
                (SELECT COUNT(*) FROM semantic_chunks WHERE item_id = ?1),
                (SELECT COUNT(*) FROM semantic_chunks
                 WHERE item_id = ?1 AND status = 'active')",
            [item_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                ))
            },
        )?;
        Ok(PaperSemanticCoverage {
            item_id: item_id.to_owned(),
            paper_status,
            paper_representation: representation,
            paper_coverage: coverage,
            document_count,
            chunk_count,
            indexed_chunk_count,
            embedding_profile: profile,
        })
    }

    pub fn relationship_evidence(
        &self,
        edge_id: &str,
    ) -> AppResult<Option<SemanticRelationshipEvidence>> {
        let connection = self.database.connect()?;
        let profile = active_profile(&connection)?.ok_or_else(model_missing_error)?;
        connection
            .query_row(
                "SELECT source_item_id, target_item_id, source_representation,
                        target_representation, similarity_metric,
                        similarity_score, mutual
                 FROM semantic_neighbor_cache
                 WHERE edge_id = ?1 AND embedding_profile_id = ?2",
                params![edge_id, profile.id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, f64>(5)?,
                        row.get::<_, bool>(6)?,
                    ))
                },
            )
            .optional()?
            .map(|row| {
                Ok(SemanticRelationshipEvidence {
                    edge_id: edge_id.to_owned(),
                    relation: "semantically-similar".to_owned(),
                    source: citation_item_summary(&connection, &row.0)?,
                    target: citation_item_summary(&connection, &row.1)?,
                    source_representation: row.2,
                    target_representation: row.3,
                    similarity_metric: row.4,
                    similarity_score: row.5,
                    similarity_label: similarity_label(row.5).to_owned(),
                    mutual: row.6,
                    embedding_profile: profile,
                    passages: Vec::new(),
                })
            })
            .transpose()
    }

    fn active_profile_required(&self) -> AppResult<EmbeddingProfileSummary> {
        active_profile(&self.database.connect()?)?.ok_or_else(model_missing_error)
    }

    fn semantic_candidates(
        &self,
        connection: &Connection,
        profile: &EmbeddingProfileSummary,
        library_id: &str,
        request: &SearchRequest,
        query_vector: &[f32],
        prefer_lexical: bool,
    ) -> AppResult<Vec<SemanticCandidate>> {
        let allowed_item_ids = filtered_item_ids(
            connection,
            library_id,
            ItemFilters {
                requested_item_ids: &request.item_ids,
                collection_id: request.collection_id.as_deref(),
                year_from: request.year_from,
                year_to: request.year_to,
                item_type: request.item_type.as_deref(),
                citation_filter: request.citation_filter.as_deref(),
            },
        )?;
        let chunk_targets = load_chunk_targets(connection, &profile.id, library_id)?;
        let allowed_chunk_keys = chunk_targets
            .iter()
            .filter(|(_, target)| allowed_item_ids.contains(&target.item_id))
            .map(|(key, _)| *key)
            .collect::<HashSet<_>>();
        let chunk_matches = self.vectors.nearest(
            &profile.id,
            "chunk",
            query_vector,
            MAX_RETRIEVAL_CANDIDATES.min(allowed_chunk_keys.len()),
            &allowed_chunk_keys,
        )?;
        let paper_targets = load_paper_targets(connection, &profile.id, library_id)?;
        let allowed_paper_keys = paper_targets
            .iter()
            .filter(|(_, target)| allowed_item_ids.contains(&target.item_id))
            .map(|(key, _)| *key)
            .collect::<HashSet<_>>();
        let paper_matches = self.vectors.nearest(
            &profile.id,
            "paper",
            query_vector,
            MAX_RETRIEVAL_CANDIDATES.min(allowed_paper_keys.len()),
            &allowed_paper_keys,
        )?;
        let mut grouped: HashMap<String, SemanticCandidate> = HashMap::new();
        for (rank, matched) in chunk_matches.into_iter().enumerate() {
            let Some(target) = chunk_targets.get(&matched.key) else {
                continue;
            };
            let candidate = grouped
                .entry(target.item_id.clone())
                .or_insert_with(|| SemanticCandidate::new(target.item_id.clone()));
            candidate.chunk_scores.push(matched.similarity);
            candidate.passages.push(SemanticPassage {
                text: target.text.clone(),
                lexical_rank: None,
                semantic_rank: Some(rank as u32 + 1),
                chunk_id: target.chunk_id.clone(),
                document_id: target.document_id.clone(),
                document_revision_id: target.document_revision_id,
                snippet: bounded_chars(&target.text, 420),
                character_start: target.character_start,
                character_end: target.character_end,
                page_start: target.page_start,
                page_end: target.page_end,
                semantic_score: matched.similarity,
                embedding_profile_id: profile.id.clone(),
            });
        }
        if retrieval_mode(request.mode.as_deref()) == "hybrid" {
            for (rank, key) in passage_search::lexical_chunk_keys(
                connection,
                &request.query,
                &profile.id,
                library_id,
                &allowed_item_ids,
                MAX_RETRIEVAL_CANDIDATES,
            )?
            .into_iter()
            .enumerate()
            {
                let Some(target) = chunk_targets.get(&key) else {
                    continue;
                };
                let candidate = grouped
                    .entry(target.item_id.clone())
                    .or_insert_with(|| SemanticCandidate::new(target.item_id.clone()));
                if let Some(passage) = candidate
                    .passages
                    .iter_mut()
                    .find(|passage| passage.chunk_id == target.chunk_id)
                {
                    passage.lexical_rank = Some(rank as u32 + 1);
                } else {
                    candidate.passages.push(SemanticPassage {
                        text: target.text.clone(),
                        lexical_rank: Some(rank as u32 + 1),
                        semantic_rank: None,
                        chunk_id: target.chunk_id.clone(),
                        document_id: target.document_id.clone(),
                        document_revision_id: target.document_revision_id,
                        snippet: bounded_chars(&target.text, 420),
                        character_start: target.character_start,
                        character_end: target.character_end,
                        page_start: target.page_start,
                        page_end: target.page_end,
                        semantic_score: 0.0,
                        embedding_profile_id: profile.id.clone(),
                    });
                }
            }
        }
        for matched in paper_matches {
            let Some(target) = paper_targets.get(&matched.key) else {
                continue;
            };
            let candidate = grouped
                .entry(target.item_id.clone())
                .or_insert_with(|| SemanticCandidate::new(target.item_id.clone()));
            candidate.paper_score = Some(matched.similarity);
            candidate.representation = Some(target.representation_kind.clone());
        }
        let mut candidates = grouped.into_values().collect::<Vec<_>>();
        for candidate in &mut candidates {
            candidate.finish(prefer_lexical);
        }
        candidates.sort_by(semantic_candidate_order);
        Ok(candidates)
    }

    fn semantic_page(
        &self,
        request: &SearchRequest,
        candidates: Vec<SemanticCandidate>,
        diagnostics: &SemanticDiagnostics,
    ) -> AppResult<SearchPage> {
        let ranked = self.materialize_candidates(request, candidates, "semantic")?;
        page_results(request, ranked, "semantic", diagnostics)
    }

    fn hybrid_page(
        &self,
        request: &SearchRequest,
        semantic: Vec<SemanticCandidate>,
        diagnostics: &SemanticDiagnostics,
    ) -> AppResult<SearchPage> {
        let mut keyword_request = request.clone();
        keyword_request.mode = Some("keyword".to_owned());
        keyword_request.limit = Some(50);
        keyword_request.offset = Some(0);
        let lexical = self.database.search(&keyword_request)?;
        let lexical_by_id = lexical
            .results
            .into_iter()
            .enumerate()
            .map(|(rank, result)| (result.item.id.clone(), (rank as u32 + 1, result)))
            .collect::<HashMap<_, _>>();
        let semantic_by_id = semantic
            .into_iter()
            .enumerate()
            .map(|(rank, result)| (result.item_id.clone(), (rank as u32 + 1, result)))
            .collect::<HashMap<_, _>>();
        let mut ids = lexical_by_id.keys().cloned().collect::<HashSet<_>>();
        ids.extend(semantic_by_id.keys().cloned());
        let items = self
            .database
            .items_by_ids(&ids.iter().cloned().collect::<Vec<_>>())?
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect::<HashMap<_, _>>();
        let normalized_query = request.query.trim().to_lowercase();
        let mut results = Vec::new();
        for item_id in ids {
            let Some(item) = items.get(&item_id).cloned() else {
                continue;
            };
            let lexical_entry = lexical_by_id.get(&item_id);
            let semantic_entry = semantic_by_id.get(&item_id);
            let semantic_candidate = semantic_entry.map(|(_, candidate)| candidate);
            let lexical_rank = lexical_entry.map(|(rank, _)| *rank).or_else(|| {
                semantic_candidate.and_then(|candidate| {
                    candidate
                        .passages
                        .iter()
                        .filter_map(|passage| passage.lexical_rank)
                        .min()
                })
            });
            let semantic_rank = semantic_entry
                .filter(|(_, candidate)| {
                    candidate.paper_score.is_some() || !candidate.chunk_scores.is_empty()
                })
                .map(|(rank, _)| *rank);
            let mut fusion = hybrid_fusion_score(lexical_rank, semantic_rank);
            let exact = exact_match_kind(&item, &normalized_query);
            if exact.is_some() {
                fusion += 1.0;
            }
            let lexical_result = lexical_entry.map(|(_, result)| result);
            let mut reasons = Vec::new();
            let mut retrieval_modes = Vec::new();
            if lexical_rank.is_some() {
                reasons.push("keyword paper or passage rank".to_owned());
                retrieval_modes.push("keyword".to_owned());
            }
            if semantic_rank.is_some() {
                reasons.push("semantic passage or paper rank".to_owned());
                retrieval_modes.push("semantic".to_owned());
            }
            if let Some(kind) = exact.as_deref() {
                reasons.push(format!("exact {kind} match"));
            }
            results.push(SearchResult {
                item,
                score: fusion,
                reasons,
                snippet: semantic_candidate
                    .and_then(|candidate| candidate.passages.first())
                    .map(|passage| passage.snippet.clone())
                    .or_else(|| lexical_result.and_then(|result| result.snippet.clone())),
                match_source: match (lexical_rank, semantic_rank) {
                    (Some(_), Some(_)) => "keyword-and-semantic",
                    (Some(_), None) => "keyword",
                    _ => "semantic",
                }
                .to_owned(),
                retrieval_modes,
                ranking: crate::model::RetrievalScoreBreakdown {
                    lexical_rank,
                    semantic_rank,
                    lexical_score: lexical_result.map(|result| result.score),
                    semantic_score: semantic_candidate.map(|candidate| candidate.score),
                    fusion_score: Some(fusion),
                    exact_match: exact,
                },
                passages: semantic_candidate
                    .map(|candidate| candidate.passages.clone())
                    .unwrap_or_default(),
            });
        }
        results.sort_by(search_result_order);
        page_results(request, results, "hybrid", diagnostics)
    }

    fn materialize_candidates(
        &self,
        request: &SearchRequest,
        candidates: Vec<SemanticCandidate>,
        mode: &str,
    ) -> AppResult<Vec<SearchResult>> {
        let item_ids = candidates
            .iter()
            .map(|candidate| candidate.item_id.clone())
            .collect::<Vec<_>>();
        let items = self
            .database
            .items_by_ids(&item_ids)?
            .into_iter()
            .map(|item| (item.id.clone(), item))
            .collect::<HashMap<_, _>>();
        let normalized_query = request.query.trim().to_lowercase();
        let mut results = Vec::new();
        for (rank, candidate) in candidates.into_iter().enumerate() {
            let Some(item) = items.get(&candidate.item_id).cloned() else {
                continue;
            };
            let exact = exact_match_kind(&item, &normalized_query);
            let score = candidate.score + if exact.is_some() { 1.0 } else { 0.0 };
            let mut reasons = vec![if candidate.passages.is_empty() {
                format!(
                    "semantic paper representation ({})",
                    candidate.representation.as_deref().unwrap_or("metadata")
                )
            } else {
                format!("{} matching indexed passage(s)", candidate.passages.len())
            }];
            if let Some(kind) = exact.as_deref() {
                reasons.push(format!("exact {kind} match"));
            }
            results.push(SearchResult {
                item,
                score,
                reasons,
                snippet: candidate
                    .passages
                    .first()
                    .map(|passage| passage.snippet.clone()),
                match_source: if candidate.passages.is_empty() {
                    "semantic-paper"
                } else {
                    "semantic-passage"
                }
                .to_owned(),
                retrieval_modes: vec![mode.to_owned()],
                ranking: crate::model::RetrievalScoreBreakdown {
                    semantic_rank: Some(rank as u32 + 1),
                    semantic_score: Some(candidate.score),
                    exact_match: exact,
                    ..Default::default()
                },
                passages: candidate.passages,
            });
        }
        results.sort_by(search_result_order);
        Ok(results)
    }

    fn run_index_batch(
        &self,
        profile: &EmbeddingProfileSummary,
        library_id: &str,
        max_papers: usize,
        max_documents: usize,
    ) -> AppResult<()> {
        self.remove_inactive_vectors(profile, library_id)?;
        if self.cancel_requested.load(Ordering::Acquire) {
            return Ok(());
        }
        self.index_papers(profile, library_id, max_papers)?;
        if self.cancel_requested.load(Ordering::Acquire) {
            return Ok(());
        }
        self.index_documents(profile, library_id, max_documents)?;
        Ok(())
    }

    fn remove_inactive_vectors(
        &self,
        profile: &EmbeddingProfileSummary,
        library_id: &str,
    ) -> AppResult<()> {
        let connection = self.database.connect()?;
        for kind in ["paper", "chunk"] {
            let mut statement = connection.prepare(
                "SELECT vector_key FROM semantic_vector_entries
                 WHERE embedding_profile_id = ?1 AND library_id = ?2
                   AND index_kind = ?3 AND active = 0",
            )?;
            let keys = statement
                .query_map(params![profile.id, library_id, kind], |row| {
                    row.get::<_, i64>(0).map(|value| value as u64)
                })?
                .collect::<Result<Vec<_>, _>>()?;
            self.vectors.remove(&profile.id, kind, &keys)?;
        }
        Ok(())
    }

    fn index_papers(
        &self,
        profile: &EmbeddingProfileSummary,
        library_id: &str,
        limit: usize,
    ) -> AppResult<()> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT i.id
             FROM research_items i
             LEFT JOIN paper_semantic_representations psr
               ON psr.item_id = i.id
              AND psr.embedding_profile_id = ?2
              AND psr.representation_version = ?3
             WHERE i.library_id = ?1 AND i.is_trashed = 0
               AND (psr.id IS NULL OR psr.status <> 'active')
             ORDER BY CASE WHEN psr.status = 'stale' THEN 0 ELSE 1 END,
                      i.title COLLATE NOCASE, i.id
             LIMIT ?4",
        )?;
        let ids = statement
            .query_map(
                params![
                    library_id,
                    profile.id,
                    PAPER_REPRESENTATION_VERSION,
                    limit as i64,
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        let mut sources = Vec::with_capacity(ids.len());
        for item_id in ids {
            if self.cancel_requested.load(Ordering::Acquire) {
                break;
            }
            sources.push(load_paper_source(&connection, profile, &item_id)?);
        }
        self.index_paper_sources(profile, library_id, sources)
    }

    fn index_paper_sources(
        &self,
        profile: &EmbeddingProfileSummary,
        library_id: &str,
        mut sources: Vec<PaperSource>,
    ) -> AppResult<()> {
        let connection = self.database.connect()?;
        if sources.is_empty() {
            return Ok(());
        }
        let texts = sources
            .iter()
            .map(|source| source.text.clone())
            .collect::<Vec<_>>();
        let vectors = self.provider.embed_documents(&texts)?;
        if vectors.len() != sources.len() {
            return Err(AppError::Semantic(
                "The model returned a different number of paper vectors than requested".to_owned(),
            ));
        }
        let mut keyed = Vec::with_capacity(sources.len());
        for source in &mut sources {
            source.vector_key = allocate_vector_key(
                &connection,
                &profile.id,
                library_id,
                "paper",
                &source.representation_id,
            )?;
        }
        for (source, vector) in sources.iter().zip(vectors) {
            keyed.push((source.vector_key, vector));
        }
        self.vectors.upsert(&profile.id, "paper", &keyed)?;
        let mut connection = self.database.connect()?;
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        for source in sources {
            transaction.execute(
                "INSERT INTO paper_semantic_representations (
                    id, item_id, library_id, embedding_profile_id,
                    representation_version, representation_kind,
                    source_fields, document_id, document_revision_id,
                    source_fingerprint, coverage, vector_key, status,
                    created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                    ?11, ?12, 'active', ?13, ?13
                 ) ON CONFLICT(
                    item_id, embedding_profile_id, representation_version
                 ) DO UPDATE SET
                    representation_kind = excluded.representation_kind,
                    source_fields = excluded.source_fields,
                    document_id = excluded.document_id,
                    document_revision_id = excluded.document_revision_id,
                    source_fingerprint = excluded.source_fingerprint,
                    coverage = excluded.coverage,
                    vector_key = excluded.vector_key,
                    status = 'active', updated_at = excluded.updated_at",
                params![
                    source.representation_id,
                    source.item_id,
                    source.library_id,
                    profile.id,
                    PAPER_REPRESENTATION_VERSION,
                    source.representation_kind,
                    source.source_fields,
                    source.document_id,
                    source.document_revision_id,
                    source.fingerprint,
                    source.coverage,
                    source.vector_key as i64,
                    now,
                ],
            )?;
            transaction.execute(
                "UPDATE semantic_vector_entries SET
                    active = 1, updated_at = ?2 WHERE vector_key = ?1",
                params![source.vector_key as i64, now],
            )?;
            transaction.execute(
                "DELETE FROM semantic_neighbor_cache
                 WHERE embedding_profile_id = ?1
                   AND (source_item_id = ?2 OR target_item_id = ?2)",
                params![profile.id, source.item_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Index only the requested document with already installed models.
    pub fn index_document(&self, document_id: &str) -> AppResult<()> {
        let _guard = self
            .operation_guard
            .try_lock()
            .map_err(|_| AppError::Semantic("Semantic indexing is already running".into()))?;
        self.cancel_requested.store(false, Ordering::Release);
        let connection = self.database.connect()?;
        let profile = active_profile(&connection)?.ok_or_else(model_missing_error)?;
        if self.provider.installed_metadata()?.is_none() {
            return Err(model_missing_error());
        }
        let (library, item): (String, String) = connection.query_row(
            "SELECT library_id, research_item_id FROM documents WHERE id=?1",
            [document_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let sources = load_document_sources(&connection, &profile, &library, 1, Some(document_id))?;
        self.index_sources(&profile, &library, sources)?;
        let ready: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM semantic_chunks c
             JOIN document_revisions r ON r.id=c.document_revision_id
             JOIN document_texts t ON t.document_id=c.document_id AND t.is_active=1
             WHERE c.document_id=?1 AND c.status='active'
               AND c.embedding_profile_id=?2 AND c.chunking_profile_id=?3
               AND r.source_version=t.source_version
               AND (r.content_hash IS NULL OR r.content_hash='' OR r.content_hash=t.content_hash))",
            params![document_id, profile.id, CHUNKING_PROFILE_ID],
            |row| row.get(0),
        )?;
        if !ready {
            return Err(AppError::Semantic("No searchable passages were indexed for the current extraction. Check its saved text and retry.".into()));
        }
        self.index_paper_sources(
            &profile,
            &library,
            vec![load_paper_source(&connection, &profile, &item)?],
        )
    }

    fn index_documents(
        &self,
        profile: &EmbeddingProfileSummary,
        library_id: &str,
        limit: usize,
    ) -> AppResult<()> {
        let sources =
            load_document_sources(&self.database.connect()?, profile, library_id, limit, None)?;
        self.index_sources(profile, library_id, sources)
    }

    fn index_sources(
        &self,
        profile: &EmbeddingProfileSummary,
        library_id: &str,
        sources: Vec<DocumentSource>,
    ) -> AppResult<()> {
        for source in sources {
            if self.cancel_requested.load(Ordering::Acquire) {
                break;
            }
            let chunks = scientific_chunks(self.provider.as_ref(), &source)?;
            let texts = chunks
                .iter()
                .map(|chunk| chunk.text.clone())
                .collect::<Vec<_>>();
            let vectors = if texts.is_empty() {
                Vec::new()
            } else {
                self.provider.embed_documents(&texts)?
            };
            if vectors.len() != chunks.len() {
                return Err(AppError::Semantic(
                    "The model returned a different number of chunk vectors than requested"
                        .to_owned(),
                ));
            }
            let connection = self.database.connect()?;
            let mut keyed = Vec::with_capacity(chunks.len());
            let mut prepared = Vec::with_capacity(chunks.len());
            for mut chunk in chunks {
                chunk.vector_key =
                    allocate_vector_key(&connection, &profile.id, library_id, "chunk", &chunk.id)?;
                prepared.push(chunk);
            }
            for (chunk, vector) in prepared.iter().zip(vectors) {
                keyed.push((chunk.vector_key, vector));
            }
            self.vectors.upsert(&profile.id, "chunk", &keyed)?;
            persist_document_chunks(&self.database, profile, &source, &prepared)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct SemanticCandidate {
    item_id: String,
    score: f64,
    paper_score: Option<f64>,
    chunk_scores: Vec<f64>,
    passages: Vec<SemanticPassage>,
    representation: Option<String>,
}

impl SemanticCandidate {
    fn new(item_id: String) -> Self {
        Self {
            item_id,
            score: 0.0,
            paper_score: None,
            chunk_scores: Vec::new(),
            passages: Vec::new(),
            representation: None,
        }
    }

    fn finish(&mut self, prefer_lexical: bool) {
        self.chunk_scores
            .sort_by(|left, right| right.total_cmp(left));
        self.passages.sort_by(|left, right| {
            let score = |passage: &SemanticPassage| {
                hybrid_fusion_score(passage.lexical_rank, passage.semantic_rank)
            };
            let lexical_order = if prefer_lexical {
                left.lexical_rank
                    .unwrap_or(u32::MAX)
                    .cmp(&right.lexical_rank.unwrap_or(u32::MAX))
            } else {
                std::cmp::Ordering::Equal
            };
            lexical_order
                .then_with(|| score(right).total_cmp(&score(left)))
                .then_with(|| {
                    right
                        .lexical_rank
                        .is_some()
                        .cmp(&left.lexical_rank.is_some())
                })
                .then_with(|| left.chunk_id.cmp(&right.chunk_id))
        });
        self.passages.truncate(3);
        let passage_score = semantic_passage_score(&self.chunk_scores);
        self.score = match (passage_score, self.paper_score) {
            (Some(passage), Some(paper)) => passage.max((passage * 0.78) + (paper * 0.22)),
            (Some(passage), None) => passage,
            (None, Some(paper)) => paper,
            (None, None) => 0.0,
        };
    }
}

#[derive(Clone, Debug)]
struct PaperTarget {
    vector_key: u64,
    item_id: String,
    representation_kind: String,
}

#[derive(Clone, Debug)]
struct ChunkTarget {
    item_id: String,
    chunk_id: String,
    document_id: String,
    document_revision_id: i64,
    text: String,
    character_start: u64,
    character_end: u64,
    page_start: Option<u32>,
    page_end: Option<u32>,
}

fn retrieval_mode(value: Option<&str>) -> &'static str {
    match value {
        Some("semantic") => "semantic",
        Some("hybrid") => "hybrid",
        _ => "keyword",
    }
}

fn empty_semantic_page(
    database: &ResearchDatabase,
    request: &SearchRequest,
    mode: &str,
) -> AppResult<SearchPage> {
    let workspace = database.load_workspace()?;
    Ok(SearchPage {
        results: Vec::new(),
        total: 0,
        offset: request.offset.unwrap_or(0),
        limit: request.limit.unwrap_or(20).clamp(1, 50),
        corpus_status: workspace.corpus.status,
        retrieval_mode: mode.to_owned(),
        semantic_indexed_papers: workspace.semantic.indexed_papers,
        semantic_eligible_papers: workspace.semantic.eligible_papers,
    })
}

fn page_results(
    request: &SearchRequest,
    results: Vec<SearchResult>,
    mode: &str,
    diagnostics: &SemanticDiagnostics,
) -> AppResult<SearchPage> {
    let total = results.len() as u64;
    let offset = request.offset.unwrap_or(0);
    let limit = request.limit.unwrap_or(20).clamp(1, 50);
    let page = results
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    Ok(SearchPage {
        results: page,
        total,
        offset,
        limit,
        corpus_status: "local-index".to_owned(),
        retrieval_mode: mode.to_owned(),
        semantic_indexed_papers: diagnostics.indexed_papers,
        semantic_eligible_papers: diagnostics.eligible_papers,
    })
}

fn semantic_candidate_order(
    left: &SemanticCandidate,
    right: &SemanticCandidate,
) -> CompareOrdering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.item_id.cmp(&right.item_id))
}

fn search_result_order(left: &SearchResult, right: &SearchResult) -> CompareOrdering {
    right
        .score
        .total_cmp(&left.score)
        .then_with(|| left.item.title.cmp(&right.item.title))
        .then_with(|| left.item.id.cmp(&right.item.id))
}

fn exact_match_kind(item: &ResearchItem, normalized_query: &str) -> Option<String> {
    if item.title.trim().eq_ignore_ascii_case(normalized_query) {
        return Some("title".to_owned());
    }
    let query_doi = normalize_doi(normalized_query);
    if let Some(kind) = item
        .doi
        .as_deref()
        .filter(|doi| !query_doi.is_empty() && normalize_doi(doi) == query_doi)
        .map(|_| "DOI".to_owned())
    {
        return Some(kind);
    }
    let year = item.year?.to_string();
    if normalized_query.split_whitespace().any(|term| term == year)
        && item.authors.iter().any(|author| {
            author
                .name
                .split_whitespace()
                .last()
                .is_some_and(|name| normalized_query.contains(&name.to_lowercase()))
        })
    {
        return Some("author/year".to_owned());
    }
    None
}

fn normalize_doi(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .trim_start_matches("doi:")
        .trim()
        .to_owned()
}

fn load_paper_targets(
    connection: &Connection,
    profile_id: &str,
    library_id: &str,
) -> AppResult<HashMap<u64, PaperTarget>> {
    let mut statement = connection.prepare(
        "SELECT psr.vector_key, psr.item_id, psr.representation_kind
         FROM paper_semantic_representations psr
         JOIN semantic_vector_entries sve ON sve.vector_key = psr.vector_key
         WHERE psr.embedding_profile_id = ?1 AND psr.library_id = ?2
           AND psr.status = 'active' AND sve.active = 1",
    )?;
    let rows = statement.query_map(params![profile_id, library_id], |row| {
        let vector_key = row.get::<_, i64>(0)? as u64;
        Ok((
            vector_key,
            PaperTarget {
                vector_key,
                item_id: row.get(1)?,
                representation_kind: row.get(2)?,
            },
        ))
    })?;
    rows.collect::<Result<HashMap<_, _>, _>>()
        .map_err(Into::into)
}

fn load_chunk_targets(
    connection: &Connection,
    profile_id: &str,
    library_id: &str,
) -> AppResult<HashMap<u64, ChunkTarget>> {
    let mut statement = connection.prepare(
        "SELECT sc.vector_key, sc.item_id, sc.id, sc.document_id,
                sc.document_revision_id, sc.text,
                sc.character_start, sc.character_end,
                sc.page_start, sc.page_end
         FROM semantic_chunks sc
         JOIN semantic_vector_entries sve ON sve.vector_key = sc.vector_key
         WHERE sc.embedding_profile_id = ?1 AND sc.library_id = ?2
           AND sc.status = 'active' AND sve.active = 1",
    )?;
    let rows = statement.query_map(params![profile_id, library_id], |row| {
        Ok((
            row.get::<_, i64>(0)? as u64,
            ChunkTarget {
                item_id: row.get(1)?,
                chunk_id: row.get(2)?,
                document_id: row.get(3)?,
                document_revision_id: row.get(4)?,
                text: row.get(5)?,
                character_start: row.get::<_, i64>(6)? as u64,
                character_end: row.get::<_, i64>(7)? as u64,
                page_start: row.get(8)?,
                page_end: row.get(9)?,
            },
        ))
    })?;
    rows.collect::<Result<HashMap<_, _>, _>>()
        .map_err(Into::into)
}

#[derive(Default)]
struct ItemFilters<'a> {
    requested_item_ids: &'a [String],
    collection_id: Option<&'a str>,
    year_from: Option<i32>,
    year_to: Option<i32>,
    item_type: Option<&'a str>,
    citation_filter: Option<&'a str>,
}

fn filtered_item_ids(
    connection: &Connection,
    library_id: &str,
    filters: ItemFilters<'_>,
) -> AppResult<HashSet<String>> {
    let citation_condition = semantic_citation_condition(filters.citation_filter.unwrap_or("all"));
    let mut statement = connection.prepare(&format!(
        "SELECT i.id FROM research_items i
         WHERE i.library_id = ?1 AND i.is_trashed = 0
           AND (?2 IS NULL OR EXISTS (
                SELECT 1 FROM item_collections ic
                WHERE ic.item_id = i.id AND ic.collection_id = ?2
           ))
           AND (?3 IS NULL OR i.publication_year >= ?3)
           AND (?4 IS NULL OR i.publication_year <= ?4)
           AND (?5 IS NULL OR i.item_type = ?5)
           {citation_condition}"
    ))?;
    let rows = statement.query_map(
        params![
            library_id,
            filters.collection_id,
            filters.year_from,
            filters.year_to,
            filters.item_type
        ],
        |row| row.get::<_, String>(0),
    )?;
    let mut item_ids = rows.collect::<Result<HashSet<_>, _>>()?;
    if !filters.requested_item_ids.is_empty() {
        let requested = filters.requested_item_ids.iter().collect::<HashSet<_>>();
        item_ids.retain(|item_id| requested.contains(item_id));
    }
    Ok(item_ids)
}

fn semantic_citation_condition(filter: &str) -> &'static str {
    match filter {
        "references-indexed" => {
            "AND EXISTS (
                SELECT 1 FROM bibliographic_references br
                JOIN reference_extractions be ON be.id = br.extraction_id
                WHERE br.source_item_id = i.id
                  AND br.is_current = 1 AND be.is_current = 1
            )"
        }
        "cited-by-local" => {
            "AND EXISTS (
                SELECT 1 FROM citation_edges ce WHERE ce.target_item_id = i.id
            )"
        }
        "has-external" => {
            "AND EXISTS (
                SELECT 1 FROM bibliographic_references br
                JOIN reference_extractions be ON be.id = br.extraction_id
                JOIN reference_resolutions rr ON rr.reference_id = br.id
                WHERE br.source_item_id = i.id AND br.is_current = 1
                  AND be.is_current = 1 AND rr.status = 'external'
            )"
        }
        "needs-review" => {
            "AND EXISTS (
                SELECT 1 FROM bibliographic_references br
                JOIN reference_extractions be ON be.id = br.extraction_id
                JOIN reference_resolutions rr ON rr.reference_id = br.id
                WHERE br.source_item_id = i.id AND br.is_current = 1
                  AND be.is_current = 1
                  AND rr.status IN ('ambiguous', 'probable', 'unresolved')
            )"
        }
        _ => "",
    }
}

fn vector_centroid(vectors: &[Vec<f32>]) -> AppResult<Vec<f32>> {
    if vectors.is_empty() {
        return Err(AppError::Semantic(
            "A semantic centroid requires at least one vector".to_owned(),
        ));
    }
    let mut centroid = vec![0.0_f32; EMBEDDING_DIMENSION];
    for vector in vectors {
        validate_dimension(vector)?;
        for (target, value) in centroid.iter_mut().zip(vector) {
            *target += *value / vectors.len() as f32;
        }
    }
    normalize_vector(centroid)
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> AppResult<f64> {
    validate_dimension(left)?;
    validate_dimension(right)?;
    Ok(left
        .iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum::<f64>()
        .clamp(-1.0, 1.0))
}

fn similarity_label(score: f64) -> &'static str {
    if score >= 0.78 {
        "strong semantic similarity"
    } else if score >= 0.64 {
        "moderate semantic similarity"
    } else {
        "related semantic representation"
    }
}

fn persist_semantic_edge(
    connection: &Connection,
    profile: &EmbeddingProfileSummary,
    source: &PaperTarget,
    target: &PaperTarget,
    score: f64,
    mutual: bool,
) -> AppResult<String> {
    let (source_id, target_id, source_representation, target_representation) =
        if source.item_id < target.item_id {
            (
                source.item_id.as_str(),
                target.item_id.as_str(),
                source.representation_kind.as_str(),
                target.representation_kind.as_str(),
            )
        } else {
            (
                target.item_id.as_str(),
                source.item_id.as_str(),
                target.representation_kind.as_str(),
                source.representation_kind.as_str(),
            )
        };
    let edge_id = stable_semantic_id("semantic-edge", &[&profile.id, source_id, target_id]);
    connection.execute(
        "INSERT INTO semantic_neighbor_cache (
            edge_id, embedding_profile_id, source_item_id,
            target_item_id, representation_version,
            source_representation, target_representation,
            similarity_metric, similarity_score, mutual, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(embedding_profile_id, source_item_id, target_item_id)
         DO UPDATE SET
            source_representation = excluded.source_representation,
            target_representation = excluded.target_representation,
            similarity_score = excluded.similarity_score,
            mutual = excluded.mutual,
            created_at = excluded.created_at",
        params![
            edge_id,
            profile.id,
            source_id,
            target_id,
            PAPER_REPRESENTATION_VERSION,
            source_representation,
            target_representation,
            profile.similarity_metric,
            score,
            mutual,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(edge_id)
}

fn citation_item_summary(connection: &Connection, item_id: &str) -> AppResult<CitationItemSummary> {
    connection
        .query_row(
            "SELECT i.id, i.title, i.publication_year, i.venue, i.doi,
                    (SELECT c.display_name FROM creators c
                     WHERE c.item_id = i.id ORDER BY c.position LIMIT 1)
             FROM research_items i WHERE i.id = ?1",
            [item_id],
            |row| {
                Ok(CitationItemSummary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    year: row.get(2)?,
                    venue: row.get(3)?,
                    doi: row.get(4)?,
                    first_author: row.get(5)?,
                })
            },
        )
        .map_err(Into::into)
}

#[derive(Clone, Debug)]
struct PaperSource {
    representation_id: String,
    item_id: String,
    library_id: String,
    text: String,
    representation_kind: String,
    source_fields: String,
    document_id: Option<String>,
    document_revision_id: Option<i64>,
    fingerprint: String,
    coverage: String,
    vector_key: u64,
}

fn load_paper_source(
    connection: &Connection,
    profile: &EmbeddingProfileSummary,
    item_id: &str,
) -> AppResult<PaperSource> {
    let (library_id, title, abstract_text) = connection.query_row(
        "SELECT library_id, title, abstract_text
         FROM research_items WHERE id = ?1 AND is_trashed = 0",
        [item_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )?;
    let document = connection
        .query_row(
            "SELECT d.id, dr.id, dt.content,
                    CASE WHEN e.status = 'references-extracted'
                              AND e.section_start IS NOT NULL
                         THEN e.section_start ELSE length(dt.content) END
             FROM documents d
             JOIN document_texts dt
               ON dt.document_id = d.id AND dt.is_active = 1
             JOIN document_revisions dr ON dr.id = (
                 SELECT dr2.id FROM document_revisions dr2
                 WHERE dr2.document_id = d.id
                   AND dr2.source_version = dt.source_version
                   AND dr2.revision_key LIKE 'text:%'
                   AND (dr2.content_hash IS NULL OR dr2.content_hash = ''
                        OR dr2.content_hash = dt.content_hash)
                 ORDER BY dr2.id DESC LIMIT 1
             )
             LEFT JOIN reference_extractions e
               ON e.document_id = d.id AND e.document_revision_id = dr.id
              AND e.is_current = 1
             WHERE d.research_item_id = ?1
             ORDER BY CASE WHEN dt.completeness = 'full' THEN 0 ELSE 1 END,
                      d.id LIMIT 1",
            [item_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let clean_abstract = abstract_text
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (text, kind, fields, document_id, revision_id, coverage) =
        if let Some(abstract_text) = clean_abstract {
            (
                format!("Title: {title}\n\nAbstract: {abstract_text}"),
                "title-abstract".to_owned(),
                "title,abstract".to_owned(),
                None,
                None,
                "title-and-abstract".to_owned(),
            )
        } else if let Some((document_id, revision_id, body, body_end)) = document {
            let body_end = safe_byte_boundary(&body, body_end.max(0) as usize);
            let bounded = bounded_chars(body[..body_end].trim(), 2_400);
            if bounded.is_empty() {
                (
                    format!("Title: {title}"),
                    "title-only".to_owned(),
                    "title".to_owned(),
                    None,
                    None,
                    "title-only".to_owned(),
                )
            } else {
                (
                    format!("Title: {title}\n\nDocument opening: {bounded}"),
                    "title-body".to_owned(),
                    "title,document-opening".to_owned(),
                    Some(document_id),
                    Some(revision_id),
                    "title-and-bounded-body".to_owned(),
                )
            }
        } else {
            (
                format!("Title: {title}"),
                "title-only".to_owned(),
                "title".to_owned(),
                None,
                None,
                "title-only".to_owned(),
            )
        };
    let fingerprint = text_fingerprint(&[
        &profile.id,
        PAPER_REPRESENTATION_VERSION,
        &kind,
        &text,
        &document_id.clone().unwrap_or_default(),
        &revision_id
            .map(|value| value.to_string())
            .unwrap_or_default(),
    ]);
    Ok(PaperSource {
        representation_id: stable_semantic_id("paper-representation", &[&profile.id, item_id]),
        item_id: item_id.to_owned(),
        library_id,
        text,
        representation_kind: kind,
        source_fields: fields,
        document_id,
        document_revision_id: revision_id,
        fingerprint,
        coverage,
        vector_key: 0,
    })
}

fn load_document_sources(
    connection: &Connection,
    profile: &EmbeddingProfileSummary,
    library_id: &str,
    limit: usize,
    document_id: Option<&str>,
) -> AppResult<Vec<DocumentSource>> {
    let mut statement = connection.prepare(
        "SELECT d.research_item_id, d.library_id, d.id, dr.id,
                dt.provider, dt.content,
                CASE WHEN e.status = 'references-extracted'
                          AND e.section_start IS NOT NULL
                     THEN e.section_start ELSE length(dt.content) END,
                CASE WHEN e.status = 'references-extracted'
                          AND e.section_start IS NOT NULL
                     THEN 1 ELSE 0 END
         FROM documents d
         JOIN document_texts dt
           ON dt.document_id = d.id AND dt.is_active = 1
         JOIN document_revisions dr ON dr.id = (
             SELECT dr2.id FROM document_revisions dr2
             WHERE dr2.document_id = d.id
               AND dr2.source_version = dt.source_version
               AND dr2.revision_key LIKE 'text:%'
                   AND (dr2.content_hash IS NULL OR dr2.content_hash = ''
                        OR dr2.content_hash = dt.content_hash)
             ORDER BY dr2.id DESC LIMIT 1
         )
         LEFT JOIN reference_extractions e
           ON e.document_id = d.id AND e.document_revision_id = dr.id
          AND e.is_current = 1
         LEFT JOIN semantic_document_states sds
           ON sds.document_id = d.id
          AND sds.embedding_profile_id = ?2
          AND sds.chunking_profile_id = ?3
         WHERE d.library_id = ?1 AND d.research_item_id IS NOT NULL
           AND (sds.document_id IS NULL OR sds.document_revision_id <> dr.id
                OR sds.status <> 'indexed')
           AND (?5 IS NULL OR d.id=?5)
         ORDER BY d.id LIMIT ?4",
    )?;
    let sources = statement
        .query_map(
            params![
                library_id,
                profile.id,
                CHUNKING_PROFILE_ID,
                limit as i64,
                document_id
            ],
            |row| {
                let text: String = row.get(5)?;
                let body_end = safe_byte_boundary(&text, row.get::<_, i64>(6)?.max(0) as usize);
                Ok(DocumentSource {
                    item_id: row.get(0)?,
                    library_id: row.get(1)?,
                    document_id: row.get(2)?,
                    document_revision_id: row.get(3)?,
                    provider: row.get(4)?,
                    text,
                    body_end_byte: body_end,
                    bibliography_excluded: row.get::<_, bool>(7)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::from)?;
    Ok(sources)
}

fn allocate_vector_key(
    connection: &Connection,
    profile_id: &str,
    library_id: &str,
    kind: &str,
    target_id: &str,
) -> AppResult<u64> {
    let now = Utc::now().to_rfc3339();
    connection.execute(
        "INSERT INTO semantic_vector_entries (
            embedding_profile_id, library_id, index_kind,
            target_id, active, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)
         ON CONFLICT(embedding_profile_id, index_kind, target_id)
         DO UPDATE SET active = 0, updated_at = excluded.updated_at",
        params![profile_id, library_id, kind, target_id, now],
    )?;
    connection
        .query_row(
            "SELECT vector_key FROM semantic_vector_entries
             WHERE embedding_profile_id = ?1 AND index_kind = ?2
               AND target_id = ?3",
            params![profile_id, kind, target_id],
            |row| row.get::<_, i64>(0).map(|value| value as u64),
        )
        .map_err(Into::into)
}

fn persist_document_chunks(
    database: &ResearchDatabase,
    profile: &EmbeddingProfileSummary,
    source: &DocumentSource,
    chunks: &[ScientificChunk],
) -> AppResult<()> {
    let mut connection = database.connect()?;
    let transaction = connection.transaction()?;
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "DELETE FROM semantic_chunks
         WHERE document_id = ?1 AND embedding_profile_id = ?2
           AND chunking_profile_id = ?3",
        params![source.document_id, profile.id, CHUNKING_PROFILE_ID],
    )?;
    let page_spans = crate::extraction::pages::active_spans(&transaction, source)?;
    for chunk in chunks {
        let (start, end, pages) =
            crate::extraction::pages::chunk_location(source, chunk, &page_spans)?;
        transaction.execute(
            "INSERT INTO semantic_chunks (
                id, item_id, library_id, document_id,
                document_revision_id, embedding_profile_id,
                chunking_profile_id, source_provider,
                character_start, character_end, page_start, page_end,
                section_heading, ordinal, token_count, text,
                text_fingerprint, vector_key, status, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                ?17, ?18, NULL, ?11, ?12, ?13, ?14, ?15,
                'active', ?16, ?16
             )",
            params![
                chunk.id,
                source.item_id,
                source.library_id,
                source.document_id,
                source.document_revision_id,
                profile.id,
                CHUNKING_PROFILE_ID,
                source.provider,
                start as i64,
                end as i64,
                chunk.ordinal as i64,
                chunk.token_count as i64,
                chunk.text,
                chunk.fingerprint,
                chunk.vector_key as i64,
                now,
                pages.map(|p| p.0),
                pages.map(|p| p.1),
            ],
        )?;
        transaction.execute(
            "UPDATE semantic_vector_entries SET
                active = 1, updated_at = ?2 WHERE vector_key = ?1",
            params![chunk.vector_key as i64, now],
        )?;
    }
    transaction.execute(
        "INSERT INTO semantic_document_states (
            document_id, document_revision_id, embedding_profile_id,
            chunking_profile_id, status, chunk_count,
            bibliography_excluded, updated_at, last_error
         ) VALUES (?1, ?2, ?3, ?4, 'indexed', ?5, ?6, ?7, NULL)
         ON CONFLICT(document_id, embedding_profile_id, chunking_profile_id)
         DO UPDATE SET
            document_revision_id = excluded.document_revision_id,
            status = 'indexed', chunk_count = excluded.chunk_count,
            bibliography_excluded = excluded.bibliography_excluded,
            updated_at = excluded.updated_at, last_error = NULL",
        params![
            source.document_id,
            source.document_revision_id,
            profile.id,
            CHUNKING_PROFILE_ID,
            chunks.len() as i64,
            source.bibliography_excluded,
            now,
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

fn refresh_semantic_state(
    connection: &Connection,
    library_id: &str,
    profile_id: &str,
    vector_bytes: u64,
    cancelled: bool,
    error: Option<&str>,
) -> AppResult<()> {
    let counts = connection.query_row(
        "SELECT
            (SELECT COUNT(*) FROM research_items i
             WHERE i.library_id = ?1 AND i.is_trashed = 0),
            (SELECT COUNT(*) FROM paper_semantic_representations psr
             WHERE psr.library_id = ?1 AND psr.embedding_profile_id = ?2
               AND psr.status = 'active'),
            (SELECT COUNT(*) FROM paper_semantic_representations psr
             WHERE psr.library_id = ?1 AND psr.embedding_profile_id = ?2
               AND psr.status = 'stale'),
            (SELECT COUNT(*) FROM documents d
             JOIN document_texts dt
               ON dt.document_id = d.id AND dt.is_active = 1
             WHERE d.library_id = ?1 AND d.research_item_id IS NOT NULL),
            (SELECT COUNT(*) FROM semantic_document_states sds
             JOIN documents d ON d.id = sds.document_id
             WHERE d.library_id = ?1 AND sds.embedding_profile_id = ?2
               AND sds.chunking_profile_id = ?3
               AND sds.status = 'indexed'),
            (SELECT COUNT(*) FROM semantic_chunks sc
             WHERE sc.library_id = ?1 AND sc.embedding_profile_id = ?2),
            (SELECT COUNT(*) FROM semantic_chunks sc
             WHERE sc.library_id = ?1 AND sc.embedding_profile_id = ?2
               AND sc.status = 'active'),
            (SELECT COUNT(*) FROM semantic_chunks sc
             WHERE sc.library_id = ?1 AND sc.embedding_profile_id = ?2
               AND sc.status = 'stale'),
            (SELECT COUNT(*) FROM paper_semantic_representations psr
             WHERE psr.library_id = ?1 AND psr.embedding_profile_id = ?2
               AND psr.status = 'active' AND NOT EXISTS (
                    SELECT 1 FROM documents d
                    JOIN document_texts dt
                      ON dt.document_id = d.id AND dt.is_active = 1
                    WHERE d.research_item_id = psr.item_id
               )),
            (SELECT COALESCE(SUM(length(sc.text)), 0)
             FROM semantic_chunks sc
             WHERE sc.library_id = ?1 AND sc.embedding_profile_id = ?2)",
        params![library_id, profile_id, CHUNKING_PROFILE_ID],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
            ))
        },
    )?;
    let status = if error.is_some() {
        "failed"
    } else if cancelled {
        "partial"
    } else if counts.1 >= counts.0 && counts.4 >= counts.3 {
        "up-to-date"
    } else if counts.1 > 0 || counts.4 > 0 {
        "partial"
    } else {
        "ready"
    };
    connection.execute(
        "INSERT INTO semantic_index_state (
            library_id, embedding_profile_id, status,
            eligible_papers, indexed_papers, stale_papers,
            eligible_documents, chunked_documents, generated_chunks,
            indexed_chunks, stale_chunks, metadata_only_papers,
            vector_index_bytes, chunk_metadata_bytes,
            last_indexed_at, last_error
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?14, ?15, ?16
         ) ON CONFLICT(library_id) DO UPDATE SET
            embedding_profile_id = excluded.embedding_profile_id,
            status = excluded.status,
            eligible_papers = excluded.eligible_papers,
            indexed_papers = excluded.indexed_papers,
            stale_papers = excluded.stale_papers,
            eligible_documents = excluded.eligible_documents,
            chunked_documents = excluded.chunked_documents,
            generated_chunks = excluded.generated_chunks,
            indexed_chunks = excluded.indexed_chunks,
            stale_chunks = excluded.stale_chunks,
            metadata_only_papers = excluded.metadata_only_papers,
            vector_index_bytes = excluded.vector_index_bytes,
            chunk_metadata_bytes = excluded.chunk_metadata_bytes,
            last_indexed_at = excluded.last_indexed_at,
            last_error = excluded.last_error",
        params![
            library_id,
            profile_id,
            status,
            counts.0,
            counts.1,
            counts.2,
            counts.3,
            counts.4,
            counts.5,
            counts.6,
            counts.7,
            counts.8,
            vector_bytes as i64,
            counts.9,
            Utc::now().to_rfc3339(),
            error,
        ],
    )?;
    Ok(())
}

fn record_semantic_error(
    connection: &Connection,
    library_id: &str,
    profile_id: Option<&str>,
    target_kind: &str,
    target_id: Option<&str>,
    message: &str,
) -> AppResult<()> {
    connection.execute(
        "INSERT INTO semantic_index_errors (
            library_id, embedding_profile_id, target_kind,
            target_id, message, recorded_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            library_id,
            profile_id,
            target_kind,
            target_id,
            message,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn bounded_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn safe_byte_boundary(value: &str, requested: usize) -> usize {
    let mut boundary = requested.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn text_fingerprint(parts: &[&str]) -> String {
    stable_semantic_id("semantic-text", parts)
}

pub fn load_semantic_diagnostics(
    connection: &Connection,
    library_id: Option<&str>,
    vector_storage_override: Option<u64>,
) -> AppResult<SemanticDiagnostics> {
    let installation = connection
        .query_row(
            "SELECT artifact_identity, installed_bytes, status
             FROM semantic_model_installations
             WHERE model_identifier = ?1",
            [DEFAULT_MODEL_ID],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let profile = active_profile(connection)?;
    // A newer PDF locator revision need not change the active text.
    let runtime_counts = match (library_id, profile.as_ref()) {
        (Some(library_id), Some(profile)) => connection.query_row(
            "WITH active_text_revisions AS (
                SELECT dt.document_id, (
                    SELECT dr.id FROM document_revisions dr
                    WHERE dr.document_id = dt.document_id
                      AND dr.source_version = dt.source_version
                      AND dr.revision_key LIKE 'text:%'
                      AND (dr.content_hash IS NULL OR dr.content_hash = ''
                           OR dr.content_hash = dt.content_hash)
                    ORDER BY dr.id DESC LIMIT 1
                ) AS revision_id
                FROM document_texts dt WHERE dt.is_active = 1
             )
             SELECT
                (SELECT COUNT(*) FROM semantic_vector_entries sve
                 WHERE sve.embedding_profile_id = ?2
                   AND sve.index_kind = 'paper' AND sve.active = 1),
                (SELECT COUNT(*) FROM semantic_vector_entries sve
                 WHERE sve.embedding_profile_id = ?2
                   AND sve.index_kind = 'chunk' AND sve.active = 1),
                (SELECT COALESCE(MAX(sds.chunk_count), 0)
                 FROM semantic_document_states sds
                 JOIN documents d ON d.id = sds.document_id
                 WHERE d.library_id = ?1 AND sds.embedding_profile_id = ?2
                   AND sds.chunking_profile_id = ?3),
                (SELECT COUNT(*) FROM semantic_document_states sds
                 JOIN documents d ON d.id = sds.document_id
                 WHERE d.library_id = ?1 AND sds.embedding_profile_id = ?2
                   AND sds.chunking_profile_id = ?3
                   AND sds.chunk_count >= ?4),
                (SELECT COUNT(*) FROM semantic_document_states sds
                 JOIN documents d ON d.id = sds.document_id
                 WHERE d.library_id = ?1 AND sds.embedding_profile_id = ?2
                   AND sds.chunking_profile_id = ?3 AND sds.status = 'indexed'
                   AND sds.document_revision_id IS (
                       SELECT revision_id FROM active_text_revisions
                       WHERE document_id = sds.document_id
                   )),
                (SELECT COUNT(*) FROM semantic_document_states sds
                 JOIN documents d ON d.id = sds.document_id
                 WHERE d.library_id = ?1 AND sds.embedding_profile_id = ?2
                   AND sds.chunking_profile_id = ?3 AND sds.status = 'indexed'
                   AND sds.document_revision_id IS NOT (
                       SELECT revision_id FROM active_text_revisions
                       WHERE document_id = sds.document_id
                   ))",
            params![
                library_id,
                profile.id,
                CHUNKING_PROFILE_ID,
                MAXIMUM_CHUNKS as i64
            ],
            |row| {
                Ok(SemanticRuntimeCounts {
                    registered_paper_vectors: row.get::<_, i64>(0)?.max(0) as u64,
                    registered_chunk_vectors: row.get::<_, i64>(1)?.max(0) as u64,
                    maximum_observed_chunks_per_document: row.get::<_, i64>(2)?.max(0) as u64,
                    documents_at_chunk_limit: row.get::<_, i64>(3)?.max(0) as u64,
                    current_document_revisions_indexed: row.get::<_, i64>(4)?.max(0) as u64,
                    stale_document_revisions_indexed: row.get::<_, i64>(5)?.max(0) as u64,
                })
            },
        )?,
        _ => SemanticRuntimeCounts::default(),
    };
    let state = library_id
        .map(|id| {
            connection
                .query_row(
                    "SELECT status, eligible_papers, indexed_papers,
                            stale_papers, eligible_documents,
                            chunked_documents, generated_chunks,
                            indexed_chunks, stale_chunks,
                            metadata_only_papers, vector_index_bytes,
                            chunk_metadata_bytes, last_indexed_at, last_error
                     FROM semantic_index_state WHERE library_id = ?1",
                    [id],
                    |row| {
                        Ok(SemanticStateRow {
                            status: row.get(0)?,
                            eligible_papers: row.get::<_, i64>(1)? as u64,
                            indexed_papers: row.get::<_, i64>(2)? as u64,
                            stale_papers: row.get::<_, i64>(3)? as u64,
                            eligible_documents: row.get::<_, i64>(4)? as u64,
                            chunked_documents: row.get::<_, i64>(5)? as u64,
                            generated_chunks: row.get::<_, i64>(6)? as u64,
                            indexed_chunks: row.get::<_, i64>(7)? as u64,
                            stale_chunks: row.get::<_, i64>(8)? as u64,
                            metadata_only_papers: row.get::<_, i64>(9)? as u64,
                            vector_index_bytes: row.get::<_, i64>(10)? as u64,
                            chunk_metadata_bytes: row.get::<_, i64>(11)? as u64,
                            last_indexed_at: row.get(12)?,
                            last_error: row.get(13)?,
                        })
                    },
                )
                .optional()
        })
        .transpose()?
        .flatten()
        .unwrap_or_default();
    let model_installed = installation
        .as_ref()
        .is_some_and(|(_, _, status)| status == "installed");
    let status = if !model_installed {
        "model-missing".to_owned()
    } else if profile.is_none() {
        "profile-missing".to_owned()
    } else {
        state.status.clone()
    };
    Ok(SemanticDiagnostics {
        indexing: status == "indexing",
        status,
        model_installed,
        model_identifier: DEFAULT_MODEL_ID.to_owned(),
        model_display_name: DEFAULT_MODEL_NAME.to_owned(),
        model_source: DEFAULT_MODEL_SOURCE.to_owned(),
        model_license: DEFAULT_MODEL_LICENSE.to_owned(),
        model_download_bytes: DEFAULT_MODEL_DOWNLOAD_BYTES,
        model_storage_bytes: installation.as_ref().map(|value| value.1).unwrap_or(0),
        model_revision: installation.as_ref().map(|value| value.0.clone()),
        active_profile: profile,
        active_chunking_profile: active_chunking_profile_summary(),
        active_retrieval_profile: active_retrieval_profile_summary(),
        vector_backend: VECTOR_BACKEND.to_owned(),
        eligible_papers: state.eligible_papers,
        indexed_papers: state.indexed_papers,
        stale_papers: state.stale_papers,
        eligible_documents: state.eligible_documents,
        chunked_documents: state.chunked_documents,
        generated_chunks: state.generated_chunks,
        indexed_chunks: state.indexed_chunks,
        stale_chunks: state.stale_chunks,
        current_document_revisions_indexed: runtime_counts.current_document_revisions_indexed,
        stale_document_revisions_indexed: runtime_counts.stale_document_revisions_indexed,
        maximum_observed_chunks_per_document: runtime_counts.maximum_observed_chunks_per_document,
        documents_at_chunk_limit: runtime_counts.documents_at_chunk_limit,
        registered_paper_vectors: runtime_counts.registered_paper_vectors,
        physical_paper_vectors: None,
        paper_vector_count_matches: None,
        registered_chunk_vectors: runtime_counts.registered_chunk_vectors,
        physical_chunk_vectors: None,
        chunk_vector_count_matches: None,
        metadata_only_papers: state.metadata_only_papers,
        vector_index_bytes: vector_storage_override.unwrap_or(state.vector_index_bytes),
        chunk_metadata_bytes: state.chunk_metadata_bytes,
        last_indexed_at: state.last_indexed_at,
        last_error: state.last_error,
    })
}

#[derive(Clone, Debug, Default)]
struct SemanticRuntimeCounts {
    registered_paper_vectors: u64,
    registered_chunk_vectors: u64,
    current_document_revisions_indexed: u64,
    stale_document_revisions_indexed: u64,
    maximum_observed_chunks_per_document: u64,
    documents_at_chunk_limit: u64,
}

fn active_chunking_profile_summary() -> ChunkingProfileSummary {
    ChunkingProfileSummary {
        id: CHUNKING_PROFILE_ID.to_owned(),
        strategy: CHUNKING_STRATEGY.to_owned(),
        version: CHUNKING_VERSION.to_owned(),
        target_tokens: TARGET_TOKENS as u32,
        overlap_tokens: OVERLAP_TOKENS as u32,
        maximum_tokens: MAXIMUM_TOKENS as u32,
        maximum_chunks_per_document: MAXIMUM_CHUNKS as u32,
        boundary_policy: BOUNDARY_POLICY.to_owned(),
    }
}

fn active_retrieval_profile_summary() -> RetrievalProfileSummary {
    RetrievalProfileSummary {
        id: "m4-hybrid-scoped-v1".to_owned(),
        maximum_candidates_per_vector_index: MAX_RETRIEVAL_CANDIDATES as u32,
        reciprocal_rank_fusion_k: RRF_K,
        lexical_weight: RRF_LEXICAL_WEIGHT,
        semantic_weight: 1.0,
        maximum_passages_per_paper: MAX_PASSAGES_PER_PAPER as u32,
        scoped_path: "scoped-hybrid-v1".to_owned(),
        scoped_chunk_limit_per_paper: None,
        context_assembler: CONTEXT_ASSEMBLER_VERSION.to_owned(),
        maximum_evidence_sources: MAX_EVIDENCE as u32,
        compact_evidence_budget_tokens: COMPACT_EVIDENCE_BUDGET_TOKENS,
        large_evidence_budget_tokens: LARGE_EVIDENCE_BUDGET_TOKENS,
        token_accounting: TOKEN_ACCOUNTING_PROFILE.to_owned(),
    }
}

#[derive(Clone, Debug, Default)]
struct SemanticStateRow {
    status: String,
    eligible_papers: u64,
    indexed_papers: u64,
    stale_papers: u64,
    eligible_documents: u64,
    chunked_documents: u64,
    generated_chunks: u64,
    indexed_chunks: u64,
    stale_chunks: u64,
    metadata_only_papers: u64,
    vector_index_bytes: u64,
    chunk_metadata_bytes: u64,
    last_indexed_at: Option<String>,
    last_error: Option<String>,
}

fn active_profile(connection: &Connection) -> AppResult<Option<EmbeddingProfileSummary>> {
    connection
        .query_row(
            "SELECT id, provider, model_identifier, model_revision,
                    artifact_identity, embedding_dimension, similarity_metric,
                    query_template, document_template, normalization,
                    runtime_version, vector_backend,
                    vector_index_format_version
             FROM embedding_profiles WHERE active = 1",
            [],
            |row| {
                Ok(EmbeddingProfileSummary {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    model_identifier: row.get(2)?,
                    model_revision: row.get(3)?,
                    artifact_identity: row.get(4)?,
                    embedding_dimension: row.get(5)?,
                    similarity_metric: row.get(6)?,
                    query_template: row.get(7)?,
                    document_template: row.get(8)?,
                    normalization: row.get(9)?,
                    runtime_version: row.get(10)?,
                    vector_backend: row.get(11)?,
                    vector_index_format_version: row.get(12)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn active_library_id(connection: &Connection) -> AppResult<Option<String>> {
    connection
        .query_row(
            "SELECT value FROM app_meta WHERE key = 'active_library_id'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn embedding_profile_id(metadata: &EmbeddingProviderMetadata) -> String {
    stable_semantic_id(
        "embedding-profile",
        &[
            &metadata.provider,
            &metadata.model_identifier,
            &metadata.artifact_identity,
            &metadata.dimension.to_string(),
            &metadata.metric,
            &metadata.query_template,
            &metadata.document_template,
            &metadata.normalization,
            VECTOR_BACKEND,
            VECTOR_FORMAT_VERSION,
        ],
    )
}

fn stable_semantic_id(namespace: &str, parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    digest.update(namespace.as_bytes());
    for part in parts {
        digest.update([0]);
        digest.update(part.as_bytes());
    }
    let mut identifier = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut identifier, "{byte:02x}").expect("write semantic identifier");
    }
    identifier
}

fn model_missing_error() -> AppError {
    AppError::Semantic(
        "The local semantic model is not installed. Install it from Data Source; Keyword search remains available."
            .to_owned(),
    )
}

fn normalize_vectors(mut vectors: Vec<Vec<f32>>) -> AppResult<Vec<Vec<f32>>> {
    for vector in &mut vectors {
        validate_dimension(vector)?;
        let norm = vector
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            .sqrt();
        if norm <= f64::EPSILON {
            return Err(AppError::Semantic(
                "The local model returned a zero-length vector".to_owned(),
            ));
        }
        for value in vector {
            *value = (*value as f64 / norm) as f32;
        }
    }
    Ok(vectors)
}

fn normalize_vector(mut vector: Vec<f32>) -> AppResult<Vec<f32>> {
    normalize_vectors(vec![std::mem::take(&mut vector)])?
        .pop()
        .ok_or_else(|| AppError::Semantic("The embedding vector is empty".to_owned()))
}

fn validate_dimension(vector: &[f32]) -> AppResult<()> {
    if vector.len() != EMBEDDING_DIMENSION {
        return Err(AppError::Semantic(format!(
            "The vector dimension is {}, but the active profile requires {EMBEDDING_DIMENSION}",
            vector.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::{Arc, Mutex};

    use rusqlite::{params, Connection};

    use super::model_lifecycle::{
        ModelInstallationMarker, MODEL_FILE, MODEL_MARKER_FILE, TOKENIZER_FILES,
    };
    use super::vector_index::VectorMatch;
    use super::{
        active_profile, cosine_similarity, embedding_profile_id, load_document_sources,
        load_semantic_diagnostics, scientific_chunks, vector_centroid, DocumentSource,
        EmbeddingProvider, EmbeddingProviderMetadata, LocalFastEmbedProvider, SemanticCoordinator,
        UsearchVectorIndex, VectorIndex, DEFAULT_MODEL_ID, DEFAULT_SIMILARITY_THRESHOLD,
        EMBEDDING_DIMENSION,
    };
    use crate::database::ResearchDatabase;
    use crate::error::{AppError, AppResult};
    use crate::extraction::model::{ExtractionInputDescriptor, ExtractionRequest};
    use crate::extraction::ExtractionCoordinator;
    use crate::model::{SearchRequest, SemanticIndexRequest};

    #[test]
    fn complementary_search_keeps_keyword_hit_before_passage_cap() {
        let mut candidate = super::SemanticCandidate::new("paper".into());
        for rank in 1..=4 {
            candidate.passages.push(crate::model::SemanticPassage {
                chunk_id: format!("chunk-{rank}"),
                document_id: "document".into(),
                document_revision_id: 1,
                text: "fixture passage".into(),
                snippet: "fixture passage".into(),
                character_start: 0,
                character_end: 15,
                page_start: Some(rank),
                page_end: Some(rank),
                semantic_score: 0.5,
                lexical_rank: Some(rank),
                semantic_rank: Some(if rank == 1 { 100 } else { rank - 1 }),
                embedding_profile_id: "profile".into(),
            });
        }
        let mut ordinary = candidate.clone();
        ordinary.finish(false);
        assert_eq!(ordinary.passages.len(), 3);
        assert!(ordinary.passages.iter().all(|p| p.chunk_id != "chunk-1"));
        candidate.finish(true);
        assert_eq!(candidate.passages.len(), 3);
        assert_eq!(candidate.passages[0].chunk_id, "chunk-1");
    }

    #[derive(Default)]
    struct FakeProvider;

    impl FakeProvider {
        fn metadata() -> EmbeddingProviderMetadata {
            EmbeddingProviderMetadata {
                provider: "test-provider".to_owned(),
                model_identifier: "test-semantic-model".to_owned(),
                model_revision: "revision-a".to_owned(),
                artifact_identity: "artifact-a".to_owned(),
                dimension: EMBEDDING_DIMENSION,
                metric: "cosine".to_owned(),
                query_template: "{query}".to_owned(),
                document_template: "{document}".to_owned(),
                normalization: "l2-unit".to_owned(),
                runtime_version: "test-runtime".to_owned(),
                installed_bytes: 512,
            }
        }

        fn vector(text: &str) -> Vec<f32> {
            let normalized = text.to_lowercase();
            let mut vector = vec![0.0_f32; EMBEDDING_DIMENSION];
            for (terms, index) in [
                (["ambient", "noise", "interferometry"], 0),
                (["surface", "dispersion", "crust"], 1),
                (["bayesian", "inversion", "posterior"], 2),
            ] {
                vector[index] = terms
                    .iter()
                    .filter(|term| normalized.contains(**term))
                    .count() as f32;
            }
            if vector.iter().all(|value| *value == 0.0) {
                vector[3] = 1.0;
            }
            super::normalize_vector(vector).expect("normalize fake vector")
        }
    }

    impl EmbeddingProvider for FakeProvider {
        fn installed_metadata(&self) -> AppResult<Option<EmbeddingProviderMetadata>> {
            Ok(Some(Self::metadata()))
        }

        fn install(&self) -> AppResult<EmbeddingProviderMetadata> {
            Ok(Self::metadata())
        }

        fn embed_documents(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|text| Self::vector(text)).collect())
        }

        fn embed_query(&self, query: &str) -> AppResult<Vec<f32>> {
            Ok(Self::vector(query))
        }

        fn token_count(&self, text: &str) -> AppResult<usize> {
            Ok(text.split_whitespace().count())
        }
    }

    #[derive(Default)]
    struct CountingProvider {
        document_embeddings: AtomicUsize,
    }

    impl EmbeddingProvider for CountingProvider {
        fn installed_metadata(&self) -> AppResult<Option<EmbeddingProviderMetadata>> {
            Ok(Some(FakeProvider::metadata()))
        }

        fn install(&self) -> AppResult<EmbeddingProviderMetadata> {
            Ok(FakeProvider::metadata())
        }

        fn embed_documents(&self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
            self.document_embeddings
                .fetch_add(texts.len(), AtomicOrdering::SeqCst);
            Ok(texts
                .iter()
                .map(|text| FakeProvider::vector(text))
                .collect())
        }

        fn embed_query(&self, query: &str) -> AppResult<Vec<f32>> {
            Ok(FakeProvider::vector(query))
        }

        fn token_count(&self, text: &str) -> AppResult<usize> {
            Ok(text.split_whitespace().count())
        }
    }

    #[derive(Default)]
    struct MemoryVectorIndex {
        vectors: Mutex<HashMap<(String, String, u64), Vec<f32>>>,
    }

    impl VectorIndex for MemoryVectorIndex {
        fn upsert(
            &self,
            profile_id: &str,
            kind: &str,
            vectors: &[(u64, Vec<f32>)],
        ) -> AppResult<()> {
            let mut stored = self
                .vectors
                .lock()
                .map_err(|_| AppError::Semantic("test vector lock".to_owned()))?;
            for (key, vector) in vectors {
                stored.insert(
                    (profile_id.to_owned(), kind.to_owned(), *key),
                    vector.clone(),
                );
            }
            Ok(())
        }

        fn remove(&self, profile_id: &str, kind: &str, keys: &[u64]) -> AppResult<()> {
            let mut stored = self
                .vectors
                .lock()
                .map_err(|_| AppError::Semantic("test vector lock".to_owned()))?;
            for key in keys {
                stored.remove(&(profile_id.to_owned(), kind.to_owned(), *key));
            }
            Ok(())
        }

        fn nearest(
            &self,
            profile_id: &str,
            kind: &str,
            query: &[f32],
            limit: usize,
            allowed_keys: &HashSet<u64>,
        ) -> AppResult<Vec<VectorMatch>> {
            let stored = self
                .vectors
                .lock()
                .map_err(|_| AppError::Semantic("test vector lock".to_owned()))?;
            let mut matches = stored
                .iter()
                .filter(|((profile, stored_kind, key), _)| {
                    profile == profile_id && stored_kind == kind && allowed_keys.contains(key)
                })
                .map(|((_, _, key), vector)| {
                    Ok(VectorMatch {
                        key: *key,
                        similarity: cosine_similarity(query, vector)?,
                    })
                })
                .collect::<AppResult<Vec<_>>>()?;
            matches.sort_by(|left, right| {
                right
                    .similarity
                    .total_cmp(&left.similarity)
                    .then_with(|| left.key.cmp(&right.key))
            });
            matches.truncate(limit);
            Ok(matches)
        }

        fn vector(&self, profile_id: &str, kind: &str, key: u64) -> AppResult<Option<Vec<f32>>> {
            Ok(self
                .vectors
                .lock()
                .map_err(|_| AppError::Semantic("test vector lock".to_owned()))?
                .get(&(profile_id.to_owned(), kind.to_owned(), key))
                .cloned())
        }

        fn entry_count(&self, profile_id: &str, kind: &str) -> AppResult<u64> {
            Ok(self
                .vectors
                .lock()
                .map_err(|_| AppError::Semantic("test vector lock".to_owned()))?
                .keys()
                .filter(|(profile, stored_kind, _)| profile == profile_id && stored_kind == kind)
                .count() as u64)
        }

        fn rebuild(&self, profile_id: &str) -> AppResult<()> {
            self.vectors
                .lock()
                .map_err(|_| AppError::Semantic("test vector lock".to_owned()))?
                .retain(|(profile, _, _), _| profile != profile_id);
            Ok(())
        }

        fn storage_bytes(&self, _profile_id: &str) -> u64 {
            0
        }
    }

    fn temporary_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "graphite-semantic-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ))
    }

    fn model_marker(snapshot_relative_path: &str) -> ModelInstallationMarker {
        ModelInstallationMarker {
            model_identifier: DEFAULT_MODEL_ID.to_owned(),
            artifact_identity: "test-artifact".to_owned(),
            snapshot_relative_path: snapshot_relative_path.to_owned(),
            installed_bytes: 1_234,
            installed_at: "2026-08-25T00:00:00Z".to_owned(),
        }
    }

    fn write_complete_model_snapshot(root: &Path, relative: &str) -> PathBuf {
        let snapshot = root.join(relative);
        fs::create_dir_all(&snapshot).expect("model snapshot");
        for file in std::iter::once(MODEL_FILE).chain(TOKENIZER_FILES.iter().copied()) {
            fs::write(snapshot.join(file), format!("fixture:{file}")).expect("model asset");
        }
        snapshot
    }

    fn seeded_database(name: &str) -> (ResearchDatabase, PathBuf) {
        let path = temporary_path(name).with_extension("sqlite3");
        let database = ResearchDatabase::initialize(path.clone()).expect("database");
        let connection = database.connect().expect("connection");
        connection
            .execute(
                "INSERT INTO sources (
                    id, server_id, api_version, schema_version,
                    compatibility, first_seen_at, last_seen_at,
                    connection_status, last_error
                 ) VALUES ('source', 'server', '3', NULL, 'compatible',
                           'now', 'now', 'ready', NULL)",
                [],
            )
            .expect("source");
        connection
            .execute(
                "INSERT INTO libraries (
                    id, source_id, kind, zotero_library_id, name,
                    successful_version, last_synced_at, last_error
                 ) VALUES ('library', 'source', 'user', '0', 'Research',
                           1, 'now', NULL)",
                [],
            )
            .expect("library");
        connection
            .execute(
                "INSERT INTO app_meta(key, value)
                 VALUES ('active_library_id', 'library')",
                [],
            )
            .expect("active library");
        for (id, key, title, abstract_text, doi, year) in [
            (
                "paper-noise",
                "NOISE",
                "Ambient noise interferometry",
                "Ambient noise correlations recover surface waves.",
                "10.1000/noise",
                2021,
            ),
            (
                "paper-bayes",
                "BAYES",
                "Bayesian crustal inversion",
                "Bayesian posterior sampling estimates crustal dispersion.",
                "10.1000/bayes",
                2023,
            ),
            (
                "paper-other",
                "OTHER",
                "Earthquake rupture dynamics",
                "Dynamic rupture propagation and stress drop.",
                "10.1000/other",
                2019,
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO research_items (
                        id, source_id, library_id, zotero_key, item_type,
                        title, abstract_text, publication_year, doi,
                        source_version, is_trashed
                     ) VALUES (?1, 'source', 'library', ?2, 'journalArticle',
                               ?3, ?4, ?5, ?6, 1, 0)",
                    params![id, key, title, abstract_text, year, doi],
                )
                .expect("paper");
            connection
                .execute(
                    "INSERT INTO search_content (
                        item_id, library_id, title, creators, tags,
                        abstract_text, venue, body
                     ) VALUES (?1, 'library', ?2, '', '', ?3, '', '')",
                    params![id, title, abstract_text],
                )
                .expect("search content");
        }
        connection
            .execute(
                "INSERT INTO attachments (
                    id, source_id, library_id, zotero_key, parent_item_id,
                    title, content_type, filename, link_mode,
                    available_locally, source_version
                 ) VALUES (
                    'attachment-noise', 'source', 'library', 'PDF-NOISE',
                    'paper-noise', 'Noise paper PDF', 'application/pdf',
                    'noise.pdf', 'imported_file', 1, 1
                 )",
                [],
            )
            .expect("attachment");
        connection
            .execute(
                "INSERT INTO documents (
                    id, source_id, library_id, attachment_id,
                    research_item_id, zotero_attachment_key, media_type,
                    title, filename, link_mode, source_version,
                    availability, created_at, updated_at
                 ) VALUES (
                    'document-noise', 'source', 'library',
                    'attachment-noise', 'paper-noise', 'PDF-NOISE',
                    'application/pdf', 'Noise paper PDF', 'noise.pdf',
                    'imported_file', 1, 'ready', 'now', 'now'
                 )",
                [],
            )
            .expect("document");
        connection
            .execute(
                "INSERT INTO document_revisions (
                    document_id, source_version, revision_key, discovered_at
                 ) VALUES (
                    'document-noise', 1, 'text:zotero-indexed:1', 'now'
                 )",
                [],
            )
            .expect("document revision");
        connection
            .execute(
                "INSERT INTO document_texts (
                    id, document_id, source_id, library_id,
                    zotero_attachment_key, provider, source_version,
                    content, indexed_pages, total_pages, indexed_chars,
                    total_chars, completeness, fetched_at
                 ) VALUES (
                    'text-noise', 'document-noise', 'source', 'library',
                    'PDF-NOISE', 'zotero-indexed', 1,
                    'Ambient seismic noise correlations recover empirical Green functions.\n\nRayleigh phase velocity constrains crustal shear-wave structure.',
                    2, 2, 130, 130, 'full', 'now'
                 )",
                [],
            )
            .expect("document text");
        (database, path)
    }

    fn request(query: &str, mode: &str) -> SearchRequest {
        SearchRequest {
            query: query.to_owned(),
            mode: Some(mode.to_owned()),
            limit: Some(20),
            offset: Some(0),
            item_ids: Vec::new(),
            collection_id: None,
            citation_filter: None,
            year_from: None,
            year_to: None,
            item_type: None,
        }
    }

    #[test]
    fn chunks_at_scientific_boundaries_and_excludes_bibliography() {
        let provider = FakeProvider;
        let text = "Introduction. Ambient noise provides observations.\n\n\
                    Methods. Surface waves constrain the croûte.\n\n\
                    References\nIgnored bibliography entry.";
        let body_end = text.find("References").expect("references boundary");
        let chunks = scientific_chunks(
            &provider,
            &DocumentSource {
                item_id: "paper".to_owned(),
                library_id: "library".to_owned(),
                document_id: "document".to_owned(),
                document_revision_id: 7,
                provider: "test".to_owned(),
                text: text.to_owned(),
                body_end_byte: body_end,
                bibliography_excluded: true,
            },
        )
        .expect("chunks");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, text[..body_end].trim());
        assert_eq!(chunks[0].character_start, 0);
        assert_eq!(
            chunks[0].character_end,
            text[..body_end].trim().chars().count()
        );
        assert!(!chunks[0].text.contains("Ignored"));
    }

    #[test]
    fn chunking_preserves_budgets_overlap_offsets_and_deterministic_ids() {
        let provider = FakeProvider;
        let sentences = (0..12)
            .map(|sentence| {
                let mut words = (0..20)
                    .map(|word| format!("s{sentence:02}_{word:02}"))
                    .collect::<Vec<_>>();
                words[19].push('.');
                words.join(" ")
            })
            .collect::<Vec<_>>();
        let text = sentences.join(" ");
        let source = DocumentSource {
            item_id: "paper".to_owned(),
            library_id: "library".to_owned(),
            document_id: "document".to_owned(),
            document_revision_id: 17,
            provider: "test".to_owned(),
            body_end_byte: text.len(),
            bibliography_excluded: false,
            text,
        };

        let chunks = scientific_chunks(&provider, &source).expect("chunks");
        let repeated = scientific_chunks(&provider, &source).expect("repeat chunks");
        let expected_first = sentences[..11].join(" ");
        let expected_second = sentences[10..].join(" ");
        let second_start = source.text.find(&sentences[10]).expect("overlap start");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].ordinal, 0);
        assert_eq!(chunks[0].text, expected_first);
        assert_eq!(chunks[0].token_count, 220);
        assert_eq!(chunks[0].character_start, 0);
        assert_eq!(chunks[0].character_end, expected_first.chars().count());
        assert_eq!(chunks[1].ordinal, 1);
        assert_eq!(chunks[1].text, expected_second);
        assert_eq!(chunks[1].token_count, 40);
        assert_eq!(chunks[1].character_start, second_start);
        assert_eq!(
            chunks[1].character_end,
            second_start + expected_second.chars().count()
        );
        assert!(chunks.iter().all(|chunk| chunk.token_count <= 360));
        assert_eq!(chunks[0].id, repeated[0].id);
        assert_eq!(chunks[1].id, repeated[1].id);
        assert_eq!(chunks[0].fingerprint, repeated[0].fingerprint);
        assert_eq!(chunks[1].fingerprint, repeated[1].fingerprint);
        assert_eq!(
            chunks[0].id,
            super::stable_semantic_id(
                "semantic-chunk",
                &[
                    "17",
                    super::CHUNKING_PROFILE_ID,
                    "0",
                    &chunks[0].fingerprint,
                ],
            )
        );
    }

    #[test]
    fn benchmark_canonical_projection_does_not_change_existing_chunks() {
        let fixture = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tools/pdf-benchmark/fixtures/canonical-unstructured.json"
        ));
        let canonical: crate::extraction::model::CanonicalDocumentV1 =
            serde_json::from_str(fixture).expect("benchmark canonical fixture");
        let existing_text = "Line one.\n\n  Indented 3.2e-4\tvalue.\n";
        let projected = crate::extraction::projection::render_plain_text(&canonical);
        assert_eq!(projected, existing_text);
        let source = |text: String| DocumentSource {
            item_id: "paper".to_owned(),
            library_id: "library".to_owned(),
            document_id: "document".to_owned(),
            document_revision_id: 17,
            provider: "zotero-fulltext".to_owned(),
            body_end_byte: text.len(),
            bibliography_excluded: false,
            text,
        };
        let before = scientific_chunks(&FakeProvider, &source(existing_text.to_owned()))
            .expect("existing chunks");
        let after = scientific_chunks(&FakeProvider, &source(projected)).expect("projected chunks");
        let snapshot = |chunks: &[super::ScientificChunk]| {
            chunks
                .iter()
                .map(|chunk| {
                    (
                        chunk.id.clone(),
                        chunk.ordinal,
                        chunk.text.clone(),
                        chunk.character_start,
                        chunk.character_end,
                        chunk.fingerprint.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(snapshot(&before), snapshot(&after));
    }

    #[test]
    fn chunking_preserves_empty_short_and_oversized_document_behavior() {
        let provider = FakeProvider;
        let source = |text: String| DocumentSource {
            item_id: "paper".to_owned(),
            library_id: "library".to_owned(),
            document_id: "document".to_owned(),
            document_revision_id: 23,
            provider: "test".to_owned(),
            body_end_byte: text.len(),
            bibliography_excluded: false,
            text,
        };

        assert!(scientific_chunks(&provider, &source(" \n\n ".to_owned()))
            .expect("empty chunks")
            .is_empty());
        let short =
            scientific_chunks(&provider, &source("Result.".to_owned())).expect("short chunk");
        assert_eq!(short.len(), 1);
        assert_eq!(short[0].text, "Result.");
        assert_eq!(short[0].token_count, 1);

        let oversized_text = (0..800)
            .map(|index| format!("token{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let oversized = scientific_chunks(&provider, &source(oversized_text.clone()))
            .expect("oversized chunks");
        assert_eq!(
            oversized
                .iter()
                .map(|chunk| chunk.token_count)
                .collect::<Vec<_>>(),
            vec![359, 359, 82]
        );
        assert!(oversized.iter().all(|chunk| chunk.token_count <= 360));
        let actual_second_start = oversized_text.find("token359").expect("second start");
        assert_eq!(oversized[1].character_start + 1, actual_second_start);
    }

    #[test]
    fn chunking_caps_unusually_large_documents_at_512_chunks() {
        let provider = FakeProvider;
        let text = (0..513)
            .map(|unit| {
                let mut words = vec!["token".to_owned(); 221];
                words[0] = format!("unit{unit}");
                words[220].push('.');
                words.join(" ")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let chunks = scientific_chunks(
            &provider,
            &DocumentSource {
                item_id: "paper".to_owned(),
                library_id: "library".to_owned(),
                document_id: "document".to_owned(),
                document_revision_id: 29,
                provider: "test".to_owned(),
                body_end_byte: text.len(),
                bibliography_excluded: false,
                text,
            },
        )
        .expect("capped chunks");

        assert_eq!(chunks.len(), 512);
        assert_eq!(chunks[0].ordinal, 0);
        assert!(chunks[0].text.starts_with("unit0 "));
        assert_eq!(chunks[511].ordinal, 511);
        assert!(chunks[511].text.starts_with("unit511 "));
        assert!(chunks.iter().all(|chunk| chunk.token_count == 221));
    }

    #[test]
    fn diagnostics_expose_runtime_profiles_chunk_cap_and_physical_vector_consistency() {
        let (database, path) = seeded_database("runtime-diagnostics");
        let vectors = Arc::new(MemoryVectorIndex::default());
        let coordinator = SemanticCoordinator::with_components(
            database.clone(),
            Arc::new(FakeProvider),
            vectors.clone(),
        );
        coordinator.install_model().expect("install model profile");
        coordinator
            .index(&SemanticIndexRequest {
                max_papers: Some(8),
                max_documents: Some(2),
            })
            .expect("index semantic fixture");
        let connection = database.connect().expect("connection");
        connection
            .execute(
                "UPDATE semantic_document_states SET chunk_count = 512
                 WHERE document_id = 'document-noise'",
                [],
            )
            .expect("mark measured cap");
        drop(connection);

        let workspace = coordinator.load_workspace().expect("diagnostic workspace");
        assert_eq!(
            workspace
                .semantic
                .active_chunking_profile
                .maximum_chunks_per_document,
            512
        );
        assert_eq!(
            workspace
                .semantic
                .active_retrieval_profile
                .scoped_chunk_limit_per_paper,
            None
        );
        assert_eq!(workspace.semantic.maximum_observed_chunks_per_document, 512);
        assert_eq!(workspace.semantic.documents_at_chunk_limit, 1);
        assert_eq!(workspace.semantic.current_document_revisions_indexed, 1);
        assert_eq!(workspace.semantic.stale_document_revisions_indexed, 0);
        assert_eq!(workspace.semantic.paper_vector_count_matches, Some(true));
        assert_eq!(workspace.semantic.chunk_vector_count_matches, Some(true));

        let profile_id = workspace
            .semantic
            .active_profile
            .as_ref()
            .expect("active profile")
            .id
            .clone();
        let paper_key = database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT vector_key FROM semantic_vector_entries
                 WHERE embedding_profile_id = ?1 AND index_kind = 'paper'
                   AND active = 1 LIMIT 1",
                [&profile_id],
                |row| row.get::<_, i64>(0).map(|value| value.max(0) as u64),
            )
            .expect("paper vector key");
        vectors
            .remove(&profile_id, "paper", &[paper_key])
            .expect("remove physical vector only");
        let mismatched = coordinator.load_workspace().expect("mismatch diagnostics");
        assert_eq!(mismatched.semantic.paper_vector_count_matches, Some(false));
        assert_eq!(
            mismatched.semantic.registered_paper_vectors,
            mismatched.semantic.physical_paper_vectors.unwrap_or(0) + 1
        );

        fs::remove_file(path).expect("remove database fixture");
    }

    #[test]
    fn diagnostics_follow_active_text_instead_of_newer_artifact_revisions() {
        let (database, path) = seeded_database("active-text-diagnostics");
        let coordinator = SemanticCoordinator::with_components(
            database.clone(),
            Arc::new(FakeProvider),
            Arc::new(MemoryVectorIndex::default()),
        );
        coordinator.install_model().expect("install model profile");
        coordinator
            .index(&SemanticIndexRequest {
                max_papers: Some(8),
                max_documents: Some(2),
            })
            .expect("index active text");
        let connection = database.connect().expect("connection");
        connection
            .execute_batch(
                "UPDATE document_texts SET content_hash = 'active-text-hash';
                 INSERT INTO document_revisions (
                     document_id, source_version, revision_key, discovered_at,
                     revision_kind, content_hash
                 ) VALUES
                     ('document-noise', 2, 'file:newer', 'now',
                      'artifact-content', 'pdf-byte-hash'),
                     ('document-noise', 1, 'text:alternate', 'now',
                      'source-provided-text', 'inactive-text-hash');",
            )
            .expect("record newer binary and unselected text revisions");
        let diagnostics = load_semantic_diagnostics(&connection, Some("library"), None)
            .expect("active text diagnostics");
        assert_eq!(diagnostics.current_document_revisions_indexed, 1);
        assert_eq!(diagnostics.stale_document_revisions_indexed, 0);
        let profile = active_profile(&connection)
            .expect("profile lookup")
            .expect("installed profile");
        assert!(
            load_document_sources(&connection, &profile, "library", 2, None)
                .expect("unchanged indexing candidates")
                .is_empty()
        );

        connection
            .execute_batch(
                "INSERT INTO document_revisions (
                     document_id, source_version, revision_key, discovered_at,
                     revision_kind, content_hash
                 ) VALUES ('document-noise', 1, 'text:matching-newer', 'now',
                           'source-provided-text', 'active-text-hash');",
            )
            .expect("record a newer revision of the active text");
        let superseded = load_semantic_diagnostics(&connection, Some("library"), None)
            .expect("superseded revision diagnostics");
        assert_eq!(superseded.current_document_revisions_indexed, 0);
        assert_eq!(superseded.stale_document_revisions_indexed, 1);

        connection
            .execute_batch(
                "INSERT INTO document_revisions (
                     document_id, source_version, revision_key, discovered_at,
                     revision_kind, content_hash
                 ) VALUES ('document-noise', 3, 'text:updated', 'now',
                           'source-provided-text', 'updated-text-hash');
                 UPDATE document_texts SET source_version = 3,
                     content_hash = 'updated-text-hash',
                     content = 'Updated active text requiring reindexing.';",
            )
            .expect("select a changed text revision");
        let stale = load_semantic_diagnostics(&connection, Some("library"), None)
            .expect("changed text diagnostics");
        assert_eq!(stale.current_document_revisions_indexed, 0);
        // Text-change invalidation removes the former index state.
        assert_eq!(stale.stale_document_revisions_indexed, 0);
        assert_eq!(
            load_document_sources(&connection, &profile, "library", 2, None)
                .expect("changed indexing candidate")
                .len(),
            1
        );

        coordinator
            .index(&SemanticIndexRequest {
                max_papers: Some(8),
                max_documents: Some(2),
            })
            .expect("index the updated active text");
        let current = load_semantic_diagnostics(&connection, Some("library"), None)
            .expect("updated index diagnostics");
        assert_eq!(current.current_document_revisions_indexed, 1);
        assert_eq!(current.stale_document_revisions_indexed, 0);
        connection
            .execute("UPDATE document_texts SET is_active = 0", [])
            .expect("remove active text selection");
        let missing = load_semantic_diagnostics(&connection, Some("library"), None)
            .expect("missing active text diagnostics");
        assert_eq!(missing.current_document_revisions_indexed, 0);
        assert_eq!(missing.stale_document_revisions_indexed, 0);
        drop(connection);
        fs::remove_file(path).expect("remove database fixture");
    }

    #[tokio::test]
    async fn canonical_backfill_preserves_chunks_registrations_vectors_and_ordinal_120() {
        let (database, path) = seeded_database("canonical-semantic-preservation");
        let provider = Arc::new(CountingProvider::default());
        let vectors = Arc::new(MemoryVectorIndex::default());
        let coordinator = SemanticCoordinator::with_components(
            database.clone(),
            provider.clone(),
            vectors.clone(),
        );
        coordinator.install_model().expect("install fixture model");
        coordinator
            .index(&SemanticIndexRequest {
                max_papers: Some(8),
                max_documents: Some(2),
            })
            .expect("index fixture");
        let connection = database.connect().expect("connection");
        connection
            .execute(
                "UPDATE semantic_chunks SET ordinal = 120
                 WHERE document_id = 'document-noise'",
                [],
            )
            .expect("place regression chunk at ordinal 120");
        let profile_id: String = connection
            .query_row(
                "SELECT id FROM embedding_profiles WHERE active = 1",
                [],
                |row| row.get(0),
            )
            .expect("active profile");
        let chunks_before = semantic_chunk_snapshot(&connection);
        let registrations_before = vector_registration_snapshot(&connection);
        let active_chunks_before: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM semantic_chunks WHERE status = 'active'",
                [],
                |row| row.get(0),
            )
            .expect("active chunks before");
        let physical_before = (
            vectors
                .entry_count(&profile_id, "paper")
                .expect("paper vectors before"),
            vectors
                .entry_count(&profile_id, "chunk")
                .expect("chunk vectors before"),
        );
        let embeddings_before = provider.document_embeddings.load(AtomicOrdering::SeqCst);
        drop(connection);

        let summary = ExtractionCoordinator::new(database.clone())
            .backfill(16)
            .await
            .expect("canonical backfill");
        assert_eq!(summary.processed, 1);
        assert_eq!(summary.remaining, 0);

        let connection = database.connect().expect("connection");
        assert_eq!(semantic_chunk_snapshot(&connection), chunks_before);
        assert_eq!(
            vector_registration_snapshot(&connection),
            registrations_before
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM semantic_chunks WHERE status = 'active'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("active chunks after"),
            active_chunks_before
        );
        assert_eq!(
            (
                vectors
                    .entry_count(&profile_id, "paper")
                    .expect("paper vectors after"),
                vectors
                    .entry_count(&profile_id, "chunk")
                    .expect("chunk vectors after"),
            ),
            physical_before
        );
        assert_eq!(
            provider.document_embeddings.load(AtomicOrdering::SeqCst),
            embeddings_before
        );
        let mut scoped = request("empirical Green functions", "hybrid");
        scoped.item_ids = vec!["paper-noise".to_owned()];
        let results = coordinator.search(&scoped).expect("scoped retrieval");
        assert!(results
            .results
            .iter()
            .any(|item| item.item.id == "paper-noise"));
        assert_eq!(
            connection
                .query_row(
                    "SELECT ordinal FROM semantic_chunks
                     WHERE document_id = 'document-noise'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("preserved ordinal"),
            120
        );
        let active_selection_before = connection
            .query_row(
                "SELECT extraction_id, quality_report_id, routing_profile_id
                 FROM active_extraction_selections
                 WHERE artifact_id = 'document-noise'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .expect("active selection before shadow");
        let active_text_before = connection
            .query_row(
                "SELECT id, content, content_hash, extraction_id
                 FROM document_texts
                 WHERE document_id = 'document-noise' AND is_active = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .expect("active text before shadow");
        let fts_body_before: String = connection
            .query_row(
                "SELECT body FROM search_content WHERE item_id = 'paper-noise'",
                [],
                |row| row.get(0),
            )
            .expect("FTS source before shadow");
        let (artifact_revision_id, input_content_hash): (i64, String) = connection
            .query_row(
                "SELECT artifact_revision_id,
                        COALESCE(plain_text_hash, canonical_hash)
                 FROM document_extractions WHERE id = ?1",
                [&active_selection_before.0],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("shadow request revision");
        drop(connection);
        let mut shadow_attempt =
            crate::extraction::quality::tests::fixture_attempt(true, "different shadow text");
        shadow_attempt.provider.profile_id = "xberg-native-shadow-v1".to_owned();
        shadow_attempt.provider.provider_id = "xberg".to_owned();
        shadow_attempt.provider.provider_version = "1.0.14".to_owned();
        shadow_attempt.provider.mapping_profile = "xberg-native-to-canonical-v1".to_owned();
        shadow_attempt.observations.provider_profile_id =
            shadow_attempt.provider.profile_id.clone();
        shadow_attempt.observations.artifact_id = "document-noise".to_owned();
        shadow_attempt.observations.artifact_revision_id = artifact_revision_id;
        shadow_attempt.observations.input_content_hash = input_content_hash.clone();
        shadow_attempt.observations.provider_diagnostics.insert(
            "shadowTerminalOutcome".to_owned(),
            serde_json::json!("succeeded"),
        );
        let canonical = shadow_attempt
            .canonical_document
            .as_mut()
            .expect("shadow canonical fixture");
        canonical.artifact_id = "document-noise".to_owned();
        canonical.artifact_revision_id = artifact_revision_id;
        canonical.extraction_profile_id = shadow_attempt.provider.profile_id.clone();
        crate::extraction::shadow::persist_fixture_shadow_attempt(
            database.clone(),
            &ExtractionRequest {
                artifact_id: "document-noise".to_owned(),
                artifact_revision_id,
                input_content_hash,
                input: ExtractionInputDescriptor {
                    media_type: "application/pdf".to_owned(),
                    has_local_file: true,
                    has_bounded_reader: false,
                    has_source_provided_text: true,
                },
                source_text: None,
            },
            &shadow_attempt,
        )
        .expect("persist shadow attempt");
        let connection = database.connect().expect("connection after shadow");
        assert_eq!(
            connection
                .query_row(
                    "SELECT extraction_id, quality_report_id, routing_profile_id
                     FROM active_extraction_selections
                     WHERE artifact_id = 'document-noise'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .expect("active selection after shadow"),
            active_selection_before
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT id, content, content_hash, extraction_id
                     FROM document_texts
                     WHERE document_id = 'document-noise' AND is_active = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .expect("active text after shadow"),
            active_text_before
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT body FROM search_content WHERE item_id = 'paper-noise'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("FTS source after shadow"),
            fts_body_before
        );
        assert_eq!(semantic_chunk_snapshot(&connection), chunks_before);
        assert_eq!(
            vector_registration_snapshot(&connection),
            registrations_before
        );
        assert_eq!(
            (
                vectors
                    .entry_count(&profile_id, "paper")
                    .expect("paper vectors after shadow"),
                vectors
                    .entry_count(&profile_id, "chunk")
                    .expect("chunk vectors after shadow"),
            ),
            physical_before
        );
        assert_eq!(
            provider.document_embeddings.load(AtomicOrdering::SeqCst),
            embeddings_before
        );
        fs::remove_file(path).expect("remove database fixture");
    }

    fn semantic_chunk_snapshot(connection: &Connection) -> Vec<(String, i64, i64, String, String)> {
        let mut statement = connection
            .prepare(
                "SELECT id, ordinal, vector_key, status, text_fingerprint
                 FROM semantic_chunks ORDER BY id",
            )
            .expect("chunk snapshot statement");
        statement
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .expect("chunk snapshot rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("chunk snapshot")
    }

    fn vector_registration_snapshot(connection: &Connection) -> Vec<(i64, String, i64)> {
        let mut statement = connection
            .prepare(
                "SELECT vector_key, target_id, active
                 FROM semantic_vector_entries ORDER BY vector_key",
            )
            .expect("vector registration statement");
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .expect("vector registration rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("vector registrations")
    }

    #[test]
    fn interrupted_indexing_refreshes_counts_and_resumes_without_duplicates() {
        let (database, path) = seeded_database("interrupted-semantic-index");
        let provider = Arc::new(CountingProvider::default());
        let vectors = Arc::new(MemoryVectorIndex::default());
        let coordinator = SemanticCoordinator::with_components(
            database.clone(),
            provider.clone(),
            vectors.clone(),
        );
        coordinator.install_model().expect("install model profile");
        let request = SemanticIndexRequest {
            max_papers: Some(64),
            max_documents: Some(16),
        };
        let indexed = coordinator.index(&request).expect("index fixture");
        let connection = database.connect().expect("connection");
        let chunks = semantic_chunk_snapshot(&connection);
        let registrations = vector_registration_snapshot(&connection);
        let embeddings = provider.document_embeddings.load(AtomicOrdering::SeqCst);
        connection
            .execute(
                "UPDATE semantic_index_state SET status = 'indexing',
             chunked_documents = 0, indexed_chunks = 0",
                [],
            )
            .expect("simulate crash before batch summary");
        let restarted =
            SemanticCoordinator::with_components(database.clone(), provider.clone(), vectors);
        restarted
            .recover_interrupted_indexing()
            .expect("recover interruption");
        let recovered = restarted.load_workspace().expect("recovered workspace");
        assert!(!recovered.semantic.indexing);
        assert_eq!(
            recovered.semantic.chunked_documents,
            indexed.semantic.chunked_documents
        );
        assert_eq!(
            recovered.semantic.indexed_chunks,
            indexed.semantic.indexed_chunks
        );
        assert!(recovered
            .semantic
            .last_error
            .as_deref()
            .unwrap()
            .contains("interrupted"));
        assert_eq!(semantic_chunk_snapshot(&connection), chunks);
        assert_eq!(vector_registration_snapshot(&connection), registrations);
        let resumed = restarted.index(&request).expect("resume unchanged corpus");
        assert!(resumed.semantic.last_error.is_none());
        assert_eq!(semantic_chunk_snapshot(&connection), chunks);
        assert_eq!(vector_registration_snapshot(&connection), registrations);
        assert_eq!(
            provider.document_embeddings.load(AtomicOrdering::SeqCst),
            embeddings
        );
        restarted
            .recover_interrupted_indexing()
            .expect("repeat startup recovery");
        assert!(restarted
            .load_workspace()
            .unwrap()
            .semantic
            .last_error
            .is_none());
        fs::remove_file(path).expect("remove database fixture");
    }

    #[test]
    fn indexing_persists_exact_chunk_text_offsets_and_null_location_metadata() {
        let (database, path) = seeded_database("persisted-chunk-boundaries");
        let vectors = Arc::new(MemoryVectorIndex::default());
        let coordinator =
            SemanticCoordinator::with_components(database.clone(), Arc::new(FakeProvider), vectors);
        coordinator.install_model().expect("install model profile");
        coordinator
            .index(&SemanticIndexRequest {
                max_papers: Some(8),
                max_documents: Some(2),
            })
            .expect("index document chunks");

        let connection = database.connect().expect("connection");
        let source_text = connection
            .query_row(
                "SELECT content FROM document_texts WHERE document_id = 'document-noise'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("source text");
        let persisted = connection
            .query_row(
                "SELECT id, document_revision_id, character_start, character_end,
                        page_start, page_end, section_heading, ordinal,
                        token_count, text, text_fingerprint
                 FROM semantic_chunks
                 WHERE document_id = 'document-noise' ORDER BY ordinal",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                    ))
                },
            )
            .expect("persisted chunk");
        let revision = persisted.1.to_string();
        assert_eq!(persisted.2, 0);
        assert_eq!(persisted.3, source_text.chars().count() as i64);
        assert_eq!(persisted.4, None);
        assert_eq!(persisted.5, None);
        assert_eq!(persisted.6, None);
        assert_eq!(persisted.7, 0);
        assert_eq!(persisted.8, source_text.split_whitespace().count() as i64);
        assert_eq!(persisted.9, source_text);
        assert_eq!(
            persisted.0,
            super::stable_semantic_id(
                "semantic-chunk",
                &[&revision, super::CHUNKING_PROFILE_ID, "0", &persisted.10,],
            )
        );
        drop(connection);
        fs::remove_file(path).expect("remove database fixture");
    }

    #[test]
    fn profile_identity_changes_with_model_artifact() {
        let original = FakeProvider::metadata();
        let mut changed = original.clone();
        changed.artifact_identity = "artifact-b".to_owned();
        assert_ne!(
            embedding_profile_id(&original),
            embedding_profile_id(&changed)
        );
    }

    #[test]
    fn model_lifecycle_reports_absent_and_partial_installations_without_network_fallback() {
        let root = temporary_path("model-absent-partial");
        let partial_snapshot = root
            .join("models--Qdrant--bge-small-en-v1.5-onnx-Q")
            .join("snapshots")
            .join("partial");
        fs::create_dir_all(&partial_snapshot).expect("partial snapshot");
        fs::write(partial_snapshot.join(MODEL_FILE), b"partial").expect("partial model asset");
        let provider = LocalFastEmbedProvider::new(root.clone()).expect("provider");

        assert!(provider
            .installed_metadata()
            .expect("missing marker status")
            .is_none());
        assert!(!root.join(MODEL_MARKER_FILE).exists());
        let error = provider
            .embed_query("offline query")
            .expect_err("ordinary inference must not install the model");
        assert!(error.to_string().contains("not installed"));
        assert!(!root.join(MODEL_MARKER_FILE).exists());

        fs::remove_dir_all(root).expect("remove partial model fixture");
    }

    #[test]
    fn model_lifecycle_preserves_marker_format_metadata_and_idempotent_install() {
        let root = temporary_path("model-installed");
        fs::create_dir_all(&root).expect("model root");
        let relative = "models--Qdrant--bge-small-en-v1.5-onnx-Q/snapshots/revision-a";
        write_complete_model_snapshot(&root, relative);
        let marker = model_marker(relative);
        let marker_bytes = serde_json::to_vec_pretty(&marker).expect("marker");
        fs::write(root.join(MODEL_MARKER_FILE), &marker_bytes).expect("write marker");
        let provider = LocalFastEmbedProvider::new(root.clone()).expect("provider");

        let metadata = provider
            .installed_metadata()
            .expect("installed status")
            .expect("installed metadata");
        assert_eq!(metadata.model_identifier, DEFAULT_MODEL_ID);
        assert_eq!(metadata.model_revision, "test-artifact");
        assert_eq!(metadata.artifact_identity, "test-artifact");
        assert_eq!(metadata.installed_bytes, 1_234);

        let repeated = provider.install().expect("already-installed fast path");
        assert_eq!(repeated.model_identifier, metadata.model_identifier);
        assert_eq!(repeated.artifact_identity, metadata.artifact_identity);
        assert_eq!(repeated.installed_bytes, metadata.installed_bytes);
        assert_eq!(
            fs::read(root.join(MODEL_MARKER_FILE)).expect("unchanged marker"),
            marker_bytes
        );

        let serialized = serde_json::to_value(marker).expect("marker shape");
        let fields = serialized.as_object().expect("marker object");
        assert_eq!(fields.len(), 5);
        assert_eq!(fields["model_identifier"], DEFAULT_MODEL_ID);
        assert_eq!(fields["artifact_identity"], "test-artifact");
        assert_eq!(fields["snapshot_relative_path"], relative);
        assert_eq!(fields["installed_bytes"], 1_234);
        assert_eq!(fields["installed_at"], "2026-08-25T00:00:00Z");

        fs::remove_dir_all(root).expect("remove installed model fixture");
    }

    #[test]
    fn model_lifecycle_selects_the_last_sorted_download_snapshot() {
        let root = temporary_path("model-snapshot-selection");
        let snapshots = root
            .join("models--Qdrant--bge-small-en-v1.5-onnx-Q")
            .join("snapshots");
        for revision in ["revision-a", "revision-c", "revision-b"] {
            let snapshot = snapshots.join(revision);
            fs::create_dir_all(&snapshot).expect("snapshot");
            fs::write(snapshot.join(MODEL_FILE), revision).expect("ONNX fixture");
        }
        let provider = LocalFastEmbedProvider::new(root.clone()).expect("provider");

        assert_eq!(
            provider
                .lifecycle
                .discover_snapshot()
                .expect("selected snapshot"),
            snapshots.join("revision-c")
        );

        fs::remove_dir_all(root).expect("remove snapshot fixture");
    }

    #[test]
    fn rejects_incomplete_or_unsafe_model_markers_without_network_fallback() {
        let root = temporary_path("model-validation");
        fs::create_dir_all(&root).expect("model root");
        let provider = LocalFastEmbedProvider::new(root.clone()).expect("provider");
        let marker = model_marker("../outside");
        fs::write(
            root.join(MODEL_MARKER_FILE),
            serde_json::to_vec(&marker).expect("marker"),
        )
        .expect("write marker");
        let error = provider
            .installed_metadata()
            .expect_err("unsafe marker must fail");
        assert!(error.to_string().contains("unsafe path"));

        let absolute_marker = ModelInstallationMarker {
            snapshot_relative_path: root.join("outside").to_string_lossy().into_owned(),
            ..marker.clone()
        };
        fs::write(
            root.join(MODEL_MARKER_FILE),
            serde_json::to_vec(&absolute_marker).expect("absolute marker"),
        )
        .expect("write absolute marker");
        let error = provider
            .installed_metadata()
            .expect_err("absolute marker path must fail");
        assert!(error.to_string().contains("unsafe path"));

        let complete_relative = "snapshots/complete";
        write_complete_model_snapshot(&root, complete_relative);
        let unsupported_marker = ModelInstallationMarker {
            model_identifier: "unsupported/model".to_owned(),
            snapshot_relative_path: complete_relative.to_owned(),
            ..marker.clone()
        };
        fs::write(
            root.join(MODEL_MARKER_FILE),
            serde_json::to_vec(&unsupported_marker).expect("unsupported marker"),
        )
        .expect("write unsupported marker");
        let error = provider
            .installed_metadata()
            .expect_err("unsupported model identity must fail");
        assert!(error.to_string().contains("not supported"));

        fs::write(root.join(MODEL_MARKER_FILE), b"{").expect("malformed marker");
        let error = provider
            .installed_metadata()
            .expect_err("malformed marker must fail");
        assert!(error.to_string().contains("marker is invalid"));

        let marker = ModelInstallationMarker {
            snapshot_relative_path: "snapshots/missing".to_owned(),
            ..marker
        };
        fs::write(
            root.join(MODEL_MARKER_FILE),
            serde_json::to_vec(&marker).expect("marker"),
        )
        .expect("write marker");
        let error = provider
            .installed_metadata()
            .expect_err("incomplete marker must fail");
        assert!(error.to_string().contains("incomplete"));
        fs::remove_dir_all(root).expect("remove model fixture");
    }

    #[test]
    fn vector_centroid_is_normalized_and_validates_dimensions() {
        let centroid = vector_centroid(&[
            FakeProvider::vector("ambient noise"),
            FakeProvider::vector("surface dispersion"),
        ])
        .expect("centroid");
        let norm = centroid
            .iter()
            .map(|value| f64::from(*value).powi(2))
            .sum::<f64>()
            .sqrt();
        assert!((norm - 1.0).abs() < 1e-6);
    }

    #[test]
    fn usearch_persists_filters_and_removes_vectors() {
        let root = temporary_path("usearch");
        let profile = "a".repeat(64);
        let index = UsearchVectorIndex::new(root.clone()).expect("vector index");
        index
            .upsert(
                &profile,
                "paper",
                &[
                    (1, FakeProvider::vector("ambient noise")),
                    (2, FakeProvider::vector("bayesian inversion")),
                ],
            )
            .expect("upsert");
        let allowed = HashSet::from([1, 2]);
        let matches = index
            .nearest(
                &profile,
                "paper",
                &FakeProvider::vector("noise interferometry"),
                2,
                &allowed,
            )
            .expect("nearest");
        assert_eq!(matches[0].key, 1);
        drop(index);
        let reopened = UsearchVectorIndex::new(root.clone()).expect("reopen");
        assert!(reopened
            .vector(&profile, "paper", 1)
            .expect("vector")
            .is_some());
        reopened.remove(&profile, "paper", &[1]).expect("remove");
        assert!(reopened
            .vector(&profile, "paper", 1)
            .expect("vector")
            .is_none());
        fs::remove_dir_all(root).expect("remove vector fixture");
    }

    #[test]
    fn usearch_keeps_large_synthetic_retrieval_bounded() {
        let root = temporary_path("usearch-large");
        let profile = "b".repeat(64);
        let index = UsearchVectorIndex::new(root.clone()).expect("vector index");
        let vectors = (1..=10_000_u64)
            .map(|key| {
                let mut vector = vec![0.0_f32; EMBEDDING_DIMENSION];
                vector[key as usize % EMBEDDING_DIMENSION] = 1.0;
                vector[(key as usize * 17 + 3) % EMBEDDING_DIMENSION] += 0.2;
                (key, super::normalize_vector(vector).expect("vector"))
            })
            .collect::<Vec<_>>();
        index
            .upsert(&profile, "chunk", &vectors)
            .expect("large upsert");
        let allowed = (1..=10_000_u64).collect::<HashSet<_>>();
        let matches = index
            .nearest(&profile, "chunk", &vectors[4_216].1, 25, &allowed)
            .expect("bounded nearest");
        assert_eq!(matches.len(), 25);
        assert!(matches.iter().all(|matched| allowed.contains(&matched.key)));
        assert!(index.storage_bytes(&profile) > 0);
        fs::remove_dir_all(root).expect("remove large vector fixture");
    }

    #[test]
    fn indexes_searches_fuses_and_invalidates_incrementally() {
        let (database, path) = seeded_database("retrieval");
        let vectors = Arc::new(MemoryVectorIndex::default());
        let coordinator =
            SemanticCoordinator::with_components(database.clone(), Arc::new(FakeProvider), vectors);
        coordinator.install_model().expect("install model profile");
        let workspace = coordinator
            .index(&SemanticIndexRequest {
                max_papers: Some(8),
                max_documents: Some(2),
            })
            .expect("index");
        assert_eq!(workspace.semantic.indexed_papers, 3);
        assert_eq!(workspace.semantic.chunked_documents, 1);
        assert!(workspace.semantic.indexed_chunks > 0);

        let semantic = coordinator
            .search(&request("ambient correlations", "semantic"))
            .expect("semantic search");
        assert_eq!(semantic.results[0].item.id, "paper-noise");
        assert_eq!(semantic.retrieval_mode, "semantic");

        let hybrid = coordinator
            .search(&request("10.1000/bayes", "hybrid"))
            .expect("hybrid search");
        assert_eq!(hybrid.results[0].item.id, "paper-bayes");
        assert_eq!(
            hybrid.results[0].ranking.exact_match.as_deref(),
            Some("DOI")
        );

        let mut scoped_semantic = request("ambient noise interferometry", "semantic");
        scoped_semantic.item_ids = vec!["paper-bayes".to_owned()];
        let scoped_semantic = coordinator
            .search(&scoped_semantic)
            .expect("item-scoped semantic search");
        assert!(!scoped_semantic.results.is_empty());
        assert!(scoped_semantic
            .results
            .iter()
            .all(|result| result.item.id == "paper-bayes"));

        let mut scoped_hybrid = request("ambient noise interferometry", "hybrid");
        scoped_hybrid.item_ids = vec!["paper-bayes".to_owned()];
        let scoped_hybrid = coordinator
            .search(&scoped_hybrid)
            .expect("item-scoped hybrid search");
        assert!(!scoped_hybrid.results.is_empty());
        assert!(scoped_hybrid
            .results
            .iter()
            .all(|result| result.item.id == "paper-bayes"));

        let mut unknown_scope = request("ambient noise interferometry", "hybrid");
        unknown_scope.item_ids = vec!["foreign-or-missing-item".to_owned()];
        let unknown_scope = coordinator
            .search(&unknown_scope)
            .expect("unknown semantic scope");
        assert_eq!(unknown_scope.total, 0);
        assert!(unknown_scope.results.is_empty());

        let similar = coordinator
            .similar_papers(&crate::model::SimilarPapersRequest {
                seed_item_ids: vec!["paper-noise".to_owned()],
                collection_id: None,
                limit: Some(5),
                minimum_similarity: Some(-1.0),
            })
            .expect("similar papers");
        assert!(!similar.results.is_empty());
        let scoped = coordinator
            .neighborhood_scoped(
                &["paper-noise".to_owned()],
                Some(8),
                Some(12),
                Some("missing-collection"),
            )
            .expect("empty collection-scoped neighborhood");
        assert!(scoped
            .projection
            .items
            .iter()
            .all(|paper| paper.id == "paper-noise"));
        assert!(scoped
            .projection
            .graph
            .edges
            .iter()
            .all(|edge| edge.relation != "semantically-similar"));

        assert!(similar
            .results
            .iter()
            .all(|result| result.item.id != "paper-noise"));

        let connection = database.connect().expect("connection");
        connection
            .execute(
                "UPDATE research_items SET abstract_text = 'changed text'
                 WHERE id = 'paper-noise'",
                [],
            )
            .expect("metadata update");
        let status: String = connection
            .query_row(
                "SELECT status FROM paper_semantic_representations
                 WHERE item_id = 'paper-noise'",
                [],
                |row| row.get(0),
            )
            .expect("stale status");
        assert_eq!(status, "stale");
        fs::remove_file(path).expect("remove database fixture");
    }

    #[test]
    #[ignore = "requires an explicitly provisioned real-model cache"]
    fn real_model_runs_end_to_end_when_explicitly_requested() {
        let cache = std::env::var_os("GRAPHITE_M4_LIVE_MODEL_CACHE")
            .map(PathBuf::from)
            .expect("set GRAPHITE_M4_LIVE_MODEL_CACHE explicitly");
        let vector_root = temporary_path("real-model-vectors");
        let (database, path) = seeded_database("real-model");
        let provider = Arc::new(
            LocalFastEmbedProvider::new(cache.clone()).expect("real local embedding provider"),
        );
        let vectors = Arc::new(UsearchVectorIndex::new(vector_root.clone()).expect("real vectors"));
        let coordinator = SemanticCoordinator::with_components(database.clone(), provider, vectors);
        coordinator
            .install_model()
            .expect("real model installation");
        let indexed = coordinator
            .index(&SemanticIndexRequest {
                max_papers: Some(8),
                max_documents: Some(4),
            })
            .expect("real paper and chunk embeddings");
        assert_eq!(indexed.semantic.indexed_papers, 3);
        assert!(indexed.semantic.indexed_chunks > 0);
        let semantic = coordinator
            .search(&request(
                "passive interferometry using background vibrations",
                "semantic",
            ))
            .expect("real semantic search");
        assert!(!semantic.results.is_empty());
        let hybrid = coordinator
            .search(&request("10.1000/bayes", "hybrid"))
            .expect("real hybrid search");
        assert_eq!(hybrid.results[0].item.id, "paper-bayes");
        let similar = coordinator
            .similar_papers(&crate::model::SimilarPapersRequest {
                seed_item_ids: vec!["paper-noise".to_owned()],
                collection_id: None,
                limit: Some(3),
                minimum_similarity: Some(-1.0),
            })
            .expect("real similar papers");
        assert!(!similar.results.is_empty());
        let neighborhood = coordinator
            .neighborhood_scoped(&["paper-noise".to_owned()], Some(8), Some(12), None)
            .expect("real semantic neighborhood");
        assert!(neighborhood.total_relationships > 0);

        database
            .connect()
            .expect("connection")
            .execute(
                "UPDATE research_items
                 SET abstract_text = 'Ambient field correlations image the crust.'
                 WHERE id = 'paper-noise'",
                [],
            )
            .expect("stale real representation");
        coordinator
            .index(&SemanticIndexRequest {
                max_papers: Some(1),
                max_documents: Some(1),
            })
            .expect("real incremental re-index");

        drop(coordinator);
        let restarted = LocalFastEmbedProvider::new(cache).expect("restart provider");
        assert!(restarted
            .installed_metadata()
            .expect("local marker")
            .is_some());
        assert_eq!(
            restarted
                .embed_query("ambient seismic interferometry")
                .expect("local restart inference")
                .len(),
            EMBEDDING_DIMENSION,
        );
        fs::remove_dir_all(vector_root).expect("remove real vector fixture");
        fs::remove_file(path).expect("remove real database fixture");
    }

    #[test]
    #[ignore = "mutates only an explicitly selected Graphite-owned derived database"]
    fn connected_library_quality_smoke_when_explicitly_requested() {
        let database_path = std::env::var_os("GRAPHITE_M4_REAL_DATABASE")
            .map(PathBuf::from)
            .expect("set GRAPHITE_M4_REAL_DATABASE explicitly");
        let model_cache = std::env::var_os("GRAPHITE_M4_REAL_MODEL_CACHE")
            .map(PathBuf::from)
            .expect("set GRAPHITE_M4_REAL_MODEL_CACHE explicitly");
        let vector_root = std::env::var_os("GRAPHITE_M4_REAL_VECTOR_INDEX")
            .map(PathBuf::from)
            .expect("set GRAPHITE_M4_REAL_VECTOR_INDEX explicitly");
        assert_eq!(
            database_path.file_name().and_then(|name| name.to_str()),
            Some("graphite-research.sqlite3"),
            "the live smoke accepts only Graphite's derived database filename"
        );
        let database = ResearchDatabase::initialize(database_path).expect("Graphite database");
        let coordinator = SemanticCoordinator::new(database, model_cache, vector_root)
            .expect("real-library coordinator");
        coordinator
            .install_model()
            .expect("explicit real-library model install");
        let mut workspace = None;
        for _ in 0..4 {
            workspace = Some(
                coordinator
                    .index(&SemanticIndexRequest {
                        max_papers: Some(64),
                        max_documents: Some(2),
                    })
                    .expect("bounded real-library semantic batch"),
            );
        }
        let workspace = workspace.expect("real workspace");
        println!(
            "M4_REAL_COUNTS papers={} chunks={} eligible_papers={}",
            workspace.semantic.indexed_papers,
            workspace.semantic.indexed_chunks,
            workspace.semantic.eligible_papers,
        );
        let queries = [
            "surface-wave dispersion in crustal studies",
            "estimating Rayleigh-wave phase velocity from ambient noise",
            "probabilistic inversion of dispersion curves",
            "passive seismic interferometry for subsurface imaging",
        ];
        let mut first_seed = None;
        for query in queries {
            for mode in ["semantic", "hybrid"] {
                let page = coordinator
                    .search(&request(query, mode))
                    .expect("real-library query");
                let top = page
                    .results
                    .iter()
                    .take(5)
                    .map(|result| {
                        format!(
                            "{}::{:.3}::{}",
                            result.item.title.replace(['\n', '\r', '\t'], " "),
                            result.score,
                            result.passages.len()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                println!("M4_REAL_QUERY mode={mode} query={query} top={top}");
                if first_seed.is_none() {
                    first_seed = page.results.first().map(|result| result.item.id.clone());
                }
            }
        }
        let seed = first_seed.expect("at least one real result");
        let similar = coordinator
            .similar_papers(&crate::model::SimilarPapersRequest {
                seed_item_ids: vec![seed.clone()],
                collection_id: None,
                limit: Some(10),
                minimum_similarity: Some(DEFAULT_SIMILARITY_THRESHOLD),
            })
            .expect("real-library similar papers");
        println!("M4_REAL_SIMILAR count={}", similar.results.len());
        let neighborhood = coordinator
            .neighborhood_scoped(&[seed], Some(32), Some(64), None)
            .expect("real-library semantic neighborhood");
        println!(
            "M4_REAL_NEIGHBORHOOD nodes={} edges={}",
            neighborhood.projection.graph.nodes.len(),
            neighborhood.total_relationships
        );
    }
    #[test]
    fn single_document_indexing_requires_current_searchable_passages() {
        let (database, path) = seeded_database("single-document");
        let coordinator = SemanticCoordinator::with_components(
            database.clone(),
            Arc::new(FakeProvider),
            Arc::new(MemoryVectorIndex::default()),
        );
        coordinator.install_model().unwrap();
        coordinator.index_document("document-noise").unwrap();
        coordinator.index_document("document-noise").unwrap();
        database
            .connect()
            .unwrap()
            .execute(
                "DELETE FROM document_texts WHERE document_id='document-noise'",
                [],
            )
            .unwrap();
        assert!(coordinator.index_document("document-noise").is_err());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn topic_cache_is_persistent_scoped_and_invalidated_by_rebuild() {
        let (database, path) = seeded_database("topic-cache");
        let coordinator = SemanticCoordinator::with_components(
            database.clone(),
            Arc::new(FakeProvider),
            Arc::new(MemoryVectorIndex::default()),
        );
        assert!(database
            .load_workspace()
            .unwrap()
            .snapshot
            .topics
            .iter()
            .all(|t| t.origin != "semantic-cluster"));
        coordinator.install_model().unwrap();
        coordinator
            .index(&SemanticIndexRequest {
                max_papers: Some(8),
                max_documents: Some(2),
            })
            .unwrap();
        let connection = database.connect().unwrap();
        let key = "semantic-topics-v1:library";
        let raw: String = connection
            .query_row("SELECT value FROM app_meta WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .unwrap();
        let mut run: serde_json::Value = serde_json::from_str(&raw).unwrap();
        // Algorithm quality is tested separately; supply an accepted map to test storage boundaries.
        let ids: Vec<_> = run["signatures"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        run["topics"] = serde_json::json!([{"id":"topic-test","name":"Surface waves","shortName":"Surface waves","origin":"semantic-cluster","color":"#5dcfd1"}]);
        run["assignments"] = serde_json::Value::Object(
            ids.iter()
                .map(|id| (id.clone(), serde_json::json!("topic-test")))
                .collect(),
        );
        connection
            .execute(
                "UPDATE app_meta SET value=?2 WHERE key=?1",
                params![key, run.to_string()],
            )
            .unwrap();
        let reopened = ResearchDatabase::initialize(path.clone()).unwrap();
        let workspace = reopened.load_workspace().unwrap();
        assert_eq!(workspace.snapshot.topics.len(), 2);
        for item in &workspace.snapshot.items {
            assert_eq!(item.topics, vec!["topic-test"]);
        }
        let projection = reopened.projection_for_item_ids(&ids[..1]).unwrap();
        assert_eq!(projection.items[0].topics, vec!["topic-test"]);
        let other = super::topics::load(&connection, "other-library").unwrap();
        assert!(other.1.is_empty());
        let accepted: String = connection
            .query_row("SELECT value FROM app_meta WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .unwrap();
        connection
            .execute(
                "UPDATE research_items SET source_version=source_version+1 WHERE id=?1",
                [&ids[0]],
            )
            .unwrap();
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let profile = super::active_profile(&connection).unwrap().unwrap();
        assert!(!super::topics::refresh(
            &connection,
            "library",
            &profile.id,
            coordinator.vectors.as_ref(),
            &cancelled
        )
        .unwrap());
        let retained: String = connection
            .query_row("SELECT value FROM app_meta WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(accepted, retained);
        assert!(!super::topics::load(&connection, "library")
            .unwrap()
            .1
            .contains_key(&ids[0]));
        connection
            .execute("UPDATE embedding_profiles SET active=0", [])
            .unwrap();
        assert!(super::topics::load(&connection, "library")
            .unwrap()
            .1
            .is_empty());
        connection
            .execute(
                "UPDATE embedding_profiles SET active=1 WHERE id=?1",
                [&profile.id],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE app_meta SET value='invalid json' WHERE key=?1",
                [key],
            )
            .unwrap();
        assert!(reopened
            .load_workspace()
            .unwrap()
            .snapshot
            .topics
            .iter()
            .all(|t| t.origin != "semantic-cluster"));
        run["assignments"] = serde_json::json!({});
        connection
            .execute(
                "UPDATE app_meta SET value=?2 WHERE key=?1",
                params![key, run.to_string()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO app_meta(key,value) VALUES (?1,'test failure')",
                ["semantic-topics-v1:error:library"],
            )
            .unwrap();
        assert_eq!(
            reopened.load_workspace().unwrap().snapshot.topics[0].name,
            "Topic analysis failed. Retry library preparation."
        );
        coordinator.rebuild().unwrap();
        assert!(reopened
            .load_workspace()
            .unwrap()
            .snapshot
            .topics
            .iter()
            .all(|t| t.origin != "semantic-cluster"));
        drop(connection);
        drop(reopened);
        drop(coordinator);
        drop(database);
        let _ = fs::remove_file(path);
    }
}
