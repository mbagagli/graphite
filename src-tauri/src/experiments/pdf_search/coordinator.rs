use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::error::{AppError, AppResult};
use crate::model::SearchPage;
use crate::semantic::{EmbeddingProvider, LocalFastEmbedProvider};

use super::index::ExperimentIndexer;
use super::model::{
    CharacterizationQuery, ExperimentCancellationToken, ExperimentDefinition,
    ExperimentIndexSummary, ExperimentRetrievalKind, ExperimentSearchPage,
    MirrorEquivalenceDifference, MirrorEquivalenceReport, ProductionSnapshot,
};
use super::retrieval::{production_context_fingerprints, ExperimentRetriever};
use super::runtime::ExperimentRuntime;

const RUNTIME_OPT_IN: &str = "GRAPHITE_PDF_SEARCH_EXPERIMENT_ENABLED";

#[derive(Clone)]
pub struct PdfSearchExperimentCoordinator {
    data_root: PathBuf,
    provider: Arc<dyn EmbeddingProvider>,
    require_runtime_opt_in: bool,
    operation_guard: Arc<Mutex<()>>,
    active: Arc<Mutex<HashMap<String, ExperimentCancellationToken>>>,
}

impl PdfSearchExperimentCoordinator {
    pub fn new(data_root: PathBuf, model_cache_root: PathBuf) -> AppResult<Self> {
        Ok(Self {
            data_root,
            provider: Arc::new(LocalFastEmbedProvider::new(model_cache_root)?),
            require_runtime_opt_in: true,
            operation_guard: Arc::new(Mutex::new(())),
            active: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_provider(data_root: PathBuf, provider: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            data_root,
            provider,
            require_runtime_opt_in: false,
            operation_guard: Arc::new(Mutex::new(())),
            active: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn definition(&self, corpus_manifest_sha256: &str) -> AppResult<ExperimentDefinition> {
        self.ensure_available()?;
        let metadata = self.provider.installed_metadata()?.ok_or_else(|| {
            AppError::Semantic("The frozen production embedding model is not installed".to_owned())
        })?;
        Ok(ExperimentDefinition::frozen(
            corpus_manifest_sha256,
            crate::semantic::embedding_profile_id(&metadata),
            metadata.artifact_identity,
        ))
    }

    pub fn build_zotero_mirror(
        &self,
        definition: &ExperimentDefinition,
        snapshot: &ProductionSnapshot,
    ) -> AppResult<ExperimentIndexSummary> {
        self.ensure_available()?;
        let _guard = self.operation_guard.try_lock().map_err(|_| {
            AppError::Semantic("A PDF search experiment operation is already running".to_owned())
        })?;
        let token = ExperimentCancellationToken::default();
        self.active
            .lock()
            .map_err(lock_error)?
            .insert(definition.definition_id.clone(), token.clone());
        let runtime = ExperimentRuntime::prepare(&self.data_root, definition)?;
        let outcome = ExperimentIndexer::new(self.provider.clone())
            .index_zotero_mirror(&runtime, definition, snapshot, &token);
        self.active
            .lock()
            .map_err(lock_error)?
            .remove(&definition.definition_id);
        outcome
    }

    pub fn search_zotero_mirror(
        &self,
        definition: &ExperimentDefinition,
        query: &CharacterizationQuery,
        retrieval_kind: ExperimentRetrievalKind,
        limit: usize,
    ) -> AppResult<ExperimentSearchPage> {
        self.ensure_available()?;
        let runtime = ExperimentRuntime::existing(&self.data_root, &definition.definition_id)?;
        runtime.validate_published(definition)?;
        ExperimentRetriever::new(self.provider.clone()).search(
            &runtime,
            query,
            retrieval_kind,
            limit,
        )
    }

    pub fn compare_zotero_mirror(
        &self,
        definition: &ExperimentDefinition,
        observations: &[(CharacterizationQuery, ExperimentRetrievalKind, SearchPage)],
    ) -> AppResult<MirrorEquivalenceReport> {
        self.ensure_available()?;
        let mut differences = Vec::new();
        for (query, retrieval_kind, production) in observations {
            let experimental = self.search_zotero_mirror(
                definition,
                query,
                *retrieval_kind,
                production.limit as usize,
            )?;
            compare_pages(
                query,
                *retrieval_kind,
                production,
                &experimental,
                self.provider.clone(),
                &mut differences,
            )?;
        }
        Ok(MirrorEquivalenceReport {
            definition_id: definition.definition_id.clone(),
            query_count: observations
                .iter()
                .map(|(query, _, _)| &query.id)
                .collect::<std::collections::HashSet<_>>()
                .len() as u32,
            comparison_count: observations.len() as u32,
            equivalent: differences.is_empty(),
            differences,
        })
    }

    pub fn cancel(&self, definition_id: &str) -> AppResult<bool> {
        let active = self.active.lock().map_err(lock_error)?;
        let Some(token) = active.get(definition_id) else {
            return Ok(false);
        };
        token.cancel();
        Ok(true)
    }

    pub fn cleanup(&self, definition_id: &str) -> AppResult<()> {
        self.ensure_available()?;
        self.cancel(definition_id)?;
        let _guard = self.operation_guard.lock().map_err(lock_error)?;
        let runtime = ExperimentRuntime::existing(&self.data_root, definition_id)?;
        runtime.cleanup()
    }

    fn ensure_available(&self) -> AppResult<()> {
        if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            return Err(AppError::ApplicationData(
                "The PDF search experiment is unavailable on this platform".to_owned(),
            ));
        }
        if self.require_runtime_opt_in && std::env::var(RUNTIME_OPT_IN).as_deref() != Ok("1") {
            return Err(AppError::ApplicationData(format!(
                "The PDF search experiment is disabled; set {RUNTIME_OPT_IN}=1 explicitly"
            )));
        }
        Ok(())
    }
}

fn compare_pages(
    query: &CharacterizationQuery,
    retrieval_kind: ExperimentRetrievalKind,
    production: &SearchPage,
    experimental: &ExperimentSearchPage,
    provider: Arc<dyn EmbeddingProvider>,
    differences: &mut Vec<MirrorEquivalenceDifference>,
) -> AppResult<()> {
    let production_ids = production
        .results
        .iter()
        .map(|result| result.item.id.clone())
        .collect::<Vec<_>>();
    let experimental_ids = experimental
        .results
        .iter()
        .map(|result| result.item_id.clone())
        .collect::<Vec<_>>();
    record_difference(
        differences,
        query,
        retrieval_kind,
        "ranked-item-ids",
        &production_ids,
        &experimental_ids,
        None,
    )?;
    for (production, experimental) in production
        .results
        .iter()
        .zip(&experimental.results)
        .filter(|(left, right)| left.item.id == right.item_id)
    {
        record_difference(
            differences,
            query,
            retrieval_kind,
            &format!("{}:lexical-rank", production.item.id),
            &production.ranking.lexical_rank,
            &experimental.lexical_rank,
            None,
        )?;
        record_difference(
            differences,
            query,
            retrieval_kind,
            &format!("{}:semantic-rank", production.item.id),
            &production.ranking.semantic_rank,
            &experimental.semantic_rank,
            None,
        )?;
        if (production.score - experimental.fusion_score).abs() > 1e-9 {
            differences.push(MirrorEquivalenceDifference {
                query_id: query.id.clone(),
                retrieval_kind,
                field: format!("{}:score", production.item.id),
                production: format!("{:.12}", production.score),
                experimental: format!("{:.12}", experimental.fusion_score),
                explanation: None,
            });
        }
        let production_chunks = production
            .passages
            .iter()
            .map(|passage| passage.chunk_id.clone())
            .collect::<Vec<_>>();
        let experimental_chunks = experimental
            .passages
            .iter()
            .map(|passage| passage.logical_chunk_id.clone())
            .collect::<Vec<_>>();
        record_difference(
            differences,
            query,
            retrieval_kind,
            &format!("{}:logical-chunk-ids", production.item.id),
            &production_chunks,
            &experimental_chunks,
            None,
        )?;
    }
    let production_context =
        production_context_fingerprints(production, &query.item_ids, provider)?;
    record_difference(
        differences,
        query,
        retrieval_kind,
        "context-fingerprints",
        &production_context,
        &experimental.context_fingerprints,
        None,
    )
}

fn record_difference<T: serde::Serialize + PartialEq>(
    differences: &mut Vec<MirrorEquivalenceDifference>,
    query: &CharacterizationQuery,
    retrieval_kind: ExperimentRetrievalKind,
    field: &str,
    production: &T,
    experimental: &T,
    explanation: Option<String>,
) -> AppResult<()> {
    if production != experimental {
        differences.push(MirrorEquivalenceDifference {
            query_id: query.id.clone(),
            retrieval_kind,
            field: field.to_owned(),
            production: serde_json::to_string(production)?,
            experimental: serde_json::to_string(experimental)?,
            explanation,
        });
    }
    Ok(())
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> AppError {
    AppError::ApplicationData("The PDF search experiment lock was poisoned".to_owned())
}

#[cfg(test)]
mod tests {
    #[test]
    fn production_search_request_has_no_experimental_mode_contract() {
        let request = crate::model::SearchRequest {
            query: "test".to_owned(),
            mode: Some("experimental-zotero-mirror".to_owned()),
            limit: None,
            offset: None,
            item_ids: Vec::new(),
            collection_id: None,
            citation_filter: None,
            year_from: None,
            year_to: None,
            item_type: None,
        };
        assert_ne!(request.mode.as_deref(), Some("semantic"));
        assert_ne!(request.mode.as_deref(), Some("hybrid"));
    }
}
