//! Download manager for Whisper GGML model files from HuggingFace.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("unknown model: {0}")]
    UnknownModel(String),
    #[error("model not downloaded: {0}")]
    NotDownloaded(String),
    #[error("download failed: {0}")]
    Download(String),
    #[error("size mismatch for {name}: expected {expected}, got {actual}")]
    SizeMismatch {
        name: String,
        expected: u64,
        actual: u64,
    },
    #[error("checksum mismatch for {name}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        name: String,
        expected: String,
        actual: String,
    },
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Model catalogue
// ---------------------------------------------------------------------------

const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

struct ModelSpec {
    name: &'static str,
    filename: &'static str,
    size_bytes: u64,
    sha256: &'static str,
    description: &'static str,
}

const MODELS: &[ModelSpec] = &[
    ModelSpec {
        name: "tiny.en",
        filename: "ggml-tiny.en.bin",
        size_bytes: 77_704_715,
        sha256: "0d686a2a6a22b02da2ef3101d4c86e68461363a623c58f27f81b1b2d36b42317",
        description: "Tiny English-only (~75 MB) - fastest, lowest accuracy",
    },
    ModelSpec {
        name: "base.en",
        filename: "ggml-base.en.bin",
        size_bytes: 147_964_211,
        sha256: "ff7d10f8526045d48149699b43aeaa014e4b337239bc5a35251116fc179aabcf",
        description: "Base English-only (~142 MB) - fast, moderate accuracy",
    },
    ModelSpec {
        name: "small.en",
        filename: "ggml-small.en.bin",
        size_bytes: 487_614_201,
        sha256: "0d57184d34ae7d736e5bb2db5bf83debe730bd53dcefa235a0979b9dcfd33fb3",
        description: "Small English-only (~466 MB) - balanced speed/accuracy",
    },
    ModelSpec {
        name: "small.en-q5_1",
        filename: "ggml-small.en-q5_1.bin",
        size_bytes: 190_098_681,
        sha256: "ba5733534a74f94f8f53afadda9dcb21d029f015065399bb22e72d8cc4bc9ced",
        description: "Small English-only Q5_1 quantised (~181 MB) - recommended default",
    },
    ModelSpec {
        name: "medium.en-q5_0",
        filename: "ggml-medium.en-q5_0.bin",
        size_bytes: 539_225_533,
        sha256: "5ce4bb290d6d5b998951eea06404a8a5c89c6ff1eec7f52bb326c4b2de45a3b3",
        description: "Medium English-only Q5_0 quantised (~515 MB) - high accuracy",
    },
    ModelSpec {
        name: "large-v3-turbo-q5_0",
        filename: "ggml-large-v3-turbo-q5_0.bin",
        size_bytes: 574_041_195,
        sha256: "9c7b9c6bf60cf555f34fe7d81e8643764ff03d2f60b6fa550f5630be52eef830",
        description: "Large-v3 Turbo Q5_0 quantised (~574 MB) - highest accuracy, multilingual",
    },
];

const DEFAULT_MODEL: &str = "small.en-q5_1";

fn find_spec(name: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.name == name)
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Metadata for a model visible to the UI layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub filename: String,
    pub url: String,
    pub size_bytes: u64,
    pub downloaded: bool,
    pub description: String,
}

// ---------------------------------------------------------------------------
// ModelManager
// ---------------------------------------------------------------------------

/// Manages Whisper GGML model files on disk.
pub struct ModelManager {
    models_dir: PathBuf,
}

impl ModelManager {
    /// Create a new manager. `models_dir` will be created if it does not exist.
    pub fn new(models_dir: PathBuf) -> Result<Self, ModelError> {
        fs::create_dir_all(&models_dir)?;
        let manager = Self { models_dir };
        manager.cleanup_partial_downloads()?;
        Ok(manager)
    }

