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

const MINIMAL_CONTENT_TYPES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
  <Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
  <Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>"#;

const MINIMAL_ROOT_RELS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const MINIMAL_WORKBOOK_RELS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

const MINIMAL_WORKBOOK: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;

const MINIMAL_SHARED_STRINGS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1">
  <si><t>Fuzz</t></si>
</sst>"#;

const MINIMAL_STYLES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
  <fills count="1"><fill><patternFill patternType="none"/></fill></fills>
  <borders count="1"><border><left/><right/><top/><bottom/></border></borders>
  <cellStyleXfs count="1"><xf/></cellStyleXfs>
  <cellXfs count="1"><xf/></cellXfs>
</styleSheet>"#;

const ODS_MIME: &[u8] = b"application/vnd.oasis.opendocument.spreadsheet";

const ODS_MANIFEST: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">
  <manifest:file-entry manifest:full-path="/" manifest:version="1.3" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/>
  <manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
  <manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>
</manifest:manifest>"#;

const MINIMAL_ODS_STYLES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" office:version="1.3">
</office:document-styles>"#;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Raw bytes import attempt
    if let Ok((wb, _)) = gridcraft_engine::io::open_bytes("fuzz.xlsx", data) {
        inspect_workbook(&wb, data);
    }
    if let Ok((wb, _)) = gridcraft_engine::io::open_bytes("fuzz.ods", data) {
        inspect_workbook(&wb, data);
    }

    // 2. Structured XLSX mode: embed mutated sheet/strings/workbook into valid ZIP
    let (part_choice, payload) = (data[0] % 6, &data[1..]);
    let mut sheet_xml = payload;
    let mut shared_strings = MINIMAL_SHARED_STRINGS;
    let mut workbook_xml = MINIMAL_WORKBOOK;
    let mut styles_xml = MINIMAL_STYLES;

    // Adversarial synthetic sheet fragments
    let extreme_cell_refs = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1"><c r="A1"><f>=A1</f></c><c r="B1"><f>=SUM(B1:B1)</f></c><c r="C1" t="s"><v>99999999</v></c></row>
    <row r="1048576"><c r="XFD1048576"><v>123</v></c></row>
    <row r="1048577"><c r="XFD1048577"><v>456</v></c></row>
    <row r="0"><c r="A0"><v>789</v></c></row>
    <row r="999999999"><c r="ZZZZ999999999"><v>999</v></c></row>
  </sheetData>
</worksheet>"#;

    let circular_formulas = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1"><f>=B1+1</f></c>
      <c r="B1"><f>=A1+1</f></c>
      <c r="C1"><f>=AVERAGE(C1:C10)</f></c>
    </row>
  </sheetData>
</worksheet>"#;

    match part_choice {
        0 => sheet_xml = payload,
        1 => shared_strings = payload,
        2 => workbook_xml = payload,
        3 => styles_xml = payload,
        4 => sheet_xml = extreme_cell_refs,
        _ => sheet_xml = circular_formulas,
    }

    let xlsx_zip = make_stored_zip(&[
        ("[Content_Types].xml", MINIMAL_CONTENT_TYPES),
        ("_rels/.rels", MINIMAL_ROOT_RELS),
        ("xl/workbook.xml", workbook_xml),
        ("xl/_rels/workbook.xml.rels", MINIMAL_WORKBOOK_RELS),
        ("xl/sharedStrings.xml", shared_strings),
        ("xl/styles.xml", styles_xml),
        ("xl/worksheets/sheet1.xml", sheet_xml),
    ]);

    if let Ok((wb, _)) = gridcraft_engine::io::open_bytes("structured.xlsx", &xlsx_zip) {
        inspect_workbook(&wb, data);
    }

    // 3. Structured ODS mode: embed mutated content.xml or styles.xml into valid ODS container
    let (ods_choice, ods_payload) = (data[0] % 4, &data[1..]);
    let mut content_xml = ods_payload;
    let mut ods_styles = MINIMAL_ODS_STYLES;

    let ods_huge_repeat = br#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
 <office:body>
  <office:spreadsheet>
   <table:table table:name="Sheet1">
    <table:table-row table:number-rows-repeated="100000000">
     <table:table-cell table:number-columns-repeated="4294967295" office:value-type="string">
      <text:p>RepeatMax</text:p>
     </table:table-cell>
    </table:table-row>
   </table:table>
  </office:spreadsheet>
 </office:body>
