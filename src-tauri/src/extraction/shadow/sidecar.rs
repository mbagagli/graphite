#![cfg_attr(
    not(all(
        feature = "pdf-shadow-pilot",
        target_os = "linux",
        target_arch = "x86_64"
    )),
    allow(dead_code)
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::database::content_hash;
use crate::error::{AppError, AppResult};
use crate::extraction::model::{
    CanonicalBlockContent, CanonicalDocumentV1, ExtractionAttempt, ExtractionInputDescriptor,
    ExtractionObservations, ExtractionProviderDescriptor, ExtractionRequest, ExtractionSignal,
    ExtractionSupport, SignalAvailability,
};
use crate::extraction::projection::render_plain_text;
use crate::extraction::provider::DocumentExtractionProvider;
use crate::knowledge::source::ProviderFuture;

use super::model::{ShadowCancellationToken, ShadowRuntimeMetrics};

const SIDECAR_PROTOCOL_VERSION: u32 = 1;
const MAX_SIDECAR_OUTPUT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SIDECAR_ERROR_BYTES: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowProviderRole {
    NativePrimary,
    OcrFallback,
    StructureDemand,
}

impl ShadowProviderRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NativePrimary => "native-primary",
            Self::OcrFallback => "ocr-fallback",
            Self::StructureDemand => "structure-demand",
        }
    }
}

#[derive(Clone)]
pub struct ShadowSidecarProvider {
    descriptor: ExtractionProviderDescriptor,
    expected_revision: &'static str,
    profile: &'static str,
    role: ShadowProviderRole,
    executable: Option<PathBuf>,
    asset_root: Option<PathBuf>,
    input_path: PathBuf,
    timeout: Duration,
    cancellation: ShadowCancellationToken,
}

impl ShadowSidecarProvider {
    pub fn xberg_native(
        executable: Option<PathBuf>,
        input_path: PathBuf,
        timeout: Duration,
        cancellation: ShadowCancellationToken,
    ) -> Self {
        Self {
            descriptor: descriptor(
                "xberg-native-shadow-v1",
                "xberg",
                "1.0.14",
                "xberg-native-to-canonical-v1",
                ShadowCapabilities {
                    ocr: false,
                    layout: true,
                    tables: true,
                    formulas: true,
                    figures: true,
                    pages: true,
                    boxes: true,
                },
            ),
            expected_revision: "b6660a759d31089075dd89112b77b918f872879a",
            profile: "native",
            role: ShadowProviderRole::NativePrimary,
            executable,
            asset_root: None,
            input_path,
            timeout,
            cancellation,
        }
    }

    pub fn pdf_inspector_ocr(
        executable: Option<PathBuf>,
        asset_root: Option<PathBuf>,
        input_path: PathBuf,
        timeout: Duration,
        cancellation: ShadowCancellationToken,
    ) -> Self {
        Self {
            descriptor: descriptor(
                "pdf-inspector-ocr-shadow-v1",
                "pdf-inspector",
                "1.15.0",
                "pdf-inspector-ocr-to-canonical-v1",
                ShadowCapabilities {
                    ocr: true,
                    layout: false,
                    tables: false,
                    formulas: false,
                    figures: false,
                    pages: true,
                    boxes: false,
                },
            ),
            expected_revision: "ca6d66714683f71d1b1cd447f766a2b287bea751",
            profile: "recommended-auto",
            role: ShadowProviderRole::OcrFallback,
            executable,
            asset_root,
            input_path,
            timeout,
            cancellation,
        }
    }

    pub fn docling_structure(
        executable: Option<PathBuf>,
        asset_root: Option<PathBuf>,
        input_path: PathBuf,
        timeout: Duration,
        cancellation: ShadowCancellationToken,
    ) -> Self {
        Self {
            descriptor: descriptor(
                "docling-structure-shadow-v1",
                "docling",
                "1.16.0",
                "docling-structure-to-canonical-v1",
                ShadowCapabilities {
                    ocr: true,
                    layout: true,
                    tables: true,
                    formulas: true,
                    figures: true,
                    pages: true,
                    boxes: true,
                },
            ),
            expected_revision: "7f8de88e860e289c42873922bd888cd50815ef26",
            profile: "recommended-auto",
            role: ShadowProviderRole::StructureDemand,
            executable,
            asset_root,
            input_path,
            timeout,
            cancellation,
        }
    }

