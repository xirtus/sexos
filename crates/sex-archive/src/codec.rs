//! Streaming decompressors (deflate, gzip, zstd, xz, lzma, lzma2) and
//! compressors (deflate, gzip) over the crate's [`Input`] trait.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use miniz_oxide::deflate::core::CompressorOxide;
use miniz_oxide::inflate::stream::InflateState;
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

use crate::io::{Input, Sink};
use crate::{Error, Result};

const IN_BUF: usize = 32 * 1024;

/// Buffered input with peeking.
pub(crate) struct BufIn<I> {
    inner: I,
    buf: Vec<u8>,
    pos: usize,
    len: usize,
}

impl<I: Input> BufIn<I> {
    pub fn new(inner: I) -> Self {
        Self { inner, buf: vec![0; IN_BUF], pos: 0, len: 0 }
    }

    /// Buffered bytes, refilling if empty. Empty slice means end of input.
    pub fn fill(&mut self) -> Result<&[u8]> {
        if self.pos == self.len {
            self.pos = 0;
            self.len = self.inner.read(&mut self.buf)?;
        }
        Ok(&self.buf[self.pos..self.len])
    }

    pub fn consume(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.len);
    }

    fn read_u8(&mut self) -> Result<u8> {
        let mut b = [0u8; 1];
        self.read_exact(&mut b)?;
        Ok(b[0])
    }
}

impl<I: Input> Input for BufIn<I> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        let avail = self.fill()?;
        let n = avail.len().min(out.len());
        out[..n].copy_from_slice(&avail[..n]);
        self.consume(n);
        Ok(n)
    }
}

/// Raw deflate decoding step shared by zip and gzip.
struct InflateCore {
    state: Box<InflateState>,
}

impl InflateCore {
    fn new() -> Self {
        Self { state: InflateState::new_boxed(DataFormat::Raw) }
    }

    /// Decode into `out`. Returns (written, stream_ended).
    fn step<I: Input>(&mut self, src: &mut BufIn<I>, out: &mut [u8]) -> Result<(usize, bool)> {
        loop {
            let input = src.fill()?;
            let eof = input.is_empty();
            let r = miniz_oxide::inflate::stream::inflate(&mut self.state, input, out, MZFlush::None);
            src.consume(r.bytes_consumed);
            match r.status {
                Ok(MZStatus::StreamEnd) => return Ok((r.bytes_written, true)),
                Ok(_) => {}
                Err(MZError::Buf) => {}
                Err(_) => return Err(Error::Corrupt("invalid deflate data")),
            }
            if r.bytes_written > 0 {
                return Ok((r.bytes_written, false));
            }
            if r.bytes_consumed == 0 && eof {
                return Err(Error::Corrupt("deflate data is truncated"));
            }
        }
    }
}

/// Raw deflate stream (zip method 8, 7z deflate coder).
pub(crate) struct Inflate<I> {
    src: BufIn<I>,
    core: InflateCore,
    done: bool,
}

impl<I: Input> Inflate<I> {
    pub fn new(inner: I) -> Self {
        Self { src: BufIn::new(inner), core: InflateCore::new(), done: false }
    }
}

impl<I: Input> Input for Inflate<I> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if self.done || out.is_empty() {
            return Ok(0);
        }
        let (n, end) = self.core.step(&mut self.src, out)?;
        self.done = end;
        Ok(n)
    }
}

/// Fields of a gzip member header that matter to a file manager.
#[derive(Debug, Default, Clone)]
pub(crate) struct GzHeader {
    pub name: Option<String>,
    pub mtime: Option<i64>,
}

