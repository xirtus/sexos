#!/usr/bin/env python3
"""Write an independent ODS formula fixture with OpenFormula argument edges."""
from pathlib import Path
import zipfile

root = Path(__file__).resolve().parent
rows = [
    '<table:table-row><table:table-cell office:value-type="float" office:value="10"/><table:table-cell table:formula="of:=IF([.A1]&gt;0;SUM([.A1];[.A3]);0)" office:value-type="float" office:value="15"/></table:table-row>',
    '<table:table-row><table:table-cell office:value-type="float" office:value="0"/><table:table-cell table:formula="of:=IFERROR([.A1]/[.A2];99)" office:value-type="float" office:value="99"/></table:table-row>',
    '<table:table-row><table:table-cell office:value-type="float" office:value="5"/><table:table-cell table:formula="of:=ROUND(AVERAGE([.A1];[.A3]);1)" office:value-type="float" office:value="7.5"/></table:table-row>',
    '<table:table-row><table:table-cell/><table:table-cell table:formula="of:=COUNTIF([.A1:.A3];&quot;&gt;0&quot;)" office:value-type="float" office:value="2"/></table:table-row>',
    '<table:table-row><table:table-cell/><table:table-cell table:formula="of:=[\'Q1:Q2\'.A1]+1" office:value-type="float" office:value="5"/></table:table-row>',
]
content = ('<?xml version="1.0" encoding="UTF-8"?>'
           '<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" '
           'xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" '
           'xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2">'
           '<office:body><office:spreadsheet><table:table table:name="Formula checks">'
           + ''.join(rows) + '</table:table>'
           '<table:table table:name="Q1:Q2"><table:table-row><table:table-cell office:value-type="float" office:value="4"/></table:table-row></table:table>'
           '</office:spreadsheet></office:body></office:document-content>')
manifest = ('<?xml version="1.0" encoding="UTF-8"?>'
            '<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">'
            '<manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>'
            '<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>'
            '</manifest:manifest>')
out = root / 'assets/formula-compat.ods'
with zipfile.ZipFile(out, 'w') as z:
    z.writestr('mimetype', 'application/vnd.oasis.opendocument.spreadsheet', compress_type=zipfile.ZIP_STORED)
    z.writestr('META-INF/manifest.xml', manifest, compress_type=zipfile.ZIP_DEFLATED)
    z.writestr('content.xml', content, compress_type=zipfile.ZIP_DEFLATED)
print(out, out.stat().st_size)
