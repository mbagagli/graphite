use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{AppError, AppResult};

use super::model::{ExperimentDefinition, EXPERIMENT_PROFILE_ID};

const DATABASE_FILENAME: &str = "experiment.sqlite3";
const MANIFEST_FILENAME: &str = "manifest.json";

#[derive(Clone, Debug)]
pub(super) struct ExperimentRuntime {
    root: PathBuf,
    definition_root: PathBuf,
}

impl ExperimentRuntime {
    pub(super) fn prepare(data_root: &Path, definition: &ExperimentDefinition) -> AppResult<Self> {
        validate_definition_id(&definition.definition_id)?;
        let experiment_root = data_root.join("experiments").join(EXPERIMENT_PROFILE_ID);
        ensure_safe_directory_chain(data_root, &experiment_root)?;
        let definition_root = experiment_root.join(&definition.definition_id);
        if definition_root.exists() {
            reject_symlink(&definition_root)?;
        } else {
            fs::create_dir(&definition_root).map_err(runtime_error)?;
        }
        ensure_direct_child(&experiment_root, &definition_root)?;
        for child in ["staging", "results"] {
            let path = definition_root.join(child);
            if path.exists() {
                reject_symlink(&path)?;
            } else {
                fs::create_dir(&path).map_err(runtime_error)?;
            }
        }
        Ok(Self {
            root: experiment_root,
            definition_root,
        })
    }

    #[cfg(test)]
    pub(super) fn definition_root(&self) -> &Path {
        &self.definition_root
    }

    pub(super) fn existing(data_root: &Path, definition_id: &str) -> AppResult<Self> {
        validate_definition_id(definition_id)?;
        let root = data_root.join("experiments").join(EXPERIMENT_PROFILE_ID);
        let definition_root = root.join(definition_id);
        ensure_direct_child(&root, &definition_root)?;
        if !root.is_dir() || !definition_root.is_dir() {
            return Err(AppError::ApplicationData(
                "The requested experimental definition does not exist".to_owned(),
            ));
        }
        reject_symlink(&root)?;
        reject_symlink(&definition_root)?;
        Ok(Self {
            root,
            definition_root,
        })
    }

    pub(super) fn database_path(&self) -> PathBuf {
        self.definition_root.join(DATABASE_FILENAME)
    }

    pub(super) fn vectors_path(&self) -> PathBuf {
        self.definition_root.join("vectors")
    }

    pub(super) fn manifest_path(&self) -> PathBuf {
        self.definition_root.join(MANIFEST_FILENAME)
    }

    pub(super) fn staging_path(&self) -> PathBuf {
        self.definition_root.join("staging")
    }

    pub(super) fn reset_staging(&self) -> AppResult<PathBuf> {
        let staging = self.staging_path();
        ensure_direct_child(&self.definition_root, &staging)?;
        reject_symlink(&staging)?;
        for entry in fs::read_dir(&staging).map_err(runtime_error)? {
            let entry = entry.map_err(runtime_error)?;
            let path = entry.path();
            reject_symlink(&path)?;
            if path.is_dir() {
                fs::remove_dir_all(&path).map_err(runtime_error)?;
            } else {
                fs::remove_file(&path).map_err(runtime_error)?;
            }
        }
        Ok(staging)
    }

    pub(super) fn publish_staging(&self) -> AppResult<()> {
        let staging = self.staging_path();
        let staged_database = staging.join(DATABASE_FILENAME);
        let staged_vectors = staging.join("vectors");
        let staged_manifest = staging.join(MANIFEST_FILENAME);
        for path in [&staged_database, &staged_vectors, &staged_manifest] {
            if !path.exists() {
                return Err(AppError::ApplicationData(
                    "The experimental index staging output is incomplete".to_owned(),
                ));
            }
            reject_symlink(path)?;
        }
        if self.database_path().exists()
            || self.vectors_path().exists()
            || self.manifest_path().exists()
        {
            return Err(AppError::ApplicationData(
                "The experimental definition is already published".to_owned(),
            ));
        }
        fs::rename(staged_database, self.database_path()).map_err(runtime_error)?;
        fs::rename(staged_vectors, self.vectors_path()).map_err(runtime_error)?;
        fs::rename(staged_manifest, self.manifest_path()).map_err(runtime_error)?;
        Ok(())
    }

