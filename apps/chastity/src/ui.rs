// SPDX-License-Identifier: GPL-3.0-only
//! Chastity's screens: what each key does and what the window shows. No SexOS calls
//! here, so the whole flow runs in host tests; `main.rs` feeds it keys and shows it.
use chastity_core::generator::{self, Options};
use chastity_core::vault::{EntryId, Kdf, NewEntry, Summary, Vault};
use chastity_core::{fingerprint, OpenError};
use zeroize::{Zeroize, Zeroizing};

/// Where the vault file lives, and the clipboard. `main.rs` gives SexOS's.
pub trait Host {
    fn load(&mut self) -> Option<Vec<u8>>;
    fn store(&mut self, bytes: &[u8]) -> Result<(), String>;
    /// Copies a secret for pasting elsewhere; an error says why it cannot.
    fn copy(&mut self, text: &str) -> Result<(), String>;
    /// A line for the serial record. Never a secret.
    fn record(&mut self, line: &str);
    /// Unix time, for TOTP.
    fn now(&self) -> u64;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "hermit"), allow(dead_code))] // Only the window sends some keys.
pub enum Input { Char(char), Enter, Backspace, Escape, Up, Down, Delete }

/// The fields of the add form, in order.
const FIELDS: [(&str, bool); 6] = [("Title", false), ("User name", false), ("Password", true), ("URL", false), ("Notes", false), ("TOTP (otpauth:// or Base32)", true)];

enum Screen {
    /// No vault yet: choose a master password, then repeat it.
    Create { first: Option<Zeroizing<String>> },
    Unlock,
    List,
    Search,
    Add { field: usize, values: Vec<Zeroizing<String>> },
    View { id: EntryId, reveal: bool },
    ConfirmDelete { id: EntryId },
}

pub struct Ui {
    screen: Screen,
    vault: Option<Vault>,
    kdf: Kdf,
    /// What is being typed (a password is never shown, only dots).
    line: Zeroizing<String>,
    query: String,
    selected: usize,
    message: String,
}

impl Ui {
    pub fn new(host: &mut dyn Host, kdf: Kdf) -> Ui {
        let exists = host.load().is_some();
        host.record(&format!("[chastity.start] vault={}", if exists { "present" } else { "absent" }));
        let screen = if exists { Screen::Unlock } else { Screen::Create { first: None } };
        Ui { screen, vault: None, kdf, line: Zeroizing::new(String::new()), query: String::new(), selected: 0, message: String::new() }
    }

    pub fn unlocked(&self) -> bool { self.vault.is_some() }

    fn shown(&self) -> Vec<Summary> {
        self.vault.as_ref().map_or_else(Vec::new, |v| v.search(&self.query))
    }

    /// Locks: the vault (decrypted entries and the held password) is dropped and zeroized.
    pub fn lock(&mut self, host: &mut dyn Host, why: &str) {
        if let Some(v) = self.vault.take() {
            v.lock();
            self.line.zeroize();
            self.query.clear();
            self.screen = Screen::Unlock;
            self.message = format!("Locked ({why}).");
            host.record(&format!("[chastity.locked] reason={why} zeroized=1"));
        }
    }

    fn save(&mut self, host: &mut dyn Host) -> bool {
        let Some(v) = &self.vault else { return false };
        match v.save().map_err(|e| e.to_string()).and_then(|b| host.store(&b).map(|_| b.len())) {
            Ok(bytes) => { host.record(&format!("[chastity.saved] entries={} bytes={bytes}", v.entries().len())); true }
            Err(e) => { self.message = format!("Not saved: {e}"); host.record("[chastity.save.failed]"); false }
        }
    }

