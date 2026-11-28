// SPDX-License-Identifier: GPL-3.0-only
//! Void-inspired aliases and rich Linux terminal builtins (`ll`, `tree`, `bat`,
//! `fastfetch`/`fetch`, `which`, `alias`).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::host::{DirEntry, EntryKind, Host};
use crate::path;
use crate::signature::{self, COMMANDS, NAMESPACES};
use crate::value::Value;
use crate::width::{str_width, truncate};

const RST: &str = "\x1b[0m";
const DIM: &str = "\x1b[38;2;100;116;139m";
const ROSE: &str = "\x1b[1;38;2;244;63;94m";
const CYAN: &str = "\x1b[1;38;2;34;211;238m";
const BLUE: &str = "\x1b[1;38;2;96;165;250m";
const EMERALD: &str = "\x1b[1;38;2;16;185;129m";
const AMBER: &str = "\x1b[38;2;245;158;11m";
const LAVENDER: &str = "\x1b[38;2;192;132;252m";
const FROST: &str = "\x1b[38;2;236;239;244m";

/// Commands handled directly by Spindle / session host rather than external binaries.
pub const SHELL_EXTRA_CMDS: &[&str] = &[
    "alias", "attach", "bars", "btop", "clear", "colors", "exit", "export", "fm", "help",
    "history", "htop", "install", "ranger", "remove", "snapshot", "spin", "theme", "top",
    "unalias", "update", "vtcheck", "which", "write", "y", "yazi",
];

/// Default Void `.zshrc` / Linux quality-of-life aliases.
pub fn default_aliases() -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (k, v) in [
        ("..", "cd .."),
        ("...", "cd ../.."),
        ("la", "ls -a"),
        ("fetch", "fastfetch"),
        ("cls", "clear"),
        ("y", "yazi"),
        ("fm", "yazi"),
        ("ranger", "yazi"),
        ("htop", "btop"),
        ("top", "btop"),
        ("spin", "bars"),
    ] {
        map.insert(String::from(k), String::from(v));
    }
    map
}

/// Expands command-head aliases across statements and pipelines (ignoring quoted tokens,
/// comments, and `^`-forced externals). Recursion depth is bounded.
pub fn expand_aliases(line: &str, aliases: &BTreeMap<String, String>) -> String {
    if aliases.is_empty() || line.trim().is_empty() {
        return String::from(line);
    }
    let mut current = String::from(line);
    for _ in 0..4 {
        let next = expand_once(&current, aliases);
        if next == current {
            break;
        }
        current = next;
    }
    current
}

fn expand_once(src: &str, aliases: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut head = true;
    let mut after_let = 0u8;
    while i < src.len() {
        let ch = src[i..].chars().next().unwrap();
        if ch == '#' {
            out.push_str(&src[i..]);
            break;
        }
        if ch.is_whitespace() {
            out.push(ch);
            if ch == '\n' {
                head = true;
                after_let = 0;
            }
            i += ch.len_utf8();
            continue;
        }
        if matches!(ch, '|' | ';' | '(') {
            out.push(ch);
            head = true;
            after_let = 0;
            i += 1;
            continue;
        }
        if ch == '"' || ch == '\'' || ch == '`' {
            let start = i;
            i += 1;
            let mut esc = false;
            while i < src.len() {
                let c = src[i..].chars().next().unwrap();
                i += c.len_utf8();
                if esc {
                    esc = false;
                } else if c == '\\' && ch == '"' {
                    esc = true;
                } else if c == ch {
                    break;
                }
            }
            out.push_str(&src[start..i]);
            head = false;
            continue;
        }
        if head {
            let mut end = i;
            while end < bytes.len() {
                let c = src[end..].chars().next().unwrap();
                if c.is_whitespace() || matches!(c, '|' | ';' | '(' | ')' | '&' | '<' | '>' | '=' | '"' | '\'' | '`') {
                    break;
                }
                end += c.len_utf8();
            }
            let word = &src[i..end];
            if word == "let" {
                after_let = 1;
                head = false;
                out.push_str(word);
            } else if after_let == 1 {
                after_let = 2;
                out.push_str(word);
            } else if let Some(rep) = aliases.get(word).filter(|_| !word.starts_with('^')) {
                out.push_str(rep);
                head = false;
            } else {
                out.push_str(word);
                head = false;
            }
            i = end;
            continue;
        }
        if after_let == 2 && ch == '=' {
            after_let = 0;
            head = true;
            out.push(ch);
            i += 1;
            continue;
        }
        let mut end = i;
        while end < bytes.len() {
            let c = src[end..].chars().next().unwrap();
            if c.is_whitespace() || matches!(c, '|' | ';' | '(' | '"' | '\'' | '`') {
                break;
            }
            end += c.len_utf8();
        }
        out.push_str(&src[i..end]);
        i = end;
    }
    out
}

pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        let tenths = (bytes * 10 / 1024) % 10;
        let whole = bytes / 1024;
        if whole >= 100 || tenths == 0 {
            format!("{} KiB", whole)
        } else {
            format!("{}.{} KiB", whole, tenths)
        }
    } else if bytes < 1024 * 1024 * 1024 {
        let tenths = (bytes * 10 / (1024 * 1024)) % 10;
        let whole = bytes / (1024 * 1024);
        format!("{}.{} MiB", whole, tenths)
    } else {
        let tenths = (bytes * 10 / (1024 * 1024 * 1024)) % 10;
        let whole = bytes / (1024 * 1024 * 1024);
        format!("{}.{} GiB", whole, tenths)
    }
}

pub fn file_glyph(entry: &DirEntry) -> &'static str {
    if entry.kind == EntryKind::Dir {
        return "▸";
    }
    if entry.name.ends_with(".wasm") {
        return "⚙";
    }
    if entry.name.ends_with(".rs") || entry.name.ends_with(".py") || entry.name.ends_with(".lua") {
        return "◆";
    }
    if entry.name.ends_with(".json") || entry.name.ends_with(".toml") || entry.name.ends_with(".conf") {
        return "◈";
    }
    "·"
}

fn entry_perms(e: &DirEntry) -> &'static str {
    if e.kind == EntryKind::Dir {
        "drwxr-xr-x"
    } else if e.name.ends_with(".wasm") {
        "-rwxr-xr-x"
    } else {
        "-rw-r--r--"
    }
}

/// Detailed colorized `ll` / `ls -la` listing.
pub fn render_ll(targets: &[String], include_hidden: bool, host: &dyn Host, color: bool) -> Result<String, (String, String)> {
    let list: Vec<String> = if targets.is_empty() { alloc::vec![String::from(".")] } else { targets.to_vec() };
    let mut out = String::new();
    for (idx, t) in list.iter().enumerate() {
        let abs = path::resolve(&host.cwd(), &host.home(), t);
        let st = host.stat(&abs).map_err(|e| (t.clone(), e))?;
        let mut items = if st.kind == EntryKind::Dir {
            host.list_dir(&abs).map_err(|e| (t.clone(), e))?
        } else {
            alloc::vec![DirEntry { name: t.clone(), ..st }]
        };
        items.retain(|e| include_hidden || !e.name.starts_with('.'));
        items.sort_by(|a, b| (a.kind != EntryKind::Dir, &a.name).cmp(&(b.kind != EntryKind::Dir, &b.name)));
        if list.len() > 1 && st.kind == EntryKind::Dir {
            if idx > 0 {
                out.push('\n');
            }
            if color {
                out.push_str(&format!("{}{}:{}\n", BLUE, t, RST));
            } else {
                out.push_str(&format!("{}:\n", t));
            }
        }
        let header = format!("total {} entries", items.len());
        if color {
            out.push_str(&format!("{}{}{}\n", DIM, header, RST));
        } else {
            out.push_str(&header);
            out.push('\n');
        }
        for e in &items {
            let perms = entry_perms(e);
            let sz = if e.kind == EntryKind::Dir { String::from("-") } else { human_size(e.size) };
            let glyph = file_glyph(e);
            let is_dir = e.kind == EntryKind::Dir;
            let is_exec = e.name.ends_with(".wasm");
            let suffix = if is_dir { "/" } else { "" };
            if color {
                let p_col = if is_dir { CYAN } else { DIM };
                let n_col = if is_dir { BLUE } else if is_exec { EMERALD } else if e.name.starts_with('.') { DIM } else { FROST };
                let g_col = if is_dir { CYAN } else if is_exec { EMERALD } else { LAVENDER };
                out.push_str(&format!("{}{}  {}{:>9}{}  {}{} {}{}{}{}\n",
                    p_col, perms, AMBER, sz, RST, g_col, glyph, n_col, e.name, suffix, RST));
            } else {
                out.push_str(&format!("{}  {:>9}  {} {}{}\n", perms, sz, glyph, e.name, suffix));
            }
        }
    }
    Ok(out)
}

