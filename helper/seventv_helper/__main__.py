"""Entry point.

    SevenTVEmotesHelper.exe            first run: install everything, then run in the tray
    ... --background                   started by Windows at login (no first-run messages)
    ... --no-tray                      run in the console instead of the tray
    ... --once                         one pass, then exit (handy for testing)
    ... --wow-dir "D:/Games/World of Warcraft"   if WoW isn't found automatically
    ... --flavor _retail_              also manage another game folder
    ... --autostart on|off
"""
from __future__ import annotations

import argparse
import logging
import logging.handlers
import sys
from pathlib import Path

from . import __version__, installer, instance
from .paths import app_dir, load_config, log_file, save_config
from .service import Service

log = logging.getLogger("seventv_helper")


def setup_logging(console: bool) -> None:
    log.setLevel(logging.INFO)
    fmt = logging.Formatter("%(asctime)s %(levelname)s %(message)s")
    fh = logging.handlers.RotatingFileHandler(log_file(), maxBytes=1_000_000, backupCount=2, encoding="utf-8")
    fh.setFormatter(fmt)
    log.addHandler(fh)
    if console and sys.stderr:
        sh = logging.StreamHandler()
        sh.setFormatter(fmt)
        log.addHandler(sh)


def message_box(title: str, text: str, ask_folder: bool = False) -> str | None:
    """Minimal dialogs for first run, using tkinter (bundled with Python)."""
    try:
        import tkinter as tk
        from tkinter import filedialog, messagebox
    except ImportError:
        print(f"{title}: {text}")
        return None
    root = tk.Tk()
    root.withdraw()
    try:
        if ask_folder:
            messagebox.showinfo(title, text)
            return filedialog.askdirectory(title="Select your World of Warcraft folder") or None
        messagebox.showinfo(title, text)
        return None
    finally:
        root.destroy()


def first_run(cfg: dict, service: Service, interactive: bool) -> None:
    changes = service.discover()
    if not service.libs and interactive:
        picked = message_box("7TV Emotes Helper",
                             "I couldn't find World of Warcraft: Forever automatically.\n\n"
                             "Pick your World of Warcraft folder (the one that contains _forever_).",
                             ask_folder=True)
        if picked:
            cfg["wow_dirs"] = list(dict.fromkeys(cfg.get("wow_dirs", []) + [picked]))
            changes = service.discover()
    if installer.frozen():  # running from source (development): leave autostart alone
        try:
            installer.set_autostart(True)
        except OSError as e:
            log.warning("couldn't enable autostart: %s", e)
    cfg["first_run_done"] = True
    save_config(cfg)
    if interactive:
        if service.libs:
            where = "\n".join(f"  {f}" for f in service.libs)
            message_box("7TV Emotes Helper",
                        "All set! The 7TV Emotes addon is installed in:\n\n" + where +
                        "\n\nThe helper now runs quietly in your system tray and starts with your computer. "
                        "In game, type /7tv to get started.\n\nIf WoW is open, restart it once.")
        else:
            message_box("7TV Emotes Helper",
                        "World of Warcraft wasn't found, so nothing was installed yet. The helper will keep "
                        "looking in the background; you can also run it with --wow-dir.")


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="SevenTVEmotesHelper", description="7TV Emotes Helper " + __version__)
    p.add_argument("--background", action="store_true", help="started automatically at login")
    p.add_argument("--no-tray", action="store_true", help="run in the console")
    p.add_argument("--once", action="store_true", help="one pass, then exit")
    p.add_argument("--wow-dir", action="append", default=[], help="World of Warcraft folder")
    p.add_argument("--flavor", action="append", default=[], help="extra game folder to manage, e.g. _retail_")
    p.add_argument("--autostart", choices=["on", "off"])
    p.add_argument("--version", action="version", version=__version__)
    args = p.parse_args(argv)

    setup_logging(console=args.no_tray or args.once)
    cfg = load_config()
    if args.wow_dir or args.flavor:
        cfg["wow_dirs"] = list(dict.fromkeys(cfg.get("wow_dirs", []) + args.wow_dir))
        cfg["flavors"] = list(dict.fromkeys(cfg.get("flavors", []) + args.flavor))
        save_config(cfg)

    if args.autostart:
        installer.set_autostart(args.autostart == "on")
        print(f"autostart {args.autostart}")
        return 0

    service = Service(cfg)

    # Only one helper at a time. A newer helper asks an older running one to quit and takes over.
    lock = instance.SingleInstance()
    got, other = instance.take_over(lock, __version__, timeout=15)
    if not got:
        if other and instance.vtuple(other) > instance.vtuple(__version__):
            msg = f"A newer helper ({other}) is already running; this one ({__version__}) won't replace it."
        else:
            msg = ("The helper is already running and didn't close when asked (versions before 0.5 can't be "
                   "asked). Right-click the 7TV icon in your system tray, choose Quit, then start this again.")
        if args.once or args.no_tray:
            print(msg)
        elif not args.background:
            message_box("7TV Emotes Helper", msg)
        return 0
    if other:
        log.info("took over from helper %s", other)

    if args.once:
        try:
            for r in service.run_once():
                print(("OK   " if r["ok"] else "FAIL ") + r["msg"])
            if not service.libs:
                print("World of Warcraft: Forever wasn't found. Use --wow-dir to point at your WoW folder.")
            for flavor, lib in service.libs.items():
                st = lib.state
                print(f"{flavor}: {len(lib.desired())} emotes, {len(st['images'])} textures "
                      f"({st['format']}), {len(st.get('waiting', []))} waiting for a WoW restart")
                if lib.notice():
                    print("  " + lib.notice())
            print("If WoW was open while this ran for the first time, restart it once so it sees the new files.")
        finally:
            lock.release()
        return 0

    # A downloaded .exe copies itself to the app folder (replacing an older copy) and hands over.
    if installer.install_self():
        lock.release()
        installer.relaunch_installed()
        return 0

    instance.write_running(__version__)
    log.info("7TV Emotes Helper %s starting", __version__)
    if not cfg.get("first_run_done"):
        first_run(cfg, service, interactive=not args.background and not args.no_tray)

    if not args.no_tray:
        from .tray import run_tray
        if run_tray(service):
            return 0
    # Console mode (or no tray support).
    service.notify = lambda title, msg: print(f"[{title}] {msg}")
    try:
        service.run()
    except KeyboardInterrupt:
        service.stop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
