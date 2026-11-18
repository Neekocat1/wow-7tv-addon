//! End-to-end tests for the helper against a fake WoW install and a fake 7TV.
//! Port of the Python helper's test suite (tests/test_helper.py, in the git history), test for test.
//!
//! Run: cargo test

use serde_json::{Value, json};
use seventv_helper::images;
use seventv_helper::instance::{self, SingleInstance};
use seventv_helper::library::Library;
use seventv_helper::luasv::{self, Key, TableBuilder, Value as Lua};
use seventv_helper::now;
use seventv_helper::paths::{AppPaths, Config};
use seventv_helper::service::Service;
use seventv_helper::seventv::{self, ApiError, ApiResult, Http};
use seventv_helper::status_board as sb;
use seventv_helper::testutil::TempDir;
use seventv_helper::wow::GameProc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ANIM: &str = "01ANIMATEDXXXXXXXXXXXXXXXX";
const STAT: &str = "01STATICXXXXXXXXXXXXXXXXXX";
const WIDE: &str = "60ae958e229664e8667aea38";
const CHAN: &str = "01CHANNELEMOTEXXXXXXXXXXXX";
const ZW: &str = "01ZEROWIDTHXXXXXXXXXXXXXXX";

fn host(eid: &str, fname: &str, w: u32, h: u32, fmt: &str) -> Value {
    json!({"url": format!("//cdn.test/emote/{eid}"), "files": [{"name": fname, "width": w, "height": h, "format": fmt}]})
}

