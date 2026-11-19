// SPDX-License-Identifier: GPL-3.0-only
//! Bounded editable MIDI and original instruments. Cache construction happens on
//! edits, never inside the live audio render path. Frames use Moan's 48 kHz clock.
use crate::{Frame, RATE};
use alloc::{sync::Arc, vec::Vec};
pub const MAX_NOTES: usize = 128;
pub const MAX_VOICES: usize = 32;
pub const MAX_FRAMES: u32 = RATE * 60;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Instrument {
    SilkTriangle = 0,
    SatinPulse = 1,
    VelvetSampler = 2,
    ArielFormant = 3,
    PrismWavetable = 4,
}
impl Instrument {
    pub fn name(self) -> &'static str {
        match self {
            Self::SilkTriangle => "Silk Keys",
            Self::SatinPulse => "Satin Bass",
            Self::VelvetSampler => "Velvet Kit",
            Self::ArielFormant => "Ariel Voice",
            Self::PrismWavetable => "Prism Synth",
        }
    }
}
impl TryFrom<u8> for Instrument {
    type Error = Error;
    fn try_from(v: u8) -> Result<Self, Error> {
        match v {
            0 => Ok(Self::SilkTriangle),
            1 => Ok(Self::SatinPulse),
            2 => Ok(Self::VelvetSampler),
            3 => Ok(Self::ArielFormant),
            4 => Ok(Self::PrismWavetable),
            _ => Err(Error::Instrument),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    pub start: u32,
    pub frames: u32,
    pub key: u8,
    pub velocity: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub frames: u32,
    pub bpm: u16,
    pub instrument: Instrument,
    pub notes: Vec<Note>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Range,
    Note,
    Polyphony,
    Memory,
    Instrument,
}
/// Direct rational eighth-note positions. Never accumulate rounded step lengths.
/// Sixteen steps cover two 4/4 bars; UI tempo is displayed, not a decorative knob.
pub fn step_frame(bpm: u16, step: u32) -> Result<u32, Error> {
    if !(30..=240).contains(&bpm) || step > 16 {
        return Err(Error::Range);
    }
    Ok((RATE as u64 * 30 * step as u64 / bpm as u64) as u32)
}
impl Pattern {
    /// Stable metadata fingerprint for save/reopen receipts; not authentication.
    pub fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                hash = (hash ^ *b as u64).wrapping_mul(0x100000001b3);
            }
        };
        feed(&self.frames.to_le_bytes());
        feed(&self.bpm.to_le_bytes());
        feed(&[self.instrument as u8]);
        for n in &self.notes {
            feed(&n.start.to_le_bytes());
            feed(&n.frames.to_le_bytes());
            feed(&[n.key, n.velocity]);
        }
        hash
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.frames == 0
            || self.frames > MAX_FRAMES
            || !(30..=240).contains(&self.bpm)
            || self.notes.len() > MAX_NOTES
        {
            return Err(Error::Range);
        }
        for note in &self.notes {
            if note.key > 127
                || !(1..=127).contains(&note.velocity)
                || note.frames == 0
                || note
                    .start
                    .checked_add(note.frames)
                    .is_none_or(|end| end > self.frames)
            {
                return Err(Error::Note);
            }
        }
        for note in &self.notes {
            if self
                .notes
                .iter()
                .filter(|other| {
                    other.start <= note.start && other.start + other.frames > note.start
                })
                .count()
                > MAX_VOICES
            {
                return Err(Error::Polyphony);
            }
        }
        Ok(())
    }
    pub fn preset(instrument: Instrument) -> Self {
        let mut notes = Vec::new();
        for step in 0..8u32 {
            let key = match instrument {
                Instrument::VelvetSampler => 60,
                Instrument::SatinPulse => [48, 48, 55, 53][step as usize % 4],
                _ => [60, 64, 67, 69][step as usize % 4],
            };
            notes.push(Note {
                start: step * RATE / 2,
                frames: RATE * 3 / 8,
                key,
                velocity: 96,
            });
        }
        Self {
            frames: RATE * 4,
            bpm: 120,
            instrument,
            notes,
        }
    }
    /// Render immutable cached PCM; bounded memory/voices and no filesystem or IPC.
    pub fn bake(&self) -> Result<Arc<Vec<Frame>>, Error> {
        self.validate()?;
        let mut mix: Vec<[i32; 2]> = Vec::new();
        mix.try_reserve_exact(self.frames as usize)
            .map_err(|_| Error::Memory)?;
        mix.resize(self.frames as usize, [0; 2]);
        if self.instrument == Instrument::PrismWavetable {
            let params = crate::wavetable::PrismParams::default();
            for note in &self.notes {
                let note_frames = crate::wavetable::render_prism_note(note.key, note.velocity, note.frames, &params)?;
                for (offset, f) in note_frames.into_iter().enumerate() {
                    let target = &mut mix[(note.start as usize) + offset];
                    for c in 0..2 {
                        target[c] += f[c] as i32;
                    }
                }
            }
        } else if self.instrument == Instrument::ArielFormant {
            let vocal_params = crate::vocal::VoiceParams::default();
            for note in &self.notes {
                let note_hz = 440.0 * crate::dsp_math::exp2_f32((note.key as f32 - 69.0) / 12.0);
                let note_ms = (note.frames as f32 * 1000.0 / RATE as f32).max(10.0);
                let mut p = vocal_params.clone();
                p.f0 = note_hz;
                p.rate_ms = note_ms;
                let (pcm, _) = crate::vocal::synthesize("AA", &p).map_err(|_| Error::Memory)?;
                let len = (note.frames as usize).min(pcm.len());
                let vel = note.velocity as i32;
                for offset in 0..len {
                    let target = &mut mix[(note.start as usize) + offset];
                    for c in 0..2 {
                        target[c] += pcm[offset][c] as i32 * vel / 127;
                    }
                }
            }
        } else {
            // The kit is original PCM, sampled at variable pitch rather than an
            // oscillator masquerading as a sampler. No downloaded/copied audio.
            let kit = if self.instrument == Instrument::VelvetSampler {
                Some(velvet_sample()?)
            } else {
                None
            };
            for note in &self.notes {
                for offset in 0..note.frames {
                    let value = match &kit {
                        Some(sample) => {
                            let at = offset as u64 * STEPS[note.key as usize] as u64 / STEPS[60] as u64;
                            sample.get(at as usize).map_or(0, |v| v[0] as i32)
                        }
                        None => {
                            let phase = (offset as u64 * STEPS[note.key as usize] as u64) as u32;
                            if self.instrument == Instrument::SatinPulse {
                                if phase < 0x6000_0000 {
                                    6000
                                } else {
                                    -6000
                                }
                            } else {
                                let x = (phase >> 16) as i32;
                                let triangle = if x < 32768 {
                                    x * 2 - 32768
                                } else {
                                    98303 - x * 2
                                };
                                triangle * 8000 / 32768
                            }
                        }
                    };
                    let attack = (offset as i64 * 65536 / 48).min(65536);
                    let release = ((note.frames - 1 - offset) as i64 * 65536 / 240).min(65536);
                    let value = (value as i64 * note.velocity as i64 / 127 * attack / 65536 * release
                        / 65536) as i32;
                    let target = &mut mix[(note.start + offset) as usize];
                    for c in target {
                        *c += value;
                    }
                }
            }
        }
        let mut pcm = Vec::new();
        pcm.try_reserve_exact(mix.len())
            .map_err(|_| Error::Memory)?;
        pcm.extend(
            mix.into_iter()
                .map(|f| f.map(|v| v.clamp(-32768, 32767) as i16)),
        );
        Ok(Arc::new(pcm))
    }
}
fn velvet_sample() -> Result<Vec<Frame>, Error> {
    let mut sample = Vec::new();
    sample.try_reserve_exact(9600).map_err(|_| Error::Memory)?;
    let mut phase = 0u32;
    let mut noise = 0x51a7_ba55u32;
    for i in 0..9600u32 {
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        phase = phase
            .wrapping_add(((110 - i.min(7200) / 100) as u64 * (1u64 << 32) / RATE as u64) as u32);
        let x = (phase >> 16) as i32;
        let tri = if x < 32768 {
            x * 2 - 32768
        } else {
            98303 - x * 2
        };
        let value = (tri as i64 * 11000 / 32768
            + (noise as i32 as i64 / 524288) * (1000 - i.min(1000)) as i64 / 1000)
            * (9600 - i) as i64
            / 9600;
        sample.push([value.clamp(-32768, 32767) as i16; 2]);
    }
    Ok(sample)
}
// Equal temperament, A4=440 Hz, phase increments rounded for 48 kHz.
const STEPS: [u32; 128] = [
    731558, 775059, 821146, 869974, 921705, 976513, 1034579, 1096099, 1161276, 1230329, 1303488,
    1380998, 1463116, 1550118, 1642292, 1739948, 1843411, 1953026, 2069159, 2192197, 2322552,
    2460658, 2606977, 2761996, 2926232, 3100235, 3284585, 3479896, 3686822, 3906052, 4138318,
    4384395, 4645104, 4921317, 5213953, 5523991, 5852465, 6200470, 6569170, 6959793, 7373644,
    7812103, 8276635, 8768789, 9290209, 9842633, 10427907, 11047982, 11704930, 12400941, 13138339,
    13919586, 14747287, 15624207, 16553270, 17537579, 18580418, 19685267, 20855814, 22095965,
    23409859, 24801882, 26276679, 27839171, 29494575, 31248413, 33106541, 35075158, 37160835,
    39370534, 41711627, 44191930, 46819719, 49603764, 52553357, 55678342, 58989149, 62496826,
    66213081, 70150316, 74321671, 78741067, 83423255, 88383859, 93639437, 99207528, 105106715,
    111356685, 117978298, 124993653, 132426162, 140300631, 148643341, 157482134, 166846509,
    176767719, 187278874, 198415056, 210213429, 222713370, 235956596, 249987305, 264852324,
    280601263, 297286682, 314964268, 333693018, 353535438, 374557749, 396830112, 420426858,
    445426740, 471913192, 499974611, 529704648, 561202526, 594573365, 629928537, 667386037,
    707070876, 749115498, 793660223, 840853716, 890853480, 943826385, 999949222, 1059409297,
    1122405052,
];
