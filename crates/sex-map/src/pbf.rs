// SPDX-License-Identifier: GPL-3.0-only
//! A small protocol buffers reader: enough wire format for vector tiles (varints,
//! zigzag, 32 and 64 bit fixed, length-delimited fields, packed repeated varints)
//! with every read bounds-checked.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error { Truncated, BadVarint, BadWireType(u8) }

/// One field: its number and value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field<'a> {
    Varint(u32, u64),
    Fixed64(u32, u64),
    Bytes(u32, &'a [u8]),
    Fixed32(u32, u32),
}

impl Field<'_> {
    pub fn number(&self) -> u32 { match *self { Field::Varint(n, _) | Field::Fixed64(n, _) | Field::Bytes(n, _) | Field::Fixed32(n, _) => n } }
}

pub struct Reader<'a> { buf: &'a [u8], pos: usize }

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Reader<'a> { Reader { buf, pos: 0 } }
    pub fn done(&self) -> bool { self.pos >= self.buf.len() }

    pub fn varint(&mut self) -> Result<u64, Error> {
        let mut v = 0u64;
        for i in 0..10 {
            let b = *self.buf.get(self.pos).ok_or(Error::Truncated)?;
            self.pos += 1;
            if i == 9 && b > 1 { return Err(Error::BadVarint); }
            v |= ((b & 0x7F) as u64) << (7 * i);
            if b & 0x80 == 0 { return Ok(v); }
        }
        Err(Error::BadVarint)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Truncated)?;
        let s = self.buf.get(self.pos..end).ok_or(Error::Truncated)?;
        self.pos = end;
        Ok(s)
    }

    /// The next field, or None at the end.
    pub fn next_field(&mut self) -> Result<Option<Field<'a>>, Error> {
        if self.done() { return Ok(None); }
        let key = self.varint()?;
        let number = (key >> 3) as u32;
        Ok(Some(match (key & 7) as u8 {
            0 => Field::Varint(number, self.varint()?),
            1 => Field::Fixed64(number, u64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            2 => { let n = self.varint()?; let n = usize::try_from(n).map_err(|_| Error::Truncated)?; Field::Bytes(number, self.take(n)?) }
            5 => Field::Fixed32(number, u32::from_le_bytes(self.take(4)?.try_into().unwrap())),
            w => return Err(Error::BadWireType(w)),
        }))
    }
}

pub fn zigzag(v: u64) -> i64 { ((v >> 1) as i64) ^ -((v & 1) as i64) }

/// A packed repeated varint field.
pub fn packed(bytes: &[u8]) -> Result<alloc::vec::Vec<u64>, Error> {
    let mut r = Reader::new(bytes);
    let mut out = alloc::vec::Vec::new();
    while !r.done() { out.push(r.varint()?); }
    Ok(out)
}
