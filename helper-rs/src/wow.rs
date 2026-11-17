//! Finding World of Warcraft installs, game folders and whether the game is running.

use std::path::{Path, PathBuf};

pub const CORE_ADDON: &str = "SevenTVEmotes";
pub const LOCAL_ADDON: &str = "SevenTVEmotes_Local";
pub const SV_FILE: &str = "SevenTVEmotes.lua";

/// Game folders ("flavors") we install into by default. The Forever folder was
/// _classic_beta_ during the beta and _forever_ at launch.
pub const DEFAULT_FLAVOR_HINTS: &[&str] = &["forever"];

#[cfg(windows)]
fn registry_paths() -> Vec<PathBuf> {
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY, RegGetValueW,
    };
    let keys = [
        "SOFTWARE\\WOW6432Node\\Blizzard Entertainment\\World of Warcraft",
        "SOFTWARE\\Blizzard Entertainment\\World of Warcraft",
    ];
    let mut out = Vec::new();
    for key in keys {
        for flag in [0, RRF_SUBKEY_WOW6464KEY] {
            let k = crate::win::wide(key);
            let v = crate::win::wide("InstallPath");
            let mut buf = vec![0u16; 1024];
            let mut len: u32 = (buf.len() * 2) as u32;
            // SAFETY: buffers are valid and sized; RegGetValueW writes at most `len` bytes.
            let rc = unsafe {
                RegGetValueW(
                    HKEY_LOCAL_MACHINE,
                    k.as_ptr(),
                    v.as_ptr(),
                    RRF_RT_REG_SZ | flag,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr().cast(),
                    &mut len,
                )
            };
            if rc == 0 {
                let n = (len as usize / 2).saturating_sub(1);
                let s = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
                let s = s.trim_end_matches('\0').to_string();
                if !s.is_empty() {
                    out.push(PathBuf::from(s));
                }
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn registry_paths() -> Vec<PathBuf> {
    Vec::new()
}

fn common_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        for d in b'A'..=b'Z' {
            let drive = format!("{}:\\", d as char);
            if !Path::new(&drive).exists() {
                continue;
            }
            for sub in [
                "Program Files (x86)",
                "Program Files",
                "Games",
                "Battle.net",
                "",
            ] {
                let mut p = PathBuf::from(&drive);
                if !sub.is_empty() {
                    p.push(sub);
                }
                p.push("World of Warcraft");
                out.push(p);
            }
        }
    } else if cfg!(target_os = "macos") {
        out.push(PathBuf::from("/Applications/World of Warcraft"));
        out.push(crate::paths::home().join("Applications/World of Warcraft"));
    }
    out
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn is_flavor_name(name: &str) -> bool {
    name.starts_with('_') && name.ends_with('_')
}

/// canonicalize without Windows' `\\?\` prefix (WoW and the user never see those).
pub fn resolve(p: &Path) -> std::io::Result<PathBuf> {
    let c = std::fs::canonicalize(p)?;
    let s = c.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return Ok(PathBuf::from(format!(r"\\{rest}")));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return Ok(PathBuf::from(rest));
    }
    Ok(c)
}

pub fn is_flavor_dir(p: &Path) -> bool {
    if !p.is_dir() || !is_flavor_name(&file_name(p)) {
        return false;
    }
    if p.join("Interface").is_dir() || p.join("WTF").is_dir() {
        return true;
    }
    std::fs::read_dir(p).is_ok_and(|rd| {
        rd.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().to_lowercase();
            (n.starts_with("wow") && n.ends_with(".exe"))
                || (n.starts_with("world of warcraft") && n.ends_with(".app"))
        })
    })
}

/// Folders like C:/Program Files (x86)/World of Warcraft (the parent of _retail_, _forever_, ...).
pub fn find_wow_roots(extra: &[String]) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut roots = Vec::new();
    let candidates: Vec<PathBuf> = extra
        .iter()
        .map(PathBuf::from)
        .chain(registry_paths())
        .chain(common_paths())
        .collect();
    for p in candidates {
        let mut p = resolve(&p).unwrap_or(p);
        if is_flavor_name(&file_name(&p)) {
            // registry often points at _retail_
            if let Some(parent) = p.parent() {
                p = parent.to_path_buf();
            }
        }
        let Ok(p) = resolve(&p) else { continue };
        let key = p.to_string_lossy().to_lowercase();
        if seen.contains(&key) || !p.is_dir() {
            continue;
        }
        seen.insert(key);
        let has_flavor =
            std::fs::read_dir(&p).is_ok_and(|rd| rd.flatten().any(|e| is_flavor_dir(&e.path())));
        if has_flavor {
            roots.push(p);
        }
    }
    roots
}

pub fn flavor_dirs(root: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| is_flavor_dir(p))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// Flavor folders to manage: ones matching the hints, ones named in `also`, and any
/// that already have the addon installed.
pub fn pick_flavors(roots: &[PathBuf], hints: &[&str], also: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in roots {
        for f in flavor_dirs(root) {
            let name = file_name(&f);
            let lname = name.to_lowercase();
            let installed = f.join("Interface").join("AddOns").join(CORE_ADDON).is_dir();
            if installed || hints.iter().any(|h| lname.contains(h)) || also.contains(&name) {
                out.push(f);
            }
        }
    }
    out
}

