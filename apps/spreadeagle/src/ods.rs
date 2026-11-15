// SPDX-License-Identifier: GPL-3.0-only
//! Formulas from an OpenDocument spreadsheet. GridCraft imports ODS data only (a formula
//! arrives as its saved result), so Spreadeagle reads the formulas itself and turns
//! OpenFormula (`of:=SUM([.B2:.B4])`) into the spelling the engine takes (`=SUM(B2:B4)`).
//! Format reference: ODF 1.3 part 3 section 9.1 (tables) and part 4 (OpenFormula).
use quick_xml::events::{BytesStart, Event};
use std::io::Read;

/// A formula found in the file: sheet, row and column counted from 0, Excel spelling.
#[derive(Clone, Debug, PartialEq)]
pub struct Found { pub sheet: usize, pub row: u32, pub col: u32, pub formula: String }

/// Rows and columns past these are not walked (a repeat count can say a million).
const MAX_ROW: u32 = 1_048_576;
const MAX_COL: u32 = 16_384;

/// Every formula in `bytes`, an .ods file.
pub fn formulas(bytes: &[u8]) -> Result<Vec<Found>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut xml = String::new();
    zip.by_name("content.xml").map_err(|e| e.to_string())?.read_to_string(&mut xml).map_err(|e| e.to_string())?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let (mut sheet, mut row, mut col) = (None::<usize>, 0u32, 0u32);
    let mut row_repeat = 1u32;
    let mut out = Vec::new();
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                match e.local_name().as_ref() {
                    b"table" => { sheet = Some(sheet.map_or(0, |s| s + 1)); row = 0; }
                    b"table-row" => {
                        col = 0;
                        row_repeat = number(e, b"number-rows-repeated").max(1);
                        if empty { row = row.saturating_add(row_repeat); }
                    }
                    b"table-cell" | b"covered-table-cell" => {
                        let repeat = number(e, b"number-columns-repeated").max(1);
                        if let (Some(s), Some(f)) = (sheet, attr(e, b"formula")) {
                            if let Some(formula) = to_excel(&f) {
                                for r in row..row.saturating_add(row_repeat).min(MAX_ROW) {
                                    for c in col..col.saturating_add(repeat).min(MAX_COL) {
                                        out.push(Found { sheet: s, row: r, col: c, formula: formula.clone() });
                                    }
                                }
                            }
                        }
                        col = col.saturating_add(repeat);
                    }
                    _ => {}
                }
            }
            Event::End(ref e) if e.local_name().as_ref() == b"table-row" => row = row.saturating_add(row_repeat),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

fn attr(e: &BytesStart, name: &[u8]) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.local_name().as_ref() == name).and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
}

fn number(e: &BytesStart, name: &[u8]) -> u32 { attr(e, name).and_then(|v| v.parse().ok()).unwrap_or(1) }

/// `of:=SUM([.B2:.B4];[Other.C1])` -> `=SUM(B2:B4,Other!C1)`. `None` for a namespace the
/// engine cannot read.
pub fn to_excel(formula: &str) -> Option<String> {
    let body = match formula.split_once(':') {
        Some(("of" | "oooc", rest)) => rest,
        Some(("msoxl", rest)) => return Some(rest.to_string()),
        Some((ns, _)) if ns.chars().all(|c| c.is_ascii_alphabetic()) && !ns.is_empty() && !formula.starts_with('=') => return None,
        _ => formula,
    };
    let body = body.strip_prefix('=').unwrap_or(body);
    let mut out = String::from("=");
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push('"');
                while let Some(s) = chars.next() {
                    out.push(s);
                    if s == '"' {
                        if chars.peek() == Some(&'"') { out.push(chars.next()?); } else { break; }
                    }
                }
            }
            '[' => {
                let mut inner = String::new();
                for s in chars.by_ref() { if s == ']' { break; } inner.push(s); }
                out.push_str(&reference(&inner));
            }
            ';' => out.push(','),
            _ => out.push(c),
        }
    }
    Some(out)
}

/// `.A1`, `$Sheet1.$A$1:.B2`, `'My sheet'.A1` -> `A1`, `Sheet1!$A$1:B2`, `'My sheet'!A1`.
fn reference(inner: &str) -> String {
    inner.split(':').enumerate().map(|(i, part)| {
        let (sheet, cell) = match part.rfind('.') {
            Some(at) => (&part[..at], &part[at + 1..]),
            None => ("", part),
        };
        let sheet = sheet.trim_start_matches('$');
        if sheet.is_empty() || i > 0 { cell.to_string() } else { format!("{sheet}!{cell}") }
    }).collect::<Vec<_>>().join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openformula_becomes_excel() {
        assert_eq!(to_excel("of:=SUM([.B2:.B4])").as_deref(), Some("=SUM(B2:B4)"));
        assert_eq!(to_excel("of:=IF([.A1]>0;\"a;b\";[$Other.$C$3])").as_deref(), Some("=IF(A1>0,\"a;b\",Other!$C$3)"));
        assert_eq!(to_excel("of:=['My sheet'.A1:.B2]").as_deref(), Some("='My sheet'!A1:B2"));
        assert_eq!(to_excel("msoxl:=A1+1").as_deref(), Some("=A1+1"));
        assert_eq!(to_excel("of:=\"say \"\"hi\"\";\"").as_deref(), Some("=\"say \"\"hi\"\";\""));
        assert_eq!(to_excel("foo:=X()"), None);
    }

    #[test]
    fn finds_formulas_with_repeats() {
        let found = formulas(include_bytes!("../assets/sample.ods")).unwrap();
        assert_eq!(found, [Found { sheet: 0, row: 4, col: 1, formula: "=SUM(B2:B4)".into() }]);
    }
}
