// SPDX-License-Identifier: GPL-3.0-only
//! SHA-256 over whole files and hex digests.

use alloc::vec;
use alloc::vec::Vec;
use sha2::{Digest as _, Sha256};

use crate::ReadAt;

pub type Digest = [u8; 32];
/// How much of a file is read at a time.
pub const CHUNK: usize = 64 * 1024;

pub fn sha256(bytes: &[u8]) -> Digest { Sha256::digest(bytes).into() }

/// The SHA-256 of a whole file, read in [`CHUNK`]s.
pub fn hash_file(r: &mut dyn ReadAt, path: &[u8]) -> Option<Digest> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut at = 0u64;
    loop {
        let n = r.read_at(path, at, &mut buf)?;
        if n == 0 { break; }
        h.update(&buf[..n]);
        at += n as u64;
    }
    Some(h.finalize().into())
}

pub fn to_hex(d: &[u8]) -> Vec<u8> {
    const H: &[u8; 16] = b"0123456789abcdef";
    d.iter().flat_map(|b| [H[(b >> 4) as usize], H[(b & 15) as usize]]).collect()
}

/// Parses 64 hex digits (either case).
pub fn from_hex(s: &[u8]) -> Option<Digest> {
    if s.len() != 64 { return None; }
    let nib = |c: u8| match c { b'0'..=b'9' => Some(c - b'0'), b'a'..=b'f' => Some(c - b'a' + 10), b'A'..=b'F' => Some(c - b'A' + 10), _ => None };
    let mut d = [0u8; 32];
    for i in 0..32 { d[i] = nib(s[2 * i])? << 4 | nib(s[2 * i + 1])?; }
    Some(d)
}

#[cfg(test)]
pub(crate) mod mem {
    //! An in-memory file set for tests.
    use alloc::collections::BTreeMap;
    use alloc::vec::Vec;
    #[derive(Default)]
    pub struct MemFs { pub files: BTreeMap<Vec<u8>, Vec<u8>>, pub reads: usize, pub bytes: u64 }
    impl MemFs {
        pub fn add(&mut self, p: &[u8], d: &[u8]) { self.files.insert(p.to_vec(), d.to_vec()); }
    }
    impl crate::ReadAt for MemFs {
        fn read_at(&mut self, path: &[u8], offset: u64, buf: &mut [u8]) -> Option<usize> {
            let d = self.files.get(path)?;
            let at = (offset as usize).min(d.len());
            let n = buf.len().min(d.len() - at);
            buf[..n].copy_from_slice(&d[at..at + n]);
            self.reads += 1;
            self.bytes += n as u64;
            Some(n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn digests() {
        let abc = to_hex(&sha256(b"abc"));
        assert_eq!(abc, b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(from_hex(&abc), Some(sha256(b"abc")));
        assert_eq!(from_hex(b"zz"), None);
        let mut fs = mem::MemFs::default();
        let big: Vec<u8> = (0..200_000u32).map(|i| (i * 7) as u8).collect();
        fs.add(b"big", &big);
        assert_eq!(hash_file(&mut fs, b"big"), Some(sha256(&big)));
        assert_eq!(hash_file(&mut fs, b"nope"), None);
    }
}
