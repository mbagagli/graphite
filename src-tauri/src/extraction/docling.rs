//! Run explicitly requested local Docling extraction and retain alternatives.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::model::{
    CanonicalBlock, CanonicalBlockContent, CanonicalDocumentMetadata, CanonicalDocumentV1,
    CanonicalPage, EvaluatedExtractionAttempt, ExtractionAttempt, ExtractionAttemptOutcome,
    ExtractionInputDescriptor, ExtractionObservations, ExtractionProviderDescriptor,
    ExtractionRequest, ExtractionRoutingProfile, ExtractionSelectionDecision, ExtractionSignal,
    PageRange, RoutedExtraction, CANONICAL_SCHEMA_VERSION,
};
use super::projection::{render_plain_text, validate_canonical_document};
use super::quality::ExtractionQualityEvaluator;
use super::store::ExtractionStore;
use crate::database::ResearchDatabase;
use crate::error::{AppError, AppResult};

const PROFILE: &str = "docling-local-2.142.1-v2";
const TIMEOUT: Duration = Duration::from_secs(960);
const OUTPUT_LIMIT: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoclingJob {
    pub document_id: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionChoice {
    pub id: String,
    pub provider: String,
    pub created_at: String,
    pub active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoclingStatus {
    pub available: bool,
    pub message: String,
    pub job: DoclingJob,
    pub choices: Vec<ExtractionChoice>,
}

#[derive(Clone)]
pub struct DoclingCoordinator {
    database: ResearchDatabase,
    runtime: Option<PathBuf>,
    cache: PathBuf,
    job: Arc<Mutex<DoclingJob>>,
    operation: Arc<Mutex<()>>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerResult {
    schema_version: u32,
    input_sha256: String,
    page_count: u32,
    blocks: Vec<WorkerBlock>,
}

#[derive(Deserialize)]
struct WorkerBlock {
    id: String,
    text: String,
    label: String,
    pages: Vec<u32>,
}

struct WorkerChild(std::process::Child);

impl Drop for WorkerChild {
    fn drop(&mut self) {
        // Reap the worker on every error path as well as cancellation.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn failure(message: impl Into<String>) -> AppError {
    AppError::Extraction(message.into())
}

fn io(error: std::io::Error) -> AppError {
    failure(format!("Local extraction storage failed: {error}"))
}

impl DoclingCoordinator {
    pub fn new(database: ResearchDatabase, cache: PathBuf) -> AppResult<Self> {
        std::fs::create_dir_all(&cache).map_err(io)?;
        let mut job: DoclingJob = match std::fs::read(cache.join("job.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| DoclingJob {
                status: "failed".into(),
                message:
                    "Previous extraction status could not be read. Saved text remains available."
                        .into(),
                ..Default::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => DoclingJob::default(),
            Err(error) => return Err(io(error)),
        };
        if job.status == "running" {
            job.status = "interrupted".into();
            job.message =
                "Extraction was interrupted. The saved text remains available; retry when ready."
                    .into();
        }
        Ok(Self {
            database,
            runtime: std::env::var_os("GRAPHITE_DOCLING_RUNTIME").map(PathBuf::from),
            cache,
            job: Arc::new(Mutex::new(job)),
            operation: Arc::new(Mutex::new(())),
            cancelled: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn status(&self, document_id: &str) -> AppResult<DoclingStatus> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT e.id, e.provider_profile_id, e.created_at,
                    EXISTS(SELECT 1 FROM active_extraction_selections s WHERE s.extraction_id=e.id)
             FROM document_extractions e WHERE e.artifact_id=?1
             AND e.canonical_json IS NOT NULL AND e.attempt_outcome='succeeded'
             ORDER BY e.created_at DESC, e.id",
        )?;
        let choices = statement
            .query_map([document_id], |row| {
                Ok(ExtractionChoice {
                    id: row.get(0)?,
                    provider: row.get(1)?,
                    created_at: row.get(2)?,
                    active: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let available = cfg!(target_os = "linux")
            && self.runtime.as_ref().is_some_and(|root| {
                root.join("envs/graphite-rag-comparison/bin/python")
                    .is_file()
                    && root.join("models").is_dir()
            });
        Ok(DoclingStatus {
            available,
            message: if available { "Optional local Docling is configured. No downloads or uploads." }
                else { "Docling is not configured. Set GRAPHITE_DOCLING_RUNTIME to the existing isolated runtime and restart Graphite." }.into(),
            job: self.job.lock().map_err(|_| failure("Extraction state is unavailable"))?.clone(),
            choices,
        })
    }

    fn update(&self, document: &str, status: &str, message: &str) -> AppResult<()> {
        let job = DoclingJob {
            document_id: document.into(),
            status: status.into(),
            message: message.into(),
        };
        std::fs::write(self.cache.join("job.tmp"), serde_json::to_vec(&job)?).map_err(io)?;
        std::fs::rename(self.cache.join("job.tmp"), self.cache.join("job.json")).map_err(io)?;
        *self
            .job
            .lock()
            .map_err(|_| failure("Extraction state is unavailable"))? = job;
        Ok(())
    }

    pub fn cancel(&self, document_id: &str) -> AppResult<()> {
        let job = self
            .job
            .lock()
            .map_err(|_| failure("Extraction state is unavailable"))?;
        if job.document_id == document_id && job.status == "running" {
            self.cancelled.store(true, Ordering::Release);
        }
        Ok(())
    }

    pub fn extract(&self, document_id: &str) -> AppResult<()> {
        let _guard = self
            .operation
            .try_lock()
            .map_err(|_| failure("Another extraction is running"))?;
        if !self.status(document_id)?.available {
            return Err(failure(
                "Configure the existing local Docling runtime first",
            ));
        }
        self.cancelled.store(false, Ordering::Release);
        self.update(
            document_id,
            "running",
            "Verifying local models and extracting this PDF. Existing text remains available.",
        )?;
        let result = self.convert(document_id);
        match &result {
            Ok(()) => self.update(
                document_id,
                "complete",
                "Docling text is active. Update the semantic index to use its passages in answers.",
            )?,
            Err(error) => self.update(
                document_id,
                if self.cancelled.load(Ordering::Acquire) {
                    "cancelled"
                } else {
                    "failed"
                },
                &error.to_string(),
            )?,
        }
        result
    }

    fn convert(&self, document_id: &str) -> AppResult<()> {
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| failure("Docling is not configured"))?;
        let context = self.database.document_resolution_context(document_id)?;
        let path = context
            .canonical_path
            .ok_or_else(|| failure("Open this PDF in the reader before extracting it"))?;
        let (hash, revision): (String, i64) = self.database.connect()?.query_row(
            "SELECT r.content_hash, r.id FROM document_revisions r
             JOIN document_locators l ON l.document_id=r.document_id AND l.revision_key=r.revision_key
             WHERE r.document_id=?1 ORDER BY r.id DESC LIMIT 1", [document_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let run = self.cache.join(format!(
            "run-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir(&run).map_err(io)?;
        for (name, data) in [
            (
                "visible_docling.py",
                include_str!("../../../tools/rag-comparison/visible_docling.py"),
            ),
            (
                "worker.py",
                include_str!("../../../tools/rag-comparison/docling_worker.py"),
            ),
            (
                "offline_exec.py",
                include_str!("../../../tools/rag-comparison/offline_exec.py"),
            ),
            (
                "models.json",
                include_str!("../../../tools/rag-comparison/models-docling.lock.json"),
            ),
        ] {
            std::fs::write(run.join(name), data).map_err(io)?;
        }
        let output = run.join("output.json");
        let errors = run.join("worker.log");
        let mut child = WorkerChild(
            Command::new(runtime.join("envs/graphite-rag-comparison/bin/python"))
                .arg(run.join("worker.py"))
                .arg("--input")
                .arg(&path)
                .arg("--sha256")
                .arg(&hash)
                .arg("--runtime")
                .arg(runtime)
                .arg("--lock")
                .arg(run.join("models.json"))
                .stdin(Stdio::null())
                .stdout(std::fs::File::create(&output).map_err(io)?)
                .stderr(std::fs::File::create(&errors).map_err(io)?)
                .spawn()
                .map_err(io)?,
        );
        let started = Instant::now();
        loop {
            let limit = std::fs::metadata(&output).map_err(io)?.len() > OUTPUT_LIMIT
                || std::fs::metadata(&errors).map_err(io)?.len() > 2 * 1024 * 1024;
            if self.cancelled.load(Ordering::Acquire) || started.elapsed() > TIMEOUT || limit {
                let _ = child.0.kill();
                let _ = child.0.wait();
                return Err(failure(if self.cancelled.load(Ordering::Acquire) {
                    "Extraction cancelled. Saved text was retained."
                } else {
                    "Extraction exceeded its time/output limit. Saved text was retained."
                }));
            }
            if let Some(status) = child.0.try_wait().map_err(io)? {
                if !status.success() {
                    return Err(failure("Local Docling conversion failed. Saved text was retained; check the local extraction log."));
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if std::fs::metadata(&output).map_err(io)?.len() > OUTPUT_LIMIT {
            return Err(failure("Extraction output exceeded its size limit"));
        }
        let result: WorkerResult = serde_json::from_slice(&std::fs::read(output).map_err(io)?)?;
        let canonical = canonical_document(result, document_id, revision, &hash)?;
        if self.cancelled.load(Ordering::Acquire) {
            return Err(failure(
                "Extraction cancelled before activation. Saved text was retained.",
            ));
        }
        // Resolve the locator again so a changed PDF cannot activate stale text.
        let current: String = self.database.connect()?.query_row(
            "SELECT revision_key FROM document_locators WHERE document_id=?1",
            [document_id],
            |row| row.get(0),
        )?;
        if Some(current) != context.revision_key {
            return Err(failure("PDF revision changed; retry extraction"));
        }
        let mut observations =
            ExtractionObservations::unreported(PROFILE, document_id, revision, &hash);
        observations.extracted_page_count =
            ExtractionSignal::observed(canonical.pages.len() as u32, PROFILE);
        observations.plain_text_character_count = ExtractionSignal::observed(
            render_plain_text(&canonical).chars().count() as u64,
            PROFILE,
        );
        let attempt = ExtractionAttempt {
            provider: descriptor(),
            canonical_document: Some(canonical),
            provenance: BTreeMap::from([(
                "networkPolicy".into(),
                serde_json::json!("linux-seccomp-network-denied"),
            )]),
            observations,
            warnings: vec![
                "Automatic OCR can misread mixed-column layouts. Verify important evidence.".into(),
            ],
            error: None,
        };
        persist_attempt(&self.database, attempt)
    }

    pub fn select(&self, document_id: &str, extraction_id: &str) -> AppResult<()> {
        let _guard = self
            .operation
            .try_lock()
            .map_err(|_| failure("Wait for extraction to finish"))?;
        let connection = self.database.connect()?;
        let (canonical, observations, capabilities): (String, String, String) = connection.query_row(
            "SELECT e.canonical_json, e.observations_json, p.capabilities_json
             FROM document_extractions e JOIN extraction_provider_profiles p ON p.id=e.provider_profile_id
             WHERE e.id=?1 AND e.artifact_id=?2 AND e.attempt_outcome='succeeded'",
            params![extraction_id, document_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let document: CanonicalDocumentV1 = serde_json::from_str(&canonical)?;
        if document.extraction_profile_id.starts_with("docling-local-") {
            let current_revision: i64 = connection.query_row(
                "SELECT r.id FROM document_locators l JOIN document_revisions r
                 ON r.document_id=l.document_id AND r.revision_key=l.revision_key
                 WHERE l.document_id=?1",
                [document_id],
                |row| row.get(0),
            )?;
            if current_revision != document.artifact_revision_id {
                return Err(failure(
                    "This saved extraction belongs to an older PDF. Extract the current PDF first.",
                ));
            }
        }
        persist_attempt(
            &self.database,
            ExtractionAttempt {
                provider: serde_json::from_str(&capabilities)?,
                canonical_document: Some(document),
                observations: serde_json::from_str(&observations)?,
                provenance: BTreeMap::new(),
                warnings: Vec::new(),
                error: None,
            },
        )?;
        self.update(
            document_id,
            "complete",
            "Saved extraction selected. Update the semantic index before asking questions.",
        )
    }
}

fn descriptor() -> ExtractionProviderDescriptor {
    ExtractionProviderDescriptor {
        profile_id: PROFILE.into(),
        provider_id: "docling-local".into(),
        provider_version: "2.142.1".into(),
        mapping_profile: "visible-text-and-markdown-tables-v2".into(),
        supported_media_types: vec!["application/pdf".into()],
        required_input_forms: vec!["local-file".into()],
        native_text_capability: true,
        layout_capability: true,
        table_capability: true,
        formula_capability: false,
        figure_capability: false,
        ocr_capability: true,
        page_provenance_capability: true,
        bounding_box_capability: false,
        configured_available: true,
    }
}

fn canonical_document(
    result: WorkerResult,
    id: &str,
    revision: i64,
    hash: &str,
) -> AppResult<CanonicalDocumentV1> {
    if result.schema_version != 1
        || result.input_sha256 != hash
        || !(1..=200).contains(&result.page_count)
    {
        return Err(failure("Docling output identity or page count is invalid"));
    }
    let mut blocks = Vec::new();
    for (index, block) in result.blocks.into_iter().enumerate() {
        if block
            .pages
            .iter()
            .any(|p| *p == 0 || *p > result.page_count)
        {
            return Err(failure("Docling returned a page outside this PDF"));
        }
        let page_range =
            block
                .pages
                .iter()
                .min()
                .zip(block.pages.iter().max())
                .map(|(low, high)| PageRange {
                    start: *low,
                    end: *high,
                });
        blocks.push(CanonicalBlock {
            id: block.id,
            parent_id: None,
            source_order: index as u32,
            section_path: Vec::new(),
            page_range,
            regions: Vec::new(),
            content: CanonicalBlockContent::Other {
                label: block.label,
                text: block.text,
            },
        });
    }
    let document = CanonicalDocumentV1 {
        schema_version: CANONICAL_SCHEMA_VERSION.into(),
        artifact_id: id.into(),
        artifact_revision_id: revision,
        extraction_profile_id: PROFILE.into(),
        metadata: CanonicalDocumentMetadata {
            reported_page_count_hint: Some(result.page_count),
            ..Default::default()
        },
        pages: (1..=result.page_count)
            .map(|p| CanonicalPage {
                id: format!("page-{p}"),
                physical_page_number: p,
                printed_label: None,
                width: None,
                height: None,
                rotation_degrees: None,
                coordinate_space: None,
            })
            .collect(),
        reading_order: blocks.iter().map(|b| b.id.clone()).collect(),
        blocks,
        relationships: Vec::new(),
        assets: Vec::new(),
    };
    validate_canonical_document(&document)?;
    if render_plain_text(&document).trim().is_empty() {
        return Err(failure("Docling returned no usable text"));
    }
    Ok(document)
}

fn persist_attempt(database: &ResearchDatabase, attempt: ExtractionAttempt) -> AppResult<()> {
    let request = ExtractionRequest {
        artifact_id: attempt.observations.artifact_id.clone(),
        artifact_revision_id: attempt.observations.artifact_revision_id,
        input_content_hash: attempt.observations.input_content_hash.clone(),
        input: ExtractionInputDescriptor {
            media_type: "application/pdf".into(),
            has_local_file: true,
            has_bounded_reader: false,
            has_source_provided_text: false,
        },
        source_text: None,
    };
    let quality = ExtractionQualityEvaluator::default_v1();
    let report = quality.evaluate(&attempt);
    if !report.usable {
        return Err(failure("Extraction did not pass the usable-text check"));
    }
    let routing = ExtractionRoutingProfile {
        id: "manual-extraction-v1".into(),
        version: "1".into(),
        provider_order: vec![attempt.provider.profile_id.clone()],
        compare_degraded: false,
        allow_usable_degraded: true,
    };
    ExtractionStore::new(database.clone()).persist_routed_extraction(
        &request,
        &routing,
        &quality,
        &RoutedExtraction {
            attempts: vec![EvaluatedExtractionAttempt {
                attempt,
                quality: report,
                outcome: ExtractionAttemptOutcome::Succeeded,
            }],
            historical_active: None,
            selection: ExtractionSelectionDecision::NewAttempt(0),
            selection_reason: "Explicit reader extraction choice".into(),
            fallback_used: false,
            fallback_reason: None,
        },
    )
}

#[cfg(test)]
mod tests;