    pub fn role(&self) -> ShadowProviderRole {
        self.role
    }

    fn availability(&self) -> Result<&Path, String> {
        if !cfg!(all(
            feature = "pdf-shadow-pilot",
            target_os = "linux",
            target_arch = "x86_64"
        )) {
            return Err("Milestone 6-A sidecars are unavailable on this build or platform".into());
        }
        let path = self
            .executable
            .as_deref()
            .ok_or_else(|| format!("{} shadow sidecar is not configured", self.role.as_str()))?;
        let metadata = path
            .metadata()
            .map_err(|_| format!("{} shadow sidecar is not prepared", self.role.as_str()))?;
        if !metadata.is_file() {
            return Err(format!(
                "{} shadow sidecar is not a regular file",
                self.role.as_str()
            ));
        }
        Ok(path)
    }

    fn terminal_attempt(
        &self,
        request: &ExtractionRequest,
        outcome: &str,
        reason: impl Into<String>,
    ) -> ExtractionAttempt {
        let mut observations = ExtractionObservations::unreported(
            &self.descriptor.profile_id,
            &request.artifact_id,
            request.artifact_revision_id,
            &request.input_content_hash,
        );
        observations.provider_diagnostics.insert(
            "shadowTerminalOutcome".to_owned(),
            Value::String(outcome.to_owned()),
        );
        ExtractionAttempt {
            provider: self.descriptor.clone(),
            canonical_document: None,
            provenance: BTreeMap::from([
                ("shadow".to_owned(), Value::Bool(true)),
                (
                    "providerRole".to_owned(),
                    Value::String(self.role.as_str().to_owned()),
                ),
            ]),
            observations,
            warnings: Vec::new(),
            error: Some(reason.into()),
        }
    }
}

impl DocumentExtractionProvider for ShadowSidecarProvider {
    fn descriptor(&self) -> ExtractionProviderDescriptor {
        let mut descriptor = self.descriptor.clone();
        descriptor.configured_available = self.availability().is_ok();
        descriptor
    }

    fn supports(&self, input: &ExtractionInputDescriptor) -> ExtractionSupport {
        if input.media_type != "application/pdf" || !input.has_local_file {
            return ExtractionSupport::Unsupported;
        }
        if self.availability().is_err() {
            return ExtractionSupport::Unavailable;
        }
        ExtractionSupport::Supported
    }

    fn extract<'a>(&'a self, request: ExtractionRequest) -> ProviderFuture<'a, ExtractionAttempt> {
        Box::pin(async move {
            let executable = match self.availability() {
                Ok(path) => path.to_owned(),
                Err(reason) => {
                    return Ok(self.terminal_attempt(&request, "unavailable", reason));
                }
            };
            if self.cancellation.is_cancelled() {
                return Ok(self.terminal_attempt(
                    &request,
                    "cancelled",
                    "shadow extraction was cancelled before the sidecar started",
                ));
            }
            let execution = execute_sidecar(
                SidecarExecutionRequest {
                    executable,
                    asset_root: self.asset_root.clone(),
                    input_path: self.input_path.clone(),
                    descriptor: self.descriptor.clone(),
                    expected_revision: self.expected_revision,
                    profile: self.profile,
                    request: request.clone(),
                },
                self.timeout,
                self.cancellation.clone(),
            )
            .await;
            match execution {
                Ok(execution) => map_envelope(
                    &request,
                    &self.descriptor,
                    self.role,
                    self.profile,
                    execution,
                ),
                Err(error) => Ok(self.terminal_attempt(&request, error.outcome, error.message)),
            }
        })
    }
}