    pub fn key(&mut self, host: &mut dyn Host, input: Input) {
        if !matches!(input, Input::Up | Input::Down) { self.message.clear(); }
        match &mut self.screen {
            Screen::Create { first } => {
                if !edit(&mut self.line, input) { return; }
                if self.line.is_empty() { self.message = "Choose a master password.".into(); return; }
                match first.take() {
                    None => { *first = Some(std::mem::take(&mut self.line)); }
                    Some(f) if *f == *self.line => {
                        let mut password = std::mem::take(&mut *self.line);
                        self.vault = Some(Vault::create("Chastity", &mut password, self.kdf));
                        if self.save(host) {
                            host.record("[chastity.created] entries=0");
                            self.screen = Screen::List;
                        } else {
                            self.vault = None;
                            self.screen = Screen::Create { first: None };
                        }
                    }
                    Some(_) => { self.line.zeroize(); self.message = "They differ. Choose again.".into(); }
                }
            }
            Screen::Unlock => {
                if !edit(&mut self.line, input) { return; }
                let Some(bytes) = host.load() else { self.message = "The vault file is gone.".into(); return };
                let mut password = std::mem::take(&mut *self.line);
                match Vault::open(&bytes, &mut password) {
                    Ok(v) => {
                        let list = v.entries();
                        let titles: Vec<&str> = list.iter().map(|s| s.title.as_str()).collect();
                        host.record(&format!("[chastity.unlocked] entries={} titles={:#x}", list.len(), fingerprint(titles.join("\n").as_bytes())));
                        self.vault = Some(v);
                        self.selected = 0;
                        self.screen = Screen::List;
                    }
                    Err(OpenError::WrongPassword) => { self.message = "Wrong password.".into(); host.record("[chastity.unlock.refused] reason=wrong_password"); }
                    Err(e) => { self.message = e.to_string(); host.record("[chastity.unlock.refused] reason=unreadable"); }
                }
            }
            Screen::List | Screen::Search => {
                let searching = matches!(self.screen, Screen::Search);
                let count = self.shown().len();
                match input {
                    Input::Up => self.selected = self.selected.saturating_sub(1),
                    Input::Down => self.selected = (self.selected + 1).min(count.saturating_sub(1)),
                    Input::Escape if searching => { self.query.clear(); self.screen = Screen::List; self.selected = 0; }
                    Input::Enter if searching => {
                        host.record(&format!("[chastity.search] words={} matches={count}", self.query.split_whitespace().count()));
                        self.screen = Screen::List;
                    }
                    Input::Backspace if searching => { self.query.pop(); self.selected = 0; }
                    Input::Char(c) if searching => { self.query.push(c); self.selected = 0; }
                    Input::Enter => if let Some(s) = self.shown().get(self.selected) { self.screen = Screen::View { id: s.id, reveal: false } },
                    Input::Char('/') => { self.screen = Screen::Search; }
                    Input::Char('a') => { self.screen = Screen::Add { field: 0, values: Vec::new() }; }
                    Input::Char('l') => self.lock(host, "asked"),
                    Input::Char('c') => if let Some(s) = self.shown().get(self.selected) { let id = s.id; self.copy(host, id, false) },
                    Input::Char('u') => if let Some(s) = self.shown().get(self.selected) { let id = s.id; self.copy(host, id, true) },
                    Input::Escape => { self.query.clear(); self.selected = 0; }
                    Input::Delete => if let Some(s) = self.shown().get(self.selected) { self.screen = Screen::ConfirmDelete { id: s.id } },
                    _ => {}
                }
            }
            Screen::Add { field, values } => {
                if input == Input::Escape { self.line.zeroize(); self.screen = Screen::List; return; }
                if !edit(&mut self.line, input) { return; }
                let (name, secret) = FIELDS[*field];
                let mut value = std::mem::take(&mut self.line);
                if name == "Password" && value.is_empty() {
                    value = generator::generate(&Options::default()).unwrap_or_default();
                    host.record("[chastity.field] name=password generated=1");
                } else if secret {
                    host.record(&format!("[chastity.field] name={} set={}", name.split(' ').next().unwrap_or(name).to_lowercase(), !value.is_empty() as u8));
                } else {
                    host.record(&format!("[chastity.field] name={} len={}", name.replace(' ', "").to_lowercase(), value.chars().count()));
                }
                values.push(value);
                *field += 1;
                if *field == FIELDS.len() {
                    let mut v = std::mem::take(values).into_iter();
                    let mut take = || v.next().unwrap_or_default();
                    let new = NewEntry { title: take().to_string(), username: take().to_string(), password: take(), url: take().to_string(), notes: take().to_string(), otp: take() };
                    let title = fingerprint(new.title.as_bytes());
                    let vault = self.vault.as_mut().expect("unlocked");
                    if !new.otp.trim().is_empty() && chastity_core::totp::Totp::parse(&new.otp).is_err() { self.message = "TOTP not understood; saved without it.".into(); }
                    let otp_ok = chastity_core::totp::Totp::parse(&new.otp).is_ok();
                    let new = if otp_ok { new } else { NewEntry { otp: Zeroizing::new(String::new()), ..new } };
                    vault.add(new);
                    host.record(&format!("[chastity.entry.added] entries={} title={title:#x}", vault.entries().len()));
                    self.save(host);
                    self.screen = Screen::List;
                }
            }
            Screen::View { id, reveal } => {
                let id = *id;
                match input {
                    Input::Escape | Input::Backspace => self.screen = Screen::List,
                    Input::Char('r') => *reveal = !*reveal,
                    Input::Char('c') => self.copy(host, id, false),
                    Input::Char('u') => self.copy(host, id, true),
                    Input::Char('t') => {
                        let code = self.vault.as_ref().and_then(|v| v.totp(id, host.now()));
                        if let Some((code, _)) = code { self.copy_text(host, &code, "totp"); } else { self.message = "No TOTP for this entry.".into(); }
                    }
                    Input::Delete => self.screen = Screen::ConfirmDelete { id },
                    _ => {}
                }
            }
            Screen::ConfirmDelete { id } => {
                let id = *id;
                if input == Input::Char('y') {
                    if let Some(v) = self.vault.as_mut() { v.remove(id); }
                    host.record("[chastity.entry.removed]");
                    self.save(host);
                    self.selected = 0;
                }
                self.screen = Screen::List;
            }
        }
    }

