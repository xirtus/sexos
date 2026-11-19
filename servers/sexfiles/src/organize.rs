// SPDX-License-Identifier: GPL-3.0-only
//! The Organizer, phase 2: files that arrive in the organized folders (`/Inbox`,
//! `/Downloads`, `/Desktop` and a program's `/data/NAME/Downloads`) are placed by rule
//! once their arrival is durable.
//!
//! Every successful change to such a file is *staged* here (`after`). A staged arrival
//! becomes an *event*, with the next sequence number, only once the filesystem has
//! committed it (`committed`), so nothing is ever placed on the strength of a write a
//! power cut could still take back. Events wait in a bounded ring; when the ring or the
//! staging table overflows, or when Files starts, a full scan of the organized folders
//! replaces the events it lost.
//!
//! With automatic organization on (`/.organizer-enabled`), Files works through the
//! events between requests (`step`): pins first, then the person's rules, then the
//! built-in placements (`sex_organize::plan`). Each move is the identity-checked,
//! no-clobber move with its undo record in one commit (`sexfs::organizer`). The
//! Organizer's own moves never produce new events, so nothing is placed twice.
//!
//! Policy is `sex_organize` (pure, no authority); the durable state is
//! `sexfs::organizer`; this module only joins them, inside the file authority, so it
//! runs whether or not a terminal is open and needs no timer of its own.
use crate::fs_service::Disk;
use alloc::format;
use sex_organize::{Plan, Stay};
use sex_pdx::fs as w;
use sexfs::{organizer as store, Error, Fs, Kind, NAME_MAX};

