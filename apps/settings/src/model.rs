// SPDX-License-Identifier: GPL-3.0-only
//! What Settings shows and how keys change it, apart from the window: pages are
//! the schema's sections, a search over every label replaces the page while it has
//! text, and Left/Right step the chosen value within its range. Host-tested
//! (`scripts/settings_host_gate.sh`).
use sex_config::{Config, Key, Kind, COUNT, KEYS};

/// Accent colours Left/Right walk through (any `#rrggbb` in the file still works).
pub const ACCENTS: [(&str, i32); 9] = [
    ("SexOS pink", 0xD6336C), ("Blue", 0x3D7EFF), ("Teal", 0x14B8A6), ("Green", 0x22C55E), ("Amber", 0xF59E0B),
    ("Orange", 0xF97316), ("Red", 0xEF4444), ("Pink", 0xEC4899), ("Violet", 0x8B5CF6),
];
pub const QUERY_MAX: usize = 24;
pub const PAGE_MAX: usize = 20;
/// The Privacy page's program rows as text, one line each (`grants.rs` writes them).
pub const EXTRA_MAX: usize = 2048;
/// The page that shows program rows after its settings.
pub const PRIVACY: &str = "privacy";
/// The page that shows what this computer is after its settings (`about.rs`).
pub const ABOUT: &str = "setup";
/// Pages with no settings, after the schema's: what Files knows (`facts.rs`).
pub const INFO_PAGES: [&str; 2] = ["updates", "storage"];
/// About's, Updates' or Storage's lines.
pub const INFO_MAX: usize = 2048;
/// Rows a page or a search can show: every key, and every item of a set.
pub const ROWS_MAX: usize = COUNT + 96;

/// Page names for people, by schema section.
pub fn page_title(section: &str) -> &str {
    match section {
        "appearance" => "Appearance", "desktop" => "Desktop", "display" => "Display",
        "keyboard" => "Keyboard", "sound" => "Sound", "time" => "Date & time",
        "session" => "Lock & idle", "setup" => "About", "open" => "Default apps",
        "startup" => "Startup programs", "shortcuts" => "Shortcuts", "notifications" => "Notifications",
        "accessibility" => "Accessibility",
        "privacy" => "Privacy and grants", "bluetooth" => "Bluetooth", "updates" => "Updates", "storage" => "Storage", other => other,
    }
}

/// A line above a page's rows that says how its settings work, or nothing.
pub fn page_note(section: &str) -> &'static str {
    match section {
        "startup" => "  The Shell starts these programs once you are logged in (Right: yes, Left: no).\n\n",
        "shortcuts" => "  Super and this key. \"default\" keeps the key in brackets; a chosen key replaces it.\n\n",
        "notifications" => "  The Shell does not show notifications from a program set to Yes.\n\n",
        _ => "",
    }
}

/// One row: a key, or one item of a set key (`Kind::Set`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row { pub key: usize, pub item: Option<u8> }

