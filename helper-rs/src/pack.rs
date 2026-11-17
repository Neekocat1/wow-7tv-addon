//! Emote packs: a ready-made SevenTVEmotes_Local folder for players who don't run the helper,
//! built by the web pack builder (web/). Also the emote-selection rules and the Local.lua emote
//! lines, which the helper shares so both produce the same result.
//!
//! A pack looks like this (zipped, extracted into Interface\AddOns):
//!
//! ```text
//! SevenTVEmotes_Local/
//!   SevenTVEmotes_Local.toc
//!   Local.lua            SevenTVEmotes_Pack = { ... } + the emote table
//!   pack/<7tv id>.blp    one texture per emote (or .tga), never both formats
//! ```
//!
//! The addon recognises SevenTVEmotes_Pack and points the player at the web page to change their
//! emotes. If the helper is installed later, it replaces the folder (see Library::install).

use crate::images::Layout;
use crate::luasv::{TableBuilder, lua_str, lua_value};
use crate::seventv::{self, Emote};
use indexmap::IndexMap;
use std::collections::HashSet;

pub const LOCAL_ADDON: &str = "SevenTVEmotes_Local";
/// Folder inside SevenTVEmotes_Local that holds a pack's textures.
pub const PACK_DIR: &str = "pack";

/// `^[^\s|]{1,64}$`: names that work as a single word in WoW chat.
pub fn valid_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=64).contains(&n) && !name.chars().any(|c| c.is_whitespace() || c == '|')
}

/// name -> (emote, source key). `sources` come in priority order (lowest first: global, then
/// channels and sets in the order they were added, then "manual"); a later source wins a name.
/// `removed` hides names from every source except "manual".
pub fn resolve(
    sources: &[(String, Vec<Emote>)],
    removed: &[String],
) -> IndexMap<String, (Emote, String)> {
    let removed: HashSet<&String> = removed.iter().collect();
    let mut out: IndexMap<String, (Emote, String)> = IndexMap::new();
    for (key, ems) in sources {
        for em in ems {
            if key != "manual" && removed.contains(&em.name) {
                continue;
            }
            if valid_name(&em.name) && seventv::is_id(&em.id) {
                out.insert(em.name.clone(), (em.clone(), key.clone()));
            }
        }
    }
    out
}

/// How the /7tv window describes where an emote came from.
pub fn source_label(key: &str, set_label: Option<&str>) -> String {
    match key {
        "global" => "7TV global emotes".into(),
        "manual" => "Added by you".into(),
        k if k.starts_with("channel:") => format!("{}'s channel", &k[8..]),
        k if k.starts_with("set:") => format!("Set: {}", set_label.unwrap_or(&k[4..])),
        k => k.to_string(),
    }
}

/// One line of Local.lua's emote table:
/// `[name] = { file, width, height, texWidth, texHeight, 7tvId [, frames, columns, msPerFrame] },`
pub fn emote_entry(name: &str, file: &str, l: &Layout, id: &str) -> String {
    let mut vals = vec![
        lua_str(file),
        l.w.to_string(),
        l.h.to_string(),
        l.tw.to_string(),
        l.th.to_string(),
        lua_str(id),
    ];
    if l.frames > 1 {
        vals.extend([l.frames.to_string(), l.cols.to_string(), l.ms.to_string()]);
    }
    format!("  [{}] = {{ {} }},", lua_str(name), vals.join(", "))
}

pub const EMOTES_HEADER: &str = "-- [name] = { file, width, height, texWidth, texHeight, 7tvId [, frames, columns, msPerFrame] }";

/// The lines after the emote entries.
pub fn register_footer() -> Vec<String> {
    vec![
        "}".into(),
        "if SevenTVEmotes and SevenTVEmotes.RegisterPack then".into(),
        format!("  SevenTVEmotes.RegisterPack(\"My emotes\", emotes, \"{LOCAL_ADDON}\")"),
        "end".into(),
        String::new(),
    ]
}

/// Where a pack keeps an emote's texture, as the addon sees it (relative to SevenTVEmotes_Local).
pub fn texture_path(id: &str, blp: bool) -> String {
    format!("{PACK_DIR}\\{id}.{}", if blp { "blp" } else { "tga" })
}

/// A built texture in a pack.
#[derive(Debug, Clone)]
pub struct PackEmote {
    pub id: String,
    pub layout: Layout,
}

/// What a pack was built from (shown by the addon).
#[derive(Debug, Clone, Default)]
pub struct PackInfo {
    pub version: String,
    /// Unix time the pack was built.
    pub built: i64,
    /// The page to build a new one.
    pub url: String,
    pub blp: bool,
    pub global: bool,
    pub channels: Vec<String>,
    /// Labels of added emote sets.
    pub sets: Vec<String>,
}

