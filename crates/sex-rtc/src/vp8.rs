// SPDX-License-Identifier: GPL-3.0-only
//! A small VP8 encoder (RFC 6386) for sending the camera when no libvpx is built in.
//!
//! Key frames code every macroblock with 16x16 DC prediction. Inter frames keep a
//! macroblock that barely changed as a skipped ZEROMV copy of the last frame, and code
//! the others as ZEROMV plus residual or as DC prediction plus residual, whichever
//! predicts better. No loop filter, one token partition, default probabilities. That is
//! conditional replenishment with a real transform: fine for a talking head, and every
//! VP8 decoder plays it. The reconstruction here is the decoder's, bit for bit.
use crate::vp8_tables::*;
use alloc::vec;
use alloc::vec::Vec;

/// The boolean entropy coder (RFC 6386 section 7), as libvpx writes it.
pub struct BoolEncoder {
    out: Vec<u8>,
    low: u32,
    range: u32,
    count: i32,
}

impl Default for BoolEncoder {
    fn default() -> Self { Self::new() }
}

impl BoolEncoder {
    pub fn new() -> Self { BoolEncoder { out: Vec::new(), low: 0, range: 255, count: -24 } }
    pub fn put(&mut self, bit: bool, prob: u8) {
        let split = 1 + (((self.range - 1) * prob as u32) >> 8);
        if bit { self.low = self.low.wrapping_add(split); self.range -= split; } else { self.range = split; }
        let mut shift = self.range.leading_zeros() as i32 - 24;
        self.range <<= shift;
        self.count += shift;
        if self.count >= 0 {
            let offset = shift - self.count;
            if (self.low << (offset - 1)) & 0x8000_0000 != 0 {
                // Carry into what is already written.
                let mut x = self.out.len();
                while x > 0 && self.out[x - 1] == 0xff { self.out[x - 1] = 0; x -= 1; }
                if x > 0 { self.out[x - 1] += 1; }
            }
            self.out.push((self.low >> (24 - offset)) as u8);
            self.low = (self.low << offset) & 0xff_ffff;
            shift = self.count;
            self.count -= 8;
        }
        self.low <<= shift;
    }
    pub fn literal(&mut self, v: u32, bits: u32) {
        for i in (0..bits).rev() { self.put((v >> i) & 1 != 0, 128); }
    }
    pub fn finish(mut self) -> Vec<u8> {
        for _ in 0..32 { self.put(false, 128); }
        self.out
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode { Intra, Inter }

struct Mb {
    mode: Mode,
    skip: bool,
    /// 0..16 Y, 16..20 U, 20..24 V, 24 Y2; quantized, raster order.
    coeffs: [[i16; 16]; 25],
}

struct Quant { y_ac: i32, y2_dc: i32, y2_ac: i32, uv_dc: i32, uv_ac: i32 }

fn quant(q: usize) -> Quant {
    let q = q.min(127);
    Quant {
        y_ac: AC_QUANT[q], y2_dc: DC_QUANT[q] * 2, y2_ac: (AC_QUANT[q] * 155 / 100).max(8),
        uv_dc: DC_QUANT[q].min(132), uv_ac: AC_QUANT[q],
    }
}

/// A picture in 4:2:0, macroblock aligned.
#[derive(Clone)]
pub struct Planes { pub w: usize, pub h: usize, pub y: Vec<u8>, pub u: Vec<u8>, pub v: Vec<u8> }

impl Planes {
    pub fn new(w: usize, h: usize) -> Planes {
        Planes { w, h, y: vec![0; w * h], u: vec![0; w * h / 4], v: vec![0; w * h / 4] }
    }
}

/// ARGB (0xAARRGGBB) pixels of `w` x `h` into BT.601 4:2:0 planes of `pw` x `ph`
/// (at least as big; the edges repeat).
pub fn argb_to_planes(px: &[u32], w: usize, h: usize, out: &mut Planes) {
    let (pw, ph) = (out.w, out.h);
    if w == 0 || h == 0 { return; }
    let at = |x: usize, y: usize| px[y.min(h - 1) * w + x.min(w - 1)];
    for y in 0..ph {
        for x in 0..pw {
            let p = at(x, y);
            let (r, g, b) = (((p >> 16) & 255) as i32, ((p >> 8) & 255) as i32, (p & 255) as i32);
            out.y[y * pw + x] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
        }
    }
    for y in 0..ph / 2 {
        for x in 0..pw / 2 {
            let (mut r, mut g, mut b) = (0, 0, 0);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = at(2 * x + dx, 2 * y + dy);
                r += ((p >> 16) & 255) as i32; g += ((p >> 8) & 255) as i32; b += (p & 255) as i32;
            }
            let (r, g, b) = (r / 4, g / 4, b / 4);
            out.u[y * pw / 2 + x] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
            out.v[y * pw / 2 + x] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(0, 255) as u8;
        }
    }
}

/// 4:2:0 planes back to ARGB, `w` x `h` cut from the top left.
pub fn planes_to_argb(p: &Planes, w: usize, h: usize, out: &mut Vec<u32>) {
    out.resize(w * h, 0);
    for y in 0..h.min(p.h) {
        for x in 0..w.min(p.w) {
            let c = p.y[y * p.w + x] as i32 - 16;
            let d = p.u[(y / 2) * (p.w / 2) + x / 2] as i32 - 128;
            let e = p.v[(y / 2) * (p.w / 2) + x / 2] as i32 - 128;
            let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u32;
            let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u32;
            let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u32;
            out[y * w + x] = 0xff00_0000 | r << 16 | g << 8 | b;
        }
    }
}

fn fdct(input: &[i32; 16], out: &mut [i32; 16]) {
    let mut t = [0i32; 16];
    for i in 0..4 {
        let ip = &input[i * 4..i * 4 + 4];
        let a1 = (ip[0] + ip[3]) * 8;
        let b1 = (ip[1] + ip[2]) * 8;
        let c1 = (ip[1] - ip[2]) * 8;
        let d1 = (ip[0] - ip[3]) * 8;
        t[i * 4] = a1 + b1;
        t[i * 4 + 2] = a1 - b1;
        t[i * 4 + 1] = (c1 * 2217 + d1 * 5352 + 14500) >> 12;
        t[i * 4 + 3] = (d1 * 2217 - c1 * 5352 + 7500) >> 12;
    }
    for i in 0..4 {
        let a1 = t[i] + t[12 + i];
        let b1 = t[4 + i] + t[8 + i];
        let c1 = t[4 + i] - t[8 + i];
        let d1 = t[i] - t[12 + i];
        out[i] = (a1 + b1 + 7) >> 4;
        out[8 + i] = (a1 - b1 + 7) >> 4;
        out[4 + i] = ((c1 * 2217 + d1 * 5352 + 12000) >> 16) + (d1 != 0) as i32;
        out[12 + i] = (d1 * 2217 - c1 * 5352 + 51000) >> 16;
    }
}

fn fwht(input: &[i32; 16], out: &mut [i32; 16]) {
    let mut t = [0i32; 16];
    for i in 0..4 {
        let ip = &input[i * 4..i * 4 + 4];
        let a1 = (ip[0] + ip[2]) * 4;
        let d1 = (ip[1] + ip[3]) * 4;
        let c1 = (ip[1] - ip[3]) * 4;
        let b1 = (ip[0] - ip[2]) * 4;
        t[i * 4] = a1 + d1 + (a1 != 0) as i32;
        t[i * 4 + 1] = b1 + c1;
        t[i * 4 + 2] = b1 - c1;
        t[i * 4 + 3] = a1 - d1;
    }
    for i in 0..4 {
        let a1 = t[i] + t[8 + i];
        let d1 = t[4 + i] + t[12 + i];
        let c1 = t[4 + i] - t[12 + i];
        let b1 = t[i] - t[8 + i];
        let mut v = [a1 + d1, b1 + c1, b1 - c1, a1 - d1];
        for x in v.iter_mut() { *x += (*x < 0) as i32; }
        out[i] = (v[0] + 3) >> 3;
        out[4 + i] = (v[1] + 3) >> 3;
        out[8 + i] = (v[2] + 3) >> 3;
        out[12 + i] = (v[3] + 3) >> 3;
    }
}

/// The decoder's inverse DCT (RFC 6386 14.3).
pub fn idct(b: &mut [i32; 16]) {
    const C1: i64 = 20091;
    const C2: i64 = 35468;
    for i in 0..4 {
        let f = |k: usize| b[k] as i64;
        let a1 = f(i) + f(8 + i);
        let b1 = f(i) - f(8 + i);
        let c1 = ((f(4 + i) * C2) >> 16) - (f(12 + i) + ((f(12 + i) * C1) >> 16));
        let d1 = (f(4 + i) + ((f(4 + i) * C1) >> 16)) + ((f(12 + i) * C2) >> 16);
        b[i] = (a1 + d1) as i32;
        b[4 + i] = (b1 + c1) as i32;
        b[12 + i] = (a1 - d1) as i32;
        b[8 + i] = (b1 - c1) as i32;
    }
    for i in 0..4 {
        let f = |k: usize| b[4 * i + k] as i64;
        let a1 = f(0) + f(2);
        let b1 = f(0) - f(2);
        let c1 = ((f(1) * C2) >> 16) - (f(3) + ((f(3) * C1) >> 16));
        let d1 = (f(1) + ((f(1) * C1) >> 16)) + ((f(3) * C2) >> 16);
        b[4 * i] = ((a1 + d1 + 4) >> 3) as i32;
        b[4 * i + 3] = ((a1 - d1 + 4) >> 3) as i32;
        b[4 * i + 1] = ((b1 + c1 + 4) >> 3) as i32;
        b[4 * i + 2] = ((b1 - c1 + 4) >> 3) as i32;
    }
}

/// The decoder's inverse Walsh-Hadamard transform (14.3).
pub fn iwht(b: &mut [i32; 16]) {
    for i in 0..4 {
        let a1 = b[i] + b[12 + i];
        let b1 = b[4 + i] + b[8 + i];
        let c1 = b[4 + i] - b[8 + i];
        let d1 = b[i] - b[12 + i];
        b[i] = a1 + b1;
        b[4 + i] = c1 + d1;
        b[8 + i] = a1 - b1;
        b[12 + i] = d1 - c1;
    }
    for r in b.chunks_exact_mut(4) {
        let a1 = r[0] + r[3];
        let b1 = r[1] + r[2];
        let c1 = r[1] - r[2];
        let d1 = r[0] - r[3];
        r[0] = (a1 + b1 + 3) >> 3;
        r[1] = (c1 + d1 + 3) >> 3;
        r[2] = (a1 - b1 + 3) >> 3;
        r[3] = (d1 - c1 + 3) >> 3;
    }
}

fn quantize(c: i32, dq: i32, dc: bool) -> i16 {
    let a = c.unsigned_abs() as i32;
    let bias = if dc { dq / 2 } else { dq / 3 };
    let q = ((a + bias) / dq).min(2048);
    (if c < 0 { -q } else { q }) as i16
}

pub struct Encoded { pub data: Vec<u8>, pub keyframe: bool }

pub struct Encoder {
    /// The picture size coded in the stream.
    pub width: usize,
    pub height: usize,
    mbw: usize,
    mbh: usize,
    src: Planes,
    reference: Planes,
    cur: Planes,
    have_reference: bool,
    /// Quantizer index 0..=127 (lower is better and bigger).
    pub q: usize,
    pub frames: u64,
    pub keyframe_interval: u64,
    since_key: u64,
    want_key: bool,
    target_bps: u32,
    fps: u32,
}

impl Encoder {
    pub fn new(width: usize, height: usize) -> Encoder {
        let (width, height) = (width.clamp(16, 16383) & !1, height.clamp(16, 16383) & !1);
        let (mbw, mbh) = (width.div_ceil(16), height.div_ceil(16));
        let p = Planes::new(mbw * 16, mbh * 16);
        Encoder {
            width, height, mbw, mbh, src: p.clone(), reference: p.clone(), cur: p, have_reference: false, q: 40, frames: 0,
            keyframe_interval: 600, since_key: 0, want_key: true, target_bps: 600_000, fps: 15,
        }
    }
    /// The next frame is a key frame (a PLI or FIR came).
    pub fn request_keyframe(&mut self) { self.want_key = true; }
    /// Rate control aims each frame at `bps / fps`.
    pub fn set_rate(&mut self, bps: u32, fps: u32) { self.target_bps = bps.max(30_000); self.fps = fps.clamp(1, 60); }
    /// What the decoder will show for the last frame (macroblock aligned).
    pub fn reconstruction(&self) -> &Planes { &self.reference }

