//! Byte sources, sinks and the crate's internal sequential reader.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use crate::{Error, Result};

/// An I/O failure reported by a [`ByteSource`], [`Sink`] or [`ItemReader`]
/// implementation. The message is shown to the user as is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoError(pub &'static str);

/// Random-access bytes an archive is read from. Loom backs this by its file
/// handle, so archives are never loaded whole.
pub trait ByteSource {
    /// Total length in bytes.
    fn len(&self) -> u64;
    /// Read up to `buf.len()` bytes at `offset`. Returning fewer bytes than
    /// asked is allowed; returning 0 before `len()` is treated as truncation.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<usize, IoError>;
}

impl ByteSource for [u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        if offset >= <[u8]>::len(self) as u64 {
            return Ok(0);
        }
        let rest = &self[offset as usize..];
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        Ok(n)
    }
}

impl ByteSource for Vec<u8> {
    fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        self.as_slice().read_at(offset, buf)
    }
}

impl<T: ByteSource + ?Sized> ByteSource for &T {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        (**self).read_at(offset, buf)
    }
}

impl<T: ByteSource + ?Sized> ByteSource for Box<T> {
    fn len(&self) -> u64 {
        (**self).len()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        (**self).read_at(offset, buf)
    }
}

/// Where extracted or created bytes go.
pub trait Sink {
    fn write_all(&mut self, data: &[u8]) -> core::result::Result<(), IoError>;
}

impl Sink for Vec<u8> {
    fn write_all(&mut self, data: &[u8]) -> core::result::Result<(), IoError> {
        self.extend_from_slice(data);
        Ok(())
    }
}

impl<T: Sink + ?Sized> Sink for &mut T {
    fn write_all(&mut self, data: &[u8]) -> core::result::Result<(), IoError> {
        (**self).write_all(data)
    }
}

/// A sink that discards everything (useful for testing an archive).
pub struct NullSink;

impl Sink for NullSink {
    fn write_all(&mut self, _data: &[u8]) -> core::result::Result<(), IoError> {
        Ok(())
    }
}

/// Sequential bytes of one item being added to a new archive.
pub trait ItemReader {
    /// Read up to `buf.len()` bytes; 0 means end of item.
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, IoError>;
}

impl ItemReader for &[u8] {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        let n = self.len().min(buf.len());
        buf[..n].copy_from_slice(&self[..n]);
        *self = &self[n..];
        Ok(n)
    }
}

impl<T: ItemReader + ?Sized> ItemReader for &mut T {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        (**self).read(buf)
    }
}

impl<T: ItemReader + ?Sized> ItemReader for Box<T> {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        (**self).read(buf)
    }
}

/// Reads a [`ByteSource`] sequentially as an [`ItemReader`] (for example to
/// re-pack an existing file into a new archive).
pub struct SourceReader<S: ByteSource> {
    src: S,
    pos: u64,
}

impl<S: ByteSource> SourceReader<S> {
    pub fn new(src: S) -> Self {
        Self { src, pos: 0 }
    }
}

impl<S: ByteSource> ItemReader for SourceReader<S> {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        let n = self.src.read_at(self.pos, buf)?;
        self.pos += n as u64;
        Ok(n)
    }
}

/// Owned bytes as an [`ItemReader`].
pub struct VecReader {
    data: Vec<u8>,
    pos: usize,
}

impl VecReader {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data, pos: 0 }
    }
}

impl ItemReader for VecReader {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        let rest = &self.data[self.pos..];
        let n = rest.len().min(buf.len());
        buf[..n].copy_from_slice(&rest[..n]);
        self.pos += n;
        Ok(n)
    }
}

/// Crate-internal sequential input.
pub(crate) trait Input {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            let n = self.read(buf)?;
            if n == 0 {
                return Err(Error::Corrupt("unexpected end of data"));
            }
            buf = &mut buf[n..];
        }
        Ok(())
    }

    /// Fill as much of `buf` as possible; returns bytes read (short only at end).
    fn read_full(&mut self, mut buf: &mut [u8]) -> Result<usize> {
        let mut total = 0;
        while !buf.is_empty() {
            let n = self.read(buf)?;
            if n == 0 {
                break;
            }
            total += n;
            buf = &mut buf[n..];
        }
        Ok(total)
    }

    fn skip(&mut self, mut n: u64) -> Result<()> {
        let mut scratch = [0u8; 4096];
        while n > 0 {
            let want = n.min(scratch.len() as u64) as usize;
            let got = self.read(&mut scratch[..want])?;
            if got == 0 {
                return Err(Error::Corrupt("unexpected end of data"));
            }
            n -= got as u64;
        }
        Ok(())
    }
}

impl<T: Input + ?Sized> Input for &mut T {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).read(buf)
    }
    fn skip(&mut self, n: u64) -> Result<()> {
        (**self).skip(n)
    }
}

impl<T: Input + ?Sized> Input for Box<T> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        (**self).read(buf)
    }
    fn skip(&mut self, n: u64) -> Result<()> {
        (**self).skip(n)
    }
}

