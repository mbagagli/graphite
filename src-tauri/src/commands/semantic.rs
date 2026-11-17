use tauri::State;

use crate::model::{
    PaperSemanticCoverage, SearchPage, SearchRequest, SemanticIndexRequest,
    SemanticRelationshipEvidence, SimilarPaperPage, SimilarPapersRequest, WorkspacePayload,
};
use crate::AppState;

#[tauri::command]
pub(crate) async fn search_library(
    request: SearchRequest,
    state: State<'_, AppState>,
) -> Result<SearchPage, String> {
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || semantic.search(&request))
        .await
        .map_err(|error| format!("The local retrieval task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn install_semantic_model(
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || semantic.install_model())
        .await
        .map_err(|error| format!("The local model installation task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn index_semantic_corpus(
    request: SemanticIndexRequest,
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || semantic.index(&request))
        .await
        .map_err(|error| format!("The semantic indexing task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn rebuild_semantic_index(
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || semantic.rebuild())
        .await
        .map_err(|error| format!("The semantic rebuild task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn cancel_semantic_indexing(state: State<'_, AppState>) {
    state.semantic.cancel_indexing();
}

#[tauri::command]
pub(crate) async fn query_similar_papers(
    request: SimilarPapersRequest,
    state: State<'_, AppState>,
) -> Result<SimilarPaperPage, String> {
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || semantic.similar_papers(&request))
        .await
        .map_err(|error| format!("The similar-paper task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn get_paper_semantic_coverage(
    item_id: String,
    state: State<'_, AppState>,
) -> Result<PaperSemanticCoverage, String> {
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || semantic.paper_coverage(&item_id))
        .await
        .map_err(|error| format!("The semantic-coverage task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn get_semantic_relationship_evidence(
    edge_id: String,
    state: State<'_, AppState>,
) -> Result<Option<SemanticRelationshipEvidence>, String> {
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || semantic.relationship_evidence(&edge_id))
        .await
        .map_err(|error| format!("The semantic-evidence task failed: {error}"))?
        .map_err(|error| error.to_string())
}
