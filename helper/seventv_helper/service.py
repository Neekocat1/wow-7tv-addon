"""The background loop: watch for in-game requests, fetch emotes, keep the library fresh."""
from __future__ import annotations

import logging
import os
import threading
import time
from pathlib import Path
from typing import Callable

from . import installer, instance, luasv, wow
from .library import Library
from .paths import cache_dir

log = logging.getLogger("seventv_helper")

POLL_SECONDS = 2.0
_UNSET = object()
HEARTBEAT_SECONDS = 5 * 60
REDISCOVER_SECONDS = 10 * 60
RETRY_SECONDS = 30
MAX_ATTEMPTS = 3


def read_sv(sv_file: Path) -> tuple[list[dict], dict]:
    """Returns (queued requests, the addon's settings table)."""
    text = sv_file.read_text(encoding="latin-1")  # Lua strings are bytes; luasv decodes UTF-8
    data = luasv.parse(text)
    db = data.get("SevenTVEmotesDB") or {}
    if not isinstance(db, dict):
        db = {}
    queue = db.get("queue")
    if isinstance(queue, dict):  # sparse table
        queue = [queue[k] for k in sorted(k for k in queue if isinstance(k, int))]
    return [r for r in (queue or []) if isinstance(r, dict) and r.get("id")], db


def read_queue(sv_file: Path) -> list[dict]:
    return read_sv(sv_file)[0]


def dir_size(path: Path) -> int:
    total = 0
    try:
        for e in os.scandir(path):
            if e.is_file():
                total += e.stat().st_size
            elif e.is_dir():
                total += dir_size(Path(e.path))
    except OSError:
        pass
    return total


