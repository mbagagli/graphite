use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::Instant;

use sysinfo::System;

use crate::database::ResearchDatabase;
use crate::error::{AppError, AppResult};
use crate::semantic::SemanticCoordinator;

use super::external::OpenAiCompatibleLocalProvider;
use super::grounding::{
    abstention_answer, prepare_run, repair_prompt, retrieve_run, system_prompt, user_prompt,
    validate_answer_for_question,
};
use super::managed::{
    install_profile, profile_spec, profile_summary, GraphiteManagedProvider, ManagedRuntime,
    PROFILE_SPECS, RUNTIME_VERSION,
};
use super::model::{
    AgentEnvironment, AgentProviderKind, AgentRequest, AgentRunResult, AgentSettings,
    AgentStreamEvent, AgentTimings, ConversationDetail, ConversationSummary,
    ExternalProviderConfig, GenerationRequest, HardwareProfile, ModelInstallEvent,
};
use super::provider::{ConservativeTokenEstimator, EvidenceTokenCounter, LocalModelProvider};
use super::store::AgentStore;

const TOOL_BUDGET: u32 = 5;
const CONTEXT_BUDGET_TOKENS: u32 = 12_000;

#[derive(Clone)]
pub struct AgentCoordinator {
    #[cfg(test)]
    pub(crate) observed_retrieval: Arc<Mutex<Vec<super::model::AgentEvidence>>>,
    database: ResearchDatabase,
    semantic: SemanticCoordinator,
    store: AgentStore,
    managed_runtime: ManagedRuntime,
    model_cache_root: PathBuf,
    cancellation: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    generation_guard: Arc<Mutex<()>>,
}

impl AgentCoordinator {
    pub fn new(
        database: ResearchDatabase,
        semantic: SemanticCoordinator,
        model_cache_root: PathBuf,
    ) -> AppResult<Self> {
        let managed_runtime = ManagedRuntime::initialize()?;
        Ok(Self {
            #[cfg(test)]
            observed_retrieval: Arc::new(Mutex::new(Vec::new())),
            store: AgentStore::new(database.clone(), model_cache_root.clone()),
            database,
            semantic,
            managed_runtime,
            model_cache_root,
            cancellation: Arc::new(Mutex::new(None)),
            generation_guard: Arc::new(Mutex::new(())),
        })
    }

    pub fn environment(&self) -> AppResult<AgentEnvironment> {
        let settings = self.store.settings()?;
        let mut profiles = Vec::new();
        for profile in PROFILE_SPECS {
            profiles.push(profile_summary(
                profile,
                self.store.installation_summary(profile.id)?,
            ));
        }
        Ok(AgentEnvironment {
            settings,
            hardware: hardware_profile(self.managed_runtime.hardware_status()?),
            model_profiles: profiles,
            runtime_name: "Direct linked GGUF runtime".to_owned(),
            runtime_version: RUNTIME_VERSION.to_owned(),
            trust_statement: "Answers are generated locally from explicitly assembled Graphite evidence. The managed provider opens no network port and no cloud fallback exists.".to_owned(),
            tool_budget: TOOL_BUDGET,
            context_budget_tokens: CONTEXT_BUDGET_TOKENS,
            reranker_status: "Not adopted: deterministic M4 Hybrid ranking and diversity are the M5 baseline.".to_owned(),
        })
    }

    pub fn save_settings(&self, settings: &AgentSettings) -> AppResult<AgentEnvironment> {
        if settings.provider_kind == AgentProviderKind::OpenaiCompatible {
            let config = ExternalProviderConfig {
                endpoint: settings.external_endpoint.clone().unwrap_or_default(),
                model: settings.external_model.clone().unwrap_or_default(),
                context_tokens: settings.external_context_tokens.unwrap_or(8_192),
                api_key: None,
            };
            OpenAiCompatibleLocalProvider::new(config)?;
        }
        self.store.save_settings(settings)?;
        self.environment()
    }

    pub fn install_model(
        &self,
        profile_id: &str,
        on_event: Arc<dyn Fn(ModelInstallEvent) + Send + Sync>,
    ) -> AppResult<AgentEnvironment> {
        let artifact = install_profile(&self.model_cache_root, profile_id, on_event)?;
        self.store.record_installation(&artifact)?;
        self.environment()
    }

