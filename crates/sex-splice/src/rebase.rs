// SPDX-License-Identifier: GPL-3.0-only
//! Source-anchored time mapping, left-out join audition, partial batch apply, and concurrent
//! edit rebasing.
//!
//! Adapted from Scarlet's `rebase.ts` and `leftOut.ts` (MIT): keeps assistant proposals anchored
//! to media source ranges rather than brittle timeline timestamps so non-overlapping human edits
//! rebase cleanly while overlapping edits refuse with a clear [`RebaseConflict`].

use crate::transcript::{ripple_ops, SourceCut};
use crate::{EditError, MediaKind, Micros, Range, Timeline, TrackKind};
use alloc::vec::Vec;

/// Sort and merge overlapping or contiguous half-open ranges.
fn merge_ranges(mut spans: Vec<Range>) -> Vec<Range> {
    spans.retain(|r| !r.is_empty());
    spans.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range> = Vec::with_capacity(spans.len());
    for r in spans {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// Set difference `base \ sub` for sorted, merged range slices.
fn subtract_ranges(base: &[Range], sub: &[Range]) -> Vec<Range> {
    let mut out = Vec::new();
    for b in base {
        let mut cur = b.start;
        for s in sub {
            if s.end <= cur {
                continue;
            }
            if s.start >= b.end {
                break;
            }
            if s.start > cur {
                out.push(Range::new(cur, s.start));
            }
            cur = cur.max(s.end);
            if cur >= b.end {
                break;
            }
        }
        if cur < b.end {
            out.push(Range::new(cur, b.end));
        }
    }
    out
}

/// All enabled timeline spans where `src` of `media` currently plays, merged when contiguous.
pub fn source_to_timeline_spans(tl: &Timeline, media: u32, src: Range) -> Vec<Range> {
    if src.is_empty() {
        return Vec::new();
    }
    let mut spans = Vec::new();
    for track in &tl.tracks {
        if track.muted && track.kind == TrackKind::Audio {
            continue;
        }
        for c in track.clips.iter().filter(|c| c.enabled && c.media == media) {
            let s = Range::new(src.start.max(c.source.start), src.end.min(c.source.end));
            if s.is_empty() {
                continue;
            }
            let (Some(a), Some(b)) = (c.timeline_at(s.start), c.timeline_at(s.end)) else {
                continue;
            };
            if b > a {
                spans.push(Range::new(a, b));
            }
        }
    }
    merge_ranges(spans)
}

fn collect_primary_sources(tl: &Timeline, media: u32, use_video: bool) -> Vec<Range> {
    let mut spans = Vec::new();
    for track in &tl.tracks {
        let include = if use_video {
            track.kind == TrackKind::Video
        } else {
            track.kind == TrackKind::Audio && !track.muted
        };
        if !include {
            continue;
        }
        for c in track.clips.iter().filter(|c| c.enabled && c.media == media) {
            spans.push(c.source);
        }
    }
    merge_ranges(spans)
}

/// Exact source ranges present in `original` (or `0..media.duration` when `original` holds no
/// clips of `media`) that no longer play anywhere on `edited`'s video/primary tracks.
pub fn left_out_ranges(original: &Timeline, edited: &Timeline, media: u32) -> Vec<Range> {
    let media_info = original.media(media).or_else(|| edited.media(media));
    let media_kind = media_info.map(|m| m.kind);

    let orig_has_video_media = original
        .tracks
        .iter()
        .any(|t| t.kind == TrackKind::Video && t.clips.iter().any(|c| c.media == media));
    let edit_has_video_media = edited
        .tracks
        .iter()
        .any(|t| t.kind == TrackKind::Video && t.clips.iter().any(|c| c.media == media));
    let orig_has_any_media = original.tracks.iter().any(|t| t.clips.iter().any(|c| c.media == media));
    let edit_has_video_track = edited.tracks.iter().any(|t| t.kind == TrackKind::Video);

    let use_video = media_kind != Some(MediaKind::Audio)
        && (orig_has_video_media
            || edit_has_video_media
            || (!orig_has_any_media && edit_has_video_track));

    let base_ranges = if orig_has_any_media {
        let orig_use_video = media_kind != Some(MediaKind::Audio) && orig_has_video_media;
        collect_primary_sources(original, media, orig_use_video)
    } else {
        match media_info {
            Some(m) if m.duration > 0 => alloc::vec![Range::new(0, m.duration)],
            _ => return Vec::new(),
        }
    };

    let played_ranges = collect_primary_sources(edited, media, use_video);
    subtract_ranges(&base_ranges, &played_ranges)
}

/// The source span cut out between two clips meeting at a single timeline join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoinRemoved {
    pub join_at: Micros,
    pub media: u32,
    pub removed_source: Range,
}

/// When two enabled clips of the same `media` meet at `join_at` with `left.source.end <
/// right.source.start`, returns the removed source range so Splicy's `\` key can toggle between
/// hearing the join and auditioning what was cut.
pub fn removed_at_join(tl: &Timeline, join_at: Micros) -> Option<JoinRemoved> {
    // Prefer picture tracks first, then unmuted audio tracks.
    for want_kind in [TrackKind::Video, TrackKind::Audio] {
        for track in &tl.tracks {
            if track.kind != want_kind || (track.kind == TrackKind::Audio && track.muted) {
                continue;
            }
            let enabled: Vec<_> = track.clips.iter().filter(|c| c.enabled).collect();
            for pair in enabled.windows(2) {
                let (left, right) = (pair[0], pair[1]);
                if left.end() == join_at
                    && right.at == join_at
                    && left.media == right.media
                    && left.source.end < right.source.start
                {
                    return Some(JoinRemoved {
                        join_at,
                        media: left.media,
                        removed_source: Range::new(left.source.end, right.source.start),
                    });
                }
            }
        }
    }
    None
}

/// Apply only the cuts `i` where `keep_mask.get(i) == Some(&true)`.
pub fn apply_cut_subset(
    tl: &Timeline,
    cuts: &[SourceCut],
    keep_mask: &[bool],
) -> Result<Timeline, EditError> {
    let mut selected = Vec::new();
    for (i, cut) in cuts.iter().enumerate() {
        if keep_mask.get(i) != Some(&true) {
            continue;
        }
        let m = tl.media(cut.media).ok_or(EditError::NoMedia(cut.media))?;
        if cut.range.is_empty()
            || cut.range.start < 0
            || (m.kind != MediaKind::Image && cut.range.end > m.duration)
        {
            return Err(EditError::BadSource);
        }
        selected.push(*cut);
    }
    tl.apply_all(&ripple_ops(tl, &selected))
}

/// Why a proposed [`SourceCut`] computed against `base` could not be rebased onto `current`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebaseConflict {
    pub media: u32,
    pub cut: Range,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CoverageSlice {
    track: u32,
    source: Range,
    speed: u32,
    rel_timeline: Micros,
}

fn source_coverage(tl: &Timeline, media: u32, range: Range) -> Vec<CoverageSlice> {
    let mut raw: Vec<(u32, Range, u32, Micros)> = Vec::new();
    for track in &tl.tracks {
        if track.muted && track.kind == TrackKind::Audio {
            continue;
        }
        for c in track.clips.iter().filter(|c| c.enabled && c.media == media) {
            let sub = Range::new(range.start.max(c.source.start), range.end.min(c.source.end));
            if sub.is_empty() {
                continue;
            }
            if let Some(tl_at) = c.timeline_at(sub.start) {
                raw.push((track.id, sub, c.speed, tl_at));
            }
        }
    }
    let anchor = raw.first().map(|x| x.3).unwrap_or(0);
    raw.into_iter()
        .map(|(track, source, speed, tl_at)| CoverageSlice {
            track,
            source,
            speed,
            rel_timeline: tl_at - anchor,
        })
        .collect()
}

/// Rebase an assistant's proposed `cuts` (computed against `base`) onto `current`.
///
/// If the editor modified, removed, split, slipped, or speed-changed any source span overlapping
/// a proposed cut between `base` and `current`, refuses cleanly with [`RebaseConflict`]. If the
/// editor only changed other parts of the timeline (even earlier ripple deletes that shifted all
/// downstream timeline timestamps), maps `cuts` onto `current` and returns the rebased timeline.
pub fn rebase_cuts(
    base: &Timeline,
    current: &Timeline,
    cuts: &[SourceCut],
) -> Result<Timeline, RebaseConflict> {
    if cuts.is_empty() {
        return Ok(current.clone());
    }

    for cut in cuts {
        if cut.range.is_empty() {
            return Err(RebaseConflict {
                media: cut.media,
                cut: cut.range,
                reason: "cut source range is empty",
            });
        }

        let base_cov = source_coverage(base, cut.media, cut.range);
        if base_cov.is_empty() {
            return Err(RebaseConflict {
                media: cut.media,
                cut: cut.range,
                reason: "cut source range not present in base timeline",
            });
        }

        let cur_cov = source_coverage(current, cut.media, cut.range);
        if cur_cov.is_empty() {
            return Err(RebaseConflict {
                media: cut.media,
                cut: cut.range,
                reason: "source region removed in current timeline",
            });
        }

        let same_spans = base_cov.len() == cur_cov.len()
            && base_cov
                .iter()
                .zip(cur_cov.iter())
                .all(|(b, c)| b.track == c.track && b.source == c.source);

        if !same_spans {
            return Err(RebaseConflict {
                media: cut.media,
                cut: cut.range,
                reason: "source region modified or trimmed in current timeline",
            });
        }

        if base_cov.iter().zip(cur_cov.iter()).any(|(b, c)| b.speed != c.speed) {
            return Err(RebaseConflict {
                media: cut.media,
                cut: cut.range,
                reason: "source speed changed in current timeline",
            });
        }

        if base_cov
            .iter()
            .zip(cur_cov.iter())
            .any(|(b, c)| b.rel_timeline != c.rel_timeline)
        {
            return Err(RebaseConflict {
                media: cut.media,
                cut: cut.range,
                reason: "track sync or internal timing modified across cut range",
            });
        }
    }

    let ops = ripple_ops(current, cuts);
    current.apply_all(&ops).map_err(|_| RebaseConflict {
        media: cuts[0].media,
        cut: cuts[0].range,
        reason: "failed to apply rebased ripple ops",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Op, SECOND};
    use alloc::vec;

    const S: Micros = SECOND;

    fn interview() -> (Timeline, u32, u32, u32) {
        let mut tl = Timeline::new(30, 1);
        let m = tl.add_media(MediaKind::Video, "/clips/interview.mp4", 20 * S);
        let v = tl.add_track(TrackKind::Video, "Picture");
        let a = tl.add_track(TrackKind::Audio, "Dialogue");
        tl.apply(&Op::AddClip { track: v, media: m, at: 0, source: Range::new(0, 20 * S) }).unwrap();
        tl.apply(&Op::AddClip { track: a, media: m, at: 0, source: Range::new(0, 20 * S) }).unwrap();
        (tl, m, v, a)
    }

    #[test]
    fn source_to_timeline_spans_merges_contiguous_and_handles_speed_and_gaps() {
        let (mut tl, m, v, _) = interview();
        let cid = tl.tracks[0].clips[0].id;
        // Seamless split at 6s: 2s..10s should still merge into a single [2s..10s] span.
        tl.apply(&Op::Split { clip: cid, at: 6 * S }).unwrap();
        assert_eq!(
            source_to_timeline_spans(&tl, m, Range::new(2 * S, 10 * S)),
            vec![Range::new(2 * S, 10 * S)]
        );

        // Move the second picture clip 3s later on V1 and mute A1 so there is a real gap.
        tl.tracks[1].muted = true;
        let second = tl.tracks.iter().find(|t| t.id == v).unwrap().clips[1].id;
        tl.apply(&Op::Move { clip: second, at: 9 * S }).unwrap();
        // Speed up the second clip (6s..20s source) to 2x => plays at 9s..16s timeline.
        tl.apply(&Op::SetSpeed { clip: second, speed: 2000 }).unwrap();

        // Source 4s..10s: 4s..6s plays at 4s..6s; 6s..10s at 2x plays at 9s..11s!
        assert_eq!(
            source_to_timeline_spans(&tl, m, Range::new(4 * S, 10 * S)),
            vec![Range::new(4 * S, 6 * S), Range::new(9 * S, 11 * S)]
        );
        assert!(source_to_timeline_spans(&tl, m, Range::new(5 * S, 5 * S)).is_empty());
    }

    #[test]
    fn left_out_ranges_tracks_removed_spans_and_empty_original_fallback() {
        let (orig, m, _, _) = interview();
        let cuts = [
            SourceCut { media: m, range: Range::new(2 * S, 5 * S) },
            SourceCut { media: m, range: Range::new(11 * S, 14 * S) },
            SourceCut { media: m, range: Range::new(18 * S, 20 * S) },
        ];
        let edited = orig.apply_all(&ripple_ops(&orig, &cuts)).unwrap();

        assert_eq!(
            left_out_ranges(&orig, &edited, m),
            vec![
                Range::new(2 * S, 5 * S),
                Range::new(11 * S, 14 * S),
                Range::new(18 * S, 20 * S),
            ]
        );

        // Passing an empty original timeline falls back to 0..media.duration.
        let empty = Timeline::new(30, 1);
        assert_eq!(
            left_out_ranges(&empty, &edited, m),
            vec![
                Range::new(2 * S, 5 * S),
                Range::new(11 * S, 14 * S),
                Range::new(18 * S, 20 * S),
            ]
        );
    }

    #[test]
    fn removed_at_join_auditions_exact_cut_span_and_ignores_splits_or_different_media() {
        let (tl, m, v, _) = interview();
        let cuts = [
            SourceCut { media: m, range: Range::new(3 * S, 7 * S) },
            SourceCut { media: m, range: Range::new(12 * S, 15 * S) },
        ];
        let mut edited = tl.apply_all(&ripple_ops(&tl, &cuts)).unwrap();

        // First join sits at 3s (removed 3s..7s); second join rippled to 8s (removed 12s..15s).
        assert_eq!(
            removed_at_join(&edited, 3 * S),
            Some(JoinRemoved { join_at: 3 * S, media: m, removed_source: Range::new(3 * S, 7 * S) })
        );
        assert_eq!(
            removed_at_join(&edited, 8 * S),
            Some(JoinRemoved { join_at: 8 * S, media: m, removed_source: Range::new(12 * S, 15 * S) })
        );
        assert_eq!(removed_at_join(&edited, 5 * S), None);

        // A bare split has left.source.end == right.source.start, so nothing was removed.
        let first_id = edited.tracks.iter().find(|t| t.id == v).unwrap().clips[0].id;
        edited.apply(&Op::Split { clip: first_id, at: S }).unwrap();
        assert_eq!(removed_at_join(&edited, S), None);

        // Different media meeting at a join returns None.
        let m2 = edited.add_media(MediaKind::Video, "/clips/broll.mp4", 5 * S);
        let end = edited.duration();
        edited.apply(&Op::AddClip { track: v, media: m2, at: end, source: Range::new(0, 2 * S) }).unwrap();
        assert_eq!(removed_at_join(&edited, end), None);
    }

    #[test]
    fn apply_cut_subset_keeps_five_of_six_cuts() {
        let (tl, m, _, _) = interview();
        let cuts = [
            SourceCut { media: m, range: Range::new(1 * S, 2 * S) },
            SourceCut { media: m, range: Range::new(4 * S, 5 * S) },
            SourceCut { media: m, range: Range::new(7 * S, 8 * S) },
            SourceCut { media: m, range: Range::new(10 * S, 11 * S) },
            SourceCut { media: m, range: Range::new(13 * S, 14 * S) },
            SourceCut { media: m, range: Range::new(16 * S, 17 * S) },
        ];
        // Keep 5 of the 6 cuts (skip index 2: 7s..8s).
        let mask = [true, true, false, true, true, true];
        let next = apply_cut_subset(&tl, &cuts, &mask).unwrap();

        assert_eq!(next.duration(), 15 * S);
        assert_eq!(
            left_out_ranges(&tl, &next, m),
            vec![
                Range::new(1 * S, 2 * S),
                Range::new(4 * S, 5 * S),
                Range::new(10 * S, 11 * S),
                Range::new(13 * S, 14 * S),
                Range::new(16 * S, 17 * S),
            ]
        );
        // 7s..8s still plays at timeline 5s..6s (shifted by two earlier 1s cuts).
        assert_eq!(
            source_to_timeline_spans(&next, m, Range::new(7 * S, 8 * S)),
            vec![Range::new(5 * S, 6 * S)]
        );

        // Invalid media in a kept cut errors, while invalid media in a skipped cut is ignored.
        let bad = [SourceCut { media: 999, range: Range::new(0, S) }];
        assert_eq!(apply_cut_subset(&tl, &bad, &[false]).unwrap(), tl);
        assert_eq!(apply_cut_subset(&tl, &bad, &[true]), Err(EditError::NoMedia(999)));
    }

    #[test]
    fn rebase_cuts_succeeds_across_concurrent_earlier_and_boundary_edits() {
        let (base, m, _, _) = interview();
        // Assistant proposes two cuts against `base`: 10s..12s and 16s..18s.
        let proposed = [
            SourceCut { media: m, range: Range::new(10 * S, 12 * S) },
            SourceCut { media: m, range: Range::new(16 * S, 18 * S) },
        ];

        // Meanwhile, human ripple-deletes 2s..6s AND 6s..10s (touching exact start boundary 10s!)
        // plus adds a marker — shifting all downstream timeline times by 8s!
        let mut current = base.apply_all(&ripple_ops(
            &base,
            &[
                SourceCut { media: m, range: Range::new(2 * S, 6 * S) },
                SourceCut { media: m, range: Range::new(6 * S, 10 * S) },
            ],
        )).unwrap();
        current.apply(&Op::AddMarker { at: 3 * S, label: "intro".into() }).unwrap();
        assert_eq!(current.duration(), 12 * S);

        let rebased = rebase_cuts(&base, &current, &proposed).unwrap();
        // 20s - 8s (human) - 4s (assistant) = 8s!
        assert_eq!(rebased.duration(), 8 * S);
        assert_eq!(
            left_out_ranges(&base, &rebased, m),
            vec![Range::new(2 * S, 12 * S), Range::new(16 * S, 18 * S)]
        );
    }

    #[test]
    fn rebase_cuts_refuses_overlapping_trim_removal_split_and_speed_change() {
        let (base, m, v, a) = interview();
        let proposed = [SourceCut { media: m, range: Range::new(10 * S, 13 * S) }];

        // 1. Overlapping ripple delete (9s..11s overlaps 10s..13s).
        let overlap_del = base
            .apply_all(&ripple_ops(&base, &[SourceCut { media: m, range: Range::new(9 * S, 11 * S) }]))
            .unwrap();
        let err = rebase_cuts(&base, &overlap_del, &proposed).unwrap_err();
        assert_eq!(err.media, m);
        assert_eq!(err.cut, Range::new(10 * S, 13 * S));

        // 2. Full removal of the region (8s..15s).
        let full_del = base
            .apply_all(&ripple_ops(&base, &[SourceCut { media: m, range: Range::new(8 * S, 15 * S) }]))
            .unwrap();
        assert!(rebase_cuts(&base, &full_del, &proposed).is_err());

        // 3. Speed change on the clip covering 10s..13s.
        let mut sped = base.clone();
        let vid = sped.tracks.iter().find(|t| t.id == v).unwrap().clips[0].id;
        let aid = sped.tracks.iter().find(|t| t.id == a).unwrap().clips[0].id;
        sped.apply(&Op::SetSpeed { clip: vid, speed: 1500 }).unwrap();
        sped.apply(&Op::SetSpeed { clip: aid, speed: 1500 }).unwrap();
        let speed_err = rebase_cuts(&base, &sped, &proposed).unwrap_err();
        assert_eq!(speed_err.reason, "source speed changed in current timeline");

        // 4. Razor split strictly inside 10s..13s (at 11s).
        let mut split_inside = base.clone();
        split_inside.apply(&Op::Split { clip: vid, at: 11 * S }).unwrap();
        assert!(rebase_cuts(&base, &split_inside, &proposed).is_err());
    }

    #[test]
    fn rebase_cuts_at_non_unit_speed_and_audio_only_left_out_ranges() {
        let mut base = Timeline::new(30, 1);
        let m = base.add_media(MediaKind::Audio, "/clips/podcast.wav", 20 * S);
        let a = base.add_track(TrackKind::Audio, "Podcast");
        base.apply(&Op::AddClip { track: a, media: m, at: 0, source: Range::new(0, 20 * S) }).unwrap();
        let cid = base.tracks[0].clips[0].id;
        // Base is already playing at 1.25x speed (16s timeline duration).
        base.apply(&Op::SetSpeed { clip: cid, speed: 1250 }).unwrap();
        assert_eq!(base.duration(), 16 * S);

        // Assistant proposes cutting source 10s..15s (which is 4s of timeline at 1.25x).
        let proposed = [SourceCut { media: m, range: Range::new(10 * S, 15 * S) }];

        // Human cuts source 0..5s (4s of timeline at 1.25x) while assistant thinks.
        let current = base
            .apply_all(&ripple_ops(&base, &[SourceCut { media: m, range: Range::new(0, 5 * S) }]))
            .unwrap();
        assert_eq!(current.duration(), 12 * S);

        let rebased = rebase_cuts(&base, &current, &proposed).unwrap();
        // Remaining source: 5s..10s (5s => 4s tl) + 15s..20s (5s => 4s tl) = 8s timeline!
        assert_eq!(rebased.duration(), 8 * S);
        assert_eq!(
            left_out_ranges(&base, &rebased, m),
            vec![Range::new(0, 5 * S), Range::new(10 * S, 15 * S)]
        );
    }
}