fn read_gz_header<I: Input>(src: &mut BufIn<I>) -> Result<GzHeader> {
    let mut h = [0u8; 10];
    src.read_exact(&mut h)?;
    if h[0] != 0x1f || h[1] != 0x8b {
        return Err(Error::Corrupt("not a gzip stream"));
    }
    if h[2] != 8 {
        return Err(Error::Unsupported("gzip compression method"));
    }
    let flags = h[3];
    let mtime = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
    if flags & 0x04 != 0 {
        let mut x = [0u8; 2];
        src.read_exact(&mut x)?;
        src.skip(u16::from_le_bytes(x) as u64)?;
    }
    let mut name = None;
    if flags & 0x08 != 0 {
        let mut bytes = Vec::new();
        loop {
            let b = src.read_u8()?;
            if b == 0 {
                break;
            }
            if bytes.len() < 4096 {
                bytes.push(b);
            }
        }
        // RFC 1952 says ISO 8859-1; accept UTF-8 when it is valid.
        name = Some(match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
        });
    }
    if flags & 0x10 != 0 {
        while src.read_u8()? != 0 {}
    }
    if flags & 0x02 != 0 {
        src.skip(2)?;
    }
    Ok(GzHeader { name, mtime: if mtime != 0 { Some(mtime as i64) } else { None } })
}

/// Read just the first gzip header from an input.
pub(crate) fn peek_gz_header<I: Input>(inner: I) -> Result<GzHeader> {
    read_gz_header(&mut BufIn::new(inner))
}

#[derive(PartialEq)]
enum GzPhase {
    Header,
    Body,
    Done,
}

/// Gzip stream decoder (multi-member, CRC and length checked).
pub(crate) struct GzDecoder<I> {
    src: BufIn<I>,
    core: InflateCore,
    crc: crc32fast::Hasher,
    size: u32,
    phase: GzPhase,
}

impl<I: Input> GzDecoder<I> {
    pub fn new(inner: I) -> Self {
        Self {
            src: BufIn::new(inner),
            core: InflateCore::new(),
            crc: crc32fast::Hasher::new(),
            size: 0,
            phase: GzPhase::Header,
        }
    }
}

impl<I: Input> Input for GzDecoder<I> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            match self.phase {
                GzPhase::Done => return Ok(0),
                GzPhase::Header => {
                    read_gz_header(&mut self.src)?;
                    self.core = InflateCore::new();
                    self.crc = crc32fast::Hasher::new();
                    self.size = 0;
                    self.phase = GzPhase::Body;
                }
                GzPhase::Body => {
                    let (n, end) = self.core.step(&mut self.src, out)?;
                    self.crc.update(&out[..n]);
                    self.size = self.size.wrapping_add(n as u32);
                    if end {
                        let mut t = [0u8; 8];
                        self.src.read_exact(&mut t)?;
                        let crc = u32::from_le_bytes([t[0], t[1], t[2], t[3]]);
                        let isize = u32::from_le_bytes([t[4], t[5], t[6], t[7]]);
                        let calc = core::mem::replace(&mut self.crc, crc32fast::Hasher::new()).finalize();
                        if crc != calc || isize != self.size {
                            return Err(Error::Checksum);
                        }
                        let next = self.src.fill()?;
                        self.phase = if next.len() >= 2 && next[0] == 0x1f && next[1] == 0x8b {
                            GzPhase::Header
                        } else {
                            // End of input, or trailing padding: stop here.
                            GzPhase::Done
                        };
                    }
                    if n > 0 {
                        return Ok(n);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------- zstd

struct ZIn<I> {
    inner: BufIn<I>,
    err: Option<Error>,
}

impl<I: Input> ruzstd::io::Read for ZIn<I> {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, ruzstd::io::Error> {
        match self.inner.read(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                self.err = Some(e);
                Err(ruzstd::io::Error::from(ruzstd::io::ErrorKind::Other))
            }
        }
    }
}

/// Zstandard stream decoder (concatenated and skippable frames).
pub(crate) struct ZstdDecoder<I> {
    src: ZIn<I>,
    dec: Box<ruzstd::decoding::FrameDecoder>,
    in_frame: bool,
    done: bool,
}

impl<I: Input> ZstdDecoder<I> {
    pub fn new(inner: I) -> Self {
        Self {
            src: ZIn { inner: BufIn::new(inner), err: None },
            dec: Box::new(ruzstd::decoding::FrameDecoder::new()),
            in_frame: false,
            done: false,
        }
    }

    fn map_err(&mut self, msg: &'static str) -> Error {
        self.src.err.take().unwrap_or(Error::Corrupt(msg))
    }

    /// Start the next frame. Returns false at end of input.
    fn start_frame(&mut self) -> Result<bool> {
        use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
        loop {
            if self.src.inner.fill()?.is_empty() {
                return Ok(false);
            }
            match self.dec.reset(&mut self.src) {
                Ok(()) => return Ok(true),
                Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame {
                    length, ..
                })) => {
                    self.src.inner.skip(length as u64)?;
                }
                Err(_) => return Err(self.map_err("invalid zstd frame header")),
            }
        }
    }

