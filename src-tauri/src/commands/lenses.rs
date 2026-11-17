//! Expose library-scoped Lens persistence through the native boundary.
use crate::{database::lenses::SaveLensRequest, model::ResearchLens, AppState};
use tauri::State;

#[tauri::command]
pub(crate) async fn list_lenses(
    library_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<ResearchLens>, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.list_lenses(&library_id))
        .await
        .map_err(|e| format!("The Lens task failed: {e}"))?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn save_lens(
    request: SaveLensRequest,
    state: State<'_, AppState>,
) -> Result<ResearchLens, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.save_lens(&request))
        .await
        .map_err(|e| format!("The Lens task failed: {e}"))?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn delete_lens(
    library_id: String,
    id: String,
    revision: i64,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.delete_lens(&library_id, &id, revision))
        .await
        .map_err(|e| format!("The Lens task failed: {e}"))?
        .map_err(|e| e.to_string())
}
