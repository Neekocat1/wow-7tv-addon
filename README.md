# 7TV Emotes for WoW: Forever

7TV emotes in World of Warcraft chat. Players get emotes either from the **7TV Emotes Helper** app
(adds emotes from inside the game) or as a downloaded **emote pack** from the
[pack builder](https://neekocat1.github.io/wow-7tv-addon/) web page (nothing to install).

```
addon/
  SevenTVEmotes/          the addon players install (chat rendering, animation, autocomplete, /7tv)
  SevenTVEmotes_Local/    template for the player's emote folder (helper-managed, or an emote pack)
  package-curseforge.bat  zip SevenTVEmotes for CurseForge into dist/
helper-rs/                the background app (Rust), and the emote code the web page shares: see helper-rs/README.md
  build.bat               build SevenTVEmotesHelper.exe and the installer
  installer/              Inno Setup script (per-user install + start with Windows)
web/                      the emote pack builder web page (WebAssembly + JS): see web/README.md
docs/                     CurseForge description
tests/                    addon tests (Lua); the helper's and the web page's tests are in helper-rs/tests and web/test
.github/workflows/        publishes web/ on GitHub Pages; uploads the addon to CurseForge on each release
```

## How it fits together

WoW addons can't download anything, so a small app on the player's PC does it. The two
sides only talk through files:

1. In game, adding an emote (window or `/7tv add <link>`) just queues it; no popup. A small
   **7TV bar** appears above the chat box with a **Send** button. Sending reloads the UI, because WoW
   only writes the addon's saved data to disk on `/reload` or logout. Queue as many as you like
   first; anything still queued is also sent automatically when you log out.
2. The helper notices the saved-settings file changed, downloads the emotes, converts them and
   rewrites `SevenTVEmotes_Local/Local.lua`.
3. Meanwhile the 7TV bar shows the helper's **live status**, without reloading: "Fetching 3
   requests...", "Downloading emotes 37/412", then "12 new emotes ready - click Reload".
   Reload then, and the emotes work and the addon reports what happened.

**Live status, how:** WoW reads a texture file's contents the first time it's shown. The helper keeps
a ring of 4,800 tiny placeholder images (`SevenTVEmotes_Local/status/`) and draws its current status
into the next few; the addon shows a different one every 3 seconds, so the picture is never more than
a few seconds old. The addon can't read the picture, it's just for you. `/7tv status` shows the bar on
demand: the clock on it should tick without a reload.

**Reserved slots.** WoW only sees texture files that existed when the game started; overwriting
an existing file only needs a `/reload`. So the helper keeps ~1,500 blank placeholder textures in
`SevenTVEmotes_Local/slots/` and writes new emotes into them. Slots created or freed while WoW is
running are only reused after a restart; if they ever run out mid-session the player is told the
emote will appear after restarting WoW.

**Why a separate `SevenTVEmotes_Local` addon:** addon managers replace the main addon's folder on
update. Keeping helper-written files in their own folder means updates never wipe a player's emotes.

The helper also adds 7TV's global emotes, keeps channel sets up to date (every 6 hours), and
writes a heartbeat so the addon can say "the helper isn't running" instead of failing silently.

## The /7tv window

Type `/7tv` to open it (`/7tv help` lists the chat commands).
There's also a 7TV button on the minimap: left-click opens the window, right-click opens Settings,
drag it to move it around the minimap. Hide it in Settings or with `/7tv minimap`; the window is also
in the minimap's addon menu (the addon compartment) on clients that have one.

- **Emotes**: search and filter (All / Mine / Animated / Hidden), animated previews. Click an emote to
  see where it came from, copy its name, hide it from chat, or remove it.
- **Add**: paste a 7tv.app link (or type a name), optionally rename it; add or remove Twitch channels,
  and put them in priority order (if two channels have an emote with the same name, the higher one wins).
- **Downloads**: helper status (running / not seen recently / not installed) and every request:
  *Not sent yet* → *In progress* → *Done* / *Failed*, plus *Retrying* when 7TV is unreachable and
  *Needs restart* when reserved slots ran out. Cancel anything still queued.
- **Settings**: chat options, emote size, disk usage, and **compressed emotes**.

The footer always says what's pending and offers the right Reload button. While the helper is
downloading, its tray icon shows "Downloading emotes 37/412", and Local.lua is rewritten every few
seconds, so a reload in the middle shows what's done so far and the same count in the Downloads tab. Progress can only update
on reload: WoW gives addons no way to read new files while the UI is running.

### Compressed emotes

Off by default. When turned on, the helper re-encodes every emote as a BLP texture with DXT5
compression (WoW's own format): about 4x smaller on disk and in video memory than uncompressed TGA,
with slightly softer colors. Each reserved slot has a `slots/sNNNN.tga` and a `cslots/sNNNN.blp`
file, so switching only needs reloads, not a restart. (They used to sit side by side with the same
name, and WoW appears to match texture files by name regardless of extension, so it loaded the wrong
one; helper 0.5 moves the .blp files and rebuilds affected emotes.) Until WoW has been restarted
after the compressed placeholders were created, new emotes are built as TGA and converted later.
The old files are blanked once the addon has reloaded onto the new ones.

## For players

Install the **7TV Emotes** addon (CurseForge, Forever version; or extract `SevenTVEmotes` from a
release zip into `World of Warcraft\_forever_\Interface\AddOns`). Then get emotes one of two ways.

**Emote pack (no install):** open the [pack builder](https://neekocat1.github.io/wow-7tv-addon/), enter
Twitch channels and/or individual emotes, click Build, and extract the zip into the same AddOns folder
(delete an old `SevenTVEmotes_Local` folder first). Restart WoW. Build a new pack to change your emotes.

**The helper app (adds emotes from inside the game):** download `SevenTVEmotesHelper.exe` from the [latest release](../../releases/latest), put it somewhere
permanent (e.g. a folder in Documents) and run it. On its first run it finds WoW: Forever, installs
both addons and sits in the system tray. To have it start with Windows, right-click the tray icon
and tick "Start with Windows". Restart WoW once after the first run. The exe isn't code-signed yet, so
Windows SmartScreen may warn the first time: choose "More info", then "Run anyway".
(A setup program that does all of this is in `helper-rs/installer`; it isn't released yet.)

After that everything happens in game:

| Command | |
|---|---|
| `/7tv add <7tv.app link>` | add one emote (`/7tv add <link> myName` to rename it) |
| `/7tv add <name>` | add by name (most popular 7TV emote with that exact name) |
| `/7tv channel <twitch name>` | add a streamer's 7TV emotes; `/7tv unchannel` removes them |
| `/7tv channels` | your channels by priority (reorder them on the Add tab) |
| `/7tv remove <name>` | remove an emote |
| `/7tv queue`, `/7tv helper` | pending requests, helper status |
| `/7tv search`, `/7tv config`, `/7tv size`, `/7tv disable` | as before |

Autocomplete: type `:` plus two characters; Tab cycles, another `:` accepts.

## Development (Windows)

**CurseForge:** publishing a GitHub release uploads the addon to CurseForge automatically
(`.github/workflows/curseforge.yml`; setup steps at the top of that file). The file is named after the
release's tag and gets the release notes as its changelog. Running that workflow by hand builds the zip
without uploading it. To upload by hand instead, run `addon\package-curseforge.bat` and upload
`dist\SevenTVEmotes-<version>.zip` as a Forever file. Either way the zip contains only `SevenTVEmotes`;
`SevenTVEmotes_Local` must never be in it, or CurseForge updates would wipe players' emotes. The
description is in `docs/curseforge-description.md`.

**Pack builder:** `web/build.bat` (or `web/build.sh`) builds its WebAssembly module from the helper's
Rust code. Pushing to `main` publishes it on GitHub Pages through `.github/workflows/pages.yml`
(one-time setup: Settings > Pages > Source: GitHub Actions; Pages on a free account needs a public repo).
Details in `web/README.md`.

The helper is the Rust program in `helper-rs/` (details in `helper-rs/README.md`):

```bat
cd helper-rs
build.bat
target\release\SevenTVEmotesHelper.exe --once --wow-dir "C:\Program Files (x86)\World of Warcraft"
cargo test
```

The helper only starts with Windows when the installer (or the tray toggle) sets that up;
use `--autostart on|off` to test it.

The original Python helper (0.5.1) was removed after the Rust port; it's in the git history
(first commit) if you ever need to compare against it.

**Updating:** run the new installer. It asks the running helper to quit (via `quit.request` in the
app folder), replaces the exe and starts the new one. Starting a newer exe by hand does the same
hand-over (the old one quits, the new one takes over); an older helper never replaces a newer one.
The Rust helper (0.6) takes over from the Python helper (0.5.x) this way, and the installer removes the
Python helper's old copy. Helpers before 0.5 don't understand the request: quit those from the tray
icon first.
The helper's own files (config, log, download cache) are in `%LOCALAPPDATA%\SevenTVEmotesHelper`.

Tests: `cargo test` in `helper-rs` (fake WoW install + fake 7TV). The addon and window tests need a
Lua interpreter (5.2+ for the window test): generate a sample Local.lua with
`set DUMP_LOCAL=%TEMP%\Local.lua` and `cargo test --test helper requests` in `helper-rs`, then run
`lua tests/test_addon.lua %TEMP%\Local.lua` and `lua tests/test_ui.lua %TEMP%\Local.lua` from here.
`test_addon.lua` also accepts an emote pack's `Local.lua` (see `web/README.md`).

## To check in game (not testable outside WoW)

- [ ] Overwriting a reserved slot and `/reload`-ing shows the new emote (the whole design relies on this).
- [ ] `/7tv status`: the time on the bar ticks every few seconds without a reload. (Restart WoW once
      after installing helper 0.4, so the game sees the new status files.)
- [ ] Chat filter and edit box hooks work on the Forever client (the addon looks up both old and new API names).
- [ ] The helper detects the running game: the tray status says "(WoW open)" while playing. If not,
      the Forever executable has a name the helper doesn't expect (see `wow.rs`).
- [ ] `/7tv add <name>` uses 7TV's search API, which I couldn't test; links always work.
- [ ] `/7tv texturetest` (after a WoW restart): which of the 5 test images show? Row 2 is the
      format compression uses. If it's blank but another BLP row shows, switch the encoder to that one.
- [ ] The /7tv window: built from plain frames, so it should be stable across client versions,
      but layout has only been checked with mock frames.

## Troubleshooting

- `/7tv debug` in game prints the addon and helper versions, whether `SevenTVEmotes_Local` loaded,
  the texture format, how many emote files WoW can't see yet, and a sample emote.
- The helper's log is `%LOCALAPPDATA%\SevenTVEmotesHelper\helper.log`. It records whether it sees
  WoW running ("WoW: WowForever.exe (pid ...)"), download progress, and any 7TV errors.
- Emotes invisible right after the helper first runs (or after turning on compression for the
  first time) usually means WoW was open when their files were created: restart WoW once.

## Known limits

- Emotes in dungeon/raid chat stay as text (the client hides that chat from addons).
- Only players with the addon see emotes, and only the ones they have installed.
- Zero-width (overlay) emotes are skipped.
