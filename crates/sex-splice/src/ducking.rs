// SPDX-License-Identifier: GPL-3.0-only
//! Automatic background music ducking under dialogue.
//!
//! Adapted from Scarlet's `volume.ts` (`duckUnderSpeech`, `feat/music-dips-voice-cleanup`, and
//! `eval/combined-nov22`, MIT, github.com/Forward-Future/scarlet). Adjacent speech bursts across
//! normal breaths are merged first so background music holds a steady bed instead of pumping up
//! and down between clauses, then eased down before voice onset and eased back up once speech has
//! cleared.
//!
//! All times are integer [`Micros`], gains are per-mille (`1000` = unity), and smoothstep ramps
//! run in deterministic integer fixed-point arithmetic with zero floating-point drift.

use crate::{Micros, Range};
use alloc::vec::Vec;

/// Unity gain in per-mille (`0 dB`).
pub const UNITY_GAIN_PER_MILLE: u32 = 1000;

/// Fixed-point fractional shift used for sub-per-mille sample-accurate gain evaluation.
const GAIN_FRAC_BITS: u32 = 20;

/// Interpolation curve from one [`VolumeKeyframe`] to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuckEase {
    Linear,
    SmoothStep,
}

/// Envelope parameters controlling how background music dips beneath spoken dialogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuckShape {
    /// Target ducked gain in per-mille (`200` is `-14 dB`).
    pub depth_per_mille: u32,
    /// Ramp-down duration before padded speech begins.
    pub down_us: Micros,
    /// Ramp-up recovery duration after padded speech ends.
    pub up_us: Micros,
    /// Maximum silence between speech spans merged into one continuous hold so music never pumps.
    pub merge_gap_us: Micros,
    /// Pre-speech hold lead so music is already at full dip before the first consonant.
    pub lead_us: Micros,
    /// Post-speech hold tail so word decay finishes before music begins rising.
    pub tail_us: Micros,
}

impl Default for DuckShape {
    fn default() -> Self {
        DuckShape {
            depth_per_mille: 200,
            down_us: 250_000,
            up_us: 600_000,
            merge_gap_us: 800_000,
            lead_us: 80_000,
            tail_us: 120_000,
        }
    }
}

/// One clip-local gain control point on a music clip (`0..=music_span.len()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeKeyframe {
    pub local_us: Micros,
    pub gain_per_mille: u32,
    pub ease: DuckEase,
}

/// Merge adjacent speech spans separated by at most `merge_gap_us`, then pad each merged span by
/// `lead_us` before and `tail_us` after (clamping non-negative starts at `0` and coalescing any
/// spans whose padding meets).
pub fn merge_speech_spans(
    spans: &[Range],
    merge_gap_us: Micros,
    lead_us: Micros,
    tail_us: Micros,
) -> Vec<Range> {
    let mut raw: Vec<Range> = spans.iter().copied().filter(|r| !r.is_empty()).collect();
    if raw.is_empty() {
        return Vec::new();
    }
    raw.sort_by_key(|r| (r.start, r.end));

    let gap_limit = merge_gap_us.max(0);
    let mut merged_raw: Vec<Range> = Vec::with_capacity(raw.len());
    for r in raw {
        match merged_raw.last_mut() {
            Some(last) if r.start.saturating_sub(last.end) <= gap_limit => {
                last.end = last.end.max(r.end);
            }
            _ => merged_raw.push(r),
        }
    }

    let lead = lead_us.max(0);
    let tail = tail_us.max(0);
    let mut padded: Vec<Range> = Vec::with_capacity(merged_raw.len());
    for r in merged_raw {
        let start = if r.start >= 0 {
            r.start.saturating_sub(lead).max(0)
        } else {
            r.start.saturating_sub(lead)
        };
        let end = r.end.saturating_add(tail);
        match padded.last_mut() {
            Some(last) if start <= last.end => {
                last.end = last.end.max(end);
            }
            _ => padded.push(Range::new(start, end)),
        }
    }
    padded
}

