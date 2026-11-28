// SPDX-License-Identifier: GPL-3.0-only
//! AnalyserNode's numbers: a fixed-point radix-2 FFT over the last `fftSize` samples
//! with a Blackman window, smoothed over time, and decibels from an integer log2.
//! Floating point is used only when the size changes (window and twiddles).
use alloc::vec;
use alloc::vec::Vec;

/// Samples go in as Q22 (1.0 = 1 << 22), which leaves room for 2.0 in an i32 even
/// after the window; every FFT stage halves, which is the spec's 1/N.
const IN_SHIFT: u32 = 7;
const Q: u32 = 22;

pub struct Analyser {
    pub size: usize,
    pub min_db: f64,
    pub max_db: f64,
    pub smoothing: f64,
    /// The last `size` mono samples (Q15), a ring.
    history: Vec<i32>,
    at: usize,
    window: Vec<i32>,
    cos: Vec<i32>,
    sin: Vec<i32>,
    /// Smoothed magnitudes (Q22), `size / 2` bins.
    mags: Vec<u32>,
    /// The frame count the magnitudes were worked out at.
    pub done_at: Option<u64>,
    re: Vec<i32>,
    im: Vec<i32>,
}

impl Analyser {
    pub fn new() -> Analyser {
        let mut a = Analyser { size: 0, min_db: -100.0, max_db: -30.0, smoothing: 0.8, history: Vec::new(), at: 0, window: Vec::new(), cos: Vec::new(), sin: Vec::new(),
            mags: Vec::new(), done_at: None, re: Vec::new(), im: Vec::new() };
        a.resize(2048);
        a
    }

    /// A power of two from 32 to 32768 (others are refused).
    pub fn resize(&mut self, n: usize) -> bool {
        if !(32..=32768).contains(&n) || !n.is_power_of_two() { return false; }
        if n == self.size { return true; }
        // Keep the newest samples.
        let mut h = vec![0i32; n];
        if self.size > 0 {
            for i in 0..n.min(self.size) { h[n - 1 - i] = self.history[(self.at + self.size - 1 - i) % self.size]; }
        }
        self.history = h;
        self.at = 0;
        self.size = n;
        let tau = 2.0 * core::f64::consts::PI / n as f64;
        self.window = (0..n).map(|i| {
            let x = i as f64 * tau;
            ((0.42 - 0.5 * libm::cos(x) + 0.08 * libm::cos(2.0 * x)) * 32768.0) as i32
        }).collect();
        self.cos = (0..n / 2).map(|i| (libm::cos(i as f64 * tau) * (1u32 << 30) as f64) as i32).collect();
        self.sin = (0..n / 2).map(|i| (libm::sin(i as f64 * tau) * (1u32 << 30) as f64) as i32).collect();
        self.mags = vec![0; n / 2];
        self.re = vec![0; n];
        self.im = vec![0; n];
        self.done_at = None;
        true
    }

    /// Takes the newest mono samples (Q15).
    pub fn push(&mut self, s: &[i32]) {
        for &v in s { self.history[self.at] = v; self.at = (self.at + 1) % self.size; }
    }

    /// Sample `i` of the last `size`, oldest first (Q15).
    fn sample(&self, i: usize) -> i32 { self.history[(self.at + i) % self.size] }

    /// Works the spectrum out again if frames came since (`now`: frames rendered).
    pub fn update(&mut self, now: u64) {
        if self.done_at == Some(now) { return; }
        self.done_at = Some(now);
        let n = self.size;
        let lim = 1i32 << (Q + 1);
        let bits = n.trailing_zeros();
        for i in 0..n {
            let x = (self.sample(i).clamp(-(lim >> IN_SHIFT), lim >> IN_SHIFT) as i64) << IN_SHIFT;
            let j = i.reverse_bits() >> (usize::BITS - bits);
            self.re[j] = ((x * self.window[i] as i64) >> 15) as i32;
            self.im[j] = 0;
        }
        let mut half = 1;
        while half < n {
            let step = n / (half * 2);
            for start in (0..n).step_by(half * 2) {
                for k in 0..half {
                    let (c, s) = (self.cos[k * step] as i64, self.sin[k * step] as i64);
                    let (a, b) = (start + k, start + k + half);
                    let (br, bi) = (self.re[b] as i64, self.im[b] as i64);
                    // e^{-i theta}: (c - i s)
                    let tr = (br * c + bi * s) >> 30;
                    let ti = (bi * c - br * s) >> 30;
                    let (ar, ai) = (self.re[a] as i64, self.im[a] as i64);
                    self.re[a] = ((ar + tr) >> 1) as i32;
                    self.im[a] = ((ai + ti) >> 1) as i32;
                    self.re[b] = ((ar - tr) >> 1) as i32;
                    self.im[b] = ((ai - ti) >> 1) as i32;
                }
            }
            half *= 2;
        }
        let tau = (self.smoothing.clamp(0.0, 1.0) * 65536.0) as u64;
        for k in 0..n / 2 {
            let (r, i) = (self.re[k] as i64, self.im[k] as i64);
            let m = isqrt((r * r + i * i) as u64);
            let prev = self.mags[k] as u64;
            self.mags[k] = ((tau * prev + (65536 - tau) * m) >> 16) as u32;
        }
    }

