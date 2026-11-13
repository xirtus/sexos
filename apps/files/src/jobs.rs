// SPDX-License-Identifier: GPL-3.0-only
//! Background work in Files: copying, moving between disks, deleting forever and
//! checksumming, one piece at a time between keys, so the window stays usable, shows
//! progress, and Esc stops it.
//!
//! A file being copied is written under a hidden `.NAME.part` name and gets its real
//! name only when every byte is there, so a cancelled copy, a closed window or a power
//! cut never leaves a file that looks whole but is not. Cancelling removes everything
//! the job made.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::files;
use sex_app::Window;
use sha2::{Digest, Sha256};

use crate::{at, base, human, join, parent, shown};

/// How much one step moves: one round trip to Files each way.
const PIECE: usize = 192 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind { Copy, Move, Delete, Sum }

enum Step {
    MakeDir(Vec<u8>),
    /// Copies `from` into `part`, then renames `part` to `to`.
    CopyFile { from: Vec<u8>, part: Vec<u8>, to: Vec<u8>, size: u64, offset: u64, started: bool },
    Remove(Vec<u8>),
    Sum { path: Vec<u8>, size: u64, offset: u64 },
}

pub enum Outcome {
    Running,
    Done(Kind, String),
    Failed(Kind, String),
}

pub struct Job {
    pub kind: Kind,
    /// What it is about, as shown: the top file or folder.
    pub what: Vec<u8>,
    pub from: Vec<u8>,
    pub to: Vec<u8>,
    steps: VecDeque<Step>,
    /// Paths this job created, in order: removed in reverse on cancel.
    made: Vec<Vec<u8>>,
    /// The `.part` file being written now, if any.
    open_part: Option<Vec<u8>>,
    pub done: u64,
    pub total: u64,
    removed: u64,
    started_at: u64,
    hash: Option<Sha256>,
    buf: Vec<u8>,
    /// Steps after this many remain only removing the source of a move: past the
    /// point where cancelling could keep both copies safe.
    pub committed: bool,
}

fn ticks() -> u64 { sex_pdx::get_ticks() }

/// The hidden name a file is written under until it is whole.
fn part_name(to: &[u8]) -> Vec<u8> {
    let mut name = b".".to_vec();
    name.extend_from_slice(base(to));
    name.extend_from_slice(b".part");
    join(&parent(to), &name)
}

/// Every file and folder under `from`, in an order where folders come before their
/// contents (`pre`) or after them (`!pre`).
fn walk(window: &mut Window, from: &[u8], pre: bool, out: &mut Vec<(Vec<u8>, bool, u64)>) -> Result<(), u8> {
    let st = files::stat(window, &at(from))?;
    if st.dir {
        if pre { out.push((from.to_vec(), true, 0)); }
        for e in files::list(window, &at(from))? { walk(window, &join(from, &e.name), pre, out)?; }
        if !pre { out.push((from.to_vec(), true, 0)); }
    } else {
        out.push((from.to_vec(), false, st.size));
    }
    Ok(())
}

impl Job {
    fn new(kind: Kind, what: &[u8], from: &[u8], to: &[u8]) -> Self {
        Job { kind, what: what.to_vec(), from: from.to_vec(), to: to.to_vec(), steps: VecDeque::new(), made: Vec::new(), open_part: None,
              done: 0, total: 0, removed: 0, started_at: ticks(), hash: None, buf: Vec::new(), committed: false }
    }

    fn plan_copy(&mut self, window: &mut Window, from: &[u8], to: &[u8]) -> Result<(), u8> {
        let mut all = Vec::new();
        walk(window, from, true, &mut all)?;
        for (path, dir, size) in all {
            let dest = { let mut d = to.to_vec(); d.extend_from_slice(&path[from.len()..]); d };
            if dir { self.steps.push_back(Step::MakeDir(dest)); }
            else {
                self.total += size;
                self.steps.push_back(Step::CopyFile { from: path, part: part_name(&dest), to: dest, size, offset: 0, started: false });
            }
        }
        Ok(())
    }
    fn plan_remove(&mut self, window: &mut Window, path: &[u8]) -> Result<(), u8> {
        let mut all = Vec::new();
        walk(window, path, false, &mut all)?;
        for (p, _, _) in all { self.steps.push_back(Step::Remove(p)); }
        Ok(())
    }

