// SPDX-License-Identifier: GPL-3.0-only
//! OpenDocument spreadsheets, both ways. GridCraft imports ODS data only (a formula arrives
//! as its saved result, and fonts and column widths are dropped) and cannot write ODS, so
//! Spreadeagle reads the formulas, fonts and widths itself ([`extras`]), turning OpenFormula
//! (`of:=SUM([.B2:.B4])`) into the spelling the engine takes (`=SUM(B2:B4)`), and writes
//! whole .ods files ([`write`]): every sheet's values, formulas, dates, percentages, bold,
//! italic, underline and column widths.
//! Format reference: ODF 1.3 part 3 section 9.1 (tables), 16 (styles) and part 4 (OpenFormula).
use quick_xml::events::{BytesStart, Event};
use gridcraft_engine::core::Value;
use std::io::Read;

/// A formula found in the file: sheet, row and column counted from 0, Excel spelling.
#[derive(Clone, Debug, PartialEq)]
pub struct Found { pub sheet: usize, pub row: u32, pub col: u32, pub formula: String }

/// A cell's font as the file styles it: sheet, row, column, (bold, italic, underline).
#[derive(Clone, Debug, PartialEq)]
pub struct Styled { pub sheet: usize, pub row: u32, pub col: u32, pub font: (bool, bool, bool) }

/// What GridCraft's ODS import leaves out: formulas, cell fonts, and column widths (sheet,
/// column, width in pixels at 96 per inch).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    pub formulas: Vec<Found>,
    pub fonts: Vec<Styled>,
    pub widths: Vec<(usize, u32, f32)>,
    /// Percentages with other than two decimal places (GridCraft imports `0.00%`): sheet,
    /// row, column, format code.
    pub percents: Vec<(usize, u32, u32, String)>,
}

/// Rows and columns past these are not walked (a repeat count can say a million).
const MAX_ROW: u32 = 1_048_576;
const MAX_COL: u32 = 16_384;