    pub fn bins(&self) -> usize { self.size / 2 }

    /// Decibels of bin `k`, Q16; None for silence (minus infinity).
    pub fn db_q16(&self, k: usize) -> Option<i64> {
        let m = self.mags[k];
        if m == 0 { return None; }
        // 20 log10(m / 2^22) = 6.0206 (log2 m - 22)
        Some(((log2_q16(m as u64) - ((Q as i64) << 16)) * 394_566) >> 16)
    }

    pub fn float_frequency(&self, out: &mut [f32]) {
        for (k, o) in out.iter_mut().enumerate().take(self.bins()) {
            *o = match self.db_q16(k) { Some(d) => d as f32 / 65536.0, None => f32::NEG_INFINITY };
        }
    }

    pub fn byte_frequency(&self, out: &mut [u8]) {
        let lo = (self.min_db * 65536.0) as i64;
        let span = (((self.max_db - self.min_db) * 65536.0) as i64).max(1);
        for (k, o) in out.iter_mut().enumerate().take(self.bins()) {
            *o = match self.db_q16(k) { Some(d) => ((d - lo) * 255 / span).clamp(0, 255) as u8, None => 0 };
        }
    }

    pub fn float_time(&self, out: &mut [f32]) {
        for (i, o) in out.iter_mut().enumerate().take(self.size) { *o = self.sample(i) as f32 / 32768.0; }
    }

    pub fn byte_time(&self, out: &mut [u8]) {
        for (i, o) in out.iter_mut().enumerate().take(self.size) { *o = (128 + (self.sample(i) >> 8)).clamp(0, 255) as u8; }
    }
}

impl Default for Analyser { fn default() -> Self { Self::new() } }

pub fn isqrt(v: u64) -> u64 {
    if v < 2 { return v; }
    let mut x = 1u64 << ((64 - v.leading_zeros()).div_ceil(2));
    loop {
        let y = (x + v / x) / 2;
        if y >= x { return x; }
        x = y;
    }
}

/// log2(v) in Q16, from the top bit and a short table for the rest.
pub fn log2_q16(v: u64) -> i64 {
    // log2(1 + i/64) in Q16, i = 0..=64
    const T: [u32; 65] = [0, 1466, 2909, 4331, 5732, 7112, 8473, 9814, 11136, 12440, 13727, 14996, 16248, 17484, 18704, 19909, 21098, 22272, 23433, 24579, 25711, 26830, 27936,
        29029, 30109, 31178, 32234, 33279, 34312, 35334, 36346, 37346, 38336, 39316, 40286, 41246, 42196, 43137, 44068, 44990, 45904, 46809, 47705, 48593, 49472, 50344, 51207,
        52063, 52911, 53751, 54584, 55410, 56229, 57040, 57845, 58643, 59434, 60219, 60997, 61769, 62534, 63294, 64047, 64794, 65536];
    let top = 63 - v.leading_zeros() as i64;
    // The 22 bits below the top one.
    let frac = if top >= 22 { (v >> (top - 22)) & 0x3F_FFFF } else { (v << (22 - top)) & 0x3F_FFFF };
    let i = (frac >> 16) as usize;
    let f = (frac & 0xFFFF) as i64;
    let (a, b) = (T[i] as i64, T[i + 1] as i64);
    (top << 16) + a + (((b - a) * f) >> 16)
}
