// SPDX-License-Identifier: GPL-3.0-only
//! Pure DSP pitch & onset detector for offline Extract MIDI.
//! 0 MB AI required: downsampled autocorrelation / YIN-style F0 + energy onset segmentation.

use crate::{
    dsp_math::{log2_f32, round_f32, sqrt_f32},
    midi::{Error, Note, Pattern, Instrument, MAX_FRAMES, MAX_NOTES, MAX_VOICES},
    Frame, RATE,
};
use alloc::vec::Vec;

/// Offline extraction of MIDI notes from audio frames using deterministic DSP.
pub fn extract_midi(frames: &[Frame], bpm: u16) -> Result<Pattern, Error> {
    if !(30..=240).contains(&bpm) {
        return Err(Error::Range);
    }
    let total_frames = frames.len() as u32;
    if total_frames == 0 || total_frames > MAX_FRAMES {
        return Err(Error::Range);
    }

    const BLOCK_SIZE: usize = 512;
    const HOP_SIZE: usize = 256;
    const SILENCE_THRESH: f32 = 250.0; // i16 amplitude threshold

    let num_hops = frames.len() / HOP_SIZE;
    if num_hops == 0 {
        return Ok(Pattern {
            frames: total_frames,
            bpm,
            instrument: Instrument::SilkTriangle,
            notes: Vec::new(),
        });
    }

    let mut detected_pitches: Vec<(usize, u8, u8, bool)> = Vec::new(); // (hop_idx, key, velocity, is_onset)
    detected_pitches
        .try_reserve(num_hops)
        .map_err(|_| Error::Memory)?;

    let mut prev_energy = 0.0f32;

    for hop in 0..num_hops {
        let start = hop * HOP_SIZE;
        let end = (start + BLOCK_SIZE).min(frames.len());
        let slice = &frames[start..end];
        if slice.len() < 256 {
            break;
        }

        // Calculate RMS energy (mono average)
        let mut sum_sq = 0.0f32;
        for f in slice {
            let mono = (f[0] as f32 + f[1] as f32) * 0.5;
            sum_sq += mono * mono;
        }
        let rms = sqrt_f32(sum_sq / slice.len() as f32);

        if rms < SILENCE_THRESH {
            prev_energy = rms;
            continue;
        }

        let is_onset = prev_energy > SILENCE_THRESH && (rms - prev_energy) > prev_energy * 0.5;

        // Pitch detection via normalized autocorrelation
        // Lag range 50 Hz..1000 Hz at 48 kHz -> lag 48..960
        let max_lag = 960.min(slice.len() - 2);
        let min_lag = 48;

        let mut corrs = [0.0f32; 962];
        let mut global_max = -1.0f32;

        for lag in min_lag..=max_lag {
            let count = slice.len() - lag;
            let mut sum_prod = 0.0f32;
            let mut sum_sq0 = 0.0f32;
            let mut sum_sq1 = 0.0f32;
            for i in 0..count {
                let x0 = (slice[i][0] as f32 + slice[i][1] as f32) * 0.5;
                let x1 = (slice[i + lag][0] as f32 + slice[i + lag][1] as f32) * 0.5;
                sum_prod += x0 * x1;
                sum_sq0 += x0 * x0;
                sum_sq1 += x1 * x1;
            }
            let denom = sqrt_f32(sum_sq0 * sum_sq1);
            let r = if denom > 1e-4 { sum_prod / denom } else { 0.0 };
            corrs[lag] = r;
            if r > global_max {
                global_max = r;
            }
        }

        // Find the first local maximum peak that is close to the global maximum (avoids octave subharmonic errors)
        let mut best_lag = 0usize;
        let peak_threshold = (global_max * 0.85).max(0.4);

        for lag in (min_lag + 1)..max_lag {
            if corrs[lag] > corrs[lag - 1] && corrs[lag] >= corrs[lag + 1] && corrs[lag] >= peak_threshold {
                best_lag = lag;
                break;
            }
        }

        if best_lag == 0 && global_max >= 0.4 {
            for lag in min_lag..=max_lag {
                if corrs[lag] == global_max {
                    best_lag = lag;
                    break;
                }
            }
        }

        if best_lag >= min_lag && best_lag < max_lag {
            // Parabolic interpolation for sub-sample lag accuracy
            let a = corrs[best_lag - 1];
            let b = corrs[best_lag];
            let c = corrs[best_lag + 1];
            let denom = a - 2.0 * b + c;
            let delta = if denom.abs() > 1e-6 {
                (a - c) / (2.0 * denom)
            } else {
                0.0
            };
            let fine_lag = best_lag as f32 + delta.clamp(-0.5, 0.5);

            let freq = RATE as f32 / fine_lag;
            let key_f = 69.0 + 12.0 * log2_f32(freq / 440.0);
            let key = (round_f32(key_f) as i32).clamp(1, 127) as u8;

            // Velocity from RMS amplitude
            let vel = ((rms / 16000.0) * 127.0).clamp(1.0, 127.0) as u8;
            detected_pitches.push((hop, key, vel, is_onset));
        }

        prev_energy = rms;
    }

    // Segment detected hops into notes
    let mut notes: Vec<Note> = Vec::new();
    notes.try_reserve(MAX_NOTES).map_err(|_| Error::Memory)?;

    let mut current_note: Option<(usize, usize, u8, u8)> = None; // (start_hop, end_hop, key, max_vel)

    for &(hop, key, vel, is_onset) in &detected_pitches {
        match current_note {
            None => {
                current_note = Some((hop, hop + 1, key, vel));
            }
            Some((start_hop, end_hop, cur_key, max_vel)) => {
                if !is_onset && hop == end_hop && key == cur_key {
                    // Contiguous and same pitch without onset: extend note
                    current_note = Some((start_hop, hop + 1, cur_key, max_vel.max(vel)));
                } else {
                    // Note ended
                    let start_frame = (start_hop * HOP_SIZE) as u32;
                    let note_frames = ((end_hop - start_hop) * HOP_SIZE) as u32;
                    if note_frames >= 480 && notes.len() < MAX_NOTES {
                        let clamped_frames = note_frames.min(total_frames.saturating_sub(start_frame));
                        if clamped_frames > 0 {
                            notes.push(Note {
                                start: start_frame,
                                frames: clamped_frames,
                                key: cur_key,
                                velocity: max_vel,
                            });
                        }
                    }
                    current_note = Some((hop, hop + 1, key, vel));
                }
            }
        }
    }

    if let Some((start_hop, end_hop, cur_key, max_vel)) = current_note {
        let start_frame = (start_hop * HOP_SIZE) as u32;
        let note_frames = ((end_hop - start_hop) * HOP_SIZE) as u32;
        if note_frames >= 480 && notes.len() < MAX_NOTES {
            let clamped_frames = note_frames.min(total_frames.saturating_sub(start_frame));
            if clamped_frames > 0 {
                notes.push(Note {
                    start: start_frame,
                    frames: clamped_frames,
                    key: cur_key,
                    velocity: max_vel,
                });
            }
        }
    }

    // Validate polyphony constraint (<= MAX_VOICES)
    for note in &notes {
        let count = notes
            .iter()
            .filter(|other| other.start <= note.start && other.start + other.frames > note.start)
            .count();
        if count > MAX_VOICES {
            return Err(Error::Polyphony);
        }
    }

    let pattern = Pattern {
        frames: total_frames,
        bpm,
        instrument: Instrument::SilkTriangle,
        notes,
    };

    pattern.validate()?;
    Ok(pattern)
}
