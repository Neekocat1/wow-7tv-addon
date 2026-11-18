//! The background loop: watch for in-game requests, fetch emotes, keep the library fresh.

use crate::installer;
use crate::instance;
use crate::library::{BuildResult, Library, NetworkError, Processed, Ready, ReqResult, Request};
use crate::luasv::{self, Value as Lua};
use crate::now;
use crate::paths::{AppPaths, Config, file_mtime};
use crate::seventv::Http;
use crate::wow::{self, ProcessSource};
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub const POLL_SECONDS: f64 = 2.0;
pub const HEARTBEAT_SECONDS: f64 = 5.0 * 60.0;
pub const REDISCOVER_SECONDS: f64 = 10.0 * 60.0;
pub const RETRY_SECONDS: f64 = 30.0;
pub const MAX_ATTEMPTS: u32 = 3;

type Hook = Box<dyn Fn() + Send + Sync>;
type NotifyHook = Box<dyn Fn(&str, &str) + Send + Sync>;
type StatusHook = Box<dyn Fn(&str) + Send + Sync>;

/// What the tray (UI thread) and the service thread share.
pub struct Shared {
    pub status: Mutex<String>,
    pub lib_dirs: Mutex<Vec<PathBuf>>,
    stop: AtomicBool,
    force_refresh: AtomicBool,
    wake: (Mutex<bool>, Condvar),
    pub notify: Mutex<NotifyHook>,
    pub on_status: Mutex<StatusHook>,
    pub on_quit: Mutex<Hook>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            status: Mutex::new("Starting...".into()),
            lib_dirs: Mutex::new(Vec::new()),
            stop: AtomicBool::new(false),
            force_refresh: AtomicBool::new(false),
            wake: (Mutex::new(false), Condvar::new()),
            notify: Mutex::new(Box::new(|_, _| {})),
            on_status: Mutex::new(Box::new(|_| {})),
            on_quit: Mutex::new(Box::new(|| {})),
        }
    }
}

impl Shared {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.wake_up();
    }
    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }
    pub fn request_refresh(&self) {
        self.force_refresh.store(true, Ordering::SeqCst);
        self.wake_up();
    }
    fn wake_up(&self) {
        let (m, c) = &self.wake;
        *m.lock().unwrap() = true;
        c.notify_all();
    }
    fn wait(&self, secs: f64) {
        let (m, c) = &self.wake;
        let mut woke = m.lock().unwrap();
        if !*woke {
            woke = c
                .wait_timeout(woke, Duration::from_secs_f64(secs))
                .unwrap()
                .0;
        }
        *woke = false;
    }
    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }
    pub fn notify(&self, title: &str, msg: &str) {
        (self.notify.lock().unwrap())(title, msg);
    }
}

/// Returns (queued requests, the addon's settings table).
pub fn read_sv(sv_file: &Path) -> Result<(Vec<Request>, Lua), String> {
    let bytes = std::fs::read(sv_file).map_err(|e| e.to_string())?;
    let data = luasv::parse(&bytes).map_err(|e| e.to_string())?;
    let db = luasv::global(&data, "SevenTVEmotesDB")
        .filter(|v| matches!(v, Lua::Table(_)))
        .cloned();
    let db = db.unwrap_or(Lua::Table(Vec::new()));
    let queue: Vec<Lua> = match db.get("queue") {
        Some(Lua::List(l)) => l.clone(),
        Some(Lua::Table(kv)) => {
            // sparse table
            let mut ints: Vec<(i64, Lua)> = kv
                .iter()
                .filter_map(|(k, v)| {
                    if let luasv::Key::Int(i) = k {
                        Some((*i, v.clone()))
                    } else {
                        None
                    }
                })
                .collect();
            ints.sort_by_key(|(k, _)| *k);
            ints.into_iter().map(|(_, v)| v).collect()
        }
        _ => Vec::new(),
    };
    let mut out = Vec::new();
    for r in queue {
        if !matches!(r, Lua::Table(_)) {
            continue;
        }
        let Some(id) = r.get("id").filter(|v| v.truthy()) else {
            continue;
        };
        let text = |k: &str| r.get(k).filter(|v| v.truthy()).map(Lua::to_plain_string);
        out.push(Request {
            id: id.to_plain_string(),
            kind: r
                .get("kind")
                .filter(|v| !matches!(v, Lua::Nil))
                .map(Lua::to_plain_string),
            reference: text("ref").unwrap_or_default(),
            alias: text("alias"),
        });
    }
    Ok((out, db))
}

