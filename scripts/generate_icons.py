"""Generate matching raster/Windows/macOS-template assets. Requires Pillow.

The SVG is the editable vector master; keep its geometry/colors in sync below.
"""
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1] / "assets"
SCALE = 4
image = Image.new("RGBA", (256 * SCALE, 256 * SCALE))
draw = ImageDraw.Draw(image)
for box, radius, color in [
    ((8, 8, 248, 248), 56, "#102333"),
    ((40, 40, 120, 216), 12, "#64e4bd"),
    ((136, 40, 216, 120), 12, "#e6f4f5"),
    ((136, 136, 216, 216), 12, "#398d99"),
]:
    draw.rounded_rectangle(tuple(v * SCALE for v in box), radius * SCALE, fill=color)
image = image.resize((256, 256), Image.Resampling.LANCZOS)
image.save(ROOT / "stile.png")
image.save(ROOT / "stile.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
image.save(ROOT / "stile.icns")
tray = image.resize((32, 32), Image.Resampling.LANCZOS)
tray.save(ROOT / "tray.png")
(ROOT / "tray.rgba").write_bytes(tray.tobytes())
# A monochrome template lets the macOS menu bar choose light/dark colors.
template = Image.new("RGBA", (128, 128))
draw = ImageDraw.Draw(template)
for box in [(12, 16, 54, 112), (66, 16, 108, 58), (66, 70, 108, 112)]:
    draw.rounded_rectangle(box, 6, fill="black")
template = template.resize((32, 32), Image.Resampling.LANCZOS)
template.save(ROOT / "tray-template.png")
(ROOT / "tray-template.rgba").write_bytes(template.tobytes())
