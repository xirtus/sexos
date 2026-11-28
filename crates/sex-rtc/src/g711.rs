// SPDX-License-Identifier: GPL-3.0-only
//! G.711 mu-law and A-law (PCMU and PCMA, RTP payload types 0 and 8), and the 48 kHz to
//! 8 kHz steps around them. Every WebRTC endpoint takes them, so a page can send its
//! microphone even where no Opus encoder is built in.
use alloc::vec::Vec;

pub fn ulaw_encode(s: i16) -> u8 {
    const BIAS: i32 = 0x84;
    let mut v = s as i32;
    let sign = if v < 0 { v = -v; 0x80 } else { 0 };
    let v = (v.min(32635) + BIAS) as u32;
    let exp = (31 - v.leading_zeros()).saturating_sub(7).min(7);
    let mant = (v >> (exp + 3)) & 0x0f;
    !(sign | (exp << 4) as i32 | mant as i32) as u8
}

pub fn ulaw_decode(b: u8) -> i16 {
    let b = !b;
    let exp = ((b >> 4) & 7) as i32;
    let mant = (b & 0x0f) as i32;
    let v = (((mant << 3) + 0x84) << exp) - 0x84;
    (if b & 0x80 != 0 { -v } else { v }) as i16
}

pub fn alaw_encode(s: i16) -> u8 {
    let mut v = s as i32 >> 3;
    let sign = if v >= 0 { 0x80 } else { v = -v - 1; 0 };
    let v = v.min(0xfff) as u32;
    let b = if v < 32 { (v >> 1) as u8 } else {
        let exp = (31 - v.leading_zeros()) - 4;
        (exp << 4) as u8 | ((v >> exp) & 0x0f) as u8
    };
    (b | sign) ^ 0x55
}

pub fn alaw_decode(b: u8) -> i16 {
    let b = b ^ 0x55;
    let exp = ((b >> 4) & 7) as i32;
    let mant = (b & 0x0f) as i32;
    let v = if exp == 0 { (mant << 4) + 8 } else { ((mant << 4) + 0x108) << (exp - 1) };
    (if b & 0x80 != 0 { v } else { -v }) as i16
}

/// 48 kHz stereo frames into 8 kHz mono samples (each one the mean of six).
pub fn downsample(frames: &[[i16; 2]], out: &mut Vec<i16>) {
    for c in frames.chunks_exact(6) {
        let s: i32 = c.iter().map(|f| (f[0] as i32 + f[1] as i32) / 2).sum();
        out.push((s / 6) as i16);
    }
}

/// 8 kHz mono back to 48 kHz stereo, by straight lines between samples.
pub fn upsample(samples: &[i16], last: &mut i16, out: &mut Vec<[i16; 2]>) {
    for &s in samples {
        for k in 1..=6 {
            let v = (*last as i32 * (6 - k) + s as i32 * k) / 6;
            out.push([v as i16, v as i16]);
        }
        *last = s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trips_closely() {
        for s in (-32768i32..32768).step_by(97) {
            let s = s as i16;
            let u = ulaw_decode(ulaw_encode(s)) as i32;
            let a = alaw_decode(alaw_encode(s)) as i32;
            let tol = (s as i32).abs() / 16 + 16;
            assert!((u - s as i32).abs() <= tol * 2, "ulaw {s} -> {u}");
            assert!((a - s as i32).abs() <= tol * 2, "alaw {s} -> {a}");
        }
        // Known codes: silence and full scale.
        assert_eq!(ulaw_encode(0), 0xff);
        assert_eq!(alaw_encode(0), 0xd5);
    }
}
