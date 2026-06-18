#!/usr/bin/env python3
"""Generate the Castellyn app icon master (1024x1024 PNG) with Pillow.

Concept: a central hub node orbited by agent nodes — a control center for many AI agents,
on the brand blue gradient (#3b82f6 -> #2563eb), rounded-square.

Usage:
    python tools/make-icon.py                 # writes <temp>/agenthub_icon.png
    npm run tauri -- icon <that path>         # regenerates src-tauri/icons/* (ico/icns/png/Square*)

Requires: Pillow (pip install pillow). No SVG toolchain needed.
"""
import math
import os
import sys
import tempfile

try:
    from PIL import Image, ImageDraw, ImageFilter
except ImportError:
    sys.exit("Pillow is required: pip install pillow")

SS = 3                      # supersample factor for crisp anti-aliasing
S = 1024 * SS
cx = cy = S // 2

C1 = (96, 167, 255)         # gradient top-left  (light brand blue)
C2 = (29, 78, 216)          # gradient bottom-right (deep brand blue)


def lerp(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


def build() -> Image.Image:
    # diagonal gradient background
    grad = Image.new("RGB", (S, S))
    gpx = grad.load()
    for y in range(S):
        for x in range(0, S, 4):
            c = lerp(C1, C2, (x + y) / (2 * S))
            for dx in range(4):
                if x + dx < S:
                    gpx[x + dx, y] = c
    bg = grad.convert("RGBA")

    # soft top highlight
    glow = Image.new("L", (S, S), 0)
    gr = int(S * 0.42)
    ImageDraw.Draw(glow).ellipse(
        [cx - gr, int(S * 0.06) - gr, cx + gr, int(S * 0.06) + gr], fill=70)
    glow = glow.filter(ImageFilter.GaussianBlur(S * 0.06))
    white = Image.new("RGBA", (S, S), (255, 255, 255, 0))
    white.putalpha(glow)
    bg = Image.alpha_composite(bg, white)

    # hub-and-nodes geometry
    orbit_r = int(S * 0.293)
    hub_r = int(S * 0.092)
    node_r = int(S * 0.050)
    line_w = int(S * 0.021)
    nodes = [(cx + orbit_r * math.cos(math.radians(-90 + i * 60)),
              cy + orbit_r * math.sin(math.radians(-90 + i * 60))) for i in range(6)]

    def draw_mark(d, line_col, node_col, hub_col, orbit_col=None):
        if orbit_col is not None:
            d.ellipse([cx - orbit_r, cy - orbit_r, cx + orbit_r, cy + orbit_r],
                      outline=orbit_col, width=int(S * 0.010))
        for nx, ny in nodes:
            d.line([cx, cy, nx, ny], fill=line_col, width=line_w)
        for nx, ny in nodes:
            d.ellipse([nx - node_r, ny - node_r, nx + node_r, ny + node_r], fill=node_col)
        d.ellipse([cx - hub_r, cy - hub_r, cx + hub_r, cy + hub_r], fill=hub_col)

    # drop shadow
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    draw_mark(ImageDraw.Draw(shadow), (10, 30, 70, 160), (10, 30, 70, 170), (10, 30, 70, 180))
    shadow = shadow.filter(ImageFilter.GaussianBlur(S * 0.012))
    bg.alpha_composite(shadow, (0, int(S * 0.012)))

    # white mark
    mark = Image.new("RGBA", (S, S), (255, 255, 255, 0))
    draw_mark(ImageDraw.Draw(mark), (255, 255, 255, 220), (255, 255, 255, 255),
              (255, 255, 255, 255), orbit_col=(255, 255, 255, 70))
    bg.alpha_composite(mark)

    # rounded-square clip
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, S - 1, S - 1], radius=int(S * 0.225), fill=255)
    bg.putalpha(mask)

    return bg.resize((1024, 1024), Image.LANCZOS)


if __name__ == "__main__":
    dst = os.path.join(tempfile.gettempdir(), "agenthub_icon.png")
    build().save(dst)
    print("wrote", dst)
    print("next: npm run tauri -- icon", f'"{dst}"')