    fn copy(&mut self, host: &mut dyn Host, id: EntryId, username: bool) {
        let Some(d) = self.vault.as_ref().and_then(|v| v.detail(id)) else { return };
        if username { self.copy_text(host, &d.summary.username, "username") } else { self.copy_text(host, &d.password, "password") }
    }

    fn copy_text(&mut self, host: &mut dyn Host, text: &str, what: &str) {
        match host.copy(text) {
            Ok(()) => { self.message = format!("Copied the {what}."); host.record(&format!("[chastity.copy] what={what} ok=1")); }
            Err(e) => { self.message = format!("Cannot copy: {e}"); host.record(&format!("[chastity.copy] what={what} ok=0")); }
        }
    }

    /// The window's text.
    pub fn render(&self, host: &dyn Host) -> String {
        let dots = "*".repeat(self.line.chars().count());
        let mut out = String::from("Chastity - password vault\n\n");
        match &self.screen {
            Screen::Create { first } => {
                out += "No vault yet. Its file is vault.kdbx (KeePass KDBX4).\n\n";
                out += if first.is_none() { "Choose a master password: " } else { "Repeat it: " };
                out += &dots;
                out += "_\n\nThere is no way back in without it.";
            }
            Screen::Unlock => { out += "Master password: "; out += &dots; out += "_\n\nEnter unlocks."; }
            Screen::List | Screen::Search => {
                let list = self.shown();
                if matches!(self.screen, Screen::Search) || !self.query.is_empty() { out += &format!("Search: {}{}\n", self.query, if matches!(self.screen, Screen::Search) { "_" } else { "" }); }
                if list.is_empty() { out += "(no entries)\n"; }
                for (i, s) in list.iter().enumerate().take(18) {
                    out += &format!("{} {:<24} {:<20}{}\n", if i == self.selected { ">" } else { " " }, clip(&s.title, 24), clip(&s.username, 20), if s.has_totp { " [2FA]" } else { "" });
                }
                out += "\nEnter open  a add  / search  c copy password\nu copy user  Del delete  l lock";
            }
            Screen::Add { field, values } => {
                for (i, (name, secret)) in FIELDS.iter().enumerate() {
                    let value = if i < values.len() { if *secret { "*".repeat(values[i].chars().count().min(12)) } else { values[i].to_string() } }
                        else if i == *field { format!("{}_", if *secret { dots.clone() } else { self.line.to_string() }) } else { String::new() };
                    out += &format!("{:<10} {}\n", name.split(' ').next().unwrap_or(name), clip(&value, 48));
                }
                out += "\nEnter next field  Esc cancel";
                if FIELDS[*field].0 == "Password" { out += "\nEmpty password: generate one (20 characters)"; }
            }
            Screen::View { id, reveal } => {
                if let Some(d) = self.vault.as_ref().and_then(|v| v.detail(*id)) {
                    out += &format!("Title     {}\nUser name {}\nPassword  {}\nURL       {}\nNotes     {}\n", d.summary.title, d.summary.username,
                        if *reveal { d.password.to_string() } else { "********".into() }, d.summary.url, d.notes);
                    if let Some(t) = &d.otp { let now = host.now(); out += &format!("TOTP      {} ({}s)\n", t.code(now), t.remaining(now)); }
                }
                out += "\nr reveal  c copy password  u copy user\nt copy TOTP  Del delete  Esc back";
            }
            Screen::ConfirmDelete { .. } => out += "Delete this entry? y deletes, any other key keeps it.",
        }
        if !self.message.is_empty() { out += "\n\n"; out += &self.message; }
        out
    }
}