fn active(eid: &str, name: &str, animated: bool, h: &Value, flags: i64) -> Value {
    json!({"id": eid, "name": name, "flags": 0, "data": {"id": eid, "name": name, "animated": animated, "flags": flags, "host": h}})
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

struct FakeHttp {
    online: AtomicBool,
    calls: Mutex<Vec<String>>,
    files: HashMap<String, PathBuf>,
    hosts: HashMap<&'static str, Value>,
    glob: Value,
    channel: Value,
    /// a second channel ("other") whose testerHi is a different emote than tester's
    other: Value,
}

impl FakeHttp {
    fn new() -> Self {
        let mut hosts = HashMap::new();
        hosts.insert(ANIM, host(ANIM, "2x.webp", 64, 64, "WEBP"));
        hosts.insert(STAT, host(STAT, "2x.webp", 64, 64, "WEBP"));
        hosts.insert(WIDE, host(WIDE, "2x.png", 128, 32, "PNG"));
        hosts.insert(CHAN, host(CHAN, "2x.webp", 64, 64, "WEBP"));
        hosts.insert(ZW, host(ZW, "2x.webp", 64, 64, "WEBP"));
        let mut files = HashMap::new();
        files.insert(
            format!("https://cdn.test/emote/{ANIM}/2x.webp"),
            fixture("anim.webp"),
        );
        files.insert(
            format!("https://cdn.test/emote/{STAT}/2x.webp"),
            fixture("static.webp"),
        );
        files.insert(
            format!("https://cdn.test/emote/{WIDE}/2x.png"),
            fixture("wide.png"),
        );
        files.insert(
            format!("https://cdn.test/emote/{CHAN}/2x.webp"),
            fixture("static.webp"),
        );
        let glob = json!({"name": "Global", "emotes": [
            active(ANIM, "catJAM", true, &hosts[ANIM], 0),
            active(STAT, "OMEGALUL", false, &hosts[STAT], 0),
            active(ZW, "RainTime", false, &hosts[ZW], 256),
        ]});
        let channel = json!({"emote_set": {"id": "SET1", "name": "tester's set", "emotes": [
            active(CHAN, "testerHi", false, &hosts[CHAN], 0),
            active(STAT, "Clap", false, &hosts[STAT], 0),
        ]}});
        let other = json!({"emote_set": {"id": "SET2", "name": "other's set", "emotes": [
            active(WIDE, "testerHi", false, &hosts[WIDE], 0),
        ]}});
        Self {
            online: AtomicBool::new(true),
            calls: Mutex::new(Vec::new()),
            files,
            hosts,
            glob,
            channel,
            other,
        }
    }

    fn enter(&self, url: &str) -> ApiResult<()> {
        self.calls.lock().unwrap().push(url.to_string());
        if !self.online.load(Ordering::SeqCst) {
            return Err(ApiError::Network("Couldn't reach 7TV (offline)".into()));
        }
        Ok(())
    }

    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl Http for FakeHttp {
    fn get_json(&self, url: &str) -> ApiResult<Value> {
        self.enter(url)?;
        if url.ends_with("/emote-sets/global") {
            return Ok(self.glob.clone());
        }
        if url.ends_with("/users/twitch/123") {
            return Ok(self.channel.clone());
        }
        if url.ends_with("/users/twitch/456") {
            return Ok(self.other.clone());
        }
        if url.contains("ivr.fi") {
            if url.ends_with("tester") {
                return Ok(json!([{"id": "123", "login": "tester"}]));
            }
            if url.ends_with("other") {
                return Ok(json!([{"id": "456", "login": "other"}]));
            }
            return Ok(json!([]));
        }
        if url.ends_with(&format!("/emotes/{WIDE}")) {
            return Ok(
                json!({"id": WIDE, "name": "WideDank", "animated": false, "host": self.hosts[WIDE]}),
            );
        }
        Err(ApiError::NotFound(url.to_string()))
    }

    fn get_bytes(&self, url: &str) -> ApiResult<Vec<u8>> {
        self.enter(url)?;
        match self.files.get(url) {
            Some(p) => Ok(std::fs::read(p).unwrap()),
            None => Err(ApiError::NotFound(url.to_string())),
        }
    }

    fn post_json(&self, url: &str, body: &Value) -> ApiResult<Value> {
        self.enter(url)?;
        if url == seventv::GQL {
            if body["variables"]["query"] == "peepoHappy" {
                return Ok(json!({"data": {"emotes": {"items": [
                    {"id": WIDE, "name": "peepoHappy", "animated": false, "host": self.hosts[WIDE]}
                ]}}}));
            }
            return Ok(json!({"data": {"emotes": {"items": []}}}));
        }
        Err(ApiError::NotFound(url.to_string()))
    }
}

type Procs = Arc<Mutex<Vec<GameProc>>>;

struct Fake {
    _tmp: TempDir,
    root: PathBuf,
    forever: PathBuf,
    http: Arc<FakeHttp>,
    procs: Procs,
    paths: AppPaths,
}

fn fake() -> Fake {
    let tmp = TempDir::new("helper");
    let root = tmp.path().join("World of Warcraft");
    let forever = root.join("_forever_");
    std::fs::create_dir_all(forever.join("Interface").join("AddOns")).unwrap();
    std::fs::create_dir_all(
        forever
            .join("WTF")
            .join("Account")
            .join("ACCT1")
            .join("SavedVariables"),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("_retail_").join("Interface").join("AddOns")).unwrap();
    let paths = AppPaths::new(tmp.path().join("appdata"));
    Fake {
        _tmp: tmp,
        root,
        forever,
        http: Arc::new(FakeHttp::new()),
        procs: Arc::new(Mutex::new(Vec::new())),
        paths,
    }
}

fn make_service(f: &Fake) -> Service {
    let cfg = Config {
        wow_dirs: vec![f.root.to_string_lossy().into_owned()],
        ..Default::default()
    };
    let procs = f.procs.clone();
    Service::new(
        cfg,
        f.paths.clone(),
        f.http.clone(),
        Box::new(move |_roots: &[PathBuf]| Some(procs.lock().unwrap().clone())),
    )
}

fn set_procs(f: &Fake, list: Vec<GameProc>) {
    *f.procs.lock().unwrap() = list;
}

fn wow_proc(pid: u32, name: &str, started: f64) -> GameProc {
    GameProc {
        pid,
        name: name.into(),
        exe: String::new(),
        started: Some(started),
    }
}

fn sv_path(f: &Fake) -> PathBuf {
    f.forever
        .join("WTF")
        .join("Account")
        .join("ACCT1")
        .join("SavedVariables")
        .join("SevenTVEmotes.lua")
}

fn set_mtime(p: &Path, t: f64) {
    let file = std::fs::OpenOptions::new().write(true).open(p).unwrap();
    file.set_modified(UNIX_EPOCH + Duration::from_secs_f64(t))
        .unwrap();
}

fn req(id: &str, kind: &str, reference: &str) -> Lua {
    TableBuilder::new()
        .set("id", id)
        .set("kind", kind)
        .set("ref", reference)
        .set("t", 1i64)
        .build()
}

fn req_alias(id: &str, kind: &str, reference: &str, alias: &str) -> Lua {
    TableBuilder::new()
        .set("id", id)
        .set("kind", kind)
        .set("ref", reference)
        .set("alias", alias)
        .set("t", 1i64)
        .build()
}

fn write_sv(f: &Fake, queue: Vec<Lua>) -> PathBuf {
    let n = queue.len();
    let db = TableBuilder::new()
        .set("enabled", true)
        .set("size", 28i64)
        .set("queue", Lua::List(queue))
        .set("disabled", Lua::Table(vec![]))
        .build();
    let sv = sv_path(f);
    std::fs::write(
        &sv,
        format!("SevenTVEmotesDB = {}\n", luasv::lua_value(&db, "")),
    )
    .unwrap();
    set_mtime(&sv, now() + n as f64); // make sure mtime changes between writes
    sv
}

fn write_sv_db(f: &Fake, db: Lua) -> f64 {
    let sv = sv_path(f);
    let text = format!("SevenTVEmotesDB = {}\n", luasv::lua_value(&db, ""));
    let t = now() + 100.0 + luasv::lua_value(&db, "").len() as f64;
    std::fs::write(&sv, text).unwrap();
    set_mtime(&sv, t);
    t
}

fn compress_db(on: bool) -> Lua {
    TableBuilder::new()
        .set("compress", on)
        .set("queue", Lua::List(vec![]))
        .build()
}

fn local_dir(f: &Fake) -> PathBuf {
    f.forever
        .join("Interface")
        .join("AddOns")
        .join("SevenTVEmotes_Local")
}

fn local_lua(f: &Fake) -> String {
    std::fs::read_to_string(local_dir(f).join("Local.lua")).unwrap()
}

fn state(f: &Fake) -> Value {
    serde_json::from_slice(&std::fs::read(local_dir(f).join("helper_state.json")).unwrap()).unwrap()
}

fn lib(svc: &mut Service) -> &mut Library {
    svc.libs.values_mut().next().unwrap()
}

/// `svc.process_library(lib, ...)` for the first library.
fn process_first(
    svc: &mut Service,
    gs: Option<f64>,
    force: bool,
) -> Vec<seventv_helper::library::Processed> {
    let mut libs = std::mem::take(&mut svc.libs);
    let out = svc.process_library(libs.get_index_mut(0).unwrap().1, gs, force, false, false);
    svc.libs = libs;
    out
}

fn size(p: &Path) -> u64 {
    std::fs::metadata(p).unwrap().len()
}

fn count_ext(dir: &Path, ext: &str) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == ext))
                .count()
        })
        .unwrap_or(0)
}

/// Width/height from a BLP2 header.
fn blp_dims(p: &Path) -> (u32, u32) {
    let b = std::fs::read(p).unwrap();
    assert_eq!(&b[..4], b"BLP2");
    (
        u32::from_le_bytes(b[12..16].try_into().unwrap()),
        u32::from_le_bytes(b[16..20].try_into().unwrap()),
    )
}

