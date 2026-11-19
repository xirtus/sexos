#!/usr/bin/env python3
"""Writes assets/welcome.docx: a short styled document in hand-written WordprocessingML,
so Assuit's first file comes from a writer other than its own. The output is committed."""
import zipfile
from pathlib import Path

W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
CT = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>'''
RELS = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>'''
DOC_RELS = '''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>'''
STYLES = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles {W}>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:pPr><w:spacing w:after="160"/></w:pPr><w:rPr><w:sz w:val="22"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:rPr><w:sz w:val="56"/><w:color w:val="0B5394"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:before="240"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/></w:rPr></w:style>
</w:styles>'''

def para(style, *runs):
    ppr = f'<w:pPr><w:pStyle w:val="{style}"/></w:pPr>' if style else ''
    return f'<w:p>{ppr}' + ''.join(runs) + '</w:p>'

def run(text, bold=False, italic=False):
    rpr = ('<w:b/>' if bold else '') + ('<w:i/>' if italic else '')
    rpr = f'<w:rPr>{rpr}</w:rPr>' if rpr else ''
    return f'<w:r>{rpr}<w:t xml:space="preserve">{text}</w:t></w:r>'

BODY = [
    para('Title', run('Welcome to Assuit')),
    para(None, run('Assuit is the SexOS word processor. It reads and writes '), run('Word documents', bold=True),
         run(' and lays them out page by page, as Word does.')),
    para('Heading1', run('Getting around')),
    para(None, run('Type anywhere. Arrows move, Shift+arrows select, Ctrl+B, Ctrl+I and Ctrl+U style the selection, '),
         run('Ctrl+Z', italic=True), run(' undoes and Ctrl+S saves.')),
    para('Heading1', run('Where it comes from')),
    para(None, run('The engine is WordCraft, from the ArtCraft suite, running unchanged on SexOS.')),
]
DOC = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<w:document {W}><w:body>' + ''.join(BODY)
       + '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>'
       + '</w:body></w:document>')

out = Path(__file__).resolve().parent / 'assets' / 'welcome.docx'
with zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED) as z:
    for name, text in [('[Content_Types].xml', CT), ('_rels/.rels', RELS), ('word/_rels/document.xml.rels', DOC_RELS),
                       ('word/styles.xml', STYLES), ('word/document.xml', DOC)]:
        zi = zipfile.ZipInfo(name, (2026, 11, 14, 0, 0, 0))
        zi.compress_type = zipfile.ZIP_DEFLATED
        z.writestr(zi, text)
print(out, out.stat().st_size)
