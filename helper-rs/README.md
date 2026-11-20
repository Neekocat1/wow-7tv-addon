# 7TV Emotes Helper (Rust)

Rust port of the original Python helper (v0.5.1, removed from the repo after the port; it's in the
git history). It writes the same files in the same
formats (`Local.lua`, `slots/`, `cslots/`, `status/`, `test/`, `helper_state.json`), so it can
take over an existing install, and it hands over from a running Python helper the same way a newer
Python helper would (lock file + `quit.request`). The main reason for the port: no PyInstaller
self-extracting bootloader, so no antivirus false positives from it.

## Build

```bat
build.bat
```

Needs Rust (https://rustup.rs, default MSVC toolchain plus Visual Studio Build Tools with the
"Desktop development with C++" workload). Produces `target\release\SevenTVEmotesHelper.exe`
(about 4 MB, no runtime needed) and, if Inno Setup 6 is installed, the installer
`dist\SevenTVEmotesHelper-Setup-<version>.exe`.

The addon folders (`../addon/SevenTVEmotes`, `../addon/SevenTVEmotes_Local`) are embedded in the
exe at build time (`build.rs`), like the PyInstaller build bundled them. Rebuild after changing
the addon.

## Run

Same flags as before:

```bat
rem one pass in the console against your install, then exit
target\release\SevenTVEmotesHelper.exe --once --wow-dir "E:\Games\World of Warcraft"

rem normal (tray icon); --no-tray runs in the console
target\release\SevenTVEmotesHelper.exe
```

`--background`, `--flavor _retail_`, `--autostart on|off` and `--version` work as in the Python
helper. The exe is a windowed program; `--once`/`--no-tray` print to the terminal they were started
from. Log, config and download cache are still in `%LOCALAPPDATA%\SevenTVEmotesHelper`.

## Test

```bat
cargo test
```

`tests/helper.rs` is a test-for-test port of the Python helper's test suite (fake WoW install, fake
7TV), plus a few extra checks (the WebP disposal bug below, an animated GIF, reading a Python-era
`helper_state.json`). Unit tests live next to the code.

To run the addon's Lua tests on the Rust helper's output:

```bat
set DUMP_LOCAL=%TEMP%\Local.lua
cargo test --test helper requests
cd .. && lua tests/test_addon.lua %TEMP%\Local.lua && lua tests/test_ui.lua %TEMP%\Local.lua
```

(`test_ui.lua` needs Lua 5.2+.)

## Module map

| Rust | Ported from (Python) | |
|---|---|---|
| `pack.rs` | | emote selection rules, Local.lua emote lines, emote packs (shared with `web/`) |
| `bundle.rs` | | the addon files embedded in the exe |
| `luasv.rs` | `luasv.py` | SavedVariables parser, Lua writer |
| `seventv.rs` | `seventv.py` | 7TV API (ureq + rustls), link parsing |
| `images.rs` | `images.py` | decode WEBP/GIF/PNG/APNG, sprite sheets, TGA, BLP2 DXT5 + mips |
| `draw.rs`, `status_board.rs` | `status_board.py` | live status images (ab_glyph text) |
| `library.rs` | `library.py` | slots, reconcile, requests, Local.lua |
| `service.rs` | `service.py` | main loop |
| `wow.rs` | `wow.py` | finding WoW, game process (sysinfo) |
| `instance.rs` | `instance.py` | single instance, hand-over |
| `installer.rs` | `installer.py` | addon install, autostart |
| `updater.rs` | (new) | updating itself from GitHub releases |
| `tray.rs`, `win.rs` | `tray.py`, dialogs | native Win32 tray, message boxes, folder picker, registry |
| `main.rs` | `__main__.py` | flags, first run |

## Shared with the pack builder

The `native` feature (on by default) is the Windows app. Without it, the crate is just the emote code
(`seventv`, `images`, `luasv`, `pack`), which `web/wasm` compiles to WebAssembly so the pack builder
picks and converts emotes exactly like the helper. `cargo build --no-default-features` checks that
this part still builds on its own.

When the helper finds a `SevenTVEmotes_Local` folder without its `helper_state.json` (an emote pack),
it sets the folder up as its own and removes the pack's `pack/` textures.

## Differences from the Python helper

- **No self-install.** The exe doesn't copy itself to `%LOCALAPPDATA%` or add itself to the Run
  key on first run any more; `installer/SevenTVEmotesHelper.iss` does that (into
  `%LOCALAPPDATA%\Programs\SevenTVEmotesHelper`), asks a running helper to quit, and removes the old
  Python copy. The tray's "Start with Windows" toggle and `--autostart` still work and use the same
  Run-key value. Run the bare exe and it simply runs from where it is.
- **Compression always available.** The DXT5 encoder is built in (a port of the numpy fallback);
  measured against the uncompressed texture it is as accurate as Pillow's encoder or better.
- **Animated WebP:** frames are composited like libwebp/Pillow. (The `image-webp` crate skips
  "dispose to background" unless told the background colour, which left trails on some frames;
  `images::load_frames` sets it to transparent.)
- **AVIF** isn't decoded (it's last in the format preference and 7TV always offers WEBP/GIF/PNG).
- **Fonts:** Segoe UI / Arial from `C:\Windows\Fonts` as before; DejaVu Sans is embedded as a
  fallback (`assets/DejaVu-LICENSE.txt`).
- **Game start time** comes from sysinfo in whole seconds (psutil had sub-second precision). The
  rounding is downward, which can only make the slot check more careful, never less.
- A panic in one pass is logged and the loop keeps going (like the Python try/except), so the
  release profile uses `panic = "unwind"` rather than `"abort"`.

Version is 0.6.0 so it takes over from a running 0.5.x Python helper.

## Still to do

Not yet checked on a real Windows PC (the exe was built and smoke-tested under Wine):

- [ ] Tray icon on a real desktop: the menu, the "Start with Windows" tick, balloon notifications.
- [ ] First-run dialogs: the "WoW not found" folder picker and the message boxes.
- [ ] Compiling `installer/SevenTVEmotesHelper.iss` and running the installer: install, start with
      Windows, upgrade over a running helper, uninstall.
- [ ] The installer taking over from a running Python helper (0.5.x) and removing its old copy.

Planned: code signing, so SmartScreen stops warning about the exe and installer (Azure Artifact
Signing, about $10/month, available to individuals in the US and Canada).

## Cross-compiling from Linux

What was used to produce the first build (no Windows machine needed):
mingw-w64, the Rust standard library source for the toolchain's exact commit in
`$(rustc --print sysroot)/lib/rustlib/src/rust/library`, then

```sh
export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
RUSTC_BOOTSTRAP=1 cargo build --release --target x86_64-pc-windows-gnu -Zbuild-std=std,panic_unwind
```

(Normally just `rustup target add x86_64-pc-windows-gnu` and `cargo build --release --target
x86_64-pc-windows-gnu`.) `build.rs` uses mingw's `windres` for the icon and version info.
