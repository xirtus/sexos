//! .xz container decoder over lzma-rust2's raw LZMA2 decoder.
//!
//! lzma-rust2's own xz reader pulls in sha2 0.11, which does not build for
//! SexOS's soft-float targets, so the (small) container is parsed here.
//! Supports LZMA2-only filter chains (what `xz` writes by default) and
//! verifies CRC32 and CRC64 checks; SHA-256 checks are skipped unverified.
//! BCJ/delta filter chains return `Unsupported`.

use alloc::boxed::Box;

use crate::codec::{BufIn, Lzma2Decoder};
use crate::io::Input;
use crate::{Counted, Error, Result};

const MAGIC: [u8; 6] = [0xfd, b'7', b'z', b'X', b'Z', 0];

fn crc64_table() -> [u64; 256] {
    let mut t = [0u64; 256];
    for (i, slot) in t.iter_mut().enumerate() {
        let mut c = i as u64;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xC96C_5795_D787_0F42 } else { c >> 1 };
        }
        *slot = c;
    }
    t
}

enum Check {
    None,
    Crc32(crc32fast::Hasher),
    Crc64(u64, Box<[u64; 256]>),
    Skip(usize),
}

impl Check {
    fn new(kind: u8) -> Self {
        match kind {
            0 => Check::None,
            1 => Check::Crc32(crc32fast::Hasher::new()),
            4 => Check::Crc64(!0, Box::new(crc64_table())),
            k => Check::Skip([0, 4, 4, 4, 8, 8, 8, 16, 16, 16, 32, 32, 32, 64, 64, 64][k as usize & 15]),
        }
    }
    fn update(&mut self, d: &[u8]) {
        match self {
            Check::Crc32(h) => h.update(d),
            Check::Crc64(c, t) => {
                for &b in d {
                    *c = t[((*c ^ b as u64) & 0xff) as usize] ^ (*c >> 8);
                }
            }
            _ => {}
        }
    }
    fn verify<I: Input>(&mut self, src: &mut I) -> Result<()> {
        match self {
            Check::None => Ok(()),
            Check::Crc32(h) => {
                let mut b = [0u8; 4];
                src.read_exact(&mut b)?;
                let calc = core::mem::replace(h, crc32fast::Hasher::new()).finalize();
                if u32::from_le_bytes(b) != calc {
                    return Err(Error::Checksum);
                }
                Ok(())
            }
            Check::Crc64(c, _) => {
                let mut b = [0u8; 8];
                src.read_exact(&mut b)?;
                let calc = !*c;
                *c = !0;
                if u64::from_le_bytes(b) != calc {
                    return Err(Error::Checksum);
                }
                Ok(())
            }
            Check::Skip(n) => src.skip(*n as u64),
        }
    }
}

