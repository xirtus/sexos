//! Fixture builders: TIFF/EXIF blocks, JPEG/PNG/WebP/JPEG XL wrappers, all
//! made at test time. Pictures inside come from sex-image's test fixtures.
#![allow(dead_code)]

/// sex-image's 64x48 4:2:0 JPEG (read only; that crate owns it).
pub const SMALL_JPEG: &[u8] = include_bytes!("../../../sex-image/tests/fixtures/gradient_420.jpg");
/// sex-image's 1200x900 JPEG.
pub const BIG_JPEG: &[u8] = include_bytes!("../../../sex-image/tests/fixtures/big.jpg");
/// sex-image's lossless WebP.
pub const WEBP: &[u8] = include_bytes!("../../../sex-image/tests/fixtures/gradient_lossless.webp");

pub enum Val {
    Ascii(&'static str),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Byte(Vec<u8>),
    Undefined(Vec<u8>),
    Rational(Vec<(u32, u32)>),
    /// A pointer to a sub-IFD (LONG).
    Sub(Ifd),
    /// Several sub-IFDs (SubIFDs, tag 0x14a).
    Subs(Vec<Ifd>),
    /// A data blob: this tag gets its offset (LONG), the second tag its length.
    Blob(Vec<u8>, u16),
}

#[derive(Default)]
pub struct Ifd {
    pub entries: Vec<(u16, Val)>,
    pub next: Option<Box<Ifd>>,
}

impl Ifd {
    pub fn new() -> Ifd {
        Ifd::default()
    }
    pub fn with(mut self, tag: u16, v: Val) -> Ifd {
        self.entries.push((tag, v));
        self
    }
    pub fn then(mut self, next: Ifd) -> Ifd {
        self.next = Some(Box::new(next));
        self
    }
}

struct W {
    buf: Vec<u8>,
    be: bool,
}

impl W {
    fn u16(&self, v: u16) -> [u8; 2] {
        if self.be { v.to_be_bytes() } else { v.to_le_bytes() }
    }
    fn u32(&self, v: u32) -> [u8; 4] {
        if self.be { v.to_be_bytes() } else { v.to_le_bytes() }
    }
    fn patch32(&mut self, at: usize, v: u32) {
        let b = self.u32(v);
        self.buf[at..at + 4].copy_from_slice(&b);
    }
    fn pad(&mut self) {
        if self.buf.len() % 2 == 1 {
            self.buf.push(0);
        }
    }

