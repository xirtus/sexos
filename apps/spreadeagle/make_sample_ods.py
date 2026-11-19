#!/usr/bin/env python3
"""Writes assets/sample.ods: a small OpenDocument spreadsheet written by hand, so the ODS
reader is tested on a file from a writer other than GridCraft. Run from anywhere; the
output is committed."""
import zipfile
from pathlib import Path

MANIFEST = '''<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">
<manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
</manifest:manifest>'''

ROWS = [('Fruit', 'Kilos'), ('Pears', 12), ('Apples', 30), ('Plums', 7)]


def cell(v):
    if isinstance(v, str):
        return f'<table:table-cell office:value-type="string"><text:p>{v}</text:p></table:table-cell>'
    return f'<table:table-cell office:value-type="float" office:value="{v}"><text:p>{v}</text:p></table:table-cell>'


rows = ''.join('<table:table-row>' + ''.join(cell(v) for v in r) + '</table:table-row>' for r in ROWS)
total = ('<table:table-row><table:table-cell office:value-type="string"><text:p>Total</text:p></table:table-cell>'
         '<table:table-cell table:formula="of:=SUM([.B2:.B4])" office:value-type="float" office:value="49"><text:p>49</text:p></table:table-cell></table:table-row>')
CONTENT = f'''<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2">
<office:body><office:spreadsheet><table:table table:name="Harvest">{rows}{total}</table:table></office:spreadsheet></office:body>
</office:document-content>'''

out = Path(__file__).resolve().parent / 'assets' / 'sample.ods'
with zipfile.ZipFile(out, 'w') as z:
    # The mimetype comes first and uncompressed, as ODF requires.
    z.writestr(zipfile.ZipInfo('mimetype'), 'application/vnd.oasis.opendocument.spreadsheet', compress_type=zipfile.ZIP_STORED)
    z.writestr('META-INF/manifest.xml', MANIFEST, compress_type=zipfile.ZIP_DEFLATED)
    z.writestr('content.xml', CONTENT, compress_type=zipfile.ZIP_DEFLATED)
print(out, out.stat().st_size)
