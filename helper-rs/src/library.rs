//! The helper-owned emote library for one game folder.
//!
//! Everything lives in Interface/AddOns/SevenTVEmotes_Local, an addon the helper owns so
//! that addon managers updating the main addon never wipe it:
//!
//! ```text
//! SevenTVEmotes_Local.toc
//! Local.lua            emote table + helper status, read by WoW when the UI loads
//! slots/s0001.tga ...  reserved texture files (see below)
//! cslots/s0001.blp ... the same slots, compressed (own folder: WoW matches textures by name)
//! status/st0000.tga .. live status ring
//! helper_state.json    the helper's own bookkeeping (same schema as the Python helper)
//! ```
//!
//! Why slots: WoW only sees texture files that existed when the game started. Overwriting
//! an existing file only needs a /reload, but a brand-new file needs a full restart. So the
//! helper keeps a stock of blank placeholder files and writes new emotes into those.
//! A slot created (or freed) while the game is running is only used after a restart.

use crate::images::{self, Layout, RenderOptions};
use crate::luasv::{TableBuilder, Value as Lua, lua_value};
use crate::seventv::{self, ApiError, Emote, Http, Ref};
use crate::status_board::StatusBoard;
use crate::wow::{LOCAL_ADDON, addons_dir};
use crate::{VERSION, now};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

pub const INITIAL_SLOTS: usize = 1500;
pub const LOW_WATER: usize = 150; // top up when fewer free slots remain
pub const TOP_UP: usize = 500;
pub const RESULT_DAYS: f64 = 14.0;
pub const REFRESH_EVERY: f64 = 6.0 * 3600.0;

pub use crate::pack::valid_name;

// --------------------------------------------------------------------------- state (helper_state.json)

fn default_format() -> String {
    "tga".into()
}
fn default_sources() -> IndexMap<String, Source> {
    let mut m = IndexMap::new();
    m.insert(
        "global".to_string(),
        Source {
            kind: "global".into(),
            ..Default::default()
        },
    );
    m
}
fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Source {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twitch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_id: Option<String>,
    #[serde(default)]
    pub added: f64,
    #[serde(default)]
    pub refreshed: f64,
    #[serde(default)]
    pub emotes: Vec<Emote>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub slot: String,
    pub w: u32,
    pub h: u32,
    pub tw: u32,
    pub th: u32,
    #[serde(default = "one")]
    pub frames: u32,
    #[serde(default = "one")]
    pub cols: u32,
    #[serde(default)]
    pub ms: u32,
    #[serde(default = "default_format")]
    pub fmt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mips: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub rebuild: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}
fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Slot {
    #[serde(default)]
    pub created: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_blp: Option<f64>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub freed: Option<f64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Processed {
    pub ok: bool,
    pub msg: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub t: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Failed {
    #[serde(default)]
    pub err: String,
    #[serde(default)]
    pub t: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Cleanup {
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub after: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    /// "tga" (uncompressed) or "blp" (DXT5 compressed), set from the addon
    #[serde(default = "default_format", deserialize_with = "format_or_tga")]
    pub format: String,
    #[serde(default = "default_sources")]
    pub sources: IndexMap<String, Source>,
    /// emotes added one by one (name may be an alias)
    #[serde(default)]
    pub manual: Vec<Emote>,
    /// names hidden from channel/global sources
    #[serde(default)]
    pub removed: Vec<String>,
    /// Twitch channels by priority, highest first: when two channels have an emote with the same
    /// name, the one higher in this list wins. Channels missing from it (added before 0.7, or
    /// since the list was last saved) count as newer and go on top, newest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channel_order: Vec<String>,
    /// 7TV id -> texture
    #[serde(default)]
    pub images: IndexMap<String, ImageInfo>,
    #[serde(default)]
    pub slots: IndexMap<String, Slot>,
    /// request id -> result
    #[serde(default)]
    pub processed: IndexMap<String, Processed>,
    /// 7TV id -> error
    #[serde(default)]
    pub failed: IndexMap<String, Failed>,
    /// 7TV ids that need a WoW restart before they can show
    #[serde(default)]
    pub waiting: Vec<String>,
    /// textures still in the old format
    #[serde(default)]
    pub converting: u64,
    /// old-format files to blank once the addon reloaded
    #[serde(default)]
    pub cleanup: Cleanup,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<u32>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn format_or_tga<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(match v.as_str() {
        Some("blp") => "blp".into(),
        _ => "tga".into(),
    })
}

impl Default for State {
    fn default() -> Self {
        serde_json::from_str("{}").expect("default state")
    }
}

// --------------------------------------------------------------------------- requests and results

/// One request from the addon's queue.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: String,
    pub kind: Option<String>,
    pub reference: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReqResult {
    pub ok: bool,
    pub msg: String,
    pub name: Option<String>,
}

impl ReqResult {
    fn fail(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            msg: msg.into(),
            name: None,
        }
    }
    fn ok(msg: impl Into<String>) -> Self {
        Self {
            ok: true,
            msg: msg.into(),
            name: None,
        }
    }
}

/// 7TV couldn't be reached; the caller retries later.
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkError(pub String);

#[derive(Debug, Default, Clone)]
pub struct BuildResult {
    pub added: HashSet<String>,
    pub waiting: HashSet<String>,
    pub failed: HashSet<String>,
    pub converted: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ready {
    pub t: f64,
    pub msg: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WorkKind {
    New,
    Rebuild,
    Convert,
}

pub type Progress<'a> = &'a mut dyn FnMut(&mut Library, usize, usize);

pub struct Library {
    pub flavor: PathBuf,
    pub dir: PathBuf,
    pub slots_dir: PathBuf,
    pub cslots_dir: PathBuf,
    pub test_dir: PathBuf,
    pub state_path: PathBuf,
    pub cache_dir: PathBuf,
    template_toc: Option<Vec<u8>>,
    http: Arc<dyn Http>,
    /// request id -> message (not persisted)
    pub retrying: IndexMap<String, String>,
    /// (done, total) while textures are being built
    pub progress: Option<(usize, usize)>,
    pub source_errors: HashMap<String, String>,
    pub board: StatusBoard,
    /// new content written since the last reload
    pub ready: Option<Ready>,
    pub latest_sv_mtime: f64,
    pub state: State,
    pub jobs: usize,
}

fn flavor_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Library {
    pub fn new(
        flavor: &Path,
        cache_dir: &Path,
        template_toc: Option<Vec<u8>>,
        http: Arc<dyn Http>,
    ) -> Self {
        let dir = addons_dir(flavor).join(LOCAL_ADDON);
        let mut lib = Library {
            flavor: flavor.to_path_buf(),
            slots_dir: dir.join("slots"),
            cslots_dir: dir.join("cslots"),
            test_dir: dir.join("test"),
            state_path: dir.join("helper_state.json"),
            board: StatusBoard::new(&dir.join("status")),
            dir,
            cache_dir: cache_dir.to_path_buf(),
            template_toc,
            http,
            retrying: IndexMap::new(),
            progress: None,
            source_errors: HashMap::new(),
            ready: None,
            latest_sv_mtime: 0.0,
            state: State::default(),
            jobs: 8,
        };
        lib.state = lib.load();
        lib.migrate();
        lib
    }

    fn name(&self) -> String {
        flavor_name(&self.flavor)
    }

    fn migrate(&mut self) {
        let t = now();
        for slot in self.state.slots.values_mut() {
            slot.created_blp.get_or_insert(t);
        }
        if self.state.layout.unwrap_or(1) < 2 {
            if self.installed() {
                // Helper 0.3/0.4 kept s0001.tga and s0001.blp side by side. WoW seems to match texture
                // files by name regardless of extension, so it could load the wrong twin, which is why
                // compressed emotes stayed invisible. Compressed files now live in cslots/.
                let _ = std::fs::create_dir_all(&self.cslots_dir);
                let names: Vec<String> = self.state.slots.keys().cloned().collect();
                for name in &names {
                    let _ = images::write_blank(&self.cslots_dir.join(format!("{name}.blp")));
                    self.state.slots[name].created_blp = Some(t);
                }
                if let Ok(rd) = std::fs::read_dir(&self.slots_dir) {
                    for e in rd.flatten() {
                        let p = e.path();
                        if p.extension()
                            .and_then(|x| x.to_str())
                            .is_some_and(|x| x.eq_ignore_ascii_case("blp"))
                        {
                            let _ = std::fs::remove_file(&p);
                        }
                    }
                }
                let mut n = 0;
                for info in self.state.images.values_mut() {
                    if info.fmt == "blp" {
                        info.fmt = "tga".into();
                        info.rebuild = true;
                        n += 1;
                    }
                }
                self.state.cleanup = Cleanup::default();
                log::info!(
                    "{}: moved compressed textures to cslots/ ({n} emotes will be rebuilt)",
                    self.name()
                );
            }
            self.state.layout = Some(2);
        }
        if self.installed() && !self.dir.join("status").join("st0000.tga").exists() {
            match self.board.ensure_files() {
                Ok(made) => log::info!(
                    "{}: created {made} live-status images (WoW sees them after its next restart)",
                    self.name()
                ),
                Err(e) => log::info!("status images: {e}"),
            }
        }
        if self.installed() && !self.test_dir.join("t_tga.tga").exists() {
            self.make_test_images();
        }
    }

    /// Small labelled images in several texture formats, for /7tv texturetest in game.
    pub fn make_test_images(&self) {
        let _ = std::fs::create_dir_all(&self.test_dir);
        let variants = [
            ("t_tga.tga", "TGA"),
            ("t_dxt5m.blp", "BLP DXT5 + mips"),
            ("t_dxt5.blp", "BLP DXT5"),
            ("t_argb.blp", "BLP raw"),
            ("t_pal.blp", "BLP palette"),
        ];
        for (fname, label) in variants {
            let img = images::resize_lanczos(
                &crate::status_board::render(label, "test image", "ready"),
                256,
                32,
            );
            let bytes = match fname {
                "t_tga.tga" => images::tga_bytes(&img),
                "t_dxt5m.blp" => images::blp_bytes(&img, true),
                "t_dxt5.blp" => images::blp_bytes(&img, false),
                "t_argb.blp" => images::blp_raw_bytes(&img),
                _ => images::blp_palette_bytes(&img),
            };
            if let Err(e) = std::fs::write(self.test_dir.join(fname), bytes) {
                log::info!("test image {fname}: {e}");
            }
        }
    }

    // ------------------------------------------------------------------ state

    fn load(&self) -> State {
        match std::fs::read(&self.state_path) {
            Ok(bytes) => match serde_json::from_slice::<State>(&bytes) {
                Ok(st) => st,
                Err(e) => {
                    log::warn!("state file unreadable ({e}); starting fresh");
                    State::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => {
                log::warn!("state file unreadable ({e}); starting fresh");
                State::default()
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join("helper_state.json.part");
        let text = serde_json::to_string_pretty(&self.state).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.state_path)
    }

    // ------------------------------------------------------------------ install

    /// The helper has set up this folder. (A folder with a .toc but no helper_state.json came
    /// from a downloaded emote pack, or lost its state: install() sets it up properly.)
    pub fn installed(&self) -> bool {
        self.dir.join(format!("{LOCAL_ADDON}.toc")).exists() && self.state_path.exists()
    }

    /// Create/refresh the SevenTVEmotes_Local addon folder and reserve slots.
    pub fn install(&mut self) -> std::io::Result<()> {
        // A downloaded emote pack keeps its textures in pack/; the helper's own files replace it.
        let pack_dir = self.dir.join(crate::pack::PACK_DIR);
        if pack_dir.is_dir() {
            match std::fs::remove_dir_all(&pack_dir) {
                Ok(()) => log::info!("{}: replaced a downloaded emote pack", self.name()),
                Err(e) => log::warn!(
                    "{}: couldn't remove the old emote pack folder: {e}",
                    self.name()
                ),
            }
        }
        std::fs::create_dir_all(&self.dir)?;
        std::fs::create_dir_all(&self.slots_dir)?;
        if let Some(toc) = &self.template_toc {
            std::fs::write(self.dir.join(format!("{LOCAL_ADDON}.toc")), toc)?;
        }
        self.create_slots(INITIAL_SLOTS.saturating_sub(self.state.slots.len()))?;
        self.board.ensure_files()?;
        self.make_test_images();
        self.state.layout = Some(2);
        self.write_lua(None, None, None)?;
        self.save()
    }

    fn create_slots(&mut self, n: usize) -> std::io::Result<()> {
        if n == 0 {
            return Ok(());
        }
        let next = self
            .state
            .slots
            .keys()
            .filter_map(|k| k.get(1..)?.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let t = now();
        std::fs::create_dir_all(&self.slots_dir)?;
        std::fs::create_dir_all(&self.cslots_dir)?;
        for i in next..next + n as u32 {
            let name = format!("s{i:04}");
            images::write_blank(&self.slots_dir.join(format!("{name}.tga")))?;
            images::write_blank(&self.cslots_dir.join(format!("{name}.blp")))?;
            self.state.slots.insert(
                name,
                Slot {
                    created: t,
                    created_blp: Some(t),
                    ..Default::default()
                },
            );
        }
        log::info!("{}: reserved {n} new texture slots", self.name());
        Ok(())
    }

    // ------------------------------------------------------------------ slots

    /// Did this slot's file in `fmt` exist when the running game started?
    fn file_visible(slot: &Slot, fmt: &str, game_start: Option<f64>) -> bool {
        let Some(gs) = game_start else { return true };
        let created = if fmt == "blp" {
            slot.created_blp.unwrap_or(0.0)
        } else {
            slot.created
        };
        created < gs
    }

    fn usable(slot: &Slot, game_start: Option<f64>, fmt: &str) -> bool {
        if slot.id.is_some() || !Self::file_visible(slot, fmt, game_start) {
            return false;
        }
        match (game_start, slot.freed) {
            (None, _) | (_, None) => true,
            (Some(gs), Some(f)) => f < gs,
        }
    }

    pub fn free_slots(&self, game_start: Option<f64>, fmt: Option<&str>) -> Vec<String> {
        let fmt = fmt.unwrap_or(&self.state.format);
        let mut v: Vec<String> = self
            .state
            .slots
            .iter()
            .filter(|(_, s)| Self::usable(s, game_start, fmt))
            .map(|(k, _)| k.clone())
            .collect();
        v.sort();
        v
    }

    pub fn slot_file(&self, slot: &str, fmt: &str) -> PathBuf {
        (if fmt == "blp" {
            &self.cslots_dir
        } else {
            &self.slots_dir
        })
        .join(format!("{slot}.{fmt}"))
    }

    fn slot_lua_path(slot: &str, fmt: &str) -> String {
        format!(
            "{}{slot}.{fmt}",
            if fmt == "blp" { "cslots\\" } else { "slots\\" }
        )
    }

    // ------------------------------------------------------------------ format (compression)

    pub fn set_format(&mut self, fmt: &str) -> bool {
        if (fmt != "tga" && fmt != "blp") || fmt == self.state.format {
            return false;
        }
        log::info!("{}: switching textures to {fmt}", self.name());
        self.state.format = fmt.to_string();
        true
    }

    fn schedule_cleanup(&mut self, files: &[PathBuf]) {
        let c = &mut self.state.cleanup;
        let mut set: std::collections::BTreeSet<String> = c.files.iter().cloned().collect();
        set.extend(files.iter().map(|f| f.to_string_lossy().into_owned()));
        c.files = set.into_iter().collect();
        c.after = None; // set when the Local.lua that stops using them is written
    }

    /// Blank old-format files once the addon has reloaded past the switch (or WoW is closed).
    pub fn run_cleanup(&mut self, game_start: Option<f64>, latest_sv_mtime: f64) -> bool {
        let c = &self.state.cleanup;
        let Some(after) = c.after else { return false };
        if c.files.is_empty() {
            return false;
        }
        if game_start.is_some() && latest_sv_mtime <= after + 2.0 {
            return false;
        }
        for f in &c.files {
            let p = Path::new(f);
            if p.exists() {
                let _ = images::write_blank(p);
            }
        }
        log::info!(
            "{}: cleared {} old texture files",
            self.name(),
            c.files.len()
        );
        self.state.cleanup = Cleanup::default();
        true
    }

    // ------------------------------------------------------------------ storage

    pub fn storage(&self, cache_bytes: Option<u64>) -> Lua {
        let mut actual = 0u64;
        for d in [&self.slots_dir, &self.cslots_dir] {
            if let Ok(rd) = std::fs::read_dir(d) {
                for e in rd.flatten() {
                    if let Ok(m) = e.metadata()
                        && m.is_file()
                    {
                        actual += m.len();
                    }
                }
            }
        }
        let imgs = &self.state.images;
        let (blank_tga, blank_blp) = (images::tga_size(8, 8), images::blp_size(8, 8));
        let empty =
            (self.state.slots.len().saturating_sub(imgs.len())) as u64 * (blank_tga + blank_blp);
        let n = imgs.len() as u64;
        let as_tga: u64 = imgs
            .values()
            .map(|i| images::tga_size(i.tw, i.th))
            .sum::<u64>()
            + n * blank_blp
            + empty;
        let as_blp: u64 = imgs
            .values()
            .map(|i| images::blp_size(i.tw, i.th))
            .sum::<u64>()
            + n * blank_tga
            + empty;
        let mut t = TableBuilder::new()
            .set("bytes", actual as i64)
            .set("tga", as_tga as i64)
            .set("blp", as_blp as i64)
            .set("format", self.state.format.as_str())
            .set("converting", self.state.converting as i64)
            .set("emotes", imgs.len())
            .set("compressAvailable", images::compression_available());
        if let Some(c) = cache_bytes {
            t.push("cache", c as i64);
        }
        t.build()
    }

    pub fn top_up_slots(&mut self) {
        let free_total = self.state.slots.values().filter(|s| s.id.is_none()).count();
        if free_total < LOW_WATER
            && let Err(e) = self.create_slots(TOP_UP)
        {
            log::warn!("{}: couldn't reserve more slots: {e}", self.name());
        }
    }

    // ------------------------------------------------------------------ desired emote set

    /// Sources from lowest to highest priority: 7TV globals, then channels (the bottom of the
    /// channel list first), then emote sets added from links (oldest first), then emotes added
    /// one by one. A later source wins a name.
    fn sources_in_order(&self) -> Vec<(String, Vec<Emote>)> {
        let src = &self.state.sources;
        let mut out: Vec<(String, Vec<Emote>)> = src
            .iter()
            .filter(|(_, s)| s.kind == "global")
            .map(|(k, s)| (k.clone(), s.emotes.clone()))
            .collect();
        for login in self.channels().iter().rev() {
            let key = format!("channel:{login}");
            if let Some(s) = src.get(&key) {
                out.push((key, s.emotes.clone()));
            }
        }
        let mut others: Vec<(&String, &Source)> = src
            .iter()
            .filter(|(k, s)| s.kind != "global" && !k.starts_with("channel:"))
            .collect();
        others.sort_by(|a, b| {
            a.1.added
                .partial_cmp(&b.1.added)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.extend(
            others
                .into_iter()
                .map(|(k, s)| (k.clone(), s.emotes.clone())),
        );
        out.push(("manual".into(), self.state.manual.clone()));
        out
    }

    /// name -> (emote, source key); later sources win (see `sources_in_order`).
    pub fn desired_with_sources(&self) -> IndexMap<String, (Emote, String)> {
        crate::pack::resolve(&self.sources_in_order(), &self.state.removed)
    }

    pub fn desired(&self) -> IndexMap<String, Emote> {
        self.desired_with_sources()
            .into_iter()
            .map(|(n, (em, _))| (n, em))
            .collect()
    }

    pub fn source_label(&self, key: &str) -> String {
        let set_label = self.state.sources.get(key).and_then(|s| s.label.as_deref());
        crate::pack::source_label(key, set_label)
    }

    // ------------------------------------------------------------------ reconcile

    /// Make textures match the desired emote set and format.
    pub fn reconcile(
        &mut self,
        game_start: Option<f64>,
        progress: Option<Progress>,
    ) -> BuildResult {
        let fmt = self.state.format.clone();
        let want = self.desired();
        let mut need_ids: IndexMap<String, Emote> = IndexMap::new();
        for em in want.values() {
            need_ids.entry(em.id.clone()).or_insert_with(|| em.clone());
        }
        let mut result = BuildResult::default();

        // Free textures nobody uses any more (blank the file to give the disk space back).
        let t = now();
        let unused: Vec<String> = self
            .state
            .images
            .keys()
            .filter(|i| !need_ids.contains_key(*i))
            .cloned()
            .collect();
        for eid in unused {
            let info = self.state.images.shift_remove(&eid).unwrap();
            if let Some(slot) = self.state.slots.get_mut(&info.slot) {
                slot.id = None;
                slot.freed = Some(t);
                let f = self.slot_file(&info.slot, &info.fmt);
                if f.exists() {
                    let _ = images::write_blank(&f);
                }
            }
        }

        // Work list: (kind, emote, slot, format)
        //   New     : emote not on disk yet, needs a free slot
        //   Rebuild : emote whose file must be rewritten in its own slot (e.g. after moving folders)
        //   Convert : emote in the other format, same slot
        // If the wanted format's files aren't visible to the running game yet, new emotes are built
        // as TGA for now and converted after the next WoW restart.
        let mut work: Vec<(WorkKind, Emote, String, String)> = Vec::new();
        let mut free = self.free_slots(game_start, Some(&fmt));
        let fallback: Vec<String> = if fmt != "tga" {
            let fs: HashSet<&String> = free.iter().collect();
            self.free_slots(game_start, Some("tga"))
                .into_iter()
                .filter(|x| !fs.contains(x))
                .collect()
        } else {
            Vec::new()
        };
        let mut fallback_set: HashSet<String> = fallback.iter().cloned().collect();
        free.reverse(); // pop() from the end == pop(0) from the front
        for (eid, em) in &need_ids {
            if self.state.images.contains_key(eid) {
                continue;
            }
            if let Some(slot) = free.pop() {
                fallback_set.remove(&slot);
                work.push((WorkKind::New, em.clone(), slot, fmt.clone()));
            } else {
                match fallback.iter().find(|x| fallback_set.contains(*x)).cloned() {
                    Some(slot) => {
                        fallback_set.remove(&slot);
                        work.push((WorkKind::New, em.clone(), slot, "tga".into()));
                    }
                    None => {
                        result.waiting.insert(eid.clone());
                    }
                }
            }
        }
        let mut old_files: Vec<PathBuf> = Vec::new();
        let mut pending_conv: u64 = 0;
        for (eid, info) in &self.state.images {
            let Some(slot_rec) = self.state.slots.get(&info.slot) else {
                continue;
            };
            let Some(em) = need_ids.get(eid) else {
                continue;
            };
            if info.rebuild {
                let f = if Self::file_visible(slot_rec, &fmt, game_start) {
                    fmt.clone()
                } else {
                    "tga".into()
                };
                work.push((WorkKind::Rebuild, em.clone(), info.slot.clone(), f));
                continue;
            }
            // BLPs from helper 0.3.0 had no mipmaps; re-encode them in place.
            let stale_blp = fmt == "blp" && info.fmt == "blp" && info.mips != Some(true);
            if info.fmt != fmt || stale_blp {
                if Self::file_visible(slot_rec, &fmt, game_start) {
                    work.push((
                        WorkKind::Convert,
                        em.clone(),
                        info.slot.clone(),
                        fmt.clone(),
                    ));
                } else {
                    pending_conv += 1;
                }
            }
        }

        if !work.is_empty() {
            let total = work.len();
            let mut done = 0;
            self.progress = Some((0, total));
            let mut progress = progress;
            if let Some(p) = progress.as_mut() {
                p(self, 0, total);
            }
            let jobs: Vec<(Emote, PathBuf)> = work
                .iter()
                .map(|(_, em, slot, f)| (em.clone(), self.slot_file(slot, f)))
                .collect();
            let http = self.http.clone();
            let cache = self.cache_dir.clone();
            let next = AtomicUsize::new(0);
            let workers = self.jobs.max(1).min(jobs.len());
            std::thread::scope(|s| {
                let (tx, rx) = mpsc::channel::<(usize, Result<Layout, String>)>();
                for _ in 0..workers {
                    let tx = tx.clone();
                    let (jobs, http, cache, next) = (&jobs, &http, &cache, &next);
                    s.spawn(move || {
                        loop {
                            let i = next.fetch_add(1, Ordering::SeqCst);
                            let Some((em, out)) = jobs.get(i) else { break };
                            let res =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    let src = images::download(http.as_ref(), em, 64, cache)?;
                                    images::render(&src, out, &RenderOptions::default())
                                }))
                                .unwrap_or_else(|_| Err("internal error while converting".into()));
                            if tx.send((i, res)).is_err() {
                                break;
                            }
                        }
                    });
                }
                drop(tx);
                for (i, res) in rx {
                    let (kind, em, slot, f) = &work[i];
                    done += 1;
                    match res {
                        Err(e) => {
                            // one bad emote shouldn't stop the rest
                            log::warn!("failed to build {} ({}): {e}", em.name, em.id);
                            match kind {
                                WorkKind::New => {
                                    let err: String = e.chars().take(200).collect();
                                    self.state
                                        .failed
                                        .insert(em.id.clone(), Failed { err, t: now() });
                                    result.failed.insert(em.id.clone());
                                }
                                WorkKind::Convert => pending_conv += 1,
                                WorkKind::Rebuild => {}
                            }
                        }
                        Ok(layout) => {
                            let prev = self.state.images.get(&em.id).map(|i| i.fmt.clone());
                            if *kind == WorkKind::New {
                                if let Some(s) = self.state.slots.get_mut(slot) {
                                    s.id = Some(em.id.clone());
                                    s.freed = None;
                                }
                                self.state.failed.shift_remove(&em.id);
                                result.added.insert(em.id.clone());
                            } else {
                                let prev = prev.unwrap_or_else(default_format);
                                if prev != *f {
                                    old_files.push(self.slot_file(slot, &prev));
                                }
                                result.converted.insert(em.id.clone());
                            }
                            let info = ImageInfo {
                                slot: slot.clone(),
                                w: layout.w,
                                h: layout.h,
                                tw: layout.tw,
                                th: layout.th,
                                frames: layout.frames,
                                cols: layout.cols,
                                ms: layout.ms,
                                fmt: f.clone(),
                                mips: Some(f == "blp"),
                                rebuild: false,
                                extra: Map::new(),
                            };
                            self.state.images.insert(em.id.clone(), info);
                            if *f != fmt {
                                pending_conv += 1;
                            }
                        }
                    }
                    self.progress = Some((done, total));
                    if let Some(p) = progress.as_mut() {
                        p(self, done, total);
                    }
                }
            });
            self.progress = None;
        }

        if !old_files.is_empty() {
            self.schedule_cleanup(&old_files);
        }
        let mut waiting: Vec<String> = result.waiting.iter().cloned().collect();
        waiting.sort();
        self.state.waiting = waiting;
        self.state.converting = pending_conv;
        // Anything that couldn't get a slot now will once the game restarts.
        self.top_up_slots();
        if !work.is_empty() {
            log::info!(
                "{}: {} new textures, {} converted, {} waiting for a restart, {} failed",
                self.name(),
                result.added.len(),
                result.converted.len(),
                result.waiting.len(),
                result.failed.len()
            );
        }
        result
    }

    // ------------------------------------------------------------------ Local.lua

    /// The user's Twitch channels by priority, highest first (see `State::channel_order`).
    pub fn channels(&self) -> Vec<String> {
        let have: Vec<(&str, f64)> = self
            .state
            .sources
            .iter()
            .filter_map(|(k, s)| k.strip_prefix("channel:").map(|l| (l, s.added)))
            .collect();
        let order = &self.state.channel_order;
        let mut fresh: Vec<&(&str, f64)> = have
            .iter()
            .filter(|(l, _)| !order.iter().any(|o| o == l))
            .collect();
        fresh.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut out: Vec<String> = fresh.into_iter().map(|(l, _)| l.to_string()).collect();
        for l in order {
            if have.iter().any(|(h, _)| h == l) && !out.contains(l) {
                out.push(l.clone());
            }
        }
        out
    }

    pub fn write_lua(
        &mut self,
        game_start: Option<f64>,
        notice: Option<&str>,
        cache_bytes: Option<u64>,
    ) -> std::io::Result<()> {
        let cutoff = now() - RESULT_DAYS * 86400.0;
        let want = self.desired_with_sources();
        let waiting_ids: HashSet<&String> = self.state.waiting.iter().collect();
        let imgs = &self.state.images;

        let mut results = TableBuilder::new();
        for (k, v) in self.state.processed.iter().filter(|(_, v)| v.t >= cutoff) {
            results.push(
                k.as_str(),
                TableBuilder::new()
                    .set("ok", v.ok)
                    .set("msg", v.msg.as_str())
                    .set("name", v.name.clone())
                    .build(),
            );
        }
        let mut retrying = TableBuilder::new();
        for (k, v) in &self.retrying {
            retrying.push(k.as_str(), v.as_str());
        }
        let mut waiting: Vec<String> = want
            .iter()
            .filter(|(_, (em, _))| waiting_ids.contains(&em.id))
            .map(|(n, _)| n.clone())
            .collect();
        waiting.sort();
        let mut origin = TableBuilder::new();
        for (n, (em, k)) in &want {
            if imgs.contains_key(&em.id) {
                origin.push(n.as_str(), self.source_label(k));
            }
        }
        let mut status = TableBuilder::new()
            .set("version", VERSION)
            .set("heartbeat", now() as i64)
            .set("slotsFree", self.free_slots(game_start, None).len())
            .set("channels", self.channels())
            .set("results", results.build())
            .set("retrying", retrying.build())
            .set("waiting", waiting)
            .set("storage", self.storage(cache_bytes))
            .set("board", self.board.info())
            .set("origin", origin.build());
        let notice = notice.map(String::from).or_else(|| self.notice());
        if let Some(n) = notice.filter(|n| !n.is_empty()) {
            status.push("notice", n);
        }
        if let Some((done, total)) = self.progress {
            status.push(
                "progress",
                TableBuilder::new()
                    .set("done", done)
                    .set("total", total)
                    .set("t", now() as i64)
                    .build(),
            );
        }

        let mut lines: Vec<String> = vec![
            "-- Written by the 7TV Emotes Helper app. Anything you change here gets overwritten."
                .into(),
            format!("SevenTVEmotes_Helper = {}", lua_value(&status.build(), "")),
            String::new(),
            crate::pack::EMOTES_HEADER.into(),
            "local emotes = {".into(),
        ];
        let mut names: Vec<(&String, &(Emote, String))> = want.iter().collect();
        names.sort_by_key(|(n, _)| n.to_lowercase()); // stable, like Python's sorted()
        for (name, (em, _)) in names {
            let Some(info) = imgs.get(&em.id) else {
                continue;
            };
            if info.rebuild {
                continue;
            }
            let layout = Layout {
                w: info.w,
                h: info.h,
                tw: info.tw,
                th: info.th,
                frames: info.frames,
                cols: info.cols,
                ms: info.ms,
            };
            lines.push(crate::pack::emote_entry(
                name,
                &Self::slot_lua_path(&info.slot, &info.fmt),
                &layout,
                &em.id,
            ));
        }
        lines.extend(crate::pack::register_footer());
        std::fs::create_dir_all(&self.dir)?;
        let out = self.dir.join("Local.lua");
        let tmp = self.dir.join("Local.lua.part");
        std::fs::write(&tmp, lines.join("\n"))?;
        std::fs::rename(&tmp, &out)?;
        let c = &mut self.state.cleanup;
        if !c.files.is_empty() && c.after.is_none() {
            c.after = Some(now());
        }
        Ok(())
    }

    // ------------------------------------------------------------------ sources

    /// Re-download global/channel emote lists that are older than max_age. Returns true if anything changed.
    ///
    /// A source that has never loaded (e.g. the global set on first run while offline) is retried
    /// every minute instead of every few hours.
    pub fn refresh_sources(&mut self, max_age: f64, force: bool) -> bool {
        let mut changed = false;
        let keys: Vec<String> = self.state.sources.keys().cloned().collect();
        for key in keys {
            let src = &self.state.sources[&key];
            let age = now() - src.refreshed;
            let never = src.refreshed == 0.0;
            if !force {
                if never && now() - src.last_attempt.unwrap_or(0.0) < 60.0 {
                    continue;
                }
                if !never && age < max_age {
                    continue;
                }
            }
            let (kind, login, set_id) = (src.kind.clone(), src.login.clone(), src.set_id.clone());
            self.state.sources[&key].last_attempt = Some(now());
            let fetched = match kind.as_str() {
                "global" => seventv::fetch_global(self.http.as_ref()),
                "channel" => seventv::fetch_twitch_channel(
                    self.http.as_ref(),
                    login.as_deref().unwrap_or(""),
                )
                .map(|(_, e)| e),
                "set" => seventv::fetch_set(self.http.as_ref(), set_id.as_deref().unwrap_or(""))
                    .map(|(_, e)| e),
                _ => continue,
            };
            let ems = match fetched {
                Ok(ems) => ems,
                Err(ApiError::NotFound(e)) => {
                    log::warn!("source {key}: {e}");
                    self.source_errors.insert(key.clone(), e);
                    self.state.sources[&key].refreshed = now();
                    continue;
                }
                Err(e) => {
                    // network, TLS, unexpected JSON...
                    log::warn!("source {key}: couldn't download ({e}); will retry");
                    self.source_errors
                        .insert(key.clone(), e.to_string().chars().take(160).collect());
                    continue;
                }
            };
            self.source_errors.remove(&key);
            let src = &mut self.state.sources[&key];
            if ems != src.emotes {
                src.emotes = ems;
                changed = true;
            }
            if never {
                log::info!("source {key}: {} emotes", src.emotes.len());
            }
            src.refreshed = now();
        }
        changed
    }

    pub fn notice(&self) -> Option<String> {
        let g = self.state.sources.get("global")?;
        if g.refreshed == 0.0
            && let Some(err) = self.source_errors.get("global")
        {
            return Some(format!(
                "Couldn't download 7TV's global emotes yet ({err}). The helper keeps trying every minute."
            ));
        }
        None
    }

    // ------------------------------------------------------------------ requests

    /// An emote by name from the user's sources, preferring the highest-priority one (the emote
    /// the name means in chat).
    fn find_cached(&self, name: &str) -> Option<Emote> {
        self.sources_in_order()
            .into_iter()
            .rev()
            .flat_map(|(_, ems)| ems)
            .find(|em| em.name == name)
    }

    /// Apply one in-game request to the desired state.
    ///
    /// Network errors come back as Err so the caller can retry later.
    pub fn apply_request(&mut self, req: &Request) -> Result<ReqResult, NetworkError> {
        let reference = req.reference.trim().to_string();
        if reference.is_empty() {
            return Ok(ReqResult::fail("Empty request."));
        }
        let kind = req.kind.clone().unwrap_or_default();
        let res = match kind.as_str() {
            "add" => self.req_add(&reference, req.alias.as_deref()),
            "remove" => Ok(self.req_remove(&reference)),
            "channel" => self.req_channel(&reference.to_lowercase()),
            "unchannel" => {
                let login = reference.to_lowercase();
                let key = format!("channel:{login}");
                Ok(if self.state.sources.shift_remove(&key).is_none() {
                    ReqResult::fail(format!("You haven't added {reference}'s channel."))
                } else {
                    self.state.channel_order.retain(|l| *l != login);
                    ReqResult::ok(format!("Removed {reference}'s channel emotes."))
                })
            }
            "order" => Ok(self.req_order(&reference)),
            _ => {
                let shown = req.kind.clone().unwrap_or_else(|| "None".into());
                return Ok(ReqResult::fail(format!("Unknown request '{shown}'.")));
            }
        };
        match res {
            Ok(r) => Ok(r),
            Err(ApiError::NotFound(e)) => {
                Ok(ReqResult::fail(friendly_not_found(&kind, &reference, &e)))
            }
            Err(ApiError::Invalid(e)) => Ok(ReqResult::fail(e)),
            Err(ApiError::Network(e)) => Err(NetworkError(e)),
        }
    }

    fn req_add(&mut self, reference: &str, alias: Option<&str>) -> Result<ReqResult, ApiError> {
        let em = match seventv::parse_ref(reference)? {
            Ref::Set(value) => {
                let (name, ems) = seventv::fetch_set(self.http.as_ref(), &value)?;
                let n = ems.len();
                self.state.sources.insert(
                    format!("set:{value}"),
                    Source {
                        kind: "set".into(),
                        set_id: Some(value.clone()),
                        added: now(),
                        refreshed: now(),
                        emotes: ems,
                        label: Some(name.clone()),
                        ..Default::default()
                    },
                );
                return Ok(ReqResult::ok(format!(
                    "Added the emote set '{name}' ({n} emotes)."
                )));
            }
            Ref::Id(value) => seventv::fetch_emote(self.http.as_ref(), &value)?,
            Ref::Name(value) => {
                let found = match self.find_cached(&value) {
                    Some(em) => Some(em),
                    None => match seventv::search_by_name(self.http.as_ref(), &value) {
                        Ok(v) => v,
                        Err(ApiError::NotFound(_)) => None,
                        Err(e) => return Err(e),
                    },
                };
                match found {
                    Some(em) => em,
                    None => {
                        return Ok(ReqResult::fail(format!(
                            "Couldn't find a 7TV emote called {value}. Copy its link from 7tv.app and use /7tv add <link>."
                        )));
                    }
                }
            }
        };
        let name = alias
            .filter(|a| !a.is_empty())
            .map(String::from)
            .unwrap_or_else(|| em.name.clone());
        if !valid_name(&name) {
            return Ok(ReqResult::fail(format!(
                "'{name}' can't be used as an emote name in WoW chat (no spaces or |)."
            )));
        }
        let em = Emote {
            name: name.clone(),
            ..em
        };
        self.state.manual.retain(|m| m.name != name);
        self.state.manual.push(em);
        self.state.removed.retain(|r| *r != name);
        Ok(ReqResult {
            ok: true,
            msg: format!("Added {name}."),
            name: Some(name),
        })
    }

    fn req_remove(&mut self, name: &str) -> ReqResult {
        let before = self.state.manual.len();
        self.state.manual.retain(|m| m.name != name);
        let from_sources = self
            .sources_in_order()
            .iter()
            .filter(|(k, _)| k != "manual")
            .any(|(_, ems)| ems.iter().any(|e| e.name == name));
        if from_sources && !self.state.removed.iter().any(|r| r == name) {
            self.state.removed.push(name.to_string());
        }
        if before == self.state.manual.len() && !from_sources {
            return ReqResult::fail(format!("{name} isn't one of your emotes."));
        }
        ReqResult::ok(format!("Removed {name}."))
    }

    fn req_channel(&mut self, login: &str) -> Result<ReqResult, ApiError> {
        let (twitch_id, ems) = seventv::fetch_twitch_channel(self.http.as_ref(), login)?;
        let key = format!("channel:{login}");
        let added = self
            .state
            .sources
            .get(&key)
            .map(|p| p.added)
            .unwrap_or_else(now);
        let n = ems.len();
        let new_channel = !self.state.sources.contains_key(&key);
        self.state.sources.insert(
            key,
            Source {
                kind: "channel".into(),
                login: Some(login.to_string()),
                twitch_id: Some(twitch_id),
                added,
                refreshed: now(),
                emotes: ems,
                ..Default::default()
            },
        );
        if new_channel {
            // a new channel goes to the top of the list (channels() puts it there)
            self.state.channel_order = self.channels();
        }
        Ok(ReqResult::ok(format!(
            "Added {n} emotes from {login}'s 7TV channel."
        )))
    }

    /// New channel priority from the addon: logins separated by commas, highest first. Channels
    /// the request doesn't mention (added after the addon sent it) stay on top.
    fn req_order(&mut self, reference: &str) -> ReqResult {
        let current = self.channels();
        let mut want: Vec<String> = Vec::new();
        for login in reference.split(',').map(|l| l.trim().to_lowercase()) {
            if current.contains(&login) && !want.contains(&login) {
                want.push(login);
            }
        }
        if want.is_empty() {
            return ReqResult::fail("None of those channels are in your list any more.");
        }
        let mut order: Vec<String> = current.into_iter().filter(|l| !want.contains(l)).collect();
        order.extend(want);
        self.state.channel_order = order;
        ReqResult::ok(format!(
            "Channel priority saved: {}.",
            self.channels().join(" > ")
        ))
    }

    /// Adjust a request's message after textures were built, then record it.
    pub fn finish_request(
        &mut self,
        req_id: &str,
        result: ReqResult,
        build: &BuildResult,
    ) -> Processed {
        let mut r = result;
        if r.ok {
            if let Some(name) = r.name.clone() {
                if let Some(em) = self.desired().get(&name) {
                    if build.waiting.contains(&em.id) {
                        r.msg = format!(
                            "{name} is ready, but WoW needs a restart to show it (out of reserved slots)."
                        );
                    } else if build.failed.contains(&em.id) {
                        let err = self
                            .state
                            .failed
                            .get(&em.id)
                            .map(|f| f.err.clone())
                            .unwrap_or_else(|| "unknown error".into());
                        r = ReqResult {
                            ok: false,
                            msg: format!("Couldn't convert {name}: {err}"),
                            name: Some(name),
                        };
                    }
                }
            } else if !build.waiting.is_empty() {
                r.msg = format!(
                    "{} {} will show after you restart WoW.",
                    r.msg,
                    build.waiting.len()
                );
            }
        }
        let p = Processed {
            ok: r.ok,
            msg: r.msg,
            name: r.name,
            t: now(),
        };
        self.state.processed.insert(req_id.to_string(), p.clone());
        p
    }

    pub fn prune(&mut self) {
        let cutoff = now() - 60.0 * 86400.0;
        self.state.processed.retain(|_, v| v.t >= cutoff);
    }
}

fn friendly_not_found(kind: &str, reference: &str, msg: &str) -> String {
    if kind == "channel" || kind == "unchannel" {
        if msg.contains("7TV") || msg.contains("Twitch") {
            return msg.to_string();
        }
        return format!("Couldn't find {reference} on 7TV.");
    }
    "Couldn't find that emote on 7TV. Check the link.".into()
}
