#!/usr/bin/env python3
"""Generate a simple 1024x1024 app icon PNG (no third-party deps).

Draws a rounded indigo square with a white lightning-bolt-through-page motif,
written as a raw RGBA PNG via zlib/struct. Used once to seed `tauri icon`.
"""
import struct, sys, zlib

SIZE = 1024


def png_chunk(tag: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    )


def inside_rounded_rect(x, y, x0, y0, x1, y1, r):
    if x0 + r <= x <= x1 - r and y0 <= y <= y1:
        return True
    if x0 <= x <= x1 and y0 + r <= y <= y1 - r:
        return True
    for cx, cy in ((x0 + r, y0 + r), (x1 - r, y0 + r), (x0 + r, y1 - r), (x1 - r, y1 - r)):
        if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
            return True
    return False


def inside_bolt(x, y):
    # Lightning bolt in the central page area, defined by two triangles.
    pts_a = ((560, 300), (400, 560), (505, 560))
    pts_b = ((465, 724), (625, 464), (520, 464))

    def in_tri(p, a, b, c):
        def s(p1, p2, p3):
            return (p1[0] - p3[0]) * (p2[1] - p3[1]) - (p2[0] - p3[0]) * (p1[1] - p3[1])

        d1, d2, d3 = s(p, a, b), s(p, b, c), s(p, c, a)
        neg = d1 < 0 or d2 < 0 or d3 < 0
        pos = d1 > 0 or d2 > 0 or d3 > 0
        return not (neg and pos)

    return in_tri((x, y), *pts_a) or in_tri((x, y), *pts_b)


def pixel(x, y):
    bg = (0, 0, 0, 0)
    if not inside_rounded_rect(x, y, 64, 64, SIZE - 64, SIZE - 64, 180):
        return bg
    # vertical indigo gradient
    t = (y - 64) / (SIZE - 128)
    base = (int(58 + 20 * t), int(80 - 20 * t), int(190 + 30 * t), 255)
    # white page card
    if inside_rounded_rect(x, y, 300, 220, 724, 804, 48):
        if inside_bolt(x, y):
            return (250, 200, 40, 255)
        return (245, 246, 250, 255)
    return base


rows = bytearray()
for y in range(SIZE):
    rows.append(0)
    for x in range(SIZE):
        rows.extend(pixel(x, y))

ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
out = (
    b"\x89PNG\r\n\x1a\n"
    + png_chunk(b"IHDR", ihdr)
    + png_chunk(b"IDAT", zlib.compress(bytes(rows), 6))
    + png_chunk(b"IEND", b"")
)
path = sys.argv[1] if len(sys.argv) > 1 else "app-icon.png"
with open(path, "wb") as f:
    f.write(out)
print(f"wrote {path}")
