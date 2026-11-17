//! Build script:
//! 1. Embeds the addon folders (../addon/SevenTVEmotes, ../addon/SevenTVEmotes_Local) in the exe,
//!    like the PyInstaller build bundled them.
//! 2. On Windows targets, adds the icon, version information and an application manifest to the exe.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else {
            let rel = p
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, p));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let addon_root = manifest.join("..").join("addon");
    println!("cargo:rerun-if-changed={}", addon_root.display());
    println!("cargo:rerun-if-changed=assets");

    let mut code = String::from(
        "/// (addon folder, path inside it, contents)\npub static FILES: &[(&str, &str, &[u8])] = &[\n",
    );
    let mut count = 0;
    for addon in ["SevenTVEmotes", "SevenTVEmotes_Local"] {
        let dir = addon_root.join(addon);
        let mut files = Vec::new();
        walk(&dir, &dir, &mut files);
        for (rel, path) in files {
            println!("cargo:rerun-if-changed={}", path.display());
            let abs = std::fs::canonicalize(&path).unwrap();
            writeln!(
                code,
                "    ({addon:?}, {rel:?}, include_bytes!({:?})),",
                abs.to_string_lossy()
            )
            .unwrap();
            count += 1;
        }
    }
    code.push_str("];\n");
    if count == 0 {
        println!(
            "cargo:warning=no addon files found in {}; the exe won't be able to install the addon",
            addon_root.display()
        );
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bundled_addons.rs");
    std::fs::write(out, code).unwrap();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        // Cross-compiling from Linux with mingw: point at its windres/ar.
        if std::env::var("HOST")
            .map(|h| !h.contains("windows"))
            .unwrap_or(false)
        {
            res.set_toolkit_path("/usr/bin");
            res.set_windres_path("x86_64-w64-mingw32-windres");
            res.set_ar_path("x86_64-w64-mingw32-ar");
        }
        res.set_icon("assets/icon.ico")
            .set("ProductName", "7TV Emotes Helper")
            .set("FileDescription", "7TV Emotes Helper")
            .set("CompanyName", "Connor")
            .set("LegalCopyright", "Connor")
            .set("OriginalFilename", "SevenTVEmotesHelper.exe")
            .set("InternalName", "SevenTVEmotesHelper")
            .set_manifest(include_str!("assets/app.manifest"));
        if let Err(e) = res.compile() {
            println!("cargo:warning=couldn't add Windows resources (icon/version info): {e}");
        }
    }
}
