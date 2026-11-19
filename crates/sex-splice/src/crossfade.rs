// SPDX-License-Identifier: GPL-3.0-only
//! Centered audio-only join crossfades.
//!
//! Adapted from Scarlet's `joinCrossfade.ts` (MIT): smooths hard dialogue or sound cuts by
//! extending each side into its real media handles across a short window centered on the join.
//! Picture cuts and total timeline runtime never shift; joins lacking real source handles or
//! audio stay plain cuts and record why they were refused.

use crate::{Clip, MediaKind, Micros, Range, Timeline, Track, TrackKind, SPEED_NORMAL};
use alloc::vec::Vec;

/// Why a hard cut could not be given a centered audio crossfade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeRefusal {
    AlreadyFaded,
    NoAudio,
    InsufficientHandles,
    ClipTooShort,
    DialogueTrackOccupied,
}

/// A planned centered audio crossfade across an edit join.
///
/// Picture remains a hard cut at `at`; audio blends `left_clip` over `left_handle` with
/// `right_clip` over `right_handle` across `[at - overlap / 2, at + (overlap - overlap / 2)]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoinFade {
    pub at: Micros,
    pub track: u32,
    pub overlap: Micros,
    pub left_clip: u32,
    pub right_clip: u32,
    pub left_handle: Range,
    pub right_handle: Range,
}

/// Every non-continuous join inspected on audible tracks: those faded and those left plain.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JoinFadeReport {
    pub faded: Vec<JoinFade>,
    pub plain: Vec<(Micros, FadeRefusal)>,
}

/// Convert a timeline duration into the source duration needed at `speed` (rounded up so the
/// source handle always covers the full timeline half-window).
fn timeline_to_source(tl_dur: Micros, speed: u32) -> Micros {
    if tl_dur <= 0 {
        return 0;
    }
    let num = tl_dur as i128 * speed as i128 + (SPEED_NORMAL as i128 - 1);
    (num / SPEED_NORMAL as i128) as Micros
}

/// Whether `left` and `right` form a real (non-continuous) butt join at `left.end()`.
fn is_discontinuous_join(left: &Clip, right: &Clip) -> bool {
    left.end() == right.at && (left.media != right.media || left.source.end != right.source.start)
}

/// Whether an unmuted audio track already has its own discontinuous join at `join_at`.
fn audio_track_owns_join(tl: &Timeline, join_at: Micros) -> bool {
    tl.tracks.iter().filter(|t| t.kind == TrackKind::Audio && !t.muted).any(|t| {
        let enabled: Vec<&Clip> = t.clips.iter().filter(|c| c.enabled).collect();
        enabled.windows(2).any(|w| w[0].end() == join_at && is_discontinuous_join(w[0], w[1]))
    })
}

/// Whether any unmuted audio track has an enabled clip occupying `window`.
fn audio_track_occupies(tl: &Timeline, window: Range) -> bool {
    tl.tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio && !t.muted)
        .any(|t| t.clips.iter().any(|c| c.enabled && c.span().overlaps(&window)))
}

/// Plan centered audio crossfades across all eligible joins on `tl`.
pub fn plan_join_crossfades(tl: &Timeline, overlap: Micros) -> JoinFadeReport {
    plan_join_crossfades_with(tl, overlap, &[])
}

/// Plan centered audio crossfades across `tl`, refusing joins already present in `existing_fades`.
pub fn plan_join_crossfades_with(
    tl: &Timeline,
    overlap: Micros,
    existing_fades: &[JoinFade],
) -> JoinFadeReport {
    let mut report = JoinFadeReport::default();
    if overlap <= 0 {
        return report;
    }

    let left_half_tl = overlap / 2;
    let right_half_tl = overlap - left_half_tl;

    for track in &tl.tracks {
        if track.muted {
            continue;
        }
        evaluate_track_joins(
            tl,
            track,
            overlap,
            left_half_tl,
            right_half_tl,
            existing_fades,
            &mut report,
        );
    }

    report.faded.sort_by_key(|f| (f.at, f.track));
    report.plain.sort_by_key(|(at, _)| *at);
    report
}