fn descriptor(
    profile_id: &str,
    provider_id: &str,
    provider_version: &str,
    mapping_profile: &str,
    capabilities: ShadowCapabilities,
) -> ExtractionProviderDescriptor {
    ExtractionProviderDescriptor {
        profile_id: profile_id.to_owned(),
        provider_id: provider_id.to_owned(),
        provider_version: provider_version.to_owned(),
        mapping_profile: mapping_profile.to_owned(),
        supported_media_types: vec!["application/pdf".to_owned()],
        required_input_forms: vec!["validated-local-file".to_owned()],
        native_text_capability: true,
        layout_capability: capabilities.layout,
        table_capability: capabilities.tables,
        formula_capability: capabilities.formulas,
        figure_capability: capabilities.figures,
        ocr_capability: capabilities.ocr,
        page_provenance_capability: capabilities.pages,
        bounding_box_capability: capabilities.boxes,
        configured_available: true,
    }
}

#[derive(Clone, Copy)]
struct ShadowCapabilities {
    ocr: bool,
    layout: bool,
    tables: bool,
    formulas: bool,
    figures: bool,
    pages: bool,
    boxes: bool,
}

#[derive(Clone)]
struct SidecarExecutionRequest {
    executable: PathBuf,
    asset_root: Option<PathBuf>,
    input_path: PathBuf,
    descriptor: ExtractionProviderDescriptor,
    expected_revision: &'static str,
    profile: &'static str,
    request: ExtractionRequest,
}

#[derive(Debug)]
struct SidecarFailure {
    outcome: &'static str,
    message: String,
}

#[derive(Debug)]
struct SidecarExecution {
    envelope: SidecarEnvelope,
    metrics: ShadowRuntimeMetrics,
}

