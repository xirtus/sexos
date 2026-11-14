// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Session restore: the programs open when the machine went down come back after it
//! starts again, each on its workspace, with the same arguments and grants the person
//! gave them. Spindle keeps the list (it started those runs) in `PATH`; Shell tells it
//! which run started, where its window is and when it ended.
//!
//! Shell -> Spindle `OP_SESSION`: arg0 = `STARTED` | `PLACED` | `ENDED`,
//! arg1 = row << 32 | task, arg2 = the Files ticket (`STARTED`) or workspace (`PLACED`).
//! Spindle -> Shell: `program_store::OP_RUN`'s flags carry `scene_flags(workspace)` for a
//! restored run, and Shell opens its window there.

pub const OP_SESSION: u64 = 0x5A;
pub const STARTED: u64 = 1;
pub const PLACED: u64 = 2;
pub const ENDED: u64 = 3;
/// The session file, in Spindle's filesystem.
pub const PATH: &[u8] = b"/.session";
/// Runs a session keeps.
pub const MAX_RUNS: usize = 12;
/// A restore runs only this soon after boot, so a restarted Spindle never opens
/// programs a second time.
pub const RESTORE_BEFORE_TICKS: u64 = 120_000;

pub fn scene_flags(scene: u8) -> u64 { ((scene as u64).min(254) + 1) << 8 }
pub fn scene_of(flags: u64) -> Option<u8> { let v = (flags >> 8) & 0xFF; (v != 0).then(|| (v - 1) as u8) }

/// One run as the file keeps it: `SCENE NET PATH ARGS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> { pub scene: u8, pub network: bool, pub path: &'a [u8], pub args: &'a [u8] }

impl<'a> Entry<'a> {
    /// The line for this run, newline ended; None if it does not fit or is not plain.
    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        if self.path.is_empty() || self.path.iter().chain(self.args).any(|b| *b == b'\n' || *b == 0) || self.path.contains(&b' ') || self.scene > 9 { return None; }
        let need = 4 + self.path.len() + if self.args.is_empty() { 0 } else { 1 + self.args.len() } + 1;
        if need > out.len() { return None; }
        out[0] = b'0' + self.scene; out[1] = b' '; out[2] = if self.network { b'1' } else { b'0' }; out[3] = b' ';
        let mut n = 4;
        out[n..n + self.path.len()].copy_from_slice(self.path); n += self.path.len();
        if !self.args.is_empty() { out[n] = b' '; out[n + 1..n + 1 + self.args.len()].copy_from_slice(self.args); n += 1 + self.args.len(); }
        out[n] = b'\n';
        Some(n + 1)
    }
    pub fn decode(line: &'a [u8]) -> Option<Self> {
        if line.len() < 5 || line[1] != b' ' || line[3] != b' ' || !line[0].is_ascii_digit() || !matches!(line[2], b'0' | b'1') { return None; }
        let rest = &line[4..];
        let (path, args) = match rest.iter().position(|b| *b == b' ') { Some(i) => (&rest[..i], &rest[i + 1..]), None => (rest, &rest[rest.len()..]) };
        if path.first() != Some(&b'/') { return None; }
        Some(Self { scene: line[0] - b'0', network: line[2] == b'1', path, args })
    }
}

/// The runs in a session file, oldest first; damaged lines are skipped.
pub fn entries(text: &[u8]) -> impl Iterator<Item = Entry<'_>> {
    text.split(|b| *b == b'\n').filter_map(Entry::decode).take(MAX_RUNS)
}
