use std::fmt::Write;
use std::fs::File;
use std::io::Read;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use encoding_rs::UTF_8;
use hf_hub::api::sync::ApiBuilder;
use hf_hub::api::Progress;
use hf_hub::{Repo, RepoType};
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

use super::hardware::{gpu_devices, preferred_gpu, GpuOptions, RuntimeHardware};
use super::model::{
    ComputeDevice, ContextWindow, GenerationRequest, GenerationResult, InferenceBackend,
    ModelInstallEvent, ModelProfile,
};
use super::provider::{EvidenceTokenCounter, LocalModelProvider};

pub const RUNTIME_VERSION: &str = "llama.cpp via llama-cpp-2 0.1.154";

#[derive(Clone, Copy, Debug)]
pub struct ProfileSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub repo_id: &'static str,
    pub revision: &'static str,
    pub filename: &'static str,
    pub sha256: &'static str,
    pub expected_bytes: u64,
    pub context_capacity: u32,
}

pub const PROFILE_SPECS: [ProfileSpec; 2] = [
    ProfileSpec {
        id: "lightweight",
        label: "Lightweight - Qwen3 4B Q4_K_M",
        repo_id: "Qwen/Qwen3-4B-GGUF",
        revision: "bc640142c66e1fdd12af0bd68f40445458f3869b",
        filename: "Qwen3-4B-Q4_K_M.gguf",
        sha256: "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5",
        expected_bytes: 2_497_280_256,
        context_capacity: 8_192,
    },
    ProfileSpec {
        id: "balanced",
        label: "Balanced - Qwen3 8B Q4_K_M",
        repo_id: "Qwen/Qwen3-8B-GGUF",
        revision: "7c41481f57cb95916b40956ab2f0b139b296d974",
        filename: "Qwen3-8B-Q4_K_M.gguf",
        sha256: "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785",
        expected_bytes: 5_027_783_488,
        context_capacity: 16_384,
    },
];

pub fn profile_spec(profile_id: &str) -> AppResult<ProfileSpec> {
    PROFILE_SPECS
        .iter()
        .copied()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| {
            AppError::LocalModel(format!(
                "Unknown model profile '{profile_id}'. Choose lightweight or balanced."
            ))
        })
}

pub fn profile_summary(
    profile: ProfileSpec,
    installation: Option<(String, u64, Option<String>)>,
) -> ModelProfile {
    let (status, installed_bytes, last_error) =
        installation.unwrap_or_else(|| ("not-installed".to_owned(), 0, None));
    ModelProfile {
        id: profile.id.to_owned(),
        label: profile.label.to_owned(),
        model_identifier: profile.repo_id.to_owned(),
        model_revision: profile.revision.to_owned(),
        artifact_filename: profile.filename.to_owned(),
        artifact_identity: format!(
            "hf://{}/{}#{}",
            profile.repo_id, profile.filename, profile.revision
        ),
        quantization: "Q4_K_M".to_owned(),
        expected_bytes: profile.expected_bytes,
        sha256: profile.sha256.to_owned(),
        context_capacity: profile.context_capacity,
        license: "Apache-2.0".to_owned(),
        installed: status == "ready",
        status,
        installed_bytes,
        last_error,
    }
}

pub struct InstalledArtifact {
    pub profile: ProfileSpec,
    pub cache_relative_path: String,
    pub installed_bytes: u64,
}

