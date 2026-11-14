#!/usr/bin/env python3
"""Writes tests/fixtures/pdf/find10.pdf: ten text pages in plain PDF syntax, written by
hand (no PDF library), with a document outline. The word ZANZIBAR appears only on page
7, so a search that lands there proves it read every page's text, not just the first.

The PDF viewer's host tests read the committed file; the QEMU gate calls `document()`
to serve the same bytes to the system it boots."""
import sys
from pathlib import Path

PAGES = 10
UNIQUE = 'ZANZIBAR'
UNIQUE_PAGE = 7
# Outline entries: (title, page). Page numbers count from 1.
OUTLINE = [('Introduction', 1), ('Middle chapter', 5), ('The island', UNIQUE_PAGE), ('Last page', PAGES)]


def esc(text):
    return text.replace('\\', '\\\\').replace('(', '\\(').replace(')', '\\)')


def content(n):
    lines = [f'Page {n} of {PAGES}', '', 'The quick brown fox jumps over the lazy dog.', f'This is filler text for page number {n}.']
    if n == UNIQUE_PAGE:
        lines += ['', f'The hidden word is {UNIQUE} and nowhere else.']
    lines += ['', 'Every page repeats the word lantern so search can step through hits.']
    body = ' T*\n'.join(f'({esc(line)}) Tj' for line in lines)
    return f'BT /F1 14 Tf 72 720 Td 20 TL\n{body} ET\n0.2 0.4 0.8 rg 72 100 {40 * n} 20 re f\n'


def document():
    # 1 catalog, 2 pages, 3 font, 4 outlines, then per page (page, contents), then outline items.
    first_page = 5
    page_ids = [first_page + 2 * i for i in range(PAGES)]
    first_item = first_page + 2 * PAGES
    item_ids = [first_item + i for i in range(len(OUTLINE))]
    objects = {
        1: '<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R /PageMode /UseOutlines >>',
        2: f'<< /Type /Pages /Kids [{" ".join(f"{p} 0 R" for p in page_ids)}] /Count {PAGES} >>',
        3: '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
        4: f'<< /Type /Outlines /First {item_ids[0]} 0 R /Last {item_ids[-1]} 0 R /Count {len(OUTLINE)} >>',
    }
    for i, pid in enumerate(page_ids):
        stream = content(i + 1)
        objects[pid] = f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {pid + 1} 0 R >>'
        objects[pid + 1] = f'<< /Length {len(stream)} >>\nstream\n{stream}endstream'
    for i, (title, page) in enumerate(OUTLINE):
        links = ''
        if i > 0: links += f' /Prev {item_ids[i - 1]} 0 R'
        if i + 1 < len(OUTLINE): links += f' /Next {item_ids[i + 1]} 0 R'
        # The first two use /Dest, the others a GoTo action: readers meet both.
        dest = f'[{page_ids[page - 1]} 0 R /XYZ 0 792 0]'
        target = f'/Dest {dest}' if i < 2 else f'/A << /S /GoTo /D {dest} >>'
        objects[item_ids[i]] = f'<< /Title ({esc(title)}) /Parent 4 0 R{links} {target} >>'
    out = bytearray(b'%PDF-1.4\n%\xe2\xe3\xcf\xd3\n')
    offsets = {}
    for num in sorted(objects):
        offsets[num] = len(out)
        out += f'{num} 0 obj\n{objects[num]}\nendobj\n'.encode('latin-1')
    xref = len(out)
    size = max(objects) + 1
    out += f'xref\n0 {size}\n0000000000 65535 f \n'.encode()
    for num in range(1, size):
        out += f'{offsets[num]:010} 00000 n \n'.encode()
    out += f'trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
    return bytes(out)


if __name__ == '__main__':
    target = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1] / 'tests/fixtures/pdf/find10.pdf'
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(document())
    print(target, len(document()))
