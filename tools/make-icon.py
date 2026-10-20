#!/usr/bin/env python3
"""Draws the SIM001 app icon (a green plane on the console's dark screen) and writes
domains/teleoperation/launcher/icon/sim001.icns (macOS only: needs iconutil)."""
import subprocess, sys, tempfile, pathlib
from PIL import Image, ImageDraw

root = pathlib.Path(__file__).resolve().parent.parent
out = root / "domains/teleoperation/launcher/icon"
S = 1024
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
d.rounded_rectangle((100, 100, S - 100, S - 100), radius=185, fill=(18, 20, 19, 255), outline=(60, 66, 62, 255), width=10)
d.rounded_rectangle((150, 150, S - 150, S - 150), radius=140, fill=(5, 9, 7, 255))
green = (70, 230, 120, 255)
for r in (150, 290):  # radar rings
    d.ellipse((S / 2 - r, S / 2 - r, S / 2 + r, S / 2 + r), outline=(30, 90, 55, 255), width=6)
# plane, nose up
c = S / 2
d.polygon([(c, 270), (c + 34, 440), (c + 250, 560), (c + 250, 620), (c + 34, 570), (c + 24, 740), (c + 110, 790),
           (c + 110, 830), (c, 800), (c - 110, 830), (c - 110, 790), (c - 24, 740), (c - 34, 570), (c - 250, 620),
           (c - 250, 560), (c - 34, 440)], fill=green)
out.mkdir(parents=True, exist_ok=True)
img.save(out / "sim001.png")
if sys.platform == "darwin":
    with tempfile.TemporaryDirectory() as t:
        iset = pathlib.Path(t) / "sim001.iconset"
        iset.mkdir()
        for n in (16, 32, 128, 256, 512):
            img.resize((n, n), Image.LANCZOS).save(iset / f"icon_{n}x{n}.png")
            img.resize((n * 2, n * 2), Image.LANCZOS).save(iset / f"icon_{n}x{n}@2x.png")
        subprocess.run(["iconutil", "-c", "icns", str(iset), "-o", str(out / "sim001.icns")], check=True)
