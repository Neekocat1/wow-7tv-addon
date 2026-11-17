"""Live status shown inside WoW without a reload.

WoW reads a texture file's contents the first time the file is displayed, as long as the file
existed when the game started. So the helper keeps a ring of placeholder images,
status/st0000.tga ... status/stNNNN.tga, and writes its current status (as a picture of text) into
the few files the addon will display next. The addon picks the file for the current time:

    index = floor(unix_time / PERIOD) % COUNT

and shows a new one every PERIOD seconds, so what you see in game is at most a few seconds old.
Files the clock has passed are blanked again so the folder stays small.
"""
from __future__ import annotations

import logging
import os
import sys
import time
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from . import images

log = logging.getLogger("seventv_helper")

PERIOD = 3          # seconds each status image is shown
COUNT = 4800        # ring size: 4 hours before an index repeats
AHEAD = 6           # images written ahead of the clock
SIZE = (384, 48)

COLORS = {
    "busy": (41, 182, 246),
    "ready": (64, 220, 96),
    "warn": (255, 176, 64),
    "error": (255, 96, 96),
    "idle": (150, 150, 160),
}

_fonts: dict[int, ImageFont.ImageFont] = {}


def _font(size: int, bold: bool = False):
    key = size * 2 + int(bold)
    if key not in _fonts:
        names = (["segoeuib.ttf", "arialbd.ttf"] if bold else ["segoeui.ttf", "arial.ttf"]) if sys.platform == "win32" \
            else (["DejaVuSans-Bold.ttf"] if bold else ["DejaVuSans.ttf"])
        font = None
        for n in names:
            try:
                font = ImageFont.truetype(n, size)
                break
            except OSError:
                continue
        if font is None:
            try:
                font = ImageFont.load_default(size=size)
            except TypeError:  # Pillow < 10.1
                font = ImageFont.load_default()
        _fonts[key] = font
    return _fonts[key]


def render(text: str, sub: str, kind: str = "busy") -> Image.Image:
    w, h = SIZE
    img = Image.new("RGBA", SIZE, (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((0, 0, w - 1, h - 1), radius=8, fill=(14, 14, 18, 235))
    color = COLORS.get(kind, COLORS["busy"])
    d.rounded_rectangle((0, 0, 6, h - 1), radius=3, fill=color + (255,))
    size = 19
    while size > 12 and d.textlength(text, font=_font(size, bold=True)) > w - 24:
        size -= 1
    d.text((16, 5 + (19 - size) // 2), text, font=_font(size, bold=True), fill=(255, 255, 255, 255))
    d.text((16, 28), sub, font=_font(13), fill=color + (255,))
    return img


def index_at(t: float) -> int:
    return int(t // PERIOD) % COUNT


class StatusBoard:
    def __init__(self, folder: Path):
        self.dir = folder
        self.content: dict[int, tuple] = {}   # index -> (text, sub-prefix, kind) written there
        self.current: tuple | None = None

    def path(self, i: int) -> Path:
        return self.dir / f"st{i:04d}.tga"

    def ensure_files(self) -> int:
        """Create any missing placeholder files. Returns how many were created
        (new files only become visible to WoW after it restarts)."""
        self.dir.mkdir(parents=True, exist_ok=True)
        made = 0
        for i in range(COUNT):
            p = self.path(i)
            if not p.exists():
                images.write_blank(p)
                made += 1
        return made

    def update(self, text: str, kind: str = "busy", now: float | None = None) -> None:
        """Show `text` from the next status image on. Cheap to call often."""
        now = time.time() if now is None else now
        self.current = (text, kind)
        k = index_at(now)
        for step in range(AHEAD + 1):
            i = (k + step) % COUNT
            stamp = time.strftime("%H:%M:%S", time.localtime((int(now // PERIOD) + step) * PERIOD))
            want = (text, kind)
            if self.content.get(i) == want:
                continue
            try:
                images.save_texture(render(text, f"7TV Emotes Helper  -  live {stamp}", kind), self.path(i))
                self.content[i] = want
            except OSError as e:
                log.info("status image %s: %s", i, e)
        # Blank the images the clock has passed (keeps the folder small).
        behind = [i for i in self.content if (k - i) % COUNT > 2 and (k - i) % COUNT < COUNT - AHEAD - 1]
        for i in behind:
            try:
                images.write_blank(self.path(i))
            except OSError:
                pass
            self.content.pop(i, None)

    def info(self) -> dict:
        return {"dir": "status", "count": COUNT, "period": PERIOD}
