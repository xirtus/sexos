#!/usr/bin/env python3
"""Build a small XLSX with a filter value outside the first 1,000 choices."""
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent
NS = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
PKG = 'http://schemas.openxmlformats.org/package/2006/relationships'
CT = 'http://schemas.openxmlformats.org/package/2006/content-types'
rows = '<row r="1"><c r="A1" t="inlineStr"><is><t>Item</t></is></c></row>'
rows += ''.join(f'<row r="{r}"><c r="A{r}" t="inlineStr"><is><t>Item {r-1:04}</t></is></c></row>' for r in range(2, 1102))
rows += ''.join(f'<row r="{r}"><c r="A{r}" t="inlineStr"><is><t>Late {label}</t></is></c></row>' for r, label in ((1102, 'amber'), (1103, 'azure')))
parts = {
    '[Content_Types].xml': f'<Types xmlns="{CT}"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>',
    '_rels/.rels': f'<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>',
    'xl/workbook.xml': f'<workbook xmlns="{NS}" xmlns:r="{REL}"><sheets><sheet name="Items" sheetId="1" r:id="rId1"/></sheets></workbook>',
    'xl/_rels/workbook.xml.rels': f'<Relationships xmlns="{PKG}"><Relationship Id="rId1" Type="{REL}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>',
    'xl/worksheets/sheet1.xml': f'<worksheet xmlns="{NS}"><dimension ref="A1:A1103"/><sheetData>{rows}</sheetData></worksheet>',
}
out = ROOT / 'assets/filter-overflow.xlsx'
with zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED) as z:
    for name, body in parts.items(): z.writestr(name, '<?xml version="1.0" encoding="UTF-8"?>' + body)
print(out, out.stat().st_size)
