//! One helper at a time, and smooth updates.
//!
//! Only one helper may run (they'd fight over the same files). When a newer helper starts while an
//! older one is running, it asks the old one to quit (by creating quit.request in the app folder),
//! waits for it to let go of the lock, and takes over. An older helper never replaces a newer one.
//! Compatible with the Python helper (0.5+): same lock file, same running.json and quit.request.

use crate::paths::AppPaths;
use std::fs::{File, OpenOptions};
use std::time::{Duration, Instant};

/// Up to three numbers from a version string ("0.5.1" -> [0, 5, 1]).
pub fn vtuple(v: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in v.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(cur.parse().unwrap_or(0));
            cur.clear();
            if out.len() == 3 {
                break;
            }
        }
    }
    out
}

/// Holds an exclusive lock on helper.lock for as long as it lives (or until release()).
pub struct SingleInstance {
    path: std::path::PathBuf,
    fh: Option<File>,
}

impl SingleInstance {
    pub fn new(paths: &AppPaths) -> Self {
        Self {
            path: paths.lock_file(),
            fh: None,
        }
    }

    pub fn acquire(&mut self) -> bool {
        if self.fh.is_none() {
            match OpenOptions::new()
                .create(true)
                .append(true)
                .read(true)
                .open(&self.path)
            {
                Ok(f) => self.fh = Some(f),
                Err(_) => return false,
            }
        }
        self.fh.as_ref().is_some_and(|f| f.try_lock().is_ok())
    }

    pub fn release(&mut self) {
        if let Some(f) = self.fh.take() {
            let _ = f.unlock();
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        self.release();
    }
}

pub fn write_running(paths: &AppPaths, version: &str) {
    let body =
        serde_json::json!({"version": version, "pid": std::process::id(), "t": crate::now()});
    let _ = std::fs::write(paths.running_file(), body.to_string());
}

pub fn running_version(paths: &AppPaths) -> Option<String> {
    let bytes = std::fs::read(paths.running_file()).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("version")?.as_str().map(String::from)
}

pub fn quit_requested(paths: &AppPaths) -> bool {
    paths.quit_file().exists()
}

pub fn clear_quit_request(paths: &AppPaths) {
    let _ = std::fs::remove_file(paths.quit_file());
}

/// Get the lock, asking an older running helper to quit if needed.
///
/// Returns (got_lock, version_of_the_other_helper_or_None).
pub fn take_over(
    paths: &AppPaths,
    lock: &mut SingleInstance,
    my_version: &str,
    timeout: Duration,
) -> (bool, Option<String>) {
    if lock.acquire() {
        clear_quit_request(paths);
        return (true, None);
    }
    let other = running_version(paths);
    if let Some(o) = &other
        && vtuple(o) > vtuple(my_version)
    {
        return (false, other); // never replace a newer helper
    }
    if std::fs::write(paths.quit_file(), my_version).is_err() {
        return (false, other);
    }
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
        if lock.acquire() {
            clear_quit_request(paths);
            return (true, other);
        }
    }
    clear_quit_request(paths);
    (false, other)
}
