"""The helper-owned emote library for one game folder.

Everything lives in Interface/AddOns/SevenTVEmotes_Local, an addon the helper owns so
that addon managers updating the main addon never wipe it:

    SevenTVEmotes_Local.toc
    Local.lua            emote table + helper status, read by WoW when the UI loads
    slots/s0001.tga ...  reserved texture files (see below)
    helper_state.json    the helper's own bookkeeping

Why slots: WoW only sees texture files that existed when the game started. Overwriting
an existing file only needs a /reload, but a brand-new file needs a full restart. So the
helper keeps a stock of blank placeholder files and writes new emotes into those.
A slot created (or freed) while the game is running is only used after a restart.
"""
from __future__ import annotations

import concurrent.futures as cf
import json
import logging
import os
import re
import shutil
import time
from pathlib import Path

from PIL import Image

from . import __version__
from . import images, seventv
from .luasv import lua_str, lua_value
from .seventv import Emote, NotFound
from .status_board import StatusBoard
from .wow import LOCAL_ADDON, addons_dir

log = logging.getLogger("seventv_helper")

INITIAL_SLOTS = 1500
LOW_WATER = 150     # top up when fewer usable free slots remain
TOP_UP = 500
RESULT_DAYS = 14
REFRESH_EVERY = 6 * 3600
NAME_RE = re.compile(r"^[^\s|]{1,64}$")

PRIORITY_GLOBAL, PRIORITY_CHANNEL, PRIORITY_MANUAL = 0, 1, 2


def _now() -> float:
    return time.time()


