"""Where the helper keeps its own files, and its config."""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

APP_NAME = "SevenTVEmotesHelper"


def app_dir() -> Path:
    if sys.platform == "win32":
        base = Path(os.environ.get("LOCALAPPDATA") or Path.home() / "AppData" / "Local")
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support"
    else:
        base = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local" / "share")
    d = base / APP_NAME
    d.mkdir(parents=True, exist_ok=True)
    return d


def cache_dir() -> Path:
    return app_dir() / "cache"


def log_file() -> Path:
    return app_dir() / "helper.log"


def bundled_addon_dir() -> Path | None:
    """The addon folders shipped with the helper (inside the .exe, or the repo when run from source)."""
    if getattr(sys, "frozen", False):
        p = Path(getattr(sys, "_MEIPASS", Path(sys.executable).parent)) / "addon"
    else:
        p = Path(__file__).resolve().parents[2] / "addon"
    return p if (p / "SevenTVEmotes" / "SevenTVEmotes.toc").exists() else None


DEFAULT_CONFIG = {
    "wow_dirs": [],      # extra World of Warcraft folders to look in
    "flavors": [],       # extra game folders to manage besides Forever, e.g. "_retail_"
    "first_run_done": False,
}


def load_config() -> dict:
    p = app_dir() / "config.json"
    cfg = dict(DEFAULT_CONFIG)
    try:
        cfg.update(json.loads(p.read_text(encoding="utf-8")))
    except (OSError, ValueError):
        pass
    return cfg


def save_config(cfg: dict) -> None:
    p = app_dir() / "config.json"
    p.write_text(json.dumps(cfg, indent=2), encoding="utf-8")