    /// Return the default model name.
    pub fn default_model() -> &'static str {
        DEFAULT_MODEL
    }

    /// List every model in the catalogue, annotated with download status.
    pub fn list_available(&self) -> Vec<ModelInfo> {
        MODELS
            .iter()
            .map(|spec| {
                let downloaded = self.model_file_path(spec).exists();
                ModelInfo {
                    name: spec.name.to_string(),
                    filename: spec.filename.to_string(),
                    url: format!("{}/{}", HF_BASE, spec.filename),
                    size_bytes: spec.size_bytes,
                    downloaded,
                    description: spec.description.to_string(),
                }
            })
            .collect()
    }

    /// List the paths of models that are already on disk.
    pub fn list_downloaded(&self) -> Vec<PathBuf> {
        MODELS
            .iter()
            .map(|spec| self.model_file_path(spec))
            .filter(|p| p.exists())
            .collect()
    }

    /// Check whether a given model has been downloaded.
    pub fn is_downloaded(&self, model_name: &str) -> bool {
        find_spec(model_name)
            .map(|spec| self.model_file_path(spec).exists())
            .unwrap_or(false)
    }

    /// Return the on-disk path for a model.
    pub fn get_model_path(&self, model_name: &str) -> Option<PathBuf> {
        find_spec(model_name).map(|spec| self.model_file_path(spec))
    }

    /// Download a model from HuggingFace.
    pub fn download(
        &self,
        model_name: &str,
        progress_callback: impl Fn(u64, u64),
    ) -> Result<PathBuf, ModelError> {
        let spec = find_spec(model_name)
            .ok_or_else(|| ModelError::UnknownModel(model_name.to_string()))?;

        let url = format!("{}/{}", HF_BASE, spec.filename);
        let dest = self.model_file_path(spec);
        let tmp_dest = self.temp_model_file_path(spec);

        tracing::info!(model = spec.name, %url, "starting model download");

        if tmp_dest.exists() {
            let _ = fs::remove_file(&tmp_dest);
        }

        let response = ureq::get(&url)
            .call()
            .map_err(|e| ModelError::Download(e.to_string()))?;

        let content_length: u64 = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(spec.size_bytes);

        let total = if content_length > 0 {
            content_length
        } else {
            spec.size_bytes
        };

        let mut reader = response.into_body().into_reader();
        let mut file = fs::File::create(&tmp_dest)?;
        let mut hasher = Sha256::new();

        let mut downloaded: u64 = 0;
        let mut buf = vec![0u8; 256 * 1024];

        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            downloaded += n as u64;
            progress_callback(downloaded, total);
        }

        file.flush()?;
        drop(file);

        let actual = fs::metadata(&tmp_dest)?.len();
        if actual != spec.size_bytes {
            let _ = fs::remove_file(&tmp_dest);
            return Err(ModelError::SizeMismatch {
                name: spec.name.to_string(),
                expected: spec.size_bytes,
                actual,
            });
        }

        let actual_hash = format!("{:x}", hasher.finalize());
        if actual_hash != spec.sha256 {
            let _ = fs::remove_file(&tmp_dest);
            return Err(ModelError::ChecksumMismatch {
                name: spec.name.to_string(),
                expected: spec.sha256.to_string(),
                actual: actual_hash,
            });
        }

        fs::rename(&tmp_dest, &dest)?;
        tracing::info!(model = spec.name, bytes = actual, "download complete");

        Ok(dest)
    }

    /// Delete a downloaded model from disk.
    pub fn delete_model(&self, model_name: &str) -> Result<(), ModelError> {
        let spec = find_spec(model_name)
            .ok_or_else(|| ModelError::UnknownModel(model_name.to_string()))?;

        let path = self.model_file_path(spec);
        if !path.exists() {
            return Err(ModelError::NotDownloaded(model_name.to_string()));
        }
        fs::remove_file(&path)?;
        Ok(())
    }

    fn model_file_path(&self, spec: &ModelSpec) -> PathBuf {
        self.models_dir.join(spec.filename)
    }

    fn temp_model_file_path(&self, spec: &ModelSpec) -> PathBuf {
        self.models_dir.join(format!("{}.part", spec.filename))
    }

    fn cleanup_partial_downloads(&self) -> Result<(), ModelError> {
        for entry in fs::read_dir(&self.models_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("part") {
                let _ = fs::remove_file(path);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join("talax-engine-tests")
            .join(format!("models-{}-{}", std::process::id(), id));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn list_available_returns_all_models() {
        let dir = tmp_dir();
        let mgr = ModelManager::new(dir.clone()).unwrap();
        let models = mgr.list_available();

        assert_eq!(models.len(), MODELS.len());

        let names: Vec<&str> = models.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"tiny.en"));
        assert!(names.contains(&"base.en"));
        assert!(names.contains(&"small.en"));
        assert!(names.contains(&"small.en-q5_1"));
        assert!(names.contains(&"medium.en-q5_0"));
        assert!(names.contains(&"large-v3-turbo-q5_0"));

        for m in &models {
            assert!(!m.downloaded, "{} should not be downloaded", m.name);
        }

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_downloaded_returns_false_for_missing() {
        let dir = tmp_dir();
        let mgr = ModelManager::new(dir.clone()).unwrap();

        assert!(!mgr.is_downloaded("tiny.en"));
        assert!(!mgr.is_downloaded("small.en-q5_1"));
        assert!(!mgr.is_downloaded("nonexistent-model"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn get_model_path_returns_correct_format() {
        let dir = tmp_dir();
        let mgr = ModelManager::new(dir.clone()).unwrap();

        let path = mgr.get_model_path("tiny.en").unwrap();
        assert_eq!(path, dir.join("ggml-tiny.en.bin"));

        let path = mgr.get_model_path("small.en-q5_1").unwrap();
        assert_eq!(path, dir.join("ggml-small.en-q5_1.bin"));

        let path = mgr.get_model_path("large-v3-turbo-q5_0").unwrap();
        assert_eq!(path, dir.join("ggml-large-v3-turbo-q5_0.bin"));

        assert!(mgr.get_model_path("nonexistent").is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_model_is_small_en_q5_1() {
        assert_eq!(ModelManager::default_model(), "small.en-q5_1");
    }

    #[test]
    fn list_downloaded_empty_initially() {
        let dir = tmp_dir();
        let mgr = ModelManager::new(dir.clone()).unwrap();
        assert!(mgr.list_downloaded().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn download_returns_error_for_unknown_model() {
        let dir = tmp_dir();
        let mgr = ModelManager::new(dir.clone()).unwrap();
        let result = mgr.download("nonexistent-model", |_, _| {});
        assert!(result.is_err());
        match result.unwrap_err() {
            ModelError::UnknownModel(name) => {
                assert_eq!(name, "nonexistent-model")
            }
            other => panic!("expected UnknownModel, got: {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_model_returns_error_when_not_present() {
        let dir = tmp_dir();
        let mgr = ModelManager::new(dir.clone()).unwrap();
        let result = mgr.delete_model("tiny.en");
        assert!(result.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_downloaded_detects_file_on_disk() {
        let dir = tmp_dir();
        let mgr = ModelManager::new(dir.clone()).unwrap();

        let path = dir.join("ggml-tiny.en.bin");
        fs::write(&path, b"fake model data").unwrap();

        assert!(mgr.is_downloaded("tiny.en"));
        assert!(!mgr.is_downloaded("base.en"));

        let downloaded = mgr.list_downloaded();
        assert_eq!(downloaded.len(), 1);
        assert_eq!(downloaded[0], path);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_urls_point_to_huggingface() {
        let dir = tmp_dir();
        let mgr = ModelManager::new(dir.clone()).unwrap();
        for info in mgr.list_available() {
            assert!(
                info.url.starts_with("https://huggingface.co/"),
                "URL for {} should point to HuggingFace: {}",
                info.name,
                info.url
            );
            assert!(
                info.url.contains(&info.filename),
                "URL for {} should contain the filename",
                info.name
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_manager_removes_partial_downloads() {
        let dir = tmp_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("ggml-small.en-q5_1.bin.part"), b"partial").unwrap();

        let _mgr = ModelManager::new(dir.clone()).unwrap();
        assert!(!dir.join("ggml-small.en-q5_1.bin.part").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn model_specs_have_sha256_metadata() {
        for spec in MODELS {
            assert_eq!(spec.sha256.len(), 64);
            assert!(spec.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }
}
