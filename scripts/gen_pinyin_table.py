#!/usr/bin/env python3
"""Writes crates/sex-ime/data/pinyin.txt, the Pinyin input method's dictionary.

Source: pyim-basedict 0.5.4 (GPL-3+, converted by its authors from libpinyin 2.6.0
data), as Debian/Ubuntu ship it: apt install elpa-pyim-basedict.
Kept: every syllable with its characters (in the source's frequency order), every
two-syllable word with its first three spellings (longer words are typed in parts);
only characters the CJK fallback font can draw.
Format: one entry a line, `syllables-joined-by-hyphens candidate candidate...`,
sorted by key, so the engine binary-searches the text itself.
Run: python3 scripts/gen_pinyin_table.py
"""
from pathlib import Path
from fontTools.ttLib import TTFont
ROOT = Path(__file__).resolve().parents[1]
SRC = Path('/usr/share/emacs/site-lisp/elpa-src/pyim-basedict-0.5.4/pyim-basedict.pyim')
cmap = TTFont(str(ROOT / 'assets/fonts/noto/NotoSansCJK-SC-Subset.otf')).getBestCmap()
CAP = {1: 48, 2: 3}
entries = {}
for line in SRC.read_text(encoding='utf-8').splitlines():
    if line.startswith(';;') or ' ' not in line:
        continue
    key, *words = line.split(' ')
    n = len(key.split('-'))
    if n not in CAP or not key.replace('-', '').isascii() or not key.replace('-', '').isalpha():
        continue
    words = [w for w in words if w and all(ord(c) in cmap for c in w)][:CAP[n]]
    if words:
        entries.setdefault(key, []).extend(w for w in words if w not in entries.get(key, []))
out = '\n'.join(k + ' ' + ' '.join(v) for k, v in sorted(entries.items(), key=lambda kv: kv[0].encode())) + '\n'
(ROOT / 'crates/sex-ime/data/pinyin.txt').write_text(out, encoding='utf-8')
print(len(entries), 'entries', len(out.encode()), 'bytes')
