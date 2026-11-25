// SPDX-License-Identifier: GPL-3.0-only
//! OpenDocument spreadsheets, both ways. GridCraft imports ODS data only (a formula arrives
//! as its saved result, and fonts and column widths are dropped) and cannot write ODS, so
//! Spreadeagle reads the formulas, fonts and widths itself ([`extras`]), turning OpenFormula
//! (`of:=SUM([.B2:.B4])`) into the spelling the engine takes (`=SUM(B2:B4)`), and writes
//! whole .ods files ([`write`]): every sheet's values, formulas, dates, percentages, bold,
//! italic, underline, wrapped text and column widths.
//! Format reference: ODF 1.3 part 3 section 9.1 (tables), 16 (styles) and part 4 (OpenFormula).
use quick_xml::events::{BytesStart, Event};
use gridcraft_engine::core::Value;
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::{FilterCriterion, Validation, ValidationKind};
use std::io::Read;

/// A formula found in the file: sheet, row and column counted from 0, Excel spelling.
#[derive(Clone, Debug, PartialEq)]
pub struct Found { pub sheet: usize, pub row: u32, pub col: u32, pub formula: String }

/// A cell's font as the file styles it: sheet, row, column, (bold, italic, underline).
#[derive(Clone, Debug, PartialEq)]
pub struct Styled { pub sheet: usize, pub row: u32, pub col: u32, pub font: (bool, bool, bool) }

/// One ODF database range that can be represented by GridCraft's AutoFilter.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundFilter { pub sheet: usize, pub range: RangeRef, pub criteria: Vec<(u32, FilterCriterion)> }

