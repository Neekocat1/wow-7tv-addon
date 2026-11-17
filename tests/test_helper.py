"""End-to-end tests for the helper against a fake WoW install and a fake 7TV.

Run:  python -m pytest tests -q
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

import pytest
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "helper"))

from seventv_helper import luasv, seventv, wow  # noqa: E402
from seventv_helper.seventv import NotFound  # noqa: E402

ANIM = "01ANIMATEDXXXXXXXXXXXXXXXX"
STAT = "01STATICXXXXXXXXXXXXXXXXXX"
WIDE = "60ae958e229664e8667aea38"
CHAN = "01CHANNELEMOTEXXXXXXXXXXXX"
ZW = "01ZEROWIDTHXXXXXXXXXXXXXXX"


def host(eid, fname, w, h, fmt):
    return {"url": f"//cdn.test/emote/{eid}", "files": [{"name": fname, "width": w, "height": h, "format": fmt}]}


def active(eid, name, animated, h, flags=0):
    return {"id": eid, "name": name, "flags": 0, "data": {"id": eid, "name": name, "animated": animated, "flags": flags, "host": h}}


@pytest.fixture
def fake(tmp_path, monkeypatch):
    fx = tmp_path / "fixtures"
    fx.mkdir()
    frames = []
    for i in range(30):
        im = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
        ImageDraw.Draw(im).ellipse((i, 0, i + 32, 32), fill=(255, i * 8, 0, 255))
        frames.append(im)
    frames[0].save(fx / "anim.webp", save_all=True, append_images=frames[1:], duration=50, loop=0)
    s = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    ImageDraw.Draw(s).rectangle((8, 8, 56, 56), fill=(0, 200, 255, 255))
    s.save(fx / "static.webp")
    Image.new("RGBA", (128, 32), (255, 255, 0, 255)).save(fx / "wide.png")

    H = {
        ANIM: host(ANIM, "2x.webp", 64, 64, "WEBP"),
        STAT: host(STAT, "2x.webp", 64, 64, "WEBP"),
        WIDE: host(WIDE, "2x.png", 128, 32, "PNG"),
        CHAN: host(CHAN, "2x.webp", 64, 64, "WEBP"),
        ZW: host(ZW, "2x.webp", 64, 64, "WEBP"),
    }
    files = {
        f"https://cdn.test/emote/{ANIM}/2x.webp": fx / "anim.webp",
        f"https://cdn.test/emote/{STAT}/2x.webp": fx / "static.webp",
        f"https://cdn.test/emote/{WIDE}/2x.png": fx / "wide.png",
        f"https://cdn.test/emote/{CHAN}/2x.webp": fx / "static.webp",
    }
    glob = {"name": "Global", "emotes": [
        active(ANIM, "catJAM", True, H[ANIM]),
        active(STAT, "OMEGALUL", False, H[STAT]),
        active(ZW, "RainTime", False, H[ZW], flags=256),
    ]}
    channel = {"emote_set": {"id": "SET1", "name": "tester's set", "emotes": [
        active(CHAN, "testerHi", False, H[CHAN]),
        active(STAT, "Clap", False, H[STAT]),
    ]}}
    state = {"online": True, "calls": []}

    def fake_http(url, *, binary=False, data=None, retries=3):
        state["calls"].append(url)
        if not state["online"]:
            raise ConnectionError("offline")
        if binary:
            return files[url].read_bytes()
        if url.endswith("/emote-sets/global"):
            return glob
        if url.endswith("/users/twitch/123"):
            return channel
        if "ivr.fi" in url:
            if url.endswith("tester"):
                return [{"id": "123", "login": "tester"}]
            return []
        if url.endswith(f"/emotes/{WIDE}"):
            return {"id": WIDE, "name": "WideDank", "animated": False, "host": H[WIDE]}
        if url == seventv.GQL:
            q = data["variables"]["query"]
            if q == "peepoHappy":
                return {"data": {"emotes": {"items": [{"id": WIDE, "name": "peepoHappy", "animated": False, "host": H[WIDE]}]}}}
            return {"data": {"emotes": {"items": []}}}
        raise NotFound(url)

    monkeypatch.setattr(seventv, "http", fake_http)
    import seventv_helper.images as images_mod
    monkeypatch.setattr(images_mod, "http", fake_http)

    # Fake WoW install
    wow_root = tmp_path / "World of Warcraft"
    forever = wow_root / "_forever_"
    (forever / "Interface" / "AddOns").mkdir(parents=True)
    (forever / "WTF" / "Account" / "ACCT1" / "SavedVariables").mkdir(parents=True)
    (wow_root / "_retail_" / "Interface" / "AddOns").mkdir(parents=True)
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "appdata"))
    monkeypatch.setattr(wow, "find_game_processes", lambda roots: [])
    return {"root": wow_root, "forever": forever, "state": state, "tmp": tmp_path}


def make_service(fake):
    from seventv_helper.service import Service
    return Service({"wow_dirs": [str(fake["root"])], "flavors": []})


def write_sv(fake, queue):
    sv = fake["forever"] / "WTF" / "Account" / "ACCT1" / "SavedVariables" / "SevenTVEmotes.lua"
    body = "SevenTVEmotesDB = " + luasv.lua_value({"enabled": True, "size": 28, "queue": queue, "disabled": {}}) + "\n"
    sv.write_text(body, encoding="utf-8")
    t = time.time() + len(queue)  # make sure mtime changes between writes
    import os
    os.utime(sv, (t, t))
    return sv


def local_lua(fake) -> str:
    return (fake["forever"] / "Interface" / "AddOns" / "SevenTVEmotes_Local" / "Local.lua").read_text(encoding="utf-8")


def state(fake) -> dict:
    return json.loads((fake["forever"] / "Interface" / "AddOns" / "SevenTVEmotes_Local" / "helper_state.json").read_text())


# --------------------------------------------------------------------------- tests

def test_install_and_global(fake):
    svc = make_service(fake)
    svc.run_once()
    ad = fake["forever"] / "Interface" / "AddOns"
    assert (ad / "SevenTVEmotes" / "SevenTVEmotes.toc").exists()
    assert (ad / "SevenTVEmotes_Local" / "SevenTVEmotes_Local.toc").exists()
    assert not (fake["root"] / "_retail_" / "Interface" / "AddOns" / "SevenTVEmotes").exists(), "retail untouched"
    assert len(list((ad / "SevenTVEmotes_Local" / "slots").glob("*.tga"))) >= 1500
    assert len(list((ad / "SevenTVEmotes_Local" / "cslots").glob("*.blp"))) >= 1500
    assert not list((ad / "SevenTVEmotes_Local" / "slots").glob("*.blp")), "no same-name twins"
    assert sorted(p.name for p in (ad / "SevenTVEmotes_Local" / "test").iterdir()) == [
        "t_argb.blp", "t_dxt5.blp", "t_dxt5m.blp", "t_pal.blp", "t_tga.tga"]
    lua = local_lua(fake)
    assert '["catJAM"] = { "slots\\\\s0001.tga"' in lua or '["catJAM"] = { "slots\\\\s0002.tga"' in lua
    assert "storage = {" in lua and 'origin = {' in lua and '7TV global emotes' in lua
    assert '["OMEGALUL"]' in lua
    assert "RainTime" not in lua, "zero-width skipped"
    st = state(fake)
    assert len(st["images"]) == 2
    anim = st["images"][ANIM]
    assert anim["frames"] > 1 and anim["cols"] >= 1
    # the slot file was overwritten with a real texture
    im = Image.open(ad / "SevenTVEmotes_Local" / ("cslots" if anim["fmt"] == "blp" else "slots") / f"{anim['slot']}.{anim['fmt']}")
    assert im.size == (anim["tw"], anim["th"])


def test_requests(fake):
    svc = make_service(fake)
    svc.run_once()
    write_sv(fake, [
        {"id": "r1", "kind": "add", "ref": f"https://7tv.app/emotes/{WIDE}", "t": 1},
        {"id": "r2", "kind": "add", "ref": "peepoHappy", "t": 1},
        {"id": "r3", "kind": "channel", "ref": "tester", "t": 1},
        {"id": "r4", "kind": "add", "ref": "https://7tv.app/emotes/01NOPEXXXXXXXXXXXXXXXXXXXX", "t": 1},
        {"id": "r5", "kind": "remove", "ref": "OMEGALUL", "t": 1},
        {"id": "r6", "kind": "add", "ref": "catJAM", "alias": "dance", "t": 1},
        {"id": "r7", "kind": "channel", "ref": "nobody", "t": 1},
        {"id": "r8", "kind": "add", "ref": "NotARealEmote", "t": 1},
    ])
    results = svc.run_once()
    msgs = {r["msg"] for r in results}
    print(msgs)
    st = state(fake)
    p = st["processed"]
    assert p["r1"]["ok"] and p["r1"]["name"] == "WideDank"
    assert p["r2"]["ok"] and p["r2"]["name"] == "peepoHappy"
    assert p["r3"]["ok"] and "2 emotes" in p["r3"]["msg"]
    assert not p["r4"]["ok"]
    assert p["r5"]["ok"]
    assert p["r6"]["ok"] and p["r6"]["name"] == "dance"
    assert not p["r7"]["ok"]
    assert not p["r8"]["ok"] and "7tv.app" in p["r8"]["msg"]
    lua = local_lua(fake)
    for name in ("WideDank", "peepoHappy", "testerHi", "Clap", "dance", "catJAM"):
        assert f'["{name}"]' in lua, name
    assert '["OMEGALUL"]' not in lua
    # results are exposed to the addon
    assert 'r1 = ' in lua and "Added WideDank." in lua
    # WideDank and peepoHappy share a 7TV id -> one texture
    assert sum(1 for i in st["images"] if i == WIDE) == 1
    import os
    if os.environ.get("DUMP_LOCAL"):
        Path(os.environ["DUMP_LOCAL"]).write_text(lua, encoding="utf-8")
    # reprocessing the same queue does nothing new
    assert svc.run_once() == []


def test_unchannel_and_readd(fake):
    svc = make_service(fake)
    write_sv(fake, [{"id": "a", "kind": "channel", "ref": "tester", "t": 1}])
    svc.run_once()
    assert '["testerHi"]' in local_lua(fake)
    slot = state(fake)["images"][CHAN]["slot"]
    write_sv(fake, [{"id": "a", "kind": "channel", "ref": "tester", "t": 1},
                    {"id": "b", "kind": "unchannel", "ref": "tester", "t": 1}])
    svc.run_once()
    st = state(fake)
    assert '["testerHi"]' not in local_lua(fake)
    assert CHAN not in st["images"]
    assert st["slots"][slot]["id"] is None and st["slots"][slot]["freed"]


def test_slots_respect_running_game(fake, monkeypatch):
    svc = make_service(fake)
    svc.run_once()  # installs + global, slots created "before the game started"
    started = time.time() + 1
    monkeypatch.setattr(wow, "find_game_processes", lambda roots: [{"pid": 1, "name": "Wow.exe", "exe": "", "started": started}])
    # Pretend every remaining free slot was created after the game started.
    lib = next(iter(svc.libs.values()))
    for s in lib.state["slots"].values():
        if s["id"] is None:
            s["created"] = started + 5
    lib.save()
    write_sv(fake, [{"id": "w1", "kind": "add", "ref": f"https://7tv.app/emotes/{WIDE}", "t": 1}])
    res = svc.run_once()
    assert res and res[0]["ok"] and "restart" in res[0]["msg"]
    assert '["WideDank"]' not in local_lua(fake)
    # After the game restarts (not running), the next pass builds it.
    monkeypatch.setattr(wow, "find_game_processes", lambda roots: [])
    svc.request_refresh()
    lib2 = next(iter(svc.libs.values()))
    svc.process_library(lib2, None, force=True)
    assert '["WideDank"]' in local_lua(fake)


def test_offline_retries_then_fails(fake):
    svc = make_service(fake)
    svc.run_once()
    fake["state"]["online"] = False
    write_sv(fake, [{"id": "o1", "kind": "add", "ref": f"https://7tv.app/emotes/{WIDE}", "t": 1}])
    assert svc.run_once() == []          # attempt 1: retry later
    assert svc.run_once() == []          # attempt 2
    res = svc.run_once()                 # attempt 3: give up with a message
    assert res and not res[0]["ok"] and "reach 7TV" in res[0]["msg"]


def test_parse_ref():
    assert seventv.parse_ref("https://7tv.app/emotes/01F6MQ33FG000FFJ97ZB8MWV52") == ("id", "01F6MQ33FG000FFJ97ZB8MWV52")
    assert seventv.parse_ref("7tv.app/emotes/60ae958e229664e8667aea38") == ("id", "60ae958e229664e8667aea38")
    assert seventv.parse_ref("https://7tv.app/emote-sets/01HKQT8EWR000ESSWF3625XCS4") == ("set", "01HKQT8EWR000ESSWF3625XCS4")
    assert seventv.parse_ref("01F6MQ33FG000FFJ97ZB8MWV52") == ("id", "01F6MQ33FG000FFJ97ZB8MWV52")
    assert seventv.parse_ref("catJAM") == ("name", "catJAM")


def test_luasv_wow_format():
    text = (
        'SevenTVEmotesDB = {\n'
        '\t["enabled"] = true,\n'
        '\t["size"] = 28,\n'
        '\t["ratio"] = 1.5,\n'
        '\t["disabled"] = {\n\t},\n'
        '\t["queue"] = {\n'
        '\t\t{\n'
        '\t\t\t["id"] = "1763164800-0042",\n'
        '\t\t\t["kind"] = "add",\n'
        '\t\t\t["ref"] = "https://7tv.app/emotes/01F6MQ33FG000FFJ97ZB8MWV52",\n'
        '\t\t\t["alias"] = "caf\\195\\169 \\"q\\"",\n'
        '\t\t}, -- [1]\n'
        '\t},\n'
        '}\n'
        'OtherVar = nil\n'
    )
    d = luasv.parse(text)
    db = d["SevenTVEmotesDB"]
    assert db["enabled"] is True and db["size"] == 28 and db["ratio"] == 1.5
    assert db["disabled"] == []
    q = db["queue"]
    assert q[0]["kind"] == "add" and q[0]["alias"] == 'café "q"'
    assert d["OtherVar"] is None
    # round trip
    again = luasv.parse("X = " + luasv.lua_value(db))
    assert again["X"]["queue"][0]["alias"] == 'café "q"'


def write_sv_db(fake, db):
    sv = fake["forever"] / "WTF" / "Account" / "ACCT1" / "SavedVariables" / "SevenTVEmotes.lua"
    sv.write_text("SevenTVEmotesDB = " + luasv.lua_value(db) + "\n", encoding="utf-8")
    import os
    t = time.time() + 100 + len(str(db))
    os.utime(sv, (t, t))
    return t


def test_compression_switch_and_cleanup(fake, monkeypatch):
    svc = make_service(fake)
    svc.run_once()
    lib = next(iter(svc.libs.values()))
    slots = lib.slots_dir
    st0 = state(fake)
    tga_slot = st0["images"][STAT]["slot"]
    size_tga = (slots / f"{tga_slot}.tga").stat().st_size
    assert size_tga > 1000
    # Game running, slots existed before it started: conversion can happen now.
    started = time.time() + 1
    monkeypatch.setattr(wow, "find_game_processes", lambda roots: [{"pid": 1, "name": "Wow.exe", "exe": "", "started": started}])
    t = write_sv_db(fake, {"compress": True, "queue": []})
    svc.run_once()
    st = state(fake)
    assert st["format"] == "blp"
    assert all(i["fmt"] == "blp" for i in st["images"].values())
    lua = local_lua(fake)
    assert ".blp\"" in lua and ".tga\"" not in lua.split("local emotes")[1]
    # BLP decodes and matches the original layout
    info = st["images"][ANIM]
    im = Image.open(lib.cslots_dir / f"{info['slot']}.blp")
    assert im.size == (info["tw"], info["th"])
    im.convert("RGBA").load()
    # old .tga files are kept until the addon has reloaded past the switch
    assert (slots / f"{tga_slot}.tga").stat().st_size == size_tga
    assert st["cleanup"]["files"] and st["cleanup"]["after"]
    # next reload (newer saved variables) -> old files blanked
    import os
    sv = fake["forever"] / "WTF" / "Account" / "ACCT1" / "SavedVariables" / "SevenTVEmotes.lua"
    later = st["cleanup"]["after"] + 10
    os.utime(sv, (later, later))
    svc.run_once()
    assert (slots / f"{tga_slot}.tga").stat().st_size < 400
    st2 = state(fake)
    assert not st2["cleanup"]["files"]
    storage = lib.storage()
    assert storage["format"] == "blp" and storage["blp"] < storage["tga"]


def test_compression_waits_for_restart_when_slots_are_new(fake, monkeypatch):
    svc = make_service(fake)
    svc.run_once()
    lib = next(iter(svc.libs.values()))
    started = time.time() + 1
    monkeypatch.setattr(wow, "find_game_processes", lambda roots: [{"pid": 1, "name": "Wow.exe", "exe": "", "started": started}])
    for s in lib.state["slots"].values():
        s["created_blp"] = started + 5   # .blp files appeared after the game started
    lib.save()
    write_sv_db(fake, {"compress": True, "queue": []})
    svc.run_once()
    st = state(fake)
    assert st["format"] == "blp" and st["converting"] == len(st["images"])
    assert all(i["fmt"] == "tga" for i in st["images"].values())
    assert "converting = %d" % len(st["images"]) in local_lua(fake)


def test_retrying_is_reported(fake):
    svc = make_service(fake)
    svc.run_once()
    fake["state"]["online"] = False
    write_sv(fake, [{"id": "q1", "kind": "channel", "ref": "tester", "t": 1}])
    svc.run_once()
    assert "retrying = {" in local_lua(fake) and "q1 = " in local_lua(fake)


def test_first_run_offline_then_global_arrives(fake, monkeypatch):
    fake["state"]["online"] = False
    svc = make_service(fake)
    svc.run_once()
    lua = local_lua(fake)
    assert "Couldn't download 7TV's global emotes yet" in lua
    assert '["catJAM"]' not in lua
    lib = next(iter(svc.libs.values()))
    # retried again only after a minute
    calls = len(fake["state"]["calls"])
    svc.run_once()
    assert len(fake["state"]["calls"]) == calls
    lib.state["sources"]["global"]["last_attempt"] -= 61
    fake["state"]["online"] = True
    svc.run_once()
    lua = local_lua(fake)
    assert '["catJAM"]' in lua and "Couldn't download" not in lua


def test_progress_reported(fake):
    svc = make_service(fake)
    seen = []
    svc.set_status = lambda text, log_it=True: seen.append(text)
    svc.run_once()
    assert "Downloading emotes 0/2" in seen and "Downloading emotes 2/2" in seen


def test_blp_has_mips_and_estimate_matches(fake, monkeypatch):
    svc = make_service(fake)
    svc.run_once()
    write_sv_db(fake, {"compress": True, "queue": []})
    svc.run_once()
    lib = next(iter(svc.libs.values()))
    from seventv_helper import images
    for info in lib.state["images"].values():
        f = lib.cslots_dir / f"{info['slot']}.blp"
        assert f.stat().st_size == images.blp_size(info["tw"], info["th"])
        raw = f.read_bytes()
        assert raw[:4] == b"BLP2" and raw[8:13] == bytes([2, 8, 7, 1]) + b""[:0] or raw[8:12] == bytes([2, 8, 7, 1])


def test_waiting_emotes_built_when_wow_restarts(fake, monkeypatch):
    svc = make_service(fake)
    svc.run_once()
    lib = next(iter(svc.libs.values()))
    started = time.time() + 1
    procs = [{"pid": 1, "name": "WowForever.exe", "exe": "", "started": started}]
    monkeypatch.setattr(wow, "find_game_processes", lambda roots: procs)
    for s in lib.state["slots"].values():
        if s["id"] is None:
            s["created"] = started + 5
    write_sv(fake, [{"id": "w9", "kind": "add", "ref": f"https://7tv.app/emotes/{WIDE}", "t": 1}])
    svc.tick()
    assert '["WideDank"]' not in local_lua(fake)
    # WoW restarts: new process, started after the slots were made
    procs[0] = {"pid": 2, "name": "WowForever.exe", "exe": "", "started": started + 60}
    svc.tick()
    assert '["WideDank"]' in local_lua(fake)


def test_old_blps_without_mips_are_reencoded(fake):
    svc = make_service(fake)
    svc.run_once()
    write_sv_db(fake, {"compress": True, "queue": []})
    svc.run_once()
    lib = next(iter(svc.libs.values()))
    for info in lib.state["images"].values():
        info.pop("mips", None)  # pretend these came from 0.3.0
    lib.save()
    svc.request_refresh()
    svc.process_library(lib, None, force=True)
    from seventv_helper import images
    for info in lib.state["images"].values():
        f = lib.cslots_dir / f"{info['slot']}.blp"
        assert info["mips"] and f.read_bytes()[11] == 1
        assert f.stat().st_size == images.blp_size(info["tw"], info["th"])


def test_status_board_written_ahead_and_ready_message(fake, monkeypatch):
    from seventv_helper import status_board as sb
    svc = make_service(fake)
    svc.run_once()
    lib = next(iter(svc.libs.values()))
    status = lib.dir / "status"
    assert len(list(status.glob("st*.tga"))) == sb.COUNT
    assert "board = {" in local_lua(fake) and "count = %d" % sb.COUNT in local_lua(fake)
    now = time.time()
    k = sb.index_at(now)
    big = [i for i in range(sb.COUNT) if (status / f"st{i:04d}.tga").stat().st_size > 1000]
    assert k in big or (k + 1) % sb.COUNT in big
    assert len(big) <= sb.AHEAD + 3
    # new request -> after processing the board says "ready - click Reload"
    write_sv(fake, [{"id": "b1", "kind": "add", "ref": f"https://7tv.app/emotes/{WIDE}", "t": 1}])
    lib.latest_sv_mtime = 0
    import os
    sv = fake["forever"] / "WTF" / "Account" / "ACCT1" / "SavedVariables" / "SevenTVEmotes.lua"
    os.utime(sv, (now - 5, now - 5))
    svc.sv_mtimes.clear()
    svc.run_once()
    text, kind = svc.board_text(lib)
    assert kind == "ready" and "click Reload" in text, text
    # the player reloads (saved variables newer than the ready time) -> back to idle
    later = time.time() + 30
    os.utime(sv, (later, later))
    svc.run_once()
    text, kind = svc.board_text(lib)
    assert kind == "idle" and "Up to date" in text, text


def test_status_board_blanks_old_images(tmp_path):
    from seventv_helper import status_board as sb
    b = sb.StatusBoard(tmp_path / "status")
    b.ensure_files()
    t0 = 1_800_000_000.0
    b.update("A", "busy", now=t0)
    first = sb.index_at(t0)
    b.update("A", "busy", now=t0 + sb.PERIOD * 20)
    assert (tmp_path / "status" / f"st{first:04d}.tga").stat().st_size < 400
    k = sb.index_at(t0 + sb.PERIOD * 20)
    assert (tmp_path / "status" / f"st{k:04d}.tga").stat().st_size > 1000


def test_migration_moves_compressed_out_of_slots(fake, monkeypatch):
    """0.4 layout: s0001.tga + s0001.blp twins in slots/. 0.5 moves .blp to cslots/ and rebuilds."""
    svc = make_service(fake)
    svc.run_once()
    write_sv_db(fake, {"compress": True, "queue": []})
    svc.run_once()
    lib = next(iter(svc.libs.values()))
    # Recreate the old layout
    for name in lib.state["slots"]:
        src = lib.cslots_dir / f"{name}.blp"
        (lib.slots_dir / f"{name}.blp").write_bytes(src.read_bytes())
    for info in lib.state["images"].values():
        info["fmt"] = "blp"
    lib.state["layout"] = 1
    lib.save()
    import shutil
    shutil.rmtree(lib.cslots_dir)
    # WoW is running: the new cslots/ files were created after it started.
    started = time.time() - 60
    monkeypatch.setattr(wow, "find_game_processes", lambda roots: [{"pid": 1, "name": "Wow.exe", "exe": "", "started": started}])
    from seventv_helper.service import Service
    svc2 = Service({"wow_dirs": [str(fake["root"])], "flavors": []})
    svc2.run_once()
    lib2 = next(iter(svc2.libs.values()))
    assert not list(lib2.slots_dir.glob("*.blp"))
    assert all(i["fmt"] == "tga" and not i.get("rebuild") for i in lib2.state["images"].values()), lib2.state["images"]
    lua = local_lua(fake)
    assert "slots\\\\s" in lua and ".tga" in lua.split("local emotes")[1]
    assert lib2.state["converting"] == len(lib2.state["images"])
    # WoW restarts -> converted into cslots/
    monkeypatch.setattr(wow, "find_game_processes", lambda roots: [{"pid": 2, "name": "Wow.exe", "exe": "", "started": time.time() + 5}])
    svc2.tick()
    lua = local_lua(fake)
    assert "cslots\\\\s" in lua.split("local emotes")[1]
    assert all(i["fmt"] == "blp" for i in lib2.state["images"].values())


def test_new_emotes_fall_back_to_tga_until_blp_visible(fake, monkeypatch):
    svc = make_service(fake)
    svc.run_once()
    write_sv_db(fake, {"compress": True, "queue": []})
    svc.run_once()
    lib = next(iter(svc.libs.values()))
    started = time.time() + 1
    monkeypatch.setattr(wow, "find_game_processes", lambda roots: [{"pid": 1, "name": "Wow.exe", "exe": "", "started": started}])
    for s in lib.state["slots"].values():
        if s["id"] is None:
            s["created_blp"] = started + 5   # compressed placeholders not visible yet
    lib.save()
    write_sv(fake, [{"id": "f1", "kind": "add", "ref": f"https://7tv.app/emotes/{WIDE}", "t": 1}])
    svc.run_once()
    info = lib.state["images"][WIDE]
    assert info["fmt"] == "tga"
    assert '["WideDank"] = { "slots\\\\' in local_lua(fake)


def test_newer_helper_takes_over(tmp_path, monkeypatch):
    import threading
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "appdata"))
    from seventv_helper import instance
    old = instance.SingleInstance()
    assert old.acquire()
    instance.write_running("0.4.0")
    quit_seen = threading.Event()

    def old_helper():  # what Service.tick does
        while not instance.quit_requested():
            time.sleep(0.1)
        quit_seen.set()
        old.release()

    threading.Thread(target=old_helper, daemon=True).start()
    new = instance.SingleInstance()
    got, other = instance.take_over(new, "0.5.0", timeout=5)
    assert got and other == "0.4.0" and quit_seen.is_set()
    assert not instance.quit_requested()
    new.release()


def test_older_helper_does_not_replace_newer(tmp_path, monkeypatch):
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "appdata"))
    from seventv_helper import instance
    running = instance.SingleInstance()
    assert running.acquire()
    instance.write_running("0.6.0")
    got, other = instance.take_over(instance.SingleInstance(), "0.5.0", timeout=1)
    assert not got and other == "0.6.0" and not instance.quit_requested()
    running.release()


def test_service_stops_when_asked(fake):
    from seventv_helper import instance
    svc = make_service(fake)
    stopped = []
    svc.on_quit = lambda: stopped.append(True)
    (instance.app_dir() / "quit.request").write_text("0.9.0")
    svc.tick()
    assert stopped and svc._stop.is_set()
