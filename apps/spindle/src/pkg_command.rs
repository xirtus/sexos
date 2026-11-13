// SPDX-License-Identifier: GPL-3.0-only
//! `pkg`: packages. Files checks signatures and installs (see `sex_pdx::package`); this is
//! the person's side: the commands, downloads, and the question a package's first run asks.
//!
//!   pkg install FILE|URL   check the signature and install or upgrade
//!   pkg check FILE         say what a package is and asks for, install nothing
//!   pkg remove NAME        remove a package (its program's saved files stay)
//!   pkg list               installed packages and what each was allowed
//!   pkg trust KEY          trust packages signed by this publisher key
//!   pkg allow|deny PROGRAM network|microphone   change an answer
//!   pkg search|get|upgrade see `store_command`
//!
//! The first run of a program whose package asks for the network stops to ask the person.
//! The answer is kept; a "no" runs it without the network, so its network calls are refused.
use crate::fetch_command::FETCH;
use crate::fs_commands::{FS, PACKAGE_STATUS};
use crate::Scrollback;
use sex_pdx::package as p;
use sex_pdx::program_store as store;

use sex_pdx::package::DOWNLOAD;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage { Idle, Fetch, Ask, Wait, Answer, Consent }
#[derive(Clone, Copy)]
struct Held { ticket: u32, flags: u64, name: [u8; 16], len: usize, open: u32, allowed: u32 }
pub struct Pkg { stage: Stage, op: u64, arg: [u8; 512], arg_len: usize, length: u64, held: Option<Held>, then_run: bool }
pub static mut PKG: Pkg = Pkg { stage: Stage::Idle, op: 0, arg: [0; 512], arg_len: 0, length: 0, held: None, then_run: false };

struct Text { buf: [u8; 160], len: usize }
impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let take = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}
macro_rules! say { ($sb:expr, $($arg:tt)*) => {{
    let mut t = Text { buf: [0; 160], len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut t, format_args!($($arg)*));
    $sb.push(&t.buf[..t.len]);
    sex_pdx::serial_record!("[spindle.pkg.say] {}", core::str::from_utf8(&t.buf[..t.len]).unwrap_or("?"));
}}}
fn shown(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("?") }
fn grant_bit(word: &[u8]) -> Option<u32> { match word { b"network" | b"net" => Some(p::NETWORK), b"microphone" | b"mic" => Some(p::MICROPHONE), _ => None } }
fn words(bits: u32) -> &'static str {
    match bits & (p::NETWORK | p::MICROPHONE) { 1 => "the network", 2 => "the microphone", 3 => "the network and the microphone", _ => "nothing" }
}

impl Pkg {
    pub fn busy(&self) -> bool { !matches!(self.stage, Stage::Idle | Stage::Wait) }
    /// A question is waiting for the person's yes or no.
    pub fn asking(&self) -> bool { self.stage == Stage::Wait }

    pub fn command(&mut self, args: &[u8], sb: &mut Scrollback) {
        if self.busy() { sb.push(b"pkg: still working on the last request."); return; }
        if self.asking() { sb.push(b"Answer the question first: yes or no."); return; }
        let mut it = args.splitn(2, |b| *b == b' ');
        let (sub, rest) = (it.next().unwrap_or(b""), it.next().unwrap_or(b"").trim_ascii());
        match sub {
            b"install" | b"check" if !rest.is_empty() => {
                let op = if sub == b"install" { p::INSTALL } else { p::CHECK };
                if rest.starts_with(b"https://") || rest.starts_with(b"http://") {
                    let mut line = [0u8; 700];
                    if rest.len() + 1 + DOWNLOAD.len() > line.len() { sb.push(b"pkg: address too long"); return; }
                    line[..rest.len()].copy_from_slice(rest);
                    line[rest.len()] = b' ';
                    line[rest.len() + 1..rest.len() + 1 + DOWNLOAD.len()].copy_from_slice(DOWNLOAD);
                    self.queue(op, DOWNLOAD, 0);
                    self.stage = Stage::Fetch;
                    unsafe { FETCH.start(&line[..rest.len() + 1 + DOWNLOAD.len()], sb, |p, o| { let k = p.len(); if k > o.len() { return None; } o[..k].copy_from_slice(p); Some(k) }); }
                    if unsafe { !FETCH.busy() } { self.stage = Stage::Idle; sb.push(b"pkg: the download could not start"); }
                    return;
                }
                let mut path = [0u8; 512];
                let Some(n) = (unsafe { FS.join(rest, &mut path) }) else { sb.push(b"Path too long."); return };
                self.queue(op, &path[..n], 0);
            }
            b"remove" | b"uninstall" if !rest.is_empty() => self.queue(p::REMOVE, rest, 0),
            b"list" | b"" => self.queue(p::LIST, b"", 0),
            b"trust" if !rest.is_empty() => self.queue(p::TRUST, rest, 0),
            b"allow" | b"deny" => {
                let mut w = rest.splitn(2, |b| *b == b' ');
                let (program, grant) = (w.next().unwrap_or(b""), w.next().unwrap_or(b"").trim_ascii());
                let Some(bit) = grant_bit(grant).filter(|_| !program.is_empty()) else { sb.push(b"Usage: pkg allow|deny PROGRAM network|microphone"); return };
                self.queue(p::CONSENT, program, (bit as u64) << 16 | if sub == b"allow" { bit as u64 } else { 0 });
            }
            _ => {
                sb.push(b"pkg install FILE|URL   install or upgrade a signed package");
                sb.push(b"pkg check FILE   pkg remove NAME   pkg list   pkg trust KEY");
                sb.push(b"pkg allow|deny PROGRAM network|microphone");
            }
        }
    }
    fn queue(&mut self, op: u64, arg: &[u8], length: u64) {
        let n = arg.len().min(self.arg.len());
        self.arg[..n].copy_from_slice(&arg[..n]);
        self.arg_len = n; self.op = op; self.length = length;
        self.stage = Stage::Ask;
        sex_pdx::serial_record!("[spindle.pkg.ask] op={} arg={}", op, shown(&arg[..n]));
    }

