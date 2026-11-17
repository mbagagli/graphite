use tauri::State;

use crate::model::{
    GraphProjectionRequest, GraphProjectionResult, ItemPage, ItemQueryRequest, SourceProbe,
    WorkspacePayload,
};
use crate::AppState;

#[tauri::command]
pub(crate) async fn load_workspace(state: State<'_, AppState>) -> Result<WorkspacePayload, String> {
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || semantic.load_workspace())
        .await
        .map_err(|error| format!("The workspace task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn probe_zotero(state: State<'_, AppState>) -> Result<SourceProbe, String> {
    state.sync.probe().await.map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn sync_zotero(state: State<'_, AppState>) -> Result<WorkspacePayload, String> {
    let workspace = state
        .sync
        .synchronize()
        .await
        .map_err(|error| error.to_string())?;
    super::enrich_workspace(state.semantic.clone(), workspace).await
}

#[tauri::command]
pub(crate) async fn rebuild_zotero_cache(
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    let workspace = state
        .sync
        .rebuild()
        .await
        .map_err(|error| error.to_string())?;
    super::enrich_workspace(state.semantic.clone(), workspace).await
}

#[tauri::command]
pub(crate) async fn set_active_library(
    library_id: String,
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    let database = state.database.clone();
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || {
        database.set_active_library(&library_id)?;
        semantic.load_workspace()
    })
    .await
    .map_err(|error| format!("The library switch task failed: {error}"))?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn query_library_items(
    request: ItemQueryRequest,
    state: State<'_, AppState>,
) -> Result<ItemPage, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.query_items(&request))
        .await
        .map_err(|error| format!("The item query task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn load_graph_projection(
    request: GraphProjectionRequest,
    state: State<'_, AppState>,
) -> Result<GraphProjectionResult, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.graph_projection(&request))
        .await
        .map_err(|error| format!("The graph projection task failed: {error}"))?
        .map_err(|error| error.to_string())
}