/// Local.lua for a pack. `chosen` comes from [`resolve`]; `built` maps 7TV ids to textures (emotes
/// whose texture failed are left out); `labels` maps source keys to what [`source_label`] shows.
pub fn local_lua(
    info: &PackInfo,
    chosen: &IndexMap<String, (Emote, String)>,
    built: &IndexMap<String, PackEmote>,
    labels: &IndexMap<String, String>,
) -> String {
    let mut origin = TableBuilder::new();
    let mut count = 0usize;
    for (name, (em, key)) in chosen {
        if built.contains_key(&em.id) {
            count += 1;
            let label = labels
                .get(key)
                .cloned()
                .unwrap_or_else(|| source_label(key, None));
            origin.push(name.as_str(), label);
        }
    }
    let header = TableBuilder::new()
        .set("version", info.version.as_str())
        .set("built", info.built)
        .set("url", info.url.as_str())
        .set("format", if info.blp { "blp" } else { "tga" })
        .set("global", info.global)
        .set("channels", info.channels.clone())
        .set("sets", info.sets.clone())
        .set("emotes", count)
        .set("origin", origin.build())
        .build();

    let mut lines: Vec<String> = vec![
        format!(
            "-- Emote pack made with the 7TV Emotes pack builder: {}",
            info.url
        ),
        "-- To change your emotes, build a new pack there, or install the 7TV Emotes Helper app."
            .into(),
        format!("SevenTVEmotes_Pack = {}", lua_value(&header, "")),
        String::new(),
        EMOTES_HEADER.into(),
        "local emotes = {".into(),
    ];
    let mut names: Vec<(&String, &(Emote, String))> = chosen.iter().collect();
    names.sort_by_key(|(n, _)| n.to_lowercase()); // stable, like the helper
    for (name, (em, _)) in names {
        if let Some(pe) = built.get(&em.id) {
            lines.push(emote_entry(
                name,
                &texture_path(&em.id, info.blp),
                &pe.layout,
                &em.id,
            ));
        }
    }
    lines.extend(register_footer());
    lines.join("\n")
}

/// The pack's .toc (the same file the helper installs).
pub fn toc() -> &'static [u8] {
    include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../addon/SevenTVEmotes_Local/SevenTVEmotes_Local.toc"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::luasv::Value as Lua;
    use serde_json::json;

    fn em(name: &str, id: &str) -> Emote {
        Emote {
            name: name.into(),
            id: id.into(),
            animated: false,
            host: json!({}),
        }
    }

    #[test]
    fn later_sources_win_and_removed_hides() {
        let sources = vec![
            (
                "global".to_string(),
                vec![em("Clap", "01GLOBALCLAP"), em("EZ", "01GLOBALEZ")],
            ),
            (
                "channel:tester".to_string(),
                vec![em("Clap", "01CHANCLAP"), em("bad name", "01X")],
            ),
            ("manual".to_string(), vec![em("EZ", "01MANUALEZ")]),
        ];
        let r = resolve(&sources, &["EZ".to_string(), "Clap".to_string()]);
        assert!(!r.contains_key("Clap"), "removed from non-manual sources");
        assert_eq!(r["EZ"].0.id, "01MANUALEZ");
        let r = resolve(&sources, &[]);
        assert_eq!(
            r["Clap"],
            (em("Clap", "01CHANCLAP"), "channel:tester".to_string())
        );
        assert!(!r.contains_key("bad name"));
        assert_eq!(
            r.keys().collect::<Vec<_>>(),
            ["Clap", "EZ"],
            "first-seen order kept"
        );
    }

    #[test]
    fn pack_lua_parses_and_lists_built_emotes() {
        let sources = vec![
            (
                "global".to_string(),
                vec![em("Clap", "01CLAP"), em("broken", "01BROKEN")],
            ),
            ("channel:tester".to_string(), vec![em("catJAM", "01CATJAM")]),
        ];
        let chosen = resolve(&sources, &[]);
        let mut built = IndexMap::new();
        built.insert(
            "01CLAP".to_string(),
            PackEmote {
                id: "01CLAP".into(),
                layout: Layout {
                    w: 64,
                    h: 64,
                    tw: 64,
                    th: 64,
                    frames: 1,
                    cols: 1,
                    ms: 0,
                },
            },
        );
        built.insert(
            "01CATJAM".to_string(),
            PackEmote {
                id: "01CATJAM".into(),
                layout: Layout {
                    w: 48,
                    h: 48,
                    tw: 256,
                    th: 512,
                    frames: 30,
                    cols: 3,
                    ms: 50,
                },
            },
        );
        let mut labels = IndexMap::new();
        labels.insert("channel:tester".to_string(), "tester's channel".to_string());
        let info = PackInfo {
            version: "0.6.1".into(),
            built: 1_800_000_000,
            url: "https://example.test/".into(),
            blp: true,
            global: true,
            channels: vec!["tester".into()],
            sets: vec![],
        };
        let lua = local_lua(&info, &chosen, &built, &labels);
        assert!(lua.contains("[\"catJAM\"] = { \"pack\\\\01CATJAM.blp\", 48, 48, 256, 512, \"01CATJAM\", 30, 3, 50 },"), "{lua}");
        assert!(
            lua.contains("[\"Clap\"] = { \"pack\\\\01CLAP.blp\", 64, 64, 64, 64, \"01CLAP\" },")
        );
        assert!(!lua.contains("broken"), "failed textures left out");
        let header = lua.split(EMOTES_HEADER).next().unwrap();
        let parsed = crate::luasv::parse(header.as_bytes()).unwrap();
        let p = crate::luasv::global(&parsed, "SevenTVEmotes_Pack").unwrap();
        assert_eq!(p.get("emotes"), Some(&Lua::Int(2)));
        assert_eq!(
            p.get("origin").unwrap().get("catJAM").unwrap().as_str(),
            Some("tester's channel")
        );
        assert_eq!(
            p.get("origin").unwrap().get("Clap").unwrap().as_str(),
            Some("7TV global emotes")
        );
        assert!(String::from_utf8_lossy(toc()).contains("Local.lua"));
    }
}
