// SPDX-License-Identifier: GPL-3.0-only
//! First-start setup (B8): the questions Settings asks while `setup.done` is off.
//! Keyboard layout, time zone (type to narrow the list), a lock password (may be
//! left empty), and whether the screen should dim, lock and turn off when idle.
//! Each answer becomes ordinary settings; the last step turns `setup.done` on.
//! Host-tested with the model (`scripts/settings_host_gate.sh`).
use sex_config::{id, LAYOUTS, ZONES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step { Layout, Zone, Password, Idle, Done }

/// Idle choices: (label, dim, lock, off) in minutes.
pub const IDLE: [(&str, i32, i32, i32); 3] = [
    ("Dim after 5 minutes, lock after 10, screen off after 15", 5, 10, 15),
    ("Dim after 10 minutes, lock after 15, screen off after 30", 10, 15, 30),
    ("Never dim, lock or turn off the screen", 0, 0, 0),
];

pub struct Setup {
    pub step: Step,
    pub layout: usize,
    pub zone: usize,
    pub idle: usize,
    pub filter: [u8; 24],
    pub filter_len: usize,
    pub password: [u8; 64],
    pub password_len: usize,
}

/// What finishing a step asks Settings to do.
pub enum Out { Nothing, Values([(u16, i32); 4]), Password([u8; 64], usize) }

fn zone_matches(zone: &str, filter: &[u8]) -> bool {
    filter.is_empty() || zone.as_bytes().windows(filter.len()).any(|w| w.eq_ignore_ascii_case(filter))
}

impl Setup {
    pub fn new() -> Self {
        Setup { step: Step::Layout, layout: 0, zone: 0, idle: 0, filter: [0; 24], filter_len: 0, password: [0; 64], password_len: 0 }
    }
    /// Zone indexes the filter lets through.
    pub fn zones(&self) -> impl Iterator<Item = usize> + '_ {
        (0..ZONES.len()).filter(move |i| zone_matches(ZONES[*i], &self.filter[..self.filter_len]))
    }
    fn fix_zone(&mut self) {
        let first = self.zones().next();
        if !self.zones().any(|z| z == self.zone) { if let Some(z) = first { self.zone = z; } }
    }
    pub fn up_down(&mut self, down: bool) {
        let step = |v: usize, n: usize| if down { (v + 1).min(n.saturating_sub(1)) } else { v.saturating_sub(1) };
        match self.step {
            Step::Layout => self.layout = step(self.layout, LAYOUTS.len()),
            Step::Idle => self.idle = step(self.idle, IDLE.len()),
            Step::Zone => {
                let list: [usize; 64] = { let mut a = [0; 64]; for (k, z) in self.zones().take(64).enumerate() { a[k] = z; } a };
                let n = self.zones().count().min(64);
                if n == 0 { return; }
                let at = list[..n].iter().position(|z| *z == self.zone).unwrap_or(0);
                self.zone = list[step(at, n)];
            }
            _ => {}
        }
    }
    /// A typed character: narrows the zone list, or adds to the password.
    pub fn typed(&mut self, c: u8) {
        match self.step {
            Step::Zone if self.filter_len < self.filter.len() && (c.is_ascii_graphic() || c == b' ') => {
                self.filter[self.filter_len] = if c == b' ' { b'_' } else { c };
                self.filter_len += 1;
                self.fix_zone();
            }
            Step::Password if self.password_len < 64 && (0x20..0x7F).contains(&c) => { self.password[self.password_len] = c; self.password_len += 1; }
            _ => {}
        }
    }
    pub fn backspace(&mut self) {
        match self.step {
            Step::Zone => { self.filter_len = self.filter_len.saturating_sub(1); self.fix_zone(); }
            Step::Password => self.password_len = self.password_len.saturating_sub(1),
            _ => {}
        }
    }
    pub fn back(&mut self) {
        self.step = match self.step { Step::Zone => Step::Layout, Step::Password => Step::Zone, Step::Idle => Step::Password, s => s };
    }
    /// Enter: the step's answer, and on to the next.
    pub fn next(&mut self) -> Out {
        match self.step {
            Step::Layout => { self.step = Step::Zone; Out::Values([(id::LAYOUT, self.layout as i32), (0, 0), (0, 0), (0, 0)]) }
            Step::Zone => { self.step = Step::Password; Out::Values([(id::ZONE, self.zone as i32), (0, 0), (0, 0), (0, 0)]) }
            Step::Password => {
                self.step = Step::Idle;
                let out = (self.password, self.password_len);
                self.password = [0; 64];
                self.password_len = 0;
                if out.1 == 0 { Out::Nothing } else { Out::Password(out.0, out.1) }
            }
            Step::Idle => {
                self.step = Step::Done;
                let (_, dim, lock, off) = IDLE[self.idle];
                Out::Values([(id::DIM_AFTER, dim), (id::LOCK_AFTER, lock), (id::BLANK_AFTER, off), (id::SETUP_DONE, 1)])
            }
            Step::Done => Out::Nothing,
        }
    }
}

