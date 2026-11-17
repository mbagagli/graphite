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

/// Honor explicit CPU/GPU choices; only Auto may select CPU as a fallback.
pub(super) fn preferred_gpu(
    devices: &[ComputeDevice],
    model_bytes: u64,
    preference: InferenceBackend,
) -> AppResult<Option<ComputeDevice>> {
    if preference == InferenceBackend::Cpu {
        return Ok(None);
    }
    let device = select_gpu(devices, model_bytes);
    if preference == InferenceBackend::Gpu && device.is_none() {
        return Err(AppError::LocalModel(
            "GPU was selected, but no compatible GPU has enough available memory. Use Auto or CPU, or a GPU-enabled build with a compatible driver.".into(),
        ));
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

/// Prefer the device with most free memory that can plausibly load the weights.
///
/// Zero total memory means the backend did not report a limit. Context and
/// compute allocations are verified by the actual load and inference attempt.
pub(super) fn select_gpu(devices: &[ComputeDevice], model_bytes: u64) -> Option<ComputeDevice> {
    devices
        .iter()
        .filter(|device| {
            device.total_memory_bytes == 0
                || device.available_memory_bytes
                    >= model_bytes.saturating_add(GPU_LOAD_HEADROOM_BYTES)
        })
        .max_by_key(|device| {
            (
                device.available_memory_bytes,
                std::cmp::Reverse(device.index),
            )
        })
        .cloned()
}

/// Name the linked backend separately from the actual execution device.
pub(super) fn compiled_backend() -> &'static str {
    if cfg!(feature = "agent-cuda") {
        "CUDA"
    } else if cfg!(feature = "agent-metal") || cfg!(target_os = "macos") {
        "Metal"
    } else if cfg!(feature = "agent-vulkan") {
        "Vulkan"
    } else {
        "CPU"
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
    ) -> Self {
        let candidate = selected.or_else(|| devices.first());
        let (backend, status, note) = if preference == InferenceBackend::Cpu {
            ("CPU", "cpu-selected", "CPU selected for answer generation. Graph rendering and text indexing are separate.".to_owned())
        } else if let Some(reason) = fallback {
            (
                "CPU",
                "cpu-fallback",
                format!("Using CPU. {reason} Unload the model to retry automatic GPU selection."),
            )
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
        } else if compiled_backend() == "CPU" {
            ("CPU", "cpu", "Using CPU. This build has no GPU inference backend; GPU rendering does not enable GPU inference.".to_owned())
        } else {
            ("CPU", "cpu", format!("Using CPU. No usable {} device was initialized. Check GPU availability and restart the app to retry detection.", compiled_backend()))
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
        assert!(preferred_gpu(&devices, 1, InferenceBackend::Cpu)
            .unwrap()
            .is_none());
        assert_eq!(
            preferred_gpu(&devices, 1, InferenceBackend::Gpu)
                .unwrap()
                .unwrap()
                .index,
            0
        );
        assert!(preferred_gpu(&[], 1, InferenceBackend::Gpu).is_err());
        assert!(preferred_gpu(&devices, 9 << 30, InferenceBackend::Gpu).is_err());
        assert!(preferred_gpu(&[], 1, InferenceBackend::Auto)
            .unwrap()
            .is_none());
        let status = RuntimeHardware::describe(devices, None, false, None, InferenceBackend::Cpu);
        assert_eq!(status.status, "cpu-selected");
        assert!(!status.available);
        assert_eq!(status.devices.len(), 1);
    }

    #[test]
    fn selection_respects_free_memory_and_has_stable_ties() {
        let devices = vec![device(0, 2), device(1, 8), device(2, 8)];
        assert_eq!(select_gpu(&devices, 5 << 30).unwrap().index, 1);
        assert!(select_gpu(&devices, 9 << 30).is_none());
        assert!(select_gpu(&[], 1).is_none());
        let mut integrated = device(0, 0);
        integrated.kind = "integrated".into();
        integrated.total_memory_bytes = 0;
        assert!(select_gpu(&[integrated], 5 << 30).is_some());
    }

    #[test]
    fn reporting_never_calls_detection_or_fallback_active_inference() {
        let gpu = device(0, 8);
        let ready =
            RuntimeHardware::describe(vec![gpu.clone()], None, false, None, InferenceBackend::Auto);
        assert_eq!(ready.status, "gpu-ready");
        let loaded = RuntimeHardware::describe(
            vec![gpu.clone()],
            Some(&gpu),
            false,
            None,
            InferenceBackend::Auto,
        );
        assert_eq!(loaded.status, "gpu-loaded");
        let active = RuntimeHardware::describe(
            vec![gpu.clone()],
            Some(&gpu),
            true,
            None,
            InferenceBackend::Auto,
        );
        assert_eq!(active.status, "gpu-active");
        let fallback = RuntimeHardware::describe(
            vec![gpu.clone()],
            Some(&gpu),
            true,
            Some("GPU load failed."),
            InferenceBackend::Auto,
        );
        assert_eq!(fallback.backend, "CPU");
        assert!(!fallback.available);
        assert!(fallback.note.contains("GPU load failed"));
        let missing = RuntimeHardware::describe(vec![], None, false, None, InferenceBackend::Auto);
        assert_eq!(missing.backend, "CPU");
        assert!(!missing.available);
    }
}