/// What GridCraft's ODS import leaves out: formulas, cell fonts, and column widths (sheet,
/// column, width in pixels at 96 per inch).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    /// Original ODS names, in table order. The engine may shorten names for XLSX rules.
    pub sheet_names: Vec<String>,
    pub formulas: Vec<Found>,
    pub fonts: Vec<Styled>,
    pub wraps: Vec<(usize, u32, u32)>,
    pub widths: Vec<(usize, u32, f32)>,
    /// Percentages with other than two decimal places (GridCraft imports `0.00%`): sheet,
    /// row, column, format code.
    pub percents: Vec<(usize, u32, u32, String)>,
    /// List rules and their cell ranges, recovered from ODF content validations.
    pub validations: Vec<(usize, RangeRef, Validation)>,
    /// Frozen rows and columns from LibreOffice's per-sheet view settings.
    pub freezes: Vec<(usize, u32, u32)>,
    pub filters: Vec<FoundFilter>,
    /// ODF filter expressions outside the native AutoFilter model are not silently applied.
    pub filter_warnings: Vec<String>,
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
    let mut wraps: std::collections::HashSet<String> = Default::default();
    let mut widths: std::collections::HashMap<String, f32> = Default::default();
    let mut style: Option<String> = None;
    let mut column = 0u32;
    // Percentage data styles' decimal places, and the cell styles that use them.
    let mut places: std::collections::HashMap<String, usize> = Default::default();
    let mut percent_style: Option<String> = None;
    let mut cell_percent: std::collections::HashMap<String, usize> = Default::default();
    let mut validations: std::collections::HashMap<String, Validation> = Default::default();
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
                    b"table-cell-properties" => if let Some(name) = &style {
                        if attr(e, b"wrap-option").as_deref() == Some("wrap") { wraps.insert(name.clone()); }
                    }
                    b"table-column-properties" => if let (Some(name), Some(w)) = (&style, attr(e, b"column-width").as_deref().and_then(length_px)) {
                        widths.insert(name.clone(), w);
                    }
                    b"table" => {
                        sheet = Some(sheet.map_or(0, |s| s + 1)); row = 0; column = 0;
                        out.sheet_names.push(attr(e, b"name").unwrap_or_default());
                    }
                    b"content-validation" if sheet.is_none() => {
                        if let (Some(name), Some(condition)) = (attr(e, b"name"), attr(e, b"condition")) {
                            if let Some(rule) = list_rule(&condition, e) { validations.insert(name, rule); }
                        }
                    }
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
                        if let (Some(s), Some(name)) = (sheet, attr(e, b"content-validation-name")) {
                            if let Some(rule) = validations.get(&name) {
                                if row < MAX_ROW && col < MAX_COL {
                                    let end = CellRef::new(row.saturating_add(row_repeat).min(MAX_ROW) - 1,
                                        col.saturating_add(repeat).min(MAX_COL) - 1);
                                    out.validations.push((s, RangeRef::new(CellRef::new(row, col), end), rule.clone()));
                                }
                            }
                        }
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
                        if let (Some(s), Some(font)) = (sheet, style_name.as_ref().and_then(|n| fonts.get(n).copied())) {
                            // A styled run of empty cells repeated down the sheet is not worth a million commands.
                            if u64::from(row_repeat) * u64::from(repeat) <= 4096 {
                                for (r, c) in cells() { out.fonts.push(Styled { sheet: s, row: r, col: c, font }); }
                            }
                        }
                        if let (Some(s), Some(name)) = (sheet, style_name.as_ref()) {
                            if wraps.contains(name) && u64::from(row_repeat) * u64::from(repeat) <= 4096 {
                                for (r, c) in cells() { out.wraps.push((s, r, c)); }
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
    // View settings are optional metadata. A damaged settings.xml must not
    // discard formulas, formatting, and validations recovered from content.xml.
    out.freezes = read_freezes(&mut zip, &out.sheet_names).unwrap_or_default();
    let (filters, filter_warnings) = read_filters(&xml, &out.sheet_names)?;
    out.filters = filters;
    out.filter_warnings = filter_warnings;
    Ok(out)
}

#[derive(Clone, Debug)]
struct FilterCondition { field: u32, operator: String, value: String, data_type: String, items: Vec<String>, case_sensitive: bool, regex_enabled: bool }
#[derive(Clone, Debug)]
enum FilterExpr { Condition(FilterCondition), And(Vec<FilterExpr>), Or(Vec<FilterExpr>) }

fn odf_filter_range(address: &str, names: &[String]) -> Option<(usize, RangeRef)> {
    fn endpoint(input: &str) -> Option<(String, CellRef)> {
        let (name, cell) = input.rsplit_once('.')?;
        let name = name.strip_prefix('$').unwrap_or(name);
        let name = if name.starts_with('\'') && name.ends_with('\'') && name.len() >= 2 {
            name[1..name.len() - 1].replace("''", "'")
        } else { name.to_string() };
        Some((name, CellRef::parse(cell)?))
    }
    let (a, b) = address.split_once(':').unwrap_or((address, address));
    let (first_name, first) = endpoint(a)?;
    let (last_name, last) = endpoint(b)?;
    if first_name != last_name || first.row > last.row || first.col > last.col { return None; }
    let sheet = names.iter().position(|name| name == &first_name)?;
    Some((sheet, RangeRef::new(first, last)))
}

fn filter_condition(c: &FilterCondition) -> Option<FilterCriterion> {
    if c.case_sensitive { return None; }
    if !c.items.is_empty() {
        return Some(FilterCriterion::Values { values: c.items.iter().filter(|v| !v.is_empty()).cloned().collect(),
            blanks: c.items.iter().any(String::is_empty) });
    }
    if c.operator == "empty" { return Some(FilterCriterion::Values { values: Vec::new(), blanks: true }); }
    let ranked = match c.operator.as_str() {
        "top values" => Some((false, false)), "bottom values" => Some((true, false)),
        "top percent" => Some((false, true)), "bottom percent" => Some((true, true)), _ => None,
    };
    if let Some((bottom, percent)) = ranked {
        let count = c.value.parse::<u32>().ok()?;
        if count == 0 || count > if percent { 100 } else { 1_000_000 } { return None; }
        return Some(FilterCriterion::Top10 { bottom, percent, count });
    }
    let op = match c.operator.as_str() {
        "=" => "=", "!=" => "<>", "<" => "<", "<=" => "<=", ">" => ">", ">=" => ">=",
        "begins" => "beginsWith", "ends" => "endsWith", "contains" => "contains", "!contains" => "notContains",
        _ => return None,
    };
    // ODF enables regular expressions for string comparisons by default.
    // GridCraft's custom filter compares literal text, so only plain values
    // can be imported while that setting is active.
    if c.data_type == "text" && c.regex_enabled && c.value.bytes().any(|b| matches!(b,
        b'.' | b'^' | b'$' | b'*' | b'+' | b'?' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'|' | b'\\')) {
        return None;
    }
    let numeric_value = c.value.parse::<f64>().is_ok_and(f64::is_finite);
    // GridCraft infers numeric comparisons from the criterion, whereas ODF's
    // data-type is authoritative. Numeric equality would also match text "2"
    // in GridCraft, and a text comparison with "2" would hide text cells.
    if !matches!(c.data_type.as_str(), "text" | "number")
        || (matches!(op, "=" | "<>") && numeric_value)
        || (matches!(op, "<" | "<=" | ">" | ">=") && numeric_value != (c.data_type == "number"))
        || (matches!(op, "beginsWith" | "endsWith" | "contains" | "notContains") && c.data_type != "text") {
        return None;
    }
    // ODF equality is literal; GridCraft's equality treats * and ? as wildcards.
    if matches!(op, "=" | "<>") && c.value.contains(['*', '?']) { return None; }
    Some(FilterCriterion::Custom { a: (op.to_string(), c.value.clone()), b: None, and: true })
}

fn filter_criteria(expr: FilterExpr, width: u32) -> Option<Vec<(u32, FilterCriterion)>> {
    fn column(expr: FilterExpr) -> Option<(u32, FilterCriterion)> {
        match expr {
            FilterExpr::Condition(c) => Some((c.field, filter_condition(&c)?)),
            FilterExpr::And(mut parts) | FilterExpr::Or(mut parts) if parts.len() == 1 => column(parts.remove(0)),
            FilterExpr::And(parts) if parts.len() == 2 => paired(parts, true),
            FilterExpr::Or(parts) if parts.len() == 2 => paired(parts, false),
            _ => None,
        }
    }
    fn paired(parts: Vec<FilterExpr>, and: bool) -> Option<(u32, FilterCriterion)> {
        let mut parts = parts.into_iter();
        let FilterExpr::Condition(a) = parts.next()? else { return None };
        let FilterExpr::Condition(b) = parts.next()? else { return None };
        if a.field != b.field { return None; }
        let FilterCriterion::Custom { a: first, b: None, .. } = filter_condition(&a)? else { return None };
        let FilterCriterion::Custom { a: second, b: None, .. } = filter_condition(&b)? else { return None };
        Some((a.field, FilterCriterion::Custom { a: first, b: Some(second), and }))
    }
    // An AND of two conditions for the same column is one custom criterion;
    // an AND across columns is the worksheet's set of criteria.
    if let Some((field, criterion)) = column(expr.clone()) {
        return (field < width).then_some(vec![(field, criterion)]);
    }
    let terms = match expr { FilterExpr::And(terms) => terms, _ => return None };
    let mut out = Vec::new();
    for term in terms {
        let (field, criterion) = column(term)?;
        if field >= width || out.iter().any(|(old, _)| *old == field) { return None; }
        out.push((field, criterion));
    }
    Some(out)
}

fn read_filters(xml: &str, names: &[String]) -> Result<(Vec<FoundFilter>, Vec<String>), String> {
    let regex_enabled = odf_regex_enabled(xml)?;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut in_ranges = false;
    // Address, buttons, has filter, expressions, and whether ODF semantics
    // cannot be represented by GridCraft's header-row, row-oriented filter.
    let mut current: Option<(String, bool, bool, Vec<FilterExpr>, bool)> = None;
    let mut groups: Vec<(bool, Vec<FilterExpr>)> = Vec::new();
    let mut condition: Option<FilterCondition> = None;
    let (mut found, mut warnings) = (Vec::new(), Vec::new());
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                match e.local_name().as_ref() {
                    b"database-ranges" => in_ranges = true,
                    b"database-range" if in_ranges => {
                        current = Some((attr(e, b"target-range-address").unwrap_or_default(),
                            attr(e, b"display-filter-buttons").as_deref() == Some("true"), false, Vec::new(),
                            attr(e, b"contains-header").as_deref() == Some("false")
                                || attr(e, b"orientation").as_deref() == Some("column")));
                    }
                    b"filter" if current.is_some() => if let Some(c) = &mut current {
                        c.2 = true;
                        c.4 |= attr(e, b"display-duplicates").as_deref() == Some("false")
                            || attr(e, b"condition-source").is_some_and(|source| source != "self")
                            || attr(e, b"condition-source-range-address").is_some();
                        if attr(e, b"target-range-address").is_some() { c.0.clear(); }
                    },
                    b"filter-and" if current.is_some() => groups.push((true, Vec::new())),
                    b"filter-or" if current.is_some() => groups.push((false, Vec::new())),
                    b"filter-condition" if current.is_some() => {
                        condition = Some(FilterCondition { field: attr(e, b"field-number").and_then(|v| v.parse().ok()).unwrap_or(u32::MAX),
                            operator: attr(e, b"operator").unwrap_or_default(), value: attr(e, b"value").unwrap_or_default(),
                            data_type: attr(e, b"data-type").unwrap_or_else(|| "text".into()),
                            items: Vec::new(), case_sensitive: attr(e, b"case-sensitive").as_deref() == Some("true"), regex_enabled });
                    }
                    b"filter-set-item" if condition.is_some() => if let Some(c) = &mut condition {
                        c.items.push(attr(e, b"value").unwrap_or_default());
                    },
                    _ => {},
                }
                if empty { finish_filter_element(e.local_name().as_ref(), &mut current, &mut groups, &mut condition, names, &mut found, &mut warnings); }
            }
            Event::End(ref e) => finish_filter_element(e.local_name().as_ref(), &mut current, &mut groups, &mut condition, names, &mut found, &mut warnings),
            Event::Eof => break,
            _ => {},
        }
        if matches!(event, Event::End(ref e) if e.local_name().as_ref() == b"database-ranges") { in_ranges = false; }
    }
    Ok((found, warnings))
}