/// Formulas, fonts and column widths in `bytes`, an .ods file.
pub fn extras(bytes: &[u8]) -> Result<Extras, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut xml = String::new();
    zip.by_name("content.xml").map_err(|e| e.to_string())?.read_to_string(&mut xml).map_err(|e| e.to_string())?;
    let mut reader = quick_xml::Reader::from_str(&xml);
    let (mut sheet, mut row, mut col) = (None::<usize>, 0u32, 0u32);
    let mut row_repeat = 1u32;
    let mut out = Extras::default();
    // Automatic styles: cell styles' fonts and column styles' widths, by name.
    let mut fonts: std::collections::HashMap<String, (bool, bool, bool)> = Default::default();
    let mut widths: std::collections::HashMap<String, f32> = Default::default();
    let mut style: Option<String> = None;
    let mut column = 0u32;
    // Percentage data styles' decimal places, and the cell styles that use them.
    let mut places: std::collections::HashMap<String, usize> = Default::default();
    let mut percent_style: Option<String> = None;
    let mut cell_percent: std::collections::HashMap<String, usize> = Default::default();
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                match e.local_name().as_ref() {
                    b"style" if sheet.is_none() => {
                        style = attr(e, b"name");
                        if let (Some(name), Some(&d)) = (&style, attr(e, b"data-style-name").and_then(|d| places.get(&d))) { cell_percent.insert(name.clone(), d); }
                    }
                    b"percentage-style" => { percent_style = attr(e, b"name"); if empty { percent_style = None; } }
                    b"number" if percent_style.is_some() => if let Some(name) = &percent_style {
                        places.insert(name.clone(), number(e, b"decimal-places").min(10) as usize);
                    }
                    b"text-properties" => if let Some(name) = &style {
                        let bold = attr(e, b"font-weight").is_some_and(|w| w == "bold" || w.parse::<u32>().is_ok_and(|n| n >= 600));
                        let italic = attr(e, b"font-style").is_some_and(|s| s == "italic" || s == "oblique");
                        let underline = attr(e, b"text-underline-style").is_some_and(|u| u != "none");
                        if bold || italic || underline { fonts.insert(name.clone(), (bold, italic, underline)); }
                    }
                    b"table-column-properties" => if let (Some(name), Some(w)) = (&style, attr(e, b"column-width").as_deref().and_then(length_px)) {
                        widths.insert(name.clone(), w);
                    }
                    b"table" => { sheet = Some(sheet.map_or(0, |s| s + 1)); row = 0; column = 0; }
                    b"table-column" => {
                        let repeat = number(e, b"number-columns-repeated").max(1);
                        if let (Some(s), Some(w)) = (sheet, attr(e, b"style-name").and_then(|n| widths.get(&n).copied())) {
                            for c in column..column.saturating_add(repeat).min(MAX_WIDTHS) { out.widths.push((s, c, w)); }
                        }
                        column = column.saturating_add(repeat);
                    }
                    b"table-row" => {
                        col = 0;
                        row_repeat = number(e, b"number-rows-repeated").max(1);
                        if empty { row = row.saturating_add(row_repeat); }
                    }
                    b"table-cell" | b"covered-table-cell" => {
                        let repeat = number(e, b"number-columns-repeated").max(1);
                        let cells = || (row..row.saturating_add(row_repeat).min(MAX_ROW)).flat_map(move |r| (col..col.saturating_add(repeat).min(MAX_COL)).map(move |c| (r, c)));
                        if let (Some(s), Some(f)) = (sheet, attr(e, b"formula")) {
                            if let Some(formula) = to_excel(&f) {
                                for (r, c) in cells() { out.formulas.push(Found { sheet: s, row: r, col: c, formula: formula.clone() }); }
                            }
                        }
                        let style_name = attr(e, b"style-name");
                        if let (Some(s), Some(&d), true) = (sheet, style_name.as_ref().and_then(|n| cell_percent.get(n)), d_differs(&style_name, &cell_percent)) {
                            if u64::from(row_repeat) * u64::from(repeat) <= 4096 {
                                let code = if d == 0 { "0%".to_string() } else { format!("0.{}%", "0".repeat(d)) };
                                for (r, c) in cells() { out.percents.push((s, r, c, code.clone())); }
                            }
                        }
                        if let (Some(s), Some(font)) = (sheet, style_name.and_then(|n| fonts.get(&n).copied())) {
                            // A styled run of empty cells repeated down the sheet is not worth a million commands.
                            if u64::from(row_repeat) * u64::from(repeat) <= 4096 {
                                for (r, c) in cells() { out.fonts.push(Styled { sheet: s, row: r, col: c, font }); }
                            }
                        }
                        col = col.saturating_add(repeat);
                    }
                    _ => {}
                }
            }
            Event::End(ref e) if e.local_name().as_ref() == b"table-row" => row = row.saturating_add(row_repeat),
            Event::End(ref e) if e.local_name().as_ref() == b"style" => style = None,
            Event::End(ref e) if e.local_name().as_ref() == b"percentage-style" => percent_style = None,
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// Whether the cell style's percentage shows other than GridCraft's two places.
fn d_differs(style: &Option<String>, cell_percent: &std::collections::HashMap<String, usize>) -> bool {
    style.as_ref().and_then(|n| cell_percent.get(n)).is_some_and(|&d| d != 2)
}

/// Columns past this keep the default width (a trailing repeat often covers them all).
const MAX_WIDTHS: u32 = 1024;

