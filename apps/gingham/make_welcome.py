#!/usr/bin/env python3
"""Writes assets/welcome.xlsx: a small two-sheet workbook in hand-written SpreadsheetML,
so Gingham's first file comes from a writer other than its own (the reader is tested on
a file the engine did not make). Run from anywhere; the output is committed."""
import zipfile
from pathlib import Path

CT = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
</Types>'''
RELS = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>'''
WB = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Budget" sheetId="1" r:id="rId1"/><sheet name="Notes" sheetId="2" r:id="rId2"/></sheets>
</workbook>'''
WB_RELS = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
</Relationships>'''
STRINGS = ['Item', 'Cost', 'Rent', 'Food', 'Fun', 'Total', 'Per year',
           'Welcome to Gingham, the SexOS spreadsheet.', 'Arrows move, type to edit, Enter commits, Ctrl+S saves.']
SST = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
       f'<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="{len(STRINGS)}" uniqueCount="{len(STRINGS)}">'
       + ''.join(f'<si><t>{s}</t></si>' for s in STRINGS) + '</sst>')

def sheet(rows):
    out = []
    for r, cells in rows:
        out.append(f'<row r="{r}">' + ''.join(cells) + '</row>')
    return ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
            '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>'
            + ''.join(out) + '</sheetData></worksheet>')

s = lambda ref, i: f'<c r="{ref}" t="s"><v>{i}</v></c>'
n = lambda ref, v: f'<c r="{ref}"><v>{v}</v></c>'
f = lambda ref, formula, v: f'<c r="{ref}"><f>{formula}</f><v>{v}</v></c>'
BUDGET = sheet([(1, [s('A1', 0), s('B1', 1)]), (2, [s('A2', 2), n('B2', 1200)]), (3, [s('A3', 3), n('B3', 300)]),
                (4, [s('A4', 4), n('B4', 150)]), (5, [s('A5', 5), f('B5', 'SUM(B2:B4)', 1650)]),
                (6, [s('A6', 6), f('B6', 'B5*12', 19800)])])
NOTES = sheet([(1, [s('A1', 7)]), (2, [s('A2', 8)])])

out = Path(__file__).resolve().parent / 'assets' / 'welcome.xlsx'
with zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED) as z:
    for name, text in [('[Content_Types].xml', CT), ('_rels/.rels', RELS), ('xl/workbook.xml', WB),
                       ('xl/_rels/workbook.xml.rels', WB_RELS), ('xl/sharedStrings.xml', SST),
                       ('xl/worksheets/sheet1.xml', BUDGET), ('xl/worksheets/sheet2.xml', NOTES)]:
        zi = zipfile.ZipInfo(name, (2026, 11, 14, 0, 0, 0))
        zi.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(zi, text)
print(out, out.stat().st_size)
