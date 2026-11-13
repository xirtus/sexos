// SPDX-License-Identifier: GPL-3.0-only
//! What Settings shows and how keys change it, apart from the window: pages are
//! the schema's sections, a search over every label replaces the page while it has
//! text, and Left/Right step the chosen value within its range. Host-tested
//! (`scripts/settings_host_gate.sh`).
use sex_config::{Config, Key, Kind, COUNT, KEYS};

/// Accent colours Left/Right walk through (any `#rrggbb` in the file still works).
pub const ACCENTS: [(&str, i32); 8] = [
    ("Blue", 0x3D7EFF), ("Teal", 0x14B8A6), ("Green", 0x22C55E), ("Amber", 0xF59E0B),
    ("Orange", 0xF97316), ("Red", 0xEF4444), ("Pink", 0xEC4899), ("Violet", 0x8B5CF6),
];
pub const QUERY_MAX: usize = 24;
pub const PAGE_MAX: usize = 12;

/// Page names for people, by schema section.
pub fn page_title(section: &str) -> &str {
    match section {
        "appearance" => "Appearance", "desktop" => "Desktop", "display" => "Display",
        "keyboard" => "Keyboard", "sound" => "Sound", "time" => "Date & time",
        "session" => "Lock & idle", "setup" => "System", "open" => "Default apps", other => other,
    }
}

/// The schema's sections in order: the pages.
pub fn pages() -> ([&'static str; PAGE_MAX], usize) {
    let mut out = [""; PAGE_MAX];
    let mut n = 0;
    for k in KEYS {
        if n == 0 || out[n - 1] != k.section() { if n < PAGE_MAX { out[n] = k.section(); n += 1; } }
    }
    (out, n)
}

pub struct Model {
    pub values: Config,
    pub page: usize,
    pub row: usize,
    pub query: [u8; QUERY_MAX],
    pub query_len: usize,
    /// One line about the last change: saved, refused, or why.
    pub status: [u8; 64],
    pub status_len: usize,
    /// Typing a new lock password (F2), if under way.
    pub entry: Option<Entry>,
}

/// A new lock password, typed twice.
pub struct Entry { pub typed: [u8; 64], pub len: usize, pub first: [u8; 64], pub first_len: usize, pub confirming: bool }
impl Entry {
    fn new() -> Self { Entry { typed: [0; 64], len: 0, first: [0; 64], first_len: 0, confirming: false } }
    fn wipe(&mut self) { self.typed = [0; 64]; self.first = [0; 64]; self.len = 0; self.first_len = 0; }
}

/// What a key asks for.
pub enum Change { None, Set(u16, i32), Password([u8; 64], usize) }

impl Model {
    pub fn new(values: Config) -> Self {
        Model { values, page: 0, row: 0, query: [0; QUERY_MAX], query_len: 0, status: [0; 64], status_len: 0, entry: None }
    }
    pub fn searching(&self) -> bool { self.query_len > 0 }
    /// The rows shown now: schema indexes.
    pub fn rows(&self) -> ([usize; COUNT], usize) {
        let mut out = [0; COUNT];
        let mut n = 0;
        let (pages, _) = pages();
        for (i, k) in KEYS.iter().enumerate() {
            let shown = if self.searching() { matches(k, &self.query[..self.query_len]) } else { k.section() == pages[self.page] };
            if shown { out[n] = i; n += 1; }
        }
        (out, n)
    }
    pub fn selected(&self) -> Option<&'static Key> {
        let (rows, n) = self.rows();
        (self.row < n).then(|| &KEYS[rows[self.row]])
    }
    pub fn say(&mut self, text: &str) {
        let n = text.len().min(self.status.len());
        self.status[..n].copy_from_slice(&text.as_bytes()[..n]);
        self.status_len = n;
    }
    pub fn next_page(&mut self, forward: bool) {
        let (_, n) = pages();
        self.page = if forward { (self.page + 1) % n } else { (self.page + n - 1) % n };
        self.row = 0;
    }
    pub fn move_row(&mut self, down: bool) {
        let (_, n) = self.rows();
        if n == 0 { self.row = 0; return; }
        self.row = if down { (self.row + 1).min(n - 1) } else { self.row.saturating_sub(1) };
    }
    pub fn type_char(&mut self, c: u8) {
        if self.query_len < QUERY_MAX && c.is_ascii_graphic() || c == b' ' && self.query_len > 0 && self.query_len < QUERY_MAX {
            self.query[self.query_len] = c;
            self.query_len += 1;
            self.row = 0;
        }
    }
    pub fn backspace(&mut self) { self.query_len = self.query_len.saturating_sub(1); self.row = 0; }
    pub fn clear_search(&mut self) { self.query_len = 0; self.row = 0; }
    /// F2: start typing a new lock password.
    pub fn start_password(&mut self) { self.entry = Some(Entry::new()); }
    pub fn entering(&self) -> bool { self.entry.is_some() }
    /// A key while typing a password: a character, Backspace (8), Esc (27) or Enter (13).
    pub fn password_key(&mut self, c: u8) -> Change {
        let Some(e) = self.entry.as_mut() else { return Change::None };
        match c {
            27 => { e.wipe(); self.entry = None; self.say("Password not changed."); }
            8 => e.len = e.len.saturating_sub(1),
            13 if e.len == 0 => self.say("Type at least one character."),
            13 if !e.confirming => {
                e.first = e.typed; e.first_len = e.len; e.typed = [0; 64]; e.len = 0; e.confirming = true;
            }
            13 => {
                let same = e.len == e.first_len && e.typed[..e.len] == e.first[..e.len];
                let out = (e.typed, e.len);
                e.wipe();
                self.entry = None;
                if same { return Change::Password(out.0, out.1); }
                self.say("The two passwords differ. Press F2 to try again.");
            }
            c if (0x20..0x7F).contains(&c) && e.len < 64 => { e.typed[e.len] = c; e.len += 1; }
            _ => {}
        }
        Change::None
    }
    /// Left/Right (or Enter, as Right) on the chosen row.
    pub fn step(&self, forward: bool, big: bool) -> Change {
        let Some(k) = self.selected() else { return Change::None };
        let v = self.values.value(k.id);
        Change::Set(k.id, step_value(k, v, forward, big))
    }
}

