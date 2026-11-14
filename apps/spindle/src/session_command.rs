// SPDX-License-Identifier: GPL-3.0-only
//! Session restore: the programs open when the machine went down come back after it
//! starts again, each on its workspace, with what the person gave them (see
//! `sex_pdx::session_restore`). Spindle started those runs, so it keeps the list in
//! `/.session`: the command line of each `run` is kept when Files hands out its ticket,
//! tied to the run Shell says started with that ticket, moved when its window moves
//! and dropped when it ends. Only the boot terminal keeps a session.
use crate::Scrollback;
use sex_pdx::fs as f;
use sex_pdx::session_restore as r;

const PATH_MAX: usize = 96;
const ARGS_MAX: usize = 256;
const FILE_MAX: usize = 4096;

#[derive(Clone, Copy)]
struct Line { path: [u8; PATH_MAX], path_len: usize, args: [u8; ARGS_MAX], args_len: usize, network: bool, scene: u8 }
impl Line {
    const EMPTY: Line = Line { path: [0; PATH_MAX], path_len: 0, args: [0; ARGS_MAX], args_len: 0, network: false, scene: 0 };
    fn new(path: &[u8], args: &[u8], network: bool, scene: u8) -> Option<Line> {
        if path.len() > PATH_MAX || args.len() > ARGS_MAX || path.is_empty() { return None; }
        let mut l = Line { network, scene, path_len: path.len(), args_len: args.len(), ..Line::EMPTY };
        l.path[..path.len()].copy_from_slice(path);
        l.args[..args.len()].copy_from_slice(args);
        Some(l)
    }
    fn entry(&self) -> r::Entry<'_> { r::Entry { scene: self.scene.min(9), network: self.network, path: &self.path[..self.path_len], args: &self.args[..self.args_len] } }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Io { Idle, Read, Create, Truncate, Write, Sync }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Restore { NotYet, Reading, Running, Done }

pub struct Session {
    on: bool,
    /// The line of the run whose ticket Files is preparing, then waiting on Shell by ticket.
    submitted: Option<Line>,
    asked: [Option<(u32, Line)>; 4],
    runs: [Option<(u32, u32, Line)>; r::MAX_RUNS],
    dirty: bool,
    changed_at: u64,
    io: Io,
    seq: u32,
    sent_at: u64,
    len: usize,
    restore: Restore,
    text: [u8; FILE_MAX],
    text_len: usize,
    next: usize,
    /// A restored run asked for, and when; a refusal it may retry.
    waiting: Option<u64>,
    retries: u8,
}
pub static mut SESSION: Session = Session::new();

fn region() -> Option<&'static mut [u8]> {
    use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
    let v = sex_pdx::memory_region_view(f::CLIENT_SLOT, 0, f::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE).ok()?;
    Some(unsafe { core::slice::from_raw_parts_mut(v.virtual_address as *mut u8, f::REGION_BYTES) })
}

impl Session {
    pub const fn new() -> Self {
        Self { on: false, submitted: None, asked: [None; 4], runs: [None; r::MAX_RUNS], dirty: false, changed_at: 0, io: Io::Idle, seq: 0x7A0000, sent_at: 0, len: 0,
            restore: Restore::NotYet, text: [0; FILE_MAX], text_len: 0, next: 0, waiting: None, retries: 0 }
    }
    /// The boot terminal keeps the session; a later one, or one started again late, does not restore.
    pub fn init(&mut self, me: u32) {
        self.on = sex_pdx::app_catalog::SPINDLE_TARGETS.first() == Some(&me);
        if !self.on || sex_pdx::get_ticks() >= r::RESTORE_BEFORE_TICKS { self.restore = Restore::Done; }
    }
    /// The Files region is in use for the session file.
    pub fn io_busy(&self) -> bool { self.io != Io::Idle }
    /// The main loop should keep turning rather than wait for a message.
    pub fn wants_turn(&self) -> bool { self.on && (self.io != Io::Idle || self.dirty || matches!(self.restore, Restore::NotYet | Restore::Reading | Restore::Running)) }