/// `2.5cm`, `1in`, `72pt`, `25.4mm`, `96px` as pixels at 96 per inch.
fn length_px(s: &str) -> Option<f32> {
    let at = s.find(|c: char| c.is_ascii_alphabetic())?;
    let n: f32 = s[..at].trim().parse().ok()?;
    let per_inch = match &s[at..] { "in" => 1.0, "cm" => 2.54, "mm" => 25.4, "pt" => 72.0, "pc" => 6.0, "px" => 96.0, _ => return None };
    Some(n / per_inch * 96.0).filter(|w| w.is_finite() && *w > 0.0)
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
    let mut braces = 0u32;
    while let Some(c) = chars.next() {
        match c {
            // Inline arrays: OpenFormula separates columns with `;` and rows with `|`.
            '{' => { braces += 1; out.push('{'); }
            '}' => { braces = braces.saturating_sub(1); out.push('}'); }
            '|' if braces > 0 => out.push(';'),
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


/// `=SUM(B2:B4,Other!C1)` -> `of:=SUM([.B2:.B4];[Other.C1])`: the engine's spelling as
/// OpenFormula, which LibreOffice and other ODF programs read.
pub fn to_odf(formula: &str) -> String {
    let body = formula.strip_prefix('=').unwrap_or(formula);
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::from("of:=");
    let mut braces = 0u32;
    let mut i = 0;
    let word_char = |c: char| c.is_alphanumeric() || matches!(c, '$' | '_' | '.' | '\\');
    // The word starting at `from`, and where it ends.
    let word = |from: usize| -> (String, usize) {
        let mut end = from;
        while end < chars.len() && word_char(chars[end]) { end += 1; }
        (chars[from..end].iter().collect(), end)
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                out.push('"');
                i += 1;
                while i < chars.len() {
                    out.push(chars[i]);
                    if chars[i] == '"' {
                        if chars.get(i + 1) == Some(&'"') { out.push('"'); i += 1; } else { break; }
                    }
                    i += 1;
                }
                i += 1;
            }
            '{' => { braces += 1; out.push('{'); i += 1; }
            '}' => { braces = braces.saturating_sub(1); out.push('}'); i += 1; }
            ',' => { out.push(';'); i += 1; }
            ';' if braces > 0 => { out.push('|'); i += 1; }
            '\'' => {
                // A quoted sheet name: 'My sheet'!A1.
                let mut end = i + 1;
                while end < chars.len() {
                    if chars[end] == '\'' { if chars.get(end + 1) == Some(&'\'') { end += 2; continue; } break; }
                    end += 1;
                }
                let sheet: String = chars[i..=end.min(chars.len() - 1)].iter().collect();
                if chars.get(end + 1) == Some(&'!') {
                    let (first, next) = word(end + 2);
                    i = range_ref(&chars, &word, Some(&sheet), first, next, &mut out);
                } else {
                    out.push_str(&sheet);
                    i = end + 1;
                }
            }
            c if word_char(c) => {
                let (first, next) = word(i);
                if chars.get(next) == Some(&'!') {
                    let (cell, after) = word(next + 1);
                    i = range_ref(&chars, &word, Some(&first), cell, after, &mut out);
                } else if chars.get(next) == Some(&'(') {
                    // A function: Excel's future-function prefixes are not OpenFormula's.
                    out.push_str(first.trim_start_matches("_xlfn.").trim_start_matches("_xlws."));
                    i = next;
                } else if is_cell(&first) || (chars.get(next) == Some(&':') && (is_col(&first) || is_row(&first))) {
                    i = range_ref(&chars, &word, None, first, next, &mut out);
                } else {
                    out.push_str(&first);
                    i = next;
                }
            }
            _ => { out.push(c); i += 1; }
        }
    }
    out
}

/// Writes `[Sheet.A1:.B2]` for a reference whose first part `first` ends at `next`; takes a
/// following `:part` too. Returns where the reference ends.
fn range_ref(chars: &[char], word: &dyn Fn(usize) -> (String, usize), sheet: Option<&str>, first: String, next: usize, out: &mut String) -> usize {
    let whole = |p: &str| is_cell(p) || is_col(p) || is_row(p);
    if !whole(&first) {
        // Not a reference after all (a defined name on another sheet, say): leave it be.
        if let Some(s) = sheet { out.push_str(s); out.push('!'); }
        out.push_str(&first);
        return next;
    }
    out.push('[');
    if let Some(s) = sheet { out.push_str(s); }
    out.push('.');
    out.push_str(&first);
    let mut end = next;
    if chars.get(next) == Some(&':') {
        let (second, after) = word(next + 1);
        if whole(&second) {
            out.push_str(":.");
            out.push_str(&second);
            end = after;
        }
    }
    out.push(']');
    end
}

/// `A1`, `$B$7`: one cell.
fn is_cell(s: &str) -> bool {
    let s = s.strip_prefix('$').unwrap_or(s);
    let letters = s.chars().take_while(char::is_ascii_alphabetic).count();
    let rest = s[letters..].strip_prefix('$').unwrap_or(&s[letters..]);
    (1..=3).contains(&letters) && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) && !rest.starts_with('0')
}
/// `A`, `$AB`: a whole column, in `A:C`.
fn is_col(s: &str) -> bool {
    let s = s.strip_prefix('$').unwrap_or(s);
    (1..=3).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphabetic())
}
/// `3`, `$12`: a whole row, in `3:5`.
fn is_row(s: &str) -> bool {
    let s = s.strip_prefix('$').unwrap_or(s);
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) && !s.starts_with('0')
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 has no place for the other control characters.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// How a number is shown, as far as an ODS file needs to know.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Kind { Plain, Date, Percent(u8) }