    /// Decompressed size of the first frame if its header records it.
    pub fn first_frame_size(&mut self) -> Result<Option<u64>> {
        if !self.in_frame && !self.done {
            if !self.start_frame()? {
                self.done = true;
                return Ok(None);
            }
            self.in_frame = true;
        }
        let s = self.dec.content_size();
        Ok(if s > 0 { Some(s) } else { None })
    }
}

impl<I: Input> Input for ZstdDecoder<I> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        use ruzstd::decoding::BlockDecodingStrategy;
        use ruzstd::io::Read as _;
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.done {
                return Ok(0);
            }
            if !self.in_frame {
                if !self.start_frame()? {
                    self.done = true;
                    return Ok(0);
                }
                self.in_frame = true;
            }
            while self.dec.can_collect() == 0 && !self.dec.is_finished() {
                if self.dec.decode_blocks(&mut self.src, BlockDecodingStrategy::UptoBlocks(1)).is_err() {
                    return Err(self.map_err("invalid zstd data"));
                }
            }
            let n = self.dec.read(out).map_err(|_| Error::Corrupt("invalid zstd data"))?;
            if n > 0 {
                return Ok(n);
            }
            if self.dec.is_finished() {
                if let (Some(a), Some(b)) = (self.dec.get_checksum_from_data(), self.dec.get_calculated_checksum()) {
                    if a != b {
                        return Err(Error::Checksum);
                    }
                }
                self.in_frame = false;
            }
        }
    }
}

// ---------------------------------------------------------------- xz / lzma

struct LIn<I> {
    inner: I,
    err: Option<Error>,
}

impl<I: Input> lzma_rust2::Read for LIn<I> {
    fn read(&mut self, buf: &mut [u8]) -> core::result::Result<usize, lzma_rust2::Error> {
        match self.inner.read(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                self.err = Some(e);
                Err(lzma_rust2::Error::Other("input error"))
            }
        }
    }
}

fn map_lzma_err<I>(src: &mut LIn<I>, e: lzma_rust2::Error) -> Error {
    if let Some(e) = src.err.take() {
        return e;
    }
    match e {
        lzma_rust2::Error::Unsupported(m) => Error::Unsupported(m),
        lzma_rust2::Error::OutOfMemory(_) => Error::TooLarge,
        lzma_rust2::Error::Eof => Error::Corrupt("compressed data is truncated"),
        _ => Error::Corrupt("invalid lzma/xz data"),
    }
}

/// Raw LZMA (7z coder 030101, zip method 14).
pub(crate) struct LzmaDecoder<I: Input> {
    r: lzma_rust2::LzmaReader<LIn<I>>,
}

impl<I: Input> LzmaDecoder<I> {
    /// `unpacked` is the exact size, or `u64::MAX` when an end marker is used.
    pub fn new(inner: I, props: u8, dict_size: u32, unpacked: u64) -> Result<Self> {
        let r = lzma_rust2::LzmaReader::new_with_props(LIn { inner, err: None }, unpacked, props, dict_size, None)
            .map_err(|_| Error::Corrupt("invalid lzma properties"))?;
        Ok(Self { r })
    }
}