/// Plan sorted clip-local [`VolumeKeyframe`]s for a music clip occupying timeline `music_span`
/// beneath timeline `speech_spans`.
///
/// Each merged speech window ramps `1000‰ -> depth_per_mille` over `down_us`, holds
/// `depth_per_mille` across the speech window, and eases `depth_per_mille -> 1000‰` over `up_us`.
/// Speech starting before or near `music_span.start` (including after J/L split-edit shifts) or
/// extending near/past `music_span.end` clamps cleanly to `0..=music_span.len()`, and adjacent
/// dips whose recovery and attack ramps meet intersect at their exact proportional peak without
/// unsorted keyframes.
pub fn plan_music_ducking(
    music_span: Range,
    speech_spans: &[Range],
    shape: DuckShape,
) -> Vec<VolumeKeyframe> {
    if music_span.is_empty() || speech_spans.is_empty() {
        return Vec::new();
    }

    let clip_len = music_span.len();
    let merged = merge_speech_spans(
        speech_spans,
        shape.merge_gap_us,
        shape.lead_us,
        shape.tail_us,
    );

    // Convert overlapping padded speech spans into clip-local coordinates.
    let local_dips: Vec<(Micros, Micros)> = merged
        .into_iter()
        .map(|r| (r.start - music_span.start, r.end - music_span.start))
        .filter(|&(s, e)| e > 0 && s < clip_len)
        .collect();

    if local_dips.is_empty() {
        return Vec::new();
    }

    let down_us = shape.down_us.max(0);
    let up_us = shape.up_us.max(0);
    let depth = shape.depth_per_mille;
    let mut out: Vec<VolumeKeyframe> = Vec::with_capacity(local_dips.len() * 4);

    let push_kf = |out: &mut Vec<VolumeKeyframe>, local_us: Micros, gain_per_mille: u32| {
        if let Some(last) = out.last_mut() {
            if last.local_us == local_us && last.gain_per_mille == gain_per_mille {
                return;
            }
        }
        out.push(VolumeKeyframe {
            local_us,
            gain_per_mille,
            ease: DuckEase::SmoothStep,
        });
    };

    for (idx, &(s, e)) in local_dips.iter().enumerate() {
        let hold_start = s.clamp(0, clip_len);
        let hold_end = e.clamp(0, clip_len);

        if idx == 0 {
            if s > 0 {
                let down_start = (s - down_us).max(0);
                push_kf(&mut out, down_start, UNITY_GAIN_PER_MILLE);
            }
            push_kf(&mut out, hold_start, depth);
        } else {
            let prev_end = local_dips[idx - 1].1.clamp(0, clip_len);
            let gap = hold_start.saturating_sub(prev_end);
            let full_cycle = up_us.saturating_add(down_us);

            if gap == 0 || full_cycle == 0 {
                push_kf(&mut out, hold_start, depth);
            } else if gap >= full_cycle {
                let up_end = (prev_end + up_us).min(clip_len);
                let down_start = (hold_start - down_us).max(0);
                push_kf(&mut out, up_end, UNITY_GAIN_PER_MILLE);
                push_kf(&mut out, down_start, UNITY_GAIN_PER_MILLE);
                push_kf(&mut out, hold_start, depth);
            } else {
                // Recovery and attack ramps cross before reaching 1000‰: meet at their exact
                // slope intersection so timestamps stay strictly sorted without pumping.
                let t_peak = prev_end
                    + ((gap as i128 * up_us as i128) / full_cycle as i128) as Micros;
                let g_peak = if UNITY_GAIN_PER_MILLE >= depth {
                    let rise = UNITY_GAIN_PER_MILLE - depth;
                    depth
                        + ((rise as u128 * gap as u128 + (full_cycle as u128 / 2))
                            / full_cycle as u128) as u32
                } else {
                    let drop = depth - UNITY_GAIN_PER_MILLE;
                    depth
                        - ((drop as u128 * gap as u128 + (full_cycle as u128 / 2))
                            / full_cycle as u128) as u32
                };
                if t_peak > prev_end && t_peak < hold_start {
                    push_kf(&mut out, t_peak, g_peak);
                }
                push_kf(&mut out, hold_start, depth);
            }
        }

        if hold_end > hold_start {
            push_kf(&mut out, hold_end, depth);
        }

        if idx + 1 == local_dips.len() && e < clip_len {
            let up_end = (e + up_us).min(clip_len);
            push_kf(&mut out, up_end, UNITY_GAIN_PER_MILLE);
        }
    }

    out
}