pub fn install_profile(
    cache_root: &Path,
    profile_id: &str,
    on_event: Arc<dyn Fn(ModelInstallEvent) + Send + Sync>,
) -> AppResult<InstalledArtifact> {
    let profile = profile_spec(profile_id)?;
    std::fs::create_dir_all(cache_root)
        .map_err(|error| AppError::ApplicationData(error.to_string()))?;
    on_event(ModelInstallEvent {
        kind: "stage".to_owned(),
        profile_id: profile.id.to_owned(),
        message: format!(
            "Downloading {} into Graphite's private model cache",
            profile.label
        ),
    });
    let api = ApiBuilder::new()
        .with_cache_dir(cache_root.to_path_buf())
        .with_progress(false)
        .with_token(None)
        .build()
        .map_err(|error| AppError::LocalModel(format!("Model download setup failed: {error}")))?;
    let repo = Repo::with_revision(
        profile.repo_id.to_owned(),
        RepoType::Model,
        profile.revision.to_owned(),
    );
    let path = api
        .repo(repo)
        .download_with_progress(
            profile.filename,
            InstallProgress::new(profile.id, on_event.clone()),
        )
        .map_err(|error| AppError::LocalModel(format!("Model download failed: {error}")))?;
    on_event(ModelInstallEvent {
        kind: "verify".to_owned(),
        profile_id: profile.id.to_owned(),
        message: "Verifying GGUF signature, byte size, and SHA-256".to_owned(),
    });
    let installed_bytes = verify_artifact(&path, profile)?;
    let canonical_root = cache_root
        .canonicalize()
        .map_err(|error| AppError::ApplicationData(error.to_string()))?;
    let canonical_path = path
        .canonicalize()
        .map_err(|error| AppError::ApplicationData(error.to_string()))?;
    if !canonical_path.starts_with(&canonical_root) {
        return Err(AppError::LocalModel(
            "The downloaded model resolved outside Graphite's model cache.".to_owned(),
        ));
    }
    let relative = path
        .strip_prefix(cache_root)
        .map_err(|_| {
            AppError::LocalModel("The installed model path was not cache-relative.".to_owned())
        })?
        .to_string_lossy()
        .into_owned();
    on_event(ModelInstallEvent {
        kind: "complete".to_owned(),
        profile_id: profile.id.to_owned(),
        message: format!("{} is verified and ready", profile.label),
    });
    Ok(InstalledArtifact {
        profile,
        cache_relative_path: relative,
        installed_bytes,
    })
}