pub fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(path) {
        for e in rd.flatten() {
            match e.file_type() {
                Ok(t) if t.is_file() => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                Ok(t) if t.is_dir() => total += dir_size(&e.path()),
                _ => {}
            }
        }
    }
    total
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum LastGs {
    Unset,
    Is(Option<f64>),
}

pub struct Service {
    pub config: Config,
    pub paths: AppPaths,
    pub shared: Arc<Shared>,
    http: Arc<dyn Http>,
    procs: Box<dyn ProcessSource>,
    pub helper_start: f64,
    pub roots: Vec<PathBuf>,
    pub libs: IndexMap<PathBuf, Library>,
    pub sv_mtimes: HashMap<PathBuf, f64>,
    attempts: HashMap<String, u32>,
    retry_at: f64,
    last_heartbeat: f64,
    last_discover: f64,
    last_game: Option<String>,
    last_gs: LastGs,
}

impl Service {
    pub fn new(
        config: Config,
        paths: AppPaths,
        http: Arc<dyn Http>,
        procs: Box<dyn ProcessSource>,
    ) -> Self {
        Self {
            config,
            paths,
            shared: Arc::new(Shared::default()),
            http,
            procs,
            helper_start: now(),
            roots: Vec::new(),
            libs: IndexMap::new(),
            sv_mtimes: HashMap::new(),
            attempts: HashMap::new(),
            retry_at: 0.0,
            last_heartbeat: 0.0,
            last_discover: 0.0,
            last_game: None,
            last_gs: LastGs::Unset,
        }
    }

    pub fn set_status(&self, text: &str, log_it: bool) {
        let changed = {
            let mut s = self.shared.status.lock().unwrap();
            if *s != text {
                *s = text.to_string();
                true
            } else {
                false
            }
        };
        if changed {
            if log_it {
                log::info!("status: {text}");
            }
            (self.shared.on_status.lock().unwrap())(text);
        }
    }

    pub fn request_refresh(&self) {
        self.shared.request_refresh();
    }

    // ------------------------------------------------------------------ discovery

    pub fn game_start(&mut self) -> Option<f64> {
        let Some(procs) = self.procs.game_processes(&self.roots) else {
            // Without process info, treat slots made during this helper run as unusable
            // until the next start (normally the next boot, before WoW is launched).
            if self.last_game.as_deref() != Some("unknown") {
                log::warn!(
                    "process list unavailable: can't tell whether WoW is running; being careful with new files"
                );
                self.last_game = Some("unknown".into());
            }
            return Some(self.helper_start);
        };
        let desc: Vec<String> = procs
            .iter()
            .map(|p| format!("{} (pid {})", p.name, p.pid))
            .collect();
        let desc = if desc.is_empty() {
            "not running".to_string()
        } else {
            desc.join(", ")
        };
        if self.last_game.as_deref() != Some(desc.as_str()) {
            log::info!("WoW: {desc}");
            self.last_game = Some(desc);
        }
        procs.iter().filter_map(|p| p.started).reduce(f64::min)
    }

    pub fn discover(&mut self) -> Vec<String> {
        self.last_discover = now();
        self.roots = wow::find_wow_roots(&self.config.wow_dirs);
        let flavors =
            wow::pick_flavors(&self.roots, wow::DEFAULT_FLAVOR_HINTS, &self.config.flavors);
        let mut changes = Vec::new();
        for f in flavors {
            if self.libs.contains_key(&f) {
                continue;
            }
            match installer::ensure_addons(&f, &self.paths.cache_dir(), self.http.clone()) {
                Ok((lib, ch)) => {
                    self.libs.insert(f.clone(), lib);
                    changes.extend(ch);
                    log::info!("managing {}", f.display());
                }
                Err(e) => log::error!("couldn't set up {}: {e}", f.display()),
            }
        }
        for c in &changes {
            log::info!("{c}");
        }
        *self.shared.lib_dirs.lock().unwrap() = self.libs.values().map(|l| l.dir.clone()).collect();
        changes
    }

