use tauri::State;

use crate::model::{DocumentOpenResult, DocumentTextPayload, ReaderState, WorkspacePayload};
use crate::AppState;

#[tauri::command]
pub(crate) async fn sync_document_corpus(
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    let workspace = state
        .corpus
        .synchronize()
        .await
        .map_err(|error| error.to_string())?;
    super::enrich_workspace(state.semantic.clone(), workspace).await
}

#[tauri::command]
pub(crate) async fn rebuild_document_index(
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    let workspace = state
        .corpus
        .rebuild()
        .await
        .map_err(|error| error.to_string())?;
    super::enrich_workspace(state.semantic.clone(), workspace).await
}

#[tauri::command]
pub(crate) async fn resolve_document(
    document_id: String,
    state: State<'_, AppState>,
) -> Result<DocumentOpenResult, String> {
    state
        .documents
        .resolve(&document_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn get_document_text(
    document_id: String,
    state: State<'_, AppState>,
) -> Result<Option<DocumentTextPayload>, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.document_text(&document_id))
        .await
        .map_err(|error| format!("The document text task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn get_reader_state(
    document_id: String,
    state: State<'_, AppState>,
) -> Result<Option<ReaderState>, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.reader_state(&document_id))
        .await
        .map_err(|error| format!("The reader-state task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn save_reader_state(
    reader_state: ReaderState,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || database.save_reader_state(&reader_state))
        .await
        .map_err(|error| format!("The reader-state save task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn get_docling_status(
    document_id: String,
    state: State<'_, AppState>,
) -> Result<crate::extraction::docling::DoclingStatus, String> {
    state
        .docling
        .status(&document_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn extract_document_docling(
    document_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .documents
        .resolve(&document_id)
        .await
        .map_err(|e| e.to_string())?;
    // Archive source text once; never re-wrap a saved alternative as Zotero.
    if state
        .docling
        .status(&document_id)
        .map_err(|e| e.to_string())?
        .choices
        .is_empty()
    {
        crate::extraction::ExtractionCoordinator::new(state.database.clone())
            .canonicalize_document(&document_id)
            .await
            .map_err(|e| e.to_string())?;
    }
    let coordinator = state.docling.clone();
    tauri::async_runtime::spawn_blocking(move || coordinator.extract(&document_id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn cancel_document_docling(
    document_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .docling
        .cancel(&document_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn select_document_extraction(
    document_id: String,
    extraction_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state
        .documents
        .resolve(&document_id)
        .await
        .map_err(|e| e.to_string())?;
    let coordinator = state.docling.clone();
    tauri::async_runtime::spawn_blocking(move || coordinator.select(&document_id, &extraction_id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn index_document_extraction(
    document_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let semantic = state.semantic.clone();
    tauri::async_runtime::spawn_blocking(move || semantic.index_document(&document_id))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn get_library_preparation_documents(
    library_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<crate::extraction::pdf_pages::PreparationDocument>, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::extraction::pdf_pages::preparation_documents(&database, &library_id)
    })
    .await
    .map_err(|_| "The preparation inventory task failed.".to_owned())?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn store_document_pdf_pages(
    document_id: String,
    revision_key: String,
    pages: Vec<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::extraction::pdf_pages::store_document_pages(
            &database,
            &document_id,
            &revision_key,
            pages,
        )
    })
    .await
    .map_err(|_| "The local PDF preparation task failed.".to_owned())?
    .map_err(|error| error.to_string())
}
