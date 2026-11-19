// SPDX-License-Identifier: GPL-3.0-only
//! Paths: joining with the current folder, `.` and `..`, and `*`/`?` patterns.

use alloc::string::String;
use alloc::vec::Vec;

/// `path` made absolute against `cwd`, with `~` for `home`, `.` and `..` resolved.
pub fn resolve(cwd: &str, home: &str, path: &str) -> String {
    let full: String = if path == "~" || path.starts_with("~/") {
        let mut s = String::from(home);
        s.push('/');
        s.push_str(&path[1..]);
        s
    } else if path.starts_with('/') {
        String::from(path)
    } else {
        let mut s = String::from(cwd);
        s.push('/');
        s.push_str(path);
        s
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in full.split('/') {
        match part {
            "" | "." => {}
            ".." => { parts.pop(); }
            p => parts.push(p),
        }
    }
    let mut out = String::from("/");
    out.push_str(&parts.join("/"));
    out
}

/// The path's folder and last part: `/a/b` gives (`/a`, `b`).
pub fn split(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(0) => ("/", &path[1..]),
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    }
}

/// `folder/name`, without doubling the `/`.
pub fn join(folder: &str, name: &str) -> String {
    let mut s = String::from(folder);
    if !s.ends_with('/') && !s.is_empty() { s.push('/'); }
    s.push_str(name);
    s
}

/// The extension, lower case: `notes.TXT` gives `txt`.
pub fn extension(path: &str) -> String {
    let name = split(path).1;
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

pub fn is_pattern(s: &str) -> bool { s.contains('*') || s.contains('?') }

/// Whether `name` matches a pattern of `*` (any run) and `?` (any one character).
pub fn matches(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) { pi += 1; ni += 1; }
        else if pi < p.len() && p[pi] == '*' { star = Some(pi); mark = ni; pi += 1; }
        else if let Some(s) = star { pi = s + 1; mark += 1; ni = mark; }
        else { return false; }
    }
    while pi < p.len() && p[pi] == '*' { pi += 1; }
    pi == p.len()
}
