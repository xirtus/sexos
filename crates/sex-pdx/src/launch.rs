// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Programs starting programs: "open this file" and "run this program", asked of the
//! Shell by any program. A started run gets only what its starter already holds.
//!
//! 1. The starter asks Files to PREPARE the run (`fs::PREPARE`, path `/bin/NAME` or a
//!    program file its own grant covers, the argument line after it). For a program,
//!    Files refuses unless every `@/path` in the arguments is a path the starter's own
//!    grant covers, and `+packages` only from a starter that holds it. Files answers
//!    with a ticket.
//! 2. The starter sends Shell `OP_LAUNCH`: arg0 = the ticket, arg1 = flags
//!    (`program_store::RUN_NETWORK` only from a run that has the network itself),
//!    arg2 = a cookie, the starter's own name for this run (32 bits).
//! 3. Shell answers with `OP_EVENT` messages to the starter: `STARTED` with the child's
//!    row and task, or `REFUSED` with a `program_store` refusal code; later `DONE` with
//!    the exit status once the child's run has ended.
//! 4. `OP_KILL` (arg0 = cookie) ends a child the starter started.
//!
//! A run reports its own exit status with `OP_EXIT` (arg0 = status) just before it
//! finishes; `sex_std`'s `exit` does. A run that ends without saying (it crashed or was
//! killed) is `DONE` with `NO_STATUS`.
//!
//! Opening a file is a run too: `program_for` names the program for a path's type, and
//! `open_args` makes the argument line that hands it exactly that file.

/// Program -> Shell: start the prepared run. arg0 ticket, arg1 flags, arg2 cookie.
pub const OP_LAUNCH: u64 = 0x56;
/// Program -> Shell: end the child started with this cookie (arg0).
pub const OP_KILL: u64 = 0x57;
/// Program -> Shell: my run is ending with this status (arg0).
pub const OP_EXIT: u64 = 0x58;
/// Shell -> starter: arg0 = kind << 32 | cookie, arg1 = child row << 32 | child task,
/// arg2 = refusal code (`REFUSED`) or exit status (`DONE`).
pub const OP_EVENT: u64 = 0x59;

pub const STARTED: u32 = 1;
pub const REFUSED: u32 = 2;
pub const DONE: u32 = 3;
/// `DONE` for a run that ended without reporting a status.
pub const NO_STATUS: u64 = 1 << 32;

/// How many started runs Shell follows at once.
pub const MAX_RUNS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Started { cookie: u32, row: u32, task: u32 },
    Refused { cookie: u32, code: u32 },
    /// `status` is None when the run ended without saying.
    Done { cookie: u32, row: u32, task: u32, status: Option<u32> },
}

impl Event {
    pub fn wire(&self) -> [u64; 3] {
        let pack = |kind: u32, cookie: u32| (kind as u64) << 32 | cookie as u64;
        let run = |row: u32, task: u32| (row as u64) << 32 | task as u64;
        match *self {
            Event::Started { cookie, row, task } => [pack(STARTED, cookie), run(row, task), 0],
            Event::Refused { cookie, code } => [pack(REFUSED, cookie), 0, code as u64],
            Event::Done { cookie, row, task, status } => [pack(DONE, cookie), run(row, task), status.map_or(NO_STATUS, |s| s as u64)],
        }
    }
    pub fn decode(op: u64, w: [u64; 3]) -> Option<Self> {
        if op != OP_EVENT { return None; }
        let (cookie, row, task) = (w[0] as u32, (w[1] >> 32) as u32, w[1] as u32);
        Some(match (w[0] >> 32) as u32 {
            STARTED => Event::Started { cookie, row, task },
            REFUSED => Event::Refused { cookie, code: w[2] as u32 },
            DONE => Event::Done { cookie, row, task, status: (w[2] != NO_STATUS).then_some(w[2] as u32) },
            _ => return None,
        })
    }
    pub fn cookie(&self) -> u32 {
        match *self { Event::Started { cookie, .. } | Event::Refused { cookie, .. } | Event::Done { cookie, .. } => cookie }
    }
}

/// The program that opens a file of this name, by its extension (case ignored).
pub fn program_for(path: &[u8]) -> Option<&'static [u8]> {
    let name = path.rsplit(|b| *b == b'/').next()?;
    let dot = name.iter().rposition(|b| *b == b'.')?;
    let ext = &name[dot + 1..];
    if ext.is_empty() || ext.len() > 5 { return None; }
    let mut low = [0u8; 5];
    for (d, s) in low.iter_mut().zip(ext) { *d = s.to_ascii_lowercase(); }
    let ext = &low[..ext.len()];
    const TABLE: &[(&[u8], &[&[u8]])] = &[
        (b"pdf", &[b"pdf"]),
        (b"play", &[b"mp4", b"m4v", b"mov", b"webm", b"mkv", b"ogv", b"avi", b"mp3", b"m4a", b"aac", b"ogg", b"opus", b"oga", b"flac", b"wav"]),
        (b"view", &[b"png", b"jpg", b"jpeg", b"webp", b"gif", b"bmp"]),
        (b"jot", &[b"txt", b"md", b"rs", b"toml", b"json", b"c", b"h", b"cpp", b"py", b"lua", b"sh", b"csv", b"log", b"ini", b"cfg", b"conf", b"yaml", b"yml", b"xml"]),
    ];
    TABLE.iter().find(|(_, exts)| exts.contains(&ext)).map(|(program, _)| *program)
}

/// The argument line that gives a program exactly one absolute path: `@/that/path`.
/// None for a path that is not absolute and plain, or too long.
pub fn open_args(path: &[u8], out: &mut [u8]) -> Option<usize> {
    if path.first() != Some(&b'/') || path.contains(&b' ') || path.contains(&0)
        || path.split(|b| *b == b'/').any(|p| p == b".." || p == b".") || path.len() + 1 > out.len() { return None; }
    out[0] = b'@';
    out[1..1 + path.len()].copy_from_slice(path);
    Some(1 + path.len())
}

/// `/bin/NAME` for a bare program name.
pub fn bin_path(name: &[u8], out: &mut [u8]) -> Option<usize> {
    let bin = crate::program_store::BIN;
    if name.is_empty() || name.contains(&b'/') || crate::program_store::pack_name(name).is_none() || bin.len() + name.len() > out.len() { return None; }
    out[..bin.len()].copy_from_slice(bin);
    out[bin.len()..bin.len() + name.len()].copy_from_slice(name);
    Some(bin.len() + name.len())
}

/// Files' check of a program's PREPARE: every grant in the argument line must be one
/// the starter holds. `holds(path)` answers for an `@/path` token, `packages` says
/// whether the starter holds `+packages`. True if the line may pass.
pub fn args_allowed(args: &[u8], packages: bool, mut holds: impl FnMut(&[u8]) -> bool) -> bool {
    args.split(|b| *b == b' ').filter(|t| !t.is_empty()).all(|t| {
        if t == crate::package::GRANT_TOKEN { packages }
        else if t.first() == Some(&b'@') { holds(t) }
        else { true }
    })
}
