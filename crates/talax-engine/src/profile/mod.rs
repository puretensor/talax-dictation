//! Voice profile management.
//!
//! Each profile is a directory containing:
//! - corrections.db (SQLite)
//! - ngram.bin (bincode-serialized n-gram model)
//! - domain_context.json (vocabulary)
//! - profile.toml (metadata)

use std::path::{Path, PathBuf};

use crate::db::Database;
use crate::pipeline::ngram_corrector::NgramCorrector;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProfileMetadata {
    pub name: String,
    pub created: String,
    pub language: String,
    pub sessions_count: u64,
    pub patterns_count: u64,
    pub last_used: String,
}

impl Default for ProfileMetadata {
    fn default() -> Self {
        let now = chrono_now();
        Self {
            name: "default".to_string(),
            created: now.clone(),
            language: "en".to_string(),
            sessions_count: 0,
            patterns_count: 0,
            last_used: now,
        }
    }
}

pub struct ProfileManager {
    base_dir: PathBuf,
    active_profile: Option<String>,
}

impl ProfileManager {
    pub fn new(base_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&base_dir).ok();
        Self {
            base_dir,
            active_profile: None,
        }
    }

    /// List all available profile names.
    pub fn list_profiles(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.base_dir) else {
            return Vec::new();
        };
        entries
            .filter_map(|e| {
                let e = e.ok()?;
                if e.file_type().ok()?.is_dir() {
                    Some(e.file_name().to_string_lossy().to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Create a new profile. Returns the profile directory path.
    pub fn create_profile(&self, name: &str) -> Result<PathBuf, String> {
        let dir = self.base_dir.join(name);
        if dir.exists() {
            return Err(format!("Profile '{name}' already exists"));
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

        // Create empty database
        let db_path = dir.join("corrections.db");
        Database::open(&db_path).map_err(|e| e.to_string())?;

        // Write default metadata
        let meta = ProfileMetadata {
            name: name.to_string(),
            ..Default::default()
        };
        let toml_str = toml_serialize(&meta);
        std::fs::write(dir.join("profile.toml"), toml_str).map_err(|e| e.to_string())?;

        // Empty domain context
        std::fs::write(
            dir.join("domain_context.json"),
            r#"{"all_proper_nouns": [], "known_accent_patterns": {}}"#,
        )
        .map_err(|e| e.to_string())?;

        Ok(dir)
    }

    /// Delete a profile.
    pub fn delete_profile(&self, name: &str) -> Result<(), String> {
        let dir = self.base_dir.join(name);
        if !dir.exists() {
            return Err(format!("Profile '{name}' does not exist"));
        }
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
    }

    /// Reset a profile (delete and recreate with empty data).
    pub fn reset_profile(&self, name: &str) -> Result<PathBuf, String> {
        self.delete_profile(name)?;
        self.create_profile(name)
    }

    /// Clone a profile to a new name.
    pub fn clone_profile(&self, source: &str, target: &str) -> Result<PathBuf, String> {
        let src = self.base_dir.join(source);
        let dst = self.base_dir.join(target);
        if !src.exists() {
            return Err(format!("Source profile '{source}' does not exist"));
        }
        if dst.exists() {
            return Err(format!("Target profile '{target}' already exists"));
        }
        copy_dir_recursive(&src, &dst).map_err(|e| e.to_string())?;

        // Update the name in profile.toml
        let toml_path = dst.join("profile.toml");
        if let Ok(contents) = std::fs::read_to_string(&toml_path) {
            let updated = contents.replace(
                &format!("name = \"{source}\""),
                &format!("name = \"{target}\""),
            );
            std::fs::write(&toml_path, updated).ok();
        }

        Ok(dst)
    }

    /// Get the profile directory path.
    pub fn profile_path(&self, name: &str) -> PathBuf {
        self.base_dir.join(name)
    }

    /// Get active profile name.
    pub fn active(&self) -> Option<&str> {
        self.active_profile.as_deref()
    }

    /// Set active profile.
    pub fn set_active(&mut self, name: &str) {
        self.active_profile = Some(name.to_string());
    }

    /// Open the database for a profile.
    pub fn open_db(&self, name: &str) -> Result<Database, String> {
        let path = self.base_dir.join(name).join("corrections.db");
        Database::open(&path).map_err(|e| e.to_string())
    }

    /// Load the n-gram model for a profile.
    pub fn load_ngram(&self, name: &str) -> NgramCorrector {
        let path = self.base_dir.join(name).join("ngram.bin");
        NgramCorrector::with_model_path(path)
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn chrono_now() -> String {
    // Simple ISO 8601 timestamp without chrono dependency
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}Z", d.as_secs())
}

fn toml_serialize(meta: &ProfileMetadata) -> String {
    format!(
        "name = \"{}\"\ncreated = \"{}\"\nlanguage = \"{}\"\nsessions_count = {}\npatterns_count = {}\nlast_used = \"{}\"",
        meta.name,
        meta.created,
        meta.language,
        meta.sessions_count,
        meta.patterns_count,
        meta.last_used
    )
}