#[cfg(all(
    feature = "pdf-shadow-pilot",
    target_os = "linux",
    target_arch = "x86_64"
))]
async fn execute_sidecar(
    execution: SidecarExecutionRequest,
    timeout: Duration,
    cancellation: ShadowCancellationToken,
) -> Result<SidecarExecution, SidecarFailure> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::process::Command;

    let mut command = Command::new(&execution.executable);
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(asset_root) = &execution.asset_root {
        command.env("GRAPHITE_PDF_BENCH_ASSET_ROOT", asset_root);
    }
    let mut child = command.spawn().map_err(|_| SidecarFailure {
        outcome: "unavailable",
        message: format!(
            "{} shadow sidecar could not be started",
            execution.descriptor.provider_id
        ),
    })?;
    let process_id = child.id().unwrap_or_default();
    let mut stdin = child.stdin.take().ok_or_else(|| SidecarFailure {
        outcome: "crashed",
        message: "shadow sidecar did not expose standard input".to_owned(),
    })?;
    let stdout = child.stdout.take().ok_or_else(|| SidecarFailure {
        outcome: "crashed",
        message: "shadow sidecar did not expose standard output".to_owned(),
    })?;
    let stderr = child.stderr.take().ok_or_else(|| SidecarFailure {
        outcome: "crashed",
        message: "shadow sidecar did not expose standard error".to_owned(),
    })?;
    let request_id = format!(
        "shadow:{}:{}:{}",
        execution.request.artifact_id,
        execution.request.artifact_revision_id,
        execution.descriptor.profile_id
    );
    let protocol = [
        serde_json::json!({
            "action": "handshake",
            "protocol_version": SIDECAR_PROTOCOL_VERSION
        }),
        serde_json::json!({
            "action": "extract",
            "request": {
                "requestId": request_id,
                "documentId": execution.request.artifact_id,
                "artifactId": execution.request.artifact_id,
                "artifactRevisionId": execution.request.artifact_revision_id,
                "inputPath": execution.input_path,
                "inputSha256": execution.request.input_content_hash,
                "mediaType": execution.request.input.media_type,
                "profile": execution.profile,
                "forcedOcrPages": [],
                "zoteroBaselinePath": null
            }
        }),
        serde_json::json!({"action": "shutdown"}),
    ];
    for message in protocol {
        let mut bytes = serde_json::to_vec(&message).map_err(|error| SidecarFailure {
            outcome: "failed",
            message: format!("shadow protocol serialization failed: {error}"),
        })?;
        bytes.push(b'\n');
        stdin.write_all(&bytes).await.map_err(|_| SidecarFailure {
            outcome: "crashed",
            message: "shadow sidecar closed before accepting its request".to_owned(),
        })?;
    }
    drop(stdin);
    let started = std::time::Instant::now();
    let sampling = start_linux_sampler(process_id);
    let run = async {
        let mut output_bytes = Vec::new();
        let mut error_bytes = Vec::new();
        let mut bounded_output = stdout.take(MAX_SIDECAR_OUTPUT_BYTES + 1);
        let mut bounded_error = stderr.take(MAX_SIDECAR_ERROR_BYTES + 1);
        let read_output = bounded_output.read_to_end(&mut output_bytes);
        let read_error = bounded_error.read_to_end(&mut error_bytes);
        let (status, _, _) =
            tokio::try_join!(child.wait(), read_output, read_error).map_err(|_| {
                SidecarFailure {
                    outcome: "crashed",
                    message: "shadow sidecar process communication failed".to_owned(),
                }
            })?;
        if output_bytes.len() as u64 > MAX_SIDECAR_OUTPUT_BYTES {
            return Err(SidecarFailure {
                outcome: "failed",
                message: "shadow sidecar exceeded the bounded output limit".to_owned(),
            });
        }
        if !status.success() {
            return Err(SidecarFailure {
                outcome: "crashed",
                message: format!(
                    "{} shadow sidecar exited unsuccessfully",
                    execution.descriptor.provider_id
                ),
            });
        }
        parse_protocol_output(
            &output_bytes,
            &execution.descriptor,
            execution.expected_revision,
            execution.profile,
        )
    };
    tokio::pin!(run);
    let result = loop {
        tokio::select! {
            value = &mut run => break value,
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                if cancellation.is_cancelled() {
                    break Err(SidecarFailure {
                        outcome: "cancelled",
                        message: "shadow extraction was cancelled".to_owned(),
                    });
                }
                if started.elapsed() >= timeout {
                    break Err(SidecarFailure {
                        outcome: "timeout",
                        message: format!(
                            "{} shadow extraction reached its {} second ceiling",
                            execution.descriptor.provider_id,
                            timeout.as_secs()
                        ),
                    });
                }
            }
        }
    };
    let mut metrics = sampling.finish();
    metrics.duration_ms = Some(started.elapsed().as_millis() as u64);
    result.map(|envelope| SidecarExecution { envelope, metrics })
}

#[cfg(not(all(
    feature = "pdf-shadow-pilot",
    target_os = "linux",
    target_arch = "x86_64"
)))]
async fn execute_sidecar(
    _execution: SidecarExecutionRequest,
    _timeout: Duration,
    _cancellation: ShadowCancellationToken,
) -> Result<SidecarExecution, SidecarFailure> {
    Err(SidecarFailure {
        outcome: "unavailable",
        message: "Milestone 6-A sidecars are unavailable on this build or platform".to_owned(),
    })
}

