// SPDX-License-Identifier: GPL-3.0-only
//! Values drawn as terminal text: box-drawn tables with an index column, numbers to the
//! right, cells cut with `…` to fit the width, and colours by type.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::value::Value;
use crate::width::{str_width, truncate};

const RESET: &str = "\x1b[0m";
const HEADER: &str = "\x1b[1;32m";
const DIR: &str = "\x1b[1;34m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const MAGENTA: &str = "\x1b[35m";
const YELLOW: &str = "\x1b[33m";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align { Left, Right, Center }

#[derive(Clone)]
struct Cell {
    text: String,
    style: &'static str,
    align: Align,
}

struct Column {
    header: String,
    cells: Vec<Cell>,
    /// The index column keeps its width.
    fixed: bool,
}

fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '\n' || c == '\t' || c == '\r' { ' ' } else { c }).collect()
}

fn cell(v: &Value, dir_name: bool) -> Cell {
    let (style, align) = match v {
        Value::Int(_) | Value::Float(_) => ("", Align::Right),
        Value::Filesize(_) => (DIM, Align::Right),
        Value::Duration(_) => (YELLOW, Align::Right),
        Value::Date(_) => (MAGENTA, Align::Left),
        Value::Bool(_) => (CYAN, Align::Left),
        Value::Error(_) => (RED, Align::Left),
        Value::String(_) if dir_name => (DIR, Align::Left),
        Value::List(_) | Value::Record(_) | Value::Binary(_) => (DIM, Align::Left),
        _ => ("", Align::Left),
    };
    Cell { text: clean(&v.to_abbreviated()), style, align }
}

fn index_cell(i: usize) -> Cell { Cell { text: i.to_string(), style: HEADER, align: Align::Right } }

/// The value as the terminal shows it, `width` cells wide (0: no limit).
pub fn render(v: &Value, width: usize, color: bool) -> String {
    let mut out = match v {
        Value::Nothing => return String::new(),
        Value::String(s) => s.clone(),
        Value::Error(e) => if color { format!("{}error: {}{}", RED, e.message, RESET) } else { format!("error: {}", e.message) },
        Value::Binary(b) => hexdump(b),
        Value::List(items) if items.is_empty() => draw(&[Column { header: String::new(), cells: vec![Cell { text: String::from("empty list"), style: "", align: Align::Left }], fixed: false }], false, width, color),
        Value::List(_) | Value::Record(_) => draw(&columns_of(v), v.is_table(), width, color),
        other => other.to_text(),
    };
    if !out.is_empty() && !out.ends_with('\n') { out.push('\n'); }
    out
}

/// Plain aligned text, without borders, colours or a width limit (`to text`).
pub fn render_plain(v: &Value) -> String {
    let cols = columns_of(v);
    let cols: Vec<&Column> = cols.iter().filter(|c| !c.fixed).collect();
    let header = v.is_table();
    let widths: Vec<usize> = cols.iter().map(|c| {
        let h = if header { str_width(&c.header) } else { 0 };
        c.cells.iter().map(|x| str_width(&x.text)).max().unwrap_or(0).max(h)
    }).collect();
    let rows = cols.first().map(|c| c.cells.len()).unwrap_or(0);
    let mut out = String::new();
    let line = |cells: Vec<(&str, Align)>, out: &mut String| {
        let mut l = String::new();
        for (i, (t, a)) in cells.iter().enumerate() {
            if i > 0 { l.push_str("  "); }
            pad(&mut l, t, widths[i], *a);
        }
        out.push_str(l.trim_end());
        out.push('\n');
    };
    if header { line(cols.iter().map(|c| (c.header.as_str(), Align::Left)).collect(), &mut out); }
    for r in 0..rows { line(cols.iter().map(|c| (c.cells[r].text.as_str(), c.cells[r].align)).collect(), &mut out); }
    if out.ends_with('\n') { out.pop(); }
    out
}

fn columns_of(v: &Value) -> Vec<Column> {
    match v {
        Value::Record(r) => vec![
            Column { header: String::new(), cells: r.columns().iter().map(|k| Cell { text: clean(k), style: HEADER, align: Align::Left }).collect(), fixed: false },
            Column { header: String::new(), cells: r.values().iter().map(|x| cell(x, false)).collect(), fixed: false },
        ],
        Value::List(items) if v.is_table() => {
            let names = v.columns();
            let mut cols = vec![Column { header: String::from("#"), cells: (0..items.len()).map(index_cell).collect(), fixed: true }];
            for name in &names {
                let cells = items.iter().map(|row| {
                    let Value::Record(r) = row else { return cell(&Value::Nothing, false) };
                    let is_dir = name == "name" && matches!(r.get("type"), Some(Value::String(t)) if t == "dir");
                    cell(r.get(name).unwrap_or(&Value::Nothing), is_dir)
                }).collect();
                cols.push(Column { header: clean(name), cells, fixed: false });
            }
            cols
        }
        Value::List(items) => vec![
            Column { header: String::new(), cells: (0..items.len()).map(index_cell).collect(), fixed: true },
            Column { header: String::new(), cells: items.iter().map(|x| cell(x, false)).collect(), fixed: false },
        ],
        other => vec![Column { header: String::new(), cells: vec![cell(other, false)], fixed: false }],
    }
}