class Service:
    def __init__(self, config: dict, notify: Callable[[str, str], None] | None = None):
        self.config = config
        self.notify = notify or (lambda title, msg: None)
        self.on_status: Callable[[], None] = lambda: None
        self.on_quit: Callable[[], None] = lambda: None
        self.helper_start = time.time()
        self.roots: list[Path] = []
        self.libs: dict[Path, Library] = {}
        self.sv_mtimes: dict[Path, float] = {}
        self.attempts: dict[str, int] = {}
        self.retry_at = 0.0
        self.last_heartbeat = 0.0
        self.last_discover = 0.0
        self.status = "Starting..."
        self._last_game = None
        self._last_gs = _UNSET
        self._stop = threading.Event()
        self._wake = threading.Event()
        self._force_refresh = False
        self._thread: threading.Thread | None = None

    # ------------------------------------------------------------------ lifecycle

    def start(self) -> None:
        self._thread = threading.Thread(target=self.run, name="seventv-helper", daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self._stop.set()
        self._wake.set()

    def request_refresh(self) -> None:
        self._force_refresh = True
        self._wake.set()

    def set_status(self, text: str, log_it: bool = True) -> None:
        if text != self.status:
            self.status = text
            if log_it:
                log.info("status: %s", text)
            self.on_status()

    # ------------------------------------------------------------------ discovery

    def game_start(self) -> float | None:
        try:
            procs = wow.find_game_processes(self.roots)
        except RuntimeError:
            # Without process info, treat slots made during this helper run as unusable
            # until the next start (normally the next boot, before WoW is launched).
            if self._last_game != "unknown":
                log.warning("psutil missing: can't tell whether WoW is running; being careful with new files")
                self._last_game = "unknown"
            return self.helper_start
        desc = ", ".join(f"{p['name']} (pid {p['pid']})" for p in procs) or "not running"
        if desc != self._last_game:
            log.info("WoW: %s", desc)
            self._last_game = desc
        starts = [p["started"] for p in procs if p["started"]]
        return min(starts) if starts else None

    def discover(self) -> list[str]:
        self.last_discover = time.time()
        self.roots = wow.find_wow_roots(self.config.get("wow_dirs"))
        flavors = wow.pick_flavors(self.roots, also=self.config.get("flavors"))
        changes = []
        for f in flavors:
            if f in self.libs:
                continue
            try:
                lib, ch = installer.ensure_addons(f)
            except OSError as e:
                log.error("couldn't set up %s: %s", f, e)
                continue
            self.libs[f] = lib
            changes += ch
            log.info("managing %s", f)
        for c in changes:
            log.info(c)
        return changes

    # ------------------------------------------------------------------ main loop

    def run(self) -> None:
        try:
            changes = self.discover()
            if changes:
                self.notify("7TV Emotes", "Addon installed. If WoW is open, restart it once to load it.")
        except Exception:
            log.exception("discovery failed")
        while not self._stop.is_set():
            try:
                self.tick()
            except Exception:
                log.exception("helper loop error")
                self.set_status("Something went wrong - see the log")
            self._wake.wait(POLL_SECONDS)
            self._wake.clear()

    def tick(self) -> None:
        if instance.quit_requested():
            log.info("a newer helper asked this one to quit")
            self.stop()
            self.on_quit()
            return
        now = time.time()
        if now - self.last_discover > REDISCOVER_SECONDS:
            self.discover()
        if not self.libs:
            self.set_status("Waiting: World of Warcraft: Forever not found")
            return
        gs = self.game_start()
        session_changed = gs != self._last_gs
        if session_changed and self._last_gs is not _UNSET:
            # WoW started or closed: files that were waiting for a restart are usable now.
            # Build them right away so they're ready before the player reaches the world.
            log.info("WoW session changed; building anything that was waiting for a restart")
        self._last_gs = gs
        force = self._force_refresh
        self._force_refresh = False
        heartbeat_due = now - self.last_heartbeat > HEARTBEAT_SECONDS
        retry_due = self.retry_at and now >= self.retry_at

        for lib in self.libs.values():
            self.process_library(lib, gs, force=force, retry=bool(retry_due),
                                 heartbeat=heartbeat_due or session_changed)
        if heartbeat_due:
            self.last_heartbeat = now
        if retry_due:
            self.retry_at = 0.0
        total = sum(len(lib.desired()) for lib in self.libs.values())
        self.set_status(f"Running - {total} emotes" + (" (WoW open)" if gs and gs != self.helper_start else ""))

    def process_library(self, lib: Library, gs: float | None, *, force=False, retry=False, heartbeat=False) -> list[dict]:
        """One pass over a game folder. Returns the results of requests handled this pass."""
        pending: list[dict] = []
        latest_mtime, latest_db = 0.0, None
        for sv in wow.saved_variable_files(lib.flavor):
            try:
                mtime = sv.stat().st_mtime
            except OSError:
                continue
            latest_mtime = max(latest_mtime, mtime)
            if self.sv_mtimes.get(sv) == mtime and not retry and not force:
                continue
            try:
                queue, db = read_sv(sv)
            except (OSError, luasv.LuaParseError) as e:
                log.info("couldn't read %s yet (%s)", sv, e)  # probably mid-write; try next poll
                continue
            self.sv_mtimes[sv] = mtime
            pending += [r for r in queue if r["id"] not in lib.state["processed"]]
            if latest_db is None or mtime >= latest_db[0]:
                latest_db = (mtime, db)

        lib.latest_sv_mtime = max(lib.latest_sv_mtime, latest_mtime)
        if latest_db and latest_db[1].get("invisible"):
            inv = latest_db[1]["invisible"]
            inv = list(inv.values()) if isinstance(inv, dict) else list(inv)
            log.warning("the addon reports %d emote file(s) WoW couldn't load this session, e.g. %s",
                        len(inv), ", ".join(map(str, inv[:3])))

        # Settings from the most recently saved account.
        fmt_changed = False
        if latest_db and "compress" in latest_db[1]:
            fmt_changed = lib.set_format("blp" if latest_db[1]["compress"] else "tga")

        handled: list[tuple[str, dict]] = []
        seen = set()
        retry_changed = False
        if pending:
            self.set_status(f"Fetching {len(pending)} request(s)...")
            self.board(lib, f"Fetching {len(pending)} request{'s' if len(pending) != 1 else ''} from 7TV...", "busy", force=True)
        for req in pending:
            if req["id"] in seen:
                continue
            seen.add(req["id"])
            try:
                res = lib.apply_request(req)
            except ConnectionError as e:
                n = self.attempts.get(req["id"], 0) + 1
                self.attempts[req["id"]] = n
                if n < MAX_ATTEMPTS:
                    log.info("7TV unreachable for %s (attempt %d): %s", req["id"], n, e)
                    lib.retrying[req["id"]] = f"7TV couldn't be reached. Retrying (attempt {n} of {MAX_ATTEMPTS})."
                    retry_changed = True
                    self.retry_at = time.time() + RETRY_SECONDS
                    continue
                res = {"ok": False, "msg": "Couldn't reach 7TV. Check your internet connection and try again."}
            lib.retrying.pop(req["id"], None)
            handled.append((req["id"], res))

        cleaned = lib.run_cleanup(gs, latest_mtime)
        refreshed = lib.refresh_sources(force=force)
        stuck = bool(lib.state.get("waiting") or lib.state.get("converting"))
        rebuild = any(i.get("rebuild") for i in lib.state["images"].values())
        if fmt_changed:
            self.set_status("Converting emotes...")
        if handled or refreshed or force or fmt_changed or rebuild or (heartbeat and stuck):
            build = lib.reconcile(gs, progress=self._progress_reporter(lib, gs))
            results = [lib.finish_request(rid, res, build) for rid, res in handled]
            lib.ready = self._ready_message(build, results, fmt_changed) or lib.ready
            lib.prune()
            lib.write_lua(gs, cache_bytes=dir_size(cache_dir()))
            lib.save()
            self.last_heartbeat = time.time()
            for r in results:
                log.info("request: %s", r["msg"])
            if lib.ready:
                lib.ready["t"] = time.time()
            self.update_board(lib)
            if results:
                ok = [r for r in results if r["ok"]]
                msg = "; ".join(r["msg"] for r in results)[:240]
                self.notify("7TV Emotes", msg + (" Reload WoW (/reload) to see it." if ok else ""))
            elif fmt_changed:
                self.notify("7TV Emotes", "Emotes converted. Reload WoW (/reload) to use them.")
            return results
        if heartbeat or retry_changed or cleaned:
            lib.write_lua(gs, cache_bytes=dir_size(cache_dir()))
            if cleaned:
                lib.save()
        self.update_board(lib)
        return []

    # ------------------------------------------------------------------ live status board

    @staticmethod
    def _ready_message(build: dict, results: list[dict], fmt_changed: bool) -> dict | None:
        added = len(build["added"])
        if added:
            return {"t": time.time(), "msg": f"{added} new emote{'s' if added != 1 else ''} ready"}
        if build["converted"]:
            return {"t": time.time(), "msg": "Emotes converted"}
        if any(r["ok"] for r in results):
            return {"t": time.time(), "msg": "Your changes are ready"}
        if results:
            return {"t": time.time(), "msg": "Request finished (see /7tv)"}
        return None

    def board_text(self, lib: Library) -> tuple[str, str]:
        if lib.progress:
            return f"Downloading emotes {lib.progress['done']}/{lib.progress['total']}", "busy"
        if lib.retrying:
            return "Can't reach 7TV - retrying", "warn"
        if lib.notice():
            return "Can't download 7TV's global emotes - retrying", "error"
        if lib.ready and lib.ready["t"] > lib.latest_sv_mtime + 1:
            return lib.ready["msg"] + " - click Reload", "ready"
        waiting = len(lib.state.get("waiting", []))
        if waiting:
            return f"{waiting} emote{'s' if waiting != 1 else ''} will show after a WoW restart", "warn"
        return f"Up to date - {len(lib.desired())} emotes", "idle"

    def board(self, lib: Library, text: str, kind: str, force: bool = False) -> None:
        try:
            lib.board.update(text, kind)
        except Exception as e:  # never let the status display break the helper
            log.info("status board: %s", e)

    def update_board(self, lib: Library) -> None:
        text, kind = self.board_text(lib)
        self.board(lib, text, kind)

    def _progress_reporter(self, lib: Library, gs: float | None):
        """Status/log updates while textures build, plus partial Local.lua writes so a
        reload in the middle of a big download already shows what's done."""
        last = {"write": time.time(), "logged": -1, "board": 0.0}

        def report(done: int, total: int):
            self.set_status(f"Downloading emotes {done}/{total}", log_it=False)
            if time.time() - last["board"] >= 1 or done == total:
                last["board"] = time.time()
                self.board(lib, f"Downloading emotes {done}/{total}", "busy")
            step = max(1, total // 10)
            if done == total or done // step != last["logged"]:
                last["logged"] = done // step
                log.info("emotes %d/%d", done, total)
            now = time.time()
            if 0 < done < total and now - last["write"] >= 5:
                last["write"] = now
                try:
                    lib.write_lua(gs)
                    lib.save()
                except OSError as e:
                    log.info("partial write failed: %s", e)
        return report

    def run_once(self) -> list[dict]:
        """Single synchronous pass (used by `--once` and tests)."""
        self.discover()
        gs = self.game_start()
        out = []
        for lib in self.libs.values():
            out += self.process_library(lib, gs, force=False, retry=True, heartbeat=True)
        return out