fn odf_regex_enabled(xml: &str) -> Result<bool, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut enabled = true; // ODF 1.3 table:use-regular-expressions default.
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(ref e) | Event::Empty(ref e) if e.local_name().as_ref() == b"calculation-settings" => {
                enabled = !matches!(attr(e, b"use-regular-expressions").as_deref(), Some("false" | "0"));
            }
            Event::Eof => return Ok(enabled),
            _ => {},
        }
    }
}

fn finish_filter_element(name: &[u8], current: &mut Option<(String, bool, bool, Vec<FilterExpr>, bool)>, groups: &mut Vec<(bool, Vec<FilterExpr>)>,
    condition: &mut Option<FilterCondition>, names: &[String], found: &mut Vec<FoundFilter>, warnings: &mut Vec<String>) {
    let push = |expr, current: &mut Option<(String, bool, bool, Vec<FilterExpr>, bool)>, groups: &mut Vec<(bool, Vec<FilterExpr>)>| {
        if let Some((_, children)) = groups.last_mut() { children.push(expr); }
        else if let Some(c) = current { c.3.push(expr); }
    };
    match name {
        b"filter-condition" => if let Some(c) = condition.take() { push(FilterExpr::Condition(c), current, groups); },
        b"filter-and" | b"filter-or" => if let Some((and, parts)) = groups.pop() {
            push(if and { FilterExpr::And(parts) } else { FilterExpr::Or(parts) }, current, groups);
        },
        b"database-range" => if let Some((address, buttons, has_filter, terms, unsupported)) = current.take() {
            if !buttons && !has_filter { return; }
            if unsupported {
                warnings.push("ODS AutoFilter uses no-header, column-oriented, unique-row, or cell-range conditions Spreadeagle cannot preserve".into());
                return;
            }
            let Some((sheet, range)) = odf_filter_range(&address, names) else {
                warnings.push("ODS AutoFilter range could not be represented".into()); return;
            };
            if found.iter().any(|f| f.sheet == sheet) { warnings.push("ODS has more than one AutoFilter on a sheet".into()); return; }
            let criteria = if terms.is_empty() { Some(Vec::new()) }
                else if terms.len() == 1 { filter_criteria(terms.into_iter().next().unwrap(), range.width()) }
                else { None }; // ODF requires an explicit group for mixed conditions.
            if let Some(criteria) = criteria { found.push(FoundFilter { sheet, range, criteria }); }
            else { warnings.push(format!("ODS AutoFilter on {} has conditions Spreadeagle cannot preserve", names[sheet])); }
        },
        _ => {},
    }
}

