// SPDX-License-Identifier: GPL-3.0-only
//! Spindle's settings file (`spindle.conf` in its folder): `key = value` lines, `#`
//! comments. Unknown keys and bad values are reported, never fatal; what is missing
//! keeps its default. The window reads it again whenever it changes.
//!
//! ```text
//! theme = nord            # mocha latte gruvbox nord solarized-dark solarized-light
//!                         # tokyo-night dracula high-contrast
//! font = jetbrains-mono    # geist-mono (the default) or jetbrains-mono, with
//!                         # coding ligatures
//! font-size = 18          # pixels per row, 10 to 48
//! padding = 6             # pixels around each pane's text
//! scrollback = 10000      # lines kept per pane
//! background = #101010    # and foreground, cursor, selection, color0 .. color15
//! ssh.work = me@build.example.org:2222   # `ssh work` in a pane logs in there
//! ```

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub theme: String,
    /// One of [`FONT_NAMES`].
    pub font: String,
    pub font_size: usize,
    pub padding: usize,
    pub scrollback: usize,
    /// Colour overrides on top of the theme: (key, 0xRRGGBB), key one of `foreground`,
    /// `background`, `cursor`, `selection`, `color0`..`color15`.
    pub colors: Vec<(String, u32)>,
    /// SSH domains (`ssh.NAME = user@host[:port]`): `ssh NAME` in a pane logs in there.
    pub ssh: Vec<SshDomain>,
    /// Lines that were not understood, as messages.
    pub problems: Vec<String>,
}

/// A machine a pane can run on, by a short name of the person's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshDomain { pub name: String, pub user: String, pub host: String, pub port: u16 }

impl Config {
    /// The SSH domain called `name`.
    pub fn domain(&self, name: &str) -> Option<&SshDomain> { self.ssh.iter().find(|d| d.name == name) }
}

/// `user@host` or `user@host:port` (port 22 when left out).
fn ssh_target(v: &str) -> Option<(String, String, u16)> {
    let (user, rest) = v.split_once('@')?;
    let (host, port) = match rest.rsplit_once(':') { Some((h, p)) => (h, p.parse::<u16>().ok().filter(|p| *p != 0)?), None => (rest, 22) };
    let ok = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    (ok(user) && ok(host)).then(|| (String::from(user), String::from(host), port))
}

impl Default for Config {
    fn default() -> Config {
        Config { theme: String::from("mocha"), font: String::from("geist-mono"), font_size: 18, padding: 4, scrollback: 10_000, colors: Vec::new(), ssh: Vec::new(), problems: Vec::new() }
    }
}

pub const THEME_NAMES: [&str; 9] = ["mocha", "latte", "gruvbox", "nord", "solarized-dark", "solarized-light", "tokyo-night", "dracula", "high-contrast"];

pub const FONT_NAMES: [&str; 2] = ["geist-mono", "jetbrains-mono"];

fn color(v: &str) -> Option<u32> {
    let h = v.strip_prefix('#').or_else(|| v.strip_prefix("0x")).unwrap_or(v);
    if h.len() != 6 { return None; }
    u32::from_str_radix(h, 16).ok()
}

pub fn parse(text: &str) -> Config {
    let mut c = Config::default();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        // A colour value starts with '#', so only cut a comment after the value.
        let line = if raw.trim_start().starts_with('#') { "" } else if let Some((k, v)) = raw.split_once('=') {
            let v = v.trim();
            let v = match v.strip_prefix('#') { Some(rest) => { let end = rest.find(char::is_whitespace).unwrap_or(rest.len()); &v[..end + 1] } None => v.split('#').next().unwrap_or("").trim() };
            let _ = line;
            c.set(k.trim(), v, n + 1);
            continue;
        } else { line };
        if !line.is_empty() { c.problems.push(format!("line {}: expected key = value", n + 1)); }
    }
    c
}

impl Config {
    fn set(&mut self, key: &str, v: &str, line: usize) {
        let num = |lo: usize, hi: usize| v.parse::<usize>().ok().filter(|x| (lo..=hi).contains(x));
        let bad = |what: &str| format!("line {}: {} {}", line, key, what);
        match key {
            "theme" => {
                let want: String = v.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect();
                match THEME_NAMES.iter().find(|t| t.chars().filter(|c| c.is_ascii_alphanumeric()).eq(want.chars())) {
                    Some(t) => self.theme = String::from(*t),
                    None => self.problems.push(bad("is not a theme")),
                }
            }
            "font" => {
                let want: String = v.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect();
                match FONT_NAMES.iter().find(|t| t.chars().filter(|c| c.is_ascii_alphanumeric()).eq(want.chars())) {
                    Some(t) => self.font = String::from(*t),
                    None => self.problems.push(bad("must be geist-mono or jetbrains-mono")),
                }
            }
            "font-size" => match num(10, 48) { Some(x) => self.font_size = x, None => self.problems.push(bad("must be 10 to 48")) },
            "padding" => match num(0, 64) { Some(x) => self.padding = x, None => self.problems.push(bad("must be 0 to 64")) },
            "scrollback" => match num(0, 1_000_000) { Some(x) => self.scrollback = x, None => self.problems.push(bad("must be 0 to 1000000")) },
            "foreground" | "background" | "cursor" | "selection" => match color(v) { Some(x) => self.colors.push((String::from(key), x)), None => self.problems.push(bad("must be #rrggbb")) },
            k if k.strip_prefix("color").and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| n < 16) => match color(v) {
                Some(x) => self.colors.push((String::from(k), x)),
                None => self.problems.push(bad("must be #rrggbb")),
            },
            k if k.starts_with("ssh.") => {
                let name = &k[4..];
                let named = !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
                match ssh_target(v).filter(|_| named) {
                    Some((user, host, port)) => {
                        self.ssh.retain(|d| d.name != name);
                        self.ssh.push(SshDomain { name: String::from(name), user, host, port });
                    }
                    None => self.problems.push(bad("must be user@host or user@host:port")),
                }
            }
            _ => self.problems.push(format!("line {}: unknown setting {}", line, key)),
        }
    }
}

/// `text` with its `key = …` line set to `value` (added at the end when missing).
pub fn with_setting(text: &str, key: &str, value: &str) -> String {
    let mut out = String::new();
    let mut done = false;
    for l in text.lines() {
        if !done && l.split_once('=').is_some_and(|(k, _)| k.trim() == key) && !l.trim_start().starts_with('#') {
            out.push_str(&format!("{} = {}\n", key, value));
            done = true;
        } else { out.push_str(l); out.push('\n'); }
    }
    if !done { out.push_str(&format!("{} = {}\n", key, value)); }
    out
}
