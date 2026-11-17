use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use super::{validate_dimension, EMBEDDING_DIMENSION};
use crate::error::{AppError, AppResult};

pub trait VectorIndex: Send + Sync {
    fn upsert(&self, profile_id: &str, kind: &str, vectors: &[(u64, Vec<f32>)]) -> AppResult<()>;
    fn remove(&self, profile_id: &str, kind: &str, keys: &[u64]) -> AppResult<()>;
    fn nearest(
        &self,
        profile_id: &str,
        kind: &str,
        query: &[f32],
        limit: usize,
        allowed_keys: &HashSet<u64>,
    ) -> AppResult<Vec<VectorMatch>>;
    fn vector(&self, profile_id: &str, kind: &str, key: u64) -> AppResult<Option<Vec<f32>>>;
    fn entry_count(&self, profile_id: &str, kind: &str) -> AppResult<u64>;
    fn rebuild(&self, profile_id: &str) -> AppResult<()>;
    fn storage_bytes(&self, profile_id: &str) -> u64;
}

#[derive(Clone, Debug)]
pub struct VectorMatch {
    pub key: u64,
    pub similarity: f64,
}

pub struct UsearchVectorIndex {
    root: PathBuf,
    loaded: Mutex<HashMap<String, Index>>,
}

impl UsearchVectorIndex {
    pub fn new(root: PathBuf) -> AppResult<Self> {
        fs::create_dir_all(&root).map_err(|error| {
            AppError::ApplicationData(format!(
                "The semantic-index directory could not be created: {error}"
            ))
        })?;
        Ok(Self {
            root,
            loaded: Mutex::new(HashMap::new()),
        })
    }

    fn index_key(profile_id: &str, kind: &str) -> AppResult<String> {
        if !profile_id.chars().all(|value| value.is_ascii_hexdigit())
            || !matches!(kind, "paper" | "chunk")
        {
            return Err(AppError::Semantic(
                "The vector-index identity is invalid".to_owned(),
            ));
        }
        Ok(format!("{profile_id}-{kind}"))
    }

    fn path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.usearch"))
    }

    fn with_index<T>(
        &self,
        profile_id: &str,
        kind: &str,
        operation: impl FnOnce(&Index) -> AppResult<T>,
    ) -> AppResult<T> {
        let key = Self::index_key(profile_id, kind)?;
        let mut loaded = self
            .loaded
            .lock()
            .map_err(|_| AppError::Semantic("The vector-index lock was poisoned".to_owned()))?;
        if !loaded.contains_key(&key) {
            let options = IndexOptions {
                dimensions: EMBEDDING_DIMENSION,
                metric: MetricKind::Cos,
                quantization: ScalarKind::F32,
                connectivity: 16,
                expansion_add: 64,
                expansion_search: 48,
                ..Default::default()
            };
            let index = Index::new(&options).map_err(vector_error)?;
            let path = self.path(&key);
            if path.exists() {
                index
                    .load(path_to_string(&path)?.as_str())
                    .map_err(|error| {
                        AppError::Semantic(format!(
                            "The semantic vector index is unreadable and must be rebuilt: {error}"
                        ))
                    })?;
            }
            loaded.insert(key.clone(), index);
        }
        operation(loaded.get(&key).expect("loaded vector index"))
    }

    fn persist(&self, profile_id: &str, kind: &str, index: &Index) -> AppResult<()> {
        let key = Self::index_key(profile_id, kind)?;
        let path = self.path(&key);
        let temporary = self.root.join(format!("{key}.usearch.partial"));
        let previous = self.root.join(format!("{key}.usearch.previous"));
        index
            .save(path_to_string(&temporary)?.as_str())
            .map_err(vector_error)?;
        if previous.exists() {
            fs::remove_file(&previous).map_err(|error| {
                AppError::Semantic(format!("The old vector-index backup is locked: {error}"))
            })?;
        }
        if path.exists() {
            fs::rename(&path, &previous).map_err(|error| {
                AppError::Semantic(format!("The vector index could not be rotated: {error}"))
            })?;
        }
        if let Err(error) = fs::rename(&temporary, &path) {
            if previous.exists() {
                let _ = fs::rename(&previous, &path);
            }
            return Err(AppError::Semantic(format!(
                "The vector-index update could not be activated: {error}"
            )));
        }
        if previous.exists() {
            fs::remove_file(previous).map_err(|error| {
                AppError::Semantic(format!(
                    "The vector-index backup could not be removed: {error}"
                ))
            })?;
        }
        Ok(())
    }
}

impl VectorIndex for UsearchVectorIndex {
    fn upsert(&self, profile_id: &str, kind: &str, vectors: &[(u64, Vec<f32>)]) -> AppResult<()> {
        if vectors.is_empty() {
            return Ok(());
        }
        self.with_index(profile_id, kind, |index| {
            index
                .reserve(index.size().saturating_add(vectors.len()).max(64))
                .map_err(vector_error)?;
            for (key, vector) in vectors {
                validate_dimension(vector)?;
                if index.contains(*key) {
                    index.remove(*key).map_err(vector_error)?;
                }
                index.add(*key, vector).map_err(vector_error)?;
            }
            self.persist(profile_id, kind, index)
        })
    }