fn parse_protocol_output(
    bytes: &[u8],
    descriptor: &ExtractionProviderDescriptor,
    expected_revision: &str,
    expected_profile: &str,
) -> Result<SidecarEnvelope, SidecarFailure> {
    let text = std::str::from_utf8(bytes).map_err(|_| SidecarFailure {
        outcome: "failed",
        message: "shadow sidecar returned non-UTF-8 protocol output".to_owned(),
    })?;
    let mut handshake = None;
    let mut extraction = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let response: SidecarResponse = serde_json::from_str(line).map_err(|_| SidecarFailure {
            outcome: "failed",
            message: "shadow sidecar returned invalid JSON-lines output".to_owned(),
        })?;
        match response {
            SidecarResponse::Handshake {
                protocol_version,
                candidate,
            } => {
                if protocol_version != SIDECAR_PROTOCOL_VERSION
                    || candidate.candidate_id != descriptor.provider_id
                    || candidate.version != descriptor.provider_version
                    || candidate.revision != expected_revision
                {
                    return Err(SidecarFailure {
                        outcome: "unavailable",
                        message: format!(
                            "{} shadow sidecar identity does not match the locked profile",
                            descriptor.provider_id
                        ),
                    });
                }
                if !candidate
                    .supported_profiles
                    .iter()
                    .any(|profile| profile == expected_profile)
                {
                    return Err(SidecarFailure {
                        outcome: "unavailable",
                        message: format!(
                            "{} shadow sidecar does not support profile {expected_profile}",
                            descriptor.provider_id
                        ),
                    });
                }
                if descriptor.provider_id == "xberg"
                    && candidate.features.iter().any(|feature| feature == "ocr")
                {
                    return Err(SidecarFailure {
                        outcome: "unavailable",
                        message: "Xberg OCR is excluded from the Milestone 6-A profile".to_owned(),
                    });
                }
                handshake = Some(candidate);
            }
            SidecarResponse::Extraction { result } => extraction = Some(*result),
            SidecarResponse::Error { _message: _ } => {
                return Err(SidecarFailure {
                    outcome: "failed",
                    message: "shadow sidecar reported a protocol error".to_owned(),
                });
            }
            SidecarResponse::Shutdown => {}
        }
    }
    if handshake.is_none() {
        return Err(SidecarFailure {
            outcome: "failed",
            message: "shadow sidecar returned no verified handshake".to_owned(),
        });
    }
    extraction.ok_or_else(|| SidecarFailure {
        outcome: "failed",
        message: "shadow sidecar returned no extraction envelope".to_owned(),
    })
}

fn map_envelope(
    request: &ExtractionRequest,
    descriptor: &ExtractionProviderDescriptor,
    role: ShadowProviderRole,
    expected_profile: &str,
    execution: SidecarExecution,
) -> AppResult<ExtractionAttempt> {
    let envelope = execution.envelope;
    if envelope.input_sha256 != request.input_content_hash
        || envelope.profile != expected_profile
        || envelope.candidate.candidate_id != descriptor.provider_id
        || envelope.candidate.version != descriptor.provider_version
    {
        return Err(AppError::ApplicationData(
            "shadow sidecar response did not match its request identity".to_owned(),
        ));
    }
    let terminal = envelope.outcome.as_str();
    let canonical = envelope.canonical_document;
    if let Some(document) = &canonical {
        if document.artifact_id != request.artifact_id
            || document.artifact_revision_id != request.artifact_revision_id
        {
            return Err(AppError::ApplicationData(
                "shadow canonical document did not preserve artifact identity".to_owned(),
            ));
        }
    }
    let mut observations = map_observations(
        request,
        descriptor,
        &envelope.observations,
        canonical.as_ref(),
        &envelope.mapping_losses,
    );
    observations.provider_diagnostics.insert(
        "shadowTerminalOutcome".to_owned(),
        Value::String(terminal.to_owned()),
    );
    if let Some(duration) = execution.metrics.duration_ms {
        observations
            .provider_diagnostics
            .insert("shadowDurationMs".to_owned(), Value::from(duration));
    }
    if let Some(rss) = execution.metrics.peak_rss_bytes {
        observations
            .provider_diagnostics
            .insert("shadowPeakRssBytes".to_owned(), Value::from(rss));
    }
    if let Some(threads) = execution.metrics.peak_thread_count {
        observations
            .provider_diagnostics
            .insert("shadowPeakThreadCount".to_owned(), Value::from(threads));
    }
    for (key, value) in envelope.diagnostics {
        if safe_diagnostic_key(&key) && safe_diagnostic_value(&value) {
            observations
                .provider_diagnostics
                .insert(format!("provider.{key}"), value);
        }
    }
    let succeeded = matches!(terminal, "succeeded" | "partial" | "suspicious");
    let error = (!succeeded).then(|| {
        format!(
            "{} shadow extraction outcome was {terminal}",
            descriptor.provider_id
        )
    });
    let mut warnings = envelope.warnings;
    warnings.extend(
        envelope
            .mapping_losses
            .iter()
            .map(|loss| format!("mapping loss {}: {}", loss.capability, loss.reason)),
    );
    Ok(ExtractionAttempt {
        provider: descriptor.clone(),
        canonical_document: canonical,
        provenance: BTreeMap::from([
            ("shadow".to_owned(), Value::Bool(true)),
            (
                "providerRole".to_owned(),
                Value::String(role.as_str().to_owned()),
            ),
            (
                "candidateRevision".to_owned(),
                Value::String(envelope.candidate.revision),
            ),
            (
                "adapterVersion".to_owned(),
                Value::String(envelope.candidate.adapter_version),
            ),
            (
                "profile".to_owned(),
                Value::String(expected_profile.to_owned()),
            ),
        ]),
        observations,
        warnings,
        error,
    })
}

