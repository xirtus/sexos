// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;

fn make_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();

    for (name, data) in entries {
        let offset = out.len() as u32;
        let crc = crc32fast::hash(data);
        let name_bytes = name.as_bytes();

        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        cd.extend_from_slice(b"PK\x01\x02");
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u32.to_le_bytes());
        cd.extend_from_slice(&crc.to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u32.to_le_bytes());
        cd.extend_from_slice(&offset.to_le_bytes());
        cd.extend_from_slice(name_bytes);
    }

    let cd_offset = out.len() as u32;
    let cd_size = cd.len() as u32;
    out.extend_from_slice(&cd);

    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    out
}

const EPUB_MIME: &[u8] = b"application/epub+zip";

const MINIMAL_CONTAINER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#;

const MINIMAL_OPF: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>Fuzz Ebook</dc:title>
    <dc:creator>SexOS</dc:creator>
  </metadata>
  <manifest>
    <item id="ch1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
  </spine>
</package>"#;

const MINIMAL_NCX: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <navMap>
    <navPoint id="p1" playOrder="1">
      <navLabel><text>Chapter 1</text></navLabel>
      <content src="text/ch1.xhtml"/>
    </navPoint>
  </navMap>
</ncx>"#;

const MINIMAL_NAV: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol>
        <li><a href="text/ch1.xhtml">Chapter 1</a></li>
      </ol>
    </nav>
  </body>
</html>"#;

const MINIMAL_CHAPTER: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <head><title>Chapter 1</title></head>
  <body>
    <h1>Chapter 1</h1>
    <p>Sample chapter text for reflow.</p>
  </body>
</html>"#;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Raw bytes open
    if let Ok(book) = sex_ebook::open("test.epub", data) {
        exercise_book(&book, data);
    }

    if let Ok(z) = sex_ebook::zip::Zip::open(data) {
        if let Ok(book) = sex_ebook::epub::read(&z) {
            exercise_book(&book, data);
        }
    }

    // 2. Structured EPUB mode: wrap mutated container, opf, ncx, nav, or chapter in valid ZIP
    let (choice, payload) = (data[0] % 7, &data[1..]);
    let mut container = MINIMAL_CONTAINER;
    let mut opf = MINIMAL_OPF;
    let mut ncx = MINIMAL_NCX;
    let mut nav = MINIMAL_NAV;
    let mut chapter = MINIMAL_CHAPTER;

    let adversarial_nav_traversal = br#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops">
  <body>
    <nav epub:type="toc">
      <ol>
        <li><a href="../../../etc/passwd">Traversal 1</a></li>
        <li><a href="../text/ch1.xhtml#p999999">Valid with huge fragment</a></li>
        <li><a href="text/%FF%FE%AA.xhtml">Bad percent encoding</a></li>
      </ol>
    </nav>
  </body>
</html>"#;

    let adversarial_chapter_utf8 = br#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
  <body>
    <h1>Chapter 1: &#x1F980; Multi-byte &amp; RTL</h1>
    <p>Word&#160;with&#x200B;zero&#xFEFF;width&#x202E;override</p>
    <pre>Preformatted&#10;line2&#10;very long word without spaces: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</pre>
    <blockquote>Quote with leading and trailing spaces   </blockquote>
  </body>
</html>"#;

    match choice {
        0 => container = payload,
        1 => opf = payload,
        2 => ncx = payload,
        3 => nav = payload,
        4 => chapter = payload,
        5 => nav = adversarial_nav_traversal,
        _ => chapter = adversarial_chapter_utf8,
    }

    let epub_zip = make_stored_zip(&[
        ("mimetype", EPUB_MIME),
        ("META-INF/container.xml", container),
        ("OEBPS/content.opf", opf),
        ("OEBPS/toc.ncx", ncx),
        ("OEBPS/nav.xhtml", nav),
        ("OEBPS/text/ch1.xhtml", chapter),
    ]);

    if let Ok(book) = sex_ebook::open("structured.epub", &epub_zip) {
        exercise_book(&book, data);
    }

    // 3. Direct layout::wrap and resolve testing
    if let Ok(text) = core::str::from_utf8(data) {
        let cols = if data.len() >= 2 { data[1] as usize } else { 0 };
        let _ = sex_ebook::layout::wrap(text, 0);
        let _ = sex_ebook::layout::wrap(text, 1);
        let _ = sex_ebook::layout::wrap(text, 2);
        let _ = sex_ebook::layout::wrap(text, cols);
        let _ = sex_ebook::resolve("OEBPS/text/", text);
        let _ = sex_ebook::resolve("", text);
        let _ = sex_ebook::folder(text);
    }
});

fn exercise_book(book: &sex_ebook::Book, data: &[u8]) {
    let _ = book.parts();

    // Derive arbitrary cols and rows (including 0, 1, 2)
    let c0 = if data.len() >= 2 { data[0] as usize } else { 0 };
    let r0 = if data.len() >= 2 { data[1] as usize } else { 0 };

    for &(cols, rows) in &[(0, 0), (1, 1), (2, 2), (8, 2), (c0, r0), (80, 24)] {
        let pages = sex_ebook::layout::paginate(book, cols, rows);
        for p in pages.iter().take(4) {
            let _ = &p.lines;
            let _ = p.start;
            let _ = p.chapter;
        }
    }
}