    pub fn unload_model(&self) -> AppResult<()> {
        self.managed_runtime.unload()
    }

    pub fn check_external_provider(&self, config: ExternalProviderConfig) -> AppResult<String> {
        OpenAiCompatibleLocalProvider::new(config)?.health_check()
    }

    pub fn run(
        &self,
        request: AgentRequest,
        on_event: Arc<dyn Fn(AgentStreamEvent) + Send + Sync>,
    ) -> AppResult<AgentRunResult> {
        let _generation = match self.generation_guard.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                return Err(AppError::Agent(
                    "Another local generation is already running. Stop it or wait for completion."
                        .to_owned(),
                ));
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(AppError::Agent(
                    "The local generation guard is unavailable after a prior failure.".to_owned(),
                ));
            }
        };
        let total_started = Instant::now();
        emit_stage(
            &on_event,
            "resolving-context",
            "Resolving intent and explicit scope",
        );
        let (context_capacity, provider_label, model_label) = self.provider_metadata(&request)?;
        emit_stage(
            &on_event,
            "retrieving",
            "Running bounded local evidence tools",
        );
        let context_started = Instant::now();
        let retrieved = retrieve_run(&self.database, &self.semantic, &request)?;
        #[cfg(test)]
        {
            *self
                .observed_retrieval
                .lock()
                .expect("test retrieval observer") = retrieved.raw_evidence.clone();
        }
        let requires_generation = retrieved.requires_generation();
        let provider_result = requires_generation.then(|| self.build_provider(&request));
        if requires_generation {
            let message = match request.provider_kind {
                AgentProviderKind::GraphiteManaged => {
                    "Loading the selected model tokenizer for exact context accounting"
                }
                AgentProviderKind::OpenaiCompatible => {
                    "Preparing conservative estimated context accounting"
                }
            };
            emit_stage(&on_event, "tokenizing-context", message);
        }
        let estimator = ConservativeTokenEstimator;
        let token_counter: &dyn EvidenceTokenCounter = match provider_result.as_ref() {
            Some(Ok(provider)) => provider.as_ref(),
            _ => &estimator,
        };
        emit_stage(
            &on_event,
            "assembling-context",
            "Deduplicating evidence and enforcing per-paper budgets",
        );
        let prepared = prepare_run(retrieved, &request, context_capacity, token_counter)?;
        let history = self.store.recent_history(
            request
                .conversation_id
                .as_deref()
                .filter(|_| !request.context.selected_only && !request.context.lens_only),
        )?;
        let generation_profile = serde_json::to_string(&serde_json::json!({
            "temperature": 0.1,
            "seed": 42,
            "maximumOutputTokens": output_tokens(&request),
            "contextCapacity": context_capacity,
            "contextTokens": prepared.context_tokens,
            "runtimeContextPolicy": "next-power-of-two-min-2048-v1",
            "answerDetail": request.detail.as_str(),
        }))?;
        let (run_id, conversation_id) = self.store.begin_run(
            &request,
            &prepared,
            &provider_label,
            &model_label,
            &generation_profile,
        )?;
        let provider = match provider_result {
            Some(Ok(provider)) => Some(provider),
            Some(Err(error)) => {
                self.store.fail_run(&run_id, &error.to_string())?;
                return Err(error);
            }
            None => None,
        };
        let cancellation = Arc::new(AtomicBool::new(false));
        *self
            .cancellation
            .lock()
            .map_err(|_| AppError::Agent("The cancellation state is unavailable.".to_owned()))? =
            Some(cancellation.clone());
        let prompt = if history.is_empty() {
            user_prompt(&request.question, &prepared)
        } else {
            format!(
                "Conversation history is continuity context, not scientific evidence:\n{}\n\n{}",
                history,
                user_prompt(&request.question, &prepared)
            )
        };
        let context_ms =
            (context_started.elapsed().as_millis() as u64).saturating_sub(prepared.retrieval_ms);
        let no_scientific_evidence = !prepared
            .evidence
            .iter()
            .any(|source| source.eligible_for_scientific_claim);
        let deterministic_abstention = prepared.evidence.is_empty()
            || no_scientific_evidence
                && matches!(
                    prepared.intent,
                    super::model::AgentIntent::CurrentPaperQa
                        | super::model::AgentIntent::MultiPaperComparison
                        | super::model::AgentIntent::StructuredExtraction
                        | super::model::AgentIntent::LibrarySynthesis
                );
        let mut streamed = String::new();
        let generation = if deterministic_abstention {
            let answer = abstention_answer(&prepared);
            on_event(AgentStreamEvent {
                kind: "delta".to_owned(),
                stage: Some("grounding-gate".to_owned()),
                message: None,
                delta: Some(answer.clone()),
                result: None,
            });
            streamed = answer.clone();
            super::model::GenerationResult {
                text: answer,
                generated_tokens: 0,
                time_to_first_token_ms: Some(0),
                generation_ms: 0,
            }
        } else {
            let provider = provider.as_ref().ok_or_else(|| {
                AppError::Agent(
                    "The grounded run required generation but no provider was prepared.".to_owned(),
                )
            })?;
            if provider.context_capacity() != context_capacity {
                let message = format!(
                    "Provider {} reported a context capacity that changed during setup.",
                    provider.provider_id()
                );
                self.store.fail_run(&run_id, &message)?;
                self.clear_cancellation();
                return Err(AppError::LocalModel(message));
            }
            emit_stage(
                &on_event,
                "generating",
                &format!(
                    "Generating with {} / {}",
                    provider.provider_label(),
                    provider.model_label()
                ),
            );
            let generation_request = GenerationRequest {
                system_prompt: system_prompt(&prepared.intent, request.detail.as_str()),
                user_prompt: prompt,
                max_tokens: output_tokens(&request),
                context_tokens: context_capacity,
                temperature: 0.1,
            };
            let result =
                provider.generate(&generation_request, cancellation.clone(), &mut |delta| {
                    streamed.push_str(delta);
                    on_event(AgentStreamEvent {
                        kind: "delta".to_owned(),
                        stage: Some("generating".to_owned()),
                        message: None,
                        delta: Some(delta.to_owned()),
                        result: None,
                    });
                });
            match result {
                Ok(result) => result,
                Err(error) if cancellation.load(Ordering::SeqCst) => {
                    self.store
                        .cancel_run(&run_id, &conversation_id, &streamed)?;
                    self.clear_cancellation();
                    on_event(AgentStreamEvent {
                        kind: "cancelled".to_owned(),
                        stage: Some("cancelled".to_owned()),
                        message: Some(
                            "Generation stopped; the partial answer is labelled cancelled."
                                .to_owned(),
                        ),
                        delta: None,
                        result: None,
                    });
                    return Err(error);
                }
                Err(error) => {
                    self.store.fail_run(&run_id, &error.to_string())?;
                    self.clear_cancellation();
                    return Err(error);
                }
            }
        };
        emit_stage(
            &on_event,
            "validating-citations",
            "Checking citation scope, eligibility, and completeness",
        );
        let validated = validate_answer_for_question(
            &request.question,
            &generation.text,
            &prepared.intent,
            &prepared.evidence,
        );
        let (answer_markdown, citations, generation) = match validated {
            Ok((answer, citations)) => (answer, citations, generation),
            Err(issues) if !deterministic_abstention => {
                on_event(AgentStreamEvent {
                    kind: "reset".to_owned(),
                    stage: Some("repairing-citations".to_owned()),
                    message: Some("Repairing citation placement once".to_owned()),
                    delta: None,
                    result: None,
                });
                streamed.clear();
                let provider = provider.as_ref().ok_or_else(|| {
                    AppError::Agent(
                        "Citation repair required generation but no provider was prepared."
                            .to_owned(),
                    )
                })?;
                let repair_request = GenerationRequest {
                    system_prompt: system_prompt(&prepared.intent, request.detail.as_str()),
                    user_prompt: repair_prompt(
                        &request.question,
                        &generation.text,
                        &issues,
                        &prepared,
                    ),
                    max_tokens: output_tokens(&request),
                    context_tokens: context_capacity,
                    temperature: 0.0,
                };
                let repaired =
                    provider.generate(&repair_request, cancellation.clone(), &mut |delta| {
                        streamed.push_str(delta);
                        on_event(AgentStreamEvent {
                            kind: "delta".to_owned(),
                            stage: Some("repairing-citations".to_owned()),
                            message: None,
                            delta: Some(delta.to_owned()),
                            result: None,
                        });
                    });
                let repaired = match repaired {
                    Ok(result) => result,
                    Err(error) if cancellation.load(Ordering::SeqCst) => {
                        self.store
                            .cancel_run(&run_id, &conversation_id, &streamed)?;
                        self.clear_cancellation();
                        on_event(AgentStreamEvent {
                            kind: "cancelled".to_owned(),
                            stage: Some("cancelled".to_owned()),
                            message: Some(
                                "Citation repair stopped; the partial answer is labelled cancelled."
                                    .to_owned(),
                            ),
                            delta: None,
                            result: None,
                        });
                        return Err(error);
                    }
                    Err(error) => {
                        self.store.fail_run(&run_id, &error.to_string())?;
                        self.clear_cancellation();
                        return Err(error);
                    }
                };
                match validate_answer_for_question(
                    &request.question,
                    &repaired.text,
                    &prepared.intent,
                    &prepared.evidence,
                ) {
                    Ok((answer, citations)) => (answer, citations, repaired),
                    Err(repair_issues) => {
                        let message = format!(
                            "The local draft failed grounding validation after one repair: {}",
                            repair_issues.join("; ")
                        );
                        self.store.fail_run(&run_id, &message)?;
                        self.clear_cancellation();
                        return Err(AppError::Agent(message));
                    }
                }
            }
            Err(issues) => {
                let message = format!("The grounding gate failed: {}", issues.join("; "));
                self.store.fail_run(&run_id, &message)?;
                self.clear_cancellation();
                return Err(AppError::Agent(message));
            }
        };
        let total_ms = total_started.elapsed().as_millis() as u64;
        let mut limitations = prepared.limitations;
        if request.provider_kind == AgentProviderKind::GraphiteManaged {
            let hardware = self.managed_runtime.hardware_status()?;
            if hardware.status == "cpu-fallback"
                || (hardware.backend == "CPU" && super::hardware::compiled_backend() != "CPU")
            {
                limitations.push(hardware.note);
            }
        }
        let result = AgentRunResult {
            run_id,
            conversation_id,
            question: request.question,
            intent: prepared.intent,
            answer_markdown,
            status: "complete".to_owned(),
            provider_label,
            model_label,
            context_label: prepared.scope.label,
            context_tokens: prepared.context_tokens,
            generation_tokens: generation.generated_tokens,
            sources: prepared.evidence,
            citations,
            limitations,
            actions: prepared.actions,
            tool_calls: prepared.tool_calls,
            prompt_versions: vec![
                super::grounding::SYSTEM_PROMPT_VERSION.to_owned(),
                super::grounding::CONTEXT_ASSEMBLER_VERSION.to_owned(),
                super::grounding::CITATION_VALIDATOR_VERSION.to_owned(),
            ],
            timings: AgentTimings {
                retrieval_ms: prepared.retrieval_ms,
                context_ms,
                time_to_first_token_ms: generation.time_to_first_token_ms,
                generation_ms: generation.generation_ms,
                total_ms,
            },
        };
        self.store.complete_run(&result)?;
        self.clear_cancellation();
        on_event(AgentStreamEvent {
            kind: "complete".to_owned(),
            stage: Some("saved-locally".to_owned()),
            message: Some("Grounded answer and reproducibility record saved locally".to_owned()),
            delta: None,
            result: Some(result.clone()),
        });
        Ok(result)
    }

    pub fn cancel(&self) {
        if let Ok(cancellation) = self.cancellation.lock() {
            if let Some(cancellation) = cancellation.as_ref() {
                cancellation.store(true, Ordering::SeqCst);
            }
        }
    }

    pub fn conversations(&self) -> AppResult<Vec<ConversationSummary>> {
        self.store.conversations()
    }

    pub fn conversation(&self, conversation_id: &str) -> AppResult<ConversationDetail> {
        self.store.conversation(conversation_id)
    }

    pub fn delete_conversation(&self, conversation_id: &str) -> AppResult<()> {
        self.store.delete_conversation(conversation_id)
    }

    pub fn export_markdown(&self, conversation_id: &str) -> AppResult<String> {
        self.store.export_markdown(conversation_id)
    }

    fn provider_metadata(&self, request: &AgentRequest) -> AppResult<(u32, String, String)> {
        match request.provider_kind {
            AgentProviderKind::GraphiteManaged => {
                let profile = profile_spec(&request.model_profile)?;
                Ok((
                    request.context_window.tokens(profile.context_capacity),
                    "Graphite managed - offline".to_owned(),
                    profile.label.to_owned(),
                ))
            }
            AgentProviderKind::OpenaiCompatible => {
                let config = request.external_provider.as_ref().ok_or_else(|| {
                    AppError::LocalModel(
                        "External loopback provider settings were not supplied.".to_owned(),
                    )
                })?;
                Ok((
                    config.context_tokens,
                    "External loopback provider".to_owned(),
                    config.model.clone(),
                ))
            }
        }
    }

    fn build_provider(&self, request: &AgentRequest) -> AppResult<Box<dyn LocalModelProvider>> {
        match request.provider_kind {
            AgentProviderKind::GraphiteManaged => {
                let profile = profile_spec(&request.model_profile)?;
                let path = self.store.model_path(profile)?;
                Ok(Box::new(
                    GraphiteManagedProvider::new(self.managed_runtime.clone(), profile.id, path)?
                        .with_preferences(request.inference_backend, request.context_window),
                ))
            }
            AgentProviderKind::OpenaiCompatible => {
                Ok(Box::new(OpenAiCompatibleLocalProvider::new(
                    request.external_provider.clone().ok_or_else(|| {
                        AppError::LocalModel(
                            "External loopback provider settings were not supplied.".to_owned(),
                        )
                    })?,
                )?))
            }
        }
    }

    fn clear_cancellation(&self) {
        if let Ok(mut state) = self.cancellation.lock() {
            *state = None;
        }
    }
}

