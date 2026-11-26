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
        lines += ['', f'The hidden word is {UNIQUE} and nowhere else.', 'A café appears only on this page.',
                  'The report describes inter-',
                  'national travel, beside the literal co-operate.']
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
        3: '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
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


FORM_WORD = 'PARAKEET'


def form_document(*, catalog_docmdp=False):
    """One US Letter page with a line of text holding FORM_WORD, an AcroForm text field
    `name` and a checkbox `agree` (with on and off appearances), for Vellum's edit gate."""
    text = (f'BT /F1 16 Tf 72 700 Td (Vellum form: the word {FORM_WORD} gets highlighted.) Tj ET\n'
            'BT /F1 12 Tf 72 630 Td (Name:) Tj ET\nBT /F1 12 Tf 100 563 Td (I agree) Tj ET\n'
            '0.6 G 1 w 72 590 228 26 re S\n')
    on = '0 g 4 4 10 10 re f 0 G 1 w 0.5 0.5 17 17 re S'
    off = '0 G 1 w 0.5 0.5 17 17 re S'
    def stream(dict_, data):
        return f'<< {dict_} /Length {len(data)} >>\nstream\n{data}\nendstream'
    objects = {
        1: f'<< /Type /Catalog /Pages 2 0 R {"/Perms << /DocMDP 10 0 R >> " if catalog_docmdp else ""}/AcroForm << /Fields [6 0 R 7 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>',
        2: '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        3: '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [6 0 R 7 0 R] >>',
        4: stream('', text),
        5: '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
        6: '<< /Type /Annot /Subtype /Widget /T (name) /FT /Tx /DA (/Helv 14 Tf 0 0 0.6 rg) /Rect [72 590 300 616] /F 4 /P 3 0 R >>',
        7: '<< /Type /Annot /Subtype /Widget /T (agree) /FT /Btn /V /Off /AS /Off /Rect [72 558 90 576] /F 4 /P 3 0 R /AP << /N << /Yes 8 0 R /Off 9 0 R >> >> >>',
        8: stream('/Type /XObject /Subtype /Form /BBox [0 0 18 18]', on),
        9: stream('/Type /XObject /Subtype /Form /BBox [0 0 18 18]', off),
    }
    if catalog_docmdp:
        # Catalog certification signature without a discoverable AcroForm /Sig field.
        objects[10] = '<< /Type /Sig /Filter /Adobe.PPKLite /ByteRange [0 1 2 3] /Contents <0000> >>'
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


def choice_document():
    """A real AcroForm with export/display pairs in a combo and single list."""
    content = ('BT /F1 15 Tf 72 725 Td (Vellum choices) Tj ET\n'
               'BT /F1 12 Tf 72 670 Td (Size:) Tj ET\n'
               'BT /F1 12 Tf 72 600 Td (Team:) Tj ET\n')
    objects = {
        1: '<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R 7 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>',
        2: '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        3: '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [6 0 R 7 0 R] >>',
        4: f'<< /Length {len(content)} >>\nstream\n{content}endstream',
        5: '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
        6: '<< /Type /Annot /Subtype /Widget /T (size) /FT /Ch /Ff 131072 /Opt [[(S) (Small)] [(M) (Medium)] [(L) (Large)]] /V (S) /DA (/Helv 12 Tf 0 g) /Rect [140 655 340 681] /F 4 /P 3 0 R >>',
        7: '<< /Type /Annot /Subtype /Widget /T (team) /FT /Ch /Opt [[(A) (Accounting)] [(E) (Engineering)] [(D) (Design)]] /V (A) /DA (/Helv 12 Tf 0 g) /Rect [140 540 340 618] /F 4 /P 3 0 R >>',
    }
    out = bytearray(b'%PDF-1.4\n%\xe2\xe3\xcf\xd3\n')
    offsets = {}
    for num in sorted(objects):
        offsets[num] = len(out)
        out += f'{num} 0 obj\n{objects[num]}\nendobj\n'.encode('latin-1')
    xref = len(out)
    out += f'xref\n0 {len(objects) + 1}\n0000000000 65535 f \n'.encode()
    for num in sorted(objects):
        out += f'{offsets[num]:010} 00000 n \n'.encode()
    out += f'trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
    return bytes(out)


