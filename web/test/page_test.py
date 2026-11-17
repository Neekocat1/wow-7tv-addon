"""Browser test of the pack builder page (web/site) with a fake 7TV.

Serves web/site locally, answers 7TV / ivr.fi / CDN requests with the helper's test fixtures,
fills in the form, builds a pack and checks the downloaded zip.

Needs: pip install playwright, and Chrome (or `playwright install chromium`).
Run:   python web/test/page_test.py [path to chrome]
"""
import http.server
import io
import json
import socketserver
import sys
import threading
import zipfile
from functools import partial
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]
SITE = ROOT / "web" / "site"
FX = ROOT / "helper-rs" / "tests" / "fixtures"

ANIM, STAT, CHAN = "01ANIMATEDXXXXXXXXXXXXXXXX", "01STATICXXXXXXXXXXXXXXXXXX", "01CHANNELEMOTEXXXXXXXXXXXX"


def host(eid, f):
    return {"url": f"//cdn.7tv.app/emote/{eid}", "files": [{"name": f, "width": 64, "height": 64, "format": "WEBP"}]}


def active(eid, name, animated):
    return {"id": eid, "name": name, "flags": 0, "data": {"id": eid, "name": name, "animated": animated, "flags": 0, "host": host(eid, "2x.webp")}}


JSON = {
    "https://7tv.io/v3/emote-sets/global": {"emotes": [active(ANIM, "catJAM", True), active(STAT, "OMEGALUL", False)]},
    "https://api.ivr.fi/v2/twitch/user?login=tester": [{"id": "123"}],
    "https://7tv.io/v3/users/twitch/123": {"emote_set": {"id": "S", "emotes": [active(CHAN, "testerHi", False)]}},
}
FILES = {
    f"https://cdn.7tv.app/emote/{ANIM}/2x.webp": (FX / "anim.webp").read_bytes(),
    f"https://cdn.7tv.app/emote/{STAT}/2x.webp": (FX / "static.webp").read_bytes(),
    f"https://cdn.7tv.app/emote/{CHAN}/2x.webp": (FX / "static.webp").read_bytes(),
}


def fake_7tv(route):
    url = route.request.url
    cors = {"Access-Control-Allow-Origin": "*"}
    if url in JSON:
        return route.fulfill(status=200, body=json.dumps(JSON[url]), headers={**cors, "Content-Type": "application/json"})
    if url in FILES:
        return route.fulfill(status=200, body=FILES[url], headers={**cors, "Content-Type": "image/webp"})
    return route.fulfill(status=404, body="not found", headers=cors)


class Quiet(http.server.SimpleHTTPRequestHandler):
    extensions_map = {**http.server.SimpleHTTPRequestHandler.extensions_map, ".wasm": "application/wasm", ".js": "text/javascript"}

    def log_message(self, *a):
        pass


def main():
    chrome = sys.argv[1] if len(sys.argv) > 1 else None
    httpd = socketserver.TCPServer(("127.0.0.1", 0), partial(Quiet, directory=str(SITE)))
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{httpd.server_address[1]}/"
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=chrome, args=["--no-sandbox"]) if chrome else p.chromium.launch()
        page = browser.new_page(accept_downloads=True)
        errors = []
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.on("console", lambda m: m.type == "error" and errors.append(m.text))
        for pattern in ("https://7tv.io/**", "https://api.ivr.fi/**", "https://cdn.7tv.app/**"):
            page.context.route(pattern, fake_7tv)

        # settings come from the link
        page.goto(base + "?channels=tester,bad!name&format=blp")
        assert page.input_value("#channels") == "tester, bad!name", page.input_value("#channels")
        page.click("#build")
        page.wait_for_selector("#result:not([hidden])", timeout=60000)
        status = page.text_content("#statusText")
        assert "3 emotes" in status, status
        assert "bad!name" in page.text_content("#problems")
        with page.expect_download() as dl:
            page.click("#download")
        data = Path(dl.value.path()).read_bytes()
        z = zipfile.ZipFile(io.BytesIO(data))
        names = sorted(z.namelist())
        assert "SevenTVEmotes_Local/Local.lua" in names and "SevenTVEmotes_Local/SevenTVEmotes_Local.toc" in names, names
        assert sum(n.endswith(".blp") for n in names) == 3, names
        lua = z.read("SevenTVEmotes_Local/Local.lua").decode()
        assert 'channels = { "tester" }' in lua and '["catJAM"]' in lua and '["testerHi"]' in lua
        assert dl.value.suggested_filename == "SevenTVEmotes_Local.zip"

        # settings are remembered; the share link reflects them
        page.reload()
        assert page.input_value("#channels").startswith("tester")
        page.fill("#channels", "tester")
        page.uncheck("#global")
        page.check("input[value=tga]")
        page.click("#build")
        page.wait_for_selector("#result:not([hidden])", timeout=60000)
        status = page.text_content("#statusText")
        assert "ready: 1 emote." in status, status
        assert page.is_hidden("#problemsBox")
        assert page.text_content("#version").strip() == "builder " + (SITE / "version.txt").read_text().strip()
        assert not errors, errors
        page.screenshot(path=str(Path("/tmp/page_test.png")), full_page=True)
        browser.close()
    httpd.shutdown()
    print("page test ok")


if __name__ == "__main__":
    main()
