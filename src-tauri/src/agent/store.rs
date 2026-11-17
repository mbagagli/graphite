use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use rusqlite::{params, OptionalExtension};

use crate::database::ResearchDatabase;
use crate::error::{AppError, AppResult};

use super::grounding::{source_is_stale, PreparedRun};
use super::managed::{InstalledArtifact, ProfileSpec};
use super::model::{
    AgentDetail, AgentProviderKind, AgentRequest, AgentRunResult, AgentSettings, ContextWindow,
    ConversationDetail, ConversationSummary, ConversationTurn, InferenceBackend,
};

static ID_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct AgentStore {
    database: ResearchDatabase,
    model_cache_root: PathBuf,
}

impl AgentStore {
    pub fn new(database: ResearchDatabase, model_cache_root: PathBuf) -> Self {
        Self {
            database,
            model_cache_root,
        }
    }

    pub fn settings(&self) -> AppResult<AgentSettings> {
        let connection = self.database.connect()?;
        connection
            .query_row(
                "SELECT provider_kind, model_profile, external_endpoint,
                    external_model, external_context_tokens, answer_detail,
                    inference_backend, context_window
             FROM agent_settings WHERE id = 1",
                [],
                |row| {
                    let provider: String = row.get(0)?;
                    let detail: String = row.get(5)?;
                    Ok(AgentSettings {
                        provider_kind: if provider == "openai-compatible" {
                            AgentProviderKind::OpenaiCompatible
                        } else {
                            AgentProviderKind::GraphiteManaged
                        },
                        model_profile: row.get(1)?,
                        inference_backend: match row.get::<_, String>(6)?.as_str() {
                            "cpu" => InferenceBackend::Cpu,
                            "gpu" => InferenceBackend::Gpu,
                            _ => InferenceBackend::Auto,
                        },
                        context_window: match row.get::<_, String>(7)?.as_str() {
                            "low" => ContextWindow::Low,
                            "high" => ContextWindow::High,
                            "extra-high" => ContextWindow::ExtraHigh,
                            _ => ContextWindow::Medium,
                        },
                        external_endpoint: row.get(2)?,
                        external_model: row.get(3)?,
                        external_context_tokens: row
                            .get::<_, Option<i64>>(4)?
                            .map(|value| value.max(0) as u32),
                        answer_detail: if detail == "detailed" {
                            AgentDetail::Detailed
                        } else {
                            AgentDetail::Concise
                        },
                    })
                },
            )
            .map_err(Into::into)
    }

    pub fn save_settings(&self, settings: &AgentSettings) -> AppResult<()> {
        if !matches!(settings.model_profile.as_str(), "lightweight" | "balanced") {
            return Err(AppError::Agent(
                "Model profile must be lightweight or balanced.".to_owned(),
            ));
        }
        let connection = self.database.connect()?;
        connection.execute(
            "UPDATE agent_settings SET
                provider_kind = ?1, model_profile = ?2,
                external_endpoint = ?3, external_model = ?4,
                external_context_tokens = ?5, answer_detail = ?6,
                updated_at = ?7, inference_backend = ?8, context_window = ?9
             WHERE id = 1",
            params![
                settings.provider_kind.as_str(),
                settings.model_profile,
                settings.external_endpoint,
                settings.external_model,
                settings.external_context_tokens,
                settings.answer_detail.as_str(),
                Utc::now().to_rfc3339(),
                settings.inference_backend.as_str(),
                settings.context_window.as_str(),
            ],
        )?;
        Ok(())
    }

