use std::sync::Arc;

use tauri::ipc::Channel;
use tauri::State;

use crate::agent::model::{
    AgentEnvironment, AgentRequest, AgentRunResult, AgentSettings, AgentStreamEvent,
    ConversationDetail, ConversationSummary, ExternalProviderConfig, ModelInstallEvent,
};
use crate::AppState;

#[tauri::command]
pub(crate) async fn get_agent_environment(
    state: State<'_, AppState>,
) -> Result<AgentEnvironment, String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || agent.environment())
        .await
        .map_err(|error| format!("The agent environment task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn save_agent_settings(
    settings: AgentSettings,
    state: State<'_, AppState>,
) -> Result<AgentEnvironment, String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || agent.save_settings(&settings))
        .await
        .map_err(|error| format!("The agent settings task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn install_agent_model(
    profile_id: String,
    on_event: Channel<ModelInstallEvent>,
    state: State<'_, AppState>,
) -> Result<AgentEnvironment, String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || {
        agent.install_model(
            &profile_id,
            Arc::new(move |event| {
                let _ = on_event.send(event);
            }),
        )
    })
    .await
    .map_err(|error| format!("The agent model installation task failed: {error}"))?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn unload_agent_model(state: State<'_, AppState>) -> Result<(), String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || agent.unload_model())
        .await
        .map_err(|error| format!("The agent model unload task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn check_external_agent_provider(
    config: ExternalProviderConfig,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || agent.check_external_provider(config))
        .await
        .map_err(|error| format!("The external provider check failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn run_research_agent(
    request: AgentRequest,
    on_event: Channel<AgentStreamEvent>,
    state: State<'_, AppState>,
) -> Result<AgentRunResult, String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || {
        agent.run(
            request,
            Arc::new(move |event| {
                let _ = on_event.send(event);
            }),
        )
    })
    .await
    .map_err(|error| format!("The local research-agent task failed: {error}"))?
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn cancel_research_agent(state: State<'_, AppState>) {
    state.agent.cancel();
}

#[tauri::command]
pub(crate) async fn list_agent_conversations(
    state: State<'_, AppState>,
) -> Result<Vec<ConversationSummary>, String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || agent.conversations())
        .await
        .map_err(|error| format!("The conversation list task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn load_agent_conversation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<ConversationDetail, String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || agent.conversation(&conversation_id))
        .await
        .map_err(|error| format!("The conversation load task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn delete_agent_conversation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || agent.delete_conversation(&conversation_id))
        .await
        .map_err(|error| format!("The conversation deletion task failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn export_agent_conversation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let agent = state.agent.clone();
    tauri::async_runtime::spawn_blocking(move || agent.export_markdown(&conversation_id))
        .await
        .map_err(|error| format!("The conversation export task failed: {error}"))?
        .map_err(|error| error.to_string())
}
