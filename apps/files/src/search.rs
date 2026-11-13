// SPDX-License-Identifier: GPL-3.0-only
//! Search in Files: by name and by the words inside text files, under the folder being
//! looked at, in the background between keys.
//!
//! What a file says is kept in an index in Files' own folder (`search.idx`), by path,
//! size and modification time; a later search reads only files that changed since, so
//! looking again is quick. A file whose time is unknown (some disks keep none) is read
//! again each time rather than trusted.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec;
use alloc::vec::Vec;
use sex_app::files;
use sex_app::Window;

use crate::{at, join};

const INDEX: &[u8] = b"search.idx";
/// Text files up to this size have their words indexed.
const TEXT_MAX: u64 = 1024 * 1024;
const WORDS_MAX: usize = 4096;
const RESULTS_MAX: usize = 500;

struct Known { size: u64, mtime: u64, words: Vec<Vec<u8>> }

pub struct Index { files: BTreeMap<Vec<u8>, Known>, loaded: bool, changed: bool }

impl Index {
    pub const fn new() -> Self { Index { files: BTreeMap::new(), loaded: false, changed: false } }
    /// Reads `search.idx` once: lines `F size mtime path` each followed by `W words...`.
    fn load(&mut self, window: &mut Window) {
        if self.loaded { return; }
        self.loaded = true;
        let Ok(st) = files::stat(window, INDEX) else { return };
        let mut raw = vec![0u8; st.size as usize];
        let Ok(n) = files::read(window, INDEX, &mut raw) else { return };
        raw.truncate(n);
        let mut lines = raw.split(|b| *b == b'\n');
        while let Some(head) = lines.next() {
            let mut parts = head.splitn(4, |b| *b == b'\t');
            let (Some(b"F"), Some(size), Some(mtime), Some(path)) = (parts.next(), parts.next(), parts.next(), parts.next()) else { continue };
            let (Some(size), Some(mtime)) = (num(size), num(mtime)) else { continue };
            let words = match lines.next() { Some(w) if w.first() == Some(&b'W') => w[1..].split(|b| *b == b' ').filter(|w| !w.is_empty()).map(|w| w.to_vec()).collect(), _ => Vec::new() };
            self.files.insert(path.to_vec(), Known { size, mtime, words });
        }
    }
    fn save(&mut self, window: &mut Window) {
        if !self.changed { return; }
        let mut out = Vec::new();
        for (path, k) in &self.files {
            out.extend_from_slice(alloc::format!("F\t{}\t{}\t", k.size, k.mtime).as_bytes());
            out.extend_from_slice(path);
            out.extend_from_slice(b"\nW");
            for w in &k.words { out.push(b' '); out.extend_from_slice(w); }
            out.push(b'\n');
        }
        if files::write(window, INDEX, &out).is_ok() { self.changed = false; }
    }
}
fn num(b: &[u8]) -> Option<u64> { core::str::from_utf8(b).ok()?.parse().ok() }

/// Lower-case words of letters and digits, 2 to 32 bytes, each once.
fn words(text: &[u8]) -> Vec<Vec<u8>> {
    let mut set = BTreeSet::new();
    for w in text.split(|b| !(b.is_ascii_alphanumeric() || *b >= 0x80)) {
        if (2..=32).contains(&w.len()) { set.insert(w.to_ascii_lowercase()); if set.len() >= WORDS_MAX { break; } }
    }
    set.into_iter().collect()
}
/// Text, not a program or a picture: no zero bytes near the start.
fn is_text(head: &[u8]) -> bool { !head.iter().take(4096).any(|b| *b == 0) }

pub enum Step { Running, Done }
/// One result: its path under the search's folder, whether only its words matched.
pub struct Found { pub path: Vec<u8>, pub by_words: bool, pub size: u64, pub dir: bool }

pub struct Search {
    pub query: Vec<u8>,
    terms: Vec<Vec<u8>>,
    /// Where it looks: results are paths relative to it.
    pub base: Vec<u8>,
    dirs: VecDeque<Vec<u8>>,
    pending: VecDeque<(Vec<u8>, u64)>,
    pub results: Vec<Found>,
    pub looked: u64,
    pub read: u64,
    seen: BTreeSet<Vec<u8>>,
    pub started: u64,
}

