"""System tray icon (Windows/macOS) via pystray."""
from __future__ import annotations

import logging
import sys

from PIL import Image, ImageDraw, ImageFont

from . import installer
from .paths import log_file
from .service import Service

log = logging.getLogger("seventv_helper")


def make_icon(size: int = 64) -> Image.Image:
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((2, 2, size - 3, size - 3), radius=size // 5, fill=(41, 182, 246, 255))
    try:
        font = ImageFont.truetype("arialbd.ttf" if sys.platform == "win32" else "DejaVuSans-Bold.ttf", int(size * 0.62))
    except OSError:
        font = ImageFont.load_default()
    d.text((size / 2, size / 2), "7", fill=(255, 255, 255, 255), font=font, anchor="mm")
    return img


def run_tray(service: Service) -> bool:
    """Run the tray icon on the main thread until Quit. Returns False if no tray is available."""
    try:
        import pystray
    except ImportError:
        log.info("pystray not installed; running without a tray icon")
        return False

    def first_folder():
        libs = list(service.libs.values())
        return libs[0].dir if libs else None

    def on_open_folder(icon, item):
        p = first_folder()
        if p:
            installer.open_path(p)

    def on_toggle_autostart(icon, item):
        installer.set_autostart(not installer.autostart_enabled())

    def on_quit(icon, item):
        service.stop()
        icon.stop()

    autostart_label = "Start with Windows" if sys.platform == "win32" else "Start at login"
    menu = pystray.Menu(
        pystray.MenuItem(lambda item: service.status, None, enabled=False),
        pystray.Menu.SEPARATOR,
        pystray.MenuItem("Check 7TV for new emotes now", lambda icon, item: service.request_refresh()),
        pystray.MenuItem("Open emote folder", on_open_folder),
        pystray.MenuItem("Open log", lambda icon, item: installer.open_path(log_file())),
        pystray.MenuItem(autostart_label, on_toggle_autostart, checked=lambda item: installer.autostart_enabled()),
        pystray.Menu.SEPARATOR,
        pystray.MenuItem("Quit", on_quit),
    )
    icon = pystray.Icon("SevenTVEmotesHelper", make_icon(), "7TV Emotes Helper", menu)

    def notify(title: str, msg: str):
        try:
            icon.notify(msg, title)
        except Exception:  # notifications aren't supported everywhere
            pass

    service.notify = notify
    def on_status():
        _safe(icon.update_menu)
        try:
            icon.title = ("7TV Emotes Helper - " + service.status)[:120]
        except Exception:
            pass

    service.on_status = on_status
    service.on_quit = lambda: _safe(icon.stop)

    def setup(icon):
        icon.visible = True
        service.start()

    icon.run(setup=setup)
    return True


def _safe(fn):
    try:
        fn()
    except Exception:
        pass