impl Input for &[u8] {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let n = self.len().min(buf.len());
        buf[..n].copy_from_slice(&self[..n]);
        *self = &self[n..];
        Ok(n)
    }
}

const SRC_BUF: usize = 64 * 1024;

/// Buffered sequential reader over a window of a [`ByteSource`].
pub(crate) struct SrcReader<'a, S: ByteSource + ?Sized> {
    src: &'a S,
    pos: u64,
    end: u64,
    buf: Vec<u8>,
    buf_pos: usize,
    buf_len: usize,
}

impl<'a, S: ByteSource + ?Sized> SrcReader<'a, S> {
    pub fn new(src: &'a S, start: u64, end: u64) -> Self {
        Self { src, pos: start, end, buf: vec![0; SRC_BUF], buf_pos: 0, buf_len: 0 }
    }
}

impl<S: ByteSource + ?Sized> Input for SrcReader<'_, S> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if self.buf_pos == self.buf_len {
            if self.pos >= self.end || out.is_empty() {
                return Ok(0);
            }
            let want = ((self.end - self.pos).min(SRC_BUF as u64)) as usize;
            let n = self.src.read_at(self.pos, &mut self.buf[..want]).map_err(Error::Source)?;
            if n == 0 {
                return Err(Error::Corrupt("archive is truncated"));
            }
            self.pos += n as u64;
            self.buf_pos = 0;
            self.buf_len = n;
        }
        let n = (self.buf_len - self.buf_pos).min(out.len());
        out[..n].copy_from_slice(&self.buf[self.buf_pos..self.buf_pos + n]);
        self.buf_pos += n;
        Ok(n)
    }

    fn skip(&mut self, n: u64) -> Result<()> {
        let buffered = (self.buf_len - self.buf_pos) as u64;
        if n <= buffered {
            self.buf_pos += n as usize;
            return Ok(());
        }
        let target = self.pos + (n - buffered);
        if target > self.end {
            return Err(Error::Corrupt("archive is truncated"));
        }
        self.pos = target;
        self.buf_pos = 0;
        self.buf_len = 0;
        Ok(())
    }
}

/// Read exactly `buf.len()` bytes at `offset`.
pub(crate) fn read_exact_at<S: ByteSource + ?Sized>(src: &S, mut offset: u64, mut buf: &mut [u8]) -> Result<()> {
    while !buf.is_empty() {
        let n = src.read_at(offset, buf).map_err(Error::Source)?;
        if n == 0 {
            return Err(Error::Corrupt("archive is truncated"));
        }
        offset += n as u64;
        buf = &mut buf[n..];
    }
    Ok(())
}

/// Input limited to `n` bytes of an inner input.
pub(crate) struct Take<I> {
    pub inner: I,
    pub left: u64,
}

impl<I: Input> Input for Take<I> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        if self.left == 0 {
            return Ok(0);
        }
        let want = (buf.len() as u64).min(self.left) as usize;
        let n = self.inner.read(&mut buf[..want])?;
        self.left -= n as u64;
        Ok(n)
    }
    fn skip(&mut self, n: u64) -> Result<()> {
        if n > self.left {
            return Err(Error::Corrupt("unexpected end of data"));
        }
        self.inner.skip(n)?;
        self.left -= n;
        Ok(())
    }
}

#[cfg(feature = "std")]
mod std_impls {
    extern crate std;
    use super::*;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::sync::Mutex;

    /// A [`ByteSource`] over any `std` seekable reader (for example a file).
    pub struct StdSource<R> {
        inner: Mutex<R>,
        len: u64,
    }

    impl<R: Read + Seek> StdSource<R> {
        pub fn new(mut inner: R) -> std::io::Result<Self> {
            let len = inner.seek(SeekFrom::End(0))?;
            Ok(Self { inner: Mutex::new(inner), len })
        }
    }

    impl<R: Read + Seek> ByteSource for StdSource<R> {
        fn len(&self) -> u64 {
            self.len
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
            let mut r = self.inner.lock().map_err(|_| IoError("lock poisoned"))?;
            r.seek(SeekFrom::Start(offset)).map_err(|_| IoError("seek failed"))?;
            r.read(buf).map_err(|_| IoError("read failed"))
        }
    }

    /// A [`Sink`] over any `std::io::Write`.
    pub struct StdSink<W>(pub W);

    impl<W: Write> Sink for StdSink<W> {
        fn write_all(&mut self, data: &[u8]) -> core::result::Result<(), IoError> {
            self.0.write_all(data).map_err(|_| IoError("write failed"))
        }
    }

    /// An [`ItemReader`] over any `std::io::Read`.
    pub struct StdItem<R>(pub R);

    impl<R: Read> ItemReader for StdItem<R> {
        fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
            self.0.read(buf).map_err(|_| IoError("read failed"))
        }
    }
}

#[cfg(feature = "std")]
pub use std_impls::{StdItem, StdSink, StdSource};
