use tauri::State;

use crate::citation;
use crate::model::{
    CitationIndexRequest, CitationItemPage, CitationListRequest, CoCitedWorkPage,
    EdgeEvidenceRequest, ExternalWorkPage, MissingWorksRequest, ReferencePage,
    ReferenceQueryRequest, RelationshipEvidence, ResearchNeighborhood, ResearchNeighborhoodRequest,
    ResolutionOverrideRequest, StructuralPaperPage, WorkspacePayload,
};
use crate::AppState;

#[tauri::command]
pub(crate) async fn index_citation_corpus(
    request: CitationIndexRequest,
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    let workspace = state
        .citations
        .index(request.priority_item_id.as_deref(), request.max_documents)
        .await
        .map_err(|error| error.to_string())?;
    super::enrich_workspace(state.semantic.clone(), workspace).await
}

#[tauri::command]
pub(crate) async fn rebuild_citation_index(
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    let workspace = state
        .citations
        .rebuild()
        .await
        .map_err(|error| error.to_string())?;
    super::enrich_workspace(state.semantic.clone(), workspace).await
}

#[tauri::command]
pub(crate) async fn reresolve_citations(
    state: State<'_, AppState>,
) -> Result<WorkspacePayload, String> {
    let workspace = state
        .citations
        .reresolve()
        .await
        .map_err(|error| error.to_string())?;
    super::enrich_workspace(state.semantic.clone(), workspace).await
}

#[tauri::command]
pub(crate) async fn query_references(
    request: ReferenceQueryRequest,
    state: State<'_, AppState>,
) -> Result<ReferencePage, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || citation::reference_page(&database, &request))
        .await
        .map_err(|error| format!("The reference query task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn query_cited_by(
    request: CitationListRequest,
    state: State<'_, AppState>,
) -> Result<CitationItemPage, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || citation::cited_by_page(&database, &request))
        .await
        .map_err(|error| format!("The cited-by query task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn query_shared_references(
    item_id: String,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<StructuralPaperPage, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        citation::shared_reference_papers(&database, &item_id, limit)
    })
    .await
    .map_err(|error| format!("The shared-reference task failed: {error}"))?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn query_co_cited(
    item_id: String,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<CoCitedWorkPage, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        citation::co_cited_works(&database, &item_id, limit)
    })
    .await
    .map_err(|error| format!("The co-citation task failed: {error}"))?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn load_research_neighborhood(
    request: ResearchNeighborhoodRequest,
    collection_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<ResearchNeighborhood, String> {
    if request.mode == "semantic" {
        let semantic = state.semantic.clone();
        return tauri::async_runtime::spawn_blocking(move || {
            semantic.neighborhood_scoped(
                &request.seed_item_ids,
                request.max_nodes,
                request.max_edges,
                collection_id.as_deref(),
            )
        })
        .await
        .map_err(|error| format!("The semantic-neighborhood task failed: {error}"))?
        .map_err(|error| error.to_string());
    }
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || citation::neighborhood(&database, &request))
        .await
        .map_err(|error| format!("The citation-neighborhood task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn get_relationship_evidence(
    request: EdgeEvidenceRequest,
    state: State<'_, AppState>,
) -> Result<Option<RelationshipEvidence>, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        citation::relationship_evidence(&database, &request)
    })
    .await
    .map_err(|error| format!("The citation-evidence task failed: {error}"))?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn review_reference_resolution(
    request: ResolutionOverrideRequest,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || citation::apply_override(&database, &request))
        .await
        .map_err(|error| format!("The reference review task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn query_missing_works(
    request: MissingWorksRequest,
    state: State<'_, AppState>,
) -> Result<ExternalWorkPage, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || citation::missing_works(&database, &request))
        .await
        .map_err(|error| format!("The missing-works task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn get_connection_details(
    source_id: String,
    target_id: String,
    relation: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let database = state.database.clone();
    tauri::async_runtime::spawn_blocking(move || {
        citation::connection_details(&database, &source_id, &target_id, &relation)
    })
    .await
    .map_err(|error| format!("Connection evidence could not be loaded: {error}"))?
    .map_err(|error| error.to_string())
}
