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

const MINIMAL_DOCX_TYPES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>
</Types>"#;

const MINIMAL_DOCX_RELS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

const MINIMAL_WORD_RELS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>
</Relationships>"#;

const MINIMAL_DOCUMENT: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Wordcraft Docx Fuzz</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
  </w:body>
</w:document>"#;

const MINIMAL_STYLES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults><w:rPrDefault><w:rPr/></w:rPrDefault></w:docDefaults>
</w:styles>"#;

const MINIMAL_NUMBERING: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
</w:numbering>"#;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Raw byte DOCX import
    if let Ok(doc) = wordcraft_engine::io::open_bytes("fuzz.docx", data) {
        exercise_doc(doc);
    }

    // 2. Structured DOCX mode: wrap mutated document, styles, numbering, or types into valid ZIP
    let (part_choice, payload) = (data[0] % 8, &data[1..]);
    let mut document_xml = MINIMAL_DOCUMENT;
    let mut styles_xml = MINIMAL_STYLES;
    let mut numbering_xml = MINIMAL_NUMBERING;
    let mut types_xml = MINIMAL_DOCX_TYPES;

    let docx_zero_pgsz = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Zero Page Size</w:t></w:r></w:p>
    <w:sectPr><w:pgSz w:w="0" w:h="0"/></w:sectPr>
  </w:body>
</w:document>"#;

    let docx_extreme_margins = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Extreme Margins</w:t></w:r></w:p>
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
      <w:pgMar w:top="-99999" w:bottom="-99999" w:left="-99999" w:right="-99999"/>
    </w:sectPr>
  </w:body>
</w:document>"#;

    let docx_zero_col_table = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl><w:tblGrid/><w:tr><w:tc><w:p><w:r><w:t>Empty Grid</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
  </w:body>
</w:document>"#;

    let docx_nested_tbl_utf8 = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:tbl>
      <w:tr>
        <w:tc>
          <w:tbl>
            <w:tr>
              <w:tc>
                <w:p><w:r><w:t>&#x1F980;&#x200B;&#xFEFF;&#x10FFFF;&#x1F468;&#x200D;&#x1F469;&#x200D;&#x1F467;&#x200D;&#x1F466;</w:t></w:r></w:p>
              </w:tc>
            </w:tr>
          </w:tbl>
        </w:tc>
      </w:tr>
    </w:tbl>
  </w:body>
</w:document>"#;

    match part_choice {
        0 => document_xml = payload,
        1 => styles_xml = payload,
        2 => numbering_xml = payload,
        3 => types_xml = payload,
        4 => document_xml = docx_zero_pgsz,
        5 => document_xml = docx_extreme_margins,
        6 => document_xml = docx_zero_col_table,
        _ => document_xml = docx_nested_tbl_utf8,
    }

    let docx_zip = make_stored_zip(&[
        ("[Content_Types].xml", types_xml),
        ("_rels/.rels", MINIMAL_DOCX_RELS),
        ("word/_rels/document.xml.rels", MINIMAL_WORD_RELS),
        ("word/styles.xml", styles_xml),
        ("word/numbering.xml", numbering_xml),
        ("word/document.xml", document_xml),
    ]);

    if let Ok(doc) = wordcraft_engine::io::open_bytes("structured.docx", &docx_zip) {
        exercise_doc(doc);
    }
});

fn exercise_doc(doc: wordcraft_engine::doc::Document) {
    let mut session = wordcraft_engine::Session::new(doc);
    let _ = session.doc.plain_text(wordcraft_engine::doc::StoryRef::Body);
    let layout = session.layout();
    let render_opts = wordcraft_engine::render::RenderOptions::default();

    for p in 0..layout.pages.len().min(4) {
        if let Some(page) = layout.pages.get(p) {
            let _ = wordcraft_engine::render::render_page(&session.doc, page, 0.25, &render_opts);
            let _ = wordcraft_engine::render::render_page(&session.doc, page, 1.0, &render_opts);
        }
    }
}