/// Evaluate effective gain in Q20 fixed-point per-mille (`1000 << 20` = unity) at `local_us`.
fn evaluate_gain_q20(keyframes: &[VolumeKeyframe], base_volume: u32, local_us: Micros) -> u64 {
    if base_volume == 0 {
        return 0;
    }
    if keyframes.is_empty() {
        return (base_volume as u64) << GAIN_FRAC_BITS;
    }

    let one_q32: u128 = 1u128 << 32;
    let idx = keyframes.partition_point(|k| k.local_us <= local_us);

    let env_q32: u128 = if idx == 0 {
        (keyframes[0].gain_per_mille as u128) << 32
    } else if idx >= keyframes.len() {
        (keyframes[keyframes.len() - 1].gain_per_mille as u128) << 32
    } else {
        let k0 = keyframes[idx - 1];
        let k1 = keyframes[idx];
        let span = (k1.local_us - k0.local_us) as u128;
        let dt = (local_us - k0.local_us) as u128;

        let t_q32 = ((dt << 32) + span / 2) / span;
        let weight_q32 = match k0.ease {
            DuckEase::Linear => t_q32,
            DuckEase::SmoothStep => {
                // Exact 3t^2 - 2t^3 in Q32 fixed-point.
                let t2 = (t_q32 * t_q32 + (one_q32 >> 1)) >> 32;
                let t3 = (t2 * t_q32 + (one_q32 >> 1)) >> 32;
                (3 * t2).saturating_sub(2 * t3).min(one_q32)
            }
        };

        let g0 = k0.gain_per_mille as i128;
        let g1 = k1.gain_per_mille as i128;
        let interp = g0 * (one_q32 as i128) + (g1 - g0) * (weight_q32 as i128);
        interp.max(0) as u128
    };

    // Scale envelope gain (per-mille in Q32) by base_volume / 1000 and shift down to Q20.
    let num = env_q32 * (base_volume as u128);
    let denom = 1000u128 << (32 - GAIN_FRAC_BITS);
    ((num + denom / 2) / denom) as u64
}

/// Evaluate effective clip gain in per-mille at clip-local timestamp `local_us`, combining
/// `base_volume` with exact integer linear or cubic Hermite smoothstep (`3t^2 - 2t^3`) keyframe
/// interpolation.
pub fn evaluate_gain_per_mille(
    keyframes: &[VolumeKeyframe],
    base_volume: u32,
    local_us: Micros,
) -> u32 {
    let q20 = evaluate_gain_q20(keyframes, base_volume, local_us);
    let half = 1u64 << (GAIN_FRAC_BITS - 1);
    ((q20 + half) >> GAIN_FRAC_BITS) as u32
}