    pub(super) fn recover_incomplete_publication(
        &self,
        definition: &ExperimentDefinition,
    ) -> AppResult<bool> {
        let database = self.database_path();
        let vectors = self.vectors_path();
        let manifest = self.manifest_path();
        let present = [database.exists(), vectors.exists(), manifest.exists()];
        if !present.iter().any(|value| *value) {
            return Ok(false);
        }
        if present.iter().all(|value| *value) && self.validate_published(definition).is_ok() {
            return Ok(true);
        }
        for path in [&database, &vectors, &manifest] {
            ensure_direct_child(&self.definition_root, path)?;
            if !path.exists() {
                continue;
            }
            reject_symlink(path)?;
            if path.is_dir() {
                fs::remove_dir_all(path).map_err(runtime_error)?;
            } else {
                fs::remove_file(path).map_err(runtime_error)?;
            }
        }
        Ok(false)
    }

    pub(super) fn validate_published(&self, definition: &ExperimentDefinition) -> AppResult<()> {
        let database = self.database_path();
        let vectors = self.vectors_path();
        let manifest = self.manifest_path();
        for path in [&database, &vectors, &manifest] {
            ensure_direct_child(&self.definition_root, path)?;
            reject_symlink(path)?;
        }
        if !database.is_file() || !vectors.is_dir() || !manifest.is_file() {
            return Err(AppError::ApplicationData(
                "The published experimental index is incomplete".to_owned(),
            ));
        }
        for kind in ["paper", "chunk"] {
            let vector_file = vectors.join(format!(
                "{}-{kind}.usearch",
                definition.embedding_profile_id
            ));
            if !vector_file.is_file() {
                return Err(AppError::ApplicationData(format!(
                    "The published experimental {kind} vector index is missing"
                )));
            }
            reject_symlink(&vector_file)?;
        }
        let payload: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest).map_err(runtime_error)?)?;
        if payload
            .pointer("/definition/definitionId")
            .and_then(serde_json::Value::as_str)
            != Some(definition.definition_id.as_str())
            || payload.get("state").and_then(serde_json::Value::as_str) != Some("ready")
        {
            return Err(AppError::ApplicationData(
                "The published experiment manifest does not match its definition".to_owned(),
            ));
        }
        Ok(())
    }

    pub(super) fn cleanup(self) -> AppResult<()> {
        validate_definition_id(
            self.definition_root
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default(),
        )?;
        ensure_direct_child(&self.root, &self.definition_root)?;
        reject_symlink(&self.definition_root)?;
        fs::remove_dir_all(&self.definition_root).map_err(runtime_error)
    }
}

fn ensure_safe_directory_chain(data_root: &Path, target: &Path) -> AppResult<()> {
    if data_root.as_os_str().is_empty() || !data_root.is_absolute() {
        return Err(AppError::ApplicationData(
            "The experiment data root must be an absolute path".to_owned(),
        ));
    }
    fs::create_dir_all(data_root).map_err(runtime_error)?;
    reject_symlink(data_root)?;
    let relative = target.strip_prefix(data_root).map_err(|_| {
        AppError::ApplicationData("The experiment root escaped app data".to_owned())
    })?;
    let mut current = data_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(AppError::ApplicationData(
                "The experiment path contains an unsafe component".to_owned(),
            ));
        };
        current.push(component);
        if current.exists() {
            reject_symlink(&current)?;
        } else {
            fs::create_dir(&current).map_err(runtime_error)?;
        }
    }
    Ok(())
}

fn ensure_direct_child(parent: &Path, child: &Path) -> AppResult<()> {
    if child.parent() != Some(parent) {
        return Err(AppError::ApplicationData(
            "The experiment target is not a direct child of its managed root".to_owned(),
        ));
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> AppResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(runtime_error)?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::ApplicationData(
            "The experimental index path cannot use a symbolic link".to_owned(),
        ));
    }
    Ok(())
}

fn validate_definition_id(value: &str) -> AppResult<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(AppError::ApplicationData(
            "The experiment definition identity is invalid".to_owned(),
        ));
    }
    Ok(())
}

fn runtime_error(error: std::io::Error) -> AppError {
    AppError::ApplicationData(format!("The experiment runtime path failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_non_hash_definition_identities() {
        assert!(validate_definition_id("../production").is_err());
        assert!(validate_definition_id(&"a".repeat(64)).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_link_runtime_roots() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!(
            "graphite-experiment-symlink-{}",
            std::process::id()
        ));
        let actual = base.join("actual");
        let linked = base.join("linked");
        fs::create_dir_all(&actual).expect("actual root");
        symlink(&actual, &linked).expect("runtime symlink");
        let definition = ExperimentDefinition::frozen("frozen", "profile", "artifact");
        assert!(ExperimentRuntime::prepare(&linked, &definition).is_err());
        fs::remove_dir_all(base).expect("remove symlink fixture");
    }
}
