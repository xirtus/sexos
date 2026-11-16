// SPDX-License-Identifier: GPL-3.0-only
//! Host reference for the guest effect waveform checker. Not a guest/Files export.
use moan_core::{
    effects::{Compressor, Equalizer, Reverb},
    Clip, Track, RATE,
};
fn main() {
    let left: Vec<_> = (0..RATE)
        .map(|i| [if i % 120 < 60 { 6000 } else { -6000 }; 2])
        .collect();
    let right: Vec<_> = (0..RATE)
        .map(|i| [if i % 80 < 40 { 6000 } else { -6000 }; 2])
        .collect();
    let mut a = Track::new();
    a.clips.push(Clip::new(&left, 0).unwrap());
    a.set_pan(-100).unwrap();
    let mut b = Track::new();
    let mut c = Clip::new(&right, 0).unwrap();
    c.set_gain(200).unwrap();
    b.clips.push(c);
    b.set_pan(100).unwrap();
    b.set_volume(50).unwrap();
    for t in [&mut a, &mut b] {
        t.effects.eq = Some(Equalizer::new(100, 100, 100).unwrap());
        t.effects.reverb = Some(Reverb::new(70, 25).unwrap());
    }
    a.effects.compressor = Some(Compressor::new(2000, 4, 1, 480).unwrap());
    b.effects.compressor = Some(Compressor::new(4000, 4, 1, 480).unwrap());
    let mut output = vec![[0i16; 2]; RATE as usize * 2];
    moan_core::render_processed(&mut [a, b], 0, &mut output);
    let length = (output.len() * 4) as u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(length + 36).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&length.to_le_bytes());
    for f in output {
        for sample in f {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
    }
    std::fs::write(std::env::args().nth(1).expect("output WAV path"), wav).unwrap();
}
