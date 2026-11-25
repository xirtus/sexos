#!/usr/bin/env python3
"""Create an ODS whose amber cell fill is lost by data-only import."""
from pathlib import Path
import zipfile

root = Path(__file__).resolve().parent
source = root / 'assets/sample.ods'
out = root / 'assets/import-warning.ods'
with zipfile.ZipFile(source) as original, zipfile.ZipFile(out, 'w') as dest:
    for member in original.infolist():
        data = original.read(member.filename)
        if member.filename == 'content.xml':
            text = data.decode()
            text = text.replace('office:version="1.2">',
                'xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" '
                'xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" '
                'office:version="1.2">', 1)
            text = text.replace('<office:body>',
                '<office:automatic-styles><style:style style:name="Amber" '
                'style:family="table-cell"><style:table-cell-properties '
                'fo:background-color="#ffee99"/></style:style></office:automatic-styles>'
                '<office:body>', 1)
            text = text.replace('<table:table-cell office:value-type="string">',
                '<table:table-cell table:style-name="Amber" office:value-type="string">', 1)
            text = text.replace(' table:formula="of:=SUM([.B2:.B4])"', '')
            data = text.encode()
        dest.writestr(member, data)
print(out, out.stat().st_size)
