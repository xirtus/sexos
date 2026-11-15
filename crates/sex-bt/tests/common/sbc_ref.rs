//! A floating-point SBC decoder written straight from the A2DP specification's
//! decoding process (appendix B: frame unpacking, bit allocation, reconstruction,
//! joint-stereo, synthesis filterbank). It shares no code with `sex_bt::sbc` but the
//! prototype filter tables, so the host tests check the integer encoder against an
//! independent reading of the specification.
#![allow(dead_code)]

use sex_bt::sbc::{WINDOW4, WINDOW8};

pub struct Frame { pub rate: u32, pub channels: usize, pub subbands: usize, pub blocks: usize, pub joint: bool, pub bitpool: u8, pub crc_ok: bool, pub bytes: usize }

struct Reader<'a> { d: &'a [u8], at: usize }
impl Reader<'_> {
    fn get(&mut self, n: usize) -> Option<u32> {
        let mut v = 0;
        for _ in 0..n {
            let b = *self.d.get(self.at / 8)? >> (7 - self.at % 8) & 1;
            v = v << 1 | b as u32;
            self.at += 1;
        }
        Some(v)
    }
}

fn crc(bytes: &[u8], bits: usize) -> u8 {
    let mut c = 0x0Fu8;
    for i in 0..bits {
        let b = bytes[i / 8] >> (7 - i % 8) & 1;
        let fb = (c >> 7) ^ b;
        c <<= 1;
        if fb == 1 { c ^= 0x1D; }
    }
    c
}

fn bit_allocation(mode: u8, alloc_snr: bool, fi: usize, ns: usize, nc: usize, bitpool: i32, scf: &[[i32; 8]; 2]) -> [[i32; 8]; 2] {
    let off4 = [[-1, 0, 0, 0], [-2, 0, 0, 1], [-2, 0, 0, 1], [-2, 0, 0, 1]];
    let off8 = [[-2, 0, 0, 0, 0, 0, 0, 1], [-3, 0, 0, 0, 0, 0, 1, 2], [-4, 0, 0, 0, 0, 0, 1, 2], [-4, 0, 0, 0, 0, 0, 1, 2]];
    let mut bitneed = [[0i32; 8]; 2];
    let mut bits = [[0i32; 8]; 2];
    for ch in 0..nc {
        for sb in 0..ns {
            if alloc_snr { bitneed[ch][sb] = scf[ch][sb]; } else if scf[ch][sb] == 0 { bitneed[ch][sb] = -5; } else {
                let loudness = scf[ch][sb] - if ns == 4 { off4[fi][sb] } else { off8[fi][sb] };
                bitneed[ch][sb] = if loudness > 0 { loudness / 2 } else { loudness };
            }
        }
    }
    let joint_pool = mode >= 2;
    let passes: Vec<Vec<usize>> = if joint_pool { vec![vec![0, 1]] } else { (0..nc).map(|c| vec![c]).collect() };
    for chans in passes {
        let mut max_bitneed = i32::MIN;
        for &ch in &chans { for sb in 0..ns { max_bitneed = max_bitneed.max(bitneed[ch][sb]); } }
        let mut bitcount = 0;
        let mut slicecount = 0;
        let mut bitslice = max_bitneed + 1;
        loop {
            bitslice -= 1;
            bitcount += slicecount;
            slicecount = 0;
            for &ch in &chans {
                for sb in 0..ns {
                    if bitneed[ch][sb] > bitslice + 1 && bitneed[ch][sb] < bitslice + 16 { slicecount += 1; }
                    else if bitneed[ch][sb] == bitslice + 1 { slicecount += 2; }
                }
            }
            if bitcount + slicecount >= bitpool { break; }
        }
        if bitcount + slicecount == bitpool { bitcount += slicecount; bitslice -= 1; }
        for &ch in &chans {
            for sb in 0..ns { bits[ch][sb] = if bitneed[ch][sb] < bitslice + 2 { 0 } else { (bitneed[ch][sb] - bitslice).min(16) }; }
        }
        let mut k = 0;
        while bitcount < bitpool && k < ns * chans.len() {
            let (ch, sb) = (chans[k % chans.len()], k / chans.len());
            if bits[ch][sb] >= 2 && bits[ch][sb] < 16 { bits[ch][sb] += 1; bitcount += 1; }
            else if bitneed[ch][sb] == bitslice + 1 && bitpool > bitcount + 1 { bits[ch][sb] = 2; bitcount += 2; }
            k += 1;
        }
        let mut k = 0;
        while bitcount < bitpool && k < ns * chans.len() {
            let (ch, sb) = (chans[k % chans.len()], k / chans.len());
            if bits[ch][sb] < 16 { bits[ch][sb] += 1; bitcount += 1; }
            k += 1;
        }
    }
    bits
}

