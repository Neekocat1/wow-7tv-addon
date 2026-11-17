# Emote pack builder

A static web page that builds a `SevenTVEmotes_Local` emote pack in the player's browser, for players
who don't want to install the helper app. Published at https://neekocat1.github.io/wow-7tv-addon/.

The player enters Twitch channels and/or individual emotes. The page fetches the emote lists and
images straight from 7TV (7TV and ivr.fi allow requests from other sites), converts them to WoW
textures and offers a zip. Nothing is uploaded, and there's no server to run.

```
site/            the page (static files; publish this folder)
  index.html, app.js, style.css
  worker.js      runs the build in a Web Worker so the page stays responsive
  pack.js        the build: fetch from 7TV, convert, write Local.lua, zip (also runs in Node)
  zip.js         small zip writer (deflate via the browser's CompressionStream)
  pack.wasm      built from wasm/ (not in git)
wasm/            Rust -> WebAssembly; uses helper-rs without its "native" feature, so emote selection,
                 image conversion (TGA/BLP DXT5) and Local.lua are the helper's own code
test/            pack.test.mjs (Node, fake 7TV) and page_test.py (headless Chrome, fake 7TV)
build.sh/.bat    builds pack.wasm into site/
```

## Build and test

```sh
rustup target add wasm32-unknown-unknown
web/build.sh                                  # or web\build.bat
node web/test/pack.test.mjs web/site/pack.wasm
python web/test/page_test.py                  # needs: pip install playwright; playwright install chromium
```

To try the page locally, serve `web/site` (for example `python -m http.server -d web/site`) and open
it in a browser; it doesn't work from a `file://` path.

`node web/test/pack.test.mjs web/site/pack.wasm pack.zip` also writes the test pack; its `Local.lua`
can be checked with the addon test: unzip it and run
`lua tests/test_addon.lua <unzipped>/SevenTVEmotes_Local/Local.lua`.

## Publishing

`.github/workflows/pages.yml` builds, tests and publishes `web/site` on every push to `main` that
touches it. One-time setup in the repo: Settings > Pages > Build and deployment > Source:
"GitHub Actions".

## The pack

```
SevenTVEmotes_Local/
  SevenTVEmotes_Local.toc
  Local.lua              SevenTVEmotes_Pack = { version, built, url, format, channels, sets, origin, ... }
                         followed by the same emote table the helper writes
  pack/<7tv id>.blp      one texture per emote (.tga when "Uncompressed" is chosen)
```

The addon recognises `SevenTVEmotes_Pack` (no helper running) and points players to this page to
change their emotes. If the helper is installed later, it takes the folder over and removes `pack/`.