/// The schema's sections in order, then the pages with no settings: the pages.
pub fn pages() -> ([&'static str; PAGE_MAX], usize) {
    let mut out = [""; PAGE_MAX];
    let mut n = 0;
    for k in KEYS {
        if n == 0 || out[n - 1] != k.section() { if n < PAGE_MAX { out[n] = k.section(); n += 1; } }
    }
    for p in INFO_PAGES { if n < PAGE_MAX { out[n] = p; n += 1; } }
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
    /// Typing a new lock password (Ctrl+P), if under way.
    pub entry: Option<Entry>,
    /// The Privacy page's program rows, one line each, after its settings rows.
    pub extra: [u8; EXTRA_MAX],
    pub extra_len: usize,
    pub extra_rows: usize,
    /// The lines after About's settings, or Updates' or Storage's: what this computer is.
    pub info: [u8; INFO_MAX],
    pub info_len: usize,
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
        Model { values, page: 0, row: 0, query: [0; QUERY_MAX], query_len: 0, status: [0; 64], status_len: 0, entry: None, extra: [0; EXTRA_MAX], extra_len: 0, extra_rows: 0, info: [0; INFO_MAX], info_len: 0 }
    }
    pub fn searching(&self) -> bool { self.query_len > 0 }
    /// The rows shown now: keys, and a set key's items in its place.
    pub fn rows(&self) -> ([Row; ROWS_MAX], usize) {
        let mut out = [Row { key: 0, item: None }; ROWS_MAX];
        let mut n = 0;
        let (pages, _) = pages();
        let query = &self.query[..self.query_len];
        for (i, k) in KEYS.iter().enumerate() {
            let page = !self.searching() && k.section() == pages[self.page];
            let whole = page || self.searching() && matches(k, query);
            match k.kind {
                Kind::Set(names) => for (j, name) in names.iter().enumerate() {
                    if (whole || self.searching() && has(name, query)) && n < ROWS_MAX { out[n] = Row { key: i, item: Some(j as u8) }; n += 1; }
                },
                _ => if whole && n < ROWS_MAX { out[n] = Row { key: i, item: None }; n += 1; },
            }
        }
        (out, n)
    }
    /// True on the Privacy page (not searching): program rows follow its settings.
    pub fn on_privacy(&self) -> bool { !self.searching() && pages().0[self.page] == PRIVACY }
    /// The page being shown (not searching) is About, Updates or Storage: its lines follow.
    pub fn info_page(&self) -> Option<&'static str> {
        let page = pages().0[self.page];
        (!self.searching() && (page == ABOUT || INFO_PAGES.contains(&page))).then_some(page)
    }
    pub fn set_info(&mut self, text: &[u8]) {
        let n = text.len().min(INFO_MAX);
        self.info[..n].copy_from_slice(&text[..n]);
        self.info_len = n;
    }
    /// Program rows' text, `\n` after each line; `rows` lines.
    pub fn set_extra(&mut self, text: &[u8], rows: usize) {
        let n = text.len().min(EXTRA_MAX);
        self.extra[..n].copy_from_slice(&text[..n]);
        self.extra_len = n;
        self.extra_rows = rows;
        let (_, keys) = self.rows();
        self.row = self.row.min((keys + rows).saturating_sub(1));
    }
    fn row_count(&self) -> usize { self.rows().1 + if self.on_privacy() { self.extra_rows } else { 0 } }
    /// The chosen program row, if the choice is past the page's settings.
    pub fn extra_row(&self) -> Option<usize> {
        let (_, keys) = self.rows();
        (self.on_privacy() && self.row >= keys && self.row - keys < self.extra_rows).then(|| self.row - keys)
    }
    pub fn selected_row(&self) -> Option<Row> {
        let (rows, n) = self.rows();
        (self.row < n).then(|| rows[self.row])
    }
    #[cfg(test)]
    pub fn selected(&self) -> Option<&'static Key> { self.selected_row().map(|r| &KEYS[r.key]) }
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
        let n = self.row_count();
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
    /// Ctrl+P: start typing a new lock password.
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
                self.say("The two passwords differ. Press Ctrl+P to try again.");
            }
            c if (0x20..0x7F).contains(&c) && e.len < 64 => { e.typed[e.len] = c; e.len += 1; }
            _ => {}
        }
        Change::None
    }
    /// Left/Right (or Enter, as Right) on the chosen row.
    /// On a set's item, Right puts the item in and Left takes it out.
    pub fn step(&self, forward: bool, big: bool) -> Change {
        let Some(row) = self.selected_row() else { return Change::None };
        let k = &KEYS[row.key];
        let v = self.values.value(k.id);
        match row.item {
            Some(i) => Change::Set(k.id, if forward { v | 1 << i } else { v & !(1 << i) }),
            None => Change::Set(k.id, step_value(k, v, forward, big)),
        }
    }
}

