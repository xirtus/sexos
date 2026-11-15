// SPDX-License-Identifier: GPL-3.0-only
//! Little-endian binary encoding used by every repository structure.

use crate::{Error, Id};
use alloc::string::String;
use alloc::vec::Vec;

pub(crate) struct Enc {
    pub buf: Vec<u8>,
}

impl Enc {
    pub fn new(magic: &[u8; 4], version: u16) -> Self {
        let mut e = Enc { buf: Vec::new() };
        e.buf.extend_from_slice(magic);
        e.u16(version);
        e
    }
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn id(&mut self, id: &Id) {
        self.buf.extend_from_slice(&id.0);
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.buf.extend_from_slice(b);
    }
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    pub fn strs(&mut self, v: &[String]) {
        self.u32(v.len() as u32);
        for s in v {
            self.str(s);
        }
    }
}

pub(crate) struct Dec<'a> {
    data: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Dec<'a> {
    /// Checks magic and version (1..=`max_version`).
    pub fn new(data: &'a [u8], magic: &[u8; 4], max_version: u16, what: &'static str) -> Result<Self, Error> {
        let mut d = Dec { data, pos: 0, what };
        if d.take(4)? != magic {
            return Err(Error::Corrupt(alloc::format!("{what}: bad magic")));
        }
        let v = d.u16()?;
        if v == 0 || v > max_version {
            return Err(Error::Unsupported(alloc::format!("{what}: format version {v}")));
        }
        Ok(d)
    }
    fn err(&self) -> Error {
        Error::Corrupt(alloc::format!("{}: truncated or malformed", self.what))
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.data.len() - self.pos < n {
            return Err(self.err());
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64, Error> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn id(&mut self) -> Result<Id, Error> {
        Ok(Id(self.take(32)?.try_into().unwrap()))
    }
    pub fn bytes(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    pub fn str(&mut self) -> Result<String, Error> {
        let b = self.bytes()?;
        core::str::from_utf8(b).map(String::from).map_err(|_| self.err())
    }
    /// An element count that must fit in the remaining bytes at `min_each` bytes apiece.
    pub fn count(&mut self, min_each: usize) -> Result<usize, Error> {
        let n = self.u32()? as usize;
        if n.saturating_mul(min_each.max(1)) > self.data.len() - self.pos {
            return Err(self.err());
        }
        Ok(n)
    }
    pub fn strs(&mut self) -> Result<Vec<String>, Error> {
        let n = self.count(4)?;
        (0..n).map(|_| self.str()).collect()
    }
    pub fn finish(&self) -> Result<(), Error> {
        if self.pos != self.data.len() {
            return Err(self.err());
        }
        Ok(())
    }
}