fn verify_artifact(path: &Path, profile: ProfileSpec) -> AppResult<u64> {
    let mut file = File::open(path).map_err(|error| {
        AppError::LocalModel(format!("The downloaded model cannot be opened: {error}"))
    })?;
    let metadata = file
        .metadata()
        .map_err(|error| AppError::LocalModel(format!("Model metadata failed: {error}")))?;
    if metadata.len() != profile.expected_bytes {
        return Err(AppError::LocalModel(format!(
            "The model size is {}, expected {} bytes. Installation was not accepted.",
            metadata.len(),
            profile.expected_bytes
        )));
    }
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic)
        .map_err(|error| AppError::LocalModel(format!("GGUF header read failed: {error}")))?;
    if &magic != b"GGUF" {
        return Err(AppError::LocalModel(
            "The downloaded artifact is not a GGUF model.".to_owned(),
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(magic);
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| AppError::LocalModel(format!("Model hashing failed: {error}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut digest = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut digest, "{byte:02x}").expect("write model checksum");
    }
    if digest != profile.sha256 {
        return Err(AppError::LocalModel(
            "The model checksum does not match the curated manifest.".to_owned(),
        ));
    }
    Ok(metadata.len())
}

struct InstallProgress {
    profile_id: String,
    current: u64,
    total: u64,
    last_reported_percent: u64,
    on_event: Arc<dyn Fn(ModelInstallEvent) + Send + Sync>,
}

impl InstallProgress {
    fn new(profile_id: &str, on_event: Arc<dyn Fn(ModelInstallEvent) + Send + Sync>) -> Self {
        Self {
            profile_id: profile_id.to_owned(),
            current: 0,
            total: 0,
            last_reported_percent: 0,
            on_event,
        }
    }
}

impl Progress for InstallProgress {
    fn init(&mut self, size: usize, _filename: &str) {
        self.total = size as u64;
    }

    fn update(&mut self, size: usize) {
        self.current = self.current.saturating_add(size as u64);
        let percent = self
            .current
            .saturating_mul(100)
            .checked_div(self.total)
            .unwrap_or(0);
        if percent >= self.last_reported_percent.saturating_add(2) {
            self.last_reported_percent = percent;
            (self.on_event)(ModelInstallEvent {
                kind: "progress".to_owned(),
                profile_id: self.profile_id.clone(),
                message: format!("{percent}% downloaded"),
            });
        }
    }

    fn finish(&mut self) {
        (self.on_event)(ModelInstallEvent {
            kind: "progress".to_owned(),
            profile_id: self.profile_id.clone(),
            message: "Download complete".to_owned(),
        });
    }
}

struct LoadedModel {
    profile_id: String,
    model: LlamaModel,
}

struct ManagedRuntimeState {
    backend: LlamaBackend,
    loaded: Option<LoadedModel>,
    selected_device: Option<ComputeDevice>,
    inference_completed: bool,
    cpu_fallback_reason: Option<String>,
    inference_backend: InferenceBackend,
    gpu_options: Result<GpuOptions, String>,
    #[cfg(test)]
    fail_gpu_stage: Option<&'static str>,
}

#[derive(Clone)]
pub struct ManagedRuntime {
    state: Arc<Mutex<ManagedRuntimeState>>,
}

impl ManagedRuntime {
    pub fn initialize() -> AppResult<Self> {
        let mut backend = LlamaBackend::init().map_err(|error| {
            AppError::LocalModel(format!("The llama.cpp backend did not initialize: {error}"))
        })?;
        backend.void_logs();
        Ok(Self {
            state: Arc::new(Mutex::new(ManagedRuntimeState {
                backend,
                loaded: None,
                selected_device: None,
                inference_completed: false,
                cpu_fallback_reason: None,
                inference_backend: InferenceBackend::Auto,
                gpu_options: GpuOptions::from_env(),
                #[cfg(test)]
                fail_gpu_stage: None,
            })),
        })
    }

    /// Report initialized devices and the outcome of actual managed inference.
    pub(super) fn hardware_status(&self) -> AppResult<RuntimeHardware> {
        let state = self
            .state
            .lock()
            .map_err(|_| AppError::LocalModel("The model runtime lock is poisoned.".to_owned()))?;
        Ok(RuntimeHardware::describe(
            gpu_devices(),
            state.selected_device.as_ref(),
            state.inference_completed,
            state.cpu_fallback_reason.as_deref(),
            state.inference_backend,
            state.gpu_options.as_ref().copied().map_err(String::as_str),
        ))
    }

    pub fn unload(&self) -> AppResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| AppError::LocalModel("The model runtime lock is poisoned.".to_owned()))?;
        state.loaded = None;
        state.selected_device = None;
        state.inference_completed = false;
        state.cpu_fallback_reason = None;
        Ok(())
    }
}

pub struct GraphiteManagedProvider {
    runtime: ManagedRuntime,
    profile: ProfileSpec,
    model_path: PathBuf,
    inference_backend: InferenceBackend,
    context_capacity: u32,
}

impl GraphiteManagedProvider {
    pub fn new(runtime: ManagedRuntime, profile_id: &str, model_path: PathBuf) -> AppResult<Self> {
        Ok(Self {
            runtime,
            profile: profile_spec(profile_id)?,
            model_path,
            inference_backend: InferenceBackend::Auto,
            context_capacity: profile_spec(profile_id)?.context_capacity,
        })
    }

    /// Apply the request snapshot without changing a running generation.
    pub fn with_preferences(mut self, backend: InferenceBackend, window: ContextWindow) -> Self {
        self.inference_backend = backend;
        self.context_capacity = window.tokens(self.profile.context_capacity);
        self
    }

    fn ensure_loaded(&self, state: &mut ManagedRuntimeState) -> AppResult<()> {
        if state.inference_backend != self.inference_backend {
            state.loaded = None;
            state.selected_device = None;
            state.cpu_fallback_reason = None;
            state.inference_completed = false;
            state.inference_backend = self.inference_backend;
        }
        let needs_load = state
            .loaded
            .as_ref()
            .is_none_or(|loaded| loaded.profile_id != self.profile.id);
        if needs_load {
            state.loaded = None;
            state.selected_device = None;
            state.inference_completed = false;
            let options = if self.inference_backend == InferenceBackend::Cpu {
                GpuOptions::default()
            } else {
                state.gpu_options.clone().map_err(AppError::LocalModel)?
            };
            let allow_fallback = options.allows_cpu_fallback(self.inference_backend);
            if self.inference_backend != InferenceBackend::Cpu
                && state.cpu_fallback_reason.is_none()
            {
                let devices = gpu_devices();
                state.selected_device = preferred_gpu(
                    &devices,
                    self.profile.expected_bytes,
                    self.inference_backend,
                    options,
                )?;
                if allow_fallback && !devices.is_empty() && state.selected_device.is_none() {
                    state.cpu_fallback_reason = Some(
                        "Insufficient free GPU memory for the configured offload and load reserve."
                            .into(),
                    );
                }
            }
            let attempt = self.load_model(state, options);
            let model = match attempt {
                Ok(model) => model,
                Err(gpu_error) if state.selected_device.is_some() && allow_fallback => {
                    state.selected_device = None;
                    state.cpu_fallback_reason = Some("GPU model loading failed.".into());
                    self.load_model(state, options).map_err(|cpu_error| AppError::LocalModel(
                        format!("GPU model loading failed ({gpu_error}); CPU fallback also failed ({cpu_error}).")
                    ))?
                }
                Err(error) => {
                    state.selected_device = None;
                    return Err(error);
                }
            };
            state.loaded = Some(LoadedModel {
                profile_id: self.profile.id.to_owned(),
                model,
            });
        }
        Ok(())
    }

    /// Attempt inference without retrying or duplicating already streamed text.
    fn generate_once(
        &self,
        state: &mut ManagedRuntimeState,
        request: &GenerationRequest,
        cancellation: Arc<AtomicBool>,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<GenerationResult, InferenceFailure> {
        let generation_started = Instant::now();
        let use_gpu = state.selected_device.is_some();
        #[cfg(test)]
        if use_gpu && state.fail_gpu_stage == Some("context") {
            state.fail_gpu_stage = None;
            return Err(InferenceFailure::runtime(
                "Injected GPU context failure".into(),
            ));
        }
        let ManagedRuntimeState {
            backend, loaded, ..
        } = &mut *state;
        let model = &mut loaded.as_mut().expect("model was loaded").model;
        let prompt = format_chat_prompt(&request.system_prompt, &request.user_prompt);
        let tokens = model
            .str_to_token(&prompt, AddBos::Never)
            .map_err(|error| {
                AppError::LocalModel(format!("Prompt tokenization failed: {error}"))
            })?;
        let context_capacity = request.context_tokens.min(self.context_capacity).max(2_048);
        let context_limit =
            validated_context_limit(tokens.len(), request.max_tokens, context_capacity)?;
        let threads = std::thread::available_parallelism()
            .map(|value| value.get().min(12) as i32)
            .unwrap_or(4);
        let batch_capacity = 512_u32.min(context_limit);
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(context_limit))
            .with_n_batch(batch_capacity)
            .with_n_ubatch(batch_capacity)
            .with_n_threads(threads)
            .with_n_threads_batch(threads)
            .with_offload_kqv(use_gpu)
            .with_op_offload(use_gpu);
        let mut context = model
            .new_context(&*backend, params)
            .map_err(|error| InferenceFailure::runtime(format!("Model context failed: {error}")))?;
        let mut batch = LlamaBatch::new(batch_capacity as usize, 1);
        let last = tokens.len().saturating_sub(1);
        for (chunk_index, chunk) in tokens.chunks(batch_capacity as usize).enumerate() {
            if cancellation.load(Ordering::SeqCst) {
                return Err(AppError::Agent(
                    "Generation stopped while evaluating the local evidence context.".to_owned(),
                )
                .into());
            }
            batch.clear();
            let offset = chunk_index * batch_capacity as usize;
            for (chunk_position, token) in chunk.iter().copied().enumerate() {
                let position = offset + chunk_position;
                batch
                    .add(token, position as i32, &[0], position == last)
                    .map_err(|error| {
                        AppError::LocalModel(format!("Prompt batching failed: {error}"))
                    })?;
            }
            context.decode(&mut batch).map_err(|error| {
                InferenceFailure::runtime(format!("Prompt evaluation failed: {error}"))
            })?;
        }

        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_k(40),
            LlamaSampler::top_p(0.9, 1),
            LlamaSampler::temp(request.temperature),
            LlamaSampler::dist(42),
        ]);
        let mut decoder = UTF_8.new_decoder();
        let mut raw = String::new();
        let mut visible = String::new();
        let mut first_token_ms = None;
        let mut generated_tokens = 0_u32;
        let mut position = tokens.len() as i32;
        while generated_tokens < request.max_tokens {
            if cancellation.load(Ordering::SeqCst) {
                return Err(AppError::Agent(
                    "Generation stopped by the user. The partial answer was not marked complete."
                        .to_owned(),
                )
                .into());
            }
            let token = sampler.sample(&context, batch.n_tokens() - 1);
            sampler.accept(token);
            if model.is_eog_token(token) {
                break;
            }
            let piece = model
                .token_to_piece(token, &mut decoder, true, None)
                .map_err(|error| AppError::LocalModel(format!("Token decoding failed: {error}")))?;
            raw.push_str(&piece);
            let next_visible = visible_without_reasoning(&raw);
            if next_visible.len() > visible.len() {
                let delta = next_visible.strip_prefix(&visible).ok_or_else(|| {
                    AppError::LocalModel(
                        "The decoded model stream changed previously emitted text.".to_owned(),
                    )
                })?;
                if first_token_ms.is_none() && !delta.trim().is_empty() {
                    first_token_ms = Some(generation_started.elapsed().as_millis() as u64);
                }
                on_delta(delta);
                visible = next_visible;
            }
            batch.clear();
            batch
                .add(token, position, &[0], true)
                .map_err(|error| AppError::LocalModel(format!("Token batching failed: {error}")))?;
            context.decode(&mut batch).map_err(|error| {
                InferenceFailure::runtime(format!("Token evaluation failed: {error}"))
            })?;
            position += 1;
            generated_tokens += 1;
            if raw.len() > 512 * 1024 {
                return Err(AppError::LocalModel(
                    "The local model exceeded Graphite's output byte limit.".to_owned(),
                )
                .into());
            }
        }
        let final_text = visible_without_reasoning(&raw).trim().to_owned();
        Ok(GenerationResult {
            text: final_text,
            generated_tokens,
            time_to_first_token_ms: first_token_ms,
            generation_ms: generation_started.elapsed().as_millis() as u64,
        })
    }

    /// Pin GPU loads to one detected device; explicitly disable GPU model layers on CPU.
    fn load_model(
        &self,
        state: &mut ManagedRuntimeState,
        options: GpuOptions,
    ) -> AppResult<LlamaModel> {
        #[cfg(test)]
        if state.selected_device.is_some() && state.fail_gpu_stage == Some("load") {
            state.fail_gpu_stage = None;
            return Err(AppError::LocalModel("Injected GPU load failure".into()));
        }
        let devices: Vec<_> = state
            .selected_device
            .iter()
            .map(|device| device.index)
            .collect();
        let params = LlamaModelParams::default()
            .with_devices(&devices)
            .map_err(|error| AppError::LocalModel(format!("Device selection failed: {error}")))?
            .with_n_gpu_layers(if devices.is_empty() {
                0
            } else {
                options.layers()
            });
        LlamaModel::load_from_file(&state.backend, &self.model_path, &params).map_err(|error| {
            AppError::LocalModel(format!("The managed model could not be loaded: {error}"))
        })
    }
}