    pub fn encode_argb(&mut self, px: &[u32], w: usize, h: usize) -> Encoded {
        let mut src = core::mem::replace(&mut self.src, Planes::new(0, 0));
        argb_to_planes(px, w, h, &mut src);
        let e = self.encode_planes(&src);
        self.src = src;
        e
    }

    /// One picture, already macroblock-aligned 4:2:0 of this encoder's padded size.
    pub fn encode_planes(&mut self, src: &Planes) -> Encoded {
        let key = self.want_key || !self.have_reference || self.since_key >= self.keyframe_interval;
        let qi = self.q;
        let qt = quant(qi);
        let mut mbs: Vec<Mb> = Vec::with_capacity(self.mbw * self.mbh);
        for my in 0..self.mbh {
            for mx in 0..self.mbw {
                mbs.push(self.code_mb(src, mx, my, key, &qt));
            }
        }
        let data = self.write(&mbs, key, qi);
        core::mem::swap(&mut self.reference, &mut self.cur);
        self.have_reference = true;
        self.frames += 1;
        if key { self.since_key = 0; self.want_key = false; } else { self.since_key += 1; }
        // Rate control: inter frames steer the quantizer toward the per-frame budget.
        let budget = (self.target_bps / self.fps) as usize / 8;
        if !key {
            if data.len() > budget + budget / 4 { self.q = (self.q + 1 + data.len() / budget.max(1)).min(127); }
            else if data.len() < budget * 2 / 3 && self.q > 4 { self.q -= 1; }
        } else if data.len() > budget * 4 { self.q = (self.q + 4).min(127); }
        Encoded { data, keyframe: key }
    }