/// LibreOffice stores frozen panes in settings.xml, under a named Tables map.
/// Split mode 2 is fixed; mode 1 is a movable split and must not become a freeze.
fn read_freezes(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, names: &[String]) -> Result<Vec<(usize, u32, u32)>, String> {
    let mut xml = String::new();
    match zip.by_name("settings.xml") {
        Ok(mut f) => { f.read_to_string(&mut xml).map_err(|e| e.to_string())?; }
        Err(zip::result::ZipError::FileNotFound) => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    }
    let mut reader = quick_xml::Reader::from_str(&xml);
    let mut depth = 0usize;
    let (mut view_settings, mut views, mut view, mut tables) = (None, None, None, None);
    let (mut table, mut item, mut value) = (None::<(usize, usize)>, None::<(String, usize)>, String::new());
    let (mut hm, mut vm, mut hp, mut vp) = (0u32, 0u32, 0u32, 0u32);
    let mut out = Vec::new();
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(ref e) => {
                depth += 1;
                match e.local_name().as_ref() {
                    b"config-item-set" if attr(e, b"name").as_deref() == Some("ooo:view-settings") =>
                        view_settings = Some(depth),
                    b"config-item-map-indexed" if view_settings == Some(depth - 1)
                        && attr(e, b"name").as_deref() == Some("Views") => views = Some(depth),
                    b"config-item-map-entry" if views == Some(depth - 1) => view = Some(depth),
                    b"config-item-map-named" if view == Some(depth - 1)
                        && attr(e, b"name").as_deref() == Some("Tables") => tables = Some(depth),
                    b"config-item-map-entry" if tables == Some(depth - 1) => {
                        table = attr(e, b"name").and_then(|n| names.iter().position(|s| *s == n))
                            .map(|s| (s, depth));
                        (hm, vm, hp, vp) = (0, 0, 0, 0);
                    }
                    b"config-item" if table.is_some_and(|(_, d)| d == depth - 1) => {
                        item = attr(e, b"name").map(|name| (name, depth)); value.clear();
                    }
                    _ => {},
                }
            }
            Event::Text(e) if item.is_some() => {
                let decoded = e.decode().map_err(|e| e.to_string())?;
                value.push_str(&quick_xml::escape::unescape(&decoded).map_err(|e| e.to_string())?);
            }
            Event::End(ref e) => {
                if e.local_name().as_ref() == b"config-item" && item.as_ref().is_some_and(|(_, d)| *d == depth) {
                    let n = value.trim().parse::<u32>().unwrap_or(0);
                    match item.take().map(|(name, _)| name).as_deref() {
                        Some("HorizontalSplitMode") => hm = n,
                        Some("VerticalSplitMode") => vm = n,
                        Some("HorizontalSplitPosition") => hp = n.min(MAX_COL),
                        Some("VerticalSplitPosition") => vp = n.min(MAX_ROW),
                        _ => {},
                    }
                }
                if e.local_name().as_ref() == b"config-item-map-entry" {
                    if let Some((s, d)) = table {
                        if d == depth {
                            let (r, c) = (if vm == 2 { vp } else { 0 }, if hm == 2 { hp } else { 0 });
                            if r != 0 || c != 0 { out.push((s, r, c)); }
                            table = None;
                        }
                    }
                }
                if tables == Some(depth) { tables = None; }
                if view == Some(depth) { view = None; }
                if views == Some(depth) { views = None; }
                if view_settings == Some(depth) { view_settings = None; }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {},
        }
    }
    Ok(out)
}

fn list_rule(condition: &str, e: &BytesStart) -> Option<Validation> {
    let body = condition.strip_prefix("of:").unwrap_or(condition)
        .strip_prefix("cell-content-is-in-list(")?.strip_suffix(')')?;
    let f1 = if body.starts_with('[') {
        to_excel(&format!("of:={body}"))?.trim_start_matches('=').to_string()
    } else {
        let items: Vec<_> = body.split(';').map(|s| s.trim().strip_prefix('"')?.strip_suffix('"')).collect::<Option<_>>()?;
        format!("\"{}\"", items.join(",").replace('"', "\"\""))
    };
    Some(Validation {
        kind: ValidationKind::List,
        f1,
        allow_blank: attr(e, b"allow-empty-cell").as_deref() != Some("false"),
        in_cell_dropdown: attr(e, b"display-list").as_deref() != Some("none"),
        ..Validation::default()
    })
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
    // A colon inside a quoted sheet name is part of that name, not a range separator.
    let mut parts = Vec::new();
    let (mut start, mut quoted) = (0, false);
    let chars: Vec<(usize, char)> = inner.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (at, ch) = chars[i];
        if ch == '\'' {
            if quoted && chars.get(i + 1).is_some_and(|(_, next)| *next == '\'') { i += 1; }
            else { quoted = !quoted; }
        } else if ch == ':' && !quoted {
            parts.push(&inner[start..at]); start = at + 1;
        }
        i += 1;
    }
    parts.push(&inner[start..]);
    parts.into_iter().enumerate().map(|(i, part)| {
        let (sheet, cell) = match part.rfind('.') {
            Some(at) => (&part[..at], &part[at + 1..]),
            None => ("", part),
        };
        let sheet = sheet.trim_start_matches('$');
        if sheet.is_empty() || i > 0 { cell.to_string() } else { format!("{sheet}!{cell}") }
    }).collect::<Vec<_>>().join(":")
}

