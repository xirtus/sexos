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
//!
//! Keys also carry a partition, the top-level page's origin ([`CodeCache::set_partition`]),
//! so one site never finds bytecode another site's visit made (no cross-site timing
//! probe). A browser that keeps the cache across runs takes new entries with
//! [`CodeCache::take_fresh`], stores them with [`encode`], and gives them back after
//! [`decode`] (which checks the engine build, key and a checksum) with
//! [`CodeCache::insert_loaded`].
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
    /// Mixed into every key: the top-level page's origin.
    partition: Vec<u8>,
    /// Written since the last [`CodeCache::take_fresh`] (only while `persist` is on).
    fresh: Vec<(CodeKey, Vec<u8>)>,
    /// Classic scripts seen once since the last [`CodeCache::take_seen`].
    fresh_seen: Vec<CodeKey>,
    /// Whether new entries are noted for saving.
    pub persist: bool,
}

/// Saved bytecode: magic, engine build, key, SHA-256 of the code, then the code.
const MAGIC: &[u8; 8] = b"SXCODE1\0";
const HEADER: usize = 8 + 32 + 32 + 32;

/// The engine build that wrote bytecode: the guest's own hash. Bytecode only goes back
/// into the exact engine that made it.
pub fn engine_id() -> [u8; 32] {
    static ID: spin_once::Once = spin_once::Once::new();
    ID.get(|| Sha256::digest(crate::QUICKJS_WASM).into())
}

mod spin_once {
    use core::sync::atomic::{AtomicU8, Ordering};
    /// A value computed once; a racing second caller computes it too (same result).
    pub struct Once { state: AtomicU8, value: core::cell::UnsafeCell<[u8; 32]> }
    unsafe impl Sync for Once {}
    impl Once {
        pub const fn new() -> Self { Self { state: AtomicU8::new(0), value: core::cell::UnsafeCell::new([0; 32]) } }
        pub fn get(&self, f: impl FnOnce() -> [u8; 32]) -> [u8; 32] {
            if self.state.load(Ordering::Acquire) == 2 { return unsafe { *self.value.get() }; }
            let v = f();
            if self.state.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                unsafe { *self.value.get() = v; }
                self.state.store(2, Ordering::Release);
            }
            v
        }
    }
}

/// A folder name for an origin's saved entries: a hash, so a list of folders is not a
/// readable list of sites.
pub fn partition_name(origin: &str) -> alloc::string::String {
    Sha256::digest(origin.as_bytes())[..8].iter().map(|b| alloc::format!("{b:02x}")).collect()
}