def radio_document():
    """One radio group whose two widgets have distinct PDF on-state names."""
    content = 'BT /F1 14 Tf 72 700 Td (Vellum radio choices) Tj ET\n'
    on = '0 g 4 4 12 12 re f'
    off = '0 G 1 w 0.5 0.5 19 19 re S'
    def stream(data):
        return f'<< /Type /XObject /Subtype /Form /BBox [0 0 20 20] /Length {len(data)} >>\nstream\n{data}\nendstream'
    objects = {
        1: '<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] /DA (/Helv 0 Tf 0 g) >> >>',
        2: '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        3: '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [7 0 R 8 0 R] >>',
        4: f'<< /Length {len(content)} >>\nstream\n{content}endstream',
        5: '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
        6: '<< /T (color) /FT /Btn /Ff 49152 /V /Red /Kids [7 0 R 8 0 R] >>',
        7: '<< /Type /Annot /Subtype /Widget /Parent 6 0 R /AS /Red /Rect [72 620 92 640] /P 3 0 R /AP << /N << /Red 9 0 R /Off 10 0 R >> >> >>',
        8: '<< /Type /Annot /Subtype /Widget /Parent 6 0 R /AS /Off /Rect [150 620 170 640] /P 3 0 R /AP << /N << /Blue 9 0 R /Off 10 0 R >> >> >>',
        9: stream(on),
        10: stream(off),
    }
    out = bytearray(b'%PDF-1.4\n%\xe2\xe3\xcf\xd3\n')
    offsets = {}
    for num in sorted(objects):
        offsets[num] = len(out)
        out += f'{num} 0 obj\n{objects[num]}\nendobj\n'.encode('latin-1')
    xref = len(out)
    out += f'xref\n0 {len(objects) + 1}\n0000000000 65535 f \n'.encode()
    for num in sorted(objects):
        out += f'{offsets[num]:010} 00000 n \n'.encode()
    out += f'trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
    return bytes(out)


def advanced_choice_document():
    """Editable combo and multi-select list with paired PDF export/display values."""
    content = ('BT /F1 15 Tf 72 725 Td (Vellum advanced choices) Tj ET\n'
               'BT /F1 12 Tf 72 670 Td (Project:) Tj ET\n'
               'BT /F1 12 Tf 72 600 Td (Teams:) Tj ET\n')
    objects = {
        1: '<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R 7 0 R] /DA (/Helv 0 Tf 0 g) /DR << /Font << /Helv 5 0 R >> >> >> >>',
        2: '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        3: '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /Annots [6 0 R 7 0 R] >>',
        4: f'<< /Length {len(content)} >>\nstream\n{content}endstream',
        5: '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>',
        6: '<< /Type /Annot /Subtype /Widget /T (project) /FT /Ch /Ff 393216 /Opt [[(S) (Small)] [(M) (Medium)] [(L) (Large)]] /V (Custom) /DA (/Helv 12 Tf 0 g) /Rect [140 655 340 681] /F 4 /P 3 0 R >>',
        7: '<< /Type /Annot /Subtype /Widget /T (teams) /FT /Ch /Ff 2097152 /Opt [[(A) (Accounting)] [(E) (Engineering)] [(D) (Design)]] /V [(A) (D)] /I [0 2] /DA (/Helv 12 Tf 0 g) /Rect [140 540 340 618] /F 4 /P 3 0 R >>',
    }
    out = bytearray(b'%PDF-1.4\n%\xe2\xe3\xcf\xd3\n')
    offsets = {}
    for num in sorted(objects):
        offsets[num] = len(out)
        out += f'{num} 0 obj\n{objects[num]}\nendobj\n'.encode('latin-1')
    xref = len(out)
    out += f'xref\n0 {len(objects) + 1}\n0000000000 65535 f \n'.encode()
    for num in sorted(objects):
        out += f'{offsets[num]:010} 00000 n \n'.encode()
    out += f'trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
    return bytes(out)


if __name__ == '__main__':
    target = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1] / 'tests/fixtures/pdf/find10.pdf'
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(document())
    print(target, len(document()))
