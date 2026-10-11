"""Render Runo's app icon from runo.svg for every platform.

Run from this folder: python3 build.py (needs cairosvg and Pillow).

- macOS: runo.icns, from the inset artwork (Apple's grid leaves a margin round the tile).
- Windows: runo.ico, full-bleed so small sizes stay legible.
- Linux: png/runo-<size>.png for the hicolor theme.
"""

import io

import cairosvg
from PIL import Image

SVG = open("runo.svg").read()
# The tile spans 100..924 of the 1024 canvas; this crops to it for full-bleed sizes.
FLAT = SVG.replace('viewBox="0 0 1024 1024"', 'viewBox="96 96 832 832"')


def render(svg, size):
    png = cairosvg.svg2png(bytestring=svg.encode(), output_width=size, output_height=size)
    return Image.open(io.BytesIO(png)).convert("RGBA")


# Linux: inset art from 64px up, full-bleed below that.
for size in (16, 24, 32, 48, 64, 128, 256, 512, 1024):
    render(SVG if size >= 64 else FLAT, size).save(f"png/runo-{size}.png")

# Windows.
render(FLAT, 256).save(
    "runo.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)]
)

# macOS.
render(SVG, 1024).save("runo.icns")
print("icons written")
