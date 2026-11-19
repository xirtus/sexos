// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! SBC, the A2DP mandatory codec (A2DP 1.3 appendix B): the encoder Cufflinks streams
//! with, written for SexOS in integer arithmetic (the SexOS target has no FPU in ring 3
//! servers' ABI: soft float), plus the frame header and CRC both sides share.
//!
//! Analysis follows the specification's reference structure: a 10 x M sample window
//! (the specification's prototype filters, `WINDOW4`/`WINDOW8`, in Q24), partial sums,
//! and an M x 2M cosine matrix (`MATRIX4`/`MATRIX8`, Q15). Subband samples are kept in
//! Q8 (eight fraction bits over 16-bit PCM). Then scale factors, joint-stereo
//! decisions, the specification's bit allocation (SNR or loudness), quantization and
//! packing with the CRC-8 (x^8 + x^4 + x^3 + x^2 + 1, initial 0x0F).
//!
//! The decoder used to check this encoder is a separate floating-point reference in
//! `tests/common/sbc_ref.rs`; Cufflinks only encodes.

use alloc::vec::Vec;

pub const SYNCWORD: u8 = 0x9C;

/// Channel modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Mono = 0, DualChannel = 1, Stereo = 2, JointStereo = 3 }

/// Bit allocation methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allocation { Loudness = 0, Snr = 1 }

/// One stream's coding parameters (all of a frame header but the CRC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Sampling frequency in Hz: 16000, 32000, 44100 or 48000.
    pub rate: u32,
    /// Blocks per frame: 4, 8, 12 or 16.
    pub blocks: u8,
    pub mode: Mode,
    pub allocation: Allocation,
    /// Subbands: 4 or 8.
    pub subbands: u8,
    pub bitpool: u8,
}

impl Config {
    /// The usual high-quality A2DP stream: 48 kHz joint stereo, 16 blocks, 8 subbands,
    /// loudness, bitpool 53 (about 345 kbit/s).
    pub const HIGH_QUALITY_48K: Config = Config { rate: 48_000, blocks: 16, mode: Mode::JointStereo, allocation: Allocation::Loudness, subbands: 8, bitpool: 53 };

    pub fn channels(&self) -> usize { if self.mode == Mode::Mono { 1 } else { 2 } }
    fn rate_index(&self) -> Option<u8> { [16_000, 32_000, 44_100, 48_000].iter().position(|&r| r == self.rate).map(|i| i as u8) }
    /// The largest bitpool this mode and subband count allow.
    pub fn max_bitpool(&self) -> u8 {
        let per = if matches!(self.mode, Mode::Mono | Mode::DualChannel) { 16 } else { 32 };
        (per * self.subbands as u32).min(250) as u8
    }
    pub fn valid(&self) -> bool {
        self.rate_index().is_some() && matches!(self.blocks, 4 | 8 | 12 | 16) && matches!(self.subbands, 4 | 8)
            && self.bitpool >= 2 && self.bitpool <= self.max_bitpool()
    }
    /// PCM frames (sample pairs) one SBC frame codes.
    pub fn samples(&self) -> usize { self.blocks as usize * self.subbands as usize }
    /// Bytes of one encoded frame.
    pub fn frame_bytes(&self) -> usize {
        let (nb, ns, nc, bp) = (self.blocks as usize, self.subbands as usize, self.channels(), self.bitpool as usize);
        let data = match self.mode {
            Mode::Mono | Mode::DualChannel => nb * nc * bp,
            Mode::Stereo => nb * bp,
            Mode::JointStereo => ns + nb * bp,
        };
        4 + (4 * ns * nc) / 8 + data.div_ceil(8)
    }
    /// Header byte 1.
    fn header(&self) -> u8 {
        (self.rate_index().unwrap_or(0) << 6) | (((self.blocks / 4).saturating_sub(1) & 3) << 4) | ((self.mode as u8) << 2)
            | ((self.allocation as u8) << 1) | (self.subbands == 8) as u8
    }
}

/// The configuration and length of the frame starting at `f`, if its header is
/// well formed and the whole frame is there (the CRC is not checked: see [`check_crc`]).
pub fn parse_header(f: &[u8]) -> Option<(Config, usize)> {
    if f.len() < 4 || f[0] != SYNCWORD { return None; }
    let h = f[1];
    let mode = [Mode::Mono, Mode::DualChannel, Mode::Stereo, Mode::JointStereo][(h >> 2 & 3) as usize];
    let allocation = if h & 2 != 0 { Allocation::Snr } else { Allocation::Loudness };
    let c = Config { rate: [16_000, 32_000, 44_100, 48_000][(h >> 6) as usize], blocks: ((h >> 4 & 3) + 1) * 4, mode, allocation,
        subbands: if h & 1 != 0 { 8 } else { 4 }, bitpool: f[2] };
    if !c.valid() { return None; }
    let n = c.frame_bytes();
    (f.len() >= n).then_some((c, n))
}

