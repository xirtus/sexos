// SPDX-License-Identifier: GPL-3.0-only
//! Rate conversion for a program's sound: a stream at any rate from 8 to 192 kHz is
//! read into Bell's 48 kHz mix by linear interpolation, its phase kept between periods
//! so the joins are seamless. Pure arithmetic, tested on the host (tests/bell_resample.rs).

/// Bell's own rate.
pub const OUT_RATE: u32 = 48_000;
pub const MIN_RATE: u32 = 8_000;
pub const MAX_RATE: u32 = 192_000;

/// The rate a ring asks for: 0 (never set) and anything out of range mean 48 kHz.
pub fn rate_of(asked: u64) -> u32 {
    if asked < MIN_RATE as u64 || asked > MAX_RATE as u64 { OUT_RATE } else { asked as u32 }
}

/// Input frames per output frame, in 1/65536.
pub fn step(rate: u32) -> u64 { ((rate as u64) << 16) / OUT_RATE as u64 }

/// How many output frames `have` input frames make from phase `frac` (1/65536 of a
/// frame), at most `want`: each needs its frame and the next one to blend.
pub fn outputs(have: usize, frac: u32, step: u64, want: usize) -> usize {
    if have < 2 { return 0; }
    // Output k reads input (frac + k*step) >> 16 and the one after it.
    let last = ((have - 2) as u64) << 16;
    if (frac as u64) > last { return 0; }
    (((last - frac as u64) / step) as usize + 1).min(want)
}

/// Mixes `n` output frames into `mix` with `gain` (1/65536), reading input frame `i`
/// through `read(i)`. Returns (input frames used up, new phase).
pub fn mix_into(mix: &mut [[i32; 2]], n: usize, frac: u32, step: u64, gain: i64, read: impl Fn(usize) -> [i16; 2]) -> (usize, u32) {
    let mut pos = frac as u64;
    for out in mix.iter_mut().take(n) {
        let i = (pos >> 16) as usize;
        let t = (pos & 0xFFFF) as i64;
        let (a, b) = (read(i), read(i + 1));
        for c in 0..2 {
            let s = a[c] as i64 + ((b[c] as i64 - a[c] as i64) * t >> 16);
            out[c] += (s * gain >> 16) as i32;
        }
        pos += step;
    }
    ((pos >> 16) as usize, (pos & 0xFFFF) as u32)
}
