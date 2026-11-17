# Rust port of the helper: handoff notes

## Status (2026-11-17): port done, in `helper-rs/`

- All modules ported (see `helper-rs/README.md` for the module map and the differences from Python).
  Version 0.6.0, so it takes over from a running 0.5.x Python helper.
- `tests/test_helper.py` ported test for test to `helper-rs/tests/helper.rs`; all pass, plus extra
  tests. The Python suite still passes too.
- Compared with the Python helper on the same fake setup: Local.lua is byte-identical apart from
  `version`/`heartbeat`; texture layouts (w, h, texW, texH, frames, cols, ms) are identical; TGA
  pixels match (what you see differs by ~0.1/255 on downscaled animated emotes, from the resampler);
  `test_addon.lua` and `test_ui.lua` pass on the Rust output.
- Found and fixed along the way: the `image-webp` crate skips "dispose to background" in animated
  WebP unless a background colour is set, which left trails on some frames.
- Windows exe cross-compiled and smoke-tested under Wine: installs the addons, slots, status ring and
  test images; tray mode runs; hand-over between instances via quit.request works.
- Installer: `helper-rs/installer/SevenTVEmotesHelper.iss` (Inno Setup 6; build with
  `helper-rs/build.bat`). The exe no longer copies itself or writes the Run key on first run.
- Not testable from where this was built (do these on Windows):
  - Real 7TV / ivr.fi traffic (the build machine had no route to them).
  - The tray icon/menu/balloons on a real desktop, the folder picker and message boxes.
  - Compiling the .iss and running the installer; hand-over from the running Python helper.
  - The in-game checks in README.md.
- Still planned: code signing.

---

## Original notes

Goal: replace the Python helper (`helper/`) with a Rust program that does exactly the same thing,
mainly to stop antivirus false positives caused by PyInstaller's self-extracting bootloader. The WoW
addon (`addon/`) stays as it is; the Rust helper must produce the same files in the same formats.
The Python code is the reference implementation, and its tests describe the expected behaviour
(`tests/test_helper.py`). Port module by module and port the tests alongside.

Also planned with the port:
- Ship an installer (e.g. Inno Setup) instead of the exe copying itself to %LOCALAPPDATA% and adding a
  Run key (that self-copy pattern resembles malware to heuristics). Installer: put the exe in
  %LOCALAPPDATA%\Programs\SevenTVEmotesHelper, add a Startup entry, launch it.
- Code signing later (Azure Artifact Signing, ~$10/month, individuals US/Canada) for SmartScreen.

## What the helper does (current Python behaviour, v0.5.1)