/// CRC-8 of `bits` bits of `data`, most significant bit first.
pub fn crc8(data: &[u8], bits: usize) -> u8 {
    let mut crc: u8 = 0x0F;
    for i in 0..bits.min(data.len() * 8) {
        let bit = data[i / 8] >> (7 - i % 8) & 1;
        let top = crc >> 7;
        crc <<= 1;
        if top ^ bit != 0 { crc ^= 0x1D; }
    }
    crc
}

/// The bits the CRC covers: header bytes 1 and 2, then (joint stereo) the join flags and
/// the scale factors, which start at byte 4.
fn crc_of(frame: &[u8], c: &Config) -> u8 {
    let ns = c.subbands as usize;
    let bits = if c.mode == Mode::JointStereo { ns } else { 0 } + 4 * ns * c.channels();
    let mut buf = [0u8; 2 + 1 + 64];
    buf[0] = frame[1];
    buf[1] = frame[2];
    let n = bits.div_ceil(8).min(frame.len().saturating_sub(4)).min(65);
    buf[2..2 + n].copy_from_slice(&frame[4..4 + n]);
    crc8(&buf, 16 + bits)
}

/// Whether a whole frame's CRC matches.
pub fn check_crc(frame: &[u8]) -> bool {
    match parse_header(frame) { Some((c, _)) => crc_of(frame, &c) == frame[3], None => false }
}

/// Loudness offsets (A2DP table 12.4.3), by sampling frequency index.
const OFFSET4: [[i32; 4]; 4] = [[-1, 0, 0, 0], [-2, 0, 0, 1], [-2, 0, 0, 1], [-2, 0, 0, 1]];
const OFFSET8: [[i32; 8]; 4] = [[-2, 0, 0, 0, 0, 0, 0, 1], [-3, 0, 0, 0, 0, 0, 1, 2], [-4, 0, 0, 0, 0, 0, 1, 2], [-4, 0, 0, 0, 0, 0, 1, 2]];

/// Bits for each subband of each channel (A2DP 12.6.3). `scf[ch][sb]`.
pub fn allocate(c: &Config, scf: &[[u8; 8]; 2]) -> [[u8; 8]; 2] {
    let ns = c.subbands as usize;
    let fi = c.rate_index().unwrap_or(3) as usize;
    let mut need = [[0i32; 8]; 2];
    for ch in 0..c.channels() {
        for sb in 0..ns {
            let s = scf[ch][sb] as i32;
            need[ch][sb] = match c.allocation {
                Allocation::Snr => s,
                Allocation::Loudness if s == 0 => -5,
                Allocation::Loudness => {
                    let off = if ns == 4 { OFFSET4[fi][sb] } else { OFFSET8[fi][sb] };
                    let l = s - off;
                    if l > 0 { l / 2 } else { l }
                }
            };
        }
    }
    let mut bits = [[0u8; 8]; 2];
    let bitpool = c.bitpool as i32;
    // Mono and dual channel allocate each channel alone; stereo modes share the pool.
    let groups: &[&[usize]] = if matches!(c.mode, Mode::Stereo | Mode::JointStereo) { &[&[0, 1]] } else if c.mode == Mode::DualChannel { &[&[0], &[1]] } else { &[&[0]] };
    for chans in groups {
        let max_need = chans.iter().flat_map(|&ch| need[ch][..ns].iter()).copied().max().unwrap_or(0);
        let (mut bitcount, mut slicecount, mut bitslice) = (0i32, 0i32, max_need + 1);
        // Every band takes at most 16 slices and a valid bitpool fits them all, so this
        // ends; the bound only guards against a bitpool `valid` would refuse.
        for _ in 0..64 {
            bitslice -= 1;
            bitcount += slicecount;
            slicecount = 0;
            for &ch in chans.iter() {
                for sb in 0..ns {
                    let n = need[ch][sb];
                    if n > bitslice + 1 && n < bitslice + 16 { slicecount += 1; } else if n == bitslice + 1 { slicecount += 2; }
                }
            }
            if bitcount + slicecount >= bitpool { break; }
        }
        if bitcount + slicecount == bitpool { bitcount += slicecount; bitslice -= 1; }
        for &ch in chans.iter() {
            for sb in 0..ns {
                let n = need[ch][sb];
                bits[ch][sb] = if n < bitslice + 2 { 0 } else { (n - bitslice).min(16) as u8 };
            }
        }
        // Hand out what is left: first to bands already coded (or about to be), then
        // one bit at a time, channels interleaved within each subband.
        let order = |sb: usize| chans.iter().map(move |&ch| (ch, sb));
        let mut it = (0..ns).flat_map(order);
        while bitcount < bitpool {
            let Some((ch, s)) = it.next() else { break };
            let b = &mut bits[ch][s];
            if *b >= 2 && *b < 16 { *b += 1; bitcount += 1; } else if need[ch][s] == bitslice + 1 && bitpool > bitcount + 1 { *b = 2; bitcount += 2; }
        }
        let mut it = (0..ns).flat_map(order);
        while bitcount < bitpool {
            let Some((ch, s)) = it.next() else { break };
            let b = &mut bits[ch][s];
            if *b < 16 { *b += 1; bitcount += 1; }
        }
    }
    bits
}