    pub fn installation_summary(
        &self,
        profile_id: &str,
    ) -> AppResult<Option<(String, u64, Option<String>)>> {
        let connection = self.database.connect()?;
        connection
            .query_row(
                "SELECT status, installed_bytes, last_error
                 FROM agent_model_installations WHERE profile_id = ?1",
                [profile_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get::<_, i64>(1)?.max(0) as u64,
                        row.get(2)?,
                    ))
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn record_installation(&self, artifact: &InstalledArtifact) -> AppResult<()> {
        let connection = self.database.connect()?;
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "INSERT INTO agent_model_installations (
                profile_id, provider_kind, model_identifier, model_revision,
                artifact_identity, artifact_filename, quantization,
                expected_bytes, installed_bytes, sha256, context_capacity,
                cache_relative_path, license, status, installed_at,
                last_checked_at, last_error
             ) VALUES (
                ?1, 'graphite-managed', ?2, ?3, ?4, ?5, 'Q4_K_M',
                ?6, ?7, ?8, ?9, ?10, 'Apache-2.0', 'ready', ?11, ?11, NULL
             ) ON CONFLICT(profile_id) DO UPDATE SET
                model_identifier = excluded.model_identifier,
                model_revision = excluded.model_revision,
                artifact_identity = excluded.artifact_identity,
                artifact_filename = excluded.artifact_filename,
                expected_bytes = excluded.expected_bytes,
                installed_bytes = excluded.installed_bytes,
                sha256 = excluded.sha256,
                context_capacity = excluded.context_capacity,
                cache_relative_path = excluded.cache_relative_path,
                status = 'ready', installed_at = excluded.installed_at,
                last_checked_at = excluded.last_checked_at, last_error = NULL",
            params![
                artifact.profile.id,
                artifact.profile.repo_id,
                artifact.profile.revision,
                artifact_identity(artifact.profile),
                artifact.profile.filename,
                artifact.profile.expected_bytes as i64,
                artifact.installed_bytes as i64,
                artifact.profile.sha256,
                artifact.profile.context_capacity,
                artifact.cache_relative_path,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn model_path(&self, profile: ProfileSpec) -> AppResult<PathBuf> {
        let connection = self.database.connect()?;
        let relative = connection
            .query_row(
                "SELECT cache_relative_path FROM agent_model_installations
                 WHERE profile_id = ?1 AND status = 'ready'
                   AND model_revision = ?2 AND sha256 = ?3
                   AND expected_bytes = ?4",
                params![
                    profile.id,
                    profile.revision,
                    profile.sha256,
                    profile.expected_bytes as i64,
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| {
                AppError::LocalModel(format!(
                    "The {} profile is not installed. Install it explicitly in Agent settings.",
                    profile.label
                ))
            })?;
        let path = safe_cache_path(&self.model_cache_root, &relative)?;
        let size = path
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(|error| {
                AppError::LocalModel(format!("The installed model is unavailable: {error}"))
            })?;
        if size != profile.expected_bytes {
            return Err(AppError::LocalModel(
                "The installed model no longer matches its curated byte size.".to_owned(),
            ));
        }
        Ok(path)
    }

    pub fn begin_run(
        &self,
        request: &AgentRequest,
        prepared: &PreparedRun,
        provider_label: &str,
        model_label: &str,
        generation_json: &str,
    ) -> AppResult<(String, String)> {
        let connection = self.database.connect()?;
        let now = Utc::now().to_rfc3339();
        let conversation_id = request
            .conversation_id
            .clone()
            .unwrap_or_else(|| new_id("conversation"));
        let title = conversation_title(&request.question);
        let library_id = connection
            .query_row(
                "SELECT value FROM app_meta WHERE key = 'active_library_id'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        connection.execute(
            "INSERT INTO agent_conversations(id, library_id, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET updated_at = excluded.updated_at",
            params![conversation_id, library_id, title, now],
        )?;
        let run_id = new_id("agent-run");
        let scope_json = serde_json::to_string(&prepared.scope)?;
        let priority_json = serde_json::to_string(&prepared.scope.priority_item_ids)?;
        let provider_json = serde_json::to_string(&serde_json::json!({
            "provider": provider_label,
            "model": model_label,
            "kind": request.provider_kind.as_str()
        }))?;
        let prompt_versions = serde_json::to_string(&vec![
            super::grounding::SYSTEM_PROMPT_VERSION,
            super::grounding::CONTEXT_ASSEMBLER_VERSION,
            super::grounding::CITATION_VALIDATOR_VERSION,
        ])?;
        let tools_json = serde_json::to_string(&prepared.tool_calls)?;
        let retrieval_json = serde_json::to_string(&prepared.retrieval_trace)?;
        let context_json = serde_json::to_string(&prepared.context_stats)?;
        let limitations_json = serde_json::to_string(&prepared.limitations)?;
        let actions_json = serde_json::to_string(&prepared.actions)?;
        connection.execute(
            "INSERT INTO agent_runs (
                id, conversation_id, question, intent, scope_json,
                priority_context_json, provider_json, generation_json,
                prompt_versions_json, tool_calls_json, retrieval_json,
                context_json, limitations_json, actions_json,
                completion_state, created_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, 'running', ?15
             )",
            params![
                run_id,
                conversation_id,
                request.question,
                prepared.intent.as_str(),
                scope_json,
                priority_json,
                provider_json,
                generation_json,
                prompt_versions,
                tools_json,
                retrieval_json,
                context_json,
                limitations_json,
                actions_json,
                now,
            ],
        )?;
        connection.execute(
            "INSERT INTO agent_turns(
                id, conversation_id, agent_run_id, role, content, status, created_at
             ) VALUES (?1, ?2, ?3, 'user', ?4, 'complete', ?5)",
            params![
                new_id("turn"),
                conversation_id,
                run_id,
                request.question,
                now
            ],
        )?;
        for source in &prepared.evidence {
            connection.execute(
                "INSERT INTO agent_sources (
                    run_id, source_id, evidence_kind, item_id, item_title,
                    document_id, document_revision_id, chunk_id, passage_text,
                    character_start, character_end, page_start, page_end,
                    retrieval_modes_json, rank_json, provider,
                    source_fingerprint, eligible_for_scientific_claim
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                    ?12, ?13, ?14, ?15, ?16, ?17, ?18
                 )",
                params![
                    run_id,
                    source.source_id,
                    source.kind,
                    source.item_id,
                    source.item_title,
                    source.document_id,
                    source.document_revision_id,
                    source.chunk_id,
                    source.excerpt,
                    source.character_start.map(|value| value as i64),
                    source.character_end.map(|value| value as i64),
                    source.page_start,
                    source.page_end,
                    serde_json::to_string(&source.retrieval_modes)?,
                    serde_json::to_string(&serde_json::json!({
                        "lexicalRank": source.lexical_rank,
                        "semanticRank": source.semantic_rank,
                    }))?,
                    source.provider,
                    source.source_fingerprint,
                    source.eligible_for_scientific_claim,
                ],
            )?;
        }
        Ok((run_id, conversation_id))
    }

