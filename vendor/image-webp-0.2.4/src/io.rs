//! SexOS: the small part of `std::io` (and `byteorder`) this decoder uses, so it
//! builds without std. Only in-memory readers exist here.
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind { UnexpectedEof, InvalidInput, InvalidData, Other }

#[derive(Debug)]
pub struct Error { kind: ErrorKind }
impl Error {
    pub fn kind(&self) -> ErrorKind { self.kind }
    pub fn from_kind(kind: ErrorKind) -> Self { Self { kind } }
}
impl From<ErrorKind> for Error { fn from(kind: ErrorKind) -> Self { Self { kind } } }
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { write!(f, "{:?}", self.kind) }
}
pub type Result<T> = core::result::Result<T, Error>;

pub trait Read {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.read(buf)? { 0 => return Err(ErrorKind::UnexpectedEof.into()), n => buf = &mut buf[n..] }
        }
        Ok(())
    }
    fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize> {
        let start = out.len();
        let mut chunk = [0u8; 4096];
        loop {
            match self.read(&mut chunk)? { 0 => return Ok(out.len() - start), n => out.extend_from_slice(&chunk[..n]) }
        }
    }
    fn take(self, limit: u64) -> Take<Self> where Self: Sized { Take { inner: self, limit } }
    fn by_ref(&mut self) -> &mut Self where Self: Sized { self }
}

pub trait BufRead: Read {
    fn fill_buf(&mut self) -> Result<&[u8]>;
    fn consume(&mut self, amt: usize);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeekFrom { Start(u64), End(i64), Current(i64) }

pub trait Seek {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64>;
    fn stream_position(&mut self) -> Result<u64> { self.seek(SeekFrom::Current(0)) }
    fn seek_relative(&mut self, offset: i64) -> Result<()> { self.seek(SeekFrom::Current(offset)).map(|_| ()) }
}

impl<R: Read + ?Sized> Read for &mut R { fn read(&mut self, buf: &mut [u8]) -> Result<usize> { (**self).read(buf) } }
impl<R: BufRead + ?Sized> BufRead for &mut R {
    fn fill_buf(&mut self) -> Result<&[u8]> { (**self).fill_buf() }
    fn consume(&mut self, amt: usize) { (**self).consume(amt) }
}
impl<S: Seek + ?Sized> Seek for &mut S { fn seek(&mut self, pos: SeekFrom) -> Result<u64> { (**self).seek(pos) } }

impl Read for &[u8] {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = buf.len().min(self.len());
        buf[..n].copy_from_slice(&self[..n]);
        *self = &self[n..];
        Ok(n)
    }
}
impl BufRead for &[u8] {
    fn fill_buf(&mut self) -> Result<&[u8]> { Ok(self) }
    fn consume(&mut self, amt: usize) { *self = &self[amt.min(self.len())..]; }
}

#[derive(Clone, Debug, Default)]
pub struct Cursor<T> { inner: T, pos: u64 }
impl<T> Cursor<T> {
    pub fn new(inner: T) -> Self { Self { inner, pos: 0 } }
    pub fn into_inner(self) -> T { self.inner }
    pub fn get_ref(&self) -> &T { &self.inner }
    pub fn position(&self) -> u64 { self.pos }
    pub fn set_position(&mut self, pos: u64) { self.pos = pos }
}
impl<T: AsRef<[u8]>> Cursor<T> {
    fn rest(&self) -> &[u8] {
        let b = self.inner.as_ref();
        &b[(self.pos.min(b.len() as u64) as usize)..]
    }
}
impl<T: AsRef<[u8]>> Read for Cursor<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = { let mut r = self.rest(); r.read(buf)? };
        self.pos += n as u64;
        Ok(n)
    }
}
impl<T: AsRef<[u8]>> BufRead for Cursor<T> {
    fn fill_buf(&mut self) -> Result<&[u8]> { Ok(self.rest()) }
    fn consume(&mut self, amt: usize) { self.pos += amt as u64; }
}
impl<T: AsRef<[u8]>> Seek for Cursor<T> {
    fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
        let (base, off) = match pos {
            SeekFrom::Start(n) => { self.pos = n; return Ok(n); }
            SeekFrom::End(n) => (self.inner.as_ref().len() as u64, n),
            SeekFrom::Current(n) => (self.pos, n),
        };
        match base.checked_add_signed(off) {
            Some(n) => { self.pos = n; Ok(n) }
            None => Err(ErrorKind::InvalidInput.into()),
        }
    }
}

#[derive(Debug)]
pub struct Take<R> { inner: R, limit: u64 }
impl<R> Take<R> {
    pub fn limit(&self) -> u64 { self.limit }
    pub fn into_inner(self) -> R { self.inner }
}
impl<R: Read> Read for Take<R> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.limit == 0 { return Ok(0); }
        let max = (buf.len() as u64).min(self.limit) as usize;
        let n = self.inner.read(&mut buf[..max])?;
        self.limit -= n as u64;
        Ok(n)
    }
}
impl<R: BufRead> BufRead for Take<R> {
    fn fill_buf(&mut self) -> Result<&[u8]> {
        if self.limit == 0 { return Ok(&[]); }
        let limit = self.limit;
        let buf = self.inner.fill_buf()?;
        let n = (buf.len() as u64).min(limit) as usize;
        Ok(&buf[..n])
    }
    fn consume(&mut self, amt: usize) {
        let amt = (amt as u64).min(self.limit) as usize;
        self.limit -= amt as u64;
        self.inner.consume(amt);
    }
}

/// byteorder's marker; only little-endian reads are used.
pub enum LittleEndian {}

pub trait ReadBytesExt: Read {
    fn read_u8(&mut self) -> Result<u8> { let mut b = [0; 1]; self.read_exact(&mut b)?; Ok(b[0]) }
    fn read_u16<B>(&mut self) -> Result<u16> { let mut b = [0; 2]; self.read_exact(&mut b)?; Ok(u16::from_le_bytes(b)) }
    fn read_u24<B>(&mut self) -> Result<u32> { let mut b = [0; 4]; self.read_exact(&mut b[..3])?; Ok(u32::from_le_bytes(b)) }
    fn read_u32<B>(&mut self) -> Result<u32> { let mut b = [0; 4]; self.read_exact(&mut b)?; Ok(u32::from_le_bytes(b)) }
}
impl<R: Read + ?Sized> ReadBytesExt for R {}