</office:document-content>"#;

    let ods_circular = br#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
 xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
 xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
 <office:body>
  <office:spreadsheet>
   <table:table table:name="Sheet1">
    <table:table-row>
     <table:table-cell table:formula="of:=Sheet1.A1" office:value-type="float" office:value="0">
      <text:p>0</text:p>
     </table:table-cell>
    </table:table-row>
   </table:table>
  </office:spreadsheet>
 </office:body>
</office:document-content>"#;

    match ods_choice {
        0 => content_xml = ods_payload,
        1 => ods_styles = ods_payload,
        2 => content_xml = ods_huge_repeat,
        _ => content_xml = ods_circular,
    }

    let ods_zip = make_stored_zip(&[
        ("mimetype", ODS_MIME),
        ("META-INF/manifest.xml", ODS_MANIFEST),
        ("content.xml", content_xml),
        ("styles.xml", ods_styles),
    ]);

    if let Ok((wb, _)) = gridcraft_engine::io::open_bytes("structured.ods", &ods_zip) {
        inspect_workbook(&wb, data);
    }

    // 4. Exercise column_name coordinate logic (apps/spreadeagle/src/book.rs)
    if data.len() >= 4 {
        let col = u32::from_le_bytes(data[..4].try_into().unwrap());
        let _ = column_name_safe(col);
    }
});

fn column_name_safe(col: u32) -> String {
    let mut n = match col.checked_add(1) {
        Some(v) => v,
        None => return String::new(),
    };
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn inspect_workbook(wb: &gridcraft_engine::model::Workbook, data: &[u8]) {
    let _ = wb.sheets.len();
    for sheet in wb.sheets.iter().take(3) {
        let _ = &sheet.name;

        // Viewport inspection as in spreadeagle UI (book.rs: shown, input, col_width, row_height)
        for r in 0..8 {
            for c in 0..8 {
                let coord = gridcraft_engine::core::CellRef::new(r, c);
                let val = sheet.value(coord);
                let text = gridcraft_engine::display::cell_text(wb, sheet, coord);
                let _right = matches!(val, gridcraft_engine::core::Value::Number(_));
                let _error = matches!(val, gridcraft_engine::core::Value::Error(_));
                let _input = sheet.input_text(coord);
                let _cw = sheet.col_width(c);
                let _rh = sheet.row_height(r);
                let _name = column_name_safe(c);
                let _ = text.len();
            }
        }

        // Fuzz-derived row and column querying
        if data.len() >= 8 {
            let r = u32::from_le_bytes(data[0..4].try_into().unwrap());
            let c = u32::from_le_bytes(data[4..8].try_into().unwrap());
            let coord = gridcraft_engine::core::CellRef::new(r, c);
            let _ = sheet.value(coord);
            let _ = gridcraft_engine::display::cell_text(wb, sheet, coord);
            let _ = sheet.input_text(coord);
            let _ = sheet.col_width(c);
            let _ = sheet.row_height(r);
            let _ = column_name_safe(c);
        }

        for (coord, cell) in sheet.cells.iter().take(32) {
            let _ = gridcraft_engine::display::cell_text(wb, sheet, coord);
            let _ = &cell.value;
            let _ = sheet.col_width(coord.col);
            let _ = sheet.row_height(coord.row);
            let _ = column_name_safe(coord.col);
        }
    }
}