impl EvidenceTokenCounter for GraphiteManagedProvider {
    fn count_tokens(&self, text: &str) -> AppResult<u32> {
        let mut state =
            self.runtime.state.lock().map_err(|_| {
                AppError::LocalModel("The model runtime lock is poisoned.".to_owned())
            })?;
        self.ensure_loaded(&mut state)?;
        let model = &state.loaded.as_ref().expect("model was loaded").model;
        let count = model
            .str_to_token(text, AddBos::Never)
            .map_err(|error| {
                AppError::LocalModel(format!("Evidence tokenization failed: {error}"))
            })?
            .len();
        Ok(count.min(u32::MAX as usize) as u32)
    }

    fn token_accounting_method(&self) -> &'static str {
        "llama-cpp-gguf-model-tokenizer-v1"
    }
}

impl LocalModelProvider for GraphiteManagedProvider {
    fn provider_id(&self) -> &'static str {
        "graphite-managed"
    }

    fn provider_label(&self) -> String {
        "Graphite managed - offline".to_owned()
    }

    fn model_label(&self) -> String {
        self.profile.label.to_owned()
    }

    fn context_capacity(&self) -> u32 {
        self.context_capacity
    }

    fn generate(
        &self,
        request: &GenerationRequest,
        cancellation: Arc<AtomicBool>,
        on_delta: &mut dyn FnMut(&str),
    ) -> AppResult<GenerationResult> {
        let generation_started = Instant::now();
        let mut state =
            self.runtime.state.lock().map_err(|_| {
                AppError::LocalModel("The model runtime lock is poisoned.".to_owned())
            })?;
        self.ensure_loaded(&mut state)?;
        let use_gpu = state.selected_device.is_some();
        let allow_fallback = state
            .gpu_options
            .as_ref()
            .is_ok_and(|options| options.allows_cpu_fallback(self.inference_backend));
        let preparation_ms = generation_started.elapsed().as_millis() as u64;
        state.inference_completed = false;
        let mut emitted = false;
        let attempt = self.generate_once(&mut state, request, cancellation.clone(), &mut |delta| {
            emitted = true;
            on_delta(delta);
        });
        match attempt {
            Ok(mut result) => {
                result.generation_ms = generation_started.elapsed().as_millis() as u64;
                result.time_to_first_token_ms =
                    result.time_to_first_token_ms.map(|ms| ms + preparation_ms);
                state.inference_completed = true;
                Ok(result)
            }
            Err(failure) if use_gpu && failure.retryable && allow_fallback => {
                state.loaded = None;
                state.selected_device = None;
                state.inference_completed = false;
                state.cpu_fallback_reason = Some("GPU context or execution failed.".into());
                if !failure.can_retry(emitted, cancellation.load(Ordering::SeqCst)) {
                    return Err(failure.error);
                }
                self.ensure_loaded(&mut state)?;
                let preparation_ms = generation_started.elapsed().as_millis() as u64;
                let mut result = self
                    .generate_once(&mut state, request, cancellation, on_delta)
                    .map_err(|cpu| {
                        AppError::LocalModel(format!(
                            "GPU inference failed ({}); CPU fallback also failed ({}).",
                            failure.error, cpu.error
                        ))
                    })?;
                result.generation_ms = generation_started.elapsed().as_millis() as u64;
                result.time_to_first_token_ms =
                    result.time_to_first_token_ms.map(|ms| ms + preparation_ms);
                state.inference_completed = true;
                Ok(result)
            }
            Err(failure) => Err(failure.error),
        }
    }
}

