//! WebAssembly module for the emote pack builder (web/site). The page fetches from 7TV itself;
//! this module does everything else with the helper's own code, so a pack matches what the helper
//! would build: picking emotes, converting images to WoW textures, writing Local.lua.
//!
//! Plain C ABI, no bindings generator. JS copies input into memory from `alloc`, calls a function,
//! then reads the result from `out_ptr()`/`out_len()`. Functions return 0 on success and 1 on
//! failure (the result is then an error message).
//!
//! - `convert(ptr, len, blp)`: emote file -> 28-byte layout (7 x u32 LE: w, h, texW, texH, frames,
//!   columns, msPerFrame) followed by the texture file.
//! - `call(op, ptr, len)`: JSON in, JSON out; see `run` for the operations.

use indexmap::IndexMap;
use serde_json::{Value, json};
use seventv_helper::images::{self, Layout, RenderOptions};
use seventv_helper::pack::{self, PackEmote, PackInfo};
use seventv_helper::seventv::{self, ApiError, Emote, Ref};
use std::cell::RefCell;

thread_local! {
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn set_out(bytes: Vec<u8>) {
    OUT.with(|o| *o.borrow_mut() = bytes);
}

#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `ptr`/`len` must come from `alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len.max(1)) });
}

#[unsafe(no_mangle)]
pub extern "C" fn out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

fn input<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if len == 0 {
        return &[];
    }
    // SAFETY: JS wrote `len` bytes at `ptr` (from `alloc`) just before the call.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// # Safety
/// `ptr`/`len` must describe bytes written by JS into memory from `alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn convert(ptr: *const u8, len: usize, blp: u32) -> u32 {
    match images::render_bytes(input(ptr, len), blp != 0, &RenderOptions::default()) {
        Ok((l, tex)) => {
            let mut out = Vec::with_capacity(28 + tex.len());
            for v in [l.w, l.h, l.tw, l.th, l.frames, l.cols, l.ms] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&tex);
            set_out(out);
            0
        }
        Err(e) => {
            set_out(e.into_bytes());
            1
        }
    }
}

/// # Safety
/// `ptr`/`len` must describe UTF-8 JSON written by JS into memory from `alloc`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn call(op: u32, ptr: *const u8, len: usize) -> u32 {
    let result = serde_json::from_slice::<Value>(input(ptr, len))
        .map_err(|e| format!("bad input: {e}"))
        .and_then(|v| run(op, &v));
    match result {
        Ok(v) => {
            set_out(serde_json::to_vec(&v).unwrap_or_default());
            0
        }
        Err(e) => {
            set_out(e.into_bytes());
            1
        }
    }
}

fn api_err(e: ApiError) -> String {
    e.to_string()
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn emotes_of(v: &Value) -> Result<Vec<Emote>, String> {
    serde_json::from_value(v.clone()).map_err(|e| format!("bad emote list: {e}"))
}

/// `[[key, [emote, ...]], ...]` in priority order.
fn sources_of(v: &Value) -> Result<Vec<(String, Vec<Emote>)>, String> {
    let mut out = Vec::new();
    for pair in v.as_array().ok_or("sources must be a list")? {
        let key = pair
            .get(0)
            .and_then(Value::as_str)
            .ok_or("source without a key")?;
        out.push((
            key.to_string(),
            emotes_of(pair.get(1).unwrap_or(&json!([])))?,
        ));
    }
    Ok(out)
}

fn labels_of(v: &Value) -> IndexMap<String, String> {
    v.as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn u32_of(v: &Value, k: &str) -> u32 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0) as u32
}