class Library:
    def __init__(self, flavor: Path, cache_dir: Path, template_dir: Path | None = None):
        self.flavor = flavor
        self.dir = addons_dir(flavor) / LOCAL_ADDON
        self.slots_dir = self.dir / "slots"     # uncompressed: slots/s0001.tga
        self.cslots_dir = self.dir / "cslots"   # compressed:   cslots/s0001.blp (own folder, see _migrate)
        self.test_dir = self.dir / "test"
        self.state_path = self.dir / "helper_state.json"
        self.cache_dir = cache_dir
        self.template_dir = template_dir
        self.retrying: dict[str, str] = {}  # request id -> message (not persisted)
        self.progress: dict | None = None    # {"done", "total"} while textures are being built
        self.source_errors: dict[str, str] = {}
        self.board = StatusBoard(self.dir / "status")
        self.ready: dict | None = None        # {"t", "msg"}: new content written since the last reload
        self.latest_sv_mtime = 0.0
        self.state = self._load()
        self._migrate()

    def _migrate(self) -> None:
        t = _now()
        for name, slot in self.state["slots"].items():
            slot.setdefault("created_blp", t)
        for info in self.state["images"].values():
            info.setdefault("fmt", "tga")
        if self.state.get("layout", 1) < 2:
            if self.installed:
                # Helper 0.3/0.4 kept s0001.tga and s0001.blp side by side. WoW seems to match texture
                # files by name regardless of extension, so it could load the wrong twin, which is why
                # compressed emotes stayed invisible. Compressed files now live in cslots/.
                self.cslots_dir.mkdir(parents=True, exist_ok=True)
                for name, slot in self.state["slots"].items():
                    images.write_blank(self.cslots_dir / f"{name}.blp")
                    slot["created_blp"] = t
                for f in self.slots_dir.glob("*.blp"):
                    try:
                        f.unlink()
                    except OSError:
                        pass
                n = 0
                for info in self.state["images"].values():
                    if info.get("fmt") == "blp":
                        info["fmt"], info["rebuild"] = "tga", True
                        n += 1
                self.state["cleanup"] = {"files": [], "after": None}
                log.info("%s: moved compressed textures to cslots/ (%d emotes will be rebuilt)", self.flavor.name, n)
            self.state["layout"] = 2
        if self.installed and not (self.dir / "status" / "st0000.tga").exists():
            made = self.board.ensure_files()
            log.info("%s: created %d live-status images (WoW sees them after its next restart)", self.flavor.name, made)
        if self.installed and not (self.test_dir / "t_tga.tga").exists():
            self.make_test_images()

    def make_test_images(self) -> None:
        """Small labelled images in several texture formats, for /7tv texturetest in game."""
        self.test_dir.mkdir(parents=True, exist_ok=True)
        variants = [
            ("t_tga.tga", "TGA", None),
            ("t_dxt5m.blp", "BLP DXT5 + mips", "dxt5m"),
            ("t_dxt5.blp", "BLP DXT5", "dxt5"),
            ("t_argb.blp", "BLP raw", "argb"),
            ("t_pal.blp", "BLP palette", "pal"),
        ]
        from .status_board import render
        for fname, label, kind in variants:
            img = render(label, "test image", "ready").resize((256, 32), Image.LANCZOS)
            path = self.test_dir / fname
            try:
                if kind is None:
                    img.save(path, format="TGA")
                elif kind == "dxt5m":
                    path.write_bytes(images.blp_bytes(img, mips=True))
                elif kind == "dxt5":
                    path.write_bytes(images.blp_bytes(img, mips=False))
                elif kind == "argb":
                    path.write_bytes(images.blp_raw_bytes(img))
                elif kind == "pal":
                    img.convert("RGB").quantize(256).save(path, format="BLP")
            except Exception as e:  # a missing encoder shouldn't break the helper
                log.info("test image %s: %s", fname, e)

    # ------------------------------------------------------------------ state

    def _default_state(self) -> dict:
        return {
            "format": 1,
            "sources": {"global": {"kind": "global", "added": 0, "refreshed": 0, "emotes": []}},
            "manual": [],       # emotes added one by one (Emote json, name may be an alias)
            "removed": [],      # names hidden from channel/global sources
            "images": {},       # 7TV id -> {slot, w, h, tw, th, frames, cols, ms}
            "slots": {},        # slot -> {created, id, freed}
            "processed": {},    # request id -> {ok, msg, name, t}
            "failed": {},       # 7TV id -> {err, t}
            "format": "tga",    # "tga" (uncompressed) or "blp" (DXT5 compressed), set from the addon
            "waiting": [],      # 7TV ids that need a WoW restart before they can show
            "converting": 0,    # textures still in the old format
            "cleanup": {"files": [], "after": None},  # old-format files to blank once the addon reloaded
        }

    def _load(self) -> dict:
        try:
            st = json.loads(self.state_path.read_text(encoding="utf-8"))
            base = self._default_state()
            base.update(st)
            return base
        except FileNotFoundError:
            return self._default_state()
        except (OSError, ValueError) as e:
            log.warning("state file unreadable (%s); starting fresh", e)
            return self._default_state()

    def save(self) -> None:
        self.dir.mkdir(parents=True, exist_ok=True)
        tmp = self.state_path.with_suffix(".json.part")
        tmp.write_text(json.dumps(self.state, indent=1), encoding="utf-8")
        os.replace(tmp, self.state_path)

    # ------------------------------------------------------------------ install

    @property
    def installed(self) -> bool:
        return (self.dir / f"{LOCAL_ADDON}.toc").exists()

    def install(self) -> None:
        """Create/refresh the SevenTVEmotes_Local addon folder and reserve slots."""
        self.dir.mkdir(parents=True, exist_ok=True)
        self.slots_dir.mkdir(exist_ok=True)
        if self.template_dir:
            shutil.copyfile(self.template_dir / f"{LOCAL_ADDON}.toc", self.dir / f"{LOCAL_ADDON}.toc")
        self._create_slots(max(0, INITIAL_SLOTS - len(self.state["slots"])))
        self.board.ensure_files()
        self.make_test_images()
        self.state["layout"] = 2
        self.write_lua(game_start=None)
        self.save()

    def _create_slots(self, n: int) -> None:
        if n <= 0:
            return
        existing = self.state["slots"]
        nums = [int(k[1:]) for k in existing] or [0]
        nxt = max(nums) + 1
        t = _now()
        self.slots_dir.mkdir(parents=True, exist_ok=True)
        self.cslots_dir.mkdir(parents=True, exist_ok=True)
        for i in range(nxt, nxt + n):
            name = f"s{i:04d}"
            images.write_blank(self.slots_dir / f"{name}.tga")
            images.write_blank(self.cslots_dir / f"{name}.blp")
            existing[name] = {"created": t, "created_blp": t, "id": None, "freed": None}
        log.info("%s: reserved %d new texture slots", self.flavor.name, n)

    # ------------------------------------------------------------------ slots

    @staticmethod
    def _file_visible(slot: dict, fmt: str, game_start: float | None) -> bool:
        """Did this slot's file in `fmt` exist when the running game started?"""
        if game_start is None:
            return True
        created = slot.get("created_blp" if fmt == "blp" else "created", 0)
        return created < game_start

    @classmethod
    def _usable(cls, slot: dict, game_start: float | None, fmt: str = "tga") -> bool:
        if slot["id"] is not None:
            return False
        if not cls._file_visible(slot, fmt, game_start):
            return False
        return game_start is None or slot["freed"] is None or slot["freed"] < game_start

    def free_slots(self, game_start: float | None, fmt: str | None = None) -> list[str]:
        fmt = fmt or self.state["format"]
        return sorted(k for k, s in self.state["slots"].items() if self._usable(s, game_start, fmt))

    def _slot_file(self, slot: str, fmt: str) -> Path:
        return (self.cslots_dir if fmt == "blp" else self.slots_dir) / f"{slot}.{fmt}"

    @staticmethod
    def _slot_lua_path(slot: str, fmt: str) -> str:
        return ("cslots\\" if fmt == "blp" else "slots\\") + f"{slot}.{fmt}"

    # ------------------------------------------------------------------ format (compression)

    def set_format(self, fmt: str) -> bool:
        if fmt == "blp" and not images.compression_available():
            log.warning("compressed textures requested but numpy isn't available")
            return False
        if fmt not in ("tga", "blp") or fmt == self.state["format"]:
            return False
        log.info("%s: switching textures to %s", self.flavor.name, fmt)
        self.state["format"] = fmt
        return True

    def _schedule_cleanup(self, files: list[Path]) -> None:
        c = self.state["cleanup"]
        c["files"] = sorted(set(c["files"]) | {str(f) for f in files})
        c["after"] = None  # set when the Local.lua that stops using them is written

    def run_cleanup(self, game_start: float | None, latest_sv_mtime: float) -> bool:
        """Blank old-format files once the addon has reloaded past the switch (or WoW is closed)."""
        c = self.state["cleanup"]
        if not c["files"] or c["after"] is None:
            return False
        if game_start is not None and latest_sv_mtime <= c["after"] + 2:
            return False
        for f in c["files"]:
            p = Path(f)
            if p.exists():
                images.write_blank(p)
        log.info("%s: cleared %d old texture files", self.flavor.name, len(c["files"]))
        c["files"], c["after"] = [], None
        return True

    # ------------------------------------------------------------------ storage

    def storage(self, cache_bytes: int | None = None) -> dict:
        actual = 0
        for d in (self.slots_dir, self.cslots_dir):
            if d.is_dir():
                for e in os.scandir(d):
                    if e.is_file():
                        actual += e.stat().st_size
        imgs = self.state["images"]
        blank_tga, blank_blp = images.tga_size(8, 8), images.blp_size(8, 8)
        empty = (len(self.state["slots"]) - len(imgs)) * (blank_tga + blank_blp)
        as_tga = sum(images.tga_size(i["tw"], i["th"]) for i in imgs.values()) + len(imgs) * blank_blp + empty
        as_blp = sum(images.blp_size(i["tw"], i["th"]) for i in imgs.values()) + len(imgs) * blank_tga + empty
        out = {
            "bytes": actual, "tga": as_tga, "blp": as_blp, "format": self.state["format"],
            "converting": self.state.get("converting", 0), "emotes": len(imgs),
            "compressAvailable": images.compression_available(),
        }
        if cache_bytes is not None:
            out["cache"] = cache_bytes
        return out

    def top_up_slots(self, game_start: float | None) -> None:
        free_total = sum(1 for s in self.state["slots"].values() if s["id"] is None)
        if free_total < LOW_WATER:
            self._create_slots(TOP_UP)

    # ------------------------------------------------------------------ desired emote set

    def _sources_in_order(self):
        srcs = self.state["sources"]
        ordered = sorted(srcs.items(), key=lambda kv: (0 if kv[1]["kind"] == "global" else 1, kv[1].get("added", 0)))
        for key, src in ordered:
            yield key, [Emote.from_json(e) for e in src.get("emotes", [])]
        yield "manual", [Emote.from_json(e) for e in self.state["manual"]]

    def desired_with_sources(self) -> dict[str, tuple[Emote, str]]:
        removed = set(self.state["removed"])
        out: dict[str, tuple[Emote, str]] = {}
        for key, ems in self._sources_in_order():
            for em in ems:
                if key != "manual" and em.name in removed:
                    continue
                if NAME_RE.match(em.name) and seventv.ID_RE.match(em.id):
                    out[em.name] = (em, key)
        return out

    def desired(self) -> dict[str, Emote]:
        return {n: em for n, (em, _) in self.desired_with_sources().items()}

    def source_label(self, key: str) -> str:
        if key == "global":
            return "7TV global emotes"
        if key == "manual":
            return "Added by you"
        if key.startswith("channel:"):
            return f"{key.split(':', 1)[1]}'s channel"
        if key.startswith("set:"):
            return "Set: " + self.state["sources"].get(key, {}).get("label", key[4:])
        return key

    # ------------------------------------------------------------------ reconcile

    def reconcile(self, game_start: float | None, jobs: int = 8, progress=None) -> dict:
        """Make textures match the desired emote set and format.

        Returns {"added", "waiting", "failed", "converted"} sets of 7TV ids."""
        fmt = self.state["format"]
        want = self.desired()
        need_ids: dict[str, Emote] = {}
        for em in want.values():
            need_ids.setdefault(em.id, em)
        imgs, slots = self.state["images"], self.state["slots"]
        result = {"added": set(), "waiting": set(), "failed": set(), "converted": set()}

        # Free textures nobody uses any more (blank the file to give the disk space back).
        t = _now()
        for eid in [i for i in imgs if i not in need_ids]:
            info = imgs.pop(eid)
            slot = info["slot"]
            if slot in slots:
                slots[slot].update(id=None, freed=t)
                f = self._slot_file(slot, info.get("fmt", "tga"))
                if f.exists():
                    images.write_blank(f)

        # Work list: (kind, emote, slot, format)
        #   new     : emote not on disk yet, needs a free slot
        #   rebuild : emote whose file must be rewritten in its own slot (e.g. after moving folders)
        #   convert : emote in the other format, same slot
        # If the wanted format's files aren't visible to the running game yet, new emotes are built
        # as TGA for now and converted after the next WoW restart.
        work = []
        free = self.free_slots(game_start, fmt)
        fallback = [x for x in self.free_slots(game_start, "tga") if x not in set(free)] if fmt != "tga" else []
        fallback_set = set(fallback)
        for eid, em in need_ids.items():
            if eid in imgs:
                continue
            if free:
                slot = free.pop(0)
                fallback_set.discard(slot)
                work.append(("new", em, slot, fmt))
            else:
                slot = next((x for x in fallback if x in fallback_set), None)
                if slot is None:
                    result["waiting"].add(eid)
                    continue
                fallback_set.discard(slot)
                work.append(("new", em, slot, "tga"))
        old_files: list[Path] = []
        pending_conv = 0
        for eid, info in imgs.items():
            slot_rec = slots[info["slot"]]
            if info.get("rebuild"):
                f = fmt if self._file_visible(slot_rec, fmt, game_start) else "tga"
                work.append(("rebuild", need_ids[eid], info["slot"], f))
                continue
            # BLPs from helper 0.3.0 had no mipmaps; re-encode them in place.
            stale_blp = fmt == "blp" and info.get("fmt") == "blp" and not info.get("mips")
            if info.get("fmt", "tga") != fmt or stale_blp:
                if self._file_visible(slot_rec, fmt, game_start):
                    work.append(("convert", need_ids[eid], info["slot"], fmt))
                else:
                    pending_conv += 1

        def build(item):
            _, em, slot, f = item
            src = images.download(em, 64, self.cache_dir)
            return images.render(src, self._slot_file(slot, f))

        if work:
            total, done = len(work), 0
            self.progress = {"done": 0, "total": total}
            if progress:
                progress(0, total)
            with cf.ThreadPoolExecutor(max_workers=max(1, jobs)) as pool:
                futs = {pool.submit(build, w): w for w in work}
                for fut in cf.as_completed(futs):
                    kind, em, slot, f = futs[fut]
                    done += 1
                    try:
                        info = fut.result()
                    except Exception as e:  # one bad emote shouldn't stop the rest
                        log.warning("failed to build %s (%s): %s", em.name, em.id, e)
                        if kind == "new":
                            self.state["failed"][em.id] = {"err": str(e)[:200], "t": _now()}
                            result["failed"].add(em.id)
                        elif kind == "convert":
                            pending_conv += 1
                    else:
                        if kind in ("convert", "rebuild"):
                            prev = imgs[em.id].get("fmt", "tga")
                            if prev != f:
                                old_files.append(self._slot_file(slot, prev))
                            result["converted"].add(em.id)
                        else:
                            slots[slot].update(id=em.id, freed=None)
                            self.state["failed"].pop(em.id, None)
                            result["added"].add(em.id)
                        imgs[em.id] = dict(info, slot=slot, fmt=f, mips=(f == "blp"))
                        if f != fmt:
                            pending_conv += 1
                    self.progress = {"done": done, "total": total}
                    if progress:
                        progress(done, total)
            self.progress = None

        if old_files:
            self._schedule_cleanup(old_files)
        self.state["waiting"] = sorted(result["waiting"])
        self.state["converting"] = pending_conv
        # Anything that couldn't get a slot now will once the game restarts.
        self.top_up_slots(game_start)
        if work:
            log.info("%s: %d new textures, %d converted, %d waiting for a restart, %d failed", self.flavor.name,
                     len(result["added"]), len(result["converted"]), len(result["waiting"]), len(result["failed"]))
        return result

    # ------------------------------------------------------------------ Local.lua

    def channels(self) -> list[str]:
        return sorted(k.split(":", 1)[1] for k in self.state["sources"] if k.startswith("channel:"))

    def write_lua(self, game_start: float | None, notice: str | None = None, cache_bytes: int | None = None) -> None:
        imgs = self.state["images"]
        cutoff = _now() - RESULT_DAYS * 86400
        results = {k: v for k, v in self.state["processed"].items() if v.get("t", 0) >= cutoff}
        want = self.desired_with_sources()
        waiting_ids = set(self.state.get("waiting", []))
        status = {
            "version": __version__,
            "heartbeat": int(_now()),
            "slotsFree": len(self.free_slots(game_start)),
            "channels": self.channels(),
            "results": {k: {"ok": v["ok"], "msg": v["msg"], "name": v.get("name")} for k, v in results.items()},
            "retrying": dict(self.retrying),
            "waiting": sorted(n for n, (em, _) in want.items() if em.id in waiting_ids),
            "storage": self.storage(cache_bytes),
            "board": self.board.info(),
            "origin": {n: self.source_label(k) for n, (em, k) in want.items() if em.id in imgs},
        }
        notice = notice or self.notice()
        if notice:
            status["notice"] = notice
        if self.progress:
            status["progress"] = dict(self.progress, t=int(_now()))
        lines = [
            "-- Written by the 7TV Emotes Helper app. Anything you change here gets overwritten.",
            "SevenTVEmotes_Helper = " + lua_value(status),
            "",
            "-- [name] = { file, width, height, texWidth, texHeight, 7tvId [, frames, columns, msPerFrame] }",
            "local emotes = {",
        ]
        for name, (em, _) in sorted(want.items(), key=lambda kv: kv[0].lower()):
            info = imgs.get(em.id)
            if not info:
                continue
            if info.get("rebuild"):
                continue
            path = self._slot_lua_path(info["slot"], info.get("fmt", "tga"))
            vals = [lua_str(path), info["w"], info["h"], info["tw"], info["th"], lua_str(em.id)]
            if info["frames"] > 1:
                vals += [info["frames"], info["cols"], info["ms"]]
            lines.append("  [%s] = { %s }," % (lua_str(name), ", ".join(str(v) for v in vals)))
        lines += [
            "}",
            "if SevenTVEmotes and SevenTVEmotes.RegisterPack then",
            '  SevenTVEmotes.RegisterPack("My emotes", emotes, "%s")' % LOCAL_ADDON,
            "end",
            "",
        ]
        out = self.dir / "Local.lua"
        tmp = out.with_suffix(".lua.part")
        tmp.write_text("\n".join(lines), encoding="utf-8", newline="\n")
        os.replace(tmp, out)
        c = self.state["cleanup"]
        if c["files"] and c["after"] is None:
            c["after"] = _now()

    # ------------------------------------------------------------------ sources

    def refresh_sources(self, max_age: float = REFRESH_EVERY, force: bool = False) -> bool:
        """Re-download global/channel emote lists that are older than max_age. Returns True if anything changed.

        A source that has never loaded (e.g. the global set on first run while offline) is retried
        every minute instead of every few hours."""
        changed = False
        for key, src in list(self.state["sources"].items()):
            age = _now() - src.get("refreshed", 0)
            never = not src.get("refreshed")
            if not force:
                if never and _now() - src.get("last_attempt", 0) < 60:
                    continue
                if not never and age < max_age:
                    continue
            src["last_attempt"] = _now()
            try:
                if src["kind"] == "global":
                    ems = seventv.fetch_global()
                elif src["kind"] == "channel":
                    _, ems = seventv.fetch_twitch_channel(src["login"])
                elif src["kind"] == "set":
                    _, ems = seventv.fetch_set(src["set_id"])
                else:
                    continue
            except NotFound as e:
                log.warning("source %s: %s", key, e)
                self.source_errors[key] = str(e)
                src["refreshed"] = _now()
                continue
            except Exception as e:  # network, TLS, unexpected JSON...
                log.warning("source %s: couldn't download (%s); will retry", key, e)
                self.source_errors[key] = str(e)[:160]
                continue
            self.source_errors.pop(key, None)
            new = [e.to_json() for e in ems]
            if new != src.get("emotes"):
                src["emotes"] = new
                changed = True
            if never:
                log.info("source %s: %d emotes", key, len(new))
            src["refreshed"] = _now()
        return changed

    def notice(self) -> str | None:
        g = self.state["sources"].get("global")
        if g and not g.get("refreshed") and "global" in self.source_errors:
            return ("Couldn't download 7TV's global emotes yet (" + self.source_errors["global"] +
                    "). The helper keeps trying every minute.")
        return None

    # ------------------------------------------------------------------ requests

    def _find_cached(self, name: str) -> Emote | None:
        for _, ems in self._sources_in_order():
            for em in ems:
                if em.name == name:
                    return em
        return None

    def apply_request(self, req: dict) -> dict:
        """Apply one in-game request to the desired state. Returns {ok, msg, name?}.

        Network errors propagate (ConnectionError) so the caller can retry later."""
        kind = req.get("kind")
        ref = str(req.get("ref") or "").strip()
        alias = req.get("alias") or None
        if not ref:
            return {"ok": False, "msg": "Empty request."}
        try:
            if kind == "add":
                return self._req_add(ref, alias)
            if kind == "remove":
                return self._req_remove(ref)
            if kind == "channel":
                return self._req_channel(ref.lower())
            if kind == "unchannel":
                key = f"channel:{ref.lower()}"
                if self.state["sources"].pop(key, None) is None:
                    return {"ok": False, "msg": f"You haven't added {ref}'s channel."}
                return {"ok": True, "msg": f"Removed {ref}'s channel emotes."}
        except NotFound as e:
            return {"ok": False, "msg": _friendly_not_found(kind, ref, e)}
        except ValueError as e:
            return {"ok": False, "msg": str(e)}
        return {"ok": False, "msg": f"Unknown request '{kind}'."}

    def _req_add(self, ref: str, alias: str | None) -> dict:
        what, value = seventv.parse_ref(ref)
        if what == "set":
            name, ems = seventv.fetch_set(value)
            self.state["sources"][f"set:{value}"] = {"kind": "set", "set_id": value, "added": _now(),
                                                    "refreshed": _now(), "emotes": [e.to_json() for e in ems],
                                                    "label": name}
            return {"ok": True, "msg": f"Added the emote set '{name}' ({len(ems)} emotes)."}
        if what == "id":
            em = seventv.fetch_emote(value)
        else:
            em = self._find_cached(value)
            if em is None:
                try:
                    em = seventv.search_by_name(value)
                except NotFound:
                    em = None
            if em is None:
                return {"ok": False, "msg": f"Couldn't find a 7TV emote called {value}. "
                                            f"Copy its link from 7tv.app and use /7tv add <link>."}
        name = alias or em.name
        if not NAME_RE.match(name):
            return {"ok": False, "msg": f"'{name}' can't be used as an emote name in WoW chat (no spaces or |)."}
        em = Emote(name, em.id, em.animated, em.host)
        self.state["manual"] = [m for m in self.state["manual"] if m["name"] != name] + [em.to_json()]
        if name in self.state["removed"]:
            self.state["removed"].remove(name)
        return {"ok": True, "msg": f"Added {name}.", "name": name}

    def _req_remove(self, name: str) -> dict:
        before = len(self.state["manual"])
        self.state["manual"] = [m for m in self.state["manual"] if m["name"] != name]
        from_sources = any(em.name == name for key, ems in self._sources_in_order() if key != "manual" for em in ems)
        if from_sources and name not in self.state["removed"]:
            self.state["removed"].append(name)
        if before == len(self.state["manual"]) and not from_sources:
            return {"ok": False, "msg": f"{name} isn't one of your emotes."}
        return {"ok": True, "msg": f"Removed {name}."}

    def _req_channel(self, login: str) -> dict:
        twitch_id, ems = seventv.fetch_twitch_channel(login)
        key = f"channel:{login}"
        prev = self.state["sources"].get(key)
        self.state["sources"][key] = {"kind": "channel", "login": login, "twitch_id": twitch_id,
                                      "added": prev["added"] if prev else _now(), "refreshed": _now(),
                                      "emotes": [e.to_json() for e in ems]}
        return {"ok": True, "msg": f"Added {len(ems)} emotes from {login}'s 7TV channel."}

    def finish_request(self, req_id: str, result: dict, build: dict) -> dict:
        """Adjust a request's message after textures were built, then record it."""
        name = result.get("name")
        if result["ok"] and name:
            em = self.desired().get(name)
            if em and em.id in build["waiting"]:
                result = dict(result, msg=f"{name} is ready, but WoW needs a restart to show it (out of reserved slots).")
            elif em and em.id in build["failed"]:
                err = self.state["failed"].get(em.id, {}).get("err", "unknown error")
                result = {"ok": False, "msg": f"Couldn't convert {name}: {err}", "name": name}
        elif result["ok"] and build["waiting"]:
            result = dict(result, msg=result["msg"] + f" {len(build['waiting'])} will show after you restart WoW.")
        result["t"] = _now()
        self.state["processed"][req_id] = result
        return result

    def prune(self) -> None:
        cutoff = _now() - 60 * 86400
        self.state["processed"] = {k: v for k, v in self.state["processed"].items() if v.get("t", 0) >= cutoff}


def _friendly_not_found(kind: str, ref: str, e: Exception) -> str:
    msg = str(e)
    if kind in ("channel", "unchannel"):
        return msg if "7TV" in msg or "Twitch" in msg else f"Couldn't find {ref} on 7TV."
    return "Couldn't find that emote on 7TV. Check the link."