fn map_observations(
    request: &ExtractionRequest,
    descriptor: &ExtractionProviderDescriptor,
    source: &SidecarObservations,
    canonical: Option<&CanonicalDocumentV1>,
    mapping_losses: &[SidecarMappingLoss],
) -> ExtractionObservations {
    let mut result = ExtractionObservations::unreported(
        &descriptor.profile_id,
        &request.artifact_id,
        request.artifact_revision_id,
        &request.input_content_hash,
    );
    result.reported_page_count = optional_signal(
        source.reported_page_count,
        &descriptor.profile_id,
        "provider did not report a source page count",
    );
    result.extracted_page_count = optional_signal(
        source.extracted_page_count,
        &descriptor.profile_id,
        "provider did not report extracted page count",
    );
    result.pages_with_text =
        ExtractionSignal::observed(source.pages_with_text.len() as u32, &descriptor.profile_id);
    result.empty_page_indices =
        ExtractionSignal::observed(source.empty_pages.clone(), &descriptor.profile_id);
    result.suspicious_page_indices =
        ExtractionSignal::observed(source.suspicious_pages.clone(), &descriptor.profile_id);
    result.native_text_presence = optional_signal(
        source.native_text_present,
        &descriptor.profile_id,
        "provider did not report native text presence",
    );
    result.ocr_used = optional_signal(
        source.ocr_used,
        &descriptor.profile_id,
        "provider did not report OCR use",
    );
    result.ocr_page_indices = if source.ocr_page_indices_reported == Some(true) {
        ExtractionSignal::observed(source.ocr_pages.clone(), &descriptor.profile_id)
    } else {
        ExtractionSignal::unavailable(
            SignalAvailability::NotReported,
            &descriptor.profile_id,
            "provider did not report OCR page identities",
        )
    };
    result.encoding_warnings =
        ExtractionSignal::observed(source.encoding_warnings.clone(), &descriptor.profile_id);
    result.layout_warnings =
        ExtractionSignal::observed(source.layout_warnings.clone(), &descriptor.profile_id);
    let mut retention_warnings = source.structure_retention_warnings.clone();
    retention_warnings.extend(
        mapping_losses
            .iter()
            .map(|loss| format!("{}: {}", loss.capability, loss.reason)),
    );
    result.structure_retention_warnings =
        ExtractionSignal::observed(retention_warnings, &descriptor.profile_id);
    result.characters_by_page =
        ExtractionSignal::observed(source.characters_by_page.clone(), &descriptor.profile_id);
    if let Some(document) = canonical {
        let plain_text = render_plain_text(document);
        result.plain_text_character_count =
            ExtractionSignal::observed(plain_text.chars().count() as u64, &descriptor.profile_id);
        result.plain_text_hash =
            ExtractionSignal::observed(content_hash(&plain_text), &descriptor.profile_id);
        let counts = block_counts(document);
        result.heading_count = ExtractionSignal::observed(counts[0], &descriptor.profile_id);
        result.table_count = ExtractionSignal::observed(counts[1], &descriptor.profile_id);
        result.formula_count = ExtractionSignal::observed(counts[2], &descriptor.profile_id);
        result.figure_count = ExtractionSignal::observed(counts[3], &descriptor.profile_id);
        result.caption_count = ExtractionSignal::observed(counts[4], &descriptor.profile_id);
        result.reference_count = ExtractionSignal::observed(counts[5], &descriptor.profile_id);
        result.replacement_character_count = ExtractionSignal::observed(
            plain_text.matches('\u{fffd}').count() as u64,
            &descriptor.profile_id,
        );
        if let Some(provider_chars) = source.provider_text_character_count {
            let ratio = if provider_chars == 0 {
                1.0
            } else {
                plain_text.chars().count() as f64 / provider_chars as f64
            };
            result.native_to_projection_retention_ratio =
                ExtractionSignal::observed(ratio, &descriptor.profile_id);
        }
    }
    result
}