    /// Copies `from` (a file or folder) to `to`, which must not exist yet.
    pub fn copy(window: &mut Window, from: &[u8], to: &[u8]) -> Result<Self, u8> {
        let mut job = Job::new(Kind::Copy, base(to), from, to);
        job.plan_copy(window, from, to)?;
        Ok(job)
    }
    /// Moves between disks: copies, then removes the source once every copy is whole.
    pub fn move_across(window: &mut Window, from: &[u8], to: &[u8]) -> Result<Self, u8> {
        let mut job = Job::new(Kind::Move, base(to), from, to);
        job.plan_copy(window, from, to)?;
        job.plan_remove(window, from)?;
        Ok(job)
    }
    /// Deletes each of `paths` and everything inside them, for good.
    pub fn delete(window: &mut Window, what: &[u8], paths: &[Vec<u8>]) -> Result<Self, u8> {
        let mut job = Job::new(Kind::Delete, what, b"", b"");
        for p in paths { job.plan_remove(window, p)?; }
        job.total = job.steps.len() as u64;
        Ok(job)
    }
    /// Reads a file through and gives its SHA-256.
    pub fn checksum(window: &mut Window, path: &[u8]) -> Result<Self, u8> {
        let st = files::stat(window, &at(path))?;
        if st.dir { return Err(sex_pdx::fs::IS_DIR); }
        let mut job = Job::new(Kind::Sum, base(path), path, b"");
        job.total = st.size;
        job.hash = Some(Sha256::new());
        job.steps.push_back(Step::Sum { path: path.to_vec(), size: st.size, offset: 0 });
        Ok(job)
    }

    /// Does one piece of work. Call again until it is not `Running`.
    pub fn step(&mut self, window: &mut Window) -> Outcome {
        match self.step_inner(window) {
            Ok(true) => Outcome::Running,
            Ok(false) => Outcome::Done(self.kind, self.summary()),
            Err(e) => {
                let why = shown(files::message(e)).into();
                // A copy that failed is cleaned like a cancelled one; a move keeps
                // its source whenever the copy did not finish.
                if self.kind != Kind::Delete && self.kind != Kind::Sum && !self.committed { self.undo_made(window); }
                Outcome::Failed(self.kind, why)
            }
        }
    }

    fn step_inner(&mut self, window: &mut Window) -> Result<bool, u8> {
        let Some(step) = self.steps.front_mut() else { return Ok(false) };
        match step {
            Step::MakeDir(path) => {
                let path = core::mem::take(path);
                files::mkdir(window, &at(&path))?;
                self.made.push(path);
                self.steps.pop_front();
            }
            Step::CopyFile { from, part, to, size, offset, started } => {
                if !*started {
                    // A leftover from an earlier stopped run goes first.
                    let _ = files::remove(window, &at(part));
                    files::create(window, &at(part))?;
                    *started = true;
                    self.open_part = Some(part.clone());
                }
                if *offset < *size {
                    if self.buf.len() != PIECE { self.buf = vec![0u8; PIECE]; }
                    let want = ((*size - *offset) as usize).min(PIECE);
                    let n = files::read_at(window, &at(from), *offset, &mut self.buf[..want])?;
                    if n == 0 { return Err(sex_pdx::fs::IO); }
                    let wrote = files::write_at(window, &at(part), *offset, &self.buf[..n])?;
                    if wrote < n { return Err(sex_pdx::fs::NO_SPACE); }
                    *offset += n as u64;
                    self.done += n as u64;
                }
                if *offset >= *size {
                    files::rename(window, &at(part), &at(to))?;
                    self.made.push(core::mem::take(to));
                    self.open_part = None;
                    self.steps.pop_front();
                    // The copy is whole: what is left of a move only removes the source.
                    if self.kind == Kind::Move && !self.steps.iter().any(|s| matches!(s, Step::CopyFile { .. } | Step::MakeDir(_))) {
                        let _ = files::sync(window);
                        self.committed = true;
                    }
                }
            }
            Step::Remove(path) => {
                let path = core::mem::take(path);
                files::remove(window, &at(&path))?;
                self.removed += 1;
                if self.kind == Kind::Delete { self.done += 1; }
                self.steps.pop_front();
            }
            Step::Sum { path, size, offset } => {
                if self.buf.len() != PIECE { self.buf = vec![0u8; PIECE]; }
                let want = ((*size - *offset) as usize).min(PIECE);
                if want > 0 {
                    let n = files::read_at(window, &at(path), *offset, &mut self.buf[..want])?;
                    if n == 0 { return Err(sex_pdx::fs::IO); }
                    self.hash.as_mut().unwrap().update(&self.buf[..n]);
                    *offset += n as u64;
                    self.done += n as u64;
                }
                if *offset >= *size { self.steps.pop_front(); }
            }
        }
        if self.steps.is_empty() { let _ = files::sync(window); return Ok(false); }
        Ok(true)
    }

