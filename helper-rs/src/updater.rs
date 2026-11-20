//! Updating the helper itself from GitHub releases.
//!
//! A minute after starting, and then every 12 hours, the helper asks GitHub for this project's
//! latest release (`check`). When that has a newer helper, and WoW isn't running, the service
//! downloads SevenTVEmotesHelper.exe from it, checks the download against the size and SHA-256 that
//! GitHub lists for the file (`verify`), puts it in place of its own exe (`swap_in`; Windows lets
//! a running exe be renamed but not overwritten) and starts it. The new helper takes over through
//! `instance::take_over` like any newer helper, then deletes the old exe (`clean_up`). If the new
//! helper doesn't take over within a minute, the old one puts its exe back (`roll_back`) and skips
//! that version.

use crate::instance::vtuple;
use crate::seventv::{ApiError, Http};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const LATEST_URL: &str = "https://api.github.com/repos/Neekocat1/wow-7tv-addon/releases/latest";
/// Where the exe may come from (a download link of one of this project's releases).
pub const DOWNLOAD_PREFIX: &str = "https://github.com/Neekocat1/wow-7tv-addon/releases/download/";
pub const RELEASES_PAGE: &str = "https://github.com/Neekocat1/wow-7tv-addon/releases";
pub const ASSET: &str = "SevenTVEmotesHelper.exe";
pub const FIRST_CHECK_SECONDS: f64 = 60.0;
pub const CHECK_SECONDS: f64 = 12.0 * 3600.0;
/// While WoW is open, look again this often (the update waits until it's closed).
pub const WAIT_FOR_GAME_SECONDS: f64 = 5.0 * 60.0;
/// How long the new helper gets to take over before the old one rolls back.
pub const TAKEOVER_SECONDS: u64 = 60;

/// A newer helper on GitHub.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// The helper in a `releases/latest` answer from the GitHub API.
pub fn parse_latest(j: &Value) -> Result<Release, String> {
    let tag = j["tag_name"]
        .as_str()
        .ok_or("GitHub's answer has no release tag")?;
    if j["draft"].as_bool() == Some(true) || j["prerelease"].as_bool() == Some(true) {
        return Err(format!("release {tag} isn't a final release"));
    }
    let asset = j["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["name"].as_str() == Some(ASSET)))
        .ok_or_else(|| format!("release {tag} has no {ASSET}"))?;
    let url = asset["browser_download_url"].as_str().unwrap_or_default();
    if !url.starts_with(DOWNLOAD_PREFIX) {
        return Err(format!("unexpected download link for release {tag}: {url}"));
    }
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| format!("GitHub lists no SHA-256 for {ASSET} in release {tag}"))?
        .to_ascii_lowercase();
    let size = asset["size"]
        .as_u64()
        .filter(|&s| s > 0)
        .ok_or_else(|| format!("release {tag} lists no size for {ASSET}"))?;
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        url: url.to_string(),
        size,
        sha256,
    })
}

pub fn is_newer(candidate: &str, mine: &str) -> bool {
    vtuple(candidate) > vtuple(mine)
}

/// The latest release, if it's newer than `mine`.
pub fn check(http: &dyn Http, mine: &str) -> Result<Option<Release>, String> {
    let j = http.get_json(LATEST_URL).map_err(|e| match e {
        ApiError::NotFound(_) => "no releases on GitHub yet".to_string(),
        other => other.to_string(),
    })?;
    let rel = parse_latest(&j)?;
    Ok(is_newer(&rel.version, mine).then_some(rel))
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Is `data` the file GitHub lists for this release (and a Windows program)?
pub fn verify(rel: &Release, data: &[u8]) -> Result<(), String> {
    if data.len() as u64 != rel.size {
        return Err(format!(
            "the download is {} bytes, GitHub lists {}",
            data.len(),
            rel.size
        ));
    }
    if sha256_hex(data) != rel.sha256 {
        return Err("the download doesn't match the SHA-256 GitHub lists".into());
    }
    if !data.starts_with(b"MZ") {
        return Err("the download isn't a Windows program".into());
    }
    Ok(())
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let name = exe
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ASSET.to_string());
    exe.with_file_name(format!("{name}{suffix}"))
}

/// Where the new exe is written before it's swapped in.
pub fn new_path(exe: &Path) -> PathBuf {
    sibling(exe, ".new")
}

/// Where the replaced exe goes (deleted by the next helper).
pub fn old_path(exe: &Path) -> PathBuf {
    sibling(exe, ".old")
}

