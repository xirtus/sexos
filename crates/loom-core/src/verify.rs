// SPDX-License-Identifier: GPL-3.0-only
//! Incremental readback verification for a staged file copy.
use sha2::{Digest, Sha256};
use crate::ReadAt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Read, Short, Length, Mismatch }

pub struct CopyVerifier {
    size: u64,
    source_len: u64,
    read_len: u64,
    source: Sha256,
    staged: Sha256,
}

impl CopyVerifier {
    pub fn new(size: u64) -> Self {
        Self { size, source_len: 0, read_len: 0, source: Sha256::new(), staged: Sha256::new() }
    }
    pub fn size(&self) -> u64 { self.size }
    pub fn record_source(&mut self, bytes: &[u8]) -> Result<(), Error> {
        self.source_len = self.source_len.checked_add(bytes.len() as u64).ok_or(Error::Length)?;
        if self.source_len > self.size { return Err(Error::Length); }
        self.source.update(bytes);
        Ok(())
    }
    /// Read one chunk from the hidden destination. `true` means more remains.
    pub fn read_next(&mut self, source: &mut impl ReadAt, path: &[u8], scratch: &mut [u8]) -> Result<bool, Error> {
        if self.read_len == self.size { return Ok(false); }
        if scratch.is_empty() { return Err(Error::Read); }
        let want = (self.size - self.read_len).min(scratch.len() as u64) as usize;
        let n = source.read_at(path, self.read_len, &mut scratch[..want]).ok_or(Error::Read)?;
        if n == 0 || n > want { return Err(Error::Short); }
        self.staged.update(&scratch[..n]);
        self.read_len += n as u64;
        Ok(self.read_len < self.size)
    }
    pub fn finish(self, staged_size: u64) -> Result<(), Error> {
        if self.source_len != self.size || self.read_len != self.size || staged_size != self.size { return Err(Error::Length); }
        if self.source.finalize() != self.staged.finalize() { return Err(Error::Mismatch); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Memory<'a> { bytes: &'a [u8], fail_at: Option<u64> }
    impl ReadAt for Memory<'_> {
        fn read_at(&mut self, _path: &[u8], offset: u64, buf: &mut [u8]) -> Option<usize> {
            if self.fail_at.is_some_and(|at| offset >= at) { return None; }
            let rest = self.bytes.get(offset as usize..)?;
            let n = rest.len().min(buf.len()); buf[..n].copy_from_slice(&rest[..n]); Some(n)
        }
    }
    fn run(bytes: &[u8], staged: &[u8], fail_at: Option<u64>) -> Result<(), Error> {
        let mut verifier = CopyVerifier::new(bytes.len() as u64);
        for chunk in bytes.chunks(3) { verifier.record_source(chunk)?; }
        let mut reader = Memory { bytes: staged, fail_at };
        let mut scratch = [0; 3];
        while verifier.read_next(&mut reader, b"hidden.part", &mut scratch)? {}
        verifier.finish(staged.len() as u64)
    }
    #[test]
    fn readback_accepts_exact_bytes_and_empty_files() {
        assert_eq!(run(b"abcdefg", b"abcdefg", None), Ok(()));
        assert_eq!(run(b"", b"", None), Ok(()));
    }
    #[test]
    fn readback_rejects_corruption_short_read_and_io_failure() {
        assert_eq!(run(b"abcdefg", b"abcxefg", None), Err(Error::Mismatch));
        assert_eq!(run(b"abcdefg", b"abc", None), Err(Error::Short));
        assert_eq!(run(b"abcdefg", b"abcdefg", Some(3)), Err(Error::Read));
        assert_eq!(run(b"abc", b"abcd", None), Err(Error::Length));
    }
}