/// Rewrites sheet references after the ODS reader has normalized a sheet name. Only
/// references outside double-quoted formula strings are changed.
pub fn remap_sheet_names(formula: &str, source: &[String], imported: &[String]) -> String {
    let mut names = Vec::new();
    for (old, new) in source.iter().zip(imported).filter(|(a, b)| a != b) {
        let target = format!("'{}'!", new.replace('\'', "''"));
        names.push((format!("'{}'!", old.replace('\'', "''")), target.clone()));
        if old.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.')) {
            names.push((format!("{old}!"), target));
        }
    }
    if names.is_empty() { return formula.to_string(); }
    let mut out = String::new();
    let mut i = 0;
    let mut string = false;
    while i < formula.len() {
        let rest = &formula[i..];
        if !string {
            if let Some((old, new)) = names.iter().find(|(old, _)| rest.starts_with(old)
                && (i == 0 || !formula[..i].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.')))) {
                out.push_str(new); i += old.len(); continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch); i += ch.len_utf8();
        if ch == '"' {
            if string && formula[i..].starts_with('"') { out.push('"'); i += 1; }
            else { string = !string; }
        }
    }
    out
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

fn odf_filter_address(name: &str, range: RangeRef) -> String {
    fn cell(at: CellRef) -> String {
        let a1 = at.a1();
        let split = a1.find(|c: char| c.is_ascii_digit()).unwrap_or(a1.len());
        format!("${}${}", &a1[..split], &a1[split..])
    }
    let sheet = format!("$'{}'", name.replace('\'', "''"));
    format!("{sheet}.{}:{sheet}.{}", cell(range.start), cell(range.end))
}

fn odf_filter_condition(out: &mut String, field: u32, criterion: &FilterCriterion) -> Result<(), String> {
    use std::fmt::Write as _;
    let put_custom = |out: &mut String, op: &str, value: &str| -> Result<(), String> {
        let odf = match op { "=" => "=", "<>" => "!=", "<" => "<", "<=" => "<=", ">" => ">", ">=" => ">=",
            "beginsWith" => "begins", "endsWith" => "ends", "contains" => "contains", "notContains" => "!contains",
            _ => return Err("ODS cannot preserve this custom AutoFilter; save as XLSX".into()) };
        if matches!(op, "=" | "<>") && value.contains(['*', '?']) {
            return Err("ODS cannot preserve wildcard AutoFilter equality; save as XLSX".into());
        }
        let numeric_value = value.parse::<f64>().is_ok_and(f64::is_finite);
        if matches!(op, "=" | "<>") && numeric_value {
            return Err("ODS cannot preserve typed numeric AutoFilter equality; save as XLSX".into());
        }
        let kind = if matches!(op, "<" | "<=" | ">" | ">=") && numeric_value { "number" } else { "text" };
        let _ = write!(out, "<table:filter-condition table:field-number=\"{field}\" table:operator=\"{}\" table:value=\"{}\" table:data-type=\"{kind}\"/>",
            escape(odf), escape(value));
        Ok(())
    };
    match criterion {
        FilterCriterion::Values { values, blanks } => {
            if values.is_empty() && !blanks { return Err("ODS cannot preserve an empty AutoFilter choice; save as XLSX".into()); }
            let first = values.first().map(String::as_str).unwrap_or("");
            let _ = write!(out, "<table:filter-condition table:field-number=\"{field}\" table:operator=\"=\" table:value=\"{}\" table:data-type=\"text\">", escape(first));
            for value in values { let _ = write!(out, "<table:filter-set-item table:value=\"{}\"/>", escape(value)); }
            if *blanks { out.push_str("<table:filter-set-item table:value=\"\"/>"); }
            out.push_str("</table:filter-condition>");
        }
        FilterCriterion::Custom { a, b, and } => {
            if let Some(b) = b { out.push_str(if *and { "<table:filter-and>" } else { "<table:filter-or>" });
                put_custom(out, &a.0, &a.1)?; put_custom(out, &b.0, &b.1)?;
                out.push_str(if *and { "</table:filter-and>" } else { "</table:filter-or>" });
            } else { put_custom(out, &a.0, &a.1)?; }
        }
        FilterCriterion::Top10 { bottom, percent, count } => {
            let op = match (*bottom, *percent) { (false, false) => "top values", (true, false) => "bottom values",
                (false, true) => "top percent", (true, true) => "bottom percent" };
            let _ = write!(out, "<table:filter-condition table:field-number=\"{field}\" table:operator=\"{op}\" table:value=\"{count}\" table:data-type=\"number\"/>");
        }
        _ => return Err("ODS cannot preserve this AutoFilter condition; save as XLSX".into()),
    }
    Ok(())
}

fn odf_filters(wb: &gridcraft_engine::model::Workbook) -> Result<String, String> {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (sheet_index, sheet) in wb.sheets.iter().enumerate() {
        let Some(filter) = &sheet.autofilter else { continue };
        if out.is_empty() { out.push_str("<table:database-ranges>"); }
        let address = odf_filter_address(&sheet.name, filter.range);
        let _ = write!(out, "<table:database-range table:name=\"Spreadeagle_Filter_{}\" table:target-range-address=\"{}\" table:contains-header=\"true\" table:display-filter-buttons=\"true\">",
            sheet_index + 1, escape(&address));
        if !filter.criteria.is_empty() {
            out.push_str("<table:filter>");
            if filter.criteria.len() > 1 { out.push_str("<table:filter-and>"); }
            for (offset, criterion) in &filter.criteria {
                if *offset >= filter.range.width() { return Err("AutoFilter column is outside its range".into()); }
                odf_filter_condition(&mut out, *offset, criterion)?;
            }
            if filter.criteria.len() > 1 { out.push_str("</table:filter-and>"); }
            out.push_str("</table:filter>");
        }
        out.push_str("</table:database-range>");
    }
    if !out.is_empty() { out.push_str("</table:database-ranges>"); }
    Ok(out)
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

    let mut cell_styles: BTreeMap<(bool, bool, bool, bool, Kind), String> = BTreeMap::new();
    let mut col_styles: BTreeMap<u32, String> = BTreeMap::new();
    let mut body = String::new();
    let mut validation_defs = String::new();
    for (sheet_index, sheet) in wb.sheets.iter().enumerate() {
        let mut validation_cells: BTreeMap<(u32, u32), String> = BTreeMap::new();
        for (rule_index, rule) in sheet.validations.iter().enumerate() {
            if rule.kind != ValidationKind::List {
                return Err("ODS export currently supports list validation rules only; save as XLSX to keep this rule".into());
            }
            let name = format!("v{sheet_index}_{rule_index}");
            let source = if rule.f1.starts_with('"') {
                let items = rule.f1.trim_matches('"').replace("\"\"", "\"");
                items.split(',').map(|s| format!("\"{}\"", s.trim().replace('"', "\"\""))).collect::<Vec<_>>().join(";")
            } else { to_odf(&format!("={}", rule.f1)).trim_start_matches("of:=").to_string() };
            let condition = escape(&format!("of:cell-content-is-in-list({source})"));
            let allow = if rule.allow_blank { "true" } else { "false" };
            let display = if rule.in_cell_dropdown { "unsorted" } else { "none" };
            let _ = write!(validation_defs, "<table:content-validation table:name=\"{name}\" table:condition=\"{condition}\" table:allow-empty-cell=\"{allow}\" table:display-list=\"{display}\"/>");
            for range in &rule.ranges {
                if range.count() > 100_000 {
                    return Err("ODS export of a list validation over 100,000 cells is not supported yet; save as XLSX to keep this rule".into());
                }
                for r in range.start.row..=range.end.row {
                    for c in range.start.col..=range.end.col {
                        validation_cells.insert((r, c), name.clone());
                    }
                }
            }
        }
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
        for &(r, c) in validation_cells.keys() { occupied.entry(r).or_default().insert(c); }
        let mut next_row = 0u32;
        for (&r, in_row) in &occupied {
            let mut row = String::new();
            let mut next_col = 0u32;
            for &c in in_row {
                let at = gridcraft_engine::core::CellRef::new(r, c);
                let cell = sheet.cell(at);
                let style = wb.styles.get(sheet.style_id(at));
                let font = (style.font.bold, style.font.italic, style.font.underline != Underline::None);
                let wrap = style.align.wrap;
                let value = sheet.value(at);
                let formula = cell.and_then(|x| x.formula.as_ref()).map(|f| format!("={}", f.text));
                let validation = validation_cells.get(&(r, c));
                if matches!(value, Value::Empty) && formula.is_none() && font == (false, false, false) && !wrap && validation.is_none() { continue; }
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
                if let Some(name) = validation { let _ = write!(open, " table:content-validation-name=\"{name}\""); }
                if font != (false, false, false) || wrap || kind != Kind::Plain {
                    let n = cell_styles.len();
                    let name = cell_styles.entry((font.0, font.1, font.2, wrap, kind)).or_insert_with(|| format!("ce{}", n + 1));
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
    for ((bold, italic, underline, wrap, kind), name) in &cell_styles {
        let data = match kind { Kind::Date => " style:data-style-name=\"N1\"".to_string(), Kind::Percent(d) => format!(" style:data-style-name=\"P{d}\""), Kind::Plain => String::new() };
        let _ = write!(styles, "<style:style style:name=\"{name}\" style:family=\"table-cell\" style:parent-style-name=\"Default\"{data}>");
        if *wrap { styles.push_str("<style:table-cell-properties fo:wrap-option=\"wrap\"/>"); }
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
    let mut places: Vec<u8> = cell_styles.keys().filter_map(|k| if let Kind::Percent(d) = k.4 { Some(d) } else { None }).collect();
    places.sort_unstable();
    places.dedup();
    let data_styles: String = std::iter::once(data_styles.to_string()).chain(places.iter().map(|d| format!(
        "<number:percentage-style style:name=\"P{d}\"><number:number number:decimal-places=\"{d}\" number:min-decimal-places=\"{d}\" number:min-integer-digits=\"1\"/><number:text>%</number:text></number:percentage-style>"))).collect();
    let filters = odf_filters(wb)?;
    // ODF defaults regex comparisons on; GridCraft's exported filter values
    // are literal. Keep the saved filter's comparison meaning explicit.
    let filter_calculation_settings = if filters.is_empty() { "" } else {
        "<table:calculation-settings table:use-regular-expressions=\"false\" table:use-wildcards=\"false\"/>"
    };
    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {NS}><office:automatic-styles>{data_styles}{styles}</office:automatic-styles><office:body><office:spreadsheet>{filter_calculation_settings}<table:content-validations>{validation_defs}</table:content-validations>{body}{filters}</office:spreadsheet></office:body></office:document-content>");
    let styles_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {NS}><office:styles><style:default-style style:family=\"table-cell\"/><style:style style:name=\"Default\" style:family=\"table-cell\"/></office:styles></office:document-styles>");
    let meta = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" office:version=\"1.3\"><office:meta><meta:generator>Spreadeagle</meta:generator></office:meta></office:document-meta>");
    let mut settings = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><office:document-settings xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\" office:version=\"1.3\"><office:settings><config:config-item-set config:name=\"ooo:view-settings\"><config:config-item-map-indexed config:name=\"Views\"><config:config-item-map-entry><config:config-item-map-named config:name=\"Tables\">");
    for sheet in &wb.sheets {
        let (r, c) = sheet.freeze.unwrap_or((0, 0));
        let _ = write!(settings, "<config:config-item-map-entry config:name=\"{}\">", escape(&sheet.name));
        for (name, kind, n) in [("HorizontalSplitMode", "short", u32::from(c > 0) * 2), ("VerticalSplitMode", "short", u32::from(r > 0) * 2), ("HorizontalSplitPosition", "int", c), ("VerticalSplitPosition", "int", r)] {
            let _ = write!(settings, "<config:config-item config:name=\"{name}\" config:type=\"{kind}\">{n}</config:config-item>");
        }
        settings.push_str("</config:config-item-map-entry>");
    }
    settings.push_str("</config:config-item-map-named></config:config-item-map-entry></config:config-item-map-indexed></config:config-item-set></office:settings></office:document-settings>");
    let manifest = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\">",
        "<manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.spreadsheet\"/>",
        "<manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/>",
        "<manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/>",
        "<manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/>",
        "<manifest:file-entry manifest:full-path=\"settings.xml\" manifest:media-type=\"text/xml\"/>",
        "</manifest:manifest>");

    use std::io::Write as _;
    use zip::write::SimpleFileOptions;
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let err = |e: &dyn std::fmt::Display| e.to_string();
    // The media type comes first and uncompressed, so a reader can sniff it (ODF part 2, 3.3).
    zip.start_file("mimetype", SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).map_err(|e| err(&e))?;
    zip.write_all(b"application/vnd.oasis.opendocument.spreadsheet").map_err(|e| err(&e))?;
    let deflated = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in [("META-INF/manifest.xml", manifest), ("content.xml", content.as_str()), ("styles.xml", styles_xml.as_str()), ("meta.xml", meta.as_str()), ("settings.xml", settings.as_str())] {
        zip.start_file(name, deflated).map_err(|e| err(&e))?;
        zip.write_all(text.as_bytes()).map_err(|e| err(&e))?;
    }
    Ok(zip.finish().map_err(|e| err(&e))?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libreoffice_database_range_restores_value_choices_and_quoted_sheet() {
        let xml = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><office:body><office:spreadsheet>
            <table:database-ranges><table:database-range table:name="Filter1" table:target-range-address="$'Q1.Q2'.$A$1:$'Q1.Q2'.$B$5" table:contains-header="true" table:display-filter-buttons="true">
            <table:filter><table:filter-and><table:filter-condition table:field-number="0" table:operator="=" table:value="East" table:data-type="text"><table:filter-set-item table:value="East"/><table:filter-set-item table:value=""/></table:filter-condition>
            <table:filter-condition table:field-number="1" table:operator="top values" table:value="2" table:data-type="number"/></table:filter-and></table:filter></table:database-range></table:database-ranges>
            </office:spreadsheet></office:body></office:document-content>"#;
        let (filters, warnings) = read_filters(xml, &["Q1.Q2".into()]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].range.a1(), "A1:B5");
        assert_eq!(filters[0].criteria, vec![(0, FilterCriterion::Values { values: vec!["East".into()], blanks: true }),
            (1, FilterCriterion::Top10 { bottom: false, percent: false, count: 2 })]);
    }

    #[test]
    fn unsupported_filter_is_reported_instead_of_misapplied() {
        let xml = r#"<office:document-content><office:body><office:spreadsheet><table:database-ranges>
            <table:database-range table:target-range-address="$Sheet1.$A$1:$Sheet1.$B$3" table:display-filter-buttons="true"><table:filter>
            <table:filter-condition table:field-number="0" table:operator="match" table:value="^A.*"/>
            </table:filter></table:database-range></table:database-ranges></office:spreadsheet></office:body></office:document-content>"#;
        let (filters, warnings) = read_filters(xml, &["Sheet1".into()]).unwrap();
        assert!(filters.is_empty());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn filters_with_unrepresentable_odf_range_semantics_are_not_misapplied() {
        for (range_attr, filter_attr) in [
            ("table:contains-header=\"false\"", ""),
            ("table:orientation=\"column\"", ""),
            ("", "table:display-duplicates=\"false\""),
            ("", "table:condition-source=\"cell-range\" table:condition-source-range-address=\"$Sheet1.$D$1:$Sheet1.$D$2\""),
        ] {
            let xml = format!(r#"<office:document-content><office:body><office:spreadsheet>
                <table:database-ranges><table:database-range table:target-range-address="$Sheet1.$A$1:$Sheet1.$A$3"
                table:display-filter-buttons="true" {range_attr}><table:filter {filter_attr}>
                <table:filter-condition table:field-number="0" table:operator="=" table:value="East"/>
                </table:filter></table:database-range></table:database-ranges>
                </office:spreadsheet></office:body></office:document-content>"#);
            let (filters, warnings) = read_filters(&xml, &["Sheet1".into()]).unwrap();
            assert!(filters.is_empty(), "{range_attr} {filter_attr}");
            assert_eq!(warnings.len(), 1, "{range_attr} {filter_attr}");
        }
    }

    #[test]
    fn odf_typed_numeric_conditions_are_not_silently_reinterpreted() {
        for (operator, value, data_type) in [
            ("&gt;", "2", "text"), // GridCraft would filter text rows as numeric.
            ("=", "2", "number"), // GridCraft would also match a text cell "2".
        ] {
            let xml = format!(r#"<office:document-content><office:body><office:spreadsheet>
                <table:database-ranges><table:database-range table:target-range-address="$Sheet1.$A$1:$Sheet1.$A$3"
                table:display-filter-buttons="true"><table:filter>
                <table:filter-condition table:field-number="0" table:operator="{operator}"
                table:value="{value}" table:data-type="{data_type}"/>
                </table:filter></table:database-range></table:database-ranges>
                </office:spreadsheet></office:body></office:document-content>"#);
            let (filters, warnings) = read_filters(&xml, &["Sheet1".into()]).unwrap();
            assert!(filters.is_empty(), "{operator} {data_type}");
            assert_eq!(warnings.len(), 1, "{operator} {data_type}");
        }
        let mut output = String::new();
        assert!(odf_filter_condition(&mut output, 0, &FilterCriterion::Custom {
            a: ("=".into(), "2".into()), b: None, and: true,
        }).unwrap_err().contains("save as XLSX"));
    }

    #[test]
    fn regex_setting_controls_literal_text_filter_import_and_export() {
        let filter = r#"<table:database-ranges><table:database-range table:target-range-address="$Sheet1.$A$1:$Sheet1.$A$3"
            table:display-filter-buttons="true"><table:filter><table:filter-condition table:field-number="0"
            table:operator="=" table:value="A.B" table:data-type="text"/></table:filter>
            </table:database-range></table:database-ranges>"#;
        let xml = |setting: &str| format!("<office:document-content><office:body><office:spreadsheet>{setting}{filter}</office:spreadsheet></office:body></office:document-content>");
        let (filters, warnings) = read_filters(&xml(""), &["Sheet1".into()]).unwrap();
        assert!(filters.is_empty());
        assert_eq!(warnings.len(), 1);
        let setting = "<table:calculation-settings table:use-regular-expressions=\"false\"/>";
        let (filters, warnings) = read_filters(&xml(setting), &["Sheet1".into()]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(filters[0].criteria, vec![(0, FilterCriterion::Custom {
            a: ("=".into(), "A.B".into()), b: None, and: true,
        })]);

        let mut session = gridcraft_engine::Session::new();
        session.new_workbook();
        session.execute_headless("cell.set", serde_json::json!({ "cell": "A1", "input": "Name" })).unwrap();
        session.execute_headless("cell.set", serde_json::json!({ "cell": "A2", "input": "A.B" })).unwrap();
        session.execute_headless("data.filter", serde_json::json!({ "range": "A1:A2" })).unwrap();
        session.execute_headless("data.filterBy", serde_json::json!({ "column": "A", "custom": { "op": "=", "value": "A.B" } })).unwrap();
        let bytes = write(&session.doc().unwrap().wb).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let mut content = String::new();
        archive.by_name("content.xml").unwrap().read_to_string(&mut content).unwrap();
        assert!(content.contains("table:use-regular-expressions=\"false\""));
        let (saved, warnings) = read_filters(&content, &["Sheet1".into()]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(saved[0].criteria, filters[0].criteria);
    }

    fn zip_parts(parts: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for &(name, text) in parts {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(text.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn libreoffice_view_settings_distinguish_fixed_from_movable_splits() {
        use std::io::Write;
        let xml = r#"<office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"><office:settings><config:config-item-set config:name="ooo:view-settings"><config:config-item-map-indexed config:name="Views"><config:config-item-map-entry><config:config-item-map-named config:name="Tables"><config:config-item-map-entry config:name="Sheet1"><config:config-item config:name="HorizontalSplitMode" config:type="short">2</config:config-item><config:config-item config:name="HorizontalSplitPosition" config:type="int">4</config:config-item><config:config-item config:name="VerticalSplitMode" config:type="short">1</config:config-item><config:config-item config:name="VerticalSplitPosition" config:type="int">100</config:config-item></config:config-item-map-entry></config:config-item-map-named></config:config-item-map-entry></config:config-item-map-indexed></config:config-item-set></office:settings></office:document-settings>"#;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("settings.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        assert_eq!(read_freezes(&mut archive, &["Sheet1".into()]).unwrap(), vec![(0, 0, 4)]);
    }

    #[test]
    fn only_tables_in_view_settings_can_freeze_a_sheet() {
        let xml = r#"<office:document-settings xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:config="urn:oasis:names:tc:opendocument:xmlns:config:1.0"><office:settings>
            <config:config-item-set config:name="other"><config:config-item-map-named config:name="Other"><config:config-item-map-entry config:name="Sheet1"><config:config-item config:name="HorizontalSplitMode" config:type="short">2</config:config-item><config:config-item config:name="HorizontalSplitPosition" config:type="int">99</config:config-item></config:config-item-map-entry></config:config-item-map-named></config:config-item-set>
            <config:config-item-set config:name="ooo:view-settings"><config:config-item-map-indexed config:name="Views"><config:config-item-map-entry><config:config-item-map-named config:name="Tables"><config:config-item-map-entry config:name="Sheet1"><config:config-item config:name="HorizontalSplitMode" config:type="short">2</config:config-item><config:config-item config:name="HorizontalSplitPosition" config:type="int">4</config:config-item></config:config-item-map-entry></config:config-item-map-named></config:config-item-map-entry></config:config-item-map-indexed></config:config-item-set>
            </office:settings></office:document-settings>"#;
        let bytes = zip_parts(&[("settings.xml", xml)]);
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.as_slice())).unwrap();
        assert_eq!(read_freezes(&mut archive, &["Sheet1".into()]).unwrap(), vec![(0, 0, 4)]);
    }

    #[test]
    fn damaged_optional_settings_keeps_content_formulas() {
        let content = r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><office:body><office:spreadsheet><table:table table:name="Sheet1"><table:table-row><table:table-cell table:formula="of:=1+1"/></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#;
        let bytes = zip_parts(&[("content.xml", content), ("settings.xml", "<broken")]);
        let found = extras(&bytes).unwrap();
        assert_eq!(found.formulas, vec![Found { sheet: 0, row: 0, col: 0, formula: "=1+1".into() }]);
        assert!(found.freezes.is_empty());
    }

    #[test]
    fn openformula_becomes_excel() {
        assert_eq!(to_excel("of:=SUM([.B2:.B4])").as_deref(), Some("=SUM(B2:B4)"));
        assert_eq!(to_excel("of:=IF([.A1]>0;\"a;b\";[$Other.$C$3])").as_deref(), Some("=IF(A1>0,\"a;b\",Other!$C$3)"));
        assert_eq!(to_excel("of:=['My sheet'.A1:.B2]").as_deref(), Some("='My sheet'!A1:B2"));
        assert_eq!(to_excel("msoxl:=A1+1").as_deref(), Some("=A1+1"));
        assert_eq!(to_excel("of:=\"say \"\"hi\"\";\"").as_deref(), Some("=\"say \"\"hi\"\";\""));
        assert_eq!(to_excel("foo:=X()"), None);
        assert_eq!(to_excel("of:={1;2|3;4}").as_deref(), Some("={1,2;3,4}"));
        assert_eq!(to_excel("of:=['Q1:Q2'.A1]+1").as_deref(), Some("='Q1:Q2'!A1+1"));
        assert_eq!(remap_sheet_names("=IF(A1>0,\"'Q1:Q2'!\",'Q1:Q2'!A1)", &["Q1:Q2".into()], &["Q1Q2".into()]),
                   "=IF(A1>0,\"'Q1:Q2'!\",'Q1Q2'!A1)");
        assert_eq!(remap_sheet_names("=SUM(LongSheetName!A1,NotLongSheetName!A1)", &["LongSheetName".into()], &["Short".into()]),
                   "=SUM('Short'!A1,NotLongSheetName!A1)");
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
