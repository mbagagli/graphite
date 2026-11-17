use tauri::State;

use crate::extraction::shadow::{ShadowBatchSummary, ShadowRunRequest};
use crate::AppState;

#[tauri::command]
pub(crate) async fn run_pdf_shadow_pilot(
    request: ShadowRunRequest,
    state: State<'_, AppState>,
) -> Result<ShadowBatchSummary, String> {
    state
        .shadow
        .run_selected(request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn cancel_pdf_shadow_pilot(state: State<'_, AppState>) -> Result<u32, String> {
    state.shadow.cancel().map_err(|error| error.to_string())
}
