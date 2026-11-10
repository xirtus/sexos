// SPDX-License-Identifier: GPL-3.0-only
//! GIF (87a and 89a): frames, transparency, interlacing, disposal and loop count.
//! Frames are found once when the file is opened and drawn one at a time onto a
//! canvas, so a long animation costs one canvas, not one picture per frame.

use alloc::vec::Vec;

use crate::{reserve_pixels, Error, Limits};

/// One frame's place in the file.
#[derive(Clone, Copy, Debug)]
struct FrameAt {
    left: usize,
    top: usize,
    width: usize,
    height: usize,
    interlaced: bool,
    /// Offset and entries of its colour table (the file's own when it has none).
    palette: (usize, usize),
    transparent: Option<u8>,
    disposal: u8,
    delay_ms: u32,
    /// Offset of the LZW minimum code size byte.
    data: usize,
}

pub(crate) struct Gif {
    pub width: usize,
    pub height: usize,
    frames: Vec<FrameAt>,
    pub loops: Option<u32>,
    next: usize,
    /// What disposal 3 puts back.
    saved: Vec<u32>,
    /// The disposal the last drawn frame asked for.
    pending: Option<FrameAt>,
}

fn u16_at(b: &[u8], at: usize) -> Result<usize, Error> {
    Ok(u16::from_le_bytes([*b.get(at).ok_or(Error::Gif)?, *b.get(at + 1).ok_or(Error::Gif)?]) as usize)
}

/// Offset just after a run of sub-blocks starting at `at`.
fn skip_blocks(b: &[u8], mut at: usize) -> Result<usize, Error> {
    loop {
        let n = *b.get(at).ok_or(Error::Gif)? as usize;
        at += 1 + n;
        if n == 0 {
            return Ok(at);
        }
    }
}

pub(crate) fn dimensions(b: &[u8]) -> Result<(usize, usize), Error> {
    if b.len() < 13 || !(b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) {
        return Err(Error::Gif);
    }
    Ok((u16_at(b, 6)?, u16_at(b, 8)?))
}

impl Gif {
    pub(crate) fn open(b: &[u8], limits: &Limits) -> Result<Self, Error> {
        let (width, height) = dimensions(b)?;
        limits.admit(width, height)?;
        let flags = b[10];
        let mut at = 13;
        let global = if flags & 0x80 != 0 {
            let n = 2usize << (flags & 7);
            let p = (at, n);
            at += 3 * n;
            Some(p)
        } else {
            None
        };
        let (mut transparent, mut disposal, mut delay_ms) = (None, 0, 0);
        let mut frames = Vec::new();
        let mut loops = None;
        loop {
            match *b.get(at).ok_or(Error::Gif)? {
                0x3B => break,
                0x21 => {
                    let label = *b.get(at + 1).ok_or(Error::Gif)?;
                    let block = at + 2;
                    if label == 0xF9 && b.get(block) == Some(&4) {
                        let f = *b.get(block + 1).ok_or(Error::Gif)?;
                        disposal = (f >> 2) & 7;
                        let cs = u16_at(b, block + 2)? as u32;
                        // As browsers do: a delay of 0 or 10 ms plays at 100 ms.
                        delay_ms = if cs <= 1 { 100 } else { cs * 10 };
                        transparent = if f & 1 != 0 { Some(*b.get(block + 4).ok_or(Error::Gif)?) } else { None };
                    } else if label == 0xFF && b.get(block) == Some(&11) && b.get(block + 1..block + 12) == Some(b"NETSCAPE2.0") {
                        let sub = block + 12;
                        if b.get(sub) == Some(&3) && b.get(sub + 1) == Some(&1) {
                            loops = Some(u16_at(b, sub + 2)? as u32);
                        }
                    }
                    at = skip_blocks(b, block)?;
                }
                0x2C => {
                    let (left, top) = (u16_at(b, at + 1)?, u16_at(b, at + 3)?);
                    let (w, h) = (u16_at(b, at + 5)?, u16_at(b, at + 7)?);
                    let f = *b.get(at + 9).ok_or(Error::Gif)?;
                    at += 10;
                    let palette = if f & 0x80 != 0 {
                        let n = 2usize << (f & 7);
                        let p = (at, n);
                        at += 3 * n;
                        p
                    } else {
                        global.unwrap_or((0, 0))
                    };
                    let data = at;
                    at = skip_blocks(b, at + 1)?;
                    if frames.try_reserve(1).is_err() {
                        return Err(Error::OutOfMemory);
                    }
                    frames.push(FrameAt { left, top, width: w, height: h, interlaced: f & 0x40 != 0, palette, transparent, disposal, delay_ms, data });
                    (transparent, disposal, delay_ms) = (None, 0, 0);
                }
                _ => return Err(Error::Gif),
            }
        }
        if frames.is_empty() {
            return Err(Error::Gif);
        }
        if b.len() < frames.iter().map(|f| f.palette.0 + 3 * f.palette.1).max().unwrap_or(0) {
            return Err(Error::Gif);
        }
        Ok(Gif { width, height, frames, loops, next: 0, saved: Vec::new(), pending: None })
    }

    pub(crate) fn frames(&self) -> usize { self.frames.len() }

    /// Draws the next frame onto `canvas` (width * height, starting cleared) and
    /// returns how long it shows. After the last frame comes the first again.
    pub(crate) fn next(&mut self, bytes: &[u8], canvas: &mut [u32]) -> Result<u32, Error> {
        if self.next == 0 {
            canvas.fill(0);
            self.pending = None;
        }
        if let Some(prev) = self.pending.take() {
            match prev.disposal {
                2 => self.rect(prev, |row| row.fill(0), canvas),
                3 if self.saved.len() == canvas.len() => canvas.copy_from_slice(&self.saved),
                _ => {}
            }
        }
        let f = self.frames[self.next];
        if f.disposal == 3 {
            if self.saved.len() != canvas.len() {
                self.saved = reserve_pixels(canvas.len())?;
            }
            self.saved.copy_from_slice(canvas);
        }
        self.draw(bytes, f, canvas)?;
        self.pending = Some(f);
        self.next = (self.next + 1) % self.frames.len();
        Ok(f.delay_ms)
    }