    fn code_mb(&mut self, src: &Planes, mx: usize, my: usize, key: bool, qt: &Quant) -> Mb {
        let yw = self.mbw * 16;
        let cw = self.mbw * 8;
        // Predictions: DC from this frame's reconstruction, or the last frame unmoved.
        let dc_y: [u8; 256];
        let mut dc_c = [[0u8; 64]; 2];
        {
            let (up, left) = (my > 0, mx > 0);
            let mut sum = 0u32;
            if up { for x in 0..16 { sum += self.cur.y[(my * 16 - 1) * yw + mx * 16 + x] as u32; } }
            if left { for y in 0..16 { sum += self.cur.y[(my * 16 + y) * yw + mx * 16 - 1] as u32; } }
            let shift = 3 + up as u32 + left as u32;
            let dc = if up || left { ((sum + (1 << (shift - 1))) >> shift) as u8 } else { 128 };
            dc_y = [dc; 256];
            for (pi, plane) in [&self.cur.u, &self.cur.v].into_iter().enumerate() {
                let mut sum = 0u32;
                if up { for x in 0..8 { sum += plane[(my * 8 - 1) * cw + mx * 8 + x] as u32; } }
                if left { for y in 0..8 { sum += plane[(my * 8 + y) * cw + mx * 8 - 1] as u32; } }
                let shift = 2 + up as u32 + left as u32;
                let dc = if up || left { ((sum + (1 << (shift - 1))) >> shift) as u8 } else { 128 };
                dc_c[pi] = [dc; 64];
            }
        }
        let sad = |pred_y: &dyn Fn(usize, usize) -> u8, pred_c: &dyn Fn(usize, usize, usize) -> u8| -> u32 {
            let mut s = 0u32;
            for y in 0..16 { for x in 0..16 { s += (src.y[(my * 16 + y) * yw + mx * 16 + x] as i32 - pred_y(x, y) as i32).unsigned_abs(); } }
            for y in 0..8 {
                for x in 0..8 {
                    let i = (my * 8 + y) * cw + mx * 8 + x;
                    s += (src.u[i] as i32 - pred_c(0, x, y) as i32).unsigned_abs() + (src.v[i] as i32 - pred_c(1, x, y) as i32).unsigned_abs();
                }
            }
            s
        };
        let r = &self.reference;
        let inter_sad = if key { u32::MAX } else {
            sad(&|x, y| r.y[(my * 16 + y) * yw + mx * 16 + x], &|p, x, y| { let i = (my * 8 + y) * cw + mx * 8 + x; if p == 0 { r.u[i] } else { r.v[i] } })
        };
        let intra_sad = sad(&|x, y| dc_y[y * 16 + x], &|p, x, y| dc_c[p][y * 8 + x]);
        let mode = if inter_sad <= intra_sad { Mode::Inter } else { Mode::Intra };
        let mut pred_y = [0u8; 256];
        let mut pred_c = [[0u8; 64]; 2];
        for y in 0..16 { for x in 0..16 { pred_y[y * 16 + x] = if mode == Mode::Inter { r.y[(my * 16 + y) * yw + mx * 16 + x] } else { dc_y[y * 16 + x] }; } }
        for y in 0..8 {
            for x in 0..8 {
                let i = (my * 8 + y) * cw + mx * 8 + x;
                pred_c[0][y * 8 + x] = if mode == Mode::Inter { r.u[i] } else { dc_c[0][y * 8 + x] };
                pred_c[1][y * 8 + x] = if mode == Mode::Inter { r.v[i] } else { dc_c[1][y * 8 + x] };
            }
        }
        let mut mb = Mb { mode, skip: false, coeffs: [[0; 16]; 25] };
        // An unchanged macroblock is a copy: nothing to code.
        let still = mode == Mode::Inter && inter_sad < 384 + qt.y_ac as u32 * 2;
        if !still {
            // Luma: 16 blocks, their DCs through the second-order transform.
            let mut dcs = [0i32; 16];
            for b in 0..16 {
                let (bx, by) = (b % 4, b / 4);
                let mut d = [0i32; 16];
                for y in 0..4 { for x in 0..4 {
                    let (px, py) = (bx * 4 + x, by * 4 + y);
                    d[y * 4 + x] = src.y[(my * 16 + py) * yw + mx * 16 + px] as i32 - pred_y[py * 16 + px] as i32;
                } }
                let mut c = [0i32; 16];
                fdct(&d, &mut c);
                dcs[b] = c[0];
                for i in 1..16 { mb.coeffs[b][i] = quantize(c[i], qt.y_ac, false); }
            }
            let mut y2 = [0i32; 16];
            fwht(&dcs, &mut y2);
            for i in 0..16 { mb.coeffs[24][i] = quantize(y2[i], if i == 0 { qt.y2_dc } else { qt.y2_ac }, i == 0); }
            for p in 0..2 {
                let plane = if p == 0 { &src.u } else { &src.v };
                for b in 0..4 {
                    let (bx, by) = (b % 2, b / 2);
                    let mut d = [0i32; 16];
                    for y in 0..4 { for x in 0..4 {
                        let (px, py) = (bx * 4 + x, by * 4 + y);
                        d[y * 4 + x] = plane[(my * 8 + py) * cw + mx * 8 + px] as i32 - pred_c[p][py * 8 + px] as i32;
                    } }
                    let mut c = [0i32; 16];
                    fdct(&d, &mut c);
                    for i in 0..16 { mb.coeffs[16 + p * 4 + b][i] = quantize(c[i], if i == 0 { qt.uv_dc } else { qt.uv_ac }, i == 0); }
                }
            }
        }
        mb.skip = mb.coeffs.iter().all(|b| b.iter().all(|&c| c == 0));
        // Reconstruct as the decoder will.
        let mut dc = [0i32; 16];
        if !mb.skip {
            for i in 0..16 { dc[i] = mb.coeffs[24][i] as i32 * if i == 0 { qt.y2_dc } else { qt.y2_ac }; }
            iwht(&mut dc);
        }
        for b in 0..16 {
            let (bx, by) = (b % 4, b / 4);
            let mut c = [0i32; 16];
            if !mb.skip {
                c[0] = dc[b];
                for i in 1..16 { c[i] = mb.coeffs[b][i] as i32 * qt.y_ac; }
                idct(&mut c);
            }
            for y in 0..4 { for x in 0..4 {
                let (px, py) = (bx * 4 + x, by * 4 + y);
                self.cur.y[(my * 16 + py) * yw + mx * 16 + px] = (pred_y[py * 16 + px] as i32 + c[y * 4 + x]).clamp(0, 255) as u8;
            } }
        }
        for p in 0..2 {
            for b in 0..4 {
                let (bx, by) = (b % 2, b / 2);
                let mut c = [0i32; 16];
                if !mb.skip {
                    for i in 0..16 { c[i] = mb.coeffs[16 + p * 4 + b][i] as i32 * if i == 0 { qt.uv_dc } else { qt.uv_ac }; }
                    idct(&mut c);
                }
                let plane = if p == 0 { &mut self.cur.u } else { &mut self.cur.v };
                for y in 0..4 { for x in 0..4 {
                    let (px, py) = (bx * 4 + x, by * 4 + y);
                    plane[(my * 8 + py) * cw + mx * 8 + px] = (pred_c[p][py * 8 + px] as i32 + c[y * 4 + x]).clamp(0, 255) as u8;
                } }
            }
        }
        mb
    }