    fn ifd(&mut self, ifd: &Ifd) -> u32 {
        self.pad();
        let start = self.buf.len();
        // Flatten: Blob adds a second (length) entry.
        let mut rows: Vec<(u16, u16, u32, Vec<u8>, usize)> = Vec::new(); // tag, type, count, inline/data, kind index
        for (i, (tag, v)) in ifd.entries.iter().enumerate() {
            match v {
                Val::Ascii(s) => {
                    let mut b = s.as_bytes().to_vec();
                    b.push(0);
                    rows.push((*tag, 2, b.len() as u32, b, i));
                }
                Val::Short(v) => rows.push((*tag, 3, v.len() as u32, v.iter().flat_map(|&x| self.u16(x)).collect(), i)),
                Val::Long(v) => rows.push((*tag, 4, v.len() as u32, v.iter().flat_map(|&x| self.u32(x)).collect(), i)),
                Val::Byte(v) => rows.push((*tag, 1, v.len() as u32, v.clone(), i)),
                Val::Undefined(v) => rows.push((*tag, 7, v.len() as u32, v.clone(), i)),
                Val::Rational(v) => rows.push((*tag, 5, v.len() as u32, v.iter().flat_map(|&(n, d)| [self.u32(n), self.u32(d)].concat()).collect(), i)),
                Val::Sub(_) => rows.push((*tag, 4, 1, vec![0; 4], i)),
                Val::Subs(s) => rows.push((*tag, 4, s.len() as u32, vec![0; 4 * s.len()], i)),
                Val::Blob(b, len_tag) => {
                    rows.push((*tag, 4, 1, vec![0; 4], i));
                    rows.push((*len_tag, 4, 1, self.u32(b.len() as u32).to_vec(), usize::MAX));
                }
            }
        }
        rows.sort_by_key(|r| r.0);
        let n = rows.len();
        let b = self.u16(n as u16);
        self.buf.extend_from_slice(&b);
        let table = self.buf.len();
        self.buf.resize(table + n * 12 + 4, 0);
        // Where each row's value lives (offset of 4-byte field or out-of-line data).
        let mut value_at = vec![0usize; n];
        for (k, (tag, ty, count, data, _)) in rows.iter().enumerate() {
            let e = table + k * 12;
            let (t, y, c) = (self.u16(*tag), self.u16(*ty), self.u32(*count));
            self.buf[e..e + 2].copy_from_slice(&t);
            self.buf[e + 2..e + 4].copy_from_slice(&y);
            self.buf[e + 4..e + 8].copy_from_slice(&c);
            if data.len() <= 4 {
                self.buf[e + 8..e + 8 + data.len()].copy_from_slice(data);
                value_at[k] = e + 8;
            } else {
                self.pad();
                let off = self.buf.len();
                self.buf.extend_from_slice(data);
                self.patch32(e + 8, off as u32);
                value_at[k] = off;
            }
        }
        // Pointers: sub-IFDs and blobs.
        for (k, row) in rows.iter().enumerate() {
            if row.4 == usize::MAX {
                continue;
            }
            match &ifd.entries[row.4].1 {
                Val::Sub(sub) => {
                    let off = self.ifd(sub);
                    self.patch32(value_at[k], off);
                }
                Val::Subs(subs) => {
                    for (j, sub) in subs.iter().enumerate() {
                        let off = self.ifd(sub);
                        self.patch32(value_at[k] + 4 * j, off);
                    }
                }
                Val::Blob(b, _) => {
                    self.pad();
                    let off = self.buf.len() as u32;
                    self.buf.extend_from_slice(b);
                    self.patch32(value_at[k], off);
                }
                _ => {}
            }
        }
        if let Some(next) = &ifd.next {
            let off = self.ifd(next);
            self.patch32(table + n * 12, off);
        }
        start as u32
    }
}

/// A TIFF block with `ifd0`; `magic` replaces 42 (RAW variants), `prefix`
/// bytes go right after the 8-byte header (CR2's "CR" marker).
pub fn tiff_with(ifd0: &Ifd, be: bool, magic: [u8; 2], prefix: &[u8]) -> Vec<u8> {
    let mut w = W { buf: Vec::new(), be };
    w.buf.extend_from_slice(if be { b"MM" } else { b"II" });
    w.buf.extend_from_slice(&magic);
    w.buf.extend_from_slice(&[0; 4]);
    w.buf.extend_from_slice(prefix);
    let off = w.ifd(ifd0);
    w.patch32(4, off);
    w.buf
}

pub fn tiff(ifd0: &Ifd, be: bool) -> Vec<u8> {
    tiff_with(ifd0, be, if be { [0, 42] } else { [42, 0] }, &[])
}

/// A JPEG with `tiff` as its APP1 Exif segment, right after SOI.
pub fn jpeg_with_exif(jpeg: &[u8], tiff: &[u8]) -> Vec<u8> {
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(tiff);
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// An EXIF block holding only an orientation.
pub fn orientation_tiff(o: u16) -> Vec<u8> {
    tiff(&Ifd::new().with(0x0112, Val::Short(vec![o])), false)
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let mut c = kind.to_vec();
    c.extend_from_slice(body);
    out.extend_from_slice(&c);
    out.extend_from_slice(&crc32(&c).to_be_bytes());
}

/// An 8-bit RGBA PNG of `rgba` (`w * h` pixels as [r, g, b, a]), compressed
/// with stored deflate blocks, with an optional eXIf chunk.
pub fn png(rgba: &[[u8; 4]], w: u32, h: u32, exif: Option<&[u8]>) -> Vec<u8> {
    let mut raw = Vec::new();
    for y in 0..h as usize {
        raw.push(0);
        for p in &rgba[y * w as usize..(y + 1) * w as usize] {
            raw.extend_from_slice(p);
        }
    }
    let mut z = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, c) in chunks.iter().enumerate() {
        z.push((i + 1 == chunks.len()) as u8);
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(&mut out, b"IHDR", &ihdr);
    if let Some(e) = exif {
        png_chunk(&mut out, b"eXIf", e);
    }
    png_chunk(&mut out, b"IDAT", &z);
    png_chunk(&mut out, b"IEND", &[]);
    out
}

/// A WebP with an EXIF chunk appended (RIFF size fixed up).
pub fn webp_with_exif(webp: &[u8], tiff: &[u8]) -> Vec<u8> {
    let mut out = webp.to_vec();
    out.extend_from_slice(b"EXIF");
    out.extend_from_slice(&(tiff.len() as u32).to_le_bytes());
    out.extend_from_slice(tiff);
    if tiff.len() % 2 == 1 {
        out.push(0);
    }
    let size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    out
}

pub fn isobmff_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// A JPEG XL container around `codestream`, with an optional Exif box.
pub fn jxl_container(codestream: &[u8], tiff: Option<&[u8]>) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a];
    out.extend(isobmff_box(b"ftyp", b"jxl \0\0\0\0jxl "));
    if let Some(t) = tiff {
        let mut body = 0u32.to_be_bytes().to_vec();
        body.extend_from_slice(t);
        out.extend(isobmff_box(b"Exif", &body));
    }
    out.extend(isobmff_box(b"jxlc", codestream));
    out
}

/// Splits sex-image's 0xAARRGGBB into [r, g, b, a].
pub fn rgba(p: u32) -> [u8; 4] {
    let [a, r, g, b] = p.to_be_bytes();
    [r, g, b, a]
}