/// Typing into the line: true when Enter ends it.
fn edit(line: &mut Zeroizing<String>, input: Input) -> bool {
    match input {
        Input::Char(c) => { if line.len() < 1024 { line.push(c); } false }
        Input::Backspace => { line.pop(); false }
        Input::Escape => { line.zeroize(); false }
        Input::Enter => true,
        _ => false,
    }
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "~" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake { file: Option<Vec<u8>>, records: Vec<String>, copied: Vec<String> }
    impl Host for Fake {
        fn load(&mut self) -> Option<Vec<u8>> { self.file.clone() }
        fn store(&mut self, bytes: &[u8]) -> Result<(), String> { self.file = Some(bytes.to_vec()); Ok(()) }
        fn copy(&mut self, text: &str) -> Result<(), String> { self.copied.push(text.into()); Ok(()) }
        fn record(&mut self, line: &str) { self.records.push(line.into()); }
        fn now(&self) -> u64 { 59 }
    }

    fn typed(ui: &mut Ui, host: &mut Fake, text: &str) {
        for c in text.chars() { ui.key(host, Input::Char(c)); }
        ui.key(host, Input::Enter);
    }

    /// The gate's story on the host: create, add, "reboot" (a new Ui on the same
    /// file), wrong password, unlock, the entry is there.
    #[test]
    fn create_add_reopen_unlock() {
        let mut host = Fake::default();
        let mut ui = Ui::new(&mut host, Kdf::TEST_ONLY);
        typed(&mut ui, &mut host, "tr0ub4dor&3");
        assert!(!ui.render(&host).contains("tr0ub4dor"), "the password is never shown");
        typed(&mut ui, &mut host, "tr0ub4dor&3");
        assert!(ui.unlocked());
        ui.key(&mut host, Input::Char('a'));
        for v in ["Mail", "andreas", "", "https://mail.example.org", "work", "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"] { typed(&mut ui, &mut host, v); }
        assert!(host.records.iter().any(|r| *r == format!("[chastity.entry.added] entries=1 title={:#x}", fingerprint(b"Mail"))));
        assert!(host.records.iter().any(|r| r.starts_with("[chastity.saved] entries=1 ")));
        assert!(host.records.iter().all(|r| !r.contains("tr0ub4dor")));
        ui.key(&mut host, Input::Char('l'));
        assert!(!ui.unlocked());

        let mut ui = Ui::new(&mut host, Kdf::TEST_ONLY);
        typed(&mut ui, &mut host, "wrong");
        assert!(!ui.unlocked() && host.records.last().unwrap() == "[chastity.unlock.refused] reason=wrong_password");
        typed(&mut ui, &mut host, "tr0ub4dor&3");
        assert_eq!(host.records.last().unwrap(), &format!("[chastity.unlocked] entries=1 titles={:#x}", fingerprint(b"Mail")));
        assert!(ui.render(&host).contains("Mail") && ui.render(&host).contains("[2FA]"));
        // Search, open, the generated password copies, TOTP at t=59 is RFC 6238's.
        ui.key(&mut host, Input::Char('/'));
        typed(&mut ui, &mut host, "mail");
        assert_eq!(host.records.last().unwrap(), "[chastity.search] words=1 matches=1");
        ui.key(&mut host, Input::Enter);
        assert!(ui.render(&host).contains("TOTP      287082"));
        ui.key(&mut host, Input::Char('c'));
        ui.key(&mut host, Input::Char('t'));
        assert_eq!(host.copied[0].len(), 20);
        assert_eq!(host.copied[1], "287082");
    }

    #[test]
    fn mismatched_repeat_starts_over() {
        let mut host = Fake::default();
        let mut ui = Ui::new(&mut host, Kdf::TEST_ONLY);
        typed(&mut ui, &mut host, "one");
        typed(&mut ui, &mut host, "two");
        assert!(!ui.unlocked() && host.file.is_none());
        assert!(ui.render(&host).contains("They differ"));
    }
}
