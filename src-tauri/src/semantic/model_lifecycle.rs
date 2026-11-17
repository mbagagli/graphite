use std::fs;
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::DEFAULT_MODEL_ID;
use crate::error::{AppError, AppResult};

pub(super) const MODEL_MARKER_FILE: &str = "graphite-semantic-model.json";
pub(super) const MODEL_FILE: &str = "model_optimized.onnx";
pub(super) const TOKENIZER_FILES: &[&str] = &[
    "tokenizer.json",
    "config.json",
    "special_tokens_map.json",
    "tokenizer_config.json",
];
const MODEL_SNAPSHOT_ROOT: &str = "models--Qdrant--bge-small-en-v1.5-onnx-Q";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct ModelInstallationMarker {
    pub(super) model_identifier: String,
    pub(super) artifact_identity: String,
    pub(super) snapshot_relative_path: String,
    pub(super) installed_bytes: u64,
    pub(super) installed_at: String,
}

#[derive(Clone)]
pub(super) struct ModelLifecycle {
    root: PathBuf,
}

impl ModelLifecycle {
    pub(super) fn initialize(root: PathBuf) -> AppResult<Self> {
        fs::create_dir_all(&root).map_err(|error| {
            AppError::ApplicationData(format!(
                "The semantic-model directory could not be created: {error}"
            ))
        })?;
        Ok(Self { root })
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn marker_path(&self) -> PathBuf {
        self.root.join(MODEL_MARKER_FILE)
    }

    pub(super) fn read_marker(&self) -> AppResult<Option<ModelInstallationMarker>> {
        let path = self.marker_path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&path).map_err(|error| {
            AppError::Semantic(format!("The local model marker could not be read: {error}"))
        })?;
        let marker: ModelInstallationMarker = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Semantic(format!("The local model marker is invalid: {error}"))
        })?;
        self.validate_marker(&marker)?;
        Ok(Some(marker))
    }

    pub(super) fn validate_marker(&self, marker: &ModelInstallationMarker) -> AppResult<PathBuf> {
        if marker.model_identifier != DEFAULT_MODEL_ID {
            return Err(AppError::Semantic(
                "The installed semantic model is not supported by this Graphite build".to_owned(),
            ));
        }
        let relative = Path::new(&marker.snapshot_relative_path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            return Err(AppError::Semantic(
                "The local semantic-model marker contains an unsafe path".to_owned(),
            ));
        }
        let snapshot = self.root.join(relative);
        for file in std::iter::once(MODEL_FILE).chain(TOKENIZER_FILES.iter().copied()) {
            let path = snapshot.join(file);
            if !path.is_file() || path.metadata().map(|value| value.len()).unwrap_or(0) == 0 {
                return Err(AppError::Semantic(format!(
                    "The local semantic model is incomplete ({file} is missing). Reinstall it from Data Source."
                )));
            }
        }
        Ok(snapshot)
    }

    pub(super) fn discover_snapshot(&self) -> AppResult<PathBuf> {
        let snapshots = self.root.join(MODEL_SNAPSHOT_ROOT).join("snapshots");
        let mut candidates = fs::read_dir(&snapshots)
            .map_err(|error| {
                AppError::Semantic(format!(
                    "The downloaded model snapshot could not be inspected: {error}"
                ))
            })?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && path.join(MODEL_FILE).is_file())
            .collect::<Vec<_>>();
        candidates.sort();
        candidates.pop().ok_or_else(|| {
            AppError::Semantic(
                "The model download completed without a usable ONNX snapshot".to_owned(),
            )
        })
    }

    pub(super) fn finalize_downloaded_snapshot(
        &self,
        snapshot: &Path,
    ) -> AppResult<ModelInstallationMarker> {
        let artifact_identity = snapshot
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| AppError::Semantic("The model revision is not valid UTF-8".to_owned()))?
            .to_owned();
        let installed_bytes = directory_size(&self.root);
        let relative = snapshot.strip_prefix(&self.root).map_err(|error| {
            AppError::Semantic(format!("The model snapshot path is invalid: {error}"))
        })?;
        let marker = ModelInstallationMarker {
            model_identifier: DEFAULT_MODEL_ID.to_owned(),
            artifact_identity,
            snapshot_relative_path: relative.to_string_lossy().into_owned(),
            installed_bytes,
            installed_at: Utc::now().to_rfc3339(),
        };
        let marker_bytes = serde_json::to_vec_pretty(&marker).map_err(|error| {
            AppError::Semantic(format!("The model marker could not be encoded: {error}"))
        })?;
        fs::write(self.marker_path(), marker_bytes).map_err(|error| {
            AppError::Semantic(format!(
                "The model installation could not be finalized: {error}"
            ))
        })?;
        Ok(marker)
    }

    pub(super) fn read_asset(snapshot: &Path, name: &str) -> AppResult<Vec<u8>> {
        fs::read(snapshot.join(name)).map_err(|error| {
            AppError::Semantic(format!(
                "The local model asset {name} could not be read: {error}"
            ))
        })
    }
}

fn directory_size(root: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .map(|path| {
            path.metadata()
                .ok()
                .map(|metadata| {
                    if metadata.is_dir() {
                        directory_size(&path)
                    } else {
                        metadata.len()
                    }
                })
                .unwrap_or(0)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_embedding_assets_reopen_without_download() {
        let root = std::env::temp_dir().join(format!(
            "graphite-semantic-reopen-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let snapshot = root.join("fixture-revision");
        fs::create_dir_all(&snapshot).unwrap();
        for name in std::iter::once(MODEL_FILE).chain(TOKENIZER_FILES.iter().copied()) {
            fs::write(snapshot.join(name), b"local fixture").unwrap();
        }
        let lifecycle = ModelLifecycle::initialize(root.clone()).unwrap();
        lifecycle.finalize_downloaded_snapshot(&snapshot).unwrap();
        drop(lifecycle);
        let reopened = ModelLifecycle::initialize(root.clone()).unwrap();
        let marker = reopened.read_marker().unwrap().unwrap();
        assert_eq!(reopened.validate_marker(&marker).unwrap(), snapshot);
        fs::remove_file(snapshot.join(MODEL_FILE)).unwrap();
        assert!(reopened
            .read_marker()
            .unwrap_err()
            .to_string()
            .contains("missing"));
        assert!(!snapshot.join(MODEL_FILE).exists());
        fs::remove_dir_all(root).unwrap();
    }
}
