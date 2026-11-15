// SPDX-License-Identifier: GPL-3.0-only
//! Compiled scripts kept between pages and between the passes over one module graph:
//! the bytecode QuickJS wrote for a large classic script or module, found again by a
//! hash of its exact source, its address, its kind and its provenance (muted or not).
//! A browser keeps one cache and gives it to every page ([`crate::Document::code_cache`]),
//! so a site's libraries are parsed once per session rather than on every page.
//!
//! The guest asks with [`crate::op::CODE_GET`] before compiling and answers a miss
//! with [`crate::op::CODE_PUT`] once compiled. Bytecode is only ever read back by the
//! engine build that wrote it (one binary) for byte-identical source, so it is never
//! trusted more than the source itself. Page script cannot reach either op.
//!
//! Classic scripts are written on their second sighting, as most are seen only once and
//! writing costs about a third of compiling. Modules are written at once: finding a
//! module graph compiles each module again before it runs.
use alloc::vec::Vec;
use sha2::{Digest, Sha256};

pub type CodeKey = [u8; 32];

pub struct CodeCache {
    /// Least recently used first.
    entries: Vec<(CodeKey, Vec<u8>)>,
    /// Classic scripts seen once, not yet written.
    seen: Vec<CodeKey>,
    /// The key of the last miss that the guest was asked to write.
    pending: Option<CodeKey>,
    bytes: usize,
    limit: usize,
    pub hits: u64,
    pub misses: u64,
}

impl CodeCache {
    /// A cache holding at most `limit` bytes of bytecode.
    pub fn new(limit: usize) -> Self { Self { entries: Vec::new(), seen: Vec::new(), pending: None, bytes: 0, limit, hits: 0, misses: 0 } }

    fn key(name: &[u8], source: &[u8], flags: &[u8]) -> CodeKey {
        let mut h = Sha256::new();
        for part in [flags, name] {
            h.update((part.len() as u64).to_le_bytes());
            h.update(part);
        }
        h.update(source);
        h.finalize().into()
    }

    /// The guest's question: Some(bytecode) on a hit; Some(empty) on a miss the guest
    /// should write; None on a miss not worth writing. `flags` is the kind (`c` or `m`)
    /// then the provenance (`0` or `1`).
    pub fn lookup(&mut self, name: &[u8], source: &[u8], flags: &[u8]) -> Option<Vec<u8>> {
        let key = Self::key(name, source, flags);
        self.pending = None;
        if let Some(i) = self.entries.iter().position(|(k, _)| *k == key) {
            self.hits += 1;
            let e = self.entries.remove(i);
            let code = e.1.clone();
            self.entries.push(e);
            return Some(code);
        }
        self.misses += 1;
        let module = flags.first() == Some(&b'm');
        if !module && !self.seen.contains(&key) {
            if self.seen.len() >= 4096 { self.seen.remove(0); }
            self.seen.push(key);
            return None;
        }
        self.seen.retain(|k| *k != key);
        self.pending = Some(key);
        Some(Vec::new())
    }

    /// The guest's bytecode for the last miss it was asked to write.
    pub fn store(&mut self, code: Vec<u8>) {
        let Some(key) = self.pending.take() else { return };
        if code.is_empty() || code.len() > self.limit / 4 { return; }
        self.bytes += code.len();
        self.entries.push((key, code));
        while self.bytes > self.limit {
            let (_, old) = self.entries.remove(0);
            self.bytes -= old.len();
        }
    }

    /// Bytes of bytecode held.
    pub fn bytes(&self) -> usize { self.bytes }
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}