fn has(hay: &str, query: &[u8]) -> bool { hay.as_bytes().windows(query.len()).any(|w| w.eq_ignore_ascii_case(query)) }
/// Case-insensitive substring of the label or the TOML name.
fn matches(k: &Key, query: &[u8]) -> bool { has(k.label, query) || has(k.name, query) }

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
        // A set changes one item at a time (`Model::step`).
        Kind::Set(_) => v,
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
        Kind::Set(_) => { w.int(v.count_ones() as i32); w.put(" chosen"); }
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
    w.put("Settings    PgDn/PgUp: pages   type: search   Ctrl+P: lock password   F11: close\n\n");
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
    if !m.searching() { w.put(page_note(pages[m.page])); }
    let (rows, count) = m.rows();
    if count == 0 && m.info_page().is_none() { w.put("  Nothing matches.\n"); }
    for (r, row) in rows[..count].iter().enumerate() {
        let k = &KEYS[row.key];
        w.put(if r == m.row { "> " } else { "  " });
        let start = w.at;
        w.put(&k.label[..k.label.len().min(38)]);
        let mut value = [0u8; 32];
        let len = match (row.item, k.kind) {
            (Some(i), Kind::Set(names)) => {
                w.put(": ");
                w.put(names[i as usize]);
                let on = m.values.holds(k.id, i as usize);
                value[..3].copy_from_slice(if on { b"Yes" } else { b"No " });
                if on { 3 } else { 2 }
            }
            _ => show_value(k, m.values.value(k.id), &mut value),
        };
        for _ in w.at - start..40 { w.byte(b' '); }
        if r == m.row { w.put("< "); }
        for b in &value[..len] { w.byte(*b); }
        if r == m.row { w.put(" >"); }
        w.byte(b'\n');
    }
    if m.on_privacy() {
        w.put("\n  Programs: Left takes a grant away, Right gives it. A change applies from the program's next run.\n");
        if m.extra_rows == 0 { w.put("  No installed program asks for anything.\n"); }
        for (r, line) in m.extra[..m.extra_len].split(|b| *b == b'\n').take(m.extra_rows).enumerate() {
            let chosen = m.extra_row() == Some(r);
            w.put(if chosen { "> " } else { "  " });
            for b in line { w.byte(*b); }
            if chosen { w.put(" <>"); }
            w.byte(b'\n');
        }
    }
    if m.info_page().is_some() {
        if count > 0 { w.put("\n"); }
        for b in &m.info[..m.info_len] { w.byte(*b); }
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
        assert_eq!(&p[..n], &["appearance", "desktop", "display", "keyboard", "sound", "time", "session", "accessibility", "setup", "open", "startup", "shortcuts", "notifications", "privacy", "bluetooth", "updates", "storage"]);
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
        assert_eq!(step_value(wall, 0, false, false), sex_config::WALLPAPERS.len() as i32 - 1, "choices wrap");
        let accent = sex_config::key(id::ACCENT).unwrap();
        assert_eq!(step_value(accent, 0x3D7EFF, true, false), 0x14B8A6);
        assert_eq!(step_value(accent, 0x123456, true, false), 0xD6336C, "custom goes to the first");
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
        assert_eq!(KEYS[rows[0].key].id, id::REPEAT_DELAY);
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
        assert_eq!(pages().0[m.page], "storage");
    }

    #[test]
    fn updates_and_storage_show_files_lines_without_rows() {
        let mut m = Model::new(Config::new());
        go(&mut m, "updates");
        assert_eq!((m.rows().1, m.info_page()), (0, Some("updates")));
        m.set_info(b"  Running from system slot A.\n");
        let t = text(&m);
        assert!(t.contains("[Updates]") && t.contains("Running from system slot A.") && !t.contains("Nothing matches"), "{t}");
        m.move_row(true);
        assert!(matches!(m.step(true, false), Change::None), "nothing to change");
        go(&mut m, "storage");
        assert!(text(&m).contains("[Storage]"));
        for c in b"slot" { m.type_char(*c); }
        assert!(m.info_page().is_none() && !text(&m).contains("Running from"));
    }

    fn go(m: &mut Model, section: &str) { while pages().0[m.page] != section { m.next_page(true); } }
    fn text(m: &Model) -> String { let mut buf = [0u8; 8192]; let n = render(m, &mut buf); String::from_utf8_lossy(&buf[..n]).into_owned() }

    #[test]
    fn startup_and_notification_pages_have_a_row_per_program() {
        let mut m = Model::new(Config::new());
        go(&mut m, "startup");
        let (rows, n) = m.rows();
        assert_eq!(n, sex_config::PROGRAMS.len());
        assert_eq!(rows[2], Row { key: rows[0].key, item: Some(2) });
        m.move_row(true); m.move_row(true);
        assert!(text(&m).contains("> Start at login: mesh"), "{}", text(&m));
        let Change::Set(k, v) = m.step(true, false) else { panic!() };
        assert_eq!((k, v), (id::STARTUP, 4));
        m.values.set(k, v).unwrap();
        assert!(text(&m).contains("< Yes >"));
        assert!(matches!(m.step(false, false), Change::Set(id::STARTUP, 0)));
        go(&mut m, "notifications");
        m.move_row(true); m.move_row(true); m.move_row(true);
        assert!(matches!(m.step(true, false), Change::Set(id::NOTIFY_HIDDEN, 8)), "abacus is fourth");
        // Search finds one program's rows on both pages.
        m.clear_search();
        for c in b"abacus" { m.type_char(*c); }
        let (rows, n) = m.rows();
        assert_eq!((n, KEYS[rows[0].key].id, KEYS[rows[1].key].id), (2, id::STARTUP, id::NOTIFY_HIDDEN));
    }

    #[test]
    fn shortcuts_and_about() {
        let mut m = Model::new(Config::new());
        go(&mut m, "shortcuts");
        assert_eq!(m.selected().unwrap().id, id::SHORTCUT_LAUNCHER);
        assert!(text(&m).contains("< default >"));
        assert!(matches!(m.step(true, false), Change::Set(id::SHORTCUT_LAUNCHER, 1)));
        go(&mut m, "setup");
        m.set_info(b"  Processor: 4 cores\n");
        assert!(m.info_page() == Some(ABOUT) && text(&m).contains("[About]") && text(&m).contains("Processor: 4 cores"));
        m.next_page(true);
        assert!(!text(&m).contains("Processor"));
    }

    #[test]
    fn privacy_page_has_program_rows() {
        let mut m = Model::new(Config::new());
        go(&mut m, "privacy");
        assert!(m.on_privacy());
        let mut buf = [0u8; 4096];
        let n = render(&m, &mut buf);
        assert!(core::str::from_utf8(&buf[..n]).unwrap().contains("No installed program asks for anything."));
        m.set_extra(b"chello  network  allowed\nchello  microphone  not asked yet\n", 2);
        // Two settings (first-run answers), then two program rows.
        assert_eq!(m.selected().unwrap().id, id::FIRST_RUN_NETWORK);
        for _ in 0..5 { m.move_row(true); }
        assert_eq!((m.row, m.extra_row(), m.selected().is_none()), (3, Some(1), true));
        assert!(matches!(m.step(true, false), Change::None));
        m.move_row(false);
        let n = render(&m, &mut buf);
        let text = core::str::from_utf8(&buf[..n]).unwrap();
        assert!(text.contains("[Privacy and grants]"), "{text}");
        assert!(text.contains("> chello  network  allowed <>"), "{text}");
        // Fewer rows later: the choice stays on the page.
        m.set_extra(b"", 0);
        assert_eq!((m.row, m.extra_row()), (1, None));
        // Search hides program rows.
        m.type_char(b'z');
        assert!(!m.on_privacy());
    }
}