impl Search {
    pub fn new(window: &mut Window, index: &mut Index, base: &[u8], query: &[u8]) -> Self {
        index.load(window);
        let terms = query.split(|b| *b == b' ').filter(|t| !t.is_empty()).map(|t| t.to_ascii_lowercase()).collect();
        Search { query: query.to_vec(), terms, base: base.to_vec(), dirs: VecDeque::from([base.to_vec()]), pending: VecDeque::new(), results: Vec::new(),
                 looked: 0, read: 0, seen: BTreeSet::new(), started: sex_pdx::get_ticks() }
    }
    fn rel<'a>(&self, path: &'a [u8]) -> &'a [u8] {
        let r = &path[self.base.len().min(path.len())..];
        r.strip_prefix(b"/").unwrap_or(r)
    }
    fn name_hit(&self, path: &[u8]) -> bool {
        let name = path.rsplit(|b| *b == b'/').next().unwrap_or(path).to_ascii_lowercase();
        self.terms.iter().all(|t| name.windows(t.len()).any(|w| w == &t[..]))
    }
    /// Every term starts some word of the file.
    fn word_hit(&self, words: &[Vec<u8>]) -> bool {
        self.terms.iter().all(|t| {
            let i = words.partition_point(|w| w < t);
            words.get(i).is_some_and(|w| w.starts_with(t))
        })
    }
    pub fn step(&mut self, window: &mut Window, index: &mut Index) -> Step {
        if self.terms.is_empty() { return Step::Done; }
        if let Some((path, size)) = self.pending.pop_front() {
            self.looked += 1;
            self.seen.insert(path.clone());
            let name_hit = self.name_hit(&path);
            let mut content_hit = false;
            if size <= TEXT_MAX {
                let mtime = files::stat(window, &at(&path)).map(|s| s.mtime).unwrap_or(0);
                let fresh = index.files.get(&path).is_some_and(|k| k.size == size && k.mtime == mtime && mtime != 0);
                if !fresh {
                    let mut buf = vec![0u8; size as usize];
                    let n = files::read(window, &at(&path), &mut buf).unwrap_or(0);
                    buf.truncate(n);
                    self.read += 1;
                    let w = if is_text(&buf) { words(&buf) } else { Vec::new() };
                    index.files.insert(path.clone(), Known { size, mtime, words: w });
                    index.changed = true;
                }
                content_hit = index.files.get(&path).is_some_and(|k| self.word_hit(&k.words));
            }
            if (name_hit || content_hit) && self.results.len() < RESULTS_MAX {
                let rel = self.rel(&path).to_vec();
                self.results.push(Found { path: rel, by_words: !name_hit && content_hit, size, dir: false });
            }
            return Step::Running;
        }
        if let Some(dir) = self.dirs.pop_front() {
            for e in files::list(window, &at(&dir)).unwrap_or_default() {
                if e.name == crate::TRASH || e.name.first() == Some(&b'.') && e.name.ends_with(b".part") { continue; }
                let path = join(&dir, &e.name);
                if e.dir {
                    if self.name_hit(&path) && self.results.len() < RESULTS_MAX { let rel = self.rel(&path).to_vec(); self.results.push(Found { path: rel, by_words: false, size: 0, dir: true }); }
                    self.dirs.push_back(path);
                } else { self.pending.push_back((path, e.size)); }
            }
            return Step::Running;
        }
        // Files under this folder that are gone leave the index too.
        let base = self.base.clone();
        let before = index.files.len();
        index.files.retain(|p, _| !(p.starts_with(&base) && (base == b"/" || p.get(base.len()) == Some(&b'/'))) || self.seen.contains(p));
        if index.files.len() != before { index.changed = true; }
        index.save(window);
        Step::Done
    }
    pub fn elapsed_ms(&self) -> u64 { sex_pdx::get_ticks().saturating_sub(self.started) }
}
