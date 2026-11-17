"""Finding World of Warcraft installs, game folders and whether the game is running."""
from __future__ import annotations

import os
import re
import string
import sys
import time
from pathlib import Path

CORE_ADDON = "SevenTVEmotes"
LOCAL_ADDON = "SevenTVEmotes_Local"
SV_FILE = "SevenTVEmotes.lua"

# Game folders ("flavors") we install into by default. The Forever folder was
# _classic_beta_ during the beta and _forever_ at launch.
DEFAULT_FLAVOR_HINTS = ("forever",)


def _registry_paths() -> list[Path]:
    if sys.platform != "win32":
        return []
    try:
        import winreg
    except ImportError:
        return []
    out = []
    keys = [
        r"SOFTWARE\WOW6432Node\Blizzard Entertainment\World of Warcraft",
        r"SOFTWARE\Blizzard Entertainment\World of Warcraft",
    ]
    for key in keys:
        for flag in (0, getattr(winreg, "KEY_WOW64_64KEY", 0)):
            try:
                with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, key, 0, winreg.KEY_READ | flag) as k:
                    val, _ = winreg.QueryValueEx(k, "InstallPath")
                    out.append(Path(val))
            except OSError:
                pass
    return out


def _common_paths() -> list[Path]:
    out = []
    if sys.platform == "win32":
        drives = [f"{d}:\\" for d in string.ascii_uppercase if os.path.exists(f"{d}:\\")]
        for d in drives:
            for sub in ("Program Files (x86)", "Program Files", "Games", "Battle.net", ""):
                out.append(Path(d) / sub / "World of Warcraft")
    elif sys.platform == "darwin":
        out += [Path("/Applications/World of Warcraft"), Path.home() / "Applications/World of Warcraft"]
    return out


def is_flavor_dir(p: Path) -> bool:
    return p.is_dir() and p.name.startswith("_") and p.name.endswith("_") and (
        (p / "Interface").is_dir() or (p / "WTF").is_dir() or any(p.glob("Wow*.exe")) or any(p.glob("World of Warcraft*.app")))


def find_wow_roots(extra: list[str] | None = None) -> list[Path]:
    """Folders like C:/Program Files (x86)/World of Warcraft (the parent of _retail_, _forever_, ...)."""
    seen, roots = set(), []
    for p in [Path(x) for x in (extra or [])] + _registry_paths() + _common_paths():
        try:
            p = p.resolve()
        except OSError:
            continue
        if p.name.startswith("_") and p.name.endswith("_"):  # registry often points at _retail_
            p = p.parent
        key = str(p).lower()
        if key in seen or not p.is_dir():
            continue
        seen.add(key)
        if any(is_flavor_dir(c) for c in p.iterdir() if c.is_dir()):
            roots.append(p)
    return roots


def flavor_dirs(root: Path) -> list[Path]:
    return sorted(c for c in root.iterdir() if is_flavor_dir(c))


def pick_flavors(roots: list[Path], hints=DEFAULT_FLAVOR_HINTS, also: list[str] | None = None) -> list[Path]:
    """Flavor folders to manage: ones matching the hints, ones named in `also`, and any
    that already have the addon installed."""
    out = []
    for root in roots:
        for f in flavor_dirs(root):
            name = f.name.lower()
            installed = (f / "Interface" / "AddOns" / CORE_ADDON).is_dir()
            if installed or any(h in name for h in hints) or (also and f.name in also):
                out.append(f)
    return out


def addons_dir(flavor: Path) -> Path:
    return flavor / "Interface" / "AddOns"


def saved_variable_files(flavor: Path) -> list[Path]:
    """Account-wide SavedVariables for the addon, one per WoW account."""
    wtf = flavor / "WTF" / "Account"
    if not wtf.is_dir():
        return []
    return sorted(wtf.glob(f"*/SavedVariables/{SV_FILE}"))


# --------------------------------------------------------------------------- game process

_EXE_RE = re.compile(r"^(wow|world ?of ?warcraft|forever)[^\\/]*?(\.exe)?$", re.I)
_NOT_GAME = ("battle.net", "agent", "blizzard", "wowup", "warcraftlogs", "raider", "tsm", "curseforge")


def find_game_processes(roots: list[Path]) -> list[dict]:
    """Running WoW clients: [{pid, name, exe, started}]. Raises RuntimeError without psutil."""
    try:
        import psutil
    except ImportError as e:
        raise RuntimeError("psutil not installed") from e
    root_strs = [str(r).lower().rstrip("\\/") for r in roots]
    found = []
    for proc in psutil.process_iter(["pid", "name", "exe", "create_time"]):
        info = proc.info
        name = info.get("name") or ""
        exe = (info.get("exe") or "").lower()
        lname = name.lower()
        if lname.startswith(_NOT_GAME):
            continue
        in_root = bool(exe) and any(exe.startswith(r + "\\") or exe.startswith(r + "/") for r in root_strs)
        if not (in_root or _EXE_RE.match(name)):
            continue
        found.append({"pid": info.get("pid"), "name": name, "exe": info.get("exe") or "",
                      "started": info.get("create_time")})
    return found


def game_start_time(roots: list[Path]) -> float | None:
    """Earliest start time of a running WoW client, None if none is running.

    Raises RuntimeError if process information isn't available (psutil missing)."""
    starts = [p["started"] for p in find_game_processes(roots) if p["started"]]
    return min(starts) if starts else None


def now() -> float:
    return time.time()