    // ------------------------------------------------------------------ main loop

    pub fn run(&mut self) {
        let changes = self.discover();
        if !changes.is_empty() {
            self.shared.notify(
                "7TV Emotes",
                "Addon installed. If WoW is open, restart it once to load it.",
            );
        }
        while !self.shared.stopped() {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.tick()));
            if res.is_err() {
                log::error!("helper loop error (see the panic message above)");
                self.set_status("Something went wrong - see the log", true);
            }
            if self.shared.stopped() {
                break;
            }
            self.shared.wait(POLL_SECONDS);
        }
    }

    pub fn tick(&mut self) {
        if instance::quit_requested(&self.paths) {
            log::info!("a newer helper asked this one to quit");
            self.shared.stop();
            (self.shared.on_quit.lock().unwrap())();
            return;
        }
        let t = now();
        if t - self.last_discover > REDISCOVER_SECONDS {
            self.discover();
        }
        if self.libs.is_empty() {
            self.set_status("Waiting: World of Warcraft: Forever not found", true);
            return;
        }
        let gs = self.game_start();
        let session_changed = self.last_gs != LastGs::Is(gs);
        if session_changed && self.last_gs != LastGs::Unset {
            // WoW started or closed: files that were waiting for a restart are usable now.
            // Build them right away so they're ready before the player reaches the world.
            log::info!("WoW session changed; building anything that was waiting for a restart");
        }
        self.last_gs = LastGs::Is(gs);
        let force = self.shared.force_refresh.swap(false, Ordering::SeqCst);
        let heartbeat_due = t - self.last_heartbeat > HEARTBEAT_SECONDS;
        let retry_due = self.retry_at != 0.0 && t >= self.retry_at;

        let mut libs = std::mem::take(&mut self.libs);
        for lib in libs.values_mut() {
            self.process_library(lib, gs, force, retry_due, heartbeat_due || session_changed);
        }
        self.libs = libs;
        if heartbeat_due {
            self.last_heartbeat = t;
        }
        if retry_due {
            self.retry_at = 0.0;
        }
        let total: usize = self.libs.values().map(|l| l.desired().len()).sum();
        let open = gs.is_some_and(|g| g != self.helper_start);
        self.set_status(
            &format!(
                "Running - {total} emotes{}",
                if open { " (WoW open)" } else { "" }
            ),
            true,
        );
    }

    /// One pass over a game folder. Returns the results of requests handled this pass.
    pub fn process_library(
        &mut self,
        lib: &mut Library,
        gs: Option<f64>,
        force: bool,
        retry: bool,
        heartbeat: bool,
    ) -> Vec<Processed> {
        let mut pending: Vec<Request> = Vec::new();
        let mut latest_mtime = 0.0f64;
        let mut latest_db: Option<(f64, Lua)> = None;
        for sv in wow::saved_variable_files(&lib.flavor) {
            let Some(mtime) = file_mtime(&sv) else {
                continue;
            };
            latest_mtime = latest_mtime.max(mtime);
            if self.sv_mtimes.get(&sv) == Some(&mtime) && !retry && !force {
                continue;
            }
            let (queue, db) = match read_sv(&sv) {
                Ok(x) => x,
                Err(e) => {
                    log::info!("couldn't read {} yet ({e})", sv.display()); // probably mid-write; try next poll
                    continue;
                }
            };
            self.sv_mtimes.insert(sv.clone(), mtime);
            pending.extend(
                queue
                    .into_iter()
                    .filter(|r| !lib.state.processed.contains_key(&r.id)),
            );
            if latest_db.as_ref().is_none_or(|(m, _)| mtime >= *m) {
                latest_db = Some((mtime, db));
            }
        }

        lib.latest_sv_mtime = lib.latest_sv_mtime.max(latest_mtime);
        if let Some((_, db)) = &latest_db
            && let Some(inv) = db.get("invisible").filter(|v| v.truthy())
        {
            let items: Vec<String> = match inv {
                Lua::List(l) => l.iter().map(Lua::to_plain_string).collect(),
                Lua::Table(kv) => kv.iter().map(|(_, v)| v.to_plain_string()).collect(),
                other => vec![other.to_plain_string()],
            };
            log::warn!(
                "the addon reports {} emote file(s) WoW couldn't load this session, e.g. {}",
                items.len(),
                items.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
            );
        }

        // Settings from the most recently saved account.
        let mut fmt_changed = false;
        if let Some((_, db)) = &latest_db
            && let Some(c) = db.get("compress")
        {
            fmt_changed = lib.set_format(if c.truthy() { "blp" } else { "tga" });
        }

        let mut handled: Vec<(String, ReqResult)> = Vec::new();
        let mut seen = HashSet::new();
        let mut retry_changed = false;
        if !pending.is_empty() {
            let n = pending.len();
            self.set_status(&format!("Fetching {n} request(s)..."), true);
            self.board(
                lib,
                &format!(
                    "Fetching {n} request{} from 7TV...",
                    if n != 1 { "s" } else { "" }
                ),
                "busy",
            );
        }
        for req in &pending {
            if !seen.insert(req.id.clone()) {
                continue;
            }
            let res = match lib.apply_request(req) {
                Ok(r) => r,
                Err(NetworkError(e)) => {
                    let n = self.attempts.get(&req.id).copied().unwrap_or(0) + 1;
                    self.attempts.insert(req.id.clone(), n);
                    if n < MAX_ATTEMPTS {
                        log::info!("7TV unreachable for {} (attempt {n}): {e}", req.id);
                        lib.retrying.insert(
                            req.id.clone(),
                            format!(
                                "7TV couldn't be reached. Retrying (attempt {n} of {MAX_ATTEMPTS})."
                            ),
                        );
                        retry_changed = true;
                        self.retry_at = now() + RETRY_SECONDS;
                        continue;
                    }
                    ReqResult {
                        ok: false,
                        msg: "Couldn't reach 7TV. Check your internet connection and try again."
                            .into(),
                        name: None,
                    }
                }
            };
            lib.retrying.shift_remove(&req.id);
            handled.push((req.id.clone(), res));
        }

        let cleaned = lib.run_cleanup(gs, latest_mtime);
        let refreshed = lib.refresh_sources(crate::library::REFRESH_EVERY, force);
        let stuck = !lib.state.waiting.is_empty() || lib.state.converting > 0;
        let rebuild = lib.state.images.values().any(|i| i.rebuild);
        let upgrade = lib.upgrades_pending(gs);
        if fmt_changed {
            self.set_status("Converting emotes...", true);
        }
        if !handled.is_empty()
            || refreshed
            || force
            || fmt_changed
            || rebuild
            || upgrade
            || (heartbeat && stuck)
        {
            let build = {
                let mut reporter = self.progress_reporter(gs);
                lib.reconcile(gs, Some(&mut reporter))
            };
            let results: Vec<Processed> = handled
                .into_iter()
                .map(|(rid, res)| lib.finish_request(&rid, res, &build))
                .collect();
            if let Some(r) = ready_message(&build, &results) {
                lib.ready = Some(r);
            }
            lib.prune();
            let cache = dir_size(&self.paths.cache_dir());
            if let Err(e) = lib.write_lua(gs, None, Some(cache)) {
                log::error!("couldn't write Local.lua: {e}");
            }
            if let Err(e) = lib.save() {
                log::error!("couldn't save helper state: {e}");
            }
            self.last_heartbeat = now();
            for r in &results {
                log::info!("request: {}", r.msg);
            }
            if let Some(r) = lib.ready.as_mut() {
                r.t = now();
            }
            self.update_board(lib);
            if !results.is_empty() {
                let ok = results.iter().any(|r| r.ok);
                let msg: String = results
                    .iter()
                    .map(|r| r.msg.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
                    .chars()
                    .take(240)
                    .collect();
                self.shared.notify(
                    "7TV Emotes",
                    &format!(
                        "{msg}{}",
                        if ok {
                            " Reload WoW (/reload) to see it."
                        } else {
                            ""
                        }
                    ),
                );
            } else if fmt_changed {
                self.shared.notify(
                    "7TV Emotes",
                    "Emotes converted. Reload WoW (/reload) to use them.",
                );
            }
            return results;
        }
        if heartbeat || retry_changed || cleaned {
            let cache = dir_size(&self.paths.cache_dir());
            if let Err(e) = lib.write_lua(gs, None, Some(cache)) {
                log::error!("couldn't write Local.lua: {e}");
            }
            if cleaned && let Err(e) = lib.save() {
                log::error!("couldn't save helper state: {e}");
            }
        }
        self.update_board(lib);
        Vec::new()
    }

    // ------------------------------------------------------------------ live status board

    pub fn board_text(&self, lib: &Library) -> (String, &'static str) {
        if let Some((done, total)) = lib.progress {
            return (format!("Downloading emotes {done}/{total}"), "busy");
        }
        if !lib.retrying.is_empty() {
            return ("Can't reach 7TV - retrying".into(), "warn");
        }
        if lib.notice().is_some() {
            return (
                "Can't download 7TV's global emotes - retrying".into(),
                "error",
            );
        }
        if let Some(r) = &lib.ready
            && r.t > lib.latest_sv_mtime + 1.0
        {
            return (format!("{} - click Reload", r.msg), "ready");
        }
        let waiting = lib.state.waiting.len();
        if waiting > 0 {
            return (
                format!(
                    "{waiting} emote{} will show after a WoW restart",
                    if waiting != 1 { "s" } else { "" }
                ),
                "warn",
            );
        }
        (
            format!("Up to date - {} emotes", lib.desired().len()),
            "idle",
        )
    }

    fn board(&self, lib: &mut Library, text: &str, kind: &str) {
        // never let the status display break the helper
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            lib.board.update(text, kind, None)
        }));
        if res.is_err() {
            log::info!("status board: failed to draw");
        }
    }

    pub fn update_board(&self, lib: &mut Library) {
        let (text, kind) = self.board_text(lib);
        self.board(lib, &text, kind);
    }

    /// Status/log updates while textures build, plus partial Local.lua writes so a
    /// reload in the middle of a big download already shows what's done.
    fn progress_reporter(&self, gs: Option<f64>) -> impl FnMut(&mut Library, usize, usize) + '_ {
        let mut last_write = now();
        let mut last_logged: i64 = -1;
        let mut last_board = 0.0;
        move |lib: &mut Library, done: usize, total: usize| {
            let text = format!("Downloading emotes {done}/{total}");
            self.set_status(&text, false);
            if now() - last_board >= 1.0 || done == total {
                last_board = now();
                self.board(lib, &text, "busy");
            }
            let step = (total / 10).max(1);
            if done == total || (done / step) as i64 != last_logged {
                last_logged = (done / step) as i64;
                log::info!("emotes {done}/{total}");
            }
            let t = now();
            if 0 < done && done < total && t - last_write >= 5.0 {
                last_write = t;
                if let Err(e) = lib.write_lua(gs, None, None).and_then(|_| lib.save()) {
                    log::info!("partial write failed: {e}");
                }
            }
        }
    }

    /// Single synchronous pass (used by `--once` and tests).
    pub fn run_once(&mut self) -> Vec<Processed> {
        self.discover();
        let gs = self.game_start();
        let mut out = Vec::new();
        let mut libs = std::mem::take(&mut self.libs);
        for lib in libs.values_mut() {
            out.extend(self.process_library(lib, gs, false, true, true));
        }
        self.libs = libs;
        out
    }
}

fn ready_message(build: &BuildResult, results: &[Processed]) -> Option<Ready> {
    let added = build.added.len();
    let msg = if added > 0 {
        format!(
            "{added} new emote{} ready",
            if added != 1 { "s" } else { "" }
        )
    } else if !build.converted.is_empty() {
        "Emotes converted".into()
    } else if !build.upgraded.is_empty() {
        "Smoother emote animations ready".into()
    } else if results.iter().any(|r| r.ok) {
        "Your changes are ready".into()
    } else if !results.is_empty() {
        "Request finished (see /7tv)".into()
    } else {
        return None;
    };
    Some(Ready { t: now(), msg })
}