fn optional_signal<T>(
    value: Option<T>,
    provenance: &str,
    explanation: &str,
) -> ExtractionSignal<T> {
    value.map_or_else(
        || ExtractionSignal::unavailable(SignalAvailability::NotReported, provenance, explanation),
        |value| ExtractionSignal::observed(value, provenance),
    )
}

fn block_counts(document: &CanonicalDocumentV1) -> [u32; 6] {
    let mut counts = [0_u32; 6];
    for block in &document.blocks {
        let index = match block.content {
            CanonicalBlockContent::Heading { .. } => Some(0),
            CanonicalBlockContent::Table { .. } => Some(1),
            CanonicalBlockContent::Formula { .. } => Some(2),
            CanonicalBlockContent::Figure { .. } => Some(3),
            CanonicalBlockContent::Caption { .. } => Some(4),
            CanonicalBlockContent::Reference { .. } => Some(5),
            _ => None,
        };
        if let Some(index) = index {
            counts[index] += 1;
        }
    }
    counts
}

fn safe_diagnostic_key(key: &str) -> bool {
    ![
        "path",
        "content",
        "text",
        "canonical",
        "markdown",
        "snippet",
    ]
    .iter()
    .any(|forbidden| key.to_ascii_lowercase().contains(forbidden))
}

