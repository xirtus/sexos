//! SBC encoder goldens: frame header and length, CRC-8, and decoding the encoder's
//! frames with an independent floating-point decoder (`common/sbc_ref.rs`): a sine
//! comes back with a high signal-to-noise ratio in every mode the encoder supports.
mod common;
use common::sbc_ref::Decoder;
use sex_bt::sbc::{self, Allocation, Config, Encoder, Mode};

fn tone(hz: f64, amp: f64, rate: u32, n: usize, phase_r: f64) -> Vec<[i16; 2]> {
    (0..n).map(|i| {
        let t = i as f64 / rate as f64;
        let l = (2.0 * std::f64::consts::PI * hz * t).sin() * amp;
        let r = (2.0 * std::f64::consts::PI * hz * 1.5 * t + phase_r).sin() * amp * 0.5;
        [l.round() as i16, r.round() as i16]
    }).collect()
}

/// Encode `pcm`, decode it again, and return (decoded channels, frames).
fn round_trip(c: Config, pcm: &[[i16; 2]]) -> (Vec<Vec<f64>>, Vec<Vec<u8>>) {
    let mut e = Encoder::new(c).expect("valid config");
    let mut d = Decoder::new();
    let mut out = vec![vec![]; c.channels()];
    let mut frames = vec![];
    for chunk in pcm.chunks_exact(c.samples()) {
        let mut f = vec![];
        assert!(e.encode(chunk, &mut f));
        assert_eq!(f.len(), c.frame_bytes());
        let (info, p) = d.decode(&f).expect("decodes");
        assert!(info.crc_ok, "CRC of frame {}", frames.len());
        assert_eq!(info.bytes, f.len(), "length by the spec's formula");
        assert_eq!((info.rate, info.subbands, info.blocks, info.bitpool), (c.rate, c.subbands as usize, c.blocks as usize, c.bitpool));
        for (o, ch) in out.iter_mut().zip(p) { o.extend(ch); }
        frames.push(f);
    }
    (out, frames)
}

/// SNR in dB of `got` against `want` after the codec's delay (searched), skipping the start.
fn snr(want: &[f64], got: &[f64]) -> f64 {
    let mut best = f64::MIN;
    for delay in 0..400 {
        let (mut sig, mut err) = (0.0, 0.0);
        for i in 1000..want.len().min(got.len() - delay) {
            sig += want[i] * want[i];
            let e = want[i] - got[i + delay];
            err += e * e;
        }
        best = best.max(10.0 * (sig / err.max(1e-9)).log10());
    }
    best
}

#[test]
fn header_and_length_of_the_usual_streams() {
    let c = Config::HIGH_QUALITY_48K;
    assert!(c.valid());
    assert_eq!(c.frame_bytes(), 119);
    assert_eq!(c.samples(), 128);
    let mut e = Encoder::new(c).unwrap();
    let mut f = vec![];
    e.encode(&[[0, 0]; 128], &mut f);
    assert_eq!(&f[..3], &[0x9C, 0xFD, 0x35]);
    // 44.1 kHz, the same coding: the familiar 9C BD 35 header, 119 bytes.
    let c44 = Config { rate: 44_100, ..c };
    let mut f = vec![];
    Encoder::new(c44).unwrap().encode(&[[0, 0]; 128], &mut f);
    assert_eq!((&f[..3], f.len()), (&[0x9C, 0xBD, 0x35][..], 119));
    assert_eq!(sbc::parse_header(&f), Some((c44, 119)));
    assert!(sbc::check_crc(&f));
    // Other modes' lengths (A2DP 12.9).
    let m = Config { rate: 16_000, blocks: 4, mode: Mode::Mono, allocation: Allocation::Snr, subbands: 4, bitpool: 10 };
    assert_eq!(m.frame_bytes(), 4 + 2 + 5);
    let d = Config { mode: Mode::DualChannel, subbands: 8, bitpool: 20, blocks: 8, ..m };
    assert_eq!(d.frame_bytes(), 4 + 8 + 40);
    let s = Config { mode: Mode::Stereo, ..d };
    assert_eq!(s.frame_bytes(), 4 + 8 + 20);
    // Refused: bitpool over the mode's limit, odd blocks, wrong sizes.
    assert!(!Config { bitpool: 129, ..m }.valid());
    assert!(!Config { blocks: 5, ..c }.valid());
    assert!(Encoder::new(Config { subbands: 6, ..c }).is_none());
    assert!(!e.encode(&[[0, 0]; 127], &mut vec![]));
}

#[test]
fn crc8_matches_the_specification_polynomial() {
    // x^8 + x^4 + x^3 + x^2 + 1, initial 0x0F, MSB first (values from an independent
    // bitwise implementation).
    assert_eq!(sbc::crc8(&[], 0), 0x0F);
    assert_eq!(sbc::crc8(&[0xFD, 0x35], 16), common_crc(&[0xFD, 0x35], 16));
    assert_eq!(sbc::crc8(&[0xBD, 0x35, 0x12, 0x34, 0x50], 36), common_crc(&[0xBD, 0x35, 0x12, 0x34, 0x50], 36));
    // A damaged frame fails its CRC.
    let mut f = vec![];
    Encoder::new(Config::HIGH_QUALITY_48K).unwrap().encode(&tone(997.0, 8000.0, 48_000, 128, 0.0), &mut f);
    assert!(sbc::check_crc(&f));
    f[5] ^= 0x10;
    assert!(!sbc::check_crc(&f));
}