/// Writes bits most significant first.
struct Bits<'a> { out: &'a mut [u8], at: usize }
impl Bits<'_> {
    fn put(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            let bit = (value >> i & 1) as u8;
            if let Some(b) = self.out.get_mut(self.at / 8) { *b |= bit << (7 - self.at % 8); }
            self.at += 1;
        }
    }
}

/// The SBC encoder for one stream.
pub struct Encoder {
    pub config: Config,
    /// Analysis history per channel, newest first (the specification's X).
    x: [[i32; 80]; 2],
}

impl Encoder {
    pub fn new(config: Config) -> Option<Encoder> { config.valid().then_some(Encoder { config, x: [[0; 80]; 2] }) }

    /// One block of one channel: `input` holds `subbands` new samples, oldest first.
    /// Returns the subband samples in Q8.
    fn analyse(&mut self, ch: usize, input: &[i32]) -> [i32; 8] {
        let m = self.config.subbands as usize;
        let l = 10 * m;
        let x = &mut self.x[ch][..l];
        x.copy_within(0..l - m, m);
        for (i, &s) in input.iter().enumerate().take(m) { x[m - 1 - i] = s; }
        let (window, matrix): (&[i32], &[i32]) = if m == 8 { (&WINDOW8, &MATRIX8) } else { (&WINDOW4, &MATRIX4) };
        let mut y = [0i64; 16];
        for (k, yk) in y.iter_mut().enumerate().take(2 * m) {
            let mut acc = 0i64;
            for j in 0..5 { let i = k + j * 2 * m; acc += window[i] as i64 * x[i] as i64; }
            *yk = acc >> 8; // Q16
        }
        let mut s = [0i32; 8];
        for (i, si) in s.iter_mut().enumerate().take(m) {
            let mut acc = 0i64;
            for k in 0..2 * m { acc += matrix[i * 2 * m + k] as i64 * y[k]; }
            *si = (acc >> 23) as i32; // Q31 -> Q8
        }
        s
    }

    /// Encode one frame from exactly `config.samples()` stereo pairs (a mono stream takes
    /// their average) and append it to `out`. False (nothing appended) when `pcm` is the
    /// wrong length.
    pub fn encode(&mut self, pcm: &[[i16; 2]], out: &mut Vec<u8>) -> bool {
        let c = self.config;
        let (nb, ns, nc) = (c.blocks as usize, c.subbands as usize, c.channels());
        if pcm.len() != c.samples() { return false; }
        // Subband samples [block][channel][subband], Q8.
        let mut sb = [[[0i32; 8]; 2]; 16];
        for blk in 0..nb {
            let frames = &pcm[blk * ns..(blk + 1) * ns];
            for ch in 0..nc {
                let mut input = [0i32; 8];
                for (i, f) in frames.iter().enumerate() {
                    input[i] = if nc == 1 { (f[0] as i32 + f[1] as i32) >> 1 } else { f[ch] as i32 };
                }
                sb[blk][ch] = self.analyse(ch, &input[..ns]);
            }
        }
        let scale = |v: &mut dyn Iterator<Item = i32>| -> u8 {
            let max = v.map(|s| s.unsigned_abs()).max().unwrap_or(0);
            // Smallest scf with |s| < 2^(scf + 1) (in Q8: 2^(scf + 9)).
            let mut f = 0u8;
            while f < 15 && max >= 1u32 << (f as u32 + 9) { f += 1; }
            f
        };
        let mut scf = [[0u8; 8]; 2];
        for ch in 0..nc { for s in 0..ns { scf[ch][s] = scale(&mut (0..nb).map(|b| sb[b][ch][s])); } }
        let mut join = 0u8;
        if c.mode == Mode::JointStereo {
            // Code a subband as mid/side when that needs smaller scale factors (not the last).
            for s in 0..ns - 1 {
                let mid = scale(&mut (0..nb).map(|b| (sb[b][0][s] + sb[b][1][s]) >> 1));
                let side = scale(&mut (0..nb).map(|b| (sb[b][0][s] - sb[b][1][s]) >> 1));
                if (mid as u32 + side as u32) < scf[0][s] as u32 + scf[1][s] as u32 {
                    join |= 0x80 >> s;
                    scf[0][s] = mid;
                    scf[1][s] = side;
                    for b in sb.iter_mut().take(nb) {
                        let (l, r) = (b[0][s], b[1][s]);
                        b[0][s] = (l + r) >> 1;
                        b[1][s] = (l - r) >> 1;
                    }
                }
            }
        }
        let bits = allocate(&c, &scf);
        let start = out.len();
        out.resize(start + c.frame_bytes(), 0);
        let frame = &mut out[start..];
        frame[0] = SYNCWORD;
        frame[1] = c.header();
        frame[2] = c.bitpool;
        let mut w = Bits { out: &mut frame[4..], at: 0 };
        if c.mode == Mode::JointStereo { w.put(join as u32 >> (8 - ns), ns as u32); }
        for ch in 0..nc { for s in 0..ns { w.put(scf[ch][s] as u32, 4); } }
        for b in sb.iter().take(nb) {
            for ch in 0..nc {
                for s in 0..ns {
                    let n = bits[ch][s] as u32;
                    if n == 0 { continue; }
                    let levels = (1i64 << n) - 1;
                    let f = scf[ch][s] as u32;
                    // q = (s / 2^(scf+1) + 1) * levels / 2, truncated.
                    let q = ((b[ch][s] as i64 + (1i64 << (f + 9))) * levels) >> (f + 10);
                    w.put(q.clamp(0, levels - 1) as u32, n);
                }
            }
        }
        frame[3] = crc_of(frame, &c);
        true
    }
}