/// The setup screen as text.
pub fn render(s: &Setup, out: &mut [u8], status: &[u8]) -> usize {
    let mut at = 0;
    let mut put = |b: &[u8]| { let n = b.len().min(out.len() - at); out[at..at + n].copy_from_slice(&b[..n]); at += n; };
    put(b"Welcome to SexOS. A few questions, then the desktop is yours.\n\n");
    let (title, number): (&[u8], &[u8]) = match s.step {
        Step::Layout => (b"Keyboard layout", b"1"), Step::Zone => (b"Time zone (type to narrow)", b"2"),
        Step::Password => (b"Lock password (Super+Esc locks; leave empty for none)", b"3"),
        Step::Idle => (b"When the computer is idle", b"4"), Step::Done => (b"Done", b"-"),
    };
    put(b"Step "); put(number); put(b" of 4: "); put(title); put(b"\n------------------------------------------------------------\n");
    match s.step {
        Step::Layout => for (i, l) in LAYOUTS.iter().enumerate() { put(if i == s.layout { b"> " } else { b"  " }); put(l.as_bytes()); put(b"\n"); },
        Step::Zone => {
            put(b"Find: "); put(&s.filter[..s.filter_len]); put(b"_\n");
            let list: [usize; 64] = { let mut a = [0; 64]; for (k, z) in s.zones().take(64).enumerate() { a[k] = z; } a };
            let n = s.zones().count().min(64);
            if n == 0 { put(b"  Nothing matches.\n"); }
            let at_sel = list[..n].iter().position(|z| *z == s.zone).unwrap_or(0);
            let first = at_sel.saturating_sub(5).min(n.saturating_sub(10));
            for &z in &list[first..(first + 10).min(n)] {
                put(if z == s.zone { b"> " } else { b"  " }); put(ZONES[z].as_bytes()); put(b"\n");
            }
        }
        Step::Password => { put(b"Password: "); for _ in 0..s.password_len { put(b"*"); } put(b"_\n"); }
        Step::Idle => for (i, (label, ..)) in IDLE.iter().enumerate() { put(if i == s.idle { b"> " } else { b"  " }); put(label.as_bytes()); put(b"\n"); },
        Step::Done => put(b"All set. Settings stays here for anything else.\n"),
    }
    put(b"------------------------------------------------------------\nUp/Down: choose   Enter: next   Esc: back\n");
    put(status);
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_all_steps() {
        let mut s = Setup::new();
        assert!(matches!(s.next(), Out::Values([(id::LAYOUT, 0), ..])));
        for c in b"berl" { s.typed(*c); }
        assert_eq!(ZONES[s.zone], "Europe/Berlin");
        s.backspace();
        for c in b"lin" { s.typed(*c); }
        let mut buf = [0u8; 2048];
        let n = render(&s, &mut buf, b"");
        assert!(core::str::from_utf8(&buf[..n]).unwrap().contains("> Europe/Berlin"));
        match s.next() { Out::Values([(k, v), ..]) => assert_eq!((k, ZONES[v as usize]), (id::ZONE, "Europe/Berlin")), _ => panic!() }
        for c in b"pw" { s.typed(*c); }
        match s.next() { Out::Password(p, n) => assert_eq!(&p[..n], b"pw"), _ => panic!() }
        assert_eq!(s.password_len, 0);
        s.up_down(true);
        match s.next() { Out::Values(v) => assert_eq!(v, [(id::DIM_AFTER, 10), (id::LOCK_AFTER, 15), (id::BLANK_AFTER, 30), (id::SETUP_DONE, 1)]), _ => panic!() }
        assert_eq!(s.step, Step::Done);
    }

    #[test]
    fn zone_list_moves_and_filters() {
        let mut s = Setup::new();
        s.step = Step::Zone;
        for c in b"america" { s.typed(*c); }
        let first = s.zone;
        assert!(ZONES[first].starts_with("America/"));
        s.up_down(true);
        assert!(ZONES[s.zone].starts_with("America/") && s.zone != first);
        for c in b"zzz" { s.typed(*c); }
        assert_eq!(s.zones().count(), 0);
        let mut buf = [0u8; 2048];
        let n = render(&s, &mut buf, b"");
        assert!(core::str::from_utf8(&buf[..n]).unwrap().contains("Nothing matches."));
        s.back();
        assert_eq!(s.step, Step::Layout);
    }

    #[test]
    fn empty_password_sets_none() {
        let mut s = Setup::new();
        s.step = Step::Password;
        assert!(matches!(s.next(), Out::Nothing));
    }
}