fn evaluate_track_joins(
    tl: &Timeline,
    track: &Track,
    overlap: Micros,
    left_half_tl: Micros,
    right_half_tl: Micros,
    existing_fades: &[JoinFade],
    report: &mut JoinFadeReport,
) {
    let enabled: Vec<&Clip> = track.clips.iter().filter(|c| c.enabled).collect();
    for pair in enabled.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        if !is_discontinuous_join(left, right) {
            continue;
        }
        let join_at = left.end();
        let fade_window = Range::new(join_at - left_half_tl, join_at + right_half_tl);

        if track.kind == TrackKind::Video && audio_track_owns_join(tl, join_at) {
            // A separate unmuted audio track carries the matching cut and handles this join.
            continue;
        }

        if existing_fades
            .iter()
            .any(|f| (f.track == track.id && f.at == join_at) || (f.left_clip == left.id && f.right_clip == right.id))
        {
            report.plain.push((join_at, FadeRefusal::AlreadyFaded));
            continue;
        }

        let (Some(left_media), Some(right_media)) = (tl.media(left.media), tl.media(right.media)) else {
            report.plain.push((join_at, FadeRefusal::NoAudio));
            continue;
        };

        if left.volume == 0
            || right.volume == 0
            || left_media.kind == MediaKind::Image
            || right_media.kind == MediaKind::Image
        {
            report.plain.push((join_at, FadeRefusal::NoAudio));
            continue;
        }

        if track.kind == TrackKind::Video && audio_track_occupies(tl, fade_window) {
            report.plain.push((join_at, FadeRefusal::DialogueTrackOccupied));
            continue;
        }

        if left.duration() < overlap || right.duration() < overlap {
            report.plain.push((join_at, FadeRefusal::ClipTooShort));
            continue;
        }

        if left.speed == 0 || right.speed == 0 {
            report.plain.push((join_at, FadeRefusal::InsufficientHandles));
            continue;
        }

        let left_half_src = timeline_to_source(left_half_tl, left.speed);
        let right_half_src = timeline_to_source(right_half_tl, right.speed);

        if left.source.end + left_half_src > left_media.duration || right.source.start < right_half_src {
            report.plain.push((join_at, FadeRefusal::InsufficientHandles));
            continue;
        }

        let left_handle = Range::new(left.source.start, left.source.end + left_half_src);
        let right_handle = Range::new(right.source.start - right_half_src, right.source.end);

        report.faded.push(JoinFade {
            at: join_at,
            track: track.id,
            overlap,
            left_clip: left.id,
            right_clip: right.id,
            left_handle,
            right_handle,
        });
    }
}

#[inline]
fn blend_sample(l: i16, r: i16, w_left: i64, w_right: i64, denom: i64) -> i16 {
    let num = l as i64 * w_left + r as i64 * w_right;
    let rounded = if num >= 0 {
        (num + denom / 2) / denom
    } else {
        (num - denom / 2) / denom
    };
    rounded.clamp(i16::MIN as i64, i16::MAX as i64) as i16
}

/// Deterministic integer sample-exact linear crossfade mixer for mono `i16` PCM.
///
/// Blends `len = out.len().min(left_pcm.len()).min(right_pcm.len())` samples using outgoing
/// weight `len - 1 - i` and incoming weight `i`, with symmetric half-up rounding and `i16`
/// saturation.
pub fn mix_crossfade_i16(left_pcm: &[i16], right_pcm: &[i16], out: &mut [i16]) {
    let len = out.len().min(left_pcm.len()).min(right_pcm.len());
    if len == 0 {
        return;
    }
    if len == 1 {
        out[0] = blend_sample(left_pcm[0], right_pcm[0], 1, 1, 2);
        return;
    }
    let denom = (len - 1) as i64;
    for i in 0..len {
        let w_left = (len - 1 - i) as i64;
        let w_right = i as i64;
        out[i] = blend_sample(left_pcm[i], right_pcm[i], w_left, w_right, denom);
    }
}