pub const WINDOW8: [i32; 80] = [
    0, 2627, 5759, 9305, 13823, 19125, 24770, 29926,
    33753, 35295, 33463, 27121, 15136, -3000, -27678, -58673,
    94951, 134711, 175464, 213863, 245829, 266834, 272141, 257000,
    217050, 148605, 49058, -82473, -245625, -438051, -655572, -892335,
    1140833, 1392253, 1637043, 1865571, 2068036, 2235806, 2361452, 2439237,
    2465497, 2439237, 2361452, 2235806, 2068036, 1865571, 1637043, 1392253,
    -1140833, -892335, -655572, -438051, -245625, -82473, 49058, 148605,
    217050, 257000, 272141, 266834, 245829, 213863, 175464, 134711,
    -94951, -58673, -27678, -3000, 15136, 27121, 33463, 35295,
    33753, 29926, 24770, 19125, 13823, 9305, 5759, 2627,
];
pub const WINDOW4: [i32; 40] = [
    0, 9002, 25030, 45864, 64378, 65298, 31303, -51340,
    183103, 342901, 484455, 540125, 434140, 102885, -483548, -1302690,
    2274878, 3271353, 4137877, 4728293, 4937792, 4728293, 4137877, 3271353,
    -2274878, -1302690, -483548, 102885, 434140, 540125, 484455, 342901,
    -183103, -51340, 31303, 65298, 64378, 45864, 25030, 9002,
];
pub const MATRIX4: [i32; 32] = [
    23170, 30274, 32768, 30274, 23170, 12540, 0, -12540,
    -23170, 12540, 32768, 12540, -23170, -30274, 0, 30274,
    -23170, -12540, 32768, -12540, -23170, 30274, 0, -30274,
    23170, -30274, 32768, -30274, 23170, -12540, 0, 12540,
];
pub const MATRIX8: [i32; 128] = [
    23170, 27246, 30274, 32138, 32768, 32138, 30274, 27246, 23170, 18205, 12540, 6393, 0, -6393, -12540, -18205,
    -23170, -6393, 12540, 27246, 32768, 27246, 12540, -6393, -23170, -32138, -30274, -18205, 0, 18205, 30274, 32138,
    -23170, -32138, -12540, 18205, 32768, 18205, -12540, -32138, -23170, 6393, 30274, 27246, 0, -27246, -30274, -6393,
    23170, -18205, -30274, 6393, 32768, 6393, -30274, -18205, 23170, 27246, -12540, -32138, 0, 32138, 12540, -27246,
    23170, 18205, -30274, -6393, 32768, -6393, -30274, 18205, 23170, -27246, -12540, 32138, 0, -32138, 12540, 27246,
    -23170, 32138, -12540, -18205, 32768, -18205, -12540, 32138, -23170, -6393, 30274, -27246, 0, 27246, -30274, 6393,
    -23170, 6393, 12540, -27246, 32768, -27246, 12540, 6393, -23170, 32138, -30274, 18205, 0, -18205, 30274, -32138,
    23170, -27246, 30274, -32138, 32768, -32138, 30274, -27246, 23170, -18205, 12540, -6393, 0, 6393, -12540, 18205,
];
