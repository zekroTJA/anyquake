#!/usr/bin/env python3
"""Regenerates AnyQuake.icns from ../anyquake.svg.

Needs ImageMagick (`magick`). Runs on any OS; macOS's iconutil isn't needed
because an .icns file is just a list of PNG images with a small header.
"""

import pathlib
import struct
import subprocess

HERE = pathlib.Path(__file__).resolve().parent
SVG = HERE.parent / "anyquake.svg"

# (icns type, pixel size)
ENTRIES = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"icp6", 64),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic09", 512),
    (b"ic10", 1024),  # 512@2x
    (b"ic11", 32),  # 16@2x
    (b"ic12", 64),  # 32@2x
    (b"ic13", 256),  # 128@2x
    (b"ic14", 512),  # 256@2x
]


def png(size: int) -> bytes:
    # macOS icons leave a transparent margin around the shape (824 of 1024
    # pixels in Apple's icon grid).
    inner = round(size * 824 / 1024)
    return subprocess.run(
        ["magick", "-background", "none", "-density", str(72 * inner // 256 + 1),
         str(SVG), "-resize", f"{inner}x{inner}", "-gravity", "center",
         "-extent", f"{size}x{size}", "png:-"],
        check=True,
        capture_output=True,
    ).stdout


body = b""
for kind, size in ENTRIES:
    data = png(size)
    body += kind + struct.pack(">I", 8 + len(data)) + data

(HERE / "AnyQuake.icns").write_bytes(b"icns" + struct.pack(">I", 8 + len(body)) + body)
