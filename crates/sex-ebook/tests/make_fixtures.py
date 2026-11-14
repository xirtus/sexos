#!/usr/bin/env python3
"""Writes the small books the sex-ebook host tests and the Bodice gate read:
tiny.epub (EPUB 3 with a nav document and an EPUB 2 NCX), tiny.fb2 and tiny.cbz.
The output is the same on every run (fixed timestamps, fixed text), so the files
are committed and this script only needs running again when they should change."""
import struct, zipfile, zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
WHEN = (2026, 11, 14, 12, 0, 0)
WORDS = ("the lamp river quiet morning letter garden window harbour road lantern orchard "
         "bridge evening market candle meadow station winter valley ferry chapel clock").split()

def paragraph(ch, n):
    words = [WORDS[(ch * 7 + n * 3 + k * 5) % len(WORDS)] for k in range(48)]
    return f"Paragraph {n} of chapter {ch}: " + " ".join(words) + "."

CHAPTERS = [(f"Chapter {c}", [paragraph(c, n) for n in range(1, 13)]) for c in (1, 2, 3)]

def add(z, name, data, stored=False):
    info = zipfile.ZipInfo(name, WHEN)
    info.compress_type = zipfile.ZIP_STORED if stored else zipfile.ZIP_DEFLATED
    info.external_attr = 0o644 << 16
    z.writestr(info, data)

def epub():
    with zipfile.ZipFile(HERE / "tiny.epub", "w") as z:
        add(z, "mimetype", "application/epub+zip", stored=True)
        add(z, "META-INF/container.xml", '<?xml version="1.0"?>\n<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">'
            '<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
        items = "".join(f'<item id="c{i}" href="text/ch{i}.xhtml" media-type="application/xhtml+xml"/>' for i in (1, 2, 3))
        spine = "".join(f'<itemref idref="c{i}"/>' for i in (1, 2, 3))
        add(z, "OEBPS/content.opf", '<?xml version="1.0" encoding="UTF-8"?>\n<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">'
            '<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">urn:sexos:tiny</dc:identifier>'
            '<dc:title>A Tiny Book</dc:title><dc:creator>Bodice Fixture</dc:creator><dc:language>en</dc:language></metadata>'
            f'<manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>'
            f'<item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>{items}</manifest>'
            f'<spine toc="ncx">{spine}</spine></package>')
        links = "".join(f'<li><a href="text/ch{i}.xhtml">{t}</a></li>' for i, (t, _) in enumerate(CHAPTERS, 1))
        add(z, "OEBPS/nav.xhtml", '<?xml version="1.0" encoding="UTF-8"?>\n<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">'
            f'<head><title>Contents</title></head><body><nav epub:type="toc"><h1>Contents</h1><ol>{links}</ol></nav></body></html>')
        points = "".join(f'<navPoint id="p{i}" playOrder="{i}"><navLabel><text>{t}</text></navLabel><content src="text/ch{i}.xhtml"/></navPoint>'
                         for i, (t, _) in enumerate(CHAPTERS, 1))
        add(z, "OEBPS/toc.ncx", '<?xml version="1.0" encoding="UTF-8"?>\n<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">'
            f'<head/><docTitle><text>A Tiny Book</text></docTitle><navMap>{points}</navMap></ncx>')
        for i, (title, paras) in enumerate(CHAPTERS, 1):
            body = "".join(f"<p>{p}</p>\n" for p in paras)
            add(z, f"OEBPS/text/ch{i}.xhtml", '<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE html>\n<html xmlns="http://www.w3.org/1999/xhtml">'
                f'<head><title>{title}</title><style>p {{ margin: 0 }}</style></head><body><h1>{title}</h1>\n{body}</body></html>')

def fb2():
    sections = "".join("<section><title><p>" + t + "</p></title>" + "".join(f"<p>{p}</p>" for p in paras[:3]) + "</section>" for t, paras in CHAPTERS)
    text = ('<?xml version="1.0" encoding="UTF-8"?>\n<FictionBook xmlns="http://www.gribuser.ru/xml/fictionbook/2.0">'
            '<description><title-info><author><first-name>Bodice</first-name><last-name>Fixture</last-name></author>'
            '<book-title>A Tiny FictionBook</book-title></title-info></description>'
            f'<body><title><p>A Tiny FictionBook</p></title>{sections}</body>'
            '<body name="notes"><section><p>A note &amp; nothing more.</p></section></body></FictionBook>\n')
    (HERE / "tiny.fb2").write_text(text)

def png(w, h, rgb):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    raw = b"".join(b"\0" + bytes(rgb) * w for _ in range(h))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")

def cbz():
    with zipfile.ZipFile(HERE / "tiny.cbz", "w") as z:
        add(z, "page10.png", png(8, 6, (0, 0, 255)))
        add(z, "page2.png", png(8, 6, (0, 255, 0)))
        add(z, "page1.png", png(8, 6, (255, 0, 0)))
        add(z, "ComicInfo.xml", "<ComicInfo><Title>Tiny</Title></ComicInfo>")

if __name__ == "__main__":
    epub(); fb2(); cbz()