fn run(op: u32, v: &Value) -> Result<Value, String> {
    Ok(match op {
        // version
        0 => json!(env!("CARGO_PKG_VERSION")),
        // url {kind, arg}
        1 => {
            let arg = str_of(v, "arg");
            json!(match str_of(v, "kind") {
                "global" => seventv::global_set_url(),
                "set" => seventv::set_url(arg),
                "emote" => seventv::emote_url(arg),
                "twitch_login" => seventv::twitch_login_url(arg),
                "twitch_user" => seventv::twitch_user_url(arg),
                "gql" => seventv::GQL.to_string(),
                other => return Err(format!("unknown url kind {other}")),
            })
        }
        // set_emotes {json} -> [emote]   (zero-width emotes skipped)
        2 => json!(seventv::emotes_from_set(&v["json"], false)),
        // set_name {json, id}
        3 => json!(seventv::set_name(&v["json"], str_of(v, "id"))),
        // emote {json}
        4 => json!(seventv::emote_from_json(&v["json"]).map_err(api_err)?),
        // twitch_id {json, login}
        5 => json!(seventv::twitch_id_from_json(&v["json"], str_of(v, "login")).map_err(api_err)?),
        // channel_set {json} -> {set, fetch}
        6 => {
            let (set, fetch) = seventv::channel_set_from_json(&v["json"]);
            json!({"set": set, "fetch": fetch})
        }
        // parse_ref {ref} -> {kind, value}
        7 => match seventv::parse_ref(str_of(v, "ref")).map_err(api_err)? {
            Ref::Id(x) => json!({"kind": "id", "value": x}),
            Ref::Set(x) => json!({"kind": "set", "value": x}),
            Ref::Name(x) => json!({"kind": "name", "value": x}),
        },
        // search_body {name}
        8 => seventv::search_body(str_of(v, "name")),
        // search_pick {json, name} -> emote | null
        9 => json!(seventv::search_pick(&v["json"], str_of(v, "name"))),
        // resolve {sources} -> [{name, id, key, file, url}]  (url null when 7TV lists no usable file)
        10 => {
            let chosen = pack::resolve(&sources_of(&v["sources"])?, &[]);
            let list: Vec<Value> = chosen
                .iter()
                .map(|(name, (em, key))| {
                    let (file, url) = match images::file_url(&em.host, 64) {
                        Some((f, u)) => (json!(f), json!(u)),
                        None => (Value::Null, Value::Null),
                    };
                    json!({"name": name, "id": em.id, "key": key, "file": file, "url": url})
                })
                .collect();
            json!(list)
        }
        // lua {info, sources, built: {id: layout}, labels} -> Local.lua text
        11 => {
            let i = &v["info"];
            let strings = |k: &str| -> Vec<String> {
                i.get(k)
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let info = PackInfo {
                version: env!("CARGO_PKG_VERSION").to_string(),
                built: i.get("built").and_then(Value::as_i64).unwrap_or(0),
                url: str_of(i, "url").to_string(),
                blp: i.get("blp").and_then(Value::as_bool).unwrap_or(true),
                global: i.get("global").and_then(Value::as_bool).unwrap_or(false),
                channels: strings("channels"),
                sets: strings("sets"),
            };
            let chosen = pack::resolve(&sources_of(&v["sources"])?, &[]);
            let mut built = IndexMap::new();
            if let Some(m) = v["built"].as_object() {
                for (id, l) in m {
                    let layout = Layout {
                        w: u32_of(l, "w"),
                        h: u32_of(l, "h"),
                        tw: u32_of(l, "tw"),
                        th: u32_of(l, "th"),
                        frames: u32_of(l, "frames").max(1),
                        cols: u32_of(l, "cols").max(1),
                        ms: u32_of(l, "ms"),
                    };
                    built.insert(
                        id.clone(),
                        PackEmote {
                            id: id.clone(),
                            layout,
                        },
                    );
                }
            }
            json!(pack::local_lua(
                &info,
                &chosen,
                &built,
                &labels_of(&v["labels"])
            ))
        }
        // toc
        12 => json!(String::from_utf8_lossy(pack::toc())),
        // texture_path {id, blp} -> "pack/<id>.blp" (zip path, forward slashes)
        13 => {
            let blp = v.get("blp").and_then(Value::as_bool).unwrap_or(true);
            json!(pack::texture_path(str_of(v, "id"), blp).replace('\\', "/"))
        }
        // source_label {key, set_label}
        14 => json!(pack::source_label(
            str_of(v, "key"),
            v.get("set_label").and_then(Value::as_str)
        )),
        // valid_name {name}
        15 => json!(pack::valid_name(str_of(v, "name"))),
        _ => return Err(format!("unknown operation {op}")),
    })
}
