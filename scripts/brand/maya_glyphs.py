#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Traces the twenty Maya day signs into vector paths.

Source: assets/maya/day-signs/*.png, the Wikimedia Commons day-sign drawings
(CJLL Wright, CC BY 2.5; assets/maya/ATTRIBUTION.md). Each sign is traced with
potrace into two filled paths on the 24-unit icon grid:

  back  the sign's silhouette (cartouche and pedestal, no holes), filled with
        the seal colour;
  ink   the drawing's lines, holes wound against their outlines (nonzero).

Writes assets/maya/day-signs.txt (read by sex-icons, `sex_icons::maya`) and
assets/maya/svg/NN-name.svg. `--check` fails if either is out of date.

Needs Pillow, numpy and potracer (`pip install potracer`).
"""
import pathlib, sys
import numpy as np
from PIL import Image, ImageDraw
import potrace

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "assets/maya/day-signs"
OUT_TXT = ROOT / "assets/maya/day-signs.txt"
OUT_SVG = ROOT / "assets/maya/svg"
# Yucatec names (Commons file names) and Dreamspell seals, in day order.
NAMES = ["Imix", "Ik", "Akbal", "Kan", "Chicchan", "Cimi", "Manik", "Lamat", "Muluc", "Oc",
         "Chuen", "Eb", "Ben", "Ix", "Men", "Cib", "Caban", "Etznab", "Cauac", "Ahau"]
SEALS = ["dragon", "wind", "night", "seed", "serpent", "worldbridger", "hand", "star", "moon", "dog",
         "monkey", "human", "skywalker", "wizard", "eagle", "warrior", "earth", "mirror", "storm", "sun"]


def source(i):
    hits = sorted(SRC.glob(f"MAYA-g-log-cal-D{i + 1:02d}-*.png"))
    if len(hits) != 1:
        sys.exit(f"day sign {i + 1}: expected one PNG in {SRC}, found {len(hits)}")
    return hits[0]


def fmt(v):
    s = f"{v:.2f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def trace(ink, place):
    """Path data for a bool bitmap (True = filled)."""
    # potracer fills the dark pixels: values under blacklevel.
    path = potrace.Bitmap(~ink).trace(turdsize=3, turnpolicy=potrace.POTRACE_TURNPOLICY_MINORITY,
                                     alphamax=1.0, opticurve=True, opttolerance=0.4)
    out = []
    for curve in path:
        p = lambda pt: f"{fmt(place[0] + pt[0] * place[2])} {fmt(place[1] + pt[1] * place[2])}"
        out.append("M" + p((curve.start_point.x, curve.start_point.y)))
        for seg in curve.segments:
            e = (seg.end_point.x, seg.end_point.y)
            if seg.is_corner:
                out.append("L" + p((seg.c.x, seg.c.y)) + " L" + p(e))
            else:
                out.append("C" + p((seg.c1.x, seg.c1.y)) + " " + p((seg.c2.x, seg.c2.y)) + " " + p(e))
        out.append("Z")
    return " ".join(out)


def sign(i):
    im = Image.open(source(i)).convert("L")
    w, h = im.size
    ink = np.array(im) < 128
    # Silhouette: everything the white background cannot reach from the border.
    flood = Image.fromarray(np.where(ink, 0, 255).astype(np.uint8))
    flood = flood.crop((0, 0, w, h)).copy()
    padded = Image.new("L", (w + 2, h + 2), 255)
    padded.paste(flood, (1, 1))
    ImageDraw.floodfill(padded, (0, 0), 128)
    back = np.array(padded)[1:-1, 1:-1] != 128
    # Fit the image into 1..23 of the 24-unit grid, centred.
    k = 22.0 / max(w, h)
    place = ((24 - w * k) / 2, (24 - h * k) / 2, k)
    return trace(back, place), trace(ink, place)


def build():
    lines = ["# Maya day signs, traced by scripts/brand/maya_glyphs.py from assets/maya/day-signs",
             "# (CJLL Wright, Wikimedia Commons, CC BY 2.5; see ATTRIBUTION.md). Do not edit by hand.",
             "# Each line: NN SEAL LAYER PATH, LAYER back (silhouette) or ink (nonzero fill), 24-unit grid."]
    svgs = {}
    for i in range(20):
        back, ink = sign(i)
        lines.append(f"{i + 1:02d} {SEALS[i]} back {back}")
        lines.append(f"{i + 1:02d} {SEALS[i]} ink {ink}")
        svgs[f"{i + 1:02d}-{NAMES[i].lower()}-{SEALS[i]}.svg"] = (
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="240" height="240">\n'
            f'<title>{NAMES[i]} ({SEALS[i].capitalize()}) - after CJLL Wright, CC BY 2.5</title>\n'
            f'<path fill="#f2efe9" d="{back}"/>\n'
            f'<path fill="#1c1a1b" fill-rule="nonzero" d="{ink}"/>\n</svg>\n')
    return "\n".join(lines) + "\n", svgs


def main():
    text, svgs = build()
    if "--check" in sys.argv:
        stale = [] if OUT_TXT.exists() and OUT_TXT.read_text() == text else [OUT_TXT]
        stale += [OUT_SVG / n for n, s in svgs.items() if not (OUT_SVG / n).exists() or (OUT_SVG / n).read_text() != s]
        if stale:
            sys.exit("out of date: " + ", ".join(str(p.relative_to(ROOT)) for p in stale))
        return
    OUT_TXT.write_text(text)
    OUT_SVG.mkdir(exist_ok=True)
    for n, s in svgs.items():
        (OUT_SVG / n).write_text(s)
    print(f"{OUT_TXT.relative_to(ROOT)}: {len(text)} bytes; {len(svgs)} SVGs")


if __name__ == "__main__":
    main()