    /// Called when Files has prepared a program to run. Returns the run's flags, or None
    /// when the person must answer first (the run waits; the ticket is kept).
    pub fn before_run(&mut self, note: Option<p::Note>, name: &[u8], ticket: u32, flags: u64, count: u64, sb: &mut Scrollback) -> Option<u64> {
        let Some(note) = note.filter(|n| n.asked != 0) else { return Some(flags) };
        sex_pdx::serial_record!("[spindle.pkg.note] program={} asked={} answered={} allowed={}", shown(name), note.asked, note.answered, note.allowed);
        let granted = |allowed: u32| if allowed & p::NETWORK != 0 { flags | store::RUN_NETWORK } else { flags };
        if note.open() == 0 || count != 0 { return Some(granted(note.allowed)); }
        let mut held = Held { ticket, flags, name: [0; 16], len: name.len().min(16), open: note.open(), allowed: note.allowed };
        held.name[..held.len].copy_from_slice(&name[..held.len]);
        self.held = Some(held);
        self.stage = Stage::Wait;
        say!(sb, "{} asks to use {}. Allow it? Type yes or no.", shown(name), words(note.open()));
        sex_pdx::serial_record!("[spindle.pkg.question] program={} grants={}", shown(name), note.open());
        None
    }
    /// The person's answer to the question. True if the line was one.
    pub fn answer(&mut self, line: &[u8], sb: &mut Scrollback) -> bool {
        if self.stage != Stage::Wait { return false; }
        let Some(mut held) = self.held else { self.stage = Stage::Idle; return false };
        let yes = match line.trim_ascii() { b"yes" | b"y" | b"Yes" | b"YES" => true, b"no" | b"n" | b"No" | b"NO" => false, _ => { sb.push(b"Type yes or no."); return true; } };
        if yes { held.allowed |= held.open; } else { held.allowed &= !held.open; }
        self.held = Some(held);
        say!(sb, "{} {} {}.", if yes { "Allowed:" } else { "Not allowed:" }, shown(&held.name[..held.len]), words(held.open));
        let len = held.len;
        let name = held.name;
        self.queue(p::CONSENT, &name[..len], (held.open as u64) << 16 | (held.allowed & held.open) as u64);
        self.then_run = true;
        self.stage = Stage::Answer;
        true
    }

    /// Moves on when the last step finished. True when it printed something.
    pub fn step(&mut self, sb: &mut Scrollback) -> bool {
        match self.stage {
            Stage::Idle | Stage::Wait => false,
            Stage::Fetch => {
                if unsafe { FETCH.busy() } { return false; }
                if unsafe { FETCH.result } != Some(true) { sb.push(b"pkg: the download failed."); self.stage = Stage::Idle; return true; }
                self.stage = Stage::Ask;
                false
            }
            Stage::Ask | Stage::Answer => {
                if unsafe { FS.busy() } { return false; }
                if !unsafe { FS.start_package(self.op, &self.arg[..self.arg_len], self.length) } { return false; }
                self.stage = Stage::Consent;
                false
            }
            Stage::Consent => {
                let Some(status) = (unsafe { PACKAGE_STATUS }) else { return false };
                unsafe { PACKAGE_STATUS = None; }
                self.stage = Stage::Idle;
                sex_pdx::serial_record!("[spindle.pkg.done] op={} status={}", self.op, status);
                if core::mem::take(&mut self.then_run) {
                    let Some(held) = self.held.take() else { return true };
                    let flags = if held.allowed & p::NETWORK != 0 { held.flags | store::RUN_NETWORK } else { held.flags };
                    let asked = sex_pdx::pdx_call_checked(crate::SLOT_SHELL, store::OP_RUN, store::TICKET, held.ticket as u64, flags).is_ok();
                    sex_pdx::serial_record!("[spindle.pkg.run] program={} network={}", shown(&held.name[..held.len]), (flags & store::RUN_NETWORK != 0) as u8);
                    sb.push(if asked { b"Asked for the program. A window opens if it starts." } else { b"Could not reach the shell." });
                } else if self.op == p::CONSENT && status == sex_pdx::fs::OK {
                    sb.push(b"Saved. It applies from the program's next run.");
                }
                true
            }
        }
    }
}
