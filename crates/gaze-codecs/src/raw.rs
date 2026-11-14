//! Camera RAW files: rather than demosaic the sensor data, take the
//! full-size (or largest) JPEG preview the camera embedded. Every common
//! RAW format carries one, and it is what the camera's own screen shows.

use crate::tiff::{Ifd, Tiff, UNDEFINED};

/// An embedded JPEG preview: a slice of the RAW file, with the size its frame header announces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Preview<'a> {
    pub jpeg: &'a [u8],
    pub width: u16,
    pub height: u16,
}

const COMPRESSION: u16 = 0x0103;
const STRIP_OFFSETS: u16 = 0x0111;
const STRIP_BYTE_COUNTS: u16 = 0x0117;
const SUB_IFDS: u16 = 0x014a;
const JPEG_OFFSET: u16 = 0x0201;
const JPEG_LENGTH: u16 = 0x0202;
/// Panasonic RW2's JpgFromRaw: the preview stored inline in IFD0.
const RW2_JPEG: u16 = 0x002e;

/// The largest embedded JPEG preview (by pixel count) of a TIFF-based RAW
/// file (DNG, CR2, NEF, ARW, ORF, RW2, PEF; any TIFF really) or a Fujifilm
/// RAF. Lossless-JPEG sensor data (DNG, CR2) is not a preview and is skipped.
pub fn extract_preview(file: &[u8]) -> Option<Preview<'_>> {
    if file.starts_with(b"FUJIFILMCCD-RAW ") {
        let be = |o: usize| file.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
        let (off, len) = (be(84)?, be(88)?);
        return preview(file.get(off..off.checked_add(len)?)?);
    }
    let t = Tiff::parse(file)?;
    let mut best: Option<Preview<'_>> = None;
    for ifd in t.chain() {
        visit(&ifd, &mut best);
        // SubIFDs hold the full-size previews of NEF and DNG; DNG may nest once more.
        for i in 0..8 {
            let Some(sub) = ifd.sub(SUB_IFDS, i) else { break };
            visit(&sub, &mut best);
            for j in 0..4 {
                let Some(subsub) = sub.sub(SUB_IFDS, j) else { break };
                visit(&subsub, &mut best);
            }
        }
    }
    best
}

/// Keeps `p` when it has more pixels (then more bytes) than `best`.
fn keep<'a>(best: &mut Option<Preview<'a>>, p: Option<Preview<'a>>) {
    if let Some(p) = p {
        let area = |q: &Preview<'_>| (q.width as u32 * q.height as u32, q.jpeg.len());
        if best.map_or(true, |b| area(&p) > area(&b)) {
            *best = Some(p);
        }
    }
}

fn visit<'a>(ifd: &Ifd<'a>, best: &mut Option<Preview<'a>>) {
    let mut consider = |p| keep(best, p);
    let data = ifd.tiff.data;
    let slice = |off: u32, len: u32| data.get(off as usize..(off as usize).checked_add(len as usize)?);
    if let (Some(off), Some(len)) = (ifd.uint(JPEG_OFFSET), ifd.uint(JPEG_LENGTH)) {
        consider(slice(off, len).and_then(preview));
    }
    if matches!(ifd.uint(COMPRESSION), Some(6) | Some(7)) {
        let one = |tag| ifd.get(tag).filter(|e| e.count == 1).and_then(|e| ifd.uint_n(&e, 0));
        if let (Some(off), Some(len)) = (one(STRIP_OFFSETS), one(STRIP_BYTE_COUNTS)) {
            consider(slice(off, len).and_then(preview));
        }
    }
    if let Some(e) = ifd.get(RW2_JPEG).filter(|e| e.kind == UNDEFINED) {
        consider(preview(e.bytes));
    }
}

/// Accepts `jpeg` as a preview when it is a baseline or progressive JPEG.
fn preview(jpeg: &[u8]) -> Option<Preview<'_>> {
    let (width, height) = jpeg_frame(jpeg)?;
    Some(Preview { jpeg, width, height })
}

/// The size from a JPEG's frame header, for the frame types sex-image
/// decodes (baseline and extended Huffman, progressive). Lossless (SOF3),
/// hierarchical and arithmetic-coded frames give `None`.
fn jpeg_frame(j: &[u8]) -> Option<(u16, u16)> {
    if !j.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut i = 2;
    loop {
        while j.get(i) == Some(&0xff) && j.get(i + 1) == Some(&0xff) {
            i += 1;
        }
        if *j.get(i)? != 0xff {
            return None;
        }
        let m = *j.get(i + 1)?;
        if (0xd0..=0xd7).contains(&m) || m == 0x01 {
            i += 2;
            continue;
        }
        if m == 0xda || m == 0xd9 {
            return None;
        }
        let len = u16::from_be_bytes([*j.get(i + 2)?, *j.get(i + 3)?]) as usize;
        match m {
            0xc0 | 0xc1 | 0xc2 => {
                let s = j.get(i + 4..i + 9)?;
                let h = u16::from_be_bytes([s[1], s[2]]);
                let w = u16::from_be_bytes([s[3], s[4]]);
                return (w > 0 && h > 0).then_some((w, h));
            }
            0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => return None,
            _ => {}
        }
        if len < 2 {
            return None;
        }
        i += 2 + len;
    }
}