fn pad(out: &mut String, text: &str, w: usize, align: Align) {
    let tw = str_width(text);
    let gap = w.saturating_sub(tw);
    let (l, r) = match align {
        Align::Left => (0, gap),
        Align::Right => (gap, 0),
        Align::Center => (gap / 2, gap - gap / 2),
    };
    for _ in 0..l { out.push(' '); }
    out.push_str(text);
    for _ in 0..r { out.push(' '); }
}

fn draw(cols: &[Column], header: bool, width: usize, color: bool) -> String {
    let natural: Vec<usize> = cols.iter().map(|c| {
        let h = if header { str_width(&c.header) } else { 0 };
        c.cells.iter().map(|x| str_width(&x.text)).max().unwrap_or(0).max(h).max(1)
    }).collect();
    let total = |widths: &[usize], shown: usize, cut: bool| 1 + widths[..shown].iter().map(|w| w + 3).sum::<usize>() + if cut { 4 } else { 0 };
    // Narrows the widest columns, none below `floor` cells (or its natural width).
    let fit = |shown: usize, cut: bool, floor: usize| -> Option<Vec<usize>> {
        let mut widths = natural.clone();
        while total(&widths, shown, cut) > width {
            let widest = (0..shown).filter(|&i| !cols[i].fixed && widths[i] > natural[i].min(floor)).max_by_key(|&i| (widths[i], i))?;
            widths[widest] -= 1;
        }
        Some(widths)
    };
    let mut widths = natural.clone();
    let mut shown = cols.len();
    let mut cut = false;
    if width > 0 && total(&natural, shown, false) > width {
        // Keep columns readable: drop columns from the right before cutting cells short.
        let least = if cols.len() > 1 && cols[0].fixed { 2 } else { 1 }.min(cols.len());
        let mut found = None;
        for n in (least..=cols.len()).rev() {
            if let Some(w) = fit(n, n < cols.len(), 10) { found = Some((n, w)); break; }
        }
        let (n, w) = found.unwrap_or_else(|| {
            let n = least;
            (n, fit(n, n < cols.len(), 1).unwrap_or_else(|| natural.clone()))
        });
        shown = n;
        cut = n < cols.len();
        widths = w;
    }
    let paint = |out: &mut String, style: &str, text: &str| {
        if color && !style.is_empty() { out.push_str(style); out.push_str(text); out.push_str(RESET); }
        else { out.push_str(text); }
    };
    let rule = |out: &mut String, l: &str, m: &str, r: &str| {
        out.push_str(l);
        for i in 0..shown {
            if i > 0 { out.push_str(m); }
            for _ in 0..widths[i] + 2 { out.push('─'); }
        }
        if cut { out.push_str(m); out.push_str("───"); }
        out.push_str(r);
        out.push('\n');
    };
    let row = |out: &mut String, cells: &[Cell]| {
        out.push('│');
        for (i, c) in cells.iter().enumerate().take(shown) {
            out.push(' ');
            let text = truncate(&c.text, widths[i]);
            let mut padded = String::new();
            pad(&mut padded, &text, widths[i], c.align);
            let lead = padded.len() - padded.trim_start().len();
            let body = padded.trim();
            let trail = padded.len() - lead - body.len();
            for _ in 0..lead { out.push(' '); }
            paint(out, c.style, body);
            for _ in 0..trail { out.push(' '); }
            out.push_str(" │");
        }
        if cut { out.push_str(" … │"); }
        out.push('\n');
    };
    let mut out = String::new();
    rule(&mut out, "╭", "┬", "╮");
    if header {
        let cells: Vec<Cell> = cols.iter().map(|c| Cell { text: c.header.clone(), style: HEADER, align: Align::Center }).collect();
        row(&mut out, &cells);
        rule(&mut out, "├", "┼", "┤");
    }
    let rows = cols.first().map(|c| c.cells.len()).unwrap_or(0);
    for r in 0..rows {
        let cells: Vec<Cell> = cols.iter().map(|c| c.cells[r].clone()).collect();
        row(&mut out, &cells);
    }
    rule(&mut out, "╰", "┴", "╯");
    out
}

fn hexdump(b: &[u8]) -> String {
    let mut out = format!("binary, {} bytes\n", b.len());
    for (n, chunk) in b.chunks(16).take(16).enumerate() {
        out.push_str(&format!("{:08x}  ", n * 16));
        for i in 0..16 {
            match chunk.get(i) { Some(x) => out.push_str(&format!("{:02x} ", x)), None => out.push_str("   ") }
        }
        out.push(' ');
        for &x in chunk { out.push(if (0x20..0x7f).contains(&x) { x as char } else { '.' }); }
        out.push('\n');
    }
    if b.len() > 256 { out.push_str("…\n"); }
    out
}