/// The decoder's state: one synthesis history per channel.
pub struct Decoder { v: [[f64; 160]; 2] }

impl Decoder {
    pub fn new() -> Decoder { Decoder { v: [[0.0; 160]; 2] } }

    /// Decode one frame; returns its description and its PCM (one Vec per channel).
    pub fn decode(&mut self, f: &[u8]) -> Option<(Frame, Vec<Vec<f64>>)> {
        if f.len() < 4 || f[0] != 0x9C { return None; }
        let fi = (f[1] >> 6) as usize;
        let blocks = [4, 8, 12, 16][(f[1] >> 4 & 3) as usize];
        let mode = f[1] >> 2 & 3;
        let snr = f[1] & 2 != 0;
        let ns = if f[1] & 1 != 0 { 8 } else { 4 };
        let nc = if mode == 0 { 1 } else { 2 };
        let bitpool = f[2];
        let mut r = Reader { d: &f[4..], at: 0 };
        let mut join = [false; 8];
        if mode == 3 { for j in join.iter_mut().take(ns) { *j = r.get(1)? == 1; } }
        let mut scf = [[0i32; 8]; 2];
        for ch in 0..nc { for sb in 0..ns { scf[ch][sb] = r.get(4)? as i32; } }
        // CRC over header bytes 1-2 then the bits read so far.
        let covered = r.at;
        let mut cbytes = vec![f[1], f[2]];
        cbytes.extend_from_slice(&f[4..4 + covered.div_ceil(8)]);
        let crc_ok = crc(&cbytes, 16 + covered) == f[3];
        let bits = bit_allocation(mode, snr, fi, ns, nc, bitpool as i32, &scf);
        let mut sb_samples = vec![[[0f64; 8]; 2]; blocks];
        for blk in 0..blocks {
            for ch in 0..nc {
                for sb in 0..ns {
                    let n = bits[ch][sb];
                    if n == 0 { continue; }
                    let q = r.get(n as usize)? as f64;
                    let levels = ((1u32 << n) - 1) as f64;
                    sb_samples[blk][ch][sb] = 2f64.powi(scf[ch][sb] + 1) * ((q * 2.0 + 1.0) / levels - 1.0);
                }
            }
        }
        if mode == 3 {
            for b in sb_samples.iter_mut() {
                for sb in 0..ns {
                    if join[sb] { let (m, s) = (b[0][sb], b[1][sb]); b[0][sb] = m + s; b[1][sb] = m - s; }
                }
            }
        }
        let bytes = 4 + r.at.div_ceil(8);
        let mut pcm = vec![Vec::with_capacity(blocks * ns); nc];
        for b in &sb_samples {
            for ch in 0..nc {
                let out = self.synthesise(ch, ns, &b[ch][..ns]);
                pcm[ch].extend_from_slice(&out);
            }
        }
        Some((Frame { rate: [16000, 32000, 44100, 48000][fi], channels: nc, subbands: ns, blocks, joint: mode == 3, bitpool, crc_ok, bytes }, pcm))
    }

    fn synthesise(&mut self, ch: usize, m: usize, s: &[f64]) -> Vec<f64> {
        let v = &mut self.v[ch];
        let len = 20 * m;
        v.copy_within(0..len - 2 * m, 2 * m);
        for k in 0..2 * m {
            v[k] = (0..m).map(|i| (((i as f64 + 0.5) * (k as f64 + m as f64 / 2.0) * std::f64::consts::PI) / m as f64).cos() * s[i]).sum();
        }
        let mut u = vec![0f64; 10 * m];
        for i in 0..5 {
            for j in 0..m {
                u[i * 2 * m + j] = v[i * 4 * m + j];
                u[i * 2 * m + j + m] = v[i * 4 * m + j + 3 * m];
            }
        }
        let window = |i: usize| if m == 8 { WINDOW8[i] as f64 } else { WINDOW4[i] as f64 } / (1u64 << 24) as f64 * -(m as f64);
        let w: Vec<f64> = (0..10 * m).map(|i| u[i] * window(i)).collect();
        (0..m).map(|j| (0..10).map(|i| w[j + m * i]).sum()).collect()
    }
}
