// SPDX-License-Identifier: GPL-3.0-only
//! Deterministic per-sample random numbers: PCG32 seeded from a hash of
//! (pixel, sample, seed), so every pixel sample is reproducible on its own.

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// PCG32 (XSH-RR variant).
#[derive(Clone, Debug)]
pub(crate) struct Rng {
    state: u64,
    inc: u64,
}

impl Rng {
    const MUL: u64 = 6_364_136_223_846_793_005;

    pub fn new(seed: u64, stream: u64) -> Rng {
        let mut r = Rng { state: 0, inc: (stream << 1) | 1 };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }

    /// The generator for sample `sample` of pixel (`x`, `y`).
    pub fn for_pixel(x: u32, y: u32, sample: u32, seed: u32) -> Rng {
        let a = splitmix64(((x as u64) << 32) | y as u64);
        let b = splitmix64(((sample as u64) << 32) | seed as u64);
        Rng::new(a ^ b.rotate_left(17), splitmix64(a.wrapping_add(b)))
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(Self::MUL).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        xorshifted.rotate_right((old >> 59) as u32)
    }

    /// Uniform in [0, 1).
    pub fn f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}
