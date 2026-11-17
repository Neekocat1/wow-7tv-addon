"""Turn 7TV images into textures WoW can load.

WoW wants power-of-two textures (TGA or BLP). Static emotes are padded onto a
power-of-two canvas; animated emotes become a sprite sheet the addon steps through.
"""
from __future__ import annotations

import math
import os
import struct
from pathlib import Path

from PIL import Image

try:  # optional fallback encoder; Pillow >= 11.2 compresses DXT5 itself
    import numpy as np
except ImportError:  # pragma: no cover
    np = None

from .seventv import Emote, http

FORMAT_PREFERENCE = ["WEBP", "GIF", "PNG", "AVIF"]


def pick_file(host: dict, size: int) -> dict:
    files = host["files"]
    for fmt in FORMAT_PREFERENCE:
        cands = sorted((f for f in files if f.get("format") == fmt), key=lambda f: f.get("height", 0))
        if cands:
            for f in cands:
                if f.get("height", 0) >= size:
                    return f
            return cands[-1]
    return files[-1]


def download(emote: Emote, size: int, cache: Path) -> Path:
    f = pick_file(emote.host, size)
    cache.mkdir(parents=True, exist_ok=True)
    path = cache / f"{emote.id}_{f['name']}"
    if not path.exists() or path.stat().st_size == 0:
        base = emote.host["url"]
        url = ("https:" + base if base.startswith("//") else base) + "/" + f["name"]
        data = http(url, binary=True)
        tmp = path.with_suffix(path.suffix + ".part")
        tmp.write_bytes(data)
        os.replace(tmp, path)
    return path


def load_frames(path: Path) -> tuple[list[Image.Image], list[int]]:
    im = Image.open(path)
    n = getattr(im, "n_frames", 1)
    frames, durations = [], []
    for i in range(n):
        im.seek(i)
        frames.append(im.convert("RGBA").copy())
        d = im.info.get("duration") or 100
        durations.append(int(d) if d >= 20 else 100)  # browsers treat tiny delays as 100ms
    return frames, durations


def pow2(n: int) -> int:
    return 1 << max(0, math.ceil(math.log2(max(1, n))))


def fit(w: int, h: int, size: int, max_ratio: float) -> tuple[int, int]:
    """Scale so height == size; very wide emotes are capped at size*max_ratio wide."""
    W, H = max(1, round(w * size / h)), size
    max_w = int(size * max_ratio)
    if W > max_w:
        W, H = max_w, max(1, round(h * max_w / w))
    return W, H


def decimate(frames, durations, max_frames):
    n = len(frames)
    if n <= max_frames:
        return frames, durations
    step = n / max_frames
    out_f, out_d = [], []
    for i in range(max_frames):
        a, b = int(i * step), int((i + 1) * step)
        out_f.append(frames[a])
        out_d.append(sum(durations[a:max(b, a + 1)]))
    return out_f, out_d


def sheet_layout(n: int, w: int, h: int, max_sheet: int):
    best = None
    for cols in range(1, n + 1):
        if cols * w > max_sheet:
            break
        rows = math.ceil(n / cols)
        sw, sh = pow2(cols * w), pow2(rows * h)
        if sw > max_sheet or sh > max_sheet:
            continue
        key = (sw * sh, abs(math.log2(sw) - math.log2(sh)))
        if best is None or key < best[0]:
            best = (key, cols, sw, sh)
    return None if best is None else best[1:]


# --------------------------------------------------------------------------- BLP (DXT5)
#
# BLP2 is WoW's own texture format. With DXT5 compression a texture takes 1 byte per pixel
# on disk and in video memory, versus 4 for an uncompressed TGA. No mipmaps: emote sprite
# sheets would bleed between frames, and UI textures don't need them.

BLP_HEADER_SIZE = 1172  # magic, type, 4 flag bytes, w, h, 16 mip offsets, 16 mip sizes, 256-entry palette


_PIL_DXT5: bool | None = None