    /// A `run` whose ticket Files is about to prepare.
    pub fn submitted(&mut self, path: &[u8], args: &[u8], flags: u64) {
        if !self.on { return; }
        self.submitted = Line::new(path, args, flags & sex_pdx::program_store::RUN_NETWORK != 0, r::scene_of(flags).unwrap_or(0));
    }
    /// Files gave that run this ticket and Shell was asked for it.
    pub fn asked(&mut self, ticket: u32) {
        let Some(line) = self.submitted.take() else { return };
        // Full: the oldest, whose run never started, gives way.
        let at = self.asked.iter().position(|a| a.is_none()).unwrap_or_else(|| { self.asked.rotate_left(1); self.asked.len() - 1 });
        self.asked[at] = Some((ticket, line));
    }
    /// Shell's word about a run (`OP_SESSION`).
    pub fn shell(&mut self, kind: u64, run: u64, value: u64) {
        if !self.on { return; }
        let (target, task) = ((run >> 32) as u32, run as u32);
        match kind {
            r::STARTED => {
                let Some(i) = self.asked.iter().position(|a| a.is_some_and(|a| a.0 as u64 == value)) else { return };
                let (_, line) = self.asked[i].take().unwrap();
                if self.waiting.is_some() { self.waiting = None; self.retries = 0; }
                let slot = self.runs.iter().position(|s| s.is_none()).unwrap_or(0);
                self.runs[slot] = Some((target, task, line));
                sex_pdx::serial_record!("[spindle.session.started] target={} task={} scene={}", target, task, line.scene);
            }
            r::PLACED => {
                let Some(run) = self.runs.iter_mut().flatten().find(|s| s.0 == target && s.1 == task) else { return };
                if run.2.scene == value as u8 { return; }
                run.2.scene = value as u8;
                sex_pdx::serial_record!("[spindle.session.placed] target={} task={} scene={}", target, task, value);
            }
            r::ENDED => {
                let Some(slot) = self.runs.iter_mut().find(|s| s.is_some_and(|s| s.0 == target && s.1 == task)) else { return };
                *slot = None;
                sex_pdx::serial_record!("[spindle.session.ended] target={} task={}", target, task);
            }
            _ => return,
        }
        self.dirty = true;
        self.changed_at = sex_pdx::get_ticks();
    }
    /// Shell refused a run: a restored one may try again shortly.
    pub fn refused(&mut self, code: u64) {
        if self.waiting.is_none() { return; }
        use sex_pdx::program_store as s;
        if matches!(code as u32, s::NO_FREE_ROW | s::IMAGE_BUSY) && self.retries < 8 {
            self.retries += 1;
            self.waiting = None;
            // Go back to this run's line: `next` already passed it.
            self.next = self.text[..self.next.saturating_sub(1)].iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        } else { self.waiting = None; self.retries = 0; }
        // Shell takes one run at a time: the refused one is the oldest asked.
        if let Some(i) = self.asked.iter().position(|a| a.is_some()) { self.asked[i] = None; }
    }

