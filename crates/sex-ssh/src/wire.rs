// SPDX-License-Identifier: GPL-3.0-only
//! SSH data types (RFC 4251 section 5).
use crate::Error;
use alloc::vec::Vec;

#[derive(Default)]
pub struct Writer(Vec<u8>);
impl Writer {
    pub fn new() -> Self { Self(Vec::new()) }
    pub fn byte(&mut self, b: u8) -> &mut Self { self.0.push(b); self }
    pub fn raw(&mut self, b: &[u8]) -> &mut Self { self.0.extend_from_slice(b); self }
    pub fn u32(&mut self, v: u32) -> &mut Self { self.0.extend_from_slice(&v.to_be_bytes()); self }
    pub fn string(&mut self, s: &[u8]) -> &mut Self { self.u32(s.len() as u32); self.raw(s) }
    /// A non-negative integer from big-endian bytes.
    pub fn mpint(&mut self, b: &[u8]) -> &mut Self {
        let b = &b[b.iter().position(|x| *x != 0).unwrap_or(b.len())..];
        if b.first().is_some_and(|x| x & 0x80 != 0) { self.u32(b.len() as u32 + 1).byte(0).raw(b) } else { self.string(b) }
    }
    pub fn clone_bytes(&self) -> Vec<u8> { self.0.clone() }
    pub fn finish(&mut self) -> Vec<u8> { core::mem::take(&mut self.0) }
}

pub struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self { Self(b) }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.0.len() < n { return Err(Error::Protocol); }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    pub fn byte(&mut self) -> Result<u8, Error> { Ok(self.take(1)?[0]) }
    pub fn bool(&mut self) -> Result<bool, Error> { Ok(self.byte()? != 0) }
    pub fn u32(&mut self) -> Result<u32, Error> { Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap())) }
    pub fn string(&mut self) -> Result<&'a [u8], Error> { let n = self.u32()? as usize; self.take(n) }
    /// An mpint's magnitude, without the sign byte.
    pub fn mpint(&mut self) -> Result<&'a [u8], Error> {
        let b = self.string()?;
        if b.first().is_some_and(|x| x & 0x80 != 0) { return Err(Error::Protocol); }
        Ok(&b[b.iter().position(|x| *x != 0).unwrap_or(b.len())..])
    }
    pub fn rest(&self) -> &'a [u8] { self.0 }
}