fn emit_stage(on_event: &Arc<dyn Fn(AgentStreamEvent) + Send + Sync>, stage: &str, message: &str) {
    on_event(AgentStreamEvent {
        kind: "stage".to_owned(),
        stage: Some(stage.to_owned()),
        message: Some(message.to_owned()),
        delta: None,
        result: None,
    });
}

fn output_tokens(request: &AgentRequest) -> u32 {
    if request.detail.as_str() == "detailed" {
        850
    } else {
        420
    }
}

fn hardware_profile(runtime: super::hardware::RuntimeHardware) -> HardwareProfile {
    let mut system = System::new_all();
    system.refresh_memory();
    let total_memory_bytes = system.total_memory();
    let available_memory_bytes = system.available_memory();
    let logical_cpus = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let recommended_profile = if total_memory_bytes >= 28 * 1024 * 1024 * 1024 {
        "balanced"
    } else if total_memory_bytes >= 14 * 1024 * 1024 * 1024 {
        "lightweight"
    } else {
        "insufficient-memory"
    };
    HardwareProfile {
        cpu_name: system
            .cpus()
            .first()
            .map(|cpu| cpu.brand().to_owned())
            .unwrap_or_default(),
        compiled_backend: super::hardware::compiled_backend(),
        execution_status: runtime.status,
        gpu_devices: runtime.devices,
        graphics_adapters: super::hardware::graphics_adapters(),
        logical_cpus,
        total_memory_bytes,
        available_memory_bytes,
        operating_system: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        acceleration_backend: runtime.backend,
        acceleration_available: runtime.available,
        recommended_profile: recommended_profile.to_owned(),
        note: runtime.note,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{hardware_profile, output_tokens};
    use crate::agent::model::{AgentContext, AgentDetail, AgentProviderKind, AgentRequest};

    #[test]
    fn generation_profiles_keep_output_bounded() {
        let mut request = AgentRequest {
            question: "Question".to_owned(),
            conversation_id: None,
            provider_kind: AgentProviderKind::GraphiteManaged,
            model_profile: "lightweight".to_owned(),
            inference_backend: Default::default(),
            context_window: Default::default(),
            detail: AgentDetail::Concise,
            context: AgentContext::default(),
            external_provider: None,
        };
        assert_eq!(output_tokens(&request), 420);
        request.detail = AgentDetail::Detailed;
        assert_eq!(output_tokens(&request), 850);
    }

    #[test]
    fn hardware_profile_reports_a_local_runtime_recommendation() {
        let profile = hardware_profile(super::super::hardware::RuntimeHardware::describe(
            vec![],
            None,
            false,
            None,
            Default::default(),
            Ok(Default::default()),
        ));
        assert!(profile.logical_cpus > 0);
        assert!(profile.total_memory_bytes > 0);
        assert!(matches!(
            profile.recommended_profile.as_str(),
            "balanced" | "lightweight" | "insufficient-memory"
        ));
        assert!(!profile.acceleration_available);
    }

    #[test]
    #[ignore = "uses an explicit local library copy and installed GGUF profile"]
    fn live_real_library_workflows_are_grounded_and_persistent() {
        let source_database =
            std::env::var("GRAPHITE_M5_LIVE_DATABASE").expect("set GRAPHITE_M5_LIVE_DATABASE");
        let semantic_root = std::path::PathBuf::from(
            std::env::var("GRAPHITE_M5_SEMANTIC_ROOT").expect("set GRAPHITE_M5_SEMANTIC_ROOT"),
        );
        let model_cache = std::path::PathBuf::from(
            std::env::var("GRAPHITE_M5_MODEL_CACHE").expect("set GRAPHITE_M5_MODEL_CACHE"),
        );
        let profile_id =
            std::env::var("GRAPHITE_M5_PROFILE").unwrap_or_else(|_| "lightweight".to_owned());
        let copied_database = std::env::temp_dir().join(format!(
            "graphite-m5-live-library-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        std::fs::copy(source_database, &copied_database).expect("copy live Graphite database");
        let database = crate::database::ResearchDatabase::initialize(copied_database.clone())
            .expect("copied database");
        let semantic = crate::semantic::SemanticCoordinator::new(
            database.clone(),
            semantic_root.join("semantic-models"),
            semantic_root.join("semantic-indexes"),
        )
        .expect("semantic coordinator");
        let agent = super::AgentCoordinator::new(database.clone(), semantic, model_cache.clone())
            .expect("agent coordinator");
        if let Ok(relative_path) = std::env::var("GRAPHITE_M5_VERIFIED_MODEL_RELATIVE_PATH") {
            let profile = crate::agent::managed::profile_spec(&profile_id).expect("known profile");
            let installed_bytes = std::fs::metadata(model_cache.join(&relative_path))
                .expect("verified cached model")
                .len();
            agent
                .store
                .record_installation(&crate::agent::managed::InstalledArtifact {
                    profile,
                    cache_relative_path: relative_path,
                    installed_bytes,
                })
                .expect("record previously verified model in copied database");
        } else {
            agent
                .install_model(&profile_id, Arc::new(|_| {}))
                .expect("record verified model in copied database");
        }
        let connection = database.connect().expect("database connection");
        let scoped_items = connection
            .prepare(
                "SELECT sc.item_id
                 FROM semantic_chunks sc
                 JOIN semantic_vector_entries sve
                   ON sve.vector_key = sc.vector_key
                 WHERE sc.status = 'active' AND sve.active = 1
                 GROUP BY sc.item_id ORDER BY COUNT(*) DESC LIMIT 2",
            )
            .expect("scoped item query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("scoped item rows")
            .collect::<Result<Vec<_>, _>>()
            .expect("scoped items");
        assert_eq!(scoped_items.len(), 2, "two indexed papers are required");
        let citation_items = connection
            .query_row(
                "SELECT left_edge.source_item_id, right_edge.source_item_id
                 FROM citation_edges left_edge
                 JOIN citation_edges right_edge
                   ON right_edge.target_item_id = left_edge.target_item_id
                  AND right_edge.source_item_id > left_edge.source_item_id
                 LIMIT 1",
                [],
                |row| Ok(vec![row.get::<_, String>(0)?, row.get::<_, String>(1)?]),
            )
            .expect("a shared-reference pair is required");
        drop(connection);

        let workflows = [
            (
                "current-paper",
                "What method or preprocessing does this paper report?",
                crate::agent::model::AgentContext {
                    current_item_id: Some(scoped_items[0].clone()),
                    ..crate::agent::model::AgentContext::default()
                },
            ),
            (
                "discovery",
                "List papers investigating surface-wave dispersion in crustal settings.",
                crate::agent::model::AgentContext::default(),
            ),
            (
                "comparison",
                "Compare the methods used by these selected papers.",
                crate::agent::model::AgentContext {
                    selected_item_ids: scoped_items.clone(),
                    ..crate::agent::model::AgentContext::default()
                },
            ),
            (
                "structured-extraction",
                "Extract a table of the methods reported by these selected papers.",
                crate::agent::model::AgentContext {
                    selected_item_ids: scoped_items.clone(),
                    ..crate::agent::model::AgentContext::default()
                },
            ),
            (
                "citation-structure",
                "Which references are shared by these papers?",
                crate::agent::model::AgentContext {
                    selected_item_ids: citation_items,
                    ..crate::agent::model::AgentContext::default()
                },
            ),
        ];
        let workflow_start = std::env::var("GRAPHITE_M5_WORKFLOW_START").ok();
        let mut conversation_id = None;
        let mut completed_workflows = 0_usize;
        let mut started = workflow_start.is_none();
        for (name, question, context) in workflows {
            if workflow_start.as_deref() == Some(name) {
                started = true;
            }
            if !started {
                continue;
            }
            let result = agent
                .run(
                    crate::agent::model::AgentRequest {
                        question: question.to_owned(),
                        conversation_id: conversation_id.clone(),
                        provider_kind: crate::agent::model::AgentProviderKind::GraphiteManaged,
                        model_profile: profile_id.clone(),
                        inference_backend: Default::default(),
                        context_window: Default::default(),
                        detail: crate::agent::model::AgentDetail::Concise,
                        context,
                        external_provider: None,
                    },
                    Arc::new(|_| {}),
                )
                .unwrap_or_else(|error| panic!("{name} workflow failed: {error}"));
            assert_eq!(result.status, "complete");
            assert!(!result.answer_markdown.trim().is_empty());
            assert!(
                !result.citations.is_empty()
                    || crate::agent::grounding::is_abstention(&result.answer_markdown)
            );
            eprintln!(
                "live-workflow={name} total-ms={} ttft-ms={:?} sources={}",
                result.timings.total_ms,
                result.timings.time_to_first_token_ms,
                result.sources.len()
            );
            conversation_id = Some(result.conversation_id);
            completed_workflows += 1;
        }
        let conversation_id = conversation_id.expect("conversation identity");
        let saved = agent
            .conversation(&conversation_id)
            .expect("saved conversation");
        assert_eq!(saved.runs.len(), completed_workflows);
        let markdown = agent
            .export_markdown(&conversation_id)
            .expect("Markdown export");
        assert!(markdown.contains("### Sources"));
        let cancellable_agent = agent.clone();
        let cancellation_profile = profile_id.clone();
        let cancellation_item = scoped_items[0].clone();
        let cancellation_run = std::thread::spawn(move || {
            cancellable_agent.run(
                crate::agent::model::AgentRequest {
                    question: "Give a detailed account of the methods and limitations reported by this paper.".to_owned(),
                    conversation_id: None,
                    provider_kind:
                        crate::agent::model::AgentProviderKind::GraphiteManaged,
                    model_profile: cancellation_profile,
                    inference_backend: Default::default(),
                    context_window: Default::default(),
                    detail: crate::agent::model::AgentDetail::Detailed,
                    context: crate::agent::model::AgentContext {
                        current_item_id: Some(cancellation_item),
                        ..crate::agent::model::AgentContext::default()
                    },
                    external_provider: None,
                },
                Arc::new(|_| {}),
            )
        });
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            agent.cancel();
            if cancellation_run.is_finished() {
                break;
            }
        }
        let cancellation_result = cancellation_run.join().expect("cancellation thread");
        assert!(cancellation_result.is_err());
        eprintln!("live-workflow=cancelled-generation status=cancelled");
        let _ = std::fs::remove_file(copied_database);
    }
}