/// Classify runtime allocation/decode failures separately from input and cancellation errors.
struct InferenceFailure {
    error: AppError,
    retryable: bool,
}

impl InferenceFailure {
    fn runtime(message: String) -> Self {
        Self {
            error: AppError::LocalModel(message),
            retryable: true,
        }
    }

    fn can_retry(&self, emitted: bool, cancelled: bool) -> bool {
        self.retryable && !emitted && !cancelled
    }
}

impl From<AppError> for InferenceFailure {
    fn from(error: AppError) -> Self {
        Self {
            error,
            retryable: false,
        }
    }
}

fn validated_context_limit(
    prompt_tokens: usize,
    output_tokens: u32,
    context_capacity: u32,
) -> AppResult<u32> {
    let required_tokens = prompt_tokens.saturating_add(output_tokens as usize);
    if required_tokens > context_capacity as usize {
        return Err(AppError::LocalModel(format!(
            "The assembled prompt requires {} tokens including output, above the {} token budget.",
            required_tokens, context_capacity
        )));
    }
    Ok((required_tokens as u32)
        .next_power_of_two()
        .max(2_048)
        .min(context_capacity))
}

fn format_chat_prompt(system: &str, user: &str) -> String {
    format!(
        "<|im_start|>system\n{system}<|im_end|>\n<|im_start|>user\n{user}\n/no_think<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
    )
}