/// Case-insensitive substring of the label or the TOML name.
fn matches(k: &Key, query: &[u8]) -> bool {
    let has = |hay: &str| hay.as_bytes().windows(query.len()).any(|w| w.eq_ignore_ascii_case(query));
    has(k.label) || has(k.name)
}

/// The value one step from `v`, staying in range.
pub fn step_value(k: &Key, v: i32, forward: bool, big: bool) -> i32 {
    match k.kind {
        Kind::Bool => 1 - v.clamp(0, 1),
        Kind::Int { min, max } => {
            let span = max - min;
            let unit = if k.name.ends_with("_min") { 1 } else if span >= 500 { 25 } else if span >= 50 { 5 } else { 1 };
            let d = if big { unit * 10 } else { unit };
            (if forward { v.saturating_add(d) } else { v.saturating_sub(d) }).clamp(min, max)
        }
        Kind::Choice(names) => {
            let n = names.len() as i32;
            if forward { (v + 1).rem_euclid(n) } else { (v - 1).rem_euclid(n) }
        }
        Kind::Color => {
            let at = ACCENTS.iter().position(|(_, c)| *c == v);
            let n = ACCENTS.len();
            let i = match (at, forward) { (Some(i), true) => (i + 1) % n, (Some(i), false) => (i + n - 1) % n, (None, _) => 0 };
            ACCENTS[i].1
        }
    }
}

/// A value as people read it.
pub fn show_value(k: &Key, v: i32, out: &mut [u8; 32]) -> usize {
    let mut w = Out { buf: out, at: 0 };
    match k.kind {
        Kind::Bool => w.put(if v != 0 { "On" } else { "Off" }),
        Kind::Int { .. } => w.int(v),
        Kind::Choice(names) => w.put(names.get(v as usize).copied().unwrap_or("?")),
        Kind::Color => {
            match ACCENTS.iter().find(|(_, c)| *c == v) { Some((name, _)) => w.put(name), None => w.put("Custom") }
            w.put(" #");
            for shift in [20, 16, 12, 8, 4, 0] { w.byte(b"0123456789abcdef"[(v >> shift & 0xF) as usize]); }
        }
    }
    w.at
}

struct Out<'a> { buf: &'a mut [u8], at: usize }
impl Out<'_> {
    fn byte(&mut self, b: u8) { if self.at < self.buf.len() { self.buf[self.at] = b; self.at += 1; } }
    fn put(&mut self, s: &str) { for b in s.bytes() { self.byte(b); } }
    fn int(&mut self, v: i32) {
        if v < 0 { self.byte(b'-'); }
        let mut d = [0u8; 10];
        let mut n = (v as i64).unsigned_abs();
        let mut i = d.len();
        loop { i -= 1; d[i] = b'0' + (n % 10) as u8; n /= 10; if n == 0 { break; } }
        for b in &d[i..] { self.byte(*b); }
    }
}

/// The whole window as text, into `out`; returns its length.
pub fn render(m: &Model, out: &mut [u8]) -> usize {
    let mut w = Out { buf: out, at: 0 };
    w.put("Settings    Tab: next page   type: search   F2: lock password   F11: close\n\n");
    if let Some(e) = &m.entry {
        w.put(if e.confirming { "Type the new lock password again: " } else { "New lock password: " });
        for _ in 0..e.len { w.byte(b'*'); }
        w.put("_\n\nEnter: next   Esc: cancel\n");
        for b in &m.status[..m.status_len] { w.byte(*b); }
        return w.at;
    }
    let (pages, n) = pages();
    if m.searching() {
        w.put("Search: ");
        for b in &m.query[..m.query_len] { w.byte(*b); }
        w.put("_   (Esc clears)\n");
    } else {
        for (i, p) in pages[..n].iter().enumerate() {
            if i == m.page { w.put("["); w.put(page_title(p)); w.put("] "); } else { w.put(page_title(p)); w.put("  "); }
        }
        w.byte(b'\n');
    }
    w.put("------------------------------------------------------------\n");
    let (rows, count) = m.rows();
    if count == 0 { w.put("  Nothing matches.\n"); }
    for (r, &i) in rows[..count].iter().enumerate() {
        let k = &KEYS[i];
        w.put(if r == m.row { "> " } else { "  " });
        let label_len = k.label.len().min(38);
        w.put(&k.label[..label_len]);
        for _ in label_len..40 { w.byte(b' '); }
        let mut value = [0u8; 32];
        let len = show_value(k, m.values.value(k.id), &mut value);
        if r == m.row { w.put("< "); }
        for b in &value[..len] { w.byte(*b); }
        if r == m.row { w.put(" >"); }
        w.byte(b'\n');
    }
    w.put("------------------------------------------------------------\n");
    w.put("Up/Down: choose   Left/Right: change   Shift: bigger steps\n");
    for b in &m.status[..m.status_len] { w.byte(*b); }
    w.at
}