fn safe_diagnostic_value(value: &Value) -> bool {
    matches!(value, Value::Null | Value::Bool(_) | Value::Number(_))
        || value
            .as_str()
            .is_some_and(|text| text.len() <= 128 && !text.contains('/') && !text.contains('\\'))
        || value.as_array().is_some_and(|values| {
            values.len() <= 256
                && values.iter().all(|item| {
                    matches!(item, Value::Bool(_) | Value::Number(_))
                        || item.as_str().is_some_and(|text| {
                            text.len() <= 64 && !text.contains('/') && !text.contains('\\')
                        })
                })
        })
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SidecarResponse {
    Handshake {
        protocol_version: u32,
        candidate: SidecarCandidate,
    },
    Extraction {
        result: Box<SidecarEnvelope>,
    },
    Error {
        #[serde(rename = "message")]
        _message: String,
    },
    Shutdown,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarCandidate {
    candidate_id: String,
    version: String,
    revision: String,
    adapter_version: String,
    features: Vec<String>,
    supported_profiles: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarEnvelope {
    input_sha256: String,
    candidate: SidecarCandidate,
    profile: String,
    outcome: SidecarOutcome,
    canonical_document: Option<CanonicalDocumentV1>,
    #[serde(default)]
    mapping_losses: Vec<SidecarMappingLoss>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    observations: SidecarObservations,
    #[serde(default)]
    diagnostics: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SidecarOutcome {
    Succeeded,
    Partial,
    Suspicious,
    EmptySuccess,
    Unsupported,
    Unavailable,
    Timeout,
    Crashed,
    Failed,
}

impl SidecarOutcome {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Partial => "partial",
            Self::Suspicious => "suspicious",
            Self::EmptySuccess => "empty-success",
            Self::Unsupported => "unsupported",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Crashed => "crashed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarMappingLoss {
    capability: String,
    reason: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SidecarObservations {
    reported_page_count: Option<u32>,
    extracted_page_count: Option<u32>,
    #[serde(default)]
    pages_with_text: Vec<u32>,
    #[serde(default)]
    empty_pages: Vec<u32>,
    #[serde(default)]
    suspicious_pages: Vec<u32>,
    native_text_present: Option<bool>,
    ocr_used: Option<bool>,
    ocr_page_indices_reported: Option<bool>,
    #[serde(default)]
    ocr_pages: Vec<u32>,
    #[serde(default)]
    encoding_warnings: Vec<String>,
    #[serde(default)]
    layout_warnings: Vec<String>,
    #[serde(default)]
    characters_by_page: Vec<u64>,
    provider_text_character_count: Option<u64>,
    #[serde(default)]
    structure_retention_warnings: Vec<String>,
}

#[cfg(target_os = "linux")]
struct LinuxSampler {
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<ShadowRuntimeMetrics>>,
}

#[cfg(target_os = "linux")]
fn start_linux_sampler(process_id: u32) -> LinuxSampler {
    use std::sync::atomic::Ordering;
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        let mut peak = ShadowRuntimeMetrics::default();
        while !worker_stop.load(Ordering::Acquire) {
            if let Some((rss, threads)) = linux_process_tree_metrics(process_id) {
                peak.peak_rss_bytes = Some(peak.peak_rss_bytes.unwrap_or_default().max(rss));
                peak.peak_thread_count =
                    Some(peak.peak_thread_count.unwrap_or_default().max(threads));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        peak
    });
    LinuxSampler {
        stop,
        handle: Some(handle),
    }
}

#[cfg(target_os = "linux")]
impl LinuxSampler {
    fn finish(mut self) -> ShadowRuntimeMetrics {
        use std::sync::atomic::Ordering;
        self.stop.store(true, Ordering::Release);
        self.handle
            .take()
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default()
    }
}

#[cfg(target_os = "linux")]
fn linux_process_tree_metrics(root_process_id: u32) -> Option<(u64, u64)> {
    use std::fs;
    let mut processes = BTreeMap::new();
    for entry in fs::read_dir("/proc").ok()? {
        let Ok(entry) = entry else { continue };
        let Some(process_id) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let fields = stat
            .get(stat.rfind(')')? + 2..)?
            .split_whitespace()
            .collect::<Vec<_>>();
        let Some(parent_id) = fields.get(1).and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let Ok(status) = fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let rss = linux_status_value(&status, "VmRSS:").unwrap_or_default() * 1024;
        let threads = linux_status_value(&status, "Threads:").unwrap_or(1);
        processes.insert(process_id, (parent_id, rss, threads));
    }
    if !processes.contains_key(&root_process_id) {
        return None;
    }
    let mut selected = BTreeSet::from([root_process_id]);
    loop {
        let before = selected.len();
        for (process_id, (parent_id, ..)) in &processes {
            if selected.contains(parent_id) {
                selected.insert(*process_id);
            }
        }
        if selected.len() == before {
            break;
        }
    }
    let (rss, threads) = selected.into_iter().fold((0_u64, 0_u64), |sum, id| {
        let (_, rss, threads) = processes[&id];
        (sum.0.saturating_add(rss), sum.1.saturating_add(threads))
    });
    Some((rss, threads))
}

#[cfg(target_os = "linux")]
fn linux_status_value(value: &str, field: &str) -> Option<u64> {
    value.lines().find_map(|line| {
        line.strip_prefix(field)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

#[cfg(not(target_os = "linux"))]
struct LinuxSampler;

#[cfg(not(target_os = "linux"))]
fn start_linux_sampler(_process_id: u32) -> LinuxSampler {
    LinuxSampler
}

#[cfg(not(target_os = "linux"))]
impl LinuxSampler {
    fn finish(self) -> ShadowRuntimeMetrics {
        ShadowRuntimeMetrics::default()
    }
}
