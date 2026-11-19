#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""The SexOS icon set, written out from its one source, assets/icons/icons.txt.

Writes
  assets/icons/svg/{line,app,file}/NAME.svg   vectors (line icons use currentColor)
  assets/icons/png/SIZE/NAME.png              app and file icons at 16..256 px
  assets/icons/preview.png                    every icon on one sheet
With --check it only verifies that every icon parses (CI-friendly, no cairosvg).

Native code never reads these files: crates/sex-icons draws the same shapes from
icons.txt at run time, with the same colours (its `Palette::BRAND`), so the raster
and vector forms cannot drift. The colours below must match that palette.
"""
import argparse, pathlib, re

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "assets/icons/icons.txt"
OUT = ROOT / "assets/icons"
SIZES = (16, 24, 32, 48, 64, 128, 256)

# Must match crates/sex-icons/src/lib.rs Palette::BRAND.
BADGE_TOP, BADGE_BOTTOM = "#3d3637", "#221e1f"
RIM = "#ffc4a6"
GLYPH = [(0.0, "#b07a66"), (0.5, "#e2a084"), (1.0, "#ffcaae")]
SHEET_TOP, SHEET_BOTTOM = "#423b3c", "#2e2a2b"
FOLDER = [(0.0, "#9c6e5e"), (0.55, "#e2a084"), (1.0, "#f5aa8d")]
STROKE = 2.0
APP_GLYPH_SCALE = 0.64   # the glyph's share of the badge


def load():
    icons = []
    for n, line in enumerate(SRC.read_text().splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        kind, name, data = line.split(" ", 2)
        assert kind in ("line", "app", "file"), "%s:%d unknown kind %s" % (SRC, n, kind)
        assert re.fullmatch(r"[a-z0-9-]+", name), name
        icons.append((kind, name, data))
    names = [(k, n) for k, n, _ in icons]
    assert len(set(names)) == len(names), "duplicate icon"
    return icons


def parts(data):
    """Path data to SVG elements: ('stroke'|'fill', d) and ('circle', cx, cy, r, filled)."""
    toks = re.findall(r"[A-Za-z]|-?\d*\.?\d+", data)
    out, cur, fill_next, current_fill = [], [], False, False
    i, cmd = 0, None

    def flush():
        nonlocal cur
        if cur:
            out.append(("fill" if current_fill else "stroke", " ".join(cur)))
        cur = []

    while i < len(toks):
        t = toks[i]
        if t.isalpha():
            cmd = t
            i += 1
            if cmd == "F":
                fill_next = True
                continue
            if cmd in "Zz":
                cur.append("Z")
                continue
            if cmd in "Mm":
                flush()
                current_fill, fill_next = fill_next, False
        arity = {"M": 2, "L": 2, "H": 1, "V": 1, "Q": 4, "C": 6, "O": 3, "D": 3}[cmd.upper()]
        nums = toks[i:i + arity]
        assert len(nums) == arity and not any(x.isalpha() for x in nums), "bad path data: " + data
        i += arity
        if cmd in "OD":
            out.append(("circle",) + tuple(float(x) for x in nums) + (cmd == "D",))
        else:
            cur.append(cmd + " " + " ".join(nums))
            if cmd in "Mm":
                cmd = "l" if cmd == "m" else "L"
    flush()
    return out


def grad(gid, stops, vertical=False):
    xy = 'x1="0" y1="0" x2="0" y2="1"' if vertical else 'x1="0" y1="0" x2="1" y2="0"'
    s = "".join('<stop offset="%g" stop-color="%s"/>' % st for st in stops)
    return '<linearGradient id="%s" %s>%s</linearGradient>' % (gid, xy, s)


def elements(data, ink, width=STROKE):
    els = []
    for p in parts(data):
        if p[0] == "circle":
            _, cx, cy, r, filled = p
            paint = 'fill="%s"' % ink if filled else 'fill="none" stroke="%s" stroke-width="%g"' % (ink, width)
            els.append('<circle cx="%g" cy="%g" r="%g" %s/>' % (cx, cy, r, paint))
        elif p[0] == "fill":
            els.append('<path d="%s" fill="%s"/>' % (p[1], ink))
        else:
            els.append('<path d="%s" fill="none" stroke="%s" stroke-width="%g" '
                       'stroke-linecap="round" stroke-linejoin="round"/>' % (p[1], ink, width))
    return "".join(els)


def svg(kind, name, data):
    head = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24"><title>%s</title>' % name
    if kind == "line":
        return head + elements(data, "currentColor") + "</svg>\n"
    # gradientUnits default objectBoundingBox: the gradient spans each shape's box,
    # so it is given in user space here for one sweep across the whole icon.
    defs = ('<defs>'
            '<linearGradient id="g" gradientUnits="userSpaceOnUse" x1="2" y1="0" x2="22" y2="0">%s</linearGradient>'
            '<linearGradient id="b" gradientUnits="userSpaceOnUse" x1="0" y1="1" x2="0" y2="23">'
            '<stop offset="0" stop-color="%s"/><stop offset="1" stop-color="%s"/></linearGradient>'
            '<linearGradient id="s" gradientUnits="userSpaceOnUse" x1="0" y1="3" x2="0" y2="21">'
            '<stop offset="0" stop-color="%s"/><stop offset="1" stop-color="%s"/></linearGradient>'
            '<linearGradient id="f" gradientUnits="userSpaceOnUse" x1="3" y1="0" x2="21" y2="0">%s</linearGradient>'
            '</defs>') % ("".join('<stop offset="%g" stop-color="%s"/>' % s for s in GLYPH), BADGE_TOP, BADGE_BOTTOM,
                         SHEET_TOP, SHEET_BOTTOM, "".join('<stop offset="%g" stop-color="%s"/>' % s for s in FOLDER))
    if kind == "app":
        k = APP_GLYPH_SCALE
        body = ('<rect x="1" y="1" width="22" height="22" rx="5.5" fill="url(#b)"/>'
                '<rect x="1.25" y="1.25" width="21.5" height="21.5" rx="5.25" fill="none" stroke="%s" stroke-opacity="0.22" stroke-width="0.5"/>'
                '<g transform="translate(12 12) scale(%g) translate(-12 -12)">%s</g>') % (RIM, k, elements(data, "url(#g)"))
        return head + defs + body + "</svg>\n"
    if name == "folder":
        body = ('<path d="M2 6 Q2 4 4 4 H9.5 L11.5 6 H20 Q22 6 22 8 V18 Q22 20 20 20 H4 Q2 20 2 18 Z" fill="url(#f)"/>'
                '<path d="M2 9.5 H22" stroke="%s" stroke-opacity="0.35" stroke-width="1"/>') % BADGE_BOTTOM
        return head + defs + body + "</svg>\n"
    sheet = ('<path d="M5 2 H15 L20 7 V22 H5 Z" fill="url(#s)" stroke="%s" stroke-opacity="0.3" stroke-width="0.6" stroke-linejoin="round"/>'
             '<path d="M15 2 V7 H20" fill="none" stroke="%s" stroke-opacity="0.5" stroke-width="0.6" stroke-linejoin="round"/>') % (RIM, RIM)
    return head + defs + sheet + elements(data, "url(#g)", 1.6) + "</svg>\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="only parse the source")
    a = ap.parse_args()
    icons = load()
    for kind, name, data in icons:
        parts(data)
    if a.check:
        print("icons: %d parse" % len(icons))
        return
    import cairosvg
    from PIL import Image
    for kind, name, data in icons:
        d = OUT / "svg" / kind
        d.mkdir(parents=True, exist_ok=True)
        text = svg(kind, name, data)
        (d / (name + ".svg")).write_text(text)
        if kind == "line":
            continue
        for px in SIZES:
            p = OUT / "png" / str(px)
            p.mkdir(parents=True, exist_ok=True)
            cairosvg.svg2png(bytestring=text.encode(), write_to=str(p / (name + ".png")), output_width=px, output_height=px)
    # One sheet: app and file icons at 64 px, line icons at 32 px in rose gold on charcoal.
    shown = [i for i in icons if i[0] != "line"]
    lines = [i for i in icons if i[0] == "line"]
    cols = 10
    rows_a, rows_l = -(-len(shown) // cols), -(-len(lines) // (cols * 2))
    sheet = Image.new("RGBA", (cols * 80 + 16, rows_a * 80 + rows_l * 40 + 32), BADGE_BOTTOM)
    for n, (kind, name, data) in enumerate(shown):
        im = Image.open(OUT / "png" / "64" / (name + ".png"))
        sheet.alpha_composite(im, (16 + (n % cols) * 80, 16 + (n // cols) * 80))
    import io
    for n, (kind, name, data) in enumerate(lines):
        text = svg(kind, name, data).replace("currentColor", "#e2a084")
        im = Image.open(io.BytesIO(cairosvg.svg2png(bytestring=text.encode(), output_width=32, output_height=32)))
        sheet.alpha_composite(im, (16 + (n % (cols * 2)) * 40, 24 + rows_a * 80 + (n // (cols * 2)) * 40))
    sheet.save(OUT / "preview.png")
    print("icons: %d written" % len(icons))


if __name__ == "__main__":
    main()