### Finding WoW (wow.py)
- Roots: registry `HKLM\SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft\InstallPath`
  (and the non-WOW6432 key), parent of `_retail_` etc.; common paths `<drive>:\{Program Files (x86),
  Program Files, Games, Battle.net, }\World of Warcraft`; plus `wow_dirs` from config.
  (Connor's install: `E:\Games\World of Warcraft\_forever_`.)
- Game folders ("flavors"): child dirs named `_x_` with Interface/WTF/Wow*.exe. Manage those whose
  name contains "forever", any that already have `Interface\AddOns\SevenTVEmotes`, and config `flavors`.
- SavedVariables: `<flavor>\WTF\Account\*\SavedVariables\SevenTVEmotes.lua` (one per account).
- Game running: any process whose exe lives under a WoW root, or whose name matches
  `^(wow|world ?of ?warcraft|forever)` (excluding battle.net, agent, blizzard, wowup, warcraftlogs,
  raider, tsm, curseforge). "Game start" = earliest create time. Log changes ("WoW: WowForever.exe (pid)").
  If process info is unavailable, treat helper start time as game start (conservative).

### Files the helper owns: `Interface\AddOns\SevenTVEmotes_Local\`
- `SevenTVEmotes_Local.toc` (copied from `addon/SevenTVEmotes_Local/`).
- `Local.lua`: generated, see format below. Rewritten atomically (write .part, rename).
- `slots\s0001.tga ...` uncompressed emote textures; `cslots\s0001.blp ...` compressed ones.
  NEVER put a .tga and .blp with the same name in one folder: WoW matches textures by name and
  loads the wrong one (this was the "compressed emotes invisible" bug).
- `status\st0000.tga ... st4799.tga`: live status ring (see below).
- `test\t_tga.tga, t_dxt5m.blp, t_dxt5.blp, t_argb.blp, t_pal.blp`: for `/7tv texturetest`.
  Result on Connor's client: TGA and all three DXT5/raw BLP variants load; Pillow's palette BLP doesn't.
- `helper_state.json`: helper bookkeeping (keep the same schema so the Rust helper can take over an
  existing install; see library.py `_default_state`, `layout` = 2).

### The key WoW constraint
WoW only sees texture files that existed when the game started. Overwriting an existing file and
then displaying it (first load, or after /reload) shows the new content. Hence:
- ~1500 reserved slots created up front (blank 8x8 files), topped up by 500 when < 150 free.
- A slot/file created or freed after the running game started is not used until WoW restarts.
- When WoW (re)starts (game start time changes), immediately build anything that was waiting.
- If compression is on but compressed placeholders aren't visible yet, build as TGA now and convert later.
- Old-format files are blanked only after the addon has reloaded onto the new paths
  (a SavedVariables write newer than the Local.lua that switched) or WoW isn't running.

### Requests from the addon (SavedVariables)
`SevenTVEmotesDB = { queue = { { id="1763...-0042", kind="add|remove|channel|unchannel", ref=..., alias=..., t=... }, ... }, compress = true|false, invisible = {...}, ... }`
- WoW writes this file on /reload and logout. Poll mtimes every 2 s; parse with a small Lua-literal
  parser (luasv.py; read bytes as latin-1, decode strings as UTF-8; handle `-- [1]` comments, `\ddd`).
- Process each request id once (remember in state `processed`). `add`: 7tv.app link / id / set link /
  exact name (search cached sources first, then 7TV GQL search). `remove`: drop manual emote or hide a
  source emote. `channel`/`unchannel`: Twitch login -> id via `https://api.ivr.fi/v2/twitch/user?login=`,
  then `https://7tv.io/v3/users/twitch/{id}`.
- `compress` from the most recently saved account selects "tga" or "blp".
- Network errors: retry up to 3 times, 30 s apart; report "retrying" in Local.lua meanwhile.

### Emote sources and reconcile (library.py)
- Sources: `global` (7TV global set, fetched on first run; retried every minute until it works),
  `channel:<login>`, `set:<id>`, plus `manual`. Priority: global < channels (by add time) < manual.
  Skip zero-width emotes (active flag 1<<0, emote flag 1<<8). Names: no whitespace or `|`, <= 64 chars.
- Refresh sources every 6 h. Reconcile = compute desired name -> emote, build missing textures
  (8 parallel downloads), free unused slots (blank their file), convert formats.
- Progress: report done/total (tray tooltip, log every 10 %), and rewrite Local.lua every 5 s during
  long builds so a reload mid-way shows what's done.

### Images (images.py)
- Pick the 7TV file: prefer WEBP, then GIF, PNG, AVIF; smallest with height >= 64.
  URL = "https:" + host.url + "/" + file.name. Cache downloads in %LOCALAPPDATA%\SevenTVEmotesHelper\cache.
- Static: scale to height 64 (animated: 48), cap width at 4x height, paste top-left on a power-of-two
  canvas. Animated: decode all frames (composited), cap at 48 frames (merge durations), grid layout
  minimising power-of-two sheet area (<= 1024), ms per frame = average (min 10; frame delays < 20 ms
  count as 100).
- TGA: 32-bit uncompressed, written atomically.
- BLP2 DXT5: header "BLP2", u32 1, u8 encoding 2, alpha depth 8, alpha type 7, has_mips 1, u32 w, h,
  16 mip offsets, 16 mip sizes, 1024-byte zero palette (header 1172 bytes), then full mip chain down to
  1x1 (box filter; levels < 4 px padded to one 4x4 block).

### Local.lua format (read by the addon; keep exactly)
```
SevenTVEmotes_Helper = { version=, heartbeat=<unix>, slotsFree=, channels={...}, results={ [reqId]={ok=,msg=,name=} },
  retrying={ [reqId]=msg }, waiting={names}, storage={bytes=,tga=,blp=,format=,converting=,emotes=,compressAvailable=,cache=},
  board={dir="status",count=4800,period=3}, origin={ [name]="7TV global emotes"|"x's channel"|"Added by you"|"Set: ..." },
  notice=?, progress={done=,total=,t=}? }
local emotes = { ["name"] = { "slots\\s0001.tga"|"cslots\\s0001.blp", w, h, texW, texH, "7tvId" [, frames, cols, msPerFrame] }, ... }
if SevenTVEmotes and SevenTVEmotes.RegisterPack then SevenTVEmotes.RegisterPack("My emotes", emotes, "SevenTVEmotes_Local") end
```
Heartbeat rewrite every 5 min. Results kept 14 days.

### Live status board (status_board.py)
- The addon shows `status\st%04d.tga` with index = floor(unix_time / 3) % 4800 (double-buffered).
- Helper renders 384x48 images: rounded dark box, coloured left bar, bold status line (shrink to fit),
  small "7TV Emotes Helper - live HH:MM:SS" line. Write the current and next 6 indices whenever the
  text changes or time advances; blank indices the clock has passed.
- Texts: "Fetching N requests from 7TV...", "Downloading emotes x/y", "Can't reach 7TV - retrying",
  "<N new emotes ready> - click Reload" (until a SavedVariables write newer than the ready time),
  "N emotes will show after a WoW restart", "Up to date - N emotes".
- Needs a font: bundle one (e.g. an open-licence TTF) and rasterise with ab_glyph/fontdue.

### App behaviour (__main__.py, instance.py, tray.py, installer.py)
- Single instance via lock file; a newer version asks an older one to quit (`quit.request` in app dir,
  `running.json` with version/pid), never the reverse.
- Tray icon (blue square with "7"): status line, "Check 7TV for new emotes now", "Open emote folder",
  "Open log", "Start with Windows" toggle, "Quit". Notifications when requests finish.
- First run: find WoW (ask with a folder picker if not found), install `SevenTVEmotes` core addon if
  missing/older (by TOC version) and create `SevenTVEmotes_Local`, enable autostart.
- `--once`, `--no-tray`, `--wow-dir`, `--flavor`, `--autostart on|off` flags.
- Log: %LOCALAPPDATA%\SevenTVEmotesHelper\helper.log (rotating).

## Suggested crates
ureq (+rustls) or reqwest blocking; serde/serde_json; image (webp, gif, png; animated frames);
texpresso or a port of images.encode_dxt5 (numpy version is in git history / images.py fallback);
ab_glyph or fontdue for status text; tray-icon (+ a minimal event loop) or native Win32 via windows-rs;
sysinfo for processes; winreg; rfd for the folder picker; directories for app paths.
Release profile: opt-level="z", lto=true, codegen-units=1, panic="abort", strip=true.

## First steps in the new chat
1. Check `cargo fetch` works (crates.io reachable).
2. Scaffold `helper-rs/` next to `helper/`; port luasv + images first (pure logic, easy to test
   against the Python output), then library/service, then tray/installer.
3. Run both test suites; compare generated Local.lua and textures with the Python helper's output.
