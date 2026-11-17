pub(crate) mod agent;
pub(crate) mod citation;
pub(crate) mod demo;
pub(crate) mod document;
pub(crate) mod lenses;
pub(crate) mod library;
pub(crate) mod pdf_export;
pub(crate) mod semantic;
pub(crate) mod shadow;

use crate::model::WorkspacePayload;
use crate::semantic::SemanticCoordinator;

pub(super) async fn enrich_workspace(
    semantic: SemanticCoordinator,
    workspace: WorkspacePayload,
) -> Result<WorkspacePayload, String> {
    tauri::async_runtime::spawn_blocking(move || semantic.enrich_workspace(workspace))
        .await
        .map_err(|error| format!("The semantic diagnostics task failed: {error}"))?
        .map_err(|error| error.to_string())
}
