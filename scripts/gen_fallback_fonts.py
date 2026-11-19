#!/usr/bin/env python3
"""Builds the fallback fonts Display draws when Geist lacks a character.

Inputs are the Noto fonts (SIL OFL 1.1) as Debian/Ubuntu ship them:
  apt install fonts-noto-core fonts-noto-cjk fonts-noto-color-emoji python3-fonttools python3-pil
Outputs, in assets/fonts/noto/:
  NotoSansMono-Regular.ttf     unchanged: Latin extended, Greek, Cyrillic, IPA, in cells
  NotoSansSymbols2-Regular.ttf unchanged: arrows, shapes, dingbats, chess, braille...
  NotoSansCJK-SC-Subset.otf    Noto Sans CJK SC cut to GB2312, JIS X 0208, KS X 1001 and
                               Big5 level 1 (about 14,000 characters), kana, Hangul
                               compatibility jamo and full-width forms
  emoji-36.sxe                 every single-scalar emoji of Noto Color Emoji as 36x36
                               4-bit coverage (shaded by lightness), for monochrome text
  emoji-color-32.sxc           the same emoji in colour, 32x32, 15 colours + transparent
                               per emoji (format in crates/sex-font/src/fallback.rs)
Run: python3 scripts/gen_fallback_fonts.py
"""
import io, shutil, struct, subprocess, sys, tempfile
from pathlib import Path
from fontTools.ttLib import TTFont, TTCollection
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'assets/fonts/noto'
NOTO = Path('/usr/share/fonts/truetype/noto')
CJK = Path('/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc')
EMOJI_SIZE = 36
COLOR_SIZE = 32

def charset(codec, rows, cols):
    s = set()
    for r in rows:
        for c in cols:
            try: s.add(bytes([r, c]).decode(codec))
            except Exception: pass
    return s

def cjk(tmp):
    hi = range(0xA1, 0xFF)
    chars = (charset('gb2312', range(0xA1, 0xF8), hi) | charset('euc_jp', range(0xA1, 0xF5), hi)
             | charset('euc_kr', range(0xA1, 0xFE), hi)
             | charset('big5', range(0xA4, 0xC7), list(range(0x40, 0x7F)) + list(range(0xA1, 0xFF))))
    text = tmp / 'chars.txt'; text.write_text(''.join(sorted(chars)))
    sc = tmp / 'sc.otf'
    TTCollection(str(CJK)).fonts[2].save(str(sc))  # Noto Sans CJK SC
    subprocess.run([sys.executable, '-m', 'fontTools.subset', str(sc), f'--text-file={text}',
                    '--unicodes=U+3000-30FF,U+3130-318F,U+FF00-FFEF', '--no-hinting', "--layout-features=",
                    f'--output-file={OUT / "NotoSansCJK-SC-Subset.otf"}'], check=True)

def emoji():
    font = TTFont(str(NOTO / 'NotoColorEmoji.ttf'))
    strike = font['CBDT'].strikeData[0]
    entries = []
    for scalar, name in sorted(font.getBestCmap().items()):
        if scalar < 0x2000 or name not in strike:  # leave digits, # and * to the text fonts
            continue
        image = Image.open(io.BytesIO(strike[name].imageData)).convert('RGBA')
        image.thumbnail((EMOJI_SIZE, EMOJI_SIZE), Image.LANCZOS)
        canvas = Image.new('RGBA', (EMOJI_SIZE, EMOJI_SIZE))
        canvas.paste(image, ((EMOJI_SIZE - image.width) // 2, (EMOJI_SIZE - image.height) // 2))
        levels = []
        for r, g, b, a in canvas.getdata():
            light = (299 * r + 587 * g + 114 * b) / 255000
            levels.append(min(15, round(a / 255 * (1 - 0.6 * light) * 15)))
        packed = bytes(levels[i] << 4 | levels[i + 1] for i in range(0, len(levels), 2))
        entries.append((scalar, packed))
    # "SXE1", size, count, then sorted scalars (u32 LE), then the bitmaps in that order.
    blob = b'SXE1' + struct.pack('<HH', EMOJI_SIZE, len(entries))
    blob += b''.join(struct.pack('<I', s) for s, _ in entries) + b''.join(p for _, p in entries)
    (OUT / f'emoji-{EMOJI_SIZE}.sxe').write_bytes(blob)
    return len(entries)

def color_emoji():
    """Every single-scalar emoji in colour: per emoji a 16-entry RGBA palette (entry 0
    transparent) and 4-bit indices, so the whole set stays under a megabyte."""
    font = TTFont(str(NOTO / 'NotoColorEmoji.ttf'))
    strike = font['CBDT'].strikeData[0]
    entries = []
    for scalar, name in sorted(font.getBestCmap().items()):
        if scalar < 0x2000 or name not in strike:
            continue
        image = Image.open(io.BytesIO(strike[name].imageData)).convert('RGBA')
        image.thumbnail((COLOR_SIZE, COLOR_SIZE), Image.LANCZOS)
        canvas = Image.new('RGBA', (COLOR_SIZE, COLOR_SIZE))
        canvas.paste(image, ((COLOR_SIZE - image.width) // 2, (COLOR_SIZE - image.height) // 2))
        q = canvas.quantize(colors=15, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE)
        pal = q.getpalette(rawmode='RGBA')[:15 * 4]
        pal += [0] * (15 * 4 - len(pal))
        idx = list(q.getdata())
        alpha = [a for *_, a in canvas.getdata()]
        # Index 0 is transparent; the quantiser's colours move up by one.
        levels = [0 if a < 8 else i + 1 for i, a in zip(idx, alpha)]
        palette = bytes([0, 0, 0, 0]) + bytes(pal)
        packed = bytes(levels[i] << 4 | levels[i + 1] for i in range(0, len(levels), 2))
        entries.append((scalar, palette + packed))
    blob = b'SXC1' + struct.pack('<HH', COLOR_SIZE, len(entries))
    blob += b''.join(struct.pack('<I', s) for s, _ in entries) + b''.join(p for _, p in entries)
    (OUT / f'emoji-color-{COLOR_SIZE}.sxc').write_bytes(blob)
    return len(entries)

def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name in ['NotoSansMono-Regular.ttf', 'NotoSansSymbols2-Regular.ttf', 'NotoSansArabic-Regular.ttf',
                 'NotoSansHebrew-Regular.ttf', 'NotoSansDevanagari-Regular.ttf', 'NotoSansThai-Regular.ttf']:
        shutil.copyfile(NOTO / name, OUT / name)
    with tempfile.TemporaryDirectory() as tmp:
        cjk(Path(tmp))
    print('emoji', emoji())
    print('colour emoji', color_emoji())

if __name__ == '__main__':
    main()
