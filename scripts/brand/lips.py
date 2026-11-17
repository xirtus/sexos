#!/usr/bin/env python3
"""The SexOS mark: rose-gold lips, traced from Andreas' prototype (2026-11-16).

This file is the one source of the mark's shape. It writes:
  assets/brand/*.svg            the vector logo (mark, badge, mono, wallpaper)
  website/sexos-lips.svg            the site's icon and nav mark
  crates/sex-pdx/src/brand_lips.rs  the outline, flattened, for native drawing
With --png it also renders PNG sizes and wallpapers (needs `pip install cairosvg`).

The mark is symmetric about its centre line; only the left half is written
here, as cubic Beziers from the corner of the mouth to the centre line.
Design units: the prototype's 460 px canvas, so the mark spans x 98..366."""
import argparse, math, pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
CENTRE = 232
# Left halves: start point, then (control, control, end) per cubic.
UPPER_OUTER = [(98, 236), (127, 217), (154, 174), (176, 161), (187.5, 151), (198, 149), (211, 159), (221, 169), (227, 174), (232, 174)]
UPPER_INNER = [(98, 236), (142, 226), (162, 201), (182, 200), (192, 199.5), (195, 213), (204, 225), (211, 233), (222, 235), (232, 235)]
LOWER_TOP = [(104, 236), (107, 230), (163, 234), (173, 246), (190.5, 263), (207, 274), (232, 274)]
LOWER_BOTTOM = [(104, 236), (133, 244), (148, 251), (162, 274), (180, 294), (208, 308), (232, 308)]
ORIGIN = (98, 149)          # top-left of the mark's box
SIZE = (268, 159)           # the mark's box, design units

# Rose-gold, dark at the left corner to peach at the right, as in the prototype.
RICH = [(0.0, "#5e3f37"), (0.38, "#9c6e5e"), (0.75, "#f5aa8d"), (1.0, "#ffc4a6")]
# Lighter stops for small sizes on dark glass, where the dark corner would vanish.
BRIGHT = [(0.0, "#b07a66"), (0.5, "#e2a084"), (1.0, "#ffcaae")]
CHARCOAL = "#2e2a2b"
WALL_TOP, WALL_BOTTOM = "#332e2f", "#1c1919"


def smooth(pts):
    """Makes each join G1: the two handles at a join point along one line."""
    pts = list(pts)
    for j in range(3, len(pts) - 1, 3):
        (ax, ay), (jx, jy), (bx, by) = pts[j - 1], pts[j], pts[j + 1]
        dx, dy = (bx - ax), (by - ay)
        n = math.hypot(dx, dy) or 1
        la, lb = math.hypot(jx - ax, jy - ay), math.hypot(bx - jx, by - jy)
        pts[j - 1] = (round(jx - dx / n * la, 1), round(jy - dy / n * la, 1))
        pts[j + 1] = (round(jx + dx / n * lb, 1), round(jy + dy / n * lb, 1))
    return [(x - ORIGIN[0], y - ORIGIN[1]) for x, y in pts]


def mirror(pts):
    c = CENTRE - ORIGIN[0]
    return [(2 * c - x, y) for x, y in pts]


def cubics(pts):
    return [(pts[i - 1], pts[i], pts[i + 1], pts[i + 2]) for i in range(1, len(pts) - 1, 3)]


def reverse(pts):
    return list(reversed(pts))


def closed(outer, inner):
    """Left corner -> centre along `outer`, on to the right corner, back along
    the mirrored `inner` and home along `inner`: one closed outline."""
    o, i = smooth(outer), smooth(inner)
    chain = o + reverse(mirror(o))[1:] + mirror(i)[1:] + reverse(i)[1:]
    return chain


def path_d(chain):
    f = lambda v: ("%g" % v)
    d = "M%s,%s" % (f(chain[0][0]), f(chain[0][1]))
    for _, a, b, c in cubics(chain):
        d += " C%s,%s %s,%s %s,%s" % (f(a[0]), f(a[1]), f(b[0]), f(b[1]), f(c[0]), f(c[1]))
    return d + "Z"


SHAPES = {"upper": closed(UPPER_OUTER, UPPER_INNER), "lower": closed(LOWER_BOTTOM, LOWER_TOP)}


def gradient(gid, stops, x0=0, x1=SIZE[0]):
    s = "".join('<stop offset="%g" stop-color="%s"/>' % (o, c) for o, c in stops)
    return ('<linearGradient id="%s" gradientUnits="userSpaceOnUse" x1="%g" y1="0" x2="%g" y2="0">%s</linearGradient>'
            % (gid, x0, x1, s))


def mark_paths(fill, transform=""):
    t = ' transform="%s"' % transform if transform else ""
    return "".join('<path fill="%s" d="%s"%s/>' % (fill, path_d(c), t) for c in SHAPES.values())


def svg_mark(stops=RICH):
    w, h = SIZE
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" height="%d">'
            '<title>SexOS</title><defs>%s</defs>%s</svg>\n' % (w, h, w, h, gradient("rose", stops), mark_paths("url(#rose)")))


def svg_mono():
    w, h = SIZE
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" height="%d">'
            '<title>SexOS</title>%s</svg>\n' % (w, h, w, h, mark_paths("currentColor")))


