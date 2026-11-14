#!/usr/bin/env python3
"""Records HarfBuzz's own output (hb-shape) for the strings crates/sex-shape/tests/shape.rs
checks rustybuzz against. Needs `apt install libharfbuzz-bin`.
Run: python3 scripts/gen_shape_reference.py"""
import json, subprocess
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
F = 'assets/fonts/noto/'
CASES = [
    (F + 'NotoSansArabic-Regular.ttf', 'سلام'), (F + 'NotoSansArabic-Regular.ttf', 'العربية'),
    (F + 'NotoSansArabic-Regular.ttf', 'مرحبا بالعالم'), (F + 'NotoSansArabic-Regular.ttf', 'لا'),
    (F + 'NotoSansArabic-Regular.ttf', 'بِسْمِ'), (F + 'NotoSansArabic-Regular.ttf', 'فارسی'),
    (F + 'NotoSansArabic-Regular.ttf', 'اردو'), (F + 'NotoSansArabic-Regular.ttf', 'ـبـ'),
    (F + 'NotoSansDevanagari-Regular.ttf', 'नमस्ते'), (F + 'NotoSansDevanagari-Regular.ttf', 'हिन्दी'),
    (F + 'NotoSansDevanagari-Regular.ttf', 'क्षत्रिय'), (F + 'NotoSansDevanagari-Regular.ttf', 'कि'),
    (F + 'NotoSansDevanagari-Regular.ttf', 'र्क'), (F + 'NotoSansDevanagari-Regular.ttf', 'श्री'),
    (F + 'NotoSansHebrew-Regular.ttf', 'שָׁלוֹם'), (F + 'NotoSansHebrew-Regular.ttf', 'עברית'),
    (F + 'NotoSansThai-Regular.ttf', 'สวัสดี'), (F + 'NotoSansThai-Regular.ttf', 'ภาษาไทย'),
    (F + 'NotoSansMono-Regular.ttf', 'é ä ñ'), (F + 'NotoSansMono-Regular.ttf', 'Ελληνικά Кириллица'),
    ('assets/fonts/geist/Geist-Regular.ttf', 'office flight AVAWA'),
]
lines = []
for font, text in CASES:
    out = subprocess.run(['hb-shape', '--no-glyph-names', '--output-format=json', str(ROOT / font), text],
                         check=True, capture_output=True, text=True).stdout
    glyphs = json.loads(out)
    lines.append('\t'.join([font, text, ' '.join(f"{g['g']},{g['cl']},{g['ax']},{g['ay']},{g['dx']},{g['dy']}" for g in glyphs)]))
(ROOT / 'crates/sex-shape/tests/data/hb_reference.tsv').write_text('\n'.join(lines) + '\n')
print(len(lines), 'strings')
