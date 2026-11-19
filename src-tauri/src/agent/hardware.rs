//! Describe usable runtime devices without confusing build flags with hardware.

use llama_cpp_2::{list_llama_ggml_backend_devices, LlamaBackendDeviceType};

use super::model::{ComputeDevice, InferenceBackend};
use crate::error::{AppError, AppResult};

/// Inventory Linux display adapters independently of inference support.
pub(super) fn graphics_adapters() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        let root = std::path::Path::new("/sys/bus/pci/devices");
        let Ok(entries) = std::fs::read_dir(root) else {
            return Vec::new();
        };
        let mut adapters = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let class = std::fs::read_to_string(path.join("class")).unwrap_or_default();
            if !class.trim().starts_with("0x03") {
                continue;
            }
            let vendor = std::fs::read_to_string(path.join("vendor")).unwrap_or_default();
            let device = std::fs::read_to_string(path.join("device")).unwrap_or_default();
            let vendor_name = match vendor.trim() {
                "0x10de" => "NVIDIA",
                "0x1002" => "AMD",
                "0x8086" => "Intel",
                _ => "Graphics adapter",
            };
            let information = std::path::Path::new("/proc/driver/nvidia/gpus")
                .join(entry.file_name())
                .join("information");
            let name = std::fs::read_to_string(information)
                .ok()
                .and_then(|text| {
                    text.lines().find_map(|line| {
                        line.strip_prefix("Model:")
                            .map(|name| name.trim().to_owned())
                    })
                })
                .unwrap_or_else(|| format!("{vendor_name} graphics ({})", device.trim()));
            adapters.push(name);
        }
        adapters.sort();
        adapters
    }
    #[cfg(not(target_os = "linux"))]
    Vec::new()
}

// Keep this order for both selection and compiled capability reporting.
const GPU_BACKENDS: [(&str, bool); 2] = [
    ("CUDA", cfg!(feature = "agent-cuda")),
    (
        "Metal",
        cfg!(feature = "agent-metal") || cfg!(target_os = "macos"),
    ),
];
const ALL_GPU_LAYERS: u32 = 1_000;

/// Process-local overrides for the linked runtime, independent of agent settings.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct GpuOptions {
    backend: Option<&'static str>,
    layers: Option<u32>,
}