/// Box-drawing directory tree (`tree`).
pub fn render_tree(target: &str, max_depth: usize, show_all: bool, host: &dyn Host, color: bool) -> Result<String, String> {
    let abs = path::resolve(&host.cwd(), &host.home(), target);
    let st = host.stat(&abs)?;
    let mut out = String::new();
    let label = if target == "." { host.cwd() } else { String::from(target) };
    if color {
        out.push_str(&format!("{}▸ {}{}\n", BLUE, label, RST));
    } else {
        out.push_str(&format!("▸ {}\n", label));
    }
    let mut dirs = 0usize;
    let mut files = 0usize;
    if st.kind == EntryKind::Dir {
        walk_tree(&abs, "", 0, max_depth, show_all, host, color, &mut out, &mut dirs, &mut files)?;
    }
    let summary = format!("\n{} director{}, {} file{}", dirs, if dirs == 1 { "y" } else { "ies" }, files, if files == 1 { "" } else { "s" });
    if color {
        out.push_str(&format!("{}{}{}\n", DIM, summary, RST));
    } else {
        out.push_str(&summary);
        out.push('\n');
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn walk_tree(
    abs: &str,
    prefix: &str,
    depth: usize,
    max_depth: usize,
    show_all: bool,
    host: &dyn Host,
    color: bool,
    out: &mut String,
    dirs: &mut usize,
    files: &mut usize,
) -> Result<(), String> {
    if depth >= max_depth {
        return Ok(());
    }
    let mut entries = host.list_dir(abs)?;
    entries.retain(|e| show_all || !e.name.starts_with('.'));
    entries.sort_by(|a, b| (a.kind != EntryKind::Dir, &a.name).cmp(&(b.kind != EntryKind::Dir, &b.name)));
    let n = entries.len();
    for (idx, e) in entries.iter().enumerate() {
        let last = idx + 1 == n;
        let branch = if last { "└── " } else { "├── " };
        let child_prefix = if last { format!("{}    ", prefix) } else { format!("{}│   ", prefix) };
        let is_dir = e.kind == EntryKind::Dir;
        if is_dir {
            *dirs += 1;
        } else {
            *files += 1;
        }
        if color {
            let name_col = if is_dir { BLUE } else if e.name.ends_with(".wasm") { EMERALD } else { FROST };
            let meta = if is_dir { String::new() } else { format!(" {}({}){}", DIM, human_size(e.size), RST) };
            out.push_str(&format!("{}{}{}{} {}{}{}{}\n",
                DIM, prefix, branch, RST, name_col, e.name, if is_dir { "/" } else { "" }, meta));
        } else {
            let meta = if is_dir { String::new() } else { format!(" ({})", human_size(e.size)) };
            out.push_str(&format!("{}{}{}{}{}\n", prefix, branch, e.name, if is_dir { "/" } else { "" }, meta));
        }
        if is_dir {
            let child_abs = path::join(abs, &e.name);
            let _ = walk_tree(&child_abs, &child_prefix, depth + 1, max_depth, show_all, host, color, out, dirs, files);
        }
    }
    Ok(())
}

/// Syntax-tinted line-numbered file viewer (`bat` / `cat -n`).
pub fn render_bat(files: &[String], host: &dyn Host, cols: usize, color: bool) -> Result<String, (usize, String)> {
    let w = if cols == 0 { 80 } else { cols.max(36) };
    let mut out = String::new();
    for (idx, name) in files.iter().enumerate() {
        let abs = path::resolve(&host.cwd(), &host.home(), name);
        let data = host.read_file(&abs).map_err(|e| (idx, e))?;
        let text = match String::from_utf8(data) {
            Ok(s) => s,
            Err(e) => {
                let bytes = e.into_bytes();
                let hdr = format!("File: {}  ({}, binary)", name, human_size(bytes.len() as u64));
                push_bat_header(&mut out, &hdr, w, color);
                out.push_str(&crate::render::render(&Value::Binary(bytes), w, color));
                continue;
            }
        };
        let line_count = text.lines().count().max(1);
        let hdr = format!("{}  ({} lines, {})", name, line_count, human_size(text.len() as u64));
        push_bat_header(&mut out, &hdr, w, color);
        let ext = path::extension(name);
        let num_w = format!("{}", line_count).len().max(3);
        for (li, line) in text.lines().enumerate() {
            let tinted = if color { tint_source_line(line, &ext) } else { String::from(line) };
            if color {
                out.push_str(&format!("{}{:>width$} │{} {}\n", DIM, li + 1, RST, tinted, width = num_w));
            } else {
                out.push_str(&format!("{:>width$} │ {}\n", li + 1, tinted, width = num_w));
            }
        }
        if color {
            out.push_str(&format!("{}╰{}╯{}\n", DIM, "─".repeat(w.saturating_sub(2).min(76)), RST));
        } else {
            out.push_str(&format!("╰{}╯\n", "─".repeat(w.saturating_sub(2).min(76))));
        }
    }
    Ok(out)
}

fn push_bat_header(out: &mut String, title: &str, cols: usize, color: bool) {
    let bar_w = cols.saturating_sub(2).min(76);
    let trimmed = truncate(title, bar_w.saturating_sub(4));
    let fill = bar_w.saturating_sub(str_width(&trimmed) + 3);
    if color {
        out.push_str(&format!("{}╭─ {}{}{} {}╮{}\n", DIM, CYAN, trimmed, DIM, "─".repeat(fill), RST));
    } else {
        out.push_str(&format!("╭─ {} {}╮\n", trimmed, "─".repeat(fill)));
    }
}

/// Lightweight per-line syntax coloring for `bat` and `yazi` live preview.
pub fn tint_source_line(line: &str, ext: &str) -> String {
    let t = line.trim_start();
    if ext == "md" && t.starts_with('#') {
        return format!("{}{}{}", CYAN, line, RST);
    }
    if (ext == "rs" && t.starts_with("//"))
        || ((ext == "toml" || ext == "conf" || ext == "sh" || ext == "py") && t.starts_with('#'))
    {
        return format!("{}{}{}", DIM, line, RST);
    }
    let mut out = String::with_capacity(line.len() + 16);
    let mut chars = line.char_indices().peekable();
    while let Some((i, ch)) = chars.next() {
        if ch == '"' || ch == '\'' {
            let mut end = line.len();
            let mut esc = false;
            for (j, c) in chars.by_ref() {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == ch {
                    end = j + c.len_utf8();
                    break;
                }
            }
            out.push_str(AMBER);
            out.push_str(&line[i..end]);
            out.push_str(RST);
        } else if ch == '/' && line[i..].starts_with("//") && matches!(ext, "rs" | "js" | "ts" | "c" | "cpp") {
            out.push_str(DIM);
            out.push_str(&line[i..]);
            out.push_str(RST);
            break;
        } else if ch == '#' && matches!(ext, "toml" | "conf" | "sh" | "py") {
            out.push_str(DIM);
            out.push_str(&line[i..]);
            out.push_str(RST);
            break;
        } else if ch.is_ascii_alphabetic() || ch == '_' {
            let mut end = i + ch.len_utf8();
            while let Some(&(j, c)) = chars.peek() {
                if c.is_ascii_alphanumeric() || c == '_' {
                    end = j + c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            let w = &line[i..end];
            if is_code_keyword(w) {
                out.push_str(LAVENDER);
                out.push_str(w);
                out.push_str(RST);
            } else if matches!(w, "true" | "false" | "null" | "None" | "Some" | "Ok" | "Err" | "Self") {
                out.push_str(CYAN);
                out.push_str(w);
                out.push_str(RST);
            } else {
                out.push_str(w);
            }
        } else if ch.is_ascii_digit() {
            let mut end = i + 1;
            while let Some(&(j, c)) = chars.peek() {
                if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
                    end = j + c.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            out.push_str(LAVENDER);
            out.push_str(&line[i..end]);
            out.push_str(RST);
        } else {
            out.push(ch);
        }
    }
    out
}

fn is_code_keyword(w: &str) -> bool {
    matches!(
        w,
        "fn" | "let" | "mut" | "pub" | "struct" | "enum" | "impl" | "trait" | "use" | "mod"
            | "match" | "if" | "else" | "for" | "while" | "loop" | "return" | "break" | "continue"
            | "const" | "static" | "unsafe" | "async" | "await" | "def" | "class" | "import" | "from"
            | "in" | "and" | "or" | "not" | "function" | "export"
    )
}

/// Sculpted SexOS Noir Lips `fastfetch` / `neofetch` / `fetch` banner + live system telemetry.
pub fn render_fetch(host: &dyn Host, theme: &str, font: &str, shell_label: &str, color: bool) -> String {
    let sys = host.sys_info();
    let rec = match &sys {
        Value::Record(r) => Some(r),
        _ => None,
    };
    let os_name = rec.and_then(|r| r.get("system")).map(|v| v.to_text()).unwrap_or_else(|| String::from("SexOS"));
    let host_name = rec.and_then(|r| r.get("host")).map(|v| v.to_text()).unwrap_or_else(|| String::from("sexos-noir"));
    let cpus = rec.and_then(|r| match r.get("cpus") { Some(Value::Int(n)) => Some(*n), _ => None }).unwrap_or(1);
    let uptime_str = rec.and_then(|r| match r.get("uptime") { Some(Value::Duration(ns)) => Some(format_uptime(*ns)), _ => None }).unwrap_or_else(|| String::from("0m 0s"));
    let free_pages = rec.and_then(|r| match r.get("free_pages") { Some(Value::Int(n)) => Some(*n as u64), _ => None });
    let total_mem = rec.and_then(|r| match r.get("memory") { Some(Value::Filesize(n)) => Some(*n as u64), _ => None });
    let lent_pages = rec.and_then(|r| match r.get("lent_pages") { Some(Value::Int(n)) => Some(*n as u64), _ => None });
    let limit_pages = rec.and_then(|r| match r.get("limit_pages") { Some(Value::Int(n)) => Some(*n as u64), _ => None });
    let procs = host.processes().len();

    let mem_line = match (lent_pages, limit_pages, free_pages, total_mem) {
        (Some(lent), Some(limit), _, _) if limit > 0 => {
            let used = lent * 4096;
            let cap = limit * 4096;
            let pct = ((lent * 100) / limit).min(100) as usize;
            format!("{} / {} ({}%)", human_size(used), human_size(cap), pct)
        }
        (_, _, Some(free), _) => format!("{} free", human_size(free * 4096)),
        (_, _, _, Some(total)) => human_size(total),
        _ => String::from("managed"),
    };

    let art_plain: [&str; 9] = [
        "    ▄▄████▄▄   ▄▄████▄▄    ",
        "  ▄██████████▄██████████▄  ",
        " █████████████████████████ ",
        " ████▀▀▀███▀▀▀▀▀███▀▀▀████ ",
        "  ▀███▄▄▄  ░░░░░  ▄▄▄███▀  ",
        "   ▀███████████████████▀   ",
        "     ▀███████████████▀     ",
        "       ▀███████████▀       ",
        "          ▀▀▀▀▀▀▀          ",
    ];
    let info: [(&str, String); 9] = [
        ("Header", format!("root@{}", host_name)),
        ("OS", format!("{} 0.1.0-noir x86_64", os_name)),
        ("Kernel", String::from("SexOS Microkernel (PKU + PDX)")),
        ("Uptime", uptime_str),
        ("CPU", format!("{} core{} active", cpus, if cpus == 1 { "" } else { "s" })),
        ("Memory", mem_line),
        ("Shell", String::from(shell_label)),
        ("Term", format!("Spindle · {} · {}", font, theme)),
        ("Tasks", format!("{} domains/yarns", procs)),
    ];

    let mut out = String::new();
    for (idx, (art, (k, v))) in art_plain.iter().zip(info.iter()).enumerate() {
        if color {
            let art_col = match idx {
                0..=2 => "\x1b[38;2;244;63;94m",
                3..=5 => "\x1b[38;2;251;113;133m",
                _ => "\x1b[38;2;192;132;252m",
            };
            if idx == 0 {
                out.push_str(&format!("{}{}{}  {}{}{}\n", art_col, art, RST, ROSE, v, RST));
            } else {
                out.push_str(&format!("{}{}{}  {}{:<8}{} {}{}{}\n", art_col, art, RST, CYAN, k, RST, FROST, v, RST));
            }
        } else if idx == 0 {
            out.push_str(&format!("{}  {}\n", art, v));
        } else {
            out.push_str(&format!("{}  {:<8} {}\n", art, k, v));
        }
    }
    if color {
        let mut swatches = String::from("                             ");
        for i in 0..8 {
            swatches.push_str(&format!("\x1b[4{}m   ", i));
        }
        swatches.push_str("\x1b[0m\n                             ");
        for i in 0..8 {
            swatches.push_str(&format!("\x1b[10{}m   ", i));
        }
        swatches.push_str("\x1b[0m\n");
        out.push_str(&swatches);
    }
    out
}

fn format_uptime(ns: i64) -> String {
    let secs = (ns / 1_000_000_000).max(0) as u64;
    let mins = secs / 60;
    let hrs = mins / 60;
    if hrs > 0 {
        format!("{}h {}m {}s", hrs, mins % 60, secs % 60)
    } else {
        format!("{}m {}s", mins, secs % 60)
    }
}

/// Explains how a command name resolves (`which`).
pub fn resolve_which(name: &str, host: &dyn Host, aliases: &BTreeMap<String, String>, extra_cmds: &[&str]) -> String {
    if let Some(target) = aliases.get(name) {
        return format!("{}: aliased to '{}'\n", name, target);
    }
    if signature::lookup(name).is_some() || NAMESPACES.contains(&name) {
        return format!("{}: spindle-shell builtin\n", name);
    }
    if extra_cmds.contains(&name) {
        return format!("{}: Spindle terminal command\n", name);
    }
    if host.external_names().iter().any(|n| n == name) || host.is_external(name) {
        return format!("/bin/{}\n", name);
    }
    format!("{} not found\n", name)
}

/// Validates a shell command head (builtin, alias, terminal cmd, or known external).
pub fn is_known_command(name: &str, host: &dyn Host, aliases: &BTreeMap<String, String>, extra_cmds: &[&str]) -> bool {
    let clean = name.strip_prefix('^').unwrap_or(name);
    if clean.is_empty() {
        return false;
    }
    if signature::lookup(clean).is_some()
        || NAMESPACES.contains(&clean)
        || clean == "let"
        || clean == "which"
        || clean == "alias"
        || clean == "unalias"
        || clean == "export"
        || aliases.contains_key(clean)
        || extra_cmds.contains(&clean)
        || host.external_names().iter().any(|n| n == clean)
    {
        return true;
    }
    if clean.contains('/') || clean.ends_with(".wasm") {
        let abs = path::resolve(&host.cwd(), &host.home(), clean);
        return host.stat(&abs).is_ok();
    }
    let wasm = path::resolve(&host.cwd(), &host.home(), &format!("{}.wasm", clean));
    host.stat(&wasm).is_ok()
}

pub fn help_commands_count() -> usize {
    COMMANDS.len()
}