fn varint(b: &[u8], p: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    for i in 0..9 {
        let x = *b.get(*p).ok_or(Error::Corrupt("xz block header"))?;
        *p += 1;
        v |= ((x & 0x7f) as u64) << (7 * i);
        if x & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(Error::Corrupt("xz varint"))
}

fn read_varint<I: Input>(src: &mut I) -> Result<u64> {
    let mut v = 0u64;
    for i in 0..9 {
        let mut b = [0u8; 1];
        src.read_exact(&mut b)?;
        v |= ((b[0] & 0x7f) as u64) << (7 * i);
        if b[0] & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(Error::Corrupt("xz varint"))
}

type Src<I> = Counted<BufIn<I>>;

pub(crate) struct XzDecoder<I: Input> {
    src: Option<Src<I>>,
    block: Option<Lzma2Decoder<Src<I>>>,
    block_start: u64,
    check: Check,
    started: bool,
    done: bool,
}

impl<I: Input> XzDecoder<I> {
    pub fn new(inner: I) -> Self {
        Self {
            src: Some(Counted { inner: BufIn::new(inner), pos: 0 }),
            block: None,
            block_start: 0,
            check: Check::None,
            started: false,
            done: false,
        }
    }

    fn src(&mut self) -> &mut Src<I> {
        self.src.as_mut().expect("xz source present between blocks")
    }

    /// Read a stream header; false at clean end of input.
    fn stream_header(&mut self) -> Result<bool> {
        // Skip stream padding (multiples of four zero bytes).
        loop {
            let s = self.src();
            let avail = s.inner.fill()?;
            if avail.is_empty() {
                return Ok(false);
            }
            if avail[0] != 0 {
                break;
            }
            s.skip(4)?;
        }
        let mut h = [0u8; 12];
        self.src().read_exact(&mut h)?;
        if h[..6] != MAGIC {
            return Err(Error::Corrupt("xz stream header"));
        }
        if crc32fast::hash(&h[6..8]) != u32::from_le_bytes([h[8], h[9], h[10], h[11]]) || h[6] != 0 || h[7] > 15 {
            return Err(Error::Corrupt("xz stream flags"));
        }
        self.check = Check::new(h[7]);
        Ok(true)
    }

    /// At a block boundary: start the next block, or consume index+footer.
    /// Returns false when the whole input is done.
    fn next_block(&mut self) -> Result<bool> {
        loop {
            if !self.started {
                if !self.stream_header()? {
                    if self.src().pos == 0 {
                        return Err(Error::Corrupt("empty xz input"));
                    }
                    return Ok(false);
                }
                self.started = true;
            }
            let mut first = [0u8; 1];
            self.src().read_exact(&mut first)?;
            if first[0] == 0 {
                // Index: count, records, padding, crc32; then 12-byte footer.
                let s = self.src();
                let start = s.pos - 1;
                let n = read_varint(s)?;
                for _ in 0..n.saturating_mul(2) {
                    read_varint(s)?;
                }
                let pad = (4 - (s.pos - start) % 4) % 4;
                s.skip(pad + 4)?;
                let mut foot = [0u8; 12];
                s.read_exact(&mut foot)?;
                if &foot[10..12] != b"YZ" {
                    return Err(Error::Corrupt("xz stream footer"));
                }
                self.started = false;
                continue;
            }
            let size = (first[0] as usize + 1) * 4;
            let mut hb = [0u8; 1024];
            hb[0] = first[0];
            self.src().read_exact(&mut hb[1..size])?;
            let h = &hb[..size];
            if crc32fast::hash(&h[..size - 4]) != u32::from_le_bytes([h[size - 4], h[size - 3], h[size - 2], h[size - 1]]) {
                return Err(Error::Corrupt("xz block header checksum"));
            }
            let flags = h[1];
            if flags & 0x3c != 0 {
                return Err(Error::Unsupported("xz block flags"));
            }
            let mut p = 2;
            if flags & 0x40 != 0 {
                varint(h, &mut p)?;
            }
            if flags & 0x80 != 0 {
                varint(h, &mut p)?;
            }
            let nfilters = (flags & 3) + 1;
            if nfilters != 1 {
                return Err(Error::Unsupported("xz filter chain (BCJ/delta)"));
            }
            let id = varint(h, &mut p)?;
            let psize = varint(h, &mut p)?;
            if id != 0x21 || psize != 1 {
                return Err(Error::Unsupported("xz filter"));
            }
            let dp = *h.get(p).ok_or(Error::Corrupt("xz block header"))?;
            if dp > 40 {
                return Err(Error::Corrupt("xz dictionary size"));
            }
            let dict = if dp == 40 { u32::MAX } else { (2 | (dp as u32 & 1)) << (dp / 2 + 11) };
            let src = self.src.take().expect("source present");
            self.block_start = src.pos;
            self.block = Some(Lzma2Decoder::new(src, dict));
            return Ok(true);
        }
    }

    fn finish_block(&mut self) -> Result<()> {
        let block = self.block.take().expect("block present");
        let src = block.into_inner();
        let used = src.pos - self.block_start;
        self.src = Some(src);
        let pad = (4 - used % 4) % 4;
        let mut z = [0u8; 3];
        self.src().read_exact(&mut z[..pad as usize])?;
        if z.iter().any(|&b| b != 0) {
            return Err(Error::Corrupt("xz block padding"));
        }
        let mut check = core::mem::replace(&mut self.check, Check::None);
        let r = check.verify(self.src());
        self.check = check;
        r
    }
}

impl<I: Input> Input for XzDecoder<I> {
    fn read(&mut self, out: &mut [u8]) -> Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.done {
                return Ok(0);
            }
            if let Some(b) = self.block.as_mut() {
                let n = b.read(out)?;
                if n > 0 {
                    self.check.update(&out[..n]);
                    return Ok(n);
                }
                self.finish_block()?;
                continue;
            }
            if !self.next_block()? {
                self.done = true;
            }
        }
    }
}