#[cfg(test)]
mod tests {
    use super::*;
    use sex_config::id;

    #[test]
    fn pages_follow_the_schema() {
        let (p, n) = pages();
        assert_eq!(&p[..n], &["appearance", "desktop", "display", "keyboard", "sound", "time", "session", "setup", "open"]);
    }

    #[test]
    fn steps_stay_in_range() {
        let night = sex_config::key(id::NIGHT_LIGHT).unwrap();
        assert_eq!(step_value(night, 3, true, false), 3);
        assert_eq!(step_value(night, 0, false, false), 0);
        let delay = sex_config::key(id::REPEAT_DELAY).unwrap();
        assert_eq!(step_value(delay, 400, true, false), 425);
        assert_eq!(step_value(delay, 400, false, true), 150);
        let wall = sex_config::key(id::WALLPAPER).unwrap();
        assert_eq!(step_value(wall, 0, false, false), 4, "choices wrap");
        let accent = sex_config::key(id::ACCENT).unwrap();
        assert_eq!(step_value(accent, 0x3D7EFF, true, false), 0x14B8A6);
        assert_eq!(step_value(accent, 0x123456, true, false), 0x3D7EFF, "custom goes to the first");
        for k in KEYS {
            for v in [k.default, step_value(k, k.default, true, true), step_value(k, k.default, false, true)] {
                assert!(k.accepts(v), "{} {}", k.name, v);
            }
        }
    }

    #[test]
    fn search_finds_across_pages_and_selects() {
        let mut m = Model::new(Config::new());
        for c in b"repeat" { m.type_char(*c); }
        let (rows, n) = m.rows();
        assert_eq!(n, 2);
        assert_eq!(KEYS[rows[0]].id, id::REPEAT_DELAY);
        m.move_row(true);
        m.move_row(true);
        assert_eq!(m.selected().unwrap().id, id::REPEAT_RATE);
        match m.step(true, false) { Change::Set(k, v) => assert_eq!((k, v), (id::REPEAT_RATE, 30)), _ => panic!() }
        m.clear_search();
        for c in b"zzz" { m.type_char(*c); }
        assert!(m.selected().is_none());
        let mut buf = [0u8; 4096];
        let n = render(&m, &mut buf);
        assert!(core::str::from_utf8(&buf[..n]).unwrap().contains("Nothing matches."));
    }

    #[test]
    fn password_typed_twice() {
        let mut m = Model::new(Config::new());
        m.start_password();
        for c in b"s3cret" { m.password_key(*c); }
        m.password_key(8);
        m.password_key(b't');
        assert!(matches!(m.password_key(13), Change::None));
        let mut buf = [0u8; 4096];
        let n = render(&m, &mut buf);
        assert!(core::str::from_utf8(&buf[..n]).unwrap().contains("Type the new lock password again: _"));
        for c in b"s3cret" { m.password_key(*c); }
        match m.password_key(13) { Change::Password(p, n) => assert_eq!(&p[..n], b"s3cret"), _ => panic!("no password") }
        assert!(!m.entering());
        m.start_password();
        for c in b"a" { m.password_key(*c); }
        m.password_key(13);
        for c in b"b" { m.password_key(*c); }
        assert!(matches!(m.password_key(13), Change::None));
        assert!(core::str::from_utf8(&m.status[..m.status_len]).unwrap().contains("differ"));
        m.start_password();
        assert!(matches!(m.password_key(13), Change::None), "empty refused");
        m.password_key(27);
        assert!(!m.entering());
    }

    #[test]
    fn render_shows_the_page_and_the_choice() {
        let mut values = Config::new();
        values.set(id::WALLPAPER, 1).unwrap();
        let mut m = Model::new(values);
        m.next_page(true);
        m.say("Saved.");
        let mut buf = [0u8; 4096];
        let n = render(&m, &mut buf);
        let text = core::str::from_utf8(&buf[..n]).unwrap();
        assert!(text.contains("[Desktop]"), "{text}");
        assert!(text.contains("> Wallpaper"));
        assert!(text.contains("< dusk >"));
        assert!(text.ends_with("Saved."));
        m.next_page(false);
        m.next_page(false);
        assert_eq!(pages().0[m.page], "open");
    }
}