// --------------------------------------------------------------------------- tests

#[test]
fn install_and_global() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    let ad = f.forever.join("Interface").join("AddOns");
    assert!(ad.join("SevenTVEmotes").join("SevenTVEmotes.toc").exists());
    assert!(ad.join("SevenTVEmotes").join("Core.lua").exists());
    assert!(
        ad.join("SevenTVEmotes_Local")
            .join("SevenTVEmotes_Local.toc")
            .exists()
    );
    assert!(
        !f.root
            .join("_retail_")
            .join("Interface")
            .join("AddOns")
            .join("SevenTVEmotes")
            .exists(),
        "retail untouched"
    );
    let local = ad.join("SevenTVEmotes_Local");
    assert!(count_ext(&local.join("slots"), "tga") >= 1500);
    assert!(count_ext(&local.join("cslots"), "blp") >= 1500);
    assert_eq!(
        count_ext(&local.join("slots"), "blp"),
        0,
        "no same-name twins"
    );
    let mut test_files: Vec<String> = std::fs::read_dir(local.join("test"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    test_files.sort();
    assert_eq!(
        test_files,
        [
            "t_argb.blp",
            "t_dxt5.blp",
            "t_dxt5m.blp",
            "t_pal.blp",
            "t_tga.tga"
        ]
    );
    let lua = local_lua(&f);
    assert!(
        lua.contains(r#"["catJAM"] = { "slots\\s0001.tga""#)
            || lua.contains(r#"["catJAM"] = { "slots\\s0002.tga""#)
    );
    assert!(
        lua.contains("storage = {")
            && lua.contains("origin = {")
            && lua.contains("7TV global emotes")
    );
    assert!(lua.contains(r#"["OMEGALUL"]"#));
    assert!(!lua.contains("RainTime"), "zero-width skipped");
    let st = state(&f);
    assert_eq!(st["images"].as_object().unwrap().len(), 2);
    let anim = &st["images"][ANIM];
    assert!(anim["frames"].as_u64().unwrap() > 1 && anim["cols"].as_u64().unwrap() >= 1);
    // the slot file was overwritten with a real texture
    let fmt = anim["fmt"].as_str().unwrap();
    let file = local
        .join(if fmt == "blp" { "cslots" } else { "slots" })
        .join(format!("{}.{fmt}", anim["slot"].as_str().unwrap()));
    let im = image::open(&file).unwrap();
    assert_eq!(
        (im.width() as u64, im.height() as u64),
        (anim["tw"].as_u64().unwrap(), anim["th"].as_u64().unwrap())
    );
}

#[test]
fn requests() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    write_sv(
        &f,
        vec![
            req("r1", "add", &format!("https://7tv.app/emotes/{WIDE}")),
            req("r2", "add", "peepoHappy"),
            req("r3", "channel", "tester"),
            req(
                "r4",
                "add",
                "https://7tv.app/emotes/01NOPEXXXXXXXXXXXXXXXXXXXX",
            ),
            req("r5", "remove", "OMEGALUL"),
            req_alias("r6", "add", "catJAM", "dance"),
            req("r7", "channel", "nobody"),
            req("r8", "add", "NotARealEmote"),
        ],
    );
    let results = svc.run_once();
    assert_eq!(results.len(), 8);
    let st = state(&f);
    let p = &st["processed"];
    assert!(p["r1"]["ok"].as_bool().unwrap() && p["r1"]["name"] == "WideDank");
    assert!(p["r2"]["ok"].as_bool().unwrap() && p["r2"]["name"] == "peepoHappy");
    assert!(
        p["r3"]["ok"].as_bool().unwrap() && p["r3"]["msg"].as_str().unwrap().contains("2 emotes")
    );
    assert!(!p["r4"]["ok"].as_bool().unwrap());
    assert!(p["r5"]["ok"].as_bool().unwrap());
    assert!(p["r6"]["ok"].as_bool().unwrap() && p["r6"]["name"] == "dance");
    assert!(!p["r7"]["ok"].as_bool().unwrap());
    assert!(
        !p["r8"]["ok"].as_bool().unwrap() && p["r8"]["msg"].as_str().unwrap().contains("7tv.app")
    );
    let lua = local_lua(&f);
    for name in [
        "WideDank",
        "peepoHappy",
        "testerHi",
        "Clap",
        "dance",
        "catJAM",
    ] {
        assert!(lua.contains(&format!("[\"{name}\"]")), "{name}");
    }
    assert!(!lua.contains(r#"["OMEGALUL"]"#));
    // results are exposed to the addon
    assert!(lua.contains("r1 = ") && lua.contains("Added WideDank."));
    // WideDank and peepoHappy share a 7TV id -> one texture
    assert_eq!(
        st["images"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| *k == WIDE)
            .count(),
        1
    );
    if let Ok(dump) = std::env::var("DUMP_LOCAL") {
        std::fs::write(dump, &lua).unwrap();
    }
    // reprocessing the same queue does nothing new
    assert!(svc.run_once().is_empty());
}

#[test]
fn unchannel_and_readd() {
    let f = fake();
    let mut svc = make_service(&f);
    write_sv(&f, vec![req("a", "channel", "tester")]);
    svc.run_once();
    assert!(local_lua(&f).contains(r#"["testerHi"]"#));
    let slot = state(&f)["images"][CHAN]["slot"]
        .as_str()
        .unwrap()
        .to_string();
    write_sv(
        &f,
        vec![
            req("a", "channel", "tester"),
            req("b", "unchannel", "tester"),
        ],
    );
    svc.run_once();
    let st = state(&f);
    assert!(!local_lua(&f).contains(r#"["testerHi"]"#));
    assert!(st["images"].get(CHAN).is_none());
    assert!(st["slots"][&slot]["id"].is_null() && st["slots"][&slot]["freed"].as_f64().is_some());
}

/// The 7TV id the addon gets for an emote name, from Local.lua's emote table.
fn emote_id(lua: &str, name: &str) -> Option<String> {
    let line = lua
        .lines()
        .find(|l| l.trim_start().starts_with(&format!("[\"{name}\"] = {{")))?;
    [ANIM, STAT, WIDE, CHAN]
        .iter()
        .find(|id| line.contains(*id))
        .map(|id| id.to_string())
}

#[test]
fn channel_priority_decides_same_name_emotes() {
    let f = fake();
    let mut svc = make_service(&f);
    // a newly added channel goes to the top of the list, so its testerHi wins
    let mut queue = vec![req("a", "channel", "tester"), req("b", "channel", "other")];
    write_sv(&f, queue.clone());
    svc.run_once();
    assert_eq!(emote_id(&local_lua(&f), "testerHi").as_deref(), Some(WIDE));
    assert_eq!(state(&f)["channel_order"], json!(["other", "tester"]));
    assert!(
        local_lua(&f).contains(r#""other", "tester""#),
        "status lists channels by priority"
    );

    // move tester up: its testerHi wins now
    queue.push(req("c", "order", "tester,other"));
    write_sv(&f, queue.clone());
    svc.run_once();
    assert_eq!(emote_id(&local_lua(&f), "testerHi").as_deref(), Some(CHAN));
    assert_eq!(state(&f)["channel_order"], json!(["tester", "other"]));
    let msg = state(&f)["processed"]["c"]["msg"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(msg.contains("tester > other"), "{msg}");

    // re-adding a channel (a refresh) keeps its place; an order naming unknown channels is
    // tidied up, and channels it leaves out stay on top
    queue.push(req("d", "channel", "other"));
    queue.push(req("e", "order", "nobody, Other"));
    write_sv(&f, queue.clone());
    svc.run_once();
    assert_eq!(state(&f)["channel_order"], json!(["tester", "other"]));
    assert_eq!(emote_id(&local_lua(&f), "testerHi").as_deref(), Some(CHAN));
    queue.push(req("f", "order", "nobody"));
    write_sv(&f, queue.clone());
    svc.run_once();
    assert_eq!(state(&f)["processed"]["f"]["ok"], json!(false));

    // removing the winner hands the name to the next channel
    queue.push(req("g", "unchannel", "tester"));
    write_sv(&f, queue);
    svc.run_once();
    assert_eq!(state(&f)["channel_order"], json!(["other"]));
    assert_eq!(emote_id(&local_lua(&f), "testerHi").as_deref(), Some(WIDE));
}

#[test]
fn channels_without_a_saved_order_rank_newest_first() {
    // helper_state.json from before 0.7 has no channel_order: the newest channel wins, as before
    let f = fake();
    let mut svc = make_service(&f);
    write_sv(
        &f,
        vec![req("a", "channel", "tester"), req("b", "channel", "other")],
    );
    svc.run_once();
    {
        let lib = lib(&mut svc);
        lib.state.channel_order.clear();
        lib.state.sources.get_mut("channel:tester").unwrap().added = now() + 50.0;
        assert_eq!(lib.channels(), vec!["tester", "other"]);
        assert_eq!(lib.desired()["testerHi"].id, CHAN);
    }
}

#[test]
fn slots_respect_running_game() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once(); // installs + global, slots created "before the game started"
    let started = now() + 1.0;
    set_procs(&f, vec![wow_proc(1, "Wow.exe", started)]);
    // Pretend every remaining free slot was created after the game started.
    {
        let lib = lib(&mut svc);
        for s in lib.state.slots.values_mut() {
            if s.id.is_none() {
                s.created = started + 5.0;
            }
        }
        lib.save().unwrap();
    }
    write_sv(
        &f,
        vec![req("w1", "add", &format!("https://7tv.app/emotes/{WIDE}"))],
    );
    let res = svc.run_once();
    assert!(
        !res.is_empty() && res[0].ok && res[0].msg.contains("restart"),
        "{res:?}"
    );
    assert!(!local_lua(&f).contains(r#"["WideDank"]"#));
    // After the game restarts (not running), the next pass builds it.
    set_procs(&f, vec![]);
    svc.request_refresh();
    process_first(&mut svc, None, true);
    assert!(local_lua(&f).contains(r#"["WideDank"]"#));
}

#[test]
fn offline_retries_then_fails() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    f.http.online.store(false, Ordering::SeqCst);
    write_sv(
        &f,
        vec![req("o1", "add", &format!("https://7tv.app/emotes/{WIDE}"))],
    );
    assert!(svc.run_once().is_empty()); // attempt 1: retry later
    assert!(svc.run_once().is_empty()); // attempt 2
    let res = svc.run_once(); // attempt 3: give up with a message
    assert!(!res.is_empty() && !res[0].ok && res[0].msg.contains("reach 7TV"));
}

#[test]
fn compression_switch_and_cleanup() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    let slots = lib(&mut svc).slots_dir.clone();
    let cslots = lib(&mut svc).cslots_dir.clone();
    let st0 = state(&f);
    let tga_slot = st0["images"][STAT]["slot"].as_str().unwrap().to_string();
    let size_tga = size(&slots.join(format!("{tga_slot}.tga")));
    assert!(size_tga > 1000);
    // Game running, slots existed before it started: conversion can happen now.
    let started = now() + 1.0;
    set_procs(&f, vec![wow_proc(1, "Wow.exe", started)]);
    write_sv_db(&f, compress_db(true));
    svc.run_once();
    let st = state(&f);
    assert_eq!(st["format"], "blp");
    assert!(
        st["images"]
            .as_object()
            .unwrap()
            .values()
            .all(|i| i["fmt"] == "blp")
    );
    let lua = local_lua(&f);
    let emotes_part = lua.split("local emotes").nth(1).unwrap();
    assert!(lua.contains(".blp\"") && !emotes_part.contains(".tga\""));
    // BLP has the original layout
    let info = &st["images"][ANIM];
    let dims = blp_dims(&cslots.join(format!("{}.blp", info["slot"].as_str().unwrap())));
    assert_eq!(
        (dims.0 as u64, dims.1 as u64),
        (info["tw"].as_u64().unwrap(), info["th"].as_u64().unwrap())
    );
    // old .tga files are kept until the addon has reloaded past the switch
    assert_eq!(size(&slots.join(format!("{tga_slot}.tga"))), size_tga);
    assert!(
        !st["cleanup"]["files"].as_array().unwrap().is_empty()
            && st["cleanup"]["after"].as_f64().is_some()
    );
    // next reload (newer saved variables) -> old files blanked
    let later = st["cleanup"]["after"].as_f64().unwrap() + 10.0;
    set_mtime(&sv_path(&f), later);
    svc.run_once();
    assert!(size(&slots.join(format!("{tga_slot}.tga"))) < 400);
    let st2 = state(&f);
    assert!(st2["cleanup"]["files"].as_array().unwrap().is_empty());
    let storage = lib(&mut svc).storage(None);
    assert_eq!(storage.get("format").unwrap().as_str(), Some("blp"));
    let (Some(Lua::Int(blp)), Some(Lua::Int(tga))) = (storage.get("blp"), storage.get("tga"))
    else {
        panic!()
    };
    assert!(blp < tga);
}

#[test]
fn compression_waits_for_restart_when_slots_are_new() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    let started = now() + 1.0;
    set_procs(&f, vec![wow_proc(1, "Wow.exe", started)]);
    {
        let lib = lib(&mut svc);
        for s in lib.state.slots.values_mut() {
            s.created_blp = Some(started + 5.0); // .blp files appeared after the game started
        }
        lib.save().unwrap();
    }
    write_sv_db(&f, compress_db(true));
    svc.run_once();
    let st = state(&f);
    let n = st["images"].as_object().unwrap().len() as u64;
    assert!(st["format"] == "blp" && st["converting"].as_u64().unwrap() == n);
    assert!(
        st["images"]
            .as_object()
            .unwrap()
            .values()
            .all(|i| i["fmt"] == "tga")
    );
    assert!(local_lua(&f).contains(&format!("converting = {n}")));
}

#[test]
fn retrying_is_reported() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    f.http.online.store(false, Ordering::SeqCst);
    write_sv(&f, vec![req("q1", "channel", "tester")]);
    svc.run_once();
    let lua = local_lua(&f);
    assert!(lua.contains("retrying = {") && lua.contains("q1 = "));
}

#[test]
fn first_run_offline_then_global_arrives() {
    let f = fake();
    f.http.online.store(false, Ordering::SeqCst);
    let mut svc = make_service(&f);
    svc.run_once();
    let lua = local_lua(&f);
    assert!(lua.contains("Couldn't download 7TV's global emotes yet"));
    assert!(!lua.contains(r#"["catJAM"]"#));
    // retried again only after a minute
    let calls = f.http.calls();
    svc.run_once();
    assert_eq!(f.http.calls(), calls);
    {
        let g = lib(&mut svc).state.sources.get_mut("global").unwrap();
        *g.last_attempt.as_mut().unwrap() -= 61.0;
    }
    f.http.online.store(true, Ordering::SeqCst);
    svc.run_once();
    let lua = local_lua(&f);
    assert!(lua.contains(r#"["catJAM"]"#) && !lua.contains("Couldn't download"));
}

#[test]
fn progress_reported() {
    let f = fake();
    let mut svc = make_service(&f);
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    *svc.shared.on_status.lock().unwrap() =
        Box::new(move |t: &str| s2.lock().unwrap().push(t.to_string()));
    svc.run_once();
    let seen = seen.lock().unwrap();
    assert!(
        seen.iter().any(|s| s == "Downloading emotes 0/2")
            && seen.iter().any(|s| s == "Downloading emotes 2/2"),
        "{seen:?}"
    );
}

#[test]
fn blp_has_mips_and_estimate_matches() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    write_sv_db(&f, compress_db(true));
    svc.run_once();
    let lib = lib(&mut svc);
    for info in lib.state.images.values() {
        let file = lib.cslots_dir.join(format!("{}.blp", info.slot));
        assert_eq!(size(&file), images::blp_size(info.tw, info.th));
        let raw = std::fs::read(&file).unwrap();
        assert_eq!(&raw[..4], b"BLP2");
        assert_eq!(&raw[8..12], &[2, 8, 7, 1]);
    }
}

#[test]
fn waiting_emotes_built_when_wow_restarts() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    let started = now() + 1.0;
    set_procs(&f, vec![wow_proc(1, "WowForever.exe", started)]);
    for s in lib(&mut svc).state.slots.values_mut() {
        if s.id.is_none() {
            s.created = started + 5.0;
        }
    }
    write_sv(
        &f,
        vec![req("w9", "add", &format!("https://7tv.app/emotes/{WIDE}"))],
    );
    svc.tick();
    assert!(!local_lua(&f).contains(r#"["WideDank"]"#));
    // WoW restarts: new process, started after the slots were made
    set_procs(&f, vec![wow_proc(2, "WowForever.exe", started + 60.0)]);
    svc.tick();
    assert!(local_lua(&f).contains(r#"["WideDank"]"#));
}

#[test]
fn old_blps_without_mips_are_reencoded() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    write_sv_db(&f, compress_db(true));
    svc.run_once();
    {
        let lib = lib(&mut svc);
        for info in lib.state.images.values_mut() {
            info.mips = None; // pretend these came from 0.3.0
        }
        lib.save().unwrap();
    }
    svc.request_refresh();
    process_first(&mut svc, None, true);
    let lib = lib(&mut svc);
    for info in lib.state.images.values() {
        let file = lib.cslots_dir.join(format!("{}.blp", info.slot));
        assert!(info.mips == Some(true) && std::fs::read(&file).unwrap()[11] == 1);
        assert_eq!(size(&file), images::blp_size(info.tw, info.th));
    }
}

#[test]
fn status_board_written_ahead_and_ready_message() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    let status = lib(&mut svc).dir.join("status");
    assert_eq!(count_ext(&status, "tga") as u64, sb::COUNT);
    let lua = local_lua(&f);
    assert!(lua.contains("board = {") && lua.contains(&format!("count = {}", sb::COUNT)));
    let t = now();
    let k = sb::index_at(t);
    let big: Vec<u64> = (0..sb::COUNT)
        .filter(|i| size(&status.join(format!("st{i:04}.tga"))) > 1000)
        .collect();
    assert!(big.contains(&k) || big.contains(&((k + 1) % sb::COUNT)));
    assert!(big.len() as u64 <= sb::AHEAD + 3);
    // new request -> after processing the board says "ready - click Reload"
    write_sv(
        &f,
        vec![req("b1", "add", &format!("https://7tv.app/emotes/{WIDE}"))],
    );
    lib(&mut svc).latest_sv_mtime = 0.0;
    set_mtime(&sv_path(&f), t - 5.0);
    svc.sv_mtimes.clear();
    svc.run_once();
    let (text, kind) = {
        let l = svc.libs.values().next().unwrap();
        svc.board_text(l)
    };
    assert!(kind == "ready" && text.contains("click Reload"), "{text}");
    // the player reloads (saved variables newer than the ready time) -> back to idle
    set_mtime(&sv_path(&f), now() + 30.0);
    svc.run_once();
    let (text, kind) = svc.board_text(svc.libs.values().next().unwrap());
    assert!(kind == "idle" && text.contains("Up to date"), "{text}");
}

#[test]
fn migration_moves_compressed_out_of_slots() {
    // 0.4 layout: s0001.tga + s0001.blp twins in slots/. 0.5 moves .blp to cslots/ and rebuilds.
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    write_sv_db(&f, compress_db(true));
    svc.run_once();
    {
        let lib = lib(&mut svc);
        // Recreate the old layout
        for name in lib.state.slots.keys() {
            let src = lib.cslots_dir.join(format!("{name}.blp"));
            std::fs::copy(&src, lib.slots_dir.join(format!("{name}.blp"))).unwrap();
        }
        for info in lib.state.images.values_mut() {
            info.fmt = "blp".into();
        }
        lib.state.layout = Some(1);
        lib.save().unwrap();
        std::fs::remove_dir_all(&lib.cslots_dir).unwrap();
    }
    // WoW is running: the new cslots/ files were created after it started.
    set_procs(&f, vec![wow_proc(1, "Wow.exe", now() - 60.0)]);
    let mut svc2 = make_service(&f);
    svc2.run_once();
    {
        let lib2 = lib(&mut svc2);
        assert_eq!(count_ext(&lib2.slots_dir, "blp"), 0);
        assert!(
            lib2.state
                .images
                .values()
                .all(|i| i.fmt == "tga" && !i.rebuild)
        );
        let lua = local_lua(&f);
        assert!(
            lua.contains("slots\\\\s")
                && lua.split("local emotes").nth(1).unwrap().contains(".tga")
        );
        assert_eq!(lib2.state.converting, lib2.state.images.len() as u64);
    }
    // WoW restarts -> converted into cslots/
    set_procs(&f, vec![wow_proc(2, "Wow.exe", now() + 5.0)]);
    svc2.tick();
    let lua = local_lua(&f);
    assert!(
        lua.split("local emotes")
            .nth(1)
            .unwrap()
            .contains("cslots\\\\s")
    );
    assert!(lib(&mut svc2).state.images.values().all(|i| i.fmt == "blp"));
}

#[test]
fn old_animations_are_rebuilt_into_new_slots() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once(); // global emotes, catJAM animated
    // as if made by a helper before 0.7.1 (no anim version)
    let old_slot = {
        let lib = lib(&mut svc);
        let info = lib.state.images.get_mut(ANIM).unwrap();
        assert!(info.frames > 1 && info.anim == Some(images::ANIM_VERSION));
        info.anim = None;
        info.slot.clone()
    };
    assert!(lib(&mut svc).upgrades_pending(None));
    svc.run_once();
    let info = {
        let l = lib(&mut svc);
        let info = l.state.images[ANIM].clone();
        assert_ne!(info.slot, old_slot, "rebuilt into another slot");
        assert_eq!(info.anim, Some(images::ANIM_VERSION));
        assert_eq!(l.state.slots[&info.slot].id.as_deref(), Some(ANIM));
        assert!(l.state.slots[&old_slot].id.is_none() && l.state.slots[&old_slot].freed.is_some());
        let old_file = l.slot_file(&old_slot, &info.fmt);
        assert!(
            std::fs::metadata(&old_file).unwrap().len() < 1000,
            "old slot blanked"
        );
        assert!(l.ready.as_ref().is_some_and(|r| r.msg.contains("Smoother")));
        assert!(!l.upgrades_pending(None), "nothing left to do");
        info
    };
    assert!(local_lua(&f).contains(&format!("{}.tga", info.slot)));
    // a pass with nothing to upgrade leaves it alone
    svc.run_once();
    assert_eq!(lib(&mut svc).state.images[ANIM].slot, info.slot);
}

#[test]
fn new_emotes_fall_back_to_tga_until_blp_visible() {
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    write_sv_db(&f, compress_db(true));
    svc.run_once();
    let started = now() + 1.0;
    set_procs(&f, vec![wow_proc(1, "Wow.exe", started)]);
    {
        let lib = lib(&mut svc);
        for s in lib.state.slots.values_mut() {
            if s.id.is_none() {
                s.created_blp = Some(started + 5.0); // compressed placeholders not visible yet
            }
        }
        lib.save().unwrap();
    }
    write_sv(
        &f,
        vec![req("f1", "add", &format!("https://7tv.app/emotes/{WIDE}"))],
    );
    svc.run_once();
    assert_eq!(lib(&mut svc).state.images[WIDE].fmt, "tga");
    assert!(local_lua(&f).contains(r#"["WideDank"] = { "slots\\"#));
}

#[test]
fn newer_helper_takes_over() {
    let tmp = TempDir::new("takeover");
    let paths = AppPaths::new(tmp.path().join("appdata"));
    let mut old = SingleInstance::new(&paths);
    assert!(old.acquire());
    instance::write_running(&paths, "0.4.0");
    let quit_seen = Arc::new(AtomicBool::new(false));
    let (p2, q2) = (paths.clone(), quit_seen.clone());
    let t = std::thread::spawn(move || {
        // what Service::tick does
        while !instance::quit_requested(&p2) {
            std::thread::sleep(Duration::from_millis(100));
        }
        q2.store(true, Ordering::SeqCst);
        old.release();
    });
    let mut new = SingleInstance::new(&paths);
    let (got, other) = instance::take_over(&paths, &mut new, "0.5.0", Duration::from_secs(5));
    t.join().unwrap();
    assert!(got && other.as_deref() == Some("0.4.0") && quit_seen.load(Ordering::SeqCst));
    assert!(!instance::quit_requested(&paths));
    new.release();
}

#[test]
fn older_helper_does_not_replace_newer() {
    let tmp = TempDir::new("older");
    let paths = AppPaths::new(tmp.path().join("appdata"));
    let mut running = SingleInstance::new(&paths);
    assert!(running.acquire());
    instance::write_running(&paths, "0.6.0");
    let (got, other) = instance::take_over(
        &paths,
        &mut SingleInstance::new(&paths),
        "0.5.0",
        Duration::from_secs(1),
    );
    assert!(!got && other.as_deref() == Some("0.6.0") && !instance::quit_requested(&paths));
    running.release();
}

#[test]
fn service_stops_when_asked() {
    let f = fake();
    let mut svc = make_service(&f);
    let stopped = Arc::new(AtomicBool::new(false));
    let s2 = stopped.clone();
    *svc.shared.on_quit.lock().unwrap() = Box::new(move || s2.store(true, Ordering::SeqCst));
    std::fs::write(f.paths.quit_file(), "0.9.0").unwrap();
    svc.tick();
    assert!(stopped.load(Ordering::SeqCst) && svc.shared.stopped());
}

// --------------------------------------------------------------------------- extra coverage (Rust port)

#[test]
fn takes_over_a_python_helper_install() {
    // A helper_state.json as the Python helper 0.5.1 writes it (indent=1, floats, nulls,
    // extra keys) must load, keep its emotes and survive a save.
    let f = fake();
    let mut svc = make_service(&f);
    svc.run_once();
    let dir = local_dir(&f);
    let mut st = state(&f);
    st["some_future_key"] = json!({"x": 1});
    st["slots"]["s0001"]["created"] = json!(1763164800.123456);
    st["images"][STAT]["future"] = json!(true);
    std::fs::write(
        dir.join("helper_state.json"),
        serde_json::to_string_pretty(&st).unwrap(),
    )
    .unwrap();
    let mut svc2 = make_service(&f);
    svc2.run_once();
    lib(&mut svc2).save().unwrap();
    let st2 = state(&f);
    assert_eq!(st2["some_future_key"], json!({"x": 1}));
    assert_eq!(st2["images"][STAT]["future"], json!(true));
    assert_eq!(st2["images"].as_object().unwrap().len(), 2);
}

#[test]
fn animated_gif_renders_a_sprite_sheet() {
    let tmp = TempDir::new("gif");
    let out = tmp.path().join("g.tga");
    let layout = images::render(
        &fixture("anim.gif"),
        &out,
        &images::RenderOptions::default(),
    )
    .unwrap();
    assert_eq!((layout.w, layout.h), (96, 48));
    // delays 10, 30, 50, 70, 90 ms (under 20 ms counts as 100): 340 ms resampled to even 34 ms
    // frames (30 fps at most), so the 100 ms first frame shows three times
    assert_eq!((layout.frames, layout.ms), (10, 34));
    let im = image::open(&out).unwrap().to_rgba8();
    assert_eq!((im.width(), im.height()), (layout.tw, layout.th));
    let frame = |i: u32| {
        let (x, y) = ((i % layout.cols) * layout.w, (i / layout.cols) * layout.h);
        image::imageops::crop_imm(&im, x, y, layout.w, layout.h).to_image()
    };
    assert!(
        frame(0) == frame(1) && frame(1) == frame(2),
        "first frame held for 3"
    );
    assert!(frame(2) != frame(3), "then the second frame");
}

#[test]
fn long_animations_keep_their_speed() {
    // 120 frames at 25 fps (40 ms each): kept whole (the old 48-frame limit made it 10 fps)
    let mut frames = Vec::new();
    for i in 0..120u32 {
        let img = image::RgbaImage::from_pixel(32, 32, image::Rgba([(i * 2) as u8, 0, 0, 255]));
        frames.push(image::Frame::from_parts(
            img,
            0,
            0,
            image::Delay::from_numer_denom_ms(40, 1),
        ));
    }
    let mut gif = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut gif);
        enc.encode_frames(frames).unwrap();
    }
    let (layout, _) = images::render_bytes(&gif, true, &images::RenderOptions::default()).unwrap();
    assert_eq!((layout.frames, layout.ms), (120, 40));
}

#[test]
fn sparse_queue_and_odd_values_are_read() {
    let f = fake();
    let sv = sv_path(&f);
    std::fs::write(
        &sv,
        "SevenTVEmotesDB = {\n[\"queue\"] = {\n[2] = {[\"id\"] = \"b\", [\"kind\"] = \"add\", [\"ref\"] = 123},\n[5] = {[\"id\"] = \"c\", [\"kind\"] = \"remove\", [\"ref\"] = \"x\"},\n},\n}\n",
    )
    .unwrap();
    let (q, db) = seventv_helper::service::read_sv(&sv).unwrap();
    assert_eq!(q.len(), 2);
    assert_eq!(q[0].id, "b");
    assert_eq!(q[0].reference, "123");
    assert!(
        matches!(db.get("queue"), Some(Lua::Table(kv)) if kv.iter().all(|(k, _)| matches!(k, Key::Int(_))))
    );
}

#[allow(dead_code)]
fn unix(t: SystemTime) -> f64 {
    t.duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
}

/// Writes textures for a side-by-side comparison with the Python helper (see README).
#[test]
fn export_textures_for_comparison() {
    let Ok(dir) = std::env::var("EXPORT_TEXTURES") else {
        return;
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (src, name) in [
        ("static.webp", "static"),
        ("anim.webp", "anim"),
        ("wide.png", "wide"),
        ("anim.gif", "gif"),
    ] {
        for ext in ["tga", "blp"] {
            let l = images::render(
                &fixture(src),
                &dir.join(format!("{name}.{ext}")),
                &images::RenderOptions::default(),
            )
            .unwrap();
            if ext == "tga" && name == "anim" {
                let (frames, _) = images::load_frames(&fixture(src)).unwrap();
                for (i, fr) in frames.iter().enumerate() {
                    fr.save(dir.join(format!("anim_frame{i}.png"))).unwrap();
                }
            }
            if ext == "tga" {
                println!(
                    "{name} {} {} {} {} {} {} {}",
                    l.w, l.h, l.tw, l.th, l.frames, l.cols, l.ms
                );
            }
        }
    }
}

#[test]
fn animated_webp_disposal_matches_libwebp() {
    // Frame 1 of the fixture is "dispose to background": its area must be cleared before frame 2
    // (which starts at x=2) is blended in. image-webp skips that unless told the background colour.
    let (frames, durations) = images::load_frames(&fixture("anim.webp")).unwrap();
    assert_eq!(frames.len(), 30);
    assert!(durations.iter().all(|&d| d == 50));
    for i in 1..30u32 {
        let fr = &frames[i as usize];
        let min_x = (0..fr.width())
            .find(|&x| (0..fr.height()).any(|y| fr.get_pixel(x, y)[3] > 0))
            .unwrap();
        assert_eq!(min_x, i, "frame {i} has leftovers from the previous frame");
    }
}

/// Writes sample status images for a visual check (see README).
#[test]
fn export_status_samples() {
    let Ok(dir) = std::env::var("EXPORT_TEXTURES") else {
        return;
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let samples = [
        ("Downloading emotes 37/412", "busy"),
        ("12 new emotes ready - click Reload", "ready"),
        (
            "3 emotes will show after a WoW restart, and a long tail to shrink",
            "warn",
        ),
        ("Up to date - 1234 emotes", "idle"),
    ];
    for (i, (text, kind)) in samples.iter().enumerate() {
        sb::render(text, "7TV Emotes Helper  -  live 21:07:42", kind)
            .save(dir.join(format!("status{i}.png")))
            .unwrap();
    }
}

#[test]
fn tray_icon_draws() {
    for size in [16, 24, 32, 48] {
        let icon = seventv_helper::draw::make_icon(size);
        assert_eq!(icon.dimensions(), (size, size));
        if let Ok(dir) = std::env::var("EXPORT_TEXTURES") {
            icon.save(PathBuf::from(dir).join(format!("icon{size}.png")))
                .unwrap();
        }
        // The background's red channel is 41; anything much redder is the white "7".
        let ink = icon.pixels().filter(|p| p[0] > 120 && p[3] > 200).count();
        assert!(
            ink > (size * size / 30) as usize,
            "the 7 is visible at {size}px ({ink} px)"
        );
    }
}

#[test]
fn helper_replaces_a_downloaded_emote_pack() {
    // A pack from the web builder: .toc, Local.lua with SevenTVEmotes_Pack, textures in pack/,
    // and no helper_state.json.
    let f = fake();
    let dir = local_dir(&f);
    std::fs::create_dir_all(dir.join("pack")).unwrap();
    std::fs::write(
        dir.join("SevenTVEmotes_Local.toc"),
        "## Interface: 16001\n\nLocal.lua\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("Local.lua"),
        "SevenTVEmotes_Pack = { built = 1 }\n",
    )
    .unwrap();
    std::fs::write(dir.join("pack").join("01X.blp"), b"x").unwrap();
    let mut svc = make_service(&f);
    svc.run_once();
    assert!(!dir.join("pack").exists(), "pack textures removed");
    assert!(
        count_ext(&dir.join("slots"), "tga") >= 1500,
        "slots reserved"
    );
    let lua = local_lua(&f);
    assert!(lua.contains("SevenTVEmotes_Helper = {") && !lua.contains("SevenTVEmotes_Pack"));
    assert!(lua.contains(r#"["catJAM"]"#));
    assert!(dir.join("helper_state.json").exists());
}