/// Decimal places of a percentage format: `0%` 0, `0.00%` 2.
fn percent_places(code: &str) -> u8 {
    code.split_once('.').map_or(0, |(_, after)| after.chars().take_while(|&c| c == '0').count().min(10) as u8)
}

/// The workbook as an .ods file: every sheet, its values and formulas (with their results,
/// so a reader that does not recalculate shows them), dates and percentages, bold, italic
/// and underline, and column widths.
pub fn write(wb: &gridcraft_engine::model::Workbook) -> Result<Vec<u8>, String> {
    use gridcraft_engine::core::date::datetime_from_serial;
    use gridcraft_engine::model::Underline;
    use gridcraft_engine::numfmt::NumberFormat;
    use std::collections::BTreeMap;
    use std::fmt::Write as _;

    let mut cell_styles: BTreeMap<(bool, bool, bool, Kind), String> = BTreeMap::new();
    let mut col_styles: BTreeMap<u32, String> = BTreeMap::new();
    let mut body = String::new();
    for sheet in &wb.sheets {
        let _ = write!(body, "<table:table table:name=\"{}\">", escape(&sheet.name));
        let used = sheet.used_range();
        let cols = used.map_or(0, |r| r.end.col + 1);
        // Columns, with runs of one width written once.
        let mut c = 0;
        while c < cols.max(1) {
            let px = sheet.col_width(c).round().max(1.0) as u32;
            let mut run = 1;
            while c + run < cols && sheet.col_width(c + run).round().max(1.0) as u32 == px { run += 1; }
            let n = col_styles.len();
            let name = col_styles.entry(px).or_insert_with(|| format!("co{}", n + 1)).clone();
            let repeat = if run > 1 { format!(" table:number-columns-repeated=\"{run}\"") } else { String::new() };
            let _ = write!(body, "<table:table-column table:style-name=\"{name}\"{repeat}/>");
            c += run;
        }
        // Only the cells the sheet holds (and spilled array values), row by row: a lone value
        // at XFD1048576 must not walk seventeen billion empty cells.
        let mut occupied: BTreeMap<u32, std::collections::BTreeSet<u32>> = BTreeMap::new();
        for at in sheet.cells.iter().map(|(at, _)| at).chain(sheet.spill.keys().copied()) {
            occupied.entry(at.row).or_default().insert(at.col);
        }
        let mut next_row = 0u32;
        for (&r, in_row) in &occupied {
            let mut row = String::new();
            let mut next_col = 0u32;
            for &c in in_row {
                let at = gridcraft_engine::core::CellRef::new(r, c);
                let cell = sheet.cell(at);
                let style = wb.styles.get(sheet.style_id(at));
                let font = (style.font.bold, style.font.italic, style.font.underline != Underline::None);
                let value = sheet.value(at);
                let formula = cell.and_then(|x| x.formula.as_ref()).map(|f| format!("={}", f.text));
                if matches!(value, Value::Empty) && formula.is_none() && font == (false, false, false) { continue; }
                let blanks = c - next_col;
                if blanks > 0 { let _ = write!(row, "<table:table-cell table:number-columns-repeated=\"{blanks}\"/>"); }
                next_col = c + 1;
                let fmt = NumberFormat::parse(style.num_fmt.as_str());
                let kind = match value {
                    Value::Number(_) if fmt.is_date() => Kind::Date,
                    Value::Number(_) if fmt.is_percent() => Kind::Percent(percent_places(style.num_fmt.as_str())),
                    _ => Kind::Plain,
                };
                let mut open = String::from("<table:table-cell");
                if font != (false, false, false) || kind != Kind::Plain {
                    let n = cell_styles.len();
                    let name = cell_styles.entry((font.0, font.1, font.2, kind)).or_insert_with(|| format!("ce{}", n + 1));
                    let _ = write!(open, " table:style-name=\"{name}\"");
                }
                if let Some(f) = &formula { let _ = write!(open, " table:formula=\"{}\"", escape(&to_odf(f))); }
                let shown = gridcraft_engine::display::value_text(wb, sheet, at);
                match &value {
                    Value::Number(n) => match kind {
                        Kind::Date => match datetime_from_serial(wb.date_system, *n) {
                            Some(d) if d.hour + d.minute + d.second + d.milli == 0 => { let _ = write!(open, " office:value-type=\"date\" office:date-value=\"{:04}-{:02}-{:02}\"", d.year, d.month, d.day); }
                            Some(d) => { let _ = write!(open, " office:value-type=\"date\" office:date-value=\"{:04}-{:02}-{:02}T{:02}:{:02}:{:02}\"", d.year, d.month, d.day, d.hour, d.minute, d.second); }
                            None => { let _ = write!(open, " office:value-type=\"float\" office:value=\"{n}\""); }
                        },
                        Kind::Percent(_) => { let _ = write!(open, " office:value-type=\"percentage\" office:value=\"{n}\""); }
                        Kind::Plain => { let _ = write!(open, " office:value-type=\"float\" office:value=\"{n}\""); }
                    },
                    Value::Bool(b) => { let _ = write!(open, " office:value-type=\"boolean\" office:boolean-value=\"{b}\""); }
                    Value::Text(_) | Value::Error(_) | Value::Array(_) => { let _ = write!(open, " office:value-type=\"string\""); }
                    Value::Empty => {}
                }
                let text = match &value { Value::Text(t) => t.to_string(), Value::Empty => String::new(), _ => shown };
                if text.is_empty() {
                    let _ = write!(row, "{open}/>");
                } else {
                    let paragraphs: String = text.split('\n').map(|line| format!("<text:p>{}</text:p>", escape(line))).collect();
                    let _ = write!(row, "{open}>{paragraphs}</table:table-cell>");
                }
            }
            if row.is_empty() { continue; }
            let blank_rows = r - next_row;
            if blank_rows > 0 {
                let _ = write!(body, "<table:table-row table:number-rows-repeated=\"{blank_rows}\"><table:table-cell/></table:table-row>");
            }
            next_row = r + 1;
            let _ = write!(body, "<table:table-row>{row}</table:table-row>");
        }
        if next_row == 0 { body.push_str("<table:table-row><table:table-cell/></table:table-row>"); }
        body.push_str("</table:table>");
    }

    let mut styles = String::new();
    for (px, name) in &col_styles {
        let _ = write!(styles, "<style:style style:name=\"{name}\" style:family=\"table-column\"><style:table-column-properties style:column-width=\"{:.4}in\"/></style:style>", *px as f32 / 96.0);
    }
    for ((bold, italic, underline, kind), name) in &cell_styles {
        let data = match kind { Kind::Date => " style:data-style-name=\"N1\"".to_string(), Kind::Percent(d) => format!(" style:data-style-name=\"P{d}\""), Kind::Plain => String::new() };
        let _ = write!(styles, "<style:style style:name=\"{name}\" style:family=\"table-cell\" style:parent-style-name=\"Default\"{data}>");
        if *bold || *italic || *underline {
            styles.push_str("<style:text-properties");
            if *bold { styles.push_str(" fo:font-weight=\"bold\" style:font-weight-asian=\"bold\" style:font-weight-complex=\"bold\""); }
            if *italic { styles.push_str(" fo:font-style=\"italic\" style:font-style-asian=\"italic\" style:font-style-complex=\"italic\""); }
            if *underline { styles.push_str(" style:text-underline-style=\"solid\" style:text-underline-width=\"auto\" style:text-underline-color=\"font-color\""); }
            styles.push_str("/>");
        }
        styles.push_str("</style:style>");
    }

    const NS: &str = concat!(
        "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" ",
        "xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" ",
        "xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" ",
        "xmlns:table=\"urn:oasis:names:tc:opendocument:xmlns:table:1.0\" ",
        "xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" ",
        "xmlns:number=\"urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0\" ",
        "xmlns:of=\"urn:oasis:names:tc:opendocument:xmlns:of:1.2\" ",
        "office:version=\"1.3\"");
    let data_styles = concat!(
        "<number:date-style style:name=\"N1\"><number:year number:style=\"long\"/><number:text>-</number:text>",
        "<number:month number:style=\"long\"/><number:text>-</number:text><number:day number:style=\"long\"/></number:date-style>",
        "");
    let mut places: Vec<u8> = cell_styles.keys().filter_map(|k| if let Kind::Percent(d) = k.3 { Some(d) } else { None }).collect();
    places.sort_unstable();
    places.dedup();
    let data_styles: String = std::iter::once(data_styles.to_string()).chain(places.iter().map(|d| format!(
        "<number:percentage-style style:name=\"P{d}\"><number:number number:decimal-places=\"{d}\" number:min-decimal-places=\"{d}\" number:min-integer-digits=\"1\"/><number:text>%</number:text></number:percentage-style>"))).collect();
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {NS}><office:automatic-styles>{data_styles}{styles}</office:automatic-styles><office:body><office:spreadsheet>{body}</office:spreadsheet></office:body></office:document-content>");
    let styles_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {NS}><office:styles><style:default-style style:family=\"table-cell\"/><style:style style:name=\"Default\" style:family=\"table-cell\"/></office:styles></office:document-styles>");
    let meta = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" office:version=\"1.3\"><office:meta><meta:generator>Spreadeagle</meta:generator></office:meta></office:document-meta>");
    let manifest = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">",
        "<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.spreadsheet\"/>",
        "<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>",
        "<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>",
        "<manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/>",
        "</manifest:manifest>");

    use std::io::Write as _;
    use zip::write::SimpleFileOptions;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let err = |e: &dyn std::fmt::Display| e.to_string();
    // The media type comes first and uncompressed, so a reader can sniff it (ODF part 2, 3.3).
    zip.start_file("mimetype", SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).map_err(|e| err(&e))?;
    zip.write_all(b"application/vnd.oasis.opendocument.spreadsheet").map_err(|e| err(&e))?;
    let deflated = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in [("META-INF/manifest.xml", manifest), ("content.xml", content.as_str()), ("styles.xml", styles_xml.as_str()), ("meta.xml", meta.as_str())] {
        zip.start_file(name, deflated).map_err(|e| err(&e))?;
        zip.write_all(text.as_bytes()).map_err(|e| err(&e))?;
    }
    Ok(zip.finish().map_err(|e| err(&e))?.into_inner())
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
        assert_eq!(to_excel("of:={1;2|3;4}").as_deref(), Some("={1,2;3,4}"));
    }

    #[test]
    fn far_corner_writes_quickly() {
        let mut s = gridcraft_engine::Session::new();
        s.new_workbook();
        s.execute_headless("cell.set", serde_json::json!({ "cell": "XFD1048576", "input": "far" })).unwrap();
        s.execute_headless("cell.set", serde_json::json!({ "cell": "B2", "input": "=1+1" })).unwrap();
        let started = std::time::Instant::now();
        let bytes = write(&s.doc().unwrap().wb).unwrap();
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
        let found = extras(&bytes).unwrap().formulas;
        assert_eq!(found, [Found { sheet: 0, row: 1, col: 1, formula: "=1+1".into() }]);
    }

    #[test]
    fn excel_becomes_openformula_and_back() {
        assert_eq!(to_odf("=SUM(B2:B4)"), "of:=SUM([.B2:.B4])");
        assert_eq!(to_odf("=IF(A1>0,\"a,b\",Other!$C$3)"), "of:=IF([.A1]>0;\"a,b\";[Other.$C$3])");
        assert_eq!(to_odf("='My sheet'!A1:B2*2"), "of:=['My sheet'.A1:.B2]*2");
        assert_eq!(to_odf("=_xlfn.XLOOKUP(A1,A:A,3:3)"), "of:=XLOOKUP([.A1];[.A:.A];[.3:.3])");
        assert_eq!(to_odf("=LOG10(A1)+1.5E+3"), "of:=LOG10([.A1])+1.5E+3");
        assert_eq!(to_odf("={1,2;3,4}"), "of:={1;2|3;4}");
        assert_eq!(to_odf("=Rate*2"), "of:=Rate*2");
        for f in ["=SUM(B2:B4)", "=IF(A1>0,\"a,b\",Other!$C$3)", "='My sheet'!A1:B2*2", "=SUM(A:A)", "={1,2;3,4}", "=\"say \"\"hi\"\"\"&A1", "=AVERAGE($A$1:$A$9)/COUNT(B1:B9)"] {
            assert_eq!(to_excel(&to_odf(f)).as_deref(), Some(f), "{f}");
        }
    }

    #[test]
    fn finds_formulas_with_repeats() {
        let found = extras(include_bytes!("../assets/sample.ods")).unwrap().formulas;
        assert_eq!(found, [Found { sheet: 0, row: 4, col: 1, formula: "=SUM(B2:B4)".into() }]);
    }
}