pub fn addons_dir(flavor: &Path) -> PathBuf {
    flavor.join("Interface").join("AddOns")
}

/// Account-wide SavedVariables for the addon, one per WoW account.
pub fn saved_variable_files(flavor: &Path) -> Vec<PathBuf> {
    let wtf = flavor.join("WTF").join("Account");
    let Ok(rd) = std::fs::read_dir(&wtf) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path().join("SavedVariables").join(SV_FILE))
        .filter(|p| p.is_file())
        .collect();
    out.sort();
    out
}

// --------------------------------------------------------------------------- game process

#[derive(Debug, Clone, PartialEq)]
pub struct GameProc {
    pub pid: u32,
    pub name: String,
    pub exe: String,
    pub started: Option<f64>,
}

/// `^(wow|world ?of ?warcraft|forever)[^\\/]*?(\.exe)?$`, case-insensitive.
pub fn looks_like_game(name: &str) -> bool {
    let l = name.to_lowercase();
    if l.contains('/') || l.contains('\\') {
        return false;
    }
    if l.starts_with("wow") || l.starts_with("forever") {
        return true;
    }
    let Some(rest) = l.strip_prefix("world") else {
        return false;
    };
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    let Some(rest) = rest.strip_prefix("of") else {
        return false;
    };
    let rest = rest.strip_prefix(' ').unwrap_or(rest);
    rest.starts_with("warcraft")
}

const NOT_GAME: &[&str] = &[
    "battle.net",
    "agent",
    "blizzard",
    "wowup",
    "warcraftlogs",
    "raider",
    "tsm",
    "curseforge",
];

/// Picks WoW clients out of a process list.
pub fn filter_game_processes(
    procs: impl IntoIterator<Item = GameProc>,
    roots: &[PathBuf],
) -> Vec<GameProc> {
    let root_strs: Vec<String> = roots
        .iter()
        .map(|r| {
            r.to_string_lossy()
                .to_lowercase()
                .trim_end_matches(['\\', '/'])
                .to_string()
        })
        .collect();
    let mut found = Vec::new();
    for p in procs {
        let lname = p.name.to_lowercase();
        if NOT_GAME.iter().any(|n| lname.starts_with(n)) {
            continue;
        }
        let exe = p.exe.to_lowercase();
        let in_root = !exe.is_empty()
            && root_strs
                .iter()
                .any(|r| exe.starts_with(&format!("{r}\\")) || exe.starts_with(&format!("{r}/")));
        if !(in_root || looks_like_game(&p.name)) {
            continue;
        }
        found.push(p);
    }
    found
}

/// Where process information comes from (replaceable in tests).
pub trait ProcessSource: Send {
    /// Running WoW clients, or None when process information isn't available.
    fn game_processes(&mut self, roots: &[PathBuf]) -> Option<Vec<GameProc>>;
}

impl<F> ProcessSource for F
where
    F: FnMut(&[PathBuf]) -> Option<Vec<GameProc>> + Send,
{
    fn game_processes(&mut self, roots: &[PathBuf]) -> Option<Vec<GameProc>> {
        self(roots)
    }
}

/// The real process list, via sysinfo.
pub struct SystemProcesses {
    sys: Option<sysinfo::System>,
}

impl Default for SystemProcesses {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemProcesses {
    pub fn new() -> Self {
        Self {
            sys: sysinfo::System::new_with_specifics(sysinfo::RefreshKind::nothing()).ok(),
        }
    }
}

impl ProcessSource for SystemProcesses {
    fn game_processes(&mut self, roots: &[PathBuf]) -> Option<Vec<GameProc>> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
        let sys = self.sys.as_mut()?;
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
        );
        let all = sys.processes().values().map(|p| GameProc {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().into_owned(),
            exe: p
                .exe()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_default(),
            started: Some(p.start_time() as f64).filter(|&t| t > 0.0),
        });
        let mut found = filter_game_processes(all, roots);
        found.sort_by_key(|p| p.pid);
        Some(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_names() {
        for n in [
            "Wow.exe",
            "WowForever.exe",
            "WoWClassic.exe",
            "World of Warcraft",
            "WorldOfWarcraft.exe",
            "Forever.exe",
        ] {
            assert!(looks_like_game(n), "{n}");
        }
        for n in ["explorer.exe", "a/wow.exe", "Worldof", "notwow.exe"] {
            assert!(!looks_like_game(n), "{n}");
        }
        let p = |name: &str, exe: &str| GameProc {
            pid: 1,
            name: name.into(),
            exe: exe.into(),
            started: None,
        };
        let roots = vec![PathBuf::from("E:\\Games\\World of Warcraft")];
        let got = filter_game_processes(
            vec![
                p("Battle.net.exe", ""),
                p("Agent.exe", "e:\\games\\world of warcraft\\agent.exe"),
                p(
                    "Custom.exe",
                    "E:\\Games\\World of Warcraft\\_forever_\\Custom.exe",
                ),
                p("WowForever.exe", ""),
                p("chrome.exe", ""),
            ],
            &roots,
        );
        let names: Vec<_> = got.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Custom.exe", "WowForever.exe"]);
    }
}