    pub fn complete_run(&self, result: &AgentRunResult) -> AppResult<()> {
        self.finish_run(result, "complete")
    }

    pub fn cancel_run(
        &self,
        run_id: &str,
        conversation_id: &str,
        partial_answer: &str,
    ) -> AppResult<()> {
        let connection = self.database.connect()?;
        let now = Utc::now().to_rfc3339();
        connection.execute(
            "UPDATE agent_runs SET answer_markdown = ?2,
                completion_state = 'cancelled', completed_at = ?3
             WHERE id = ?1",
            params![run_id, partial_answer, now],
        )?;
        if !partial_answer.trim().is_empty() {
            connection.execute(
                "INSERT INTO agent_turns(
                    id, conversation_id, agent_run_id, role, content, status, created_at
                 ) VALUES (?1, ?2, ?3, 'assistant', ?4, 'cancelled', ?5)",
                params![new_id("turn"), conversation_id, run_id, partial_answer, now],
            )?;
        }
        Ok(())
    }

    pub fn fail_run(&self, run_id: &str, message: &str) -> AppResult<()> {
        let connection = self.database.connect()?;
        connection.execute(
            "UPDATE agent_runs SET completion_state = 'failed',
                limitations_json = ?2, completed_at = ?3 WHERE id = ?1",
            params![
                run_id,
                serde_json::to_string(&vec![message])?,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    fn finish_run(&self, result: &AgentRunResult, state: &str) -> AppResult<()> {
        let connection = self.database.connect()?;
        let now = Utc::now().to_rfc3339();
        let result_json = serde_json::to_string(result)?;
        connection.execute(
            "UPDATE agent_runs SET answer_markdown = ?2, citations_json = ?3,
                limitations_json = ?4, actions_json = ?5, timings_json = ?6,
                result_json = ?7, completion_state = ?8, completed_at = ?9
             WHERE id = ?1",
            params![
                result.run_id,
                result.answer_markdown,
                serde_json::to_string(&result.citations)?,
                serde_json::to_string(&result.limitations)?,
                serde_json::to_string(&result.actions)?,
                serde_json::to_string(&result.timings)?,
                result_json,
                state,
                now,
            ],
        )?;
        connection.execute(
            "INSERT INTO agent_turns(
                id, conversation_id, agent_run_id, role, content, status, created_at
             ) VALUES (?1, ?2, ?3, 'assistant', ?4, ?5, ?6)",
            params![
                new_id("turn"),
                result.conversation_id,
                result.run_id,
                result.answer_markdown,
                state,
                now,
            ],
        )?;
        connection.execute(
            "UPDATE agent_conversations SET updated_at = ?2 WHERE id = ?1",
            params![result.conversation_id, now],
        )?;
        Ok(())
    }

    pub fn conversations(&self) -> AppResult<Vec<ConversationSummary>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT c.id, c.title, c.created_at, c.updated_at, COUNT(t.id)
             FROM agent_conversations c
             LEFT JOIN agent_turns t ON t.conversation_id = c.id
             GROUP BY c.id ORDER BY c.updated_at DESC LIMIT 100",
        )?;
        let conversations = statement
            .query_map([], |row| {
                Ok(ConversationSummary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                    turn_count: row.get::<_, i64>(4)?.max(0) as u32,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        Ok(conversations)
    }

    pub fn recent_history(&self, conversation_id: Option<&str>) -> AppResult<String> {
        let Some(conversation_id) = conversation_id else {
            return Ok(String::new());
        };
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT role, content FROM agent_turns
             WHERE conversation_id = ?1
             ORDER BY created_at DESC, id DESC LIMIT 6",
        )?;
        let mut turns = statement
            .query_map([conversation_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        turns.reverse();
        let mut output = String::new();
        for (role, content) in turns {
            let remaining = 6_000_usize.saturating_sub(output.len());
            if remaining == 0 {
                break;
            }
            let bounded = content.chars().take(remaining).collect::<String>();
            output.push_str(&format!("{}: {}\n", role, bounded));
        }
        Ok(output)
    }

    pub fn conversation(&self, conversation_id: &str) -> AppResult<ConversationDetail> {
        let connection = self.database.connect()?;
        let summary = connection
            .query_row(
                "SELECT c.id, c.title, c.created_at, c.updated_at,
                        (SELECT COUNT(*) FROM agent_turns t WHERE t.conversation_id = c.id)
                 FROM agent_conversations c WHERE c.id = ?1",
                [conversation_id],
                |row| {
                    Ok(ConversationSummary {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        created_at: row.get(2)?,
                        updated_at: row.get(3)?,
                        turn_count: row.get::<_, i64>(4)?.max(0) as u32,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| AppError::Agent("The conversation no longer exists.".to_owned()))?;
        let mut turn_statement = connection.prepare(
            "SELECT id, agent_run_id, role, content, status, created_at
             FROM agent_turns WHERE conversation_id = ?1 ORDER BY created_at, id",
        )?;
        let turns = turn_statement
            .query_map([conversation_id], |row| {
                Ok(ConversationTurn {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    status: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut run_statement = connection.prepare(
            "SELECT result_json FROM agent_runs
             WHERE conversation_id = ?1 AND result_json IS NOT NULL
             ORDER BY created_at",
        )?;
        let encoded_runs = run_statement
            .query_map([conversation_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut runs = Vec::new();
        for encoded in encoded_runs {
            let mut result: AgentRunResult = serde_json::from_str(&encoded)?;
            for source in &mut result.sources {
                source.stale = source_is_stale(&self.database, source)?;
            }
            runs.push(result);
        }
        Ok(ConversationDetail {
            summary,
            turns,
            runs,
        })
    }

    pub fn delete_conversation(&self, conversation_id: &str) -> AppResult<()> {
        let connection = self.database.connect()?;
        connection.execute(
            "DELETE FROM agent_conversations WHERE id = ?1",
            [conversation_id],
        )?;
        Ok(())
    }

    pub fn export_markdown(&self, conversation_id: &str) -> AppResult<String> {
        let detail = self.conversation(conversation_id)?;
        let mut output = format!(
            "# {}\n\nExported locally from Graphite at {}.\n\n",
            detail.summary.title,
            Utc::now().to_rfc3339()
        );
        for turn in &detail.turns {
            let heading = if turn.role == "user" {
                "Question"
            } else {
                "Answer"
            };
            output.push_str(&format!("## {heading}\n\n{}\n\n", turn.content));
            if let Some(run_id) = turn.run_id.as_deref() {
                if let Some(run) = detail.runs.iter().find(|run| run.run_id == run_id) {
                    output.push_str("### Sources\n\n");
                    for source in &run.sources {
                        let page = source.page_start.map_or_else(
                            || "page unavailable".to_owned(),
                            |value| format!("p. {value}"),
                        );
                        let stale = if source.stale {
                            " - source changed"
                        } else {
                            ""
                        };
                        output.push_str(&format!(
                            "- [{}] {} - {}{}\n",
                            source.source_id, source.item_title, page, stale
                        ));
                    }
                    output.push_str(&format!(
                        "\nRun: `{}` | Intent: `{}` | Provider: {} | Model: {}\n\n",
                        run.run_id,
                        run.intent.as_str(),
                        run.provider_label,
                        run.model_label
                    ));
                }
            }
        }
        Ok(output)
    }
}

fn safe_cache_path(root: &Path, relative: &str) -> AppResult<PathBuf> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AppError::LocalModel(
            "The persisted model path is not a safe cache-relative path.".to_owned(),
        ));
    }
    Ok(root.join(relative))
}

fn artifact_identity(profile: ProfileSpec) -> String {
    format!(
        "hf://{}/{}#{}",
        profile.repo_id, profile.filename, profile.revision
    )
}

fn conversation_title(question: &str) -> String {
    let mut title = question
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect::<String>();
    if title.is_empty() {
        title = "Research conversation".to_owned();
    }
    title
}

pub fn new_id(prefix: &str) -> String {
    let sequence = ID_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{}:{}:{}", prefix, Utc::now().timestamp_micros(), sequence)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{conversation_title, safe_cache_path, AgentStore};
    use crate::agent::grounding::{PreparedRun, ResolvedScope};
    use crate::agent::model::{
        AgentContext, AgentDetail, AgentIntent, AgentProviderKind, AgentRequest, AgentRunResult,
        AgentTimings, ContextAssemblyStats, RetrievalTrace,
    };
    use crate::database::ResearchDatabase;

    #[test]
    fn inference_preferences_default_and_persist_after_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.sqlite3");
        let database = ResearchDatabase::initialize(path.clone()).unwrap();
        let store = AgentStore::new(database, directory.path().to_path_buf());
        let mut settings = store.settings().unwrap();
        assert_eq!(settings.inference_backend, super::InferenceBackend::Auto);
        assert_eq!(settings.context_window, super::ContextWindow::Medium);
        settings.inference_backend = super::InferenceBackend::Cpu;
        settings.context_window = super::ContextWindow::ExtraHigh;
        store.save_settings(&settings).unwrap();
        drop(store);
        let reopened = AgentStore::new(
            ResearchDatabase::initialize(path).unwrap(),
            directory.path().to_path_buf(),
        );
        let settings = reopened.settings().unwrap();
        assert_eq!(settings.inference_backend, super::InferenceBackend::Cpu);
        assert_eq!(settings.context_window, super::ContextWindow::ExtraHigh);
    }

    #[test]
    fn installed_model_reopens_offline_and_missing_files_are_explicit() {
        let root = std::env::temp_dir().join(format!(
            "graphite-model-reopen-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        std::fs::create_dir_all(&root).unwrap();
        let database_path = root.join("research.sqlite3");
        let model_path = root.join("fixture.gguf");
        let bytes = b"local model storage fixture";
        std::fs::write(&model_path, bytes).unwrap();
        let profile = super::ProfileSpec {
            expected_bytes: bytes.len() as u64,
            ..super::super::managed::PROFILE_SPECS[0]
        };
        {
            let database = ResearchDatabase::initialize(database_path.clone()).unwrap();
            let store = AgentStore::new(database, root.clone());
            store
                .record_installation(&super::InstalledArtifact {
                    profile,
                    cache_relative_path: "fixture.gguf".into(),
                    installed_bytes: bytes.len() as u64,
                })
                .unwrap();
            assert_eq!(store.model_path(profile).unwrap(), model_path);
        }
        let database = ResearchDatabase::initialize(database_path).unwrap();
        let reopened = AgentStore::new(database, root.clone());
        assert_eq!(reopened.model_path(profile).unwrap(), model_path);
        assert_eq!(std::fs::read(&model_path).unwrap(), bytes);
        std::fs::remove_file(&model_path).unwrap();
        assert!(reopened
            .model_path(profile)
            .unwrap_err()
            .to_string()
            .contains("installed model is unavailable"));
        assert!(!model_path.exists());
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn model_paths_cannot_escape_the_private_cache() {
        assert!(safe_cache_path(Path::new("/tmp/graphite-models"), "models/file.gguf").is_ok());
        assert!(safe_cache_path(Path::new("/tmp/graphite-models"), "../file.gguf").is_err());
        assert!(safe_cache_path(Path::new("/tmp/graphite-models"), "/tmp/file.gguf").is_err());
    }

    #[test]
    fn conversation_titles_are_bounded() {
        let title = conversation_title(&"word ".repeat(100));
        assert!(title.chars().count() <= 80);
    }

    #[test]
    fn conversations_survive_restart_export_and_explicit_deletion() {
        let path = std::env::temp_dir().join(format!(
            "graphite-agent-store-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let model_root = std::env::temp_dir().join("graphite-agent-store-models");
        let database = ResearchDatabase::initialize(path.clone()).expect("agent database");
        let store = AgentStore::new(database, model_root.clone());
        let request = AgentRequest {
            question: "What does the selected evidence support?".to_owned(),
            conversation_id: None,
            provider_kind: AgentProviderKind::GraphiteManaged,
            model_profile: "lightweight".to_owned(),
            inference_backend: Default::default(),
            context_window: Default::default(),
            detail: AgentDetail::Concise,
            context: AgentContext::default(),
            external_provider: None,
        };
        let prepared = PreparedRun {
            intent: AgentIntent::LibrarySynthesis,
            scope: ResolvedScope {
                kind: "active-library".to_owned(),
                item_ids: Vec::new(),
                priority_item_ids: Vec::new(),
                collection_id: None,
                label: "Active library".to_owned(),
            },
            evidence: Vec::new(),
            context: String::new(),
            context_tokens: 0,
            retrieval_trace: RetrievalTrace {
                profile: "m4-hybrid-v1".to_owned(),
                requested_mode: "hybrid".to_owned(),
                effective_mode: "hybrid".to_owned(),
                retrieval_path: "m4-hybrid-v1".to_owned(),
                scope_kind: "active-library".to_owned(),
                scope_item_count: 0,
                scoped_chunk_limit_per_paper: None,
                loaded_chunk_count: 0,
                highest_loaded_chunk_ordinal: None,
                search_result_count: 0,
                lexical_result_count: 0,
                semantic_result_count: 0,
                fused_result_count: 0,
                evidence_candidate_count: 0,
                eligible_passage_count: 0,
                distinct_paper_count: 0,
                metadata_fallback_count: 0,
                fallback_reason: None,
                duration_ms: 1,
            },
            context_stats: ContextAssemblyStats {
                profile: "bounded-diverse-context-v2".to_owned(),
                token_accounting: "deterministic-whitespace-test-v1".to_owned(),
                budget_tokens: 6_000,
                used_tokens: 0,
                unused_tokens: 6_000,
                input_evidence_count: 0,
                scope_filtered_count: 0,
                exact_duplicates_omitted: 0,
                redundancy_policy: "exact-fingerprint-and-contained-range-v1".to_owned(),
                redundant_evidence_omitted: 0,
                selection_limit_omitted: 0,
                candidate_source_count: 0,
                included_source_count: 0,
                oversized_evidence_omitted: 0,
                omitted_for_budget: 0,
                stopped_on_non_fitting_source: false,
                first_non_fitting_source_id: None,
                first_non_fitting_source_tokens: None,
                first_non_fitting_source_oversized: false,
                unevaluated_after_budget_stop: 0,
                duration_ms: 1,
            },
            tool_calls: Vec::new(),
            limitations: vec!["Fixture evidence only".to_owned()],
            actions: Vec::new(),
            retrieval_ms: 1,
        };
        let (run_id, conversation_id) = store
            .begin_run(
                &request,
                &prepared,
                "Deterministic provider",
                "Fixture model",
                "{}",
            )
            .expect("begin run");
        let (retrieval_json, context_json) = store
            .database
            .connect()
            .expect("connection")
            .query_row(
                "SELECT retrieval_json, context_json FROM agent_runs WHERE id = ?1",
                [&run_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .expect("persisted traces");
        let retrieval: serde_json::Value =
            serde_json::from_str(&retrieval_json).expect("retrieval trace JSON");
        let context: serde_json::Value =
            serde_json::from_str(&context_json).expect("context trace JSON");
        assert_eq!(retrieval["retrievalPath"], "m4-hybrid-v1");
        assert_eq!(
            context["tokenAccounting"],
            "deterministic-whitespace-test-v1"
        );
        let result = AgentRunResult {
            run_id,
            conversation_id: conversation_id.clone(),
            question: request.question,
            intent: AgentIntent::LibrarySynthesis,
            answer_markdown: "Insufficient local evidence.".to_owned(),
            status: "complete".to_owned(),
            provider_label: "Deterministic provider".to_owned(),
            model_label: "Fixture model".to_owned(),
            context_label: "Active library".to_owned(),
            context_tokens: 0,
            generation_tokens: 0,
            sources: Vec::new(),
            citations: Vec::new(),
            limitations: vec!["Fixture evidence only".to_owned()],
            actions: Vec::new(),
            tool_calls: Vec::new(),
            prompt_versions: vec!["fixture-v1".to_owned()],
            timings: AgentTimings {
                retrieval_ms: 1,
                context_ms: 1,
                time_to_first_token_ms: Some(0),
                generation_ms: 0,
                total_ms: 2,
            },
        };
        store.complete_run(&result).expect("complete run");

        let reopened_database =
            ResearchDatabase::initialize(path.clone()).expect("reopen database");
        let reopened = AgentStore::new(reopened_database, model_root);
        let detail = reopened
            .conversation(&conversation_id)
            .expect("saved conversation");
        assert_eq!(detail.turns.len(), 2);
        assert_eq!(detail.runs.len(), 1);
        let markdown = reopened
            .export_markdown(&conversation_id)
            .expect("Markdown export");
        assert!(markdown.contains("## Question"));
        assert!(markdown.contains("## Answer"));
        reopened
            .delete_conversation(&conversation_id)
            .expect("delete conversation");
        assert!(reopened.conversation(&conversation_id).is_err());
        let _ = std::fs::remove_file(path);
    }
}