/// A key as the file name it is saved under, and back.
pub fn key_name(key: &CodeKey) -> alloc::string::String { key.iter().map(|b| alloc::format!("{b:02x}")).collect() }
pub fn key_from_name(name: &[u8]) -> Option<CodeKey> {
    if name.len() != 64 { return None; }
    let mut key = [0u8; 32];
    for (i, pair) in name.chunks(2).enumerate() {
        key[i] = u8::from_str_radix(core::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(key)
}

/// Saved form of one entry.
pub fn encode(key: &CodeKey, code: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + code.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&engine_id());
    out.extend_from_slice(key);
    out.extend_from_slice(&Sha256::digest(code));
    out.extend_from_slice(code);
    out
}

/// The bytecode in a saved entry, if it was written by this engine build for `key` and
/// is whole (checksum); None for anything else.
pub fn decode(key: &CodeKey, saved: &[u8]) -> Option<Vec<u8>> {
    if saved.len() <= HEADER || &saved[..8] != MAGIC || saved[8..40] != engine_id() || &saved[40..72] != key { return None; }
    let code = &saved[HEADER..];
    (saved[72..104] == Sha256::digest(code)[..]).then(|| code.to_vec())
}

impl CodeCache {
    /// A cache holding at most `limit` bytes of bytecode.
    pub fn new(limit: usize) -> Self { Self { entries: Vec::new(), seen: Vec::new(), pending: None, bytes: 0, limit, hits: 0, misses: 0, partition: Vec::new(), fresh: Vec::new(), fresh_seen: Vec::new(), persist: false } }

    /// Scripts compiled from now on belong to this top-level origin.
    pub fn set_partition(&mut self, origin: &[u8]) { self.partition = origin.to_vec(); }

    fn key(&self, name: &[u8], source: &[u8], flags: &[u8]) -> CodeKey {
        let mut h = Sha256::new();
        for part in [&self.partition[..], flags, name] {
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
        let key = self.key(name, source, flags);
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
            if self.persist && self.fresh_seen.len() < 4096 { self.fresh_seen.push(key); }
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
        if self.persist { self.fresh.push((key, code.clone())); }
        self.insert_loaded(key, code);
    }

    /// Bytecode read back from storage (already checked by [`decode`]).
    pub fn insert_loaded(&mut self, key: CodeKey, code: Vec<u8>) {
        if code.is_empty() || code.len() > self.limit / 4 || self.entries.iter().any(|(k, _)| *k == key) { return; }
        self.bytes += code.len();
        self.entries.push((key, code));
        while self.bytes > self.limit {
            let (_, old) = self.entries.remove(0);
            self.bytes -= old.len();
        }
    }

    /// A classic script seen once in an earlier run: it is written on its next sighting.
    pub fn insert_seen(&mut self, key: CodeKey) {
        if self.seen.contains(&key) { return; }
        if self.seen.len() >= 4096 { self.seen.remove(0); }
        self.seen.push(key);
    }

    /// Entries written since the last call, to save.
    pub fn take_fresh(&mut self) -> Vec<(CodeKey, Vec<u8>)> { core::mem::take(&mut self.fresh) }
    /// Classic scripts first seen since the last call, to save.
    pub fn take_seen(&mut self) -> Vec<CodeKey> { core::mem::take(&mut self.fresh_seen) }
    /// Forgets everything held (a site's data was cleared).
    pub fn clear(&mut self) { self.entries.clear(); self.seen.clear(); self.fresh.clear(); self.fresh_seen.clear(); self.pending = None; self.bytes = 0; }

    /// Whether anything waits to be saved.
    pub fn has_fresh(&self) -> bool { !self.fresh.is_empty() || !self.fresh_seen.is_empty() }

    /// Bytes of bytecode held.
    pub fn bytes(&self) -> usize { self.bytes }
    pub fn len(&self) -> usize { self.entries.len() }
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_entries_are_checked() {
        let key = [7u8; 32];
        let code = b"bytecode".to_vec();
        let saved = encode(&key, &code);
        assert_eq!(decode(&key, &saved), Some(code.clone()));
        // Another key, a damaged byte, a cut file or another engine build: refused.
        assert_eq!(decode(&[8u8; 32], &saved), None);
        let mut bad = saved.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(decode(&key, &bad), None);
        assert_eq!(decode(&key, &saved[..saved.len() - 1]), None);
        assert_eq!(decode(&key, &saved[..HEADER]), None);
        let mut other_engine = saved.clone();
        other_engine[8] ^= 1;
        assert_eq!(decode(&key, &other_engine), None);
    }

    #[test]
    fn names_round_trip_and_partitions_differ() {
        let key: CodeKey = core::array::from_fn(|i| (i * 9) as u8);
        assert_eq!(key_from_name(key_name(&key).as_bytes()), Some(key));
        assert_eq!(key_from_name(b"seen"), None);
        assert_eq!(key_from_name(&[b'g'; 64]), None);
        let a = partition_name("https://a.test");
        assert_eq!(a.len(), 16);
        assert_ne!(a, partition_name("https://b.test"));
        let mut c = CodeCache::new(1 << 20);
        let k1 = c.key(b"n", b"src", b"m0");
        c.set_partition(b"https://a.test");
        assert_ne!(k1, c.key(b"n", b"src", b"m0"));
    }
}