type F = Fs<'static, Disk>;
const PATH_MAX: usize = 256;
const STAGED_MAX: usize = 32;
const RING: usize = 64;
/// Files looked at in one folder per scan; later ones wait for the next.
const SCAN_FILES: usize = 128;
/// An arrival is placed once nothing has written to it for this long (ms ticks).
/// Creating a file commits at once and its bytes follow in later requests (a large
/// write even commits part way), so a committed arrival may not be whole yet. Programs
/// that write slowly should use a hidden `.NAME.part` and rename it, as Loom's copies and
/// Kaleidoscope's downloads do; hidden names are never placed.
const SETTLE: u64 = 1500;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Op { New = 1, MovedIn = 2 }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome { Waiting, Moved, Stayed(&'static str), Conflict(u32, u32), Failed(u8), Off }
#[derive(Clone, Copy)]
struct Event { seq: u64, op: Op, id: u64, touched: u64, len: u16, path: [u8; PATH_MAX], outcome: Outcome, rule: u32, dest_len: u16, dest: [u8; PATH_MAX] }
const EMPTY: Event = Event { seq: 0, op: Op::New, id: 0, touched: 0, len: 0, path: [0; PATH_MAX], outcome: Outcome::Waiting, rule: 0, dest_len: 0, dest: [0; PATH_MAX] };
#[derive(Clone, Copy)]
struct Staged { op: Op, id: u64, touched: u64, len: u16, path: [u8; PATH_MAX] }

static mut STAGED: [Option<Staged>; STAGED_MAX] = [None; STAGED_MAX];
static mut EVENTS: [Event; RING] = [EMPTY; RING];
/// The last sequence handed out, and the last one worked through.
static mut SEQ: u64 = 0;
static mut DONE: u64 = 0;
/// Automatic organization, as last read from disk (None: not read yet).
static mut ENABLED: Option<bool> = None;
/// A full scan is owed: Files started, events were lost, or it was just switched on.
static mut SCAN_OWED: bool = false;
/// Uncommitted changes are in the way (a failed commit): wait for the next commit.
static mut STALLED: bool = false;
static mut RULES: [u8; store::RULES_MAX] = [0; store::RULES_MAX];
static mut RULES_LEN: Option<usize> = None;
static mut NAMES: [[u8; NAME_MAX]; SCAN_FILES] = [[0; NAME_MAX]; SCAN_FILES];
static mut NAME_LENS: [u8; SCAN_FILES] = [0; SCAN_FILES];

fn staged() -> &'static mut [Option<Staged>; STAGED_MAX] { unsafe { &mut *core::ptr::addr_of_mut!(STAGED) } }
fn events() -> &'static mut [Event; RING] { unsafe { &mut *core::ptr::addr_of_mut!(EVENTS) } }
fn shown(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap_or("?") }
fn file_name(path: &[u8]) -> &[u8] { path.rsplit(|&b| b == b'/').next().unwrap_or(path) }

/// Called after every successful request on the SexOS filesystem: stages an arrival
/// when the request left a file in an organized folder, then publishes what the
/// filesystem has committed.
pub fn after(fs: &mut F, kind: u8, path: &[u8]) {
    let (op, target) = match kind {
        w::CREATE | w::WRITE | w::TRUNCATE => (Op::New, path),
        w::RENAME | w::MOVE_IF_ID => match path.iter().position(|b| *b == 0) { Some(i) => (Op::MovedIn, &path[i + 1..]), None => return },
        _ => { committed(fs); return; }
    };
    if sex_organize::ingress(target) && !sex_organize::unsettled(file_name(target)) && target.len() <= PATH_MAX {
        if let Ok(st) = fs.open(target) {
            if st.kind == Kind::File { stage(op, st.id, target); }
        }
    }
    committed(fs);
}

fn stage(op: Op, id: u64, path: &[u8]) {
    let now = sex_pdx::get_ticks();
    // Still being written after its arrival was published: the same event, later.
    let done = unsafe { DONE };
    if let Some(e) = events().iter_mut().find(|e| e.seq > done && e.id == id && e.outcome == Outcome::Waiting) {
        e.touched = now;
        if op == Op::MovedIn { e.op = op; e.len = path.len() as u16; e.path[..path.len()].copy_from_slice(path); }
        return;
    }
    let table = staged();
    // One file written in many requests is one arrival.
    let slot = match table.iter().position(|s| s.is_some_and(|s| s.id == id)) {
        Some(i) => i,
        None => match table.iter().position(|s| s.is_none()) {
            Some(i) => i,
            None => { unsafe { SCAN_OWED = true; } sex_pdx::serial_record!("[files.organize.staged.full] scan=1"); return; }
        },
    };
    let mut s = Staged { op, id, touched: now, len: path.len() as u16, path: [0; PATH_MAX] };
    if let Some(old) = table[slot] { if old.op == Op::MovedIn { s.op = Op::MovedIn; } }
    s.path[..path.len()].copy_from_slice(path);
    table[slot] = Some(s);
}

/// After a commit: what was staged is durable now, so it becomes events.
pub fn committed(fs: &mut F) {
    if fs.has_changes() { return; }
    unsafe { STALLED = false; }
    for slot in staged().iter_mut() {
        let Some(s) = slot.take() else { continue };
        let seq = unsafe { SEQ += 1; SEQ };
        let e = &mut events()[(seq % RING as u64) as usize];
        // Overwriting an event not yet worked through: a scan catches up instead.
        if e.seq != 0 && e.seq > unsafe { DONE } && unsafe { ENABLED } == Some(true) {
            unsafe { SCAN_OWED = true; }
            sex_pdx::serial_record!("[files.organize.events.lost] seq={} scan=1", e.seq);
        }
        *e = Event { seq, op: s.op, id: s.id, touched: s.touched, len: s.len, path: s.path, ..EMPTY };
        sex_pdx::serial_record!("[files.organize.event] seq={} op={} id={} path={}", seq, if s.op == Op::New { "new" } else { "moved-in" }, s.id, shown(&s.path[..s.len as usize]));
    }
}

fn enabled(fs: &mut F) -> bool {
    if let Some(on) = unsafe { ENABLED } { return on; }
    let on = store::enabled(fs).unwrap_or(false);
    unsafe { ENABLED = Some(on); }
    on
}

fn rules(fs: &mut F) -> &'static [u8] {
    unsafe {
        if matches!({ RULES_LEN }, None) { RULES_LEN = Some(store::rules(fs, &mut *core::ptr::addr_of_mut!(RULES)).unwrap_or(0)); }
        &(&*core::ptr::addr_of!(RULES))[..RULES_LEN.unwrap_or(0)]
    }
}

/// Work is waiting for the idle loop.
pub fn pending() -> bool {
    unsafe { !STALLED && (ENABLED == Some(true) && (SCAN_OWED || DONE < SEQ) || (matches!({ ENABLED }, None) && SCAN_OWED)) }
}

/// Between requests, with nothing uncommitted: one event, or one full scan.
pub fn step(fs: &mut F) {
    if fs.has_changes() { unsafe { STALLED = true; } return; }
    if !enabled(fs) {
        // Off: events are still logged (for `organize log`) but nothing moves.
        unsafe { SCAN_OWED = false; DONE = SEQ; }
        return;
    }
    if unsafe { SCAN_OWED } {
        // A file still being written waits; the scan comes once arrivals have settled.
        let (done, now) = (unsafe { DONE }, sex_pdx::get_ticks());
        if events().iter().any(|e| e.seq > done && now.saturating_sub(e.touched) < SETTLE) { return; }
        unsafe { SCAN_OWED = false; DONE = SEQ; }
        let (moved, looked) = scan(fs, false, &mut |_| {});
        sex_pdx::serial_record!("[files.organize.scan] moved={} looked={} auto=1", moved, looked);
        return;
    }
    let seq = unsafe { DONE + 1 };
    let e = &mut events()[(seq % RING as u64) as usize];
    if e.seq != seq { unsafe { DONE = seq; } return; }
    // Not settled yet: Files keeps polling between requests until it is.
    if sex_pdx::get_ticks().saturating_sub(e.touched) < SETTLE { return; }
    unsafe { DONE = seq; }
    let path = e.path;
    let (outcome, rule, dest, dest_len) = place(fs, &path[..e.len as usize], Some(e.id), false);
    e.outcome = outcome; e.rule = rule; e.dest = dest; e.dest_len = dest_len as u16;
}

/// Decides one file and, unless only reporting, moves it. Returns what happened, the
/// rule, and the destination path.
fn place(fs: &mut F, path: &[u8], expected: Option<u64>, report: bool) -> (Outcome, u32, [u8; PATH_MAX], usize) {
    let mut dest = [0u8; PATH_MAX];
    let st = match fs.open(path) {
        Ok(st) if st.kind == Kind::File => st,
        Ok(_) => return (Outcome::Stayed("a folder"), 0, dest, 0),
        Err(_) => return (Outcome::Stayed("gone"), 0, dest, 0),
    };
    if expected.is_some_and(|id| id != st.id) { return (Outcome::Stayed("replaced since"), 0, dest, 0); }
    let pinned = store::pinned(fs, st.id).unwrap_or(false);
    let rules = rules(fs);
    let (folder, rule) = match sex_organize::plan(path, st.size, pinned, rules) {
        Plan::Move { destination, rule_id } => (destination, rule_id),
        Plan::Stay(Stay::Pinned) => return (Outcome::Stayed("pinned"), 0, dest, 0),
        Plan::Stay(Stay::AlreadyPlaced) => return (Outcome::Stayed("already in place"), 0, dest, 0),
        Plan::Stay(_) => return (Outcome::Stayed("no rule"), 0, dest, 0),
        Plan::Conflict { first, second } => return (Outcome::Conflict(first, second), 0, dest, 0),
        Plan::Ignore => return (Outcome::Stayed("not organized"), 0, dest, 0),
    };
    let Ok(n) = sex_organize::join_destination(folder, file_name(path), &mut dest) else { return (Outcome::Failed(w::NAME), rule, dest, 0) };
    if report { return (Outcome::Waiting, rule, dest, n); }
    let result = make_folders(fs, folder).and_then(|_| store::move_and_record(fs, path, &dest[..n], st.id)).and_then(|_| fs.commit());
    match result {
        Ok(()) => {
            crate::fs_service::forget_all_stats();
            sex_pdx::serial_record!("[files.organize.moved] rule={} from={} to={}", rule, shown(path), shown(&dest[..n]));
            (Outcome::Moved, rule, dest, n)
        }
        Err(e) => {
            if !matches!(e, Error::Io | Error::Corrupt | Error::Poisoned) { let _ = fs.rollback(); }
            sex_pdx::serial_record!("[files.organize.stayed] rule={} from={} why={}", rule, shown(path), shown(w::message(crate::fs_service::status(e))));
            (Outcome::Failed(crate::fs_service::status(e)), rule, dest, n)
        }
    }
}

/// `/Documents/Taxes` and every folder above it, as needed, in the open transaction.
fn make_folders(fs: &mut F, folder: &[u8]) -> Result<(), Error> {
    if !sex_organize::destination_allowed(folder) { return Err(Error::Name); }
    let now = sex_pdx::civil_time_ms().map(|ms| ms / 1000).unwrap_or(0);
    for (i, b) in folder.iter().enumerate().skip(1).chain(core::iter::once((folder.len(), &b'/'))) {
        if *b != b'/' { continue; }
        match fs.open(&folder[..i]) {
            Ok(st) if st.kind == Kind::Dir => {}
            Ok(_) => return Err(Error::NotDir),
            Err(Error::NotFound) => { fs.mkdir(&folder[..i], now)?; }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Every organized folder, once. `say` gets one report line per file looked at.
/// Returns (moved, looked at).
fn scan(fs: &mut F, report: bool, say: &mut dyn FnMut(&str)) -> (u32, u32) {
    let (mut moved, mut looked) = (0, 0);
    for dir in [&b"/Inbox"[..], b"/Downloads", b"/Desktop"] {
        scan_dir(fs, dir, report, say, &mut moved, &mut looked);
    }
    // Programs' own Downloads folders.
    let Ok(data) = fs.open(sex_pdx::program_store::DATA.strip_suffix(b"/").unwrap_or(b"/data")) else { return (moved, looked) };
    let mut cursor = 0;
    let mut name = [0u8; NAME_MAX];
    for _ in 0..64 {
        let Ok(Some((len, kind, _, next))) = fs.read_dir(data.id, cursor, &mut name) else { break };
        cursor = next;
        if kind != Kind::Dir { continue; }
        let mut dir = [0u8; PATH_MAX];
        let text = format!("/data/{}/Downloads", shown(&name[..len]));
        if text.len() > dir.len() { continue; }
        dir[..text.len()].copy_from_slice(text.as_bytes());
        scan_dir(fs, &dir[..text.len()], report, say, &mut moved, &mut looked);
    }
    (moved, looked)
}

fn scan_dir(fs: &mut F, dir: &[u8], report: bool, say: &mut dyn FnMut(&str), moved: &mut u32, looked: &mut u32) {
    let Ok(st) = fs.open(dir) else { return };
    if st.kind != Kind::Dir { return; }
    // Names first, then moves: moving changes the folder being read.
    let names = unsafe { &mut *core::ptr::addr_of_mut!(NAMES) };
    let lens = unsafe { &mut *core::ptr::addr_of_mut!(NAME_LENS) };
    let (mut n, mut cursor) = (0, 0);
    while n < SCAN_FILES {
        let Ok(Some((len, kind, _, next))) = fs.read_dir(st.id, cursor, &mut names[n]) else { break };
        cursor = next;
        if kind == Kind::File && !sex_organize::unsettled(&names[n][..len]) && len <= 255 { lens[n] = len as u8; n += 1; }
    }
    for i in 0..n {
        let mut path = [0u8; PATH_MAX];
        let Ok(len) = sex_organize::join_destination(dir, &names[i][..lens[i] as usize], &mut path) else { continue };
        *looked += 1;
        let (outcome, rule, dest, dest_len) = place(fs, &path[..len], None, report);
        let line = match outcome {
            Outcome::Moved => { *moved += 1; format!("moved {} -> {} (rule {})", shown(&path[..len]), shown(&dest[..dest_len]), rule) }
            Outcome::Waiting => format!("would move {} -> {} (rule {})", shown(&path[..len]), shown(&dest[..dest_len]), rule),
            other => format!("stays {}: {}", shown(&path[..len]), why(other)),
        };
        say(&line);
    }
}

fn why(o: Outcome) -> alloc::string::String {
    match o {
        Outcome::Stayed(s) => s.into(),
        Outcome::Conflict(a, b) => format!("rules {} and {} disagree", a, b),
        Outcome::Failed(st) => shown(w::message(st)).into(),
        Outcome::Waiting => "waiting".into(),
        Outcome::Off => "automatic organization is off".into(),
        Outcome::Moved => "moved".into(),
    }
}

/// Lines into `data`, as far as they fit; returns the length.
struct Out<'a> { data: &'a mut [u8], len: usize, full: bool }
impl Out<'_> {
    fn line(&mut self, s: &str) {
        if self.full { return; }
        if self.len + s.len() + 1 > self.data.len() { self.full = true; return; }
        self.data[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
        self.data[self.len + s.len()] = b'\n';
        self.len += s.len() + 1;
    }
}

/// Files was (re)started on this filesystem: read the switch again and reconcile.
pub fn mounted() {
    unsafe { ENABLED = None; RULES_LEN = None; SCAN_OWED = true; STALLED = false; }
}
/// No SexOS filesystem (yet): nothing to organize until one is mounted.
pub fn unmounted() {
    unsafe { ENABLED = Some(false); SCAN_OWED = false; }
}

/// The organizer requests (Spindle only; checked by the caller). Pending writes are
/// already committed.
pub fn request(fs: &mut F, r: w::Request, path: &[u8], data: &mut [u8]) -> Result<u32, u8> {
    let st = |e: Error| crate::fs_service::status(e);
    let commit = |fs: &mut F| fs.commit().map_err(|e| { let _ = fs.rollback(); st(e) });
    match r.kind {
        w::ORGANIZE_STATE => {
            if !path.is_empty() || data.len() < 21 { return Err(w::NAME); }
            let on = enabled(fs);
            let pin_count = pin_count(fs);
            let rule_count = rules(fs).split(|b| *b == b'\n').filter(|l| !l.is_empty()).count();
            data[0] = on as u8;
            data[1..9].copy_from_slice(&fs.statfs().generation.to_le_bytes());
            data[9..17].copy_from_slice(&unsafe { SEQ }.to_le_bytes());
            data[17..19].copy_from_slice(&(pin_count as u16).to_le_bytes());
            data[19..21].copy_from_slice(&(rule_count as u16).to_le_bytes());
            Ok(21)
        }
        w::ORGANIZE_SET => {
            if !path.is_empty() || r.offset > 1 { return Err(w::NAME); }
            store::set_enabled(fs, r.offset == 1).map_err(st)?;
            commit(fs)?;
            unsafe { ENABLED = Some(r.offset == 1); SCAN_OWED = r.offset == 1; DONE = SEQ; }
            sex_pdx::serial_record!("[files.organize.auto] enabled={}", r.offset);
            Ok(r.offset as u32)
        }
        w::ORGANIZE_MOVE => {
            // An explicit placement ("put this file there"): the person's override of the
            // rules, checked like the Organizer's own and just as undoable.
            let split = path.iter().position(|b| *b == 0).ok_or(w::NAME)?;
            let (from, to) = (&path[..split], &path[split + 1..]);
            let parent = &to[..to.len() - file_name(to).len()];
            let folder = parent.strip_suffix(b"/").unwrap_or(b"");
            if !sex_organize::ingress(from) || !sex_organize::destination_allowed(folder) { return Err(w::DENIED); }
            // The person named the file just now: offset 0 means "the file there now".
            let id = if r.offset != 0 { r.offset } else { fs.open(from).map_err(st)?.id };
            make_folders(fs, folder).map_err(|e| { let _ = fs.rollback(); st(e) })?;
            store::move_and_record(fs, from, to, id).map_err(|e| { let _ = fs.rollback(); st(e) })?;
            commit(fs)?;
            crate::fs_service::forget_all_stats();
            sex_pdx::serial_record!("[files.organize.moved] rule=0 from={} to={}", shown(from), shown(to));
            Ok(0)
        }
        w::ORGANIZE_UNDO => {
            if !path.is_empty() { return Err(w::NAME); }
            let mut record = [0u8; store::RECORD_BYTES];
            let shown_move = store::last(fs, &mut record).ok().flatten();
            let left = store::undo_last(fs).map_err(|e| { let _ = fs.rollback(); st(e) })?;
            commit(fs)?;
            crate::fs_service::forget_all_stats();
            let mut n = 0;
            if let Some((a, b)) = shown_move {
                // The record holds from then to at bytes 16..; hand back `from\0to`.
                if a + b + 1 <= data.len() {
                    data[..a].copy_from_slice(&record[16..16 + a]);
                    data[a] = 0;
                    data[a + 1..a + 1 + b].copy_from_slice(&record[16 + a..16 + a + b]);
                    n = a + 1 + b;
                    sex_pdx::serial_record!("[files.organize.undone] to={} from={} pinned=1 left={}", shown(&record[16..16 + a]), shown(&record[16 + a..16 + a + b]), left);
                }
            }
            Ok(n as u32)
        }
        w::ORGANIZE_SCAN => {
            if !path.is_empty() || r.offset > 1 { return Err(w::NAME); }
            let mut out = Out { data, len: 0, full: false };
            let (moved, looked) = scan(fs, r.offset == 1, &mut |l| out.line(l));
            sex_pdx::serial_record!("[files.organize.scan] moved={} looked={} auto=0 report={}", moved, looked, r.offset);
            Ok(out.len as u32)
        }
        w::ORGANIZE_PIN => {
            if r.offset > 1 { return Err(w::NAME); }
            let f = fs.open(path).map_err(st)?;
            if f.kind != Kind::File { return Err(w::IS_DIR); }
            store::pin(fs, f.id, r.offset == 1).map_err(|e| { let _ = fs.rollback(); st(e) })?;
            commit(fs)?;
            sex_pdx::serial_record!("[files.organize.pin] path={} on={}", shown(path), r.offset);
            Ok(0)
        }
        w::ORGANIZE_RULE => {
            if path.is_empty() {
                let text = rules(fs);
                if text.len() > data.len() { return Err(w::NAME); }
                data[..text.len()].copy_from_slice(text);
                return Ok(text.len() as u32);
            }
            let (pattern, dest) = match path.iter().position(|b| *b == 0) { Some(i) => (&path[..i], Some(&path[i + 1..])), None => (path, None) };
            let mut line = [0u8; 512];
            let mut next = [0u8; store::RULES_MAX];
            let mut len = 0;
            // The other patterns stay, in order; this one is replaced or dropped.
            let mut found = false;
            for l in rules(fs).split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                if l.split(|b| *b == b' ').next() == Some(pattern) { found = true; continue; }
                if len + l.len() + 1 > next.len() { return Err(w::NO_SPACE); }
                next[len..len + l.len()].copy_from_slice(l); next[len + l.len()] = b'\n'; len += l.len() + 1;
            }
            if let Some(dest) = dest {
                let n = pattern.len() + 1 + dest.len();
                if n > line.len() { return Err(w::NAME); }
                line[..pattern.len()].copy_from_slice(pattern); line[pattern.len()] = b' '; line[pattern.len() + 1..n].copy_from_slice(dest);
                if sex_organize::parse_rule(&line[..n]).is_none() { return Err(w::NAME); }
                if next[..len].split(|b| *b == b'\n').filter(|l| !l.is_empty()).count() >= sex_organize::USER_RULES_MAX { return Err(w::NO_SPACE); }
                if len + n + 1 > next.len() { return Err(w::NO_SPACE); }
                next[len..len + n].copy_from_slice(&line[..n]); next[len + n] = b'\n'; len += n + 1;
            } else if !found { return Err(w::NOT_FOUND); }
            store::set_rules(fs, &next[..len]).map_err(|e| { let _ = fs.rollback(); st(e) })?;
            commit(fs)?;
            unsafe { RULES_LEN = None; }
            sex_pdx::serial_record!("[files.organize.rule] pattern={} dest={}", shown(pattern), dest.map(shown).unwrap_or("-"));
            Ok(0)
        }
        w::ORGANIZE_LOG => {
            if !path.is_empty() { return Err(w::NAME); }
            let mut out = Out { data, len: 0, full: false };
            let last = unsafe { SEQ };
            let first = r.offset.max(last.saturating_sub(RING as u64 - 1)).max(1);
            let on = enabled(fs);
            for seq in first..=last {
                let e = events()[(seq % RING as u64) as usize];
                if e.seq != seq { continue; }
                let p = shown(&e.path[..e.len as usize]);
                let op = if e.op == Op::New { "new" } else { "moved-in" };
                let what = match e.outcome {
                    Outcome::Moved => format!("moved to {} (rule {})", shown(&e.dest[..e.dest_len as usize]), e.rule),
                    Outcome::Waiting if !on || seq <= unsafe { DONE } => why(Outcome::Off),
                    Outcome::Waiting => "waiting".into(),
                    other => format!("stays: {}", why(other)),
                };
                out.line(&format!("{} {} {}: {}", seq, op, p, what));
            }
            Ok(out.len as u32)
        }
        _ => Err(w::INVALID),
    }
}

fn pin_count(fs: &mut F) -> usize {
    let mut ids = [0u64; store::MAX_PINS as usize];
    store::pins(fs, &mut ids).unwrap_or(0)
}