    fn send(&mut self, kind: u8, offset: u64, length: u64) -> bool {
        let Some(region) = region() else { return false };
        region[..r::PATH.len()].copy_from_slice(r::PATH);
        let seq = (self.seq + 1) & 0xFF_FFFF;
        let req = f::Request { seq, kind, path_len: r::PATH.len() as u16, offset, length };
        let a = req.args();
        if sex_pdx::pdx_call_checked(sex_pdx::SLOT_STORAGE, f::OP, a[0], a[1], a[2]).is_err() { return false; }
        self.seq = seq;
        self.sent_at = sex_pdx::get_ticks();
        true
    }
    /// One step; `idle` says no other part of Spindle is using the Files region.
    pub fn step(&mut self, idle: bool, sb: &mut Scrollback) -> bool {
        if !self.on { return false; }
        let now = sex_pdx::get_ticks();
        if self.io != Io::Idle {
            // Files restarted and never answered: give up this attempt.
            if now.saturating_sub(self.sent_at) > 15_000 { self.io = Io::Idle; if self.restore == Restore::Reading { self.restore = Restore::Done; } }
            return false;
        }
        match self.restore {
            Restore::NotYet if idle => { if self.send(f::READ, 0, FILE_MAX as u64) { self.io = Io::Read; self.restore = Restore::Reading; } return false; }
            Restore::NotYet | Restore::Reading => return false,
            Restore::Running => return self.restore_next(idle, now, sb),
            Restore::Done => {}
        }
        if self.dirty && idle && now.saturating_sub(self.changed_at) >= 800 {
            let Some(region) = region() else { return false };
            let data = &mut region[f::PATH_BYTES..f::PATH_BYTES + FILE_MAX];
            let mut n = 0;
            for (_, _, line) in self.runs.iter().flatten() {
                if let Some(k) = line.entry().encode(&mut data[n..]) { n += k; }
            }
            self.len = n;
            self.dirty = false;
            if self.send(f::CREATE, 0, 0) { self.io = Io::Create; } else { self.dirty = true; }
        }
        false
    }
    fn restore_next(&mut self, idle: bool, now: u64, sb: &mut Scrollback) -> bool {
        if let Some(at) = self.waiting {
            if now.saturating_sub(at) < 20_000 { return false; }
            self.waiting = None;
        }
        if !idle || self.submitted.is_some() { return false; }
        // A refused run waits a little before it tries again.
        if self.retries > 0 && now.saturating_sub(self.sent_at) < 2_000 { return false; }
        while self.next < self.text_len {
            let end = self.text[self.next..self.text_len].iter().position(|b| *b == b'\n').map_or(self.text_len, |i| self.next + i);
            let line = &self.text[self.next..end];
            self.next = end + 1;
            let Some(e) = r::Entry::decode(line) else { continue };
            let packages = e.args.split(|b| *b == b' ').any(|t| t == sex_pdx::package::GRANT_TOKEN);
            let flags = r::scene_flags(e.scene) | if e.network { sex_pdx::program_store::RUN_NETWORK } else { 0 };
            sex_pdx::serial_record!("[spindle.session.restore] scene={} path={}", e.scene, core::str::from_utf8(e.path).unwrap_or("?"));
            unsafe { crate::fs_commands::FS.submit_program_granted(e.path, e.args, flags, 0, packages, sb); }
            self.waiting = Some(now);
            self.sent_at = now;
            return true;
        }
        self.restore = Restore::Done;
        sex_pdx::serial_record!("[spindle.session.restored]");
        false
    }
    /// Files' answer to a session request; false if it is not one.
    pub fn receive(&mut self, word: u64, sb: &mut Scrollback) -> bool {
        if self.io == Io::Idle { return false; }
        let Some((seq, status, value)) = f::decode_reply(word) else { return false };
        if seq != self.seq { return false; }
        let io = self.io;
        self.io = Io::Idle;
        match io {
            Io::Read => {
                let n = if status == f::OK { (value as usize).min(FILE_MAX) } else { 0 };
                if let Some(region) = region() { self.text[..n].copy_from_slice(&region[f::PATH_BYTES..f::PATH_BYTES + n]); }
                self.text_len = n;
                self.next = 0;
                let count = r::entries(&self.text[..n]).count();
                sex_pdx::serial_record!("[spindle.session.read] status={} programs={}", status, count);
                if count > 0 {
                    sb.push(b"Opening the programs from the last session.");
                    self.restore = Restore::Running;
                    return true;
                }
                self.restore = Restore::Done;
            }
            Io::Create if status == f::OK || status == f::EXISTS => { if self.send(f::TRUNCATE, 0, 0) { self.io = Io::Truncate; } }
            Io::Truncate if status == f::OK => {
                if self.len == 0 { if self.send(f::SYNC, 0, 0) { self.io = Io::Sync; } }
                else if self.send(f::WRITE, 0, self.len as u64) { self.io = Io::Write; }
            }
            Io::Write if status == f::OK => { if self.send(f::SYNC, 0, 0) { self.io = Io::Sync; } }
            Io::Sync => { sex_pdx::serial_record!("[spindle.session.saved] bytes={} status={}", self.len, status); }
            _ => { sex_pdx::serial_record!("[spindle.session.save.failed] status={}", status); self.dirty = true; self.changed_at = sex_pdx::get_ticks(); }
        }
        false
    }
}