fn common_crc(d: &[u8], bits: usize) -> u8 {
    let mut reg: u16 = 0x0F;
    for i in 0..bits {
        let b = (d[i / 8] >> (7 - i % 8) & 1) as u16;
        reg = (reg << 1) | 0;
        if ((reg >> 8) & 1) ^ b == 1 { reg ^= 0x11D; } else { reg &= 0xFF; }
        reg &= 0xFF;
    }
    reg as u8
}

#[test]
fn sine_round_trip_high_quality_48k() {
    let c = Config::HIGH_QUALITY_48K;
    let pcm = tone(1000.0, 12_000.0, 48_000, 128 * 200, 0.3);
    let (out, _) = round_trip(c, &pcm);
    let l: Vec<f64> = pcm.iter().map(|p| p[0] as f64).collect();
    let r: Vec<f64> = pcm.iter().map(|p| p[1] as f64).collect();
    let (sl, sr) = (snr(&l, &out[0]), snr(&r, &out[1]));
    assert!(sl > 45.0 && sr > 45.0, "SNR left {sl:.1} dB right {sr:.1} dB");
}

#[test]
fn every_mode_decodes() {
    let pcm = tone(440.0, 9_000.0, 44_100, 128 * 120, 1.0);
    let l: Vec<f64> = pcm.iter().map(|p| p[0] as f64).collect();
    for mode in [Mode::Mono, Mode::DualChannel, Mode::Stereo, Mode::JointStereo] {
        for subbands in [4u8, 8] {
            for allocation in [Allocation::Loudness, Allocation::Snr] {
                for blocks in [4u8, 16] {
                    let bitpool = if matches!(mode, Mode::Mono | Mode::DualChannel) { 2 * subbands * 2 } else { 4 * subbands * 2 }.min(53);
                    let c = Config { rate: 44_100, blocks, mode, allocation, subbands, bitpool };
                    let n = pcm.len() / c.samples() * c.samples();
                    let (out, _) = round_trip(c, &pcm[..n]);
                    let want: Vec<f64> = if mode == Mode::Mono { pcm.iter().map(|p| ((p[0] as i32 + p[1] as i32) >> 1) as f64).collect() } else { l.clone() };
                    let s = snr(&want[..n], &out[0]);
                    assert!(s > 40.0, "{c:?}: SNR {s:.1} dB");
                }
            }
        }
    }
}

#[test]
fn silence_and_full_scale_stay_in_range() {
    let c = Config::HIGH_QUALITY_48K;
    let (out, frames) = round_trip(c, &vec![[0, 0]; 128 * 10]);
    assert!(out[0].iter().all(|s| s.abs() < 4.0), "silence decodes to silence");
    assert!(frames.iter().all(|f| sbc::check_crc(f)));
    // Full-scale square wave: clipping in the encoder, never a panic or a bad frame.
    let pcm: Vec<[i16; 2]> = (0..128 * 20).map(|i| if i / 24 % 2 == 0 { [i16::MAX, i16::MIN] } else { [i16::MIN, i16::MAX] }).collect();
    let (out, _) = round_trip(c, &pcm);
    assert!(out[0].iter().all(|s| s.abs() < 80_000.0));
}

#[test]
fn malformed_headers_are_refused() {
    assert_eq!(sbc::parse_header(&[]), None);
    assert_eq!(sbc::parse_header(&[0x9C, 0xFD]), None);
    assert_eq!(sbc::parse_header(&[0x9D, 0xFD, 0x35, 0]), None);
    // Bitpool 0 and over the mode's maximum.
    assert_eq!(sbc::parse_header(&[0x9C, 0xFD, 0x00, 0, 0, 0]), None);
    assert_eq!(sbc::parse_header(&[0x9C, 0xFD, 0xFF, 0, 0, 0]), None);
    // A header promising more than is there.
    assert_eq!(sbc::parse_header(&[0x9C, 0xFD, 0x35, 0, 0, 0]), None);
    assert!(!sbc::check_crc(&[0x9C]));
}

#[test]
fn music_like_signal_keeps_its_shape() {
    // Three tones and a little noise in both channels, different per channel: the bits
    // spread over many subbands, as with music.
    let mut seed = 0x1234_5678u32;
    let mut noise = || { seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223); (seed >> 16) as f64 / 65536.0 - 0.5 };
    let pcm: Vec<[i16; 2]> = (0..128 * 200).map(|i| {
        let t = i as f64 / 48_000.0;
        let w = |f: f64| (2.0 * std::f64::consts::PI * f * t).sin();
        let l = 6000.0 * w(220.0) + 3000.0 * w(1870.0) + 1500.0 * w(7300.0) + 400.0 * noise();
        let r = 5000.0 * w(330.0) + 2500.0 * w(2500.0) + 1200.0 * w(11_000.0) + 400.0 * noise();
        [l as i16, r as i16]
    }).collect();
    let (out, _) = round_trip(Config::HIGH_QUALITY_48K, &pcm);
    let l: Vec<f64> = pcm.iter().map(|p| p[0] as f64).collect();
    let r: Vec<f64> = pcm.iter().map(|p| p[1] as f64).collect();
    let (sl, sr) = (snr(&l, &out[0]), snr(&r, &out[1]));
    assert!(sl > 25.0 && sr > 25.0, "SNR left {sl:.1} dB right {sr:.1} dB");
}