def svg_badge():
    # The prototype's own framing: the mark at 58% width on a charcoal square.
    side = 460
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" height="%d">'
            '<title>SexOS</title><defs>%s</defs><rect width="%d" height="%d" rx="0" fill="%s"/>'
            '<g transform="translate(%d %d)">%s</g></svg>\n'
            % (side, side, side, side, gradient("rose", RICH), side, side, CHARCOAL, ORIGIN[0], ORIGIN[1], mark_paths("url(#rose)")))


def svg_wallpaper(w=1920, h=1080):
    # Same placement as Display draws it: 26% of the width, centre at 48% height.
    mw = w * 0.26
    k = mw / SIZE[0]
    mh = SIZE[1] * k
    x, y = (w - mw) / 2, h * 0.48 - mh / 2
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" height="%d">'
            '<title>SexOS</title><defs>%s'
            '<linearGradient id="wall" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="%s"/><stop offset="1" stop-color="%s"/></linearGradient>'
            '</defs><rect width="%d" height="%d" fill="url(#wall)"/>'
            '<g transform="translate(%g %g) scale(%g)">%s</g></svg>\n'
            % (w, h, w, h, gradient("rose", RICH), WALL_TOP, WALL_BOTTOM, w, h, x, y, k, mark_paths("url(#rose)")))


def bezier(p0, p1, p2, p3, t):
    u = 1 - t
    return tuple(u * u * u * a + 3 * u * u * t * b + 3 * u * t * t * c + t * t * t * d for a, b, c, d in zip(p0, p1, p2, p3))


def flatten(chain, steps=12):
    out = [chain[0]]
    for p0, p1, p2, p3 in cubics(chain):
        for s in range(1, steps + 1):
            out.append(bezier(p0, p1, p2, p3, s / steps))
    return [(round(x * 16), round(y * 16)) for x, y in out[:-1]]  # closed: last == first


def hexrgb(c):
    return int(c[1:], 16)


def rust():
    lines = ["// SPDX-License-Identifier: GPL-3.0-only",
             "// Generated by scripts/brand/lips.py from the traced mark; do not edit by hand.",
             "",
             "/// The mark's box in design units (the prototype's 460 px canvas).",
             "pub const DESIGN_W: u32 = %d;" % SIZE[0],
             "pub const DESIGN_H: u32 = %d;" % SIZE[1], ""]
    for name, chain in SHAPES.items():
        pts = flatten(chain)
        lines.append("/// The %s lip, closed outline in 1/16 design units." % name)
        lines.append("pub const %s: [(i16, i16); %d] = [" % (name.upper(), len(pts)))
        for i in range(0, len(pts), 8):
            lines.append("    " + " ".join("(%d, %d)," % p for p in pts[i:i + 8]))
        lines.append("];")
    for name, stops in (("RICH", RICH), ("BRIGHT", BRIGHT)):
        lines.append("/// Gradient stops, left to right: (position 0..=255, 0xRRGGBB).")
        lines.append("pub const %s: [(u32, u32); %d] = [%s];" % (
            name, len(stops), ", ".join("(%d, 0x%06X)" % (round(o * 255), hexrgb(c)) for o, c in stops)))
    lines.append("pub const CHARCOAL: u32 = 0x%06X;" % hexrgb(CHARCOAL))
    lines.append("pub const WALL_TOP: u32 = 0x%06X;" % hexrgb(WALL_TOP))
    lines.append("pub const WALL_BOTTOM: u32 = 0x%06X;" % hexrgb(WALL_BOTTOM))
    return "\n".join(lines) + "\n"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--png", action="store_true", help="also render PNGs (needs cairosvg)")
    a = ap.parse_args()
    out = ROOT / "assets/brand"
    out.mkdir(parents=True, exist_ok=True)
    files = {"sexos-lips.svg": svg_mark(), "sexos-lips-bright.svg": svg_mark(BRIGHT), "sexos-lips-mono.svg": svg_mono(),
             "sexos-lips-badge.svg": svg_badge(), "sexos-wallpaper.svg": svg_wallpaper()}
    for name, text in files.items():
        (out / name).write_text(text)
    (ROOT / "website/sexos-lips.svg").write_text(files["sexos-lips.svg"])
    (ROOT / "crates/sex-pdx/src/brand_lips.rs").write_text(rust())
    if a.png:
        import cairosvg
        for px in (16, 32, 64, 128, 256, 512, 1024):
            cairosvg.svg2png(bytestring=files["sexos-lips-badge.svg"].encode(), write_to=str(out / ("sexos-lips-badge-%d.png" % px)),
                             output_width=px, output_height=px)
        cairosvg.svg2png(bytestring=files["sexos-lips.svg"].encode(), write_to=str(out / "sexos-lips-512.png"), output_width=512)
        for w, h in ((1920, 1080), (2560, 1440), (3840, 2160)):
            cairosvg.svg2png(bytestring=svg_wallpaper(w, h).encode(), write_to=str(out / ("sexos-wallpaper-%dx%d.png" % (w, h))))
    print("wrote", ", ".join(files), "and crates/sex-pdx/src/brand_lips.rs")


if __name__ == "__main__":
    main()