/// Put `data` in place of `exe`, which may be running: the old exe moves to `old_path`. If anything
/// fails, `exe` is left as it was.
pub fn swap_in(exe: &Path, data: &[u8]) -> Result<(), String> {
    let (new, old) = (new_path(exe), old_path(exe));
    let folder = exe.parent().unwrap_or(exe).display().to_string();
    std::fs::write(&new, data).map_err(|e| format!("couldn't save it in {folder} ({e})"))?;
    if old.exists() && std::fs::remove_file(&old).is_err() {
        let _ = std::fs::remove_file(&new);
        return Err(format!(
            "{} from an earlier update is still in use",
            old.display()
        ));
    }
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(format!("couldn't move the current helper aside ({e})"));
    }
    if let Err(e) = std::fs::rename(&new, exe) {
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&new);
        return Err(format!("couldn't put the new helper in place ({e})"));
    }
    Ok(())
}

/// Undo `swap_in`: the old exe goes back (the new one is deleted, or kept as `.new` if it's
/// still running).
pub fn roll_back(exe: &Path) -> Result<(), String> {
    let (new, old) = (new_path(exe), old_path(exe));
    if !old.exists() {
        return Err("the previous helper isn't there any more".into());
    }
    let _ = std::fs::remove_file(&new);
    std::fs::rename(exe, &new).map_err(|e| format!("couldn't move the new helper aside ({e})"))?;
    std::fs::rename(&old, exe).map_err(|e| format!("couldn't put the old helper back ({e})"))?;
    let _ = std::fs::remove_file(&new);
    Ok(())
}

/// Delete what an earlier update left next to `exe`. The old exe can only go once it has exited,
/// so this tries a few times.
pub fn clean_up(exe: &Path) {
    for path in [old_path(exe), new_path(exe)] {
        for attempt in 0..10 {
            if !path.exists() || std::fs::remove_file(&path).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500 * (attempt + 1)));
        }
    }
}

/// Start the helper at `exe` in the background (it takes over from this one).
pub fn launch(exe: &Path) -> std::io::Result<()> {
    std::process::Command::new(exe)
        .args(["--background", "--updated"])
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn latest(tag: &str, data: &[u8]) -> Value {
        json!({
            "tag_name": tag, "draft": false, "prerelease": false,
            "assets": [
                {"name": "notes.txt", "browser_download_url": format!("{DOWNLOAD_PREFIX}{tag}/notes.txt")},
                {"name": ASSET, "size": data.len(), "digest": format!("sha256:{}", sha256_hex(data)),
                 "browser_download_url": format!("{DOWNLOAD_PREFIX}{tag}/{ASSET}")},
            ]
        })
    }

    #[test]
    fn reads_the_latest_release() {
        let exe = b"MZ fake program";
        let rel = parse_latest(&latest("v0.8.0", exe)).unwrap();
        assert_eq!(rel.version, "0.8.0");
        assert_eq!(rel.size, exe.len() as u64);
        assert!(rel.url.ends_with("/v0.8.0/SevenTVEmotesHelper.exe"));
        assert!(verify(&rel, exe).is_ok());
        assert!(verify(&rel, b"MZ fake programme").is_err(), "size");
        assert!(verify(&rel, b"MZ fake progran").is_err(), "hash");
        assert!(
            is_newer("0.8.0", "0.7.2")
                && !is_newer("0.7.2", "0.7.2")
                && !is_newer("0.7.1", "0.7.2")
        );
        assert!(is_newer("0.10.0", "0.9.9"));

        let mut j = latest("v0.8.0", exe);
        j["assets"][1]["digest"] = Value::Null;
        assert!(parse_latest(&j).unwrap_err().contains("SHA-256"));
        let mut j = latest("v0.8.0", exe);
        j["assets"][1]["browser_download_url"] =
            json!("https://example.com/SevenTVEmotesHelper.exe");
        assert!(parse_latest(&j).is_err(), "only this project's downloads");
        let mut j = latest("v0.8.0", exe);
        j["prerelease"] = json!(true);
        assert!(parse_latest(&j).is_err());
        let mut j = latest("v0.8.0", exe);
        j["assets"].as_array_mut().unwrap().pop();
        assert!(parse_latest(&j).unwrap_err().contains("has no"));
    }

    #[test]
    fn swaps_rolls_back_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("seventv-updater-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join(ASSET);
        std::fs::write(&exe, b"old").unwrap();
        std::fs::write(old_path(&exe), b"older").unwrap(); // left over from an earlier update

        swap_in(&exe, b"new").unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"old");
        assert!(!new_path(&exe).exists());

        roll_back(&exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        assert!(!old_path(&exe).exists() && !new_path(&exe).exists());

        swap_in(&exe, b"new").unwrap();
        clean_up(&exe);
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert!(!old_path(&exe).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
