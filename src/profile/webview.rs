#![allow(dead_code)]
// Webview context isolation per profile.
//
// Each profile gets its own wry::WebContext pointing to <profile_dir>/webdata/
// for full cookie + localStorage isolation.

use std::path::{Path, PathBuf};

/// Manages per-profile webview contexts.
pub struct ProfileWebviewManager {
    /// Directory containing WebContext data.
    pub data_dir: PathBuf,
}

impl ProfileWebviewManager {
    /// Create a new webview manager for the given profile data directory.
    pub fn new(data_dir: &Path) -> Self {
        // Ensure the directory exists.
        if let Err(e) = std::fs::create_dir_all(data_dir) {
            log::warn!("Failed to create webdata dir {}: {}", data_dir.display(), e);
        }
        Self {
            data_dir: data_dir.to_path_buf(),
        }
    }

    /// Get the data directory path.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}