/// Deterministic integer sample-exact linear crossfade mixer for interleaved stereo `[i16; 2]`.
pub fn mix_crossfade_stereo(left: &[[i16; 2]], right: &[[i16; 2]], out: &mut [[i16; 2]]) {
    let len = out.len().min(left.len()).min(right.len());
    if len == 0 {
        return;
    }
    if len == 1 {
        out[0] = [
            blend_sample(left[0][0], right[0][0], 1, 1, 2),
            blend_sample(left[0][1], right[0][1], 1, 1, 2),
        ];
        return;
    }
    let denom = (len - 1) as i64;
    for i in 0..len {
        let w_left = (len - 1 - i) as i64;
        let w_right = i as i64;
        out[i] = [
            blend_sample(left[i][0], right[i][0], w_left, w_right, denom),
            blend_sample(left[i][1], right[i][1], w_left, w_right, denom),
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Op, SECOND};
    use alloc::vec;

    const S: Micros = SECOND;
    const MS: Micros = 1_000;

    fn synced_interview() -> (Timeline, u32, u32, u32) {
        let mut tl = Timeline::new(30, 1);
        let m = tl.add_media(MediaKind::Video, "/clips/interview.mp4", 20 * S);
        let v = tl.add_track(TrackKind::Video, "Picture");
        let a = tl.add_track(TrackKind::Audio, "Dialogue");
        tl.apply(&Op::AddClip { track: v, media: m, at: 0, source: Range::new(0, 20 * S) }).unwrap();
        tl.apply(&Op::AddClip { track: a, media: m, at: 0, source: Range::new(0, 20 * S) }).unwrap();
        (tl, m, v, a)
    }

    #[test]
    fn happy_path_centered_crossfade_preserves_runtime_and_uses_real_handles() {
        let (mut tl, _, v, a) = synced_interview();
        tl.apply(&Op::RippleDelete { range: Range::new(5 * S, 9 * S) }).unwrap();
        let before_dur = tl.duration();
        let before_v = tl.tracks.iter().find(|t| t.id == v).unwrap().clips.clone();

        let overlap = 40 * MS;
        let report = plan_join_crossfades(&tl, overlap);

        assert_eq!(tl.duration(), before_dur);
        assert_eq!(tl.tracks.iter().find(|t| t.id == v).unwrap().clips, before_v);
        assert!(report.plain.is_empty());
        assert_eq!(report.faded.len(), 1);

        let fade = report.faded[0];
        let a_track = tl.tracks.iter().find(|t| t.id == a).unwrap();
        assert_eq!(fade.at, 5 * S);
        assert_eq!(fade.track, a);
        assert_eq!(fade.overlap, overlap);
        assert_eq!(fade.left_clip, a_track.clips[0].id);
        assert_eq!(fade.right_clip, a_track.clips[1].id);
        assert_eq!(fade.left_handle, Range::new(0, 5 * S + 20 * MS));
        assert_eq!(fade.right_handle, Range::new(9 * S - 20 * MS, 20 * S));
    }

    #[test]
    fn seamless_split_is_ignored_not_faded_or_refused() {
        let (mut tl, _, _, a) = synced_interview();
        let aid = tl.tracks.iter().find(|t| t.id == a).unwrap().clips[0].id;
        tl.apply(&Op::Split { clip: aid, at: 6 * S }).unwrap();

        let report = plan_join_crossfades(&tl, 40 * MS);
        assert!(report.faded.is_empty());
        assert!(report.plain.is_empty());
    }

    #[test]
    fn video_only_timeline_fades_embedded_audio_and_respects_dialogue_occupation() {
        let mut tl = Timeline::new(30, 1);
        let m1 = tl.add_media(MediaKind::Video, "/clips/a.mp4", 10 * S);
        let m2 = tl.add_media(MediaKind::Video, "/clips/b.mp4", 10 * S);
        let v = tl.add_track(TrackKind::Video, "Picture");
        tl.apply(&Op::AddClip { track: v, media: m1, at: 0, source: Range::new(0, 4 * S) }).unwrap();
        tl.apply(&Op::AddClip { track: v, media: m2, at: 4 * S, source: Range::new(2 * S, 8 * S) }).unwrap();

        // With no separate audio track, V1's join is faded directly.
        let r1 = plan_join_crossfades(&tl, 30 * MS);
        assert_eq!(r1.faded.len(), 1);
        assert_eq!(r1.faded[0].track, v);
        assert_eq!(r1.faded[0].left_handle, Range::new(0, 4 * S + 15 * MS));
        assert_eq!(r1.faded[0].right_handle, Range::new(2 * S - 15 * MS, 8 * S));

        // Add an unmuted dialogue track playing continuous speech across 4s: V1 refuses.
        let nar = tl.add_media(MediaKind::Audio, "/clips/narration.wav", 10 * S);
        let a = tl.add_track(TrackKind::Audio, "Dialogue");
        tl.apply(&Op::AddClip { track: a, media: nar, at: 0, source: Range::new(0, 8 * S) }).unwrap();

        let r2 = plan_join_crossfades(&tl, 30 * MS);
        assert!(r2.faded.is_empty());
        assert_eq!(r2.plain, vec![(4 * S, FadeRefusal::DialogueTrackOccupied)]);

        // If the dialogue track is muted, V1 is free to fade again.
        tl.tracks.iter_mut().find(|t| t.id == a).unwrap().muted = true;
        let r3 = plan_join_crossfades(&tl, 30 * MS);
        assert_eq!(r3.faded.len(), 1);
        assert!(r3.plain.is_empty());
    }

    #[test]
    fn insufficient_handles_at_head_and_tail_are_refused() {
        let mut tl = Timeline::new(30, 1);
        let m1 = tl.add_media(MediaKind::Audio, "/clips/one.wav", 5 * S);
        let m2 = tl.add_media(MediaKind::Audio, "/clips/two.wav", 5 * S);
        let a = tl.add_track(TrackKind::Audio, "A1");

        // Left clip plays all the way to media end (5s): zero outgoing handle!
        tl.apply(&Op::AddClip { track: a, media: m1, at: 0, source: Range::new(0, 5 * S) }).unwrap();
        tl.apply(&Op::AddClip { track: a, media: m2, at: 5 * S, source: Range::new(S, 4 * S) }).unwrap();
        // Third clip starts at source 0: zero incoming handle!
        tl.apply(&Op::AddClip { track: a, media: m1, at: 8 * S, source: Range::new(0, 3 * S) }).unwrap();

        let report = plan_join_crossfades(&tl, 40 * MS);
        assert!(report.faded.is_empty());
        assert_eq!(
            report.plain,
            vec![
                (5 * S, FadeRefusal::InsufficientHandles),
                (8 * S, FadeRefusal::InsufficientHandles),
            ]
        );
    }

    #[test]
    fn non_unit_speed_scales_required_source_handles() {
        let mut tl = Timeline::new(30, 1);
        let m1 = tl.add_media(MediaKind::Audio, "/clips/fast.wav", 4_030_000);
        let m2 = tl.add_media(MediaKind::Audio, "/clips/slow.wav", 10 * S);
        let a = tl.add_track(TrackKind::Audio, "A1");

        // Left clip: 0..4s at 2x speed => 2s timeline duration.
        // For overlap = 40ms (half = 20ms timeline), 2x speed needs 40ms of source handle,
        // so source end needs 4_040_000 > 4_030_000 => InsufficientHandles!
        tl.apply(&Op::AddClip { track: a, media: m1, at: 0, source: Range::new(0, 4 * S) }).unwrap();
        let left_id = tl.tracks[0].clips[0].id;
        tl.apply(&Op::SetSpeed { clip: left_id, speed: 2000 }).unwrap();

        tl.apply(&Op::AddClip { track: a, media: m2, at: 2 * S, source: Range::new(10 * MS, 4 * S + 10 * MS) }).unwrap();
        let right_id = tl.tracks[0].clips[1].id;
        // Right clip at 0.5x speed: 20ms timeline half-window only needs 10ms of source!
        tl.apply(&Op::SetSpeed { clip: right_id, speed: 500 }).unwrap();

        let r_fail = plan_join_crossfades(&tl, 40 * MS);
        assert!(r_fail.faded.is_empty());
        assert_eq!(r_fail.plain, vec![(2 * S, FadeRefusal::InsufficientHandles)]);

        // With overlap = 30ms (half = 15ms timeline):
        // Left needs 15ms * 2 = 30ms source (4s + 30ms == 4_030_000, exact boundary fit!).
        // Right needs 15ms * 0.5 = 7.5ms => 8ms rounded up (10ms >= 8ms, fits!).
        let r_ok = plan_join_crossfades(&tl, 30 * MS);
        assert!(r_ok.plain.is_empty());
        assert_eq!(r_ok.faded.len(), 1);
        assert_eq!(r_ok.faded[0].left_handle, Range::new(0, 4_030_000));
        assert_eq!(r_ok.faded[0].right_handle, Range::new(10 * MS - 7_500, 4 * S + 10 * MS));
    }

    #[test]
    fn clip_too_short_no_audio_and_already_faded_refusals() {
        let mut tl = Timeline::new(30, 1);
        let m = tl.add_media(MediaKind::Audio, "/clips/a.wav", 10 * S);
        let a = tl.add_track(TrackKind::Audio, "A1");

        // Join 1 at 30ms: left clip is 30ms (< 50ms overlap) => ClipTooShort.
        tl.apply(&Op::AddClip { track: a, media: m, at: 0, source: Range::new(S, S + 30 * MS) }).unwrap();
        // Middle clip: 30ms..2030ms (2s long).
        tl.apply(&Op::AddClip { track: a, media: m, at: 30 * MS, source: Range::new(3 * S, 5 * S) }).unwrap();
        // Join 2 at 2030ms: muted clip (volume = 0) => NoAudio.
        tl.apply(&Op::AddClip { track: a, media: m, at: 2 * S + 30 * MS, source: Range::new(6 * S, 8 * S) }).unwrap();
        let silent_id = tl.tracks[0].clips[2].id;
        tl.apply(&Op::SetVolume { clip: silent_id, volume: 0 }).unwrap();
        // Join 3 at 4030ms..6030ms after unmuting a fourth clip: test AlreadyFaded.
        tl.apply(&Op::AddClip { track: a, media: m, at: 4 * S + 30 * MS, source: Range::new(S, 3 * S) }).unwrap();
        tl.apply(&Op::AddClip { track: a, media: m, at: 6 * S + 30 * MS, source: Range::new(4 * S, 6 * S) }).unwrap();

        let first = plan_join_crossfades(&tl, 50 * MS);
        assert_eq!(first.faded.len(), 1);
        assert_eq!(first.faded[0].at, 6 * S + 30 * MS);
        assert_eq!(
            first.plain,
            vec![
                (30 * MS, FadeRefusal::ClipTooShort),
                (2 * S + 30 * MS, FadeRefusal::NoAudio),
                (4 * S + 30 * MS, FadeRefusal::NoAudio),
            ]
        );

        let second = plan_join_crossfades_with(&tl, 50 * MS, &first.faded);
        assert!(second.faded.is_empty());
        assert_eq!(second.plain[3], (6 * S + 30 * MS, FadeRefusal::AlreadyFaded));

        // Non-positive overlap plans nothing.
        assert_eq!(plan_join_crossfades(&tl, 0), JoinFadeReport::default());
        assert_eq!(plan_join_crossfades(&tl, -10), JoinFadeReport::default());
    }

    #[test]
    fn sample_exact_mono_and_stereo_crossfade_mixers() {
        let left = [10_000i16, 10_000, 10_000, 10_000, 10_000];
        let right = [-10_000i16, -10_000, -10_000, -10_000, -10_000];
        let mut out = [0i16; 5];
        mix_crossfade_i16(&left, &right, &mut out);
        assert_eq!(out, [10_000, 5_000, 0, -5_000, -10_000]);

        // Constant signal across extreme i16 bounds is bit-exact throughout.
        let max_in = [i16::MAX; 8];
        let min_in = [i16::MIN; 8];
        let mut out_max = [0i16; 8];
        let mut out_min = [0i16; 8];
        mix_crossfade_i16(&max_in, &max_in, &mut out_max);
        mix_crossfade_i16(&min_in, &min_in, &mut out_min);
        assert_eq!(out_max, max_in);
        assert_eq!(out_min, min_in);

        // Single-sample & empty edge cases, plus differing slice lengths.
        let mut single = [0i16; 3];
        mix_crossfade_i16(&[100], &[200, 300], &mut single);
        assert_eq!(single, [150, 0, 0]);
        mix_crossfade_i16(&[], &[100], &mut single);
        assert_eq!(single, [150, 0, 0]);

        // Stereo mixer blends L and R independently.
        let st_l = [[20_000, -30_000], [20_000, -30_000], [20_000, -30_000]];
        let st_r = [[0, 10_000], [0, 10_000], [0, 10_000]];
        let mut st_out = [[0, 0]; 3];
        mix_crossfade_stereo(&st_l, &st_r, &mut st_out);
        assert_eq!(st_out, [[20_000, -30_000], [10_000, -10_000], [0, 10_000]]);
    }
}