    /// Removes the half-written file and everything this job made, newest first.
    fn undo_made(&mut self, window: &mut Window) {
        if let Some(part) = self.open_part.take() { let _ = files::remove(window, &at(&part)); }
        while let Some(p) = self.made.pop() { let _ = files::remove(window, &at(&p)); }
        let _ = files::sync(window);
    }

    /// Stops the job. Returns what was kept, as shown to the person.
    pub fn cancel(&mut self, window: &mut Window) -> String {
        let label: String = shown(&self.what).into();
        let s = match self.kind {
            Kind::Copy => { self.undo_made(window); format!("Stopped copying {}: nothing half-copied was left.", label) }
            Kind::Move if !self.committed => { self.undo_made(window); format!("Stopped moving {}: it is still where it was.", label) }
            Kind::Move => return format!("Moving {}: almost done, finishing.", label),
            Kind::Delete => format!("Stopped deleting: {} items were already gone.", self.removed),
            Kind::Sum => format!("Stopped checking {}.", label),
        };
        self.steps.clear();
        let _ = files::sync(window);
        s
    }
    /// True when Esc can no longer stop it.
    pub fn finishing(&self) -> bool { self.kind == Kind::Move && self.committed }

    pub fn sha256(&self) -> Option<String> {
        let h = self.hash.clone()?.finalize();
        let mut s = String::with_capacity(64);
        for b in h.iter() { s.push_str(&format!("{:02x}", b)); }
        Some(s)
    }
    fn elapsed_ms(&self) -> u64 { ticks().saturating_sub(self.started_at).max(1) }
    fn rate(&self) -> String {
        let per_s = self.done * 1000 / self.elapsed_ms();
        format!("{}/s", human(per_s))
    }

    fn summary(&self) -> String {
        match self.kind {
            Kind::Copy => format!("Copied {} ({}).", shown(&self.what), human(self.total)),
            Kind::Move => format!("Moved {} ({}).", shown(&self.what), human(self.total)),
            Kind::Delete => format!("Deleted {} for good ({} items).", shown(&self.what), self.removed),
            Kind::Sum => format!("{}: {}  SHA-256 {}", shown(&self.what), human(self.total), self.sha256().unwrap_or_default()),
        }
    }

    /// The progress line, with a bar.
    pub fn progress(&self) -> String {
        let pct = if self.total == 0 { 100 } else { (self.done.min(self.total) * 100 / self.total) as usize };
        let filled = pct / 5;
        let mut bar = String::from("[");
        for i in 0..20 { bar.push(if i < filled { '#' } else { '.' }); }
        bar.push(']');
        let verb = match self.kind { Kind::Copy => "Copying", Kind::Move => "Moving", Kind::Delete => "Deleting", Kind::Sum => "Checking" };
        let amount = match self.kind {
            Kind::Delete => format!("{} of {} items", self.done, self.total),
            _ => format!("{} of {}, {}", human(self.done), human(self.total), self.rate()),
        };
        let stop = if self.finishing() { "finishing" } else { "Esc stops" };
        format!("{} {} {} {}%  {}  ({})", verb, shown(&self.what), bar, pct, amount, stop)
    }
    pub fn percent(&self) -> u64 { if self.total == 0 { 100 } else { self.done.min(self.total) * 100 / self.total } }
}
