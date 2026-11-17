//! Prepare the real collection using native dialogs and bounded commands.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use crate::demo::{DemoArchiveInfo, DemoProgress, DemoStatus};
use crate::model::WorkspacePayload;
use crate::AppState;

#[tauri::command]
pub(crate) async fn activate_demo_library(
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    activate(state, true).await
}

#[tauri::command]
pub(crate) async fn activate_zotero_library(
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    activate(state, false).await
}

async fn activate(state: State<'_, AppState>, demo: bool) -> Result<WorkspacePayload, String> {
    let coordinator = state.demo.clone();
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || {
        semantic.enrich_workspace(coordinator.activate(demo)?)
    })
    .await
    .map_err(|_| "The library switch task failed.".to_owned())?
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn get_demo_status(state: State<'_, AppState>) -> Result<DemoStatus, String> {
    let coordinator = state.demo.clone();
    tauri::async_runtime::spawn_blocking(move || coordinator.status())
        .await
        .map_err(|_| "The demo status task failed.".to_owned())?
        .map_err(|e| e.to_string())
}

fn callback(app: AppHandle) -> Arc<dyn Fn(DemoProgress) + Send + Sync> {
    Arc::new(move |event| {
        let _ = app.emit("graphite-demo-progress", event);
    })
}

#[tauri::command]
pub(crate) async fn download_demo_pdfs(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DemoStatus, String> {
    let coordinator = state.demo.clone();
    tauri::async_runtime::spawn_blocking(move || coordinator.acquire_archive(None, callback(app)))
        .await
        .map_err(|_| "The demo download task failed.".to_owned())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn import_demo_pdfs(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<DemoStatus>, String> {
    let coordinator = state.demo.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let selected = app
            .dialog()
            .file()
            .set_title("Choose the Graphite demo ZIP")
            .add_filter("Graphite demo archive", &["zip"])
            .blocking_pick_file();
        let Some(selected) = selected else {
            return Ok(None);
        };
        let archive = selected
            .as_path()
            .ok_or_else(|| "Choose a local demo ZIP.".to_owned())?;
        coordinator
            .acquire_archive(Some(archive), callback(app))
            .map(Some)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "The demo import task failed.".to_owned())?
}

#[tauri::command]
pub(crate) async fn store_demo_document_pages(
    document_id: String,
    pages: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let coordinator = state.demo.clone();
    tauri::async_runtime::spawn_blocking(move || coordinator.store_pages(&document_id, pages))
        .await
        .map_err(|_| "The demo text task failed.".to_owned())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn cancel_demo_preparation(state: State<'_, AppState>) {
    state.demo.cancel();
}

#[tauri::command]
pub(crate) fn get_demo_archive_info() -> Result<DemoArchiveInfo, String> {
    crate::demo::archive_info().map_err(|error| error.to_string())
}
