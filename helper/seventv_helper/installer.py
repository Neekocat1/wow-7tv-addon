"""Installing the addons into WoW, copying the helper somewhere permanent, and autostart."""
from __future__ import annotations

import logging
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

from .library import Library
from .paths import APP_NAME, app_dir, bundled_addon_dir, cache_dir
from .wow import CORE_ADDON, LOCAL_ADDON, addons_dir

log = logging.getLogger("seventv_helper")


# --------------------------------------------------------------------------- addons

def toc_version(toc: Path) -> tuple[int, ...]:
    try:
        m = re.search(r"^##\s*Version:\s*([\d.]+)", toc.read_text(encoding="utf-8", errors="replace"), re.M)
        return tuple(int(x) for x in m.group(1).split(".")) if m else ()
    except OSError:
        return ()


def ensure_addons(flavor: Path) -> tuple[Library, list[str]]:
    """Make sure both addons are in this game folder. Returns (library, list of changes made)."""
    changes = []
    bundle = bundled_addon_dir()
    ad = addons_dir(flavor)
    ad.mkdir(parents=True, exist_ok=True)

    core_dst = ad / CORE_ADDON
    if bundle:
        core_src = bundle / CORE_ADDON
        have = toc_version(core_dst / f"{CORE_ADDON}.toc")
        ship = toc_version(core_src / f"{CORE_ADDON}.toc")
        if not have or ship > have:
            shutil.copytree(core_src, core_dst, dirs_exist_ok=True)
            changes.append(f"{'installed' if not have else 'updated'} {CORE_ADDON} in {flavor.name}")

    lib = Library(flavor, cache_dir(), bundle / LOCAL_ADDON if bundle else None)
    if not lib.installed:
        lib.install()
        changes.append(f"created {LOCAL_ADDON} in {flavor.name}")
    return lib, changes


# --------------------------------------------------------------------------- the helper itself

def frozen() -> bool:
    return bool(getattr(sys, "frozen", False))


def installed_exe() -> Path:
    return app_dir() / f"{APP_NAME}.exe"


def launch_command(background: bool = True) -> list[str]:
    """Command line that starts the helper (used for autostart)."""
    flag = ["--background"] if background else []
    if frozen():
        return [str(installed_exe() if installed_exe().exists() else Path(sys.executable))] + flag
    runner = Path(__file__).resolve().parents[1] / "run_helper.pyw"
    py = Path(sys.executable)
    pyw = py.with_name("pythonw.exe")
    return [str(pyw if pyw.exists() else py), str(runner)] + flag


def install_self() -> Path | None:
    """Copy the .exe into the app folder so it keeps working if the download is deleted, and so
    autostart always runs the newest version. Never replaces a newer installed copy.
    Returns the installed path if a copy was made."""
    if not frozen() or sys.platform != "win32":
        return None
    from . import __version__
    from .instance import vtuple
    src, dst = Path(sys.executable).resolve(), installed_exe()
    if dst.exists() and src == dst.resolve():
        return None
    marker = app_dir() / "installed_version"
    try:
        installed = marker.read_text().strip()
    except OSError:
        installed = ""
    if dst.exists() and vtuple(installed) > vtuple(__version__):
        log.info("installed helper %s is newer than this one (%s); not replacing it", installed, __version__)
        return None
    # The old helper may take a moment to fully exit and unlock its .exe.
    import time
    for attempt in range(20):
        try:
            shutil.copy2(src, dst)
            break
        except PermissionError:
            if attempt == 19:
                raise
            time.sleep(0.5)
    marker.write_text(__version__)
    try:
        set_autostart(True)
    except OSError:
        pass
    return dst


def relaunch_installed() -> None:
    subprocess.Popen([str(installed_exe())], close_fds=True,
                     creationflags=getattr(subprocess, "DETACHED_PROCESS", 0) | getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0))


# --------------------------------------------------------------------------- autostart

_RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"
_PLIST = Path.home() / "Library" / "LaunchAgents" / "com.seventvemotes.helper.plist"


def _quote(args: list[str]) -> str:
    return " ".join(f'"{a}"' if " " in a else a for a in args)


def autostart_enabled() -> bool:
    if sys.platform == "win32":
        import winreg
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, _RUN_KEY) as k:
                winreg.QueryValueEx(k, APP_NAME)
                return True
        except OSError:
            return False
    if sys.platform == "darwin":
        return _PLIST.exists()
    return False


def set_autostart(enabled: bool) -> None:
    if sys.platform == "win32":
        import winreg
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, _RUN_KEY, 0, winreg.KEY_SET_VALUE) as k:
            if enabled:
                winreg.SetValueEx(k, APP_NAME, 0, winreg.REG_SZ, _quote(launch_command()))
            else:
                try:
                    winreg.DeleteValue(k, APP_NAME)
                except FileNotFoundError:
                    pass
    elif sys.platform == "darwin":
        if enabled:
            args = "".join(f"<string>{a}</string>" for a in launch_command())
            _PLIST.parent.mkdir(parents=True, exist_ok=True)
            _PLIST.write_text(
                '<?xml version="1.0" encoding="UTF-8"?>\n'
                '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
                '<plist version="1.0"><dict><key>Label</key><string>com.seventvemotes.helper</string>'
                f'<key>ProgramArguments</key><array>{args}</array>'
                '<key>RunAtLoad</key><true/></dict></plist>\n', encoding="utf-8")
        elif _PLIST.exists():
            _PLIST.unlink()
    else:
        log.info("autostart isn't supported on this platform")


# --------------------------------------------------------------------------- misc

def open_path(p: Path) -> None:
    if sys.platform == "win32":
        os.startfile(str(p))  # type: ignore[attr-defined]
    elif sys.platform == "darwin":
        subprocess.Popen(["open", str(p)])
    else:
        subprocess.Popen(["xdg-open", str(p)])
