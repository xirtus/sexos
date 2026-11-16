// SPDX-License-Identifier: GPL-3.0-only
//! Suno Studio Clip Settings: On Beat, Transpose, and Speed warping.
//! Implements non-destructive windowed overlap-add (WSOLA) pitch-shifting and time-stretching.

use crate::{
    dsp_math::{cos_f32, exp2_f32, floor_f32, PI},
    Frame,
};
use alloc::{sync::Arc, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Range,
    Memory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarpSpec {
    pub transpose_semitones: i8,         // -24..=24
    pub speed_percent: u16,              // 50..=200 (100 = 1x)
    pub on_beat_bpm: Option<(u16, u16)>, // (source_bpm, target_bpm)
}

impl Default for WarpSpec {
    fn default() -> Self {
        Self {
            transpose_semitones: 0,
            speed_percent: 100,
            on_beat_bpm: None,
        }
    }
}

/// Non-destructive windowed overlap-add pitch & time-stretch.
pub fn bake_warped(source: &[Frame], spec: WarpSpec) -> Result<Arc<Vec<Frame>>, Error> {
    if !(-24..=24).contains(&spec.transpose_semitones) {
        return Err(Error::Range);
    }
    if !(50..=200).contains(&spec.speed_percent) {
        return Err(Error::Range);
    }
    if let Some((src_bpm, tgt_bpm)) = spec.on_beat_bpm {
        if !(30..=240).contains(&src_bpm) || !(30..=240).contains(&tgt_bpm) {
            return Err(Error::Range);
        }
    }

    if source.is_empty() {
        let mut empty = Vec::new();
        empty.try_reserve_exact(0).map_err(|_| Error::Memory)?;
        return Ok(Arc::new(empty));
    }

    // Fast path: bypass if no warping is requested
    if spec.transpose_semitones == 0 && spec.speed_percent == 100 && spec.on_beat_bpm.is_none() {
        let mut out = Vec::new();
        out.try_reserve_exact(source.len())
            .map_err(|_| Error::Memory)?;
        out.extend_from_slice(source);
        return Ok(Arc::new(out));
    }

    let speed_mult = spec.speed_percent as f32 / 100.0;
    let bpm_mult = if let Some((src_bpm, tgt_bpm)) = spec.on_beat_bpm {
        tgt_bpm as f32 / src_bpm as f32
    } else {
        1.0
    };
    let effective_speed = speed_mult * bpm_mult;
    let pitch_ratio = exp2_f32(spec.transpose_semitones as f32 / 12.0);

    // Target length in frames
    let target_len = (floor_f32(source.len() as f32 / effective_speed) as usize).max(1);

    // Step 1: Pitch shift by resampling if pitch_ratio != 1.0
    let resampled = if (pitch_ratio - 1.0).abs() > 0.001 {
        resample_pcm(source, pitch_ratio)?
    } else {
        None
    };

    let pcm_to_stretch = match &resampled {
        Some(v) => v.as_slice(),
        None => source,
    };

    // Step 2: Time-stretch to exact target_len using WSOLA
    let result = wsola_stretch(pcm_to_stretch, target_len)?;
    Ok(Arc::new(result))
}

fn resample_pcm(source: &[Frame], ratio: f32) -> Result<Option<Vec<Frame>>, Error> {
    let out_len = (floor_f32(source.len() as f32 / ratio) as usize).max(1);
    let mut out = Vec::new();
    out.try_reserve_exact(out_len).map_err(|_| Error::Memory)?;

    for i in 0..out_len {
        let pos = i as f32 * ratio;
        let idx = floor_f32(pos) as usize;
        let frac = pos - idx as f32;

        if idx + 1 < source.len() {
            let s0 = source[idx];
            let s1 = source[idx + 1];
            let l = s0[0] as f32 * (1.0 - frac) + s1[0] as f32 * frac;
            let r = s0[1] as f32 * (1.0 - frac) + s1[1] as f32 * frac;
            out.push([l.clamp(-32768.0, 32767.0) as i16, r.clamp(-32768.0, 32767.0) as i16]);
        } else if idx < source.len() {
            out.push(source[idx]);
        } else {
            out.push([0, 0]);
        }
    }

    Ok(Some(out))
}

/// WSOLA (Waveform Similarity Overlap-Add) time-stretch.
fn wsola_stretch(source: &[Frame], target_len: usize) -> Result<Vec<Frame>, Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(target_len)
        .map_err(|_| Error::Memory)?;

    if target_len == 0 {
        return Ok(out);
    }

    const WIN_SIZE: usize = 1024;
    const HOP_SIZE: usize = 512;
    const SEARCH_RANGE: usize = 128;

    if source.len() <= WIN_SIZE {
        // Source is too short for granular decomposition; simple linear stretch
        for i in 0..target_len {
            let pos = i as f32 * (source.len() - 1) as f32 / target_len.max(1) as f32;
            let idx = floor_f32(pos) as usize;
            let frac = pos - idx as f32;
            if idx + 1 < source.len() {
                let s0 = source[idx];
                let s1 = source[idx + 1];
                let l = s0[0] as f32 * (1.0 - frac) + s1[0] as f32 * frac;
                let r = s0[1] as f32 * (1.0 - frac) + s1[1] as f32 * frac;
                out.push([l as i16, r as i16]);
            } else {
                out.push(source[idx]);
            }
        }
        return Ok(out);
    }

    // Precompute Hann window
    let mut window = [0.0f32; WIN_SIZE];
    for (i, w) in window.iter_mut().enumerate() {
        *w = 0.5 * (1.0 - cos_f32(2.0 * PI * i as f32 / WIN_SIZE as f32));
    }

    let mut accum_left = Vec::new();
    let mut accum_right = Vec::new();
    let mut weight = Vec::new();

    accum_left.try_reserve_exact(target_len + WIN_SIZE).map_err(|_| Error::Memory)?;
    accum_right.try_reserve_exact(target_len + WIN_SIZE).map_err(|_| Error::Memory)?;
    weight.try_reserve_exact(target_len + WIN_SIZE).map_err(|_| Error::Memory)?;

    accum_left.resize(target_len + WIN_SIZE, 0.0f32);
    accum_right.resize(target_len + WIN_SIZE, 0.0f32);
    weight.resize(target_len + WIN_SIZE, 0.0f32);

    let stretch_ratio = target_len as f32 / source.len() as f32;
    let mut out_pos = 0usize;

    while out_pos < target_len {
        let in_nominal = floor_f32(out_pos as f32 / stretch_ratio) as usize;

        // Search for best correlation offset in [in_nominal - SEARCH_RANGE, in_nominal + SEARCH_RANGE]
        let mut best_offset = 0i32;
        if out_pos > 0 && in_nominal >= SEARCH_RANGE && in_nominal + WIN_SIZE + SEARCH_RANGE < source.len() {
            let mut best_corr = -1e20f32;
            let test_step = 4; // Downsampled search for speed
            for delta in (- (SEARCH_RANGE as i32)..=(SEARCH_RANGE as i32)).step_by(test_step) {
                let cand_pos = (in_nominal as i32 + delta) as usize;
                let mut corr = 0.0f32;
                for k in (0..HOP_SIZE).step_by(8) {
                    let s_accum = accum_left[out_pos + k];
                    let s_cand = source[cand_pos + k][0] as f32;
                    corr += s_accum * s_cand;
                }
                if corr > best_corr {
                    best_corr = corr;
                    best_offset = delta;
                }
            }
        }

        let actual_in = (in_nominal as i32 + best_offset)
            .max(0)
            .min((source.len().saturating_sub(WIN_SIZE)) as i32) as usize;

        // Overlap-add the grain
        let copy_len = WIN_SIZE.min(source.len() - actual_in);
        for k in 0..copy_len {
            let w = window[k];
            let s = source[actual_in + k];
            accum_left[out_pos + k] += s[0] as f32 * w;
            accum_right[out_pos + k] += s[1] as f32 * w;
            weight[out_pos + k] += w;
        }

        out_pos += HOP_SIZE;
    }

    // Normalize by weight and output
    for i in 0..target_len {
        let w = weight[i];
        let norm = if w > 1e-4 { 1.0 / w } else { 1.0 };
        let l = (accum_left[i] * norm).clamp(-32768.0, 32767.0) as i16;
        let r = (accum_right[i] * norm).clamp(-32768.0, 32767.0) as i16;
        out.push([l, r]);
    }

    Ok(out)
}