impl GpuOptions {
    pub fn from_env() -> Result<Self, String> {
        fn read(name: &str) -> Result<Option<String>, String> {
            match std::env::var(name) {
                Ok(value) => Ok(Some(value)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(std::env::VarError::NotUnicode(_)) => {
                    Err(format!("{name} must be valid text."))
                }
            }
        }
        let backend = read("GRAPHITE_GPU_BACKEND")?;
        let layers = read("GRAPHITE_GPU_LAYERS")?;
        Self::parse(backend.as_deref(), layers.as_deref())
    }

    fn parse(backend: Option<&str>, layers: Option<&str>) -> Result<Self, String> {
        let backend = match backend.map(str::trim) {
            None => None,
            Some(value) if value.eq_ignore_ascii_case("auto") => None,
            Some(value) => Some(
                GPU_BACKENDS
                    .iter()
                    .map(|(name, _)| *name)
                    .find(|name| name.eq_ignore_ascii_case(value))
                    .ok_or("GRAPHITE_GPU_BACKEND must be auto, cuda, or metal.")?,
            ),
        };
        let layers = layers
            .map(|value| {
                value.trim().parse::<u32>().ok()
                    .filter(|count| (1..=i32::MAX as u32).contains(count))
                    .ok_or("GRAPHITE_GPU_LAYERS must be a positive integer up to 2147483647. Use CPU mode to disable offload.")
            })
            .transpose()?;
        Ok(Self { backend, layers })
    }

    pub fn layers(self) -> u32 {
        self.layers.unwrap_or(ALL_GPU_LAYERS)
    }

    pub fn allows_cpu_fallback(self, preference: InferenceBackend) -> bool {
        preference == InferenceBackend::Auto && self.backend.is_none()
    }
}

/// Honor explicit CPU and GPU choices and verify the selected device's budget.
pub(super) fn preferred_gpu(
    devices: &[ComputeDevice],
    model_bytes: u64,
    preference: InferenceBackend,
    options: GpuOptions,
) -> AppResult<Option<ComputeDevice>> {
    if preference == InferenceBackend::Cpu {
        return Ok(None);
    }
    let required_bytes = if options.layers.is_some() {
        // The cap is explicit. Tensor sizes and context overhead vary by layer
        // and model, so llama.cpp verifies the actual allocations.
        GPU_LOAD_HEADROOM_BYTES
    } else {
        model_bytes.saturating_add(GPU_LOAD_HEADROOM_BYTES)
    };
    let device = select_gpu(devices, required_bytes, options.backend);
    if device.is_none() && !options.allows_cpu_fallback(preference) {
        let target = options.backend.unwrap_or("GPU");
        let reason = if select_gpu(devices, 0, options.backend).is_none() {
            "no compatible device was initialized. Check the build and GPU driver"
        } else {
            "no compatible device has enough free memory. Lower the GPU layer cap or use a smaller model"
        };
        return Err(AppError::LocalModel(format!(
            "{target} inference was requested, but {reason}. Choose CPU to continue."
        )));
    }
    Ok(device)
}

/// Leave room for runtime allocations before attempting a model load.
const GPU_LOAD_HEADROOM_BYTES: u64 = 512 * 1024 * 1024;

/// Enumerate GPUs initialized by the linked runtime, including integrated GPUs.
pub(super) fn gpu_devices() -> Vec<ComputeDevice> {
    list_llama_ggml_backend_devices()
        .into_iter()
        .filter(|device| {
            matches!(
                device.device_type,
                LlamaBackendDeviceType::Gpu | LlamaBackendDeviceType::IntegratedGpu
            )
        })
        .map(|device| ComputeDevice {
            index: device.index,
            name: device.description,
            backend: device.backend,
            kind: if device.device_type == LlamaBackendDeviceType::IntegratedGpu {
                "integrated"
            } else {
                "discrete"
            }
            .to_owned(),
            total_memory_bytes: device.memory_total as u64,
            available_memory_bytes: device.memory_free as u64,
        })
        .collect()
}

/// Prefer the backend first, then free memory and the lowest runtime index.
///
/// Zero total memory means the backend did not report a limit. Context and
/// compute allocations are verified by the actual load and inference attempt.
fn select_gpu(
    devices: &[ComputeDevice],
    required_bytes: u64,
    backend: Option<&str>,
) -> Option<ComputeDevice> {
    devices
        .iter()
        .filter(|device| backend.is_none_or(|name| name == device.backend))
        .filter(|device| {
            device.total_memory_bytes == 0 || device.available_memory_bytes >= required_bytes
        })
        .max_by_key(|device| {
            (
                std::cmp::Reverse(
                    GPU_BACKENDS
                        .iter()
                        .position(|(name, _)| *name == device.backend)
                        .unwrap_or(GPU_BACKENDS.len()),
                ),
                device.available_memory_bytes,
                std::cmp::Reverse(device.index),
            )
        })
        .cloned()
}

/// Name the linked backend separately from the actual execution device.
pub(super) fn compiled_backend() -> String {
    let backends = GPU_BACKENDS
        .iter()
        .filter_map(|(name, enabled)| enabled.then_some(*name))
        .collect::<Vec<_>>();
    if backends.is_empty() {
        "CPU".to_owned()
    } else {
        backends.join(", ")
    }
}

#[derive(Clone, serde::Serialize)]
pub(super) struct RuntimeHardware {
    pub devices: Vec<ComputeDevice>,
    pub backend: String,
    pub available: bool,
    pub status: String,
    pub note: String,
}

impl RuntimeHardware {
    /// Describe detection, model loading and completed inference separately.
    pub fn describe(
        devices: Vec<ComputeDevice>,
        selected: Option<&ComputeDevice>,
        completed: bool,
        fallback: Option<&str>,
        preference: InferenceBackend,
        options: Result<GpuOptions, &str>,
    ) -> Self {
        let candidate = options
            .ok()
            .and_then(|options| select_gpu(&devices, 0, options.backend));
        let candidate = selected.or(candidate.as_ref());
        let requested_backend = options.ok().and_then(|options| options.backend);
        let compiled = compiled_backend();
        let (backend, status, note) = if preference == InferenceBackend::Cpu {
            ("CPU", "cpu-selected", "CPU selected for answer generation. Graph rendering and text indexing are separate.".to_owned())
        } else if let Some(reason) = fallback {
            (
                "CPU",
                "cpu-fallback",
                format!("Using CPU. {reason} Unload the model to retry automatic GPU selection."),
            )
        } else if let Err(error) = options {
            ("CPU", "gpu-unavailable", error.to_owned())
        } else if let Some(device) = candidate {
            let (status, action) = if completed && selected.is_some() {
                ("gpu-active", "GPU inference completed successfully")
            } else if selected.is_some() {
                (
                    "gpu-loaded",
                    "GPU model loaded; inference is not yet verified",
                )
            } else {
                (
                    "gpu-ready",
                    "GPU available; the selected inference mode applies when the model loads",
                )
            };
            (
                device.backend.as_str(),
                status,
                format!("{action}: {}.", device.name),
            )
        } else if let Some(backend) = requested_backend {
            (backend, "gpu-unavailable", format!("No usable {backend} device was initialized. Check the build and driver, or choose CPU."))
        } else if preference == InferenceBackend::Gpu {
            (
                "GPU",
                "gpu-unavailable",
                "No usable GPU device was initialized. Check the build and driver, or choose CPU."
                    .to_owned(),
            )
        } else if compiled == "CPU" {
            ("CPU", "cpu", "Using CPU. This build has no GPU inference backend; GPU rendering does not enable GPU inference.".to_owned())
        } else {
            ("CPU", "cpu", format!("Using CPU. No usable GPU device was initialized by the linked {compiled} runtime. Check GPU availability and restart the app to retry detection."))
        };
        Self {
            backend: backend.to_owned(),
            available: candidate.is_some()
                && fallback.is_none()
                && preference != InferenceBackend::Cpu,
            status: status.to_owned(),
            note,
            devices,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(index: usize, free_gib: u64) -> ComputeDevice {
        ComputeDevice {
            index,
            name: format!("GPU {index}"),
            backend: "CUDA".into(),
            kind: "discrete".into(),
            total_memory_bytes: 16 << 30,
            available_memory_bytes: free_gib << 30,
        }
    }

    #[test]
    fn explicit_backend_choice_is_respected_and_unavailable_gpu_errors() {
        let devices = vec![device(0, 8)];
        let options = GpuOptions::default();
        assert!(preferred_gpu(&devices, 1, InferenceBackend::Cpu, options)
            .unwrap()
            .is_none());
        assert_eq!(
            preferred_gpu(&devices, 1, InferenceBackend::Gpu, options)
                .unwrap()
                .unwrap()
                .index,
            0
        );
        assert!(preferred_gpu(&[], 1, InferenceBackend::Gpu, options).is_err());
        assert!(preferred_gpu(&devices, 9 << 30, InferenceBackend::Gpu, options).is_err());
        assert!(preferred_gpu(&[], 1, InferenceBackend::Auto, options)
            .unwrap()
            .is_none());
        let status = RuntimeHardware::describe(
            devices,
            None,
            false,
            None,
            InferenceBackend::Cpu,
            Ok(options),
        );
        assert_eq!(status.status, "cpu-selected");
        assert!(!status.available);
        assert_eq!(status.devices.len(), 1);
    }

    #[test]
    fn selection_respects_free_memory_and_has_stable_ties() {
        let select = |devices: &[ComputeDevice], bytes| {
            preferred_gpu(
                devices,
                bytes,
                InferenceBackend::Auto,
                GpuOptions::default(),
            )
            .unwrap()
        };
        let devices = vec![device(0, 2), device(1, 8), device(2, 8)];
        assert_eq!(select(&devices, 5 << 30).unwrap().index, 1);
        assert!(select(&devices, 9 << 30).is_none());
        assert!(select(&[], 1).is_none());
        let mut integrated = device(0, 0);
        integrated.kind = "integrated".into();
        integrated.total_memory_bytes = 0;
        assert!(select(&[integrated], 5 << 30).is_some());
    }

    #[test]
    fn backend_priority_respects_memory_and_explicit_overrides() {
        let cuda = device(1, 4);
        let metal = ComputeDevice {
            backend: "Metal".into(),
            ..device(0, 6)
        };
        let devices = vec![metal, cuda];
        let select = |gib: u64| {
            preferred_gpu(
                &devices,
                gib << 30,
                InferenceBackend::Auto,
                GpuOptions::default(),
            )
            .unwrap()
            .unwrap()
            .index
        };
        assert_eq!(select(3), 1);
        assert_eq!(select(4), 0);

        let manual = GpuOptions::parse(Some("metal"), None).unwrap();
        assert_eq!(
            preferred_gpu(&devices, 1, InferenceBackend::Auto, manual)
                .unwrap()
                .unwrap()
                .index,
            0
        );
        let error = preferred_gpu(&devices[1..], 1, InferenceBackend::Auto, manual)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Metal inference was requested"));
        assert!(!manual.allows_cpu_fallback(InferenceBackend::Auto));
        let status = RuntimeHardware::describe(
            devices[1..].to_vec(),
            None,
            false,
            None,
            InferenceBackend::Auto,
            Ok(manual),
        );
        assert_eq!(status.status, "gpu-unavailable");
        assert!(!status.available);
    }

    #[test]
    fn layer_cap_supports_partial_offload_and_validates_overrides() {
        let options = GpuOptions::parse(Some(" Auto "), Some("8")).unwrap();
        assert_eq!(options.layers(), 8);
        assert!(options.allows_cpu_fallback(InferenceBackend::Auto));
        assert!(
            preferred_gpu(&[device(0, 2)], 5 << 30, InferenceBackend::Gpu, options)
                .unwrap()
                .is_some()
        );
        assert!(
            preferred_gpu(&[device(0, 0)], 5 << 30, InferenceBackend::Auto, options)
                .unwrap()
                .is_none()
        );
        for unsupported in ["vulkan", "rocm"] {
            assert!(GpuOptions::parse(Some(unsupported), None).is_err());
        }
        for invalid in ["", "0", "-1", "1.5", "2147483648"] {
            assert!(GpuOptions::parse(None, Some(invalid)).is_err());
        }
    }

    #[test]
    fn reporting_never_calls_detection_or_fallback_active_inference() {
        let gpu = device(0, 8);
        let options = Ok(GpuOptions::default());
        let ready = RuntimeHardware::describe(
            vec![gpu.clone()],
            None,
            false,
            None,
            InferenceBackend::Auto,
            options,
        );
        assert_eq!(ready.status, "gpu-ready");
        let loaded = RuntimeHardware::describe(
            vec![gpu.clone()],
            Some(&gpu),
            false,
            None,
            InferenceBackend::Auto,
            options,
        );
        assert_eq!(loaded.status, "gpu-loaded");
        let active = RuntimeHardware::describe(
            vec![gpu.clone()],
            Some(&gpu),
            true,
            None,
            InferenceBackend::Auto,
            options,
        );
        assert_eq!(active.status, "gpu-active");
        let fallback = RuntimeHardware::describe(
            vec![gpu.clone()],
            Some(&gpu),
            true,
            Some("GPU load failed."),
            InferenceBackend::Auto,
            options,
        );
        assert_eq!(fallback.backend, "CPU");
        assert!(!fallback.available);
        assert!(fallback.note.contains("GPU load failed"));
        let missing =
            RuntimeHardware::describe(vec![], None, false, None, InferenceBackend::Auto, options);
        assert_eq!(missing.backend, "CPU");
        assert!(!missing.available);
    }
}
