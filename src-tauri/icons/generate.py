"""Generate placeholder square icons for the Tauri bundle.

Solid IntelliJ-blue (matches --accent in the dark theme). Design is
intentional placeholder; real art lands later. Uses only the stdlib.
"""

from __future__ import annotations

import struct
import zlib
from pathlib import Path

ACCENT_RGBA = (0x4A, 0x88, 0xC7, 0xFF)


def _png_chunk(tag: bytes, data: bytes) -> bytes:
    body = tag + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def make_png(size: int, color: tuple[int, int, int, int] = ACCENT_RGBA) -> bytes:
    row = b"\x00" + bytes(color) * size  # filter byte + RGBA pixels
    raw = row * size
    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)  # 8-bit RGBA
    return (
        sig
        + _png_chunk(b"IHDR", ihdr)
        + _png_chunk(b"IDAT", zlib.compress(raw, 9))
        + _png_chunk(b"IEND", b"")
    )


def make_ico(png: bytes, dim: int) -> bytes:
    if dim < 256:
        width_byte = dim
        height_byte = dim
    else:
        width_byte = 0  # 0 means 256 in ICO format
        height_byte = 0
    header = struct.pack("<HHH", 0, 1, 1)  # reserved, type=icon, count=1
    entry = struct.pack(
        "<BBBBHHII",
        width_byte,
        height_byte,
        0,  # color count (0 for >=256 colors)
        0,  # reserved
        1,  # color planes
        32,  # bits per pixel
        len(png),  # bytes of image data
        22,  # offset to image data (6 header + 16 entry)
    )
    return header + entry + png


def main() -> None:
    out = Path(__file__).resolve().parent
    sizes = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 256,
    }
    for name, size in sizes.items():
        (out / name).write_bytes(make_png(size))
    ico_png = make_png(256)
    (out / "icon.ico").write_bytes(make_ico(ico_png, 256))


if __name__ == "__main__":
    main()