    fn write(&self, mbs: &[Mb], key: bool, qi: usize) -> Vec<u8> {
        let total = mbs.len().max(1);
        let coded = mbs.iter().filter(|m| !m.skip).count();
        let intra = mbs.iter().filter(|m| m.mode == Mode::Intra).count();
        let p = |n: usize| ((n * 256 / total).clamp(1, 255)) as u8;
        let prob_skip_false = p(coded);
        let prob_intra = p(intra);
        let mut h = BoolEncoder::new();
        if key { h.literal(0, 1); h.literal(0, 1); }
        h.literal(0, 1); // no segmentation
        h.literal(0, 1); // filter type
        h.literal(0, 6); // loop filter level 0: none
        h.literal(0, 3); // sharpness
        h.literal(0, 1); // no loop filter deltas
        h.literal(0, 2); // one token partition
        h.literal(qi as u32, 7);
        h.literal(0, 5); // no quantizer deltas
        if key {
            h.literal(1, 1); // refresh entropy probs
        } else {
            h.literal(0, 1); // refresh golden
            h.literal(0, 1); // refresh altref
            h.literal(0, 2); // copy to golden: none
            h.literal(0, 2); // copy to altref: none
            h.literal(0, 1); // sign bias golden
            h.literal(0, 1); // sign bias altref
            h.literal(1, 1); // refresh entropy probs
            h.literal(1, 1); // refresh last
        }
        for i in 0..4 { for j in 0..8 { for k in 0..3 { for l in 0..11 { h.put(false, COEFF_UPDATE_PROBS[i][j][k][l]); } } } }
        h.literal(1, 1); // macroblocks may skip
        h.literal(prob_skip_false as u32, 8);
        if !key {
            h.literal(prob_intra as u32, 8);
            h.literal(255, 8); // prob_last: always the last frame
            h.literal(128, 8); // prob_gf
            h.literal(0, 1); // keep the 16x16 mode probabilities
            h.literal(0, 1); // and the chroma ones
            for i in 0..2 { for j in 0..19 { h.put(false, MV_UPDATE_PROBS[i][j]); } }
        }
        let mut t = BoolEncoder::new();
        let mut above = vec![[false; 9]; self.mbw];
        for my in 0..self.mbh {
            let mut left = [false; 9];
            for mx in 0..self.mbw {
                let mb = &mbs[my * self.mbw + mx];
                h.put(mb.skip, prob_skip_false);
                if key {
                    h.put(true, 145); h.put(false, 156); h.put(false, 163); // DC_PRED
                    h.put(false, 142); // chroma DC_PRED
                } else {
                    let inter = |x: usize, y: usize| mbs[y * self.mbw + x].mode == Mode::Inter;
                    h.put(mb.mode == Mode::Inter, prob_intra);
                    if mb.mode == Mode::Inter {
                        h.put(false, 255); // the last frame
                        // Every motion vector is zero, so only the zero count matters.
                        let mut cnt = 0;
                        if my > 0 && inter(mx, my - 1) { cnt += 2; }
                        if mx > 0 && inter(mx - 1, my) { cnt += 2; }
                        if my > 0 && mx > 0 && inter(mx - 1, my - 1) { cnt += 1; }
                        h.put(false, MODE_CONTEXTS[cnt][0]); // ZEROMV
                    } else {
                        h.put(false, 112); // DC_PRED
                        h.put(false, 162); // chroma DC_PRED
                    }
                }
                let a = &mut above[mx];
                if mb.skip {
                    *a = [false; 9];
                    left = [false; 9];
                    continue;
                }
                // Context slots: 0..4 luma, 4..6 U, 6..8 V, 8 Y2.
                let nz = put_block(&mut t, &COEFF_PROBS[1], &mb.coeffs[24], 0, a[8] as usize + left[8] as usize);
                a[8] = nz; left[8] = nz;
                for b in 0..16 {
                    let (bx, by) = (b % 4, b / 4);
                    let nz = put_block(&mut t, &COEFF_PROBS[0], &mb.coeffs[b], 1, a[bx] as usize + left[by] as usize);
                    a[bx] = nz; left[by] = nz;
                }
                for p in 0..2 {
                    for b in 0..4 {
                        let (bx, by) = (b % 2, b / 2);
                        let (ai, li) = (4 + p * 2 + bx, 4 + p * 2 + by);
                        let nz = put_block(&mut t, &COEFF_PROBS[2], &mb.coeffs[16 + p * 4 + b], 0, a[ai] as usize + left[li] as usize);
                        a[ai] = nz; left[li] = nz;
                    }
                }
            }
        }
        let first = h.finish();
        let tokens = t.finish();
        let mut out = Vec::with_capacity(10 + first.len() + tokens.len());
        let tag = (!key) as u32 | (1 << 4) | ((first.len() as u32) << 5);
        out.extend_from_slice(&tag.to_le_bytes()[..3]);
        if key {
            out.extend_from_slice(&[0x9d, 0x01, 0x2a]);
            out.extend_from_slice(&(self.width as u16).to_le_bytes());
            out.extend_from_slice(&(self.height as u16).to_le_bytes());
        }
        out.extend_from_slice(&first);
        out.extend_from_slice(&tokens);
        out
    }
}

/// One block's tokens (RFC 6386 section 13). Returns whether it had any.
fn put_block(e: &mut BoolEncoder, probs: &[[[u8; 11]; 3]; 8], raster: &[i16; 16], first: usize, ctx: usize) -> bool {
    let mut z = [0i32; 16];
    for i in 0..16 { z[i] = raster[ZIGZAG[i]] as i32; }
    let Some(last) = (first..16).rev().find(|&i| z[i] != 0) else {
        e.put(false, probs[COEFF_BANDS[first]][ctx][0]);
        return false;
    };
    let mut ctx = ctx;
    let mut after_zero = false;
    for i in first..=last {
        let p = &probs[COEFF_BANDS[i]][ctx];
        if !after_zero { e.put(true, p[0]); }
        let v = z[i];
        let a = v.unsigned_abs();
        if a == 0 { e.put(false, p[1]); ctx = 0; after_zero = true; continue; }
        e.put(true, p[1]);
        if a == 1 {
            e.put(false, p[2]);
        } else {
            e.put(true, p[2]);
            if a <= 4 {
                e.put(false, p[3]);
                if a == 2 { e.put(false, p[4]); } else { e.put(true, p[4]); e.put(a == 4, p[5]); }
            } else {
                e.put(true, p[3]);
                let (cat, base) = if a <= 6 { (0, 5) } else if a <= 10 { (1, 7) } else if a <= 18 { (2, 11) } else if a <= 34 { (3, 19) } else if a <= 66 { (4, 35) } else { (5, 67) };
                if cat < 2 { e.put(false, p[6]); e.put(cat == 1, p[7]); }
                else { e.put(true, p[6]); if cat < 4 { e.put(false, p[8]); e.put(cat == 3, p[9]); } else { e.put(true, p[8]); e.put(cat == 5, p[10]); } }
                let probs = &PROB_DCT_CAT[cat];
                let bits = probs.iter().take_while(|&&p| p != 0).count();
                let extra = a - base;
                for (k, &pp) in probs[..bits].iter().enumerate() { e.put((extra >> (bits - 1 - k)) & 1 != 0, pp); }
            }
        }
        e.put(v < 0, 128);
        ctx = if a == 1 { 1 } else { 2 };
        after_zero = false;
    }
    if last < 15 { e.put(false, probs[COEFF_BANDS[last + 1]][ctx][0]); }
    true
}