/// Apply smooth sample-accurate stereo ducking in-place starting at clip-local `start_local_us`,
/// using sub-per-mille fixed-point gain evaluation with symmetric rounding and `i16` saturation.
pub fn apply_ducking_stereo_i16(
    pcm: &mut [[i16; 2]],
    sample_rate: u32,
    start_local_us: Micros,
    base_volume: u32,
    keyframes: &[VolumeKeyframe],
) {
    if pcm.is_empty() || sample_rate == 0 {
        return;
    }

    let unity_q20 = (UNITY_GAIN_PER_MILLE as u64) << GAIN_FRAC_BITS;
    if keyframes.is_empty() && base_volume == UNITY_GAIN_PER_MILLE {
        return;
    }

    let denom = unity_q20 as i128;
    let rate = sample_rate as i128;

    for (i, frame) in pcm.iter_mut().enumerate() {
        let local_us = start_local_us + ((i as i128 * 1_000_000) / rate) as Micros;
        let g_q20 = evaluate_gain_q20(keyframes, base_volume, local_us) as i128;

        if g_q20 == denom {
            continue;
        }
        if g_q20 == 0 {
            *frame = [0, 0];
            continue;
        }

        for ch in frame.iter_mut() {
            let num = (*ch as i128) * g_q20;
            let scaled = if num >= 0 {
                (num + denom / 2) / denom
            } else {
                (num - denom / 2) / denom
            };
            *ch = scaled.clamp(i16::MIN as i128, i16::MAX as i128) as i16;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SECOND;
    use alloc::vec;

    const S: Micros = SECOND;
    const MS: Micros = 1_000;

    #[test]
    fn default_duck_shape_matches_anti_pumping_parameters() {
        let shape = DuckShape::default();
        assert_eq!(shape.depth_per_mille, 200);
        assert_eq!(shape.down_us, 250_000);
        assert_eq!(shape.up_us, 600_000);
        assert_eq!(shape.merge_gap_us, 800_000);
        assert_eq!(shape.lead_us, 80_000);
        assert_eq!(shape.tail_us, 120_000);
    }

    #[test]
    fn merge_speech_spans_merges_across_breaths_and_pads_edges() {
        // Unsorted spans with a 750 ms breath (< 800 ms merge_gap_us) and a 2 s pause (> 800 ms).
        let spans = [
            Range::new(2_750 * MS, 4_000 * MS),
            Range::new(1_000 * MS, 2_000 * MS),
            Range::new(3_000 * MS, 3_000 * MS), // empty span ignored
            Range::new(6_000 * MS, 7_000 * MS),
        ];
        let merged = merge_speech_spans(&spans, 800 * MS, 80 * MS, 120 * MS);
        assert_eq!(
            merged,
            vec![
                Range::new(920 * MS, 4_120 * MS),
                Range::new(5_920 * MS, 7_120 * MS),
            ]
        );

        // Near zero start clamps cleanly to 0 rather than going negative.
        let near_zero = merge_speech_spans(&[Range::new(30 * MS, 500 * MS)], 800 * MS, 80 * MS, 120 * MS);
        assert_eq!(near_zero, vec![Range::new(0, 620 * MS)]);
    }

    #[test]
    fn plan_music_ducking_builds_sorted_clip_local_envelope_without_pumping() {
        // Music clip sits at 2s..14s on timeline; two speech bursts at 4s..5s and 5.6s..7s merge.
        let music = Range::new(2 * S, 14 * S);
        let speech = [
            Range::new(4 * S, 5 * S),
            Range::new(5_600 * MS, 7 * S),
        ];
        let shape = DuckShape::default();
        let kfs = plan_music_ducking(music, &speech, shape);

        // Merged speech in timeline: (4.0s - 80ms)..(7.0s + 120ms) = 3.92s..7.12s.
        // Clip-local (music starts at 2.0s): hold 1.92s..5.12s, down from 1.67s, up to 5.72s.
        assert_eq!(
            kfs,
            vec![
                VolumeKeyframe { local_us: 1_670 * MS, gain_per_mille: 1000, ease: DuckEase::SmoothStep },
                VolumeKeyframe { local_us: 1_920 * MS, gain_per_mille: 200, ease: DuckEase::SmoothStep },
                VolumeKeyframe { local_us: 5_120 * MS, gain_per_mille: 200, ease: DuckEase::SmoothStep },
                VolumeKeyframe { local_us: 5_720 * MS, gain_per_mille: 1000, ease: DuckEase::SmoothStep },
            ]
        );
    }

    #[test]
    fn plan_music_ducking_clamps_near_clip_boundaries_and_split_edit_shifts() {
        let shape = DuckShape::default();

        // 1. Speech already underway before music clip starts (e.g. J-cut or music enters mid-line)
        //    and continues past music clip end (e.g. L-cut): stays held at depth throughout.
        let music = Range::new(4 * S, 8 * S);
        let covering = [Range::new(3 * S, 9 * S)];
        let kfs = plan_music_ducking(music, &covering, shape);
        assert_eq!(
            kfs,
            vec![
                VolumeKeyframe { local_us: 0, gain_per_mille: 200, ease: DuckEase::SmoothStep },
                VolumeKeyframe { local_us: 4 * S, gain_per_mille: 200, ease: DuckEase::SmoothStep },
            ]
        );
        assert_eq!(evaluate_gain_per_mille(&kfs, 1000, 0), 200);
        assert_eq!(evaluate_gain_per_mille(&kfs, 1000, 2 * S), 200);

        // 2. Speech starts 200ms into clip (< down_us + lead_us) and ends 200ms before clip end:
        //    ramps clamp cleanly to [0, clip_len] with strictly monotonic local_us.
        let near_edges = [Range::new(4_200 * MS, 7_800 * MS)];
        let kfs2 = plan_music_ducking(music, &near_edges, shape);
        assert!(kfs2.windows(2).all(|w| w[0].local_us < w[1].local_us), "{kfs2:?}");
        assert_eq!(kfs2.first().unwrap().local_us, 0);
        assert_eq!(kfs2.last().unwrap().local_us, 4 * S);

        // 3. Intermediate pause just over merge_gap_us (900ms raw => 700ms padded < 850ms up+down):
        //    meets at proportional peak without unsorted keyframes!
        let medium_pause = [
            Range::new(5 * S, 5_500 * MS),
            Range::new(6_400 * MS, 7 * S),
        ];
        let kfs3 = plan_music_ducking(Range::new(4 * S, 10 * S), &medium_pause, shape);
        assert!(kfs3.windows(2).all(|w| w[0].local_us < w[1].local_us), "{kfs3:?}");
        let peak = kfs3.iter().find(|k| k.gain_per_mille > 200 && k.gain_per_mille < 1000).unwrap();
        assert!(peak.gain_per_mille > 200 && peak.gain_per_mille < 1000);
    }

    #[test]
    fn evaluate_gain_linear_and_smoothstep_exact_points() {
        let linear = [
            VolumeKeyframe { local_us: 0, gain_per_mille: 1000, ease: DuckEase::Linear },
            VolumeKeyframe { local_us: 1_000, gain_per_mille: 200, ease: DuckEase::Linear },
        ];
        let smooth = [
            VolumeKeyframe { local_us: 0, gain_per_mille: 1000, ease: DuckEase::SmoothStep },
            VolumeKeyframe { local_us: 1_000, gain_per_mille: 200, ease: DuckEase::SmoothStep },
        ];

        // Endpoints and midpoint match identically on both curves.
        assert_eq!(evaluate_gain_per_mille(&linear, 1000, -500), 1000);
        assert_eq!(evaluate_gain_per_mille(&linear, 1000, 0), 1000);
        assert_eq!(evaluate_gain_per_mille(&linear, 1000, 500), 600);
        assert_eq!(evaluate_gain_per_mille(&linear, 1000, 1_000), 200);
        assert_eq!(evaluate_gain_per_mille(&linear, 1000, 2_000), 200);

        assert_eq!(evaluate_gain_per_mille(&smooth, 1000, 0), 1000);
        assert_eq!(evaluate_gain_per_mille(&smooth, 1000, 500), 600);
        assert_eq!(evaluate_gain_per_mille(&smooth, 1000, 1_000), 200);

        // At t = 0.25: linear drops 25% (1000 - 200 = 800), whereas smoothstep eases gently
        // (3*(1/16) - 2*(1/64) = 5/32 = 15.625% => 1000 - 800 * 5/32 = 875 exact!).
        assert_eq!(evaluate_gain_per_mille(&linear, 1000, 250), 800);
        assert_eq!(evaluate_gain_per_mille(&smooth, 1000, 250), 875);
        // At t = 0.75: smoothstep is symmetric (1000 - 800 * 27/32 = 325 exact!).
        assert_eq!(evaluate_gain_per_mille(&smooth, 1000, 750), 325);

        // Scales proportionally with non-unity base_volume.
        assert_eq!(evaluate_gain_per_mille(&smooth, 500, 500), 300);
        assert_eq!(evaluate_gain_per_mille(&[], 750, 123_456), 750);
    }

    #[test]
    fn apply_ducking_stereo_i16_is_smooth_click_free_and_overflow_safe() {
        let rate = 48_000u32;
        let music = Range::new(0, 2 * S);
        let speech = [Range::new(500 * MS, 1_000 * MS)];
        let kfs = plan_music_ducking(music, &speech, DuckShape::default());

        // 1 second of constant full-scale DC stereo signal ([10_000, -10_000]) to verify ramp continuity.
        let mut pcm = vec![[10_000i16, -10_000i16]; rate as usize];
        apply_ducking_stereo_i16(&mut pcm, rate, 0, 1000, &kfs);

        // Before ramp-down (at 0ms): untouched unity gain.
        assert_eq!(pcm[0], [10_000, -10_000]);
        // Inside held speech window (at 600ms => sample 28_800): exact 200‰ ([2_000, -2_000]).
        let mid_idx = (rate as usize * 600) / 1000;
        assert_eq!(pcm[mid_idx], [2_000, -2_000]);

        // Every adjacent sample across the entire 250ms attack ramp moves by <= 2 LSBs (no clicks!).
        for pair in pcm.windows(2) {
            assert!((pair[1][0] as i32 - pair[0][0] as i32).abs() <= 2);
            assert!((pair[1][1] as i32 - pair[0][1] as i32).abs() <= 2);
        }

        // Extreme i16::MIN / i16::MAX with boosted base_volume = 1500 saturates cleanly without overflow.
        let mut extremes = [[i16::MAX, i16::MIN]; 4];
        apply_ducking_stereo_i16(&mut extremes, rate, 0, 1500, &[]);
        assert_eq!(extremes[0], [i16::MAX, i16::MIN]);
    }

    #[test]
    fn ducking_tracks_dialogue_across_split_edit_j_cut() {
        use crate::split_edit::split_edit_ops;
        use crate::{MediaKind, Op, Timeline, TrackKind};

        // Shot A (silent B-roll) 0..4s followed by Shot B (dialogue) 4..8s, over music bed 3.8..8s.
        let mut tl = Timeline::new(30, 1);
        let a = tl.add_media(MediaKind::Video, "/clips/broll.mp4", 10 * S);
        let b = tl.add_media(MediaKind::Video, "/clips/host.mp4", 10 * S);
        let bgm = tl.add_media(MediaKind::Audio, "/clips/music.wav", 20 * S);
        let v = tl.add_track(TrackKind::Video, "V1");
        let a1 = tl.add_track(TrackKind::Audio, "Dialogue");
        let a2 = tl.add_track(TrackKind::Audio, "Music");

        tl.apply(&Op::AddClip { track: v, media: a, at: 0, source: Range::new(0, 4 * S) }).unwrap();
        tl.apply(&Op::AddClip { track: v, media: b, at: 4 * S, source: Range::new(2 * S, 6 * S) }).unwrap();
        tl.apply(&Op::AddClip { track: a2, media: bgm, at: 3_800 * MS, source: Range::new(0, 4_200 * MS) }).unwrap();

        // J-cut at 4s brings Shot B's dialogue audio in 500ms early (at 3.5s, before music starts at 3.8s!).
        let edited = tl.apply_all(&split_edit_ops(&tl, 4 * S, -500 * MS).unwrap()).unwrap();
        let host_audio = edited
            .tracks
            .iter()
            .find(|t| t.id == a1)
            .unwrap()
            .clips
            .iter()
            .find(|c| c.media == b)
            .unwrap()
            .span();
        assert_eq!(host_audio, Range::new(3_500 * MS, 8 * S));

        let music_clip = edited.tracks.iter().find(|t| t.id == a2).unwrap().clips[0].span();
        let kfs = plan_music_ducking(music_clip, &[host_audio], DuckShape::default());
        assert_eq!(kfs[0].local_us, 0);
        assert_eq!(kfs[0].gain_per_mille, 200);
    }
}