def _pillow_dxt5(img: Image.Image) -> bytes | None:
    """DXT5 blocks via Pillow's own encoder (saves a DDS in memory and strips its header)."""
    global _PIL_DXT5
    if _PIL_DXT5 is False:
        return None
    import io
    try:
        buf = io.BytesIO()
        img.save(buf, format="DDS", pixel_format="DXT5")
        raw = buf.getvalue()
        header = 128 + (20 if raw[84:88] == b"DX10" else 0)
        data = raw[header:]
        if len(data) != (img.width // 4) * (img.height // 4) * 16:
            raise ValueError("unexpected DDS layout")
        _PIL_DXT5 = True
        return data
    except Exception:
        _PIL_DXT5 = False
        return None


def compression_available() -> bool:
    if np is not None:
        return True
    return _pillow_dxt5(Image.new("RGBA", (4, 4))) is not None


def _to_565(rgb):
    return ((rgb[..., 0].astype(np.uint16) >> 3) << 11) | ((rgb[..., 1].astype(np.uint16) >> 2) << 5) | (rgb[..., 2].astype(np.uint16) >> 3)


def _from_565(c):
    r = (c >> 11) & 31
    g = (c >> 5) & 63
    b = c & 31
    return np.stack([(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)], axis=-1).astype(np.int32)


def encode_dxt5(img: Image.Image) -> bytes:
    """Compress an RGBA image (dimensions multiples of 4) to DXT5/BC3 blocks."""
    data = _pillow_dxt5(img.convert("RGBA"))
    if data is not None:
        return data
    if np is None:
        raise RuntimeError("compressed textures need Pillow 11.2+ or numpy")
    a = np.asarray(img.convert("RGBA"), dtype=np.uint8)
    h, w, _ = a.shape
    blocks = a.reshape(h // 4, 4, w // 4, 4, 4).transpose(0, 2, 1, 3, 4).reshape(-1, 16, 4).astype(np.int32)
    nb = blocks.shape[0]

    # Alpha: two endpoints, 8-value ramp, 3-bit index per pixel.
    alpha = blocks[..., 3]
    a0 = alpha.max(axis=1)
    a1 = alpha.min(axis=1)
    k = np.arange(8)
    ramp = np.empty((nb, 8), dtype=np.int32)
    ramp[:, 0], ramp[:, 1] = a0, a1
    for i in range(1, 7):
        ramp[:, i + 1] = ((7 - i) * a0 + i * a1) // 7
    aidx = np.abs(alpha[:, :, None] - ramp[:, None, :]).argmin(axis=2).astype(np.uint64)
    abits = np.zeros(nb, dtype=np.uint64)
    for p in range(16):
        abits |= aidx[:, p] << np.uint64(3 * p)

    # Color: endpoints along each block's principal color axis (visible pixels only),
    # 4-color ramp, 2-bit index per pixel.
    rgb = blocks[..., :3].astype(np.float64)
    wv = (alpha > 0).astype(np.float64)
    none = wv.sum(axis=1) == 0
    wv[none] = 1.0
    wsum = wv.sum(axis=1, keepdims=True)
    mean = (rgb * wv[..., None]).sum(axis=1) / wsum
    centered = (rgb - mean[:, None, :]) * np.sqrt(wv)[..., None]
    cov = np.einsum("bpi,bpj->bij", centered, centered)
    axis = np.ones((nb, 3)) / np.sqrt(3)
    for _ in range(8):  # power iteration
        axis = np.einsum("bij,bj->bi", cov, axis)
        norm = np.linalg.norm(axis, axis=1, keepdims=True)
        axis = np.where(norm > 1e-9, axis / np.maximum(norm, 1e-9), np.ones((nb, 3)) / np.sqrt(3))
    proj = ((rgb - mean[:, None, :]) * axis[:, None, :]).sum(axis=2)
    proj_v = np.where(wv > 0, proj, np.nan)
    tmin, tmax = np.nanmin(proj_v, axis=1), np.nanmax(proj_v, axis=1)
    cmin = np.clip(np.rint(mean + axis * tmin[:, None]), 0, 255).astype(np.int32)
    cmax = np.clip(np.rint(mean + axis * tmax[:, None]), 0, 255).astype(np.int32)
    rgb = blocks[..., :3]
    c0, c1 = _to_565(cmax), _to_565(cmin)
    swap = c0 < c1
    c0, c1 = np.where(swap, c1, c0), np.where(swap, c0, c1)
    e0, e1 = _from_565(c0), _from_565(c1)
    pal = np.stack([e0, e1, (2 * e0 + e1) // 3, (e0 + 2 * e1) // 3], axis=1)
    d = ((rgb[:, :, None, :] - pal[:, None, :, :]) ** 2).sum(axis=3)
    cidx = d.argmin(axis=2).astype(np.uint32)
    cidx[c0 == c1] = 0
    cbits = np.zeros(nb, dtype=np.uint32)
    for p in range(16):
        cbits |= cidx[:, p] << np.uint32(2 * p)

    out = np.zeros((nb, 16), dtype=np.uint8)
    out[:, 0], out[:, 1] = a0, a1
    for i in range(6):
        out[:, 2 + i] = ((abits >> np.uint64(8 * i)) & np.uint64(0xFF)).astype(np.uint8)
    out[:, 8], out[:, 9] = c0 & 0xFF, c0 >> 8
    out[:, 10], out[:, 11] = c1 & 0xFF, c1 >> 8
    for i in range(4):
        out[:, 12 + i] = (cbits >> np.uint32(8 * i)) & 0xFF
    return out.tobytes()


def _mip_chain(img: Image.Image) -> list[Image.Image]:
    """Full mipmap chain down to 1x1. Box-filtering keeps sprite-sheet cells separate for the
    first few levels (cell sizes are multiples of 2^n), which are the only ones chat ever uses."""
    levels = [img]
    while (levels[-1].width > 1 or levels[-1].height > 1) and len(levels) < 16:
        w, h = max(1, levels[-1].width // 2), max(1, levels[-1].height // 2)
        levels.append(levels[-1].resize((w, h), Image.BOX))
    return levels


def _encode_level(img: Image.Image) -> bytes:
    if img.width % 4 or img.height % 4:  # DXT works on 4x4 blocks; pad tiny levels
        canvas = Image.new("RGBA", (max(4, (img.width + 3) // 4 * 4), max(4, (img.height + 3) // 4 * 4)), (0, 0, 0, 0))
        canvas.paste(img, (0, 0))
        img = canvas
    return encode_dxt5(img)


def blp_bytes(img: Image.Image, mips: bool = True) -> bytes:
    """BLP2, DXT5 (color encoding 2, alpha depth 8, pixel format 7), with a full mip chain."""
    img = img.convert("RGBA")
    w, h = img.size
    levels = _mip_chain(img) if mips else [img]
    datas = [_encode_level(l) for l in levels]
    offsets, sizes, pos = [], [], BLP_HEADER_SIZE
    for d in datas:
        offsets.append(pos)
        sizes.append(len(d))
        pos += len(d)
    offsets += [0] * (16 - len(offsets))
    sizes += [0] * (16 - len(sizes))
    header = b"BLP2" + struct.pack("<I4BII", 1, 2, 8, 7, 1 if mips else 0, w, h)
    header += struct.pack("<16I", *offsets) + struct.pack("<16I", *sizes) + bytes(1024)
    assert len(header) == BLP_HEADER_SIZE
    return header + b"".join(datas)


def blp_raw_bytes(img: Image.Image) -> bytes:
    """BLP2 with uncompressed BGRA pixels (color encoding 3). Only used for the texture test."""
    img = img.convert("RGBA")
    w, h = img.size
    r, g, b, a = img.split()
    data = Image.merge("RGBA", (b, g, r, a)).tobytes()
    header = b"BLP2" + struct.pack("<I4BII", 1, 3, 8, 8, 0, w, h)
    header += struct.pack("<16I", BLP_HEADER_SIZE, *[0] * 15) + struct.pack("<16I", len(data), *[0] * 15) + bytes(1024)
    return header + data


def blp_size(tw: int, th: int) -> int:
    total, w, h = BLP_HEADER_SIZE, tw, th
    for _ in range(16):
        total += max(1, (w + 3) // 4) * max(1, (h + 3) // 4) * 16
        if w == 1 and h == 1:
            break
        w, h = max(1, w // 2), max(1, h // 2)
    return total


def tga_size(tw: int, th: int) -> int:
    return 18 + tw * th * 4 + 26


def save_texture(img: Image.Image, out: Path) -> None:
    """Save as .tga or .blp depending on the extension, atomically (WoW never sees half a file)."""
    tmp = out.with_name(out.name + ".part")
    if out.suffix.lower() == ".blp":
        if img.width < 4 or img.height < 4:
            canvas = Image.new("RGBA", (max(4, img.width), max(4, img.height)), (0, 0, 0, 0))
            canvas.paste(img, (0, 0))
            img = canvas
        tmp.write_bytes(blp_bytes(img))
    else:
        img.save(tmp, format="TGA")
    os.replace(tmp, out)


save_tga = save_texture  # backwards-compatible name


def render(src: Path, out: Path, *, size: int = 64, anim_size: int = 48, max_ratio: float = 4.0,
           max_frames: int = 48, max_sheet: int = 1024, animate: bool = True) -> dict:
    """Write `src` as a WoW texture at `out`. Returns the layout the addon needs."""
    frames, durations = load_frames(src)
    if not animate:
        frames, durations = frames[:1], durations[:1]
    W, H = fit(frames[0].width, frames[0].height, size if len(frames) == 1 else anim_size, max_ratio)

    if len(frames) == 1:
        tw, th = pow2(W), pow2(H)
        canvas = Image.new("RGBA", (tw, th), (0, 0, 0, 0))
        canvas.paste(frames[0].resize((W, H), Image.LANCZOS), (0, 0))
        save_tga(canvas, out)
        return {"w": W, "h": H, "tw": tw, "th": th, "frames": 1, "cols": 1, "ms": 0}

    frames, durations = decimate(frames, durations, max_frames)
    layout = sheet_layout(len(frames), W, H, max_sheet)
    while layout is None:  # too many frames for the sheet: halve until it fits
        frames, durations = decimate(frames, durations, max(1, len(frames) // 2))
        layout = sheet_layout(len(frames), W, H, max_sheet)
    cols, tw, th = layout
    canvas = Image.new("RGBA", (tw, th), (0, 0, 0, 0))
    for i, fr in enumerate(frames):
        canvas.paste(fr.resize((W, H), Image.LANCZOS), ((i % cols) * W, (i // cols) * H))
    save_tga(canvas, out)
    ms = max(10, round(sum(durations) / len(frames)))
    return {"w": W, "h": H, "tw": tw, "th": th, "frames": len(frames), "cols": cols, "ms": ms}


_BLANK_BLP: bytes | None = None


def write_blank(out: Path) -> None:
    """Tiny transparent placeholder used to reserve a texture slot (.tga or .blp)."""
    global _BLANK_BLP
    if out.suffix.lower() == ".blp":
        if _BLANK_BLP is None and compression_available():
            _BLANK_BLP = blp_bytes(Image.new("RGBA", (8, 8), (0, 0, 0, 0)))
        if _BLANK_BLP is None:
            # Hand-built so it works without numpy: one all-transparent DXT5 block per 4x4.
            block = bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
            data = block * 4
            header = b"BLP2" + struct.pack("<I4BII", 1, 2, 8, 7, 0, 8, 8)
            header += struct.pack("<16I", BLP_HEADER_SIZE, *[0] * 15) + struct.pack("<16I", len(data), *[0] * 15) + bytes(1024)
            _BLANK_BLP = header + data
        out.write_bytes(_BLANK_BLP)
    else:
        Image.new("RGBA", (8, 8), (0, 0, 0, 0)).save(out, format="TGA")