impl<I: Input> Input for LzmaDecoder<I> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        use lzma_rust2::Read as _;
        self.r.read(out).map_err(|e| map_lzma_err(self.r.inner_mut(), e))
    }
}

/// Raw LZMA2 (7z coder 21).
pub(crate) struct Lzma2Decoder<I: Input> {
    r: lzma_rust2::Lzma2Reader<LIn<I>>,
}

impl<I: Input> Lzma2Decoder<I> {
    pub fn new(inner: I, dict_size: u32) -> Self {
        Self { r: lzma_rust2::Lzma2Reader::new(LIn { inner, err: None }, dict_size, None) }
    }

    pub fn into_inner(self) -> I {
        self.r.into_inner().inner
    }
}

impl<I: Input> Input for Lzma2Decoder<I> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        use lzma_rust2::Read as _;
        self.r.read(out).map_err(|e| map_lzma_err(self.r.inner_mut(), e))
    }
}

// ---------------------------------------------------------------- compression

/// Push-style raw deflate compressor writing to a sink.
pub(crate) struct DeflateEncoder {
    c: Box<CompressorOxide>,
    out: Vec<u8>,
    pub compressed: u64,
}

impl DeflateEncoder {
    pub fn new(level: u8) -> Self {
        let mut c = Box::<CompressorOxide>::default();
        c.set_format_and_level(DataFormat::Raw, level);
        Self { c, out: vec![0; 64 * 1024], compressed: 0 }
    }

    fn run(&mut self, mut data: &[u8], flush: MZFlush, sink: &mut dyn Sink) -> Result<()> {
        loop {
            let r = miniz_oxide::deflate::stream::deflate(&mut self.c, data, &mut self.out, flush);
            data = &data[r.bytes_consumed..];
            if r.bytes_written > 0 {
                sink.write_all(&self.out[..r.bytes_written]).map_err(Error::Sink)?;
                self.compressed += r.bytes_written as u64;
            }
            match r.status {
                Ok(MZStatus::StreamEnd) => return Ok(()),
                Ok(_) | Err(MZError::Buf) => {}
                Err(_) => return Err(Error::Corrupt("deflate compressor failed")),
            }
            if flush == MZFlush::Finish {
                continue;
            }
            if data.is_empty() && r.bytes_written < self.out.len() {
                return Ok(());
            }
        }
    }

    pub fn write(&mut self, data: &[u8], sink: &mut dyn Sink) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        self.run(data, MZFlush::None, sink)
    }

    pub fn finish(&mut self, sink: &mut dyn Sink) -> Result<()> {
        self.run(&[], MZFlush::Finish, sink)
    }
}

/// Sink adapter that gzips everything written to it.
pub(crate) struct GzEncoder<'a> {
    inner: &'a mut dyn Sink,
    enc: DeflateEncoder,
    crc: crc32fast::Hasher,
    size: u32,
}

impl<'a> GzEncoder<'a> {
    pub fn new(inner: &'a mut dyn Sink) -> Result<Self> {
        // No name, no mtime, OS = unknown.
        inner.write_all(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255]).map_err(Error::Sink)?;
        Ok(Self { inner, enc: DeflateEncoder::new(6), crc: crc32fast::Hasher::new(), size: 0 })
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.crc.update(data);
        self.size = self.size.wrapping_add(data.len() as u32);
        self.enc.write(data, self.inner)
    }

    pub fn finish(mut self) -> Result<()> {
        self.enc.finish(self.inner)?;
        let crc = self.crc.clone().finalize();
        let mut t = [0u8; 8];
        t[..4].copy_from_slice(&crc.to_le_bytes());
        t[4..].copy_from_slice(&self.size.to_le_bytes());
        self.inner.write_all(&t).map_err(Error::Sink)
    }
}