    /// Runs `op` on each canvas row the frame covers, clipped to the canvas.
    fn rect(&self, f: FrameAt, op: impl Fn(&mut [u32]), canvas: &mut [u32]) {
        let x1 = (f.left + f.width).min(self.width);
        let y1 = (f.top + f.height).min(self.height);
        if f.left >= x1 {
            return;
        }
        for y in f.top..y1 {
            op(&mut canvas[y * self.width + f.left..y * self.width + x1]);
        }
    }

    fn draw(&self, b: &[u8], f: FrameAt, canvas: &mut [u32]) -> Result<(), Error> {
        let mut colours = [0u32; 256];
        for (i, c) in colours.iter_mut().enumerate().take(f.palette.1) {
            let p = f.palette.0 + 3 * i;
            *c = 0xFF00_0000 | (b[p] as u32) << 16 | (b[p + 1] as u32) << 8 | b[p + 2] as u32;
        }
        if let Some(t) = f.transparent {
            colours[t as usize] = 0;
        }
        let total = f.width * f.height;
        // Rows in the order they arrive: 0, 8, 16.. then 4, 12.. then 2, 6.. then 1, 3..
        let mut row_order = (0usize, 0usize); // (pass, row)
        let (mut x, mut y) = (0usize, 0usize);
        let mut n = 0usize;
        let (w, cw, ch) = (self.width, self.width, self.height);
        lzw(b, f.data, total, |index| {
            if n >= total {
                return;
            }
            let (cx, cy) = (f.left + x, f.top + y);
            if cx < cw && cy < ch && (f.transparent != Some(index)) {
                canvas[cy * w + cx] = colours[index as usize];
            }
            n += 1;
            x += 1;
            if x == f.width {
                x = 0;
                if f.interlaced {
                    const START: [usize; 4] = [0, 4, 2, 1];
                    const STEP: [usize; 4] = [8, 8, 4, 2];
                    let (mut pass, mut row) = row_order;
                    row += STEP[pass];
                    while row >= f.height && pass < 3 {
                        pass += 1;
                        row = START[pass];
                    }
                    row_order = (pass, row);
                    y = row;
                } else {
                    y += 1;
                }
            }
        })
    }
}

/// Decodes the LZW image data at `at` (code size byte, then sub-blocks), calling
/// `put` for each colour index, at most `limit` of them. A file that ends early
/// leaves the rest of the frame as it was, as browsers do.
fn lzw(b: &[u8], at: usize, limit: usize, mut put: impl FnMut(u8)) -> Result<(), Error> {
    let min = *b.get(at).ok_or(Error::Gif)? as u32;
    if !(1..=11).contains(&min) {
        return Err(Error::Gif);
    }
    const MAX: usize = 4096;
    let mut prefix = [0u16; MAX];
    let mut suffix = [0u8; MAX];
    let mut first = [0u8; MAX];
    let mut stack = [0u8; MAX];
    let clear = 1u32 << min;
    let end = clear + 1;
    for i in 0..clear as usize {
        suffix[i] = i as u8;
        first[i] = i as u8;
    }
    let mut size = min + 1;
    let mut next = end + 1;
    let mut old: Option<u32> = None;
    let (mut acc, mut bits) = (0u32, 0u32);
    let mut out = 0usize;
    let mut pos = at + 1;
    let mut left = 0usize;
    loop {
        while bits < size {
            if left == 0 {
                left = match b.get(pos) { Some(&n) => n as usize, None => return Ok(()) };
                pos += 1;
                if left == 0 {
                    return Ok(());
                }
            }
            acc |= (*b.get(pos).ok_or(Error::Gif)? as u32) << bits;
            pos += 1;
            left -= 1;
            bits += 8;
        }
        let code = acc & ((1 << size) - 1);
        acc >>= size;
        bits -= size;
        if code == clear {
            size = min + 1;
            next = end + 1;
            old = None;
            continue;
        }
        if code == end {
            return Ok(());
        }
        let Some(prev) = old else {
            if code >= clear {
                return Err(Error::Gif);
            }
            put(code as u8);
            out += 1;
            old = Some(code);
            continue;
        };
        let (mut c, mut sp) = (code, 0usize);
        if code >= next {
            if code > next {
                return Err(Error::Gif);
            }
            // The KwKwK case: the new code is prev + first(prev).
            stack[sp] = first[prev as usize];
            sp += 1;
            c = prev;
        }
        while c > end {
            stack[sp] = suffix[c as usize];
            sp += 1;
            c = prefix[c as usize] as u32;
        }
        stack[sp] = c as u8;
        sp += 1;
        let head = c as u8;
        while sp > 0 {
            sp -= 1;
            put(stack[sp]);
            out += 1;
        }
        if (next as usize) < MAX {
            prefix[next as usize] = prev as u16;
            suffix[next as usize] = head;
            first[next as usize] = first[prev as usize];
            next += 1;
            if next == 1 << size && size < 12 {
                size += 1;
            }
        }
        old = Some(code);
        if out >= limit {
            return Ok(());
        }
    }
}

impl Gif {
    /// A placeholder carrying only a size, for stills that do not animate.
    pub(crate) fn empty(width: usize, height: usize) -> Self {
        Gif { width, height, frames: Vec::new(), loops: Some(0), next: 0, saved: Vec::new(), pending: None }
    }
}