    fn remove(&self, profile_id: &str, kind: &str, keys: &[u64]) -> AppResult<()> {
        if keys.is_empty() {
            return Ok(());
        }
        self.with_index(profile_id, kind, |index| {
            let mut changed = false;
            for key in keys {
                changed |= index.remove(*key).map_err(vector_error)? > 0;
            }
            if changed {
                index.compact().map_err(vector_error)?;
                self.persist(profile_id, kind, index)?;
            }
            Ok(())
        })
    }

    fn nearest(
        &self,
        profile_id: &str,
        kind: &str,
        query: &[f32],
        limit: usize,
        allowed_keys: &HashSet<u64>,
    ) -> AppResult<Vec<VectorMatch>> {
        validate_dimension(query)?;
        if limit == 0 || allowed_keys.is_empty() {
            return Ok(Vec::new());
        }
        self.with_index(profile_id, kind, |index| {
            let matches = index
                .filtered_search(query, limit, |key| allowed_keys.contains(&key))
                .map_err(vector_error)?;
            Ok(matches
                .keys
                .into_iter()
                .zip(matches.distances)
                .map(|(key, distance)| VectorMatch {
                    key,
                    similarity: (1.0 - f64::from(distance)).clamp(-1.0, 1.0),
                })
                .collect())
        })
    }

    fn vector(&self, profile_id: &str, kind: &str, key: u64) -> AppResult<Option<Vec<f32>>> {
        self.with_index(profile_id, kind, |index| {
            if !index.contains(key) {
                return Ok(None);
            }
            let mut vector = vec![0.0_f32; EMBEDDING_DIMENSION];
            let count = index.get(key, &mut vector).map_err(vector_error)?;
            Ok((count > 0).then_some(vector))
        })
    }

    fn entry_count(&self, profile_id: &str, kind: &str) -> AppResult<u64> {
        self.with_index(profile_id, kind, |index| Ok(index.size() as u64))
    }

    fn rebuild(&self, profile_id: &str) -> AppResult<()> {
        let prefix = format!("{profile_id}-");
        self.loaded
            .lock()
            .map_err(|_| AppError::Semantic("The vector-index lock was poisoned".to_owned()))?
            .retain(|key, _| !key.starts_with(&prefix));
        for kind in ["paper", "chunk"] {
            let key = Self::index_key(profile_id, kind)?;
            for path in [
                self.path(&key),
                self.root.join(format!("{key}.usearch.partial")),
                self.root.join(format!("{key}.usearch.previous")),
            ] {
                if path.exists() {
                    fs::remove_file(path).map_err(|error| {
                        AppError::Semantic(format!(
                            "The derived vector index could not be rebuilt: {error}"
                        ))
                    })?;
                }
            }
        }
        Ok(())
    }

    fn storage_bytes(&self, profile_id: &str) -> u64 {
        ["paper", "chunk"]
            .into_iter()
            .filter_map(|kind| Self::index_key(profile_id, kind).ok())
            .map(|key| self.path(&key))
            .filter_map(|path| path.metadata().ok().map(|metadata| metadata.len()))
            .sum()
    }
}

fn vector_error(error: impl std::fmt::Display) -> AppError {
    AppError::Semantic(format!("The embedded vector index failed: {error}"))
}

fn path_to_string(path: &Path) -> AppResult<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| AppError::Semantic("The vector-index path is not valid UTF-8".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{UsearchVectorIndex, VectorIndex};
    use crate::semantic::EMBEDDING_DIMENSION;

    #[test]
    fn vector_index_identity_rejects_unsafe_paths_and_unknown_kinds() {
        assert!(UsearchVectorIndex::index_key("../profile", "paper").is_err());
        assert!(UsearchVectorIndex::index_key("abc123", "documents").is_err());
        assert_eq!(
            UsearchVectorIndex::index_key("abc123", "chunk").expect("valid identity"),
            "abc123-chunk"
        );
    }

    #[test]
    fn physical_entry_count_tracks_persisted_vector_entries() {
        let root = std::env::temp_dir().join(format!(
            "graphite-vector-count-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let index = UsearchVectorIndex::new(root.clone()).expect("vector index");
        let vectors = vec![
            (11, vec![0.1_f32; EMBEDDING_DIMENSION]),
            (12, vec![0.2_f32; EMBEDDING_DIMENSION]),
        ];
        index
            .upsert("abc123", "chunk", &vectors)
            .expect("insert vectors");
        assert_eq!(index.entry_count("abc123", "chunk").expect("count"), 2);

        index
            .remove("abc123", "chunk", &[11])
            .expect("remove vector");
        assert_eq!(index.entry_count("abc123", "chunk").expect("count"), 1);
        drop(index);
        std::fs::remove_dir_all(root).expect("remove vector fixture");
    }
}
