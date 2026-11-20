//! Where the helper keeps its own files, and its config.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "SevenTVEmotesHelper";

/// The helper's own folder (%LOCALAPPDATA%\SevenTVEmotesHelper on Windows): config, log,
/// download cache, lock and hand-over files. Passed around explicitly so tests can use a temp dir.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub dir: PathBuf,
}

impl AppPaths {
    pub fn default_dir() -> PathBuf {
        let base = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home().join("AppData").join("Local"))
        } else if cfg!(target_os = "macos") {
            home().join("Library").join("Application Support")
        } else {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home().join(".local").join("share"))
        };
        base.join(APP_NAME)
    }

    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.dir.join("cache")
    }
    pub fn log_file(&self) -> PathBuf {
        self.dir.join("helper.log")
    }
    pub fn config_file(&self) -> PathBuf {
        self.dir.join("config.json")
    }
    pub fn lock_file(&self) -> PathBuf {
        self.dir.join("helper.lock")
    }
    pub fn running_file(&self) -> PathBuf {
        self.dir.join("running.json")
    }
    pub fn quit_file(&self) -> PathBuf {
        self.dir.join("quit.request")
    }
}

pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// extra World of Warcraft folders to look in
    #[serde(default)]
    pub wow_dirs: Vec<String>,
    /// extra game folders to manage besides Forever, e.g. "_retail_"
    #[serde(default)]
    pub flavors: Vec<String>,
    #[serde(default)]
    pub first_run_done: bool,
    /// Install new helper releases from GitHub by itself (tray: "Update automatically").
    #[serde(default = "yes")]
    pub auto_update: bool,
    /// A release that failed to start: not installed again automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_update: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            wow_dirs: Vec::new(),
            flavors: Vec::new(),
            first_run_done: false,
            auto_update: true,
            skip_update: None,
            extra: Map::new(),
        }
    }
}

impl Config {
    pub fn load(paths: &AppPaths) -> Config {
        std::fs::read(paths.config_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, paths: &AppPaths) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into());
        std::fs::write(paths.config_file(), text)
    }
}

/// Append items not already present, keeping order (`list(dict.fromkeys(a + b))`).
pub fn merge_unique(list: &mut Vec<String>, more: &[String]) {
    for m in more {
        if !list.contains(m) {
            list.push(m.clone());
        }
    }
}

pub fn file_mtime(p: &Path) -> Option<f64> {
    let m = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(crate::system_time_secs(m))
}