pub(crate) fn visible_without_reasoning(raw: &str) -> String {
    let mut visible = String::new();
    let mut remaining = raw;
    loop {
        let think = remaining.find("<think>");
        let analysis = remaining.find("<analysis>");
        let opening = match (think, analysis) {
            (Some(left), Some(right)) => Some((
                left.min(right),
                if left <= right { "think" } else { "analysis" },
            )),
            (Some(index), None) => Some((index, "think")),
            (None, Some(index)) => Some((index, "analysis")),
            (None, None) => {
                let withheld = trailing_tag_prefix_len(remaining);
                visible.push_str(&remaining[..remaining.len().saturating_sub(withheld)]);
                break;
            }
        };
        let Some((index, tag)) = opening else {
            break;
        };
        visible.push_str(&remaining[..index]);
        let close = format!("</{tag}>");
        let after_open = &remaining[index + tag.len() + 2..];
        let Some(close_index) = after_open.find(&close) else {
            break;
        };
        remaining = &after_open[close_index + close.len()..];
    }
    visible
}

fn trailing_tag_prefix_len(value: &str) -> usize {
    const TAGS: [&str; 2] = ["<think>", "<analysis>"];
    let value = value.as_bytes();
    for length in (1..=value.len().min(9)).rev() {
        if TAGS.iter().any(|tag| {
            let tag = tag.as_bytes();
            length <= tag.len() && value.ends_with(&tag[..length])
        }) {
            return length;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use super::{
        profile_spec, validated_context_limit, verify_artifact, visible_without_reasoning,
        GraphiteManagedProvider, ManagedRuntime, ProfileSpec,
    };
    use crate::agent::model::GenerationRequest;
    use crate::agent::provider::{EvidenceTokenCounter, LocalModelProvider};

    #[test]
    fn curated_profiles_are_closed_and_pinned() {
        let profile = profile_spec("lightweight").expect("profile");
        assert_eq!(profile.expected_bytes, 2_497_280_256);
        assert!(profile.revision.len() >= 40);
        assert!(profile_spec("custom-path").is_err());
    }

    #[test]
    fn reasoning_blocks_never_reach_visible_output() {
        assert_eq!(
            visible_without_reasoning("<think>private plan</think>Supported answer [S1]."),
            "Supported answer [S1]."
        );
        assert_eq!(visible_without_reasoning("<thi"), "");
        assert_eq!(
            visible_without_reasoning("Visible <analysis>hidden"),
            "Visible "
        );
        assert_eq!(
            visible_without_reasoning("surface\u{2010}wave<thi"),
            "surface\u{2010}wave"
        );
    }

    #[test]
    fn artifact_verification_rejects_non_gguf_and_wrong_checksums() {
        let path = std::env::temp_dir().join(format!(
            "graphite-agent-artifact-{}-{}.gguf",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        std::fs::write(&path, b"NOPE").expect("fixture artifact");
        let profile = ProfileSpec {
            id: "fixture",
            label: "Fixture",
            repo_id: "fixture/model",
            revision: "fixture-revision",
            filename: "fixture.gguf",
            sha256: "not-a-valid-digest",
            expected_bytes: 4,
            context_capacity: 2_048,
        };
        assert!(verify_artifact(&path, profile).is_err());
        std::fs::write(&path, b"GGUF").expect("GGUF fixture artifact");
        assert!(verify_artifact(&path, profile).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cpu_retry_is_limited_to_runtime_failures_before_streaming() {
        let runtime = super::InferenceFailure::runtime("allocation failure".into());
        assert!(runtime.can_retry(false, false));
        assert!(!runtime.can_retry(true, false));
        assert!(!runtime.can_retry(false, true));
        let input = super::InferenceFailure::from(crate::error::AppError::LocalModel(
            "invalid input".into(),
        ));
        assert!(!input.can_retry(false, false));
    }

    #[test]
    #[ignore = "requires an explicitly supplied existing GGUF; never downloads"]
    fn isolated_hardware_inference_and_fallback() {
        let path = std::env::var("GRAPHITE_HARDWARE_MODEL_FILE").expect("explicit GGUF path");
        let profile =
            std::env::var("GRAPHITE_HARDWARE_PROFILE").unwrap_or_else(|_| "balanced".into());
        let expected =
            std::env::var("GRAPHITE_HARDWARE_EXPECT").expect("expected execution status");
        let runtime = ManagedRuntime::initialize().expect("runtime");
        let before = runtime.hardware_status().expect("device detection");
        if let Ok(stage) = std::env::var("GRAPHITE_HARDWARE_FAILURE_STAGE") {
            assert!(
                !before.devices.is_empty(),
                "fault injection requires a real GPU"
            );
            runtime.state.lock().unwrap().fail_gpu_stage = Some(match stage.as_str() {
                "load" => "load",
                "context" => "context",
                _ => panic!("unsupported failure stage"),
            });
        }
        let preference = match std::env::var("GRAPHITE_HARDWARE_BACKEND").as_deref() {
            Ok("cpu") => super::InferenceBackend::Cpu,
            Ok("gpu") => super::InferenceBackend::Gpu,
            Ok("auto") | Err(_) => super::InferenceBackend::Auto,
            Ok(value) => panic!("unsupported backend: {value}"),
        };
        let provider = GraphiteManagedProvider::new(runtime.clone(), &profile, path.clone().into())
            .unwrap()
            .with_preferences(preference, super::ContextWindow::Medium);
        if expected == "gpu-unavailable" {
            let error = provider
                .count_tokens("Require a compatible GPU.")
                .unwrap_err();
            assert!(error
                .to_string()
                .contains("no compatible device was initialized"));
            let status = runtime.hardware_status().unwrap();
            assert_eq!(status.status, expected);
            assert!(!status.available);
            assert!(runtime.state.lock().unwrap().loaded.is_none());
            return;
        }
        assert!(
            provider
                .count_tokens("Verify the selected model tokenizer.")
                .unwrap()
                > 0
        );
        let loaded = runtime.hardware_status().expect("load status");
        let request = GenerationRequest {
            system_prompt: "Answer only from the supplied evidence and cite [S1].".into(),
            user_prompt:
                "[S1] The reported frequency band is 0.08 to 0.35 Hz. What is the frequency band?"
                    .into(),
            max_tokens: 80,
            context_tokens: provider.context_capacity(),
            temperature: 0.0,
        };
        let mut streamed = String::new();
        let result = provider
            .generate(&request, Arc::new(AtomicBool::new(false)), &mut |delta| {
                streamed.push_str(delta)
            })
            .expect("inference or CPU fallback");
        assert_eq!(streamed.trim(), result.text);
        assert!(result.text.contains("0.08") && result.text.contains("0.35"));
        let after = runtime.hardware_status().expect("execution status");
        assert_eq!(after.status, expected);
        if expected == "cpu-fallback" {
            assert_eq!(after.backend, "CPU");
            assert!(!after.available);
            assert!(runtime.state.lock().unwrap().selected_device.is_none());
        }
        let installed_size = std::fs::metadata(&path).unwrap().len();
        runtime.unload().unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), installed_size);
        let reopened = GraphiteManagedProvider::new(runtime.clone(), &profile, path.clone().into())
            .unwrap()
            .with_preferences(preference, super::ContextWindow::Medium);
        assert!(
            reopened
                .count_tokens("Reuse the installed model offline.")
                .unwrap()
                > 0
        );
        runtime.unload().unwrap();
        let unloaded = runtime.hardware_status().unwrap();
        assert_ne!(unloaded.status, "gpu-active");
        assert_ne!(unloaded.status, "cpu-fallback");
        eprintln!(
            "hardware-check: {}",
            serde_json::json!({
                "before": before, "loaded": loaded, "after": after, "unloaded": unloaded,
                "answer": result.text, "generationMs": result.generation_ms,
            })
        );
    }

    #[test]
    fn managed_accounting_uses_gguf_tokenizer_and_final_capacity_guard_remains() {
        let runtime = ManagedRuntime::initialize().expect("managed runtime");
        let provider = GraphiteManagedProvider::new(
            runtime,
            "lightweight",
            std::path::PathBuf::from("unused-fixture.gguf"),
        )
        .expect("managed provider");
        assert_eq!(
            provider.token_accounting_method(),
            "llama-cpp-gguf-model-tokenizer-v1"
        );
        assert_eq!(
            validated_context_limit(1_000, 400, 2_048).expect("fitting prompt"),
            2_048
        );
        assert!(validated_context_limit(1_900, 200, 2_048).is_err());
        let provider =
            provider.with_preferences(super::InferenceBackend::Cpu, super::ContextWindow::Medium);
        assert_eq!(provider.context_capacity(), 4_096);
        assert_eq!(provider.inference_backend, super::InferenceBackend::Cpu);
    }

    #[test]
    #[ignore = "downloads a multi-gigabyte curated model for explicit live validation"]
    fn live_managed_profile_installs_and_generates_without_a_server() {
        let cache = std::env::var("GRAPHITE_M5_MODEL_CACHE")
            .expect("set GRAPHITE_M5_MODEL_CACHE to an explicit test cache");
        let profile_id =
            std::env::var("GRAPHITE_M5_PROFILE").unwrap_or_else(|_| "lightweight".to_owned());
        let model_path =
            if let Ok(relative_path) = std::env::var("GRAPHITE_M5_VERIFIED_MODEL_RELATIVE_PATH") {
                std::path::Path::new(&cache).join(relative_path)
            } else {
                let artifact = super::install_profile(
                    std::path::Path::new(&cache),
                    &profile_id,
                    Arc::new(|event| eprintln!("{}: {}", event.kind, event.message)),
                )
                .expect("verified profile installation");
                std::path::Path::new(&cache).join(artifact.cache_relative_path)
            };
        let runtime = super::ManagedRuntime::initialize().expect("managed runtime");
        let provider = super::GraphiteManagedProvider::new(runtime, &profile_id, model_path)
            .expect("managed provider");
        let request = GenerationRequest {
            system_prompt: "Answer only from evidence and cite [S1].".to_owned(),
            user_prompt:
                "Evidence [S1]: The frequency band is 0.08 to 0.35 Hz. What is the frequency band?"
                    .to_owned(),
            max_tokens: 80,
            context_tokens: provider.context_capacity(),
            temperature: 0.0,
        };
        for pass in ["cold", "warm"] {
            let mut streamed = String::new();
            let result = provider
                .generate(&request, Arc::new(AtomicBool::new(false)), &mut |delta| {
                    streamed.push_str(delta)
                })
                .expect("local generation");
            assert!(!result.text.trim().is_empty());
            assert_eq!(result.text, streamed.trim());
            assert!(!result.text.contains("<think>"));
            eprintln!("live-pass: {pass}");
            eprintln!("live-output: {}", result.text);
            eprintln!("live-ttft-ms: {:?}", result.time_to_first_token_ms);
            eprintln!("live-generation-ms: {}", result.generation_ms);
            eprintln!("live-generated-tokens: {}", result.generated_tokens);
        }
    }
}
