//! Split edits: J and L cuts.
//!
//! Adapted from Scarlet's `splitJoin.ts` (MIT, github.com/Forward-Future/scarlet). A split
//! edit is a join whose sound cuts at another moment than its picture. With a J cut the next
//! shot's sound starts a little before its picture does; with an L cut a shot's sound carries
//! on a little after the picture has changed.
//!
//! The sound of each side plays from an audio track apart from its picture. A picture clip that
//! played its own sound hands a copy of it to an audio track and goes silent, and the two sounds
//! meet at the new moment. The picture, the runtime and every other join stay as they were, and
//! rendering needs nothing new: it plays what is on the tracks. The result is plain [`Op`]s, so
//! a split edit is undoable, can be staged as a suggestion, and replays like any other edit.
//!
//! The code keeps it safe and nothing else: a sound can't reach past what was recorded, shrink
//! to nothing, or overlap other sound on its track. How far, and at which joins, is the editor's
//! call.

use crate::{Clip, EditError, MediaKind, Micros, Op, Timeline, Track, TrackKind, SPEED_NORMAL};
use alloc::vec::Vec;

/// Why a split edit can't be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitEditError {
    /// No two shots meet at that moment on the picture track.
    NoJoin,
    /// The same footage runs on through the moment: there is no cut to split.
    ContinuousFootage,
    /// A side has no sound to move: its picture is silent and no audio track plays its
    /// recording in sync.
    NoSound,
    /// No audio track has room for the sound the picture hands over.
    NoRoom,
    /// The shift is zero, or would shrink a side's sound to nothing.
    TooFar,
    /// The recording has no sound that far before or after the clip (no handle), or the moved
    /// sound would overlap other sound on its track.
    Edit(EditError),
}

/// Two clips this close meet.
const MEET: Micros = 1_000;

fn plays(track: &Track, clip: &Clip) -> bool {
    clip.enabled && clip.volume > 0 && !(track.muted && track.kind == TrackKind::Audio)
}

/// Where a clip's source clock sits against the timeline: two clips with the same are in sync.
fn sync_of(c: &Clip) -> i128 {
    c.source.start as i128 * SPEED_NORMAL as i128 - c.at as i128 * c.speed as i128
}

/// Source length for `t` of timeline time at the clip's speed.
fn src(c: &Clip, t: Micros) -> Micros {
    t * c.speed as i64 / SPEED_NORMAL as i64
}

/// The two picture clips that meet at `at` on the first picture track.
fn picture_join(tl: &Timeline, at: Micros) -> Result<(&Clip, &Clip), SplitEditError> {
    let track = tl.tracks.iter().find(|t| t.kind == TrackKind::Video).ok_or(SplitEditError::NoJoin)?;
    let right = track.clips.iter().find(|c| c.enabled && (c.at - at).abs() <= MEET).ok_or(SplitEditError::NoJoin)?;
    let left = track.clips.iter().find(|c| c.id != right.id && c.enabled && (c.end() - right.at).abs() <= MEET).ok_or(SplitEditError::NoJoin)?;
    if left.media == right.media && left.speed == right.speed && (left.source.end - right.source.start).abs() <= MEET {
        return Err(SplitEditError::ContinuousFootage);
    }
    Ok((left, right))
}

/// The sound clip on an audio track in sync with picture clip `p` whose edge meets the join.
fn synced_sound<'a>(tl: &'a Timeline, p: &Clip, left: bool, at: Micros) -> Option<&'a Clip> {
    tl.tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio)
        .flat_map(|t| t.clips.iter().filter(move |c| plays(t, c)))
        .filter(|c| c.media == p.media && c.speed == p.speed && sync_of(c) == sync_of(p))
        .filter(|c| if left { (c.end() - at).abs() <= MEET } else { (c.at - at).abs() <= MEET })
        .min_by_key(|c| if left { (c.end() - at).abs() } else { (c.at - at).abs() })
}

/// The ops that put the sound cut at the join at timeline time `at` `shift` microseconds from
/// its picture cut: negative, the incoming sound leads in under the outgoing picture (a J cut);
/// positive, the outgoing sound carries on under the incoming picture (an L cut).
pub fn split_edit_ops(tl: &Timeline, at: Micros, shift: Micros) -> Result<Vec<Op>, SplitEditError> {
    if shift == 0 {
        return Err(SplitEditError::TooFar);
    }
    let (pl, pr) = picture_join(tl, at)?;
    let video_plays = |c: &Clip| {
        let t = tl.tracks.iter().find(|t| t.clips.iter().any(|x| x.id == c.id));
        t.is_some_and(|t| plays(t, c)) && tl.media(c.media).is_some_and(|m| m.kind == MediaKind::Video)
    };
    let mut ops = Vec::new();
    let mut work = tl.clone();
    // Each side's sound: a synced audio clip, or a copy the picture hands to an audio track.
    let mut sides = [0u32; 2];
    for (k, (p, left)) in [(pl, true), (pr, false)].into_iter().enumerate() {
        if let Some(c) = synced_sound(tl, p, left, at) {
            sides[k] = c.id;
            continue;
        }
        if !video_plays(p) {
            return Err(SplitEditError::NoSound);
        }
        if p.speed != SPEED_NORMAL {
            return Err(SplitEditError::NoSound);
        }
        let reach = shift.abs();
        let wanted = crate::Range::new((p.at - reach).max(0), p.end() + reach);
        let track = work
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio && !t.muted)
            .find(|t| t.clips.iter().all(|c| !c.span().overlaps(&wanted) || sides.contains(&c.id)))
            .map(|t| t.id)
            .ok_or(SplitEditError::NoRoom)?;
        let id = work.peek_next_id();
        let add = Op::AddClip { track, media: p.media, at: p.at, source: p.source };
        let mute = Op::SetVolume { clip: p.id, volume: 0 };
        work.apply(&add).map_err(SplitEditError::Edit)?;
        work.apply(&mute).map_err(SplitEditError::Edit)?;
        ops.push(add);
        ops.push(mute);
        sides[k] = id;
    }
    let find = |w: &Timeline, id: u32| w.clip(id).map(|(_, c)| c.clone()).ok_or(SplitEditError::Edit(EditError::NoClip(id)));
    let (al, ar) = (find(&work, sides[0])?, find(&work, sides[1])?);
    if (al.end() - ar.at).abs() > MEET {
        return Err(SplitEditError::NoSound);
    }
    let new = at + shift;
    if new <= al.at || new >= ar.end() {
        return Err(SplitEditError::TooFar);
    }
    let delta = new - ar.at;
    let mut steps = Vec::new();
    if delta < 0 {
        // J cut: the outgoing sound stops early, the incoming one starts early.
        let e = -delta;
        steps.push(Op::Trim { clip: al.id, source: crate::Range::new(al.source.start, al.source_at(new)) });
        steps.push(Op::Move { clip: ar.id, at: new });
        steps.push(Op::Trim { clip: ar.id, source: crate::Range::new(ar.source.start - src(&ar, e), ar.source.end) });
    } else {
        // L cut: the incoming sound starts late, the outgoing one carries on.
        steps.push(Op::Trim { clip: ar.id, source: crate::Range::new(ar.source.start + src(&ar, delta), ar.source.end) });
        steps.push(Op::Move { clip: ar.id, at: new });
        steps.push(Op::Trim { clip: al.id, source: crate::Range::new(al.source.start, al.source.end + src(&al, delta)) });
    }
    for op in &steps {
        work.apply(op).map_err(SplitEditError::Edit)?;
    }
    ops.extend(steps);
    Ok(ops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MediaKind, Range, SECOND};

    /// Two interview shots cut together at 4 s; each plays its own sound.
    fn two_shots() -> (Timeline, u32, u32, u32) {
        let mut tl = Timeline::new(30, 1);
        let a = tl.add_media(MediaKind::Video, "a.webm", 20 * SECOND);
        let b = tl.add_media(MediaKind::Video, "b.webm", 20 * SECOND);
        let v = tl.add_track(TrackKind::Video, "V1");
        let s = tl.add_track(TrackKind::Audio, "A1");
        tl.apply(&Op::AddClip { track: v, media: a, at: 0, source: Range::new(2 * SECOND, 6 * SECOND) }).unwrap();
        tl.apply(&Op::AddClip { track: v, media: b, at: 4 * SECOND, source: Range::new(5 * SECOND, 9 * SECOND) }).unwrap();
        (tl, v, s, a)
    }

    fn sound(tl: &Timeline, track: u32) -> Vec<(u32, Range, Range)> {
        tl.tracks.iter().find(|t| t.id == track).unwrap().clips.iter().map(|c| (c.media, c.span(), c.source)).collect()
    }

    #[test]
    fn a_j_cut_brings_the_next_shots_sound_in_early() {
        let (tl, v, s, _) = two_shots();
        let ops = split_edit_ops(&tl, 4 * SECOND, -SECOND / 2).unwrap();
        let out = tl.apply_all(&ops).unwrap();
        // The pictures are untouched; they went silent and their sound moved to A1.
        let pics: Vec<_> = out.tracks.iter().find(|t| t.id == v).unwrap().clips.iter().map(|c| (c.span(), c.volume)).collect();
        assert_eq!(pics, [(Range::new(0, 4 * SECOND), 0), (Range::new(4 * SECOND, 8 * SECOND), 0)]);
        let a1 = sound(&out, s);
        assert_eq!(a1[0].1, Range::new(0, 3_500_000));
        assert_eq!(a1[1].1, Range::new(3_500_000, 8 * SECOND));
        assert_eq!(a1[1].2, Range::new(4_500_000, 9 * SECOND), "b's sound plays half a second earlier in its recording");
        assert_eq!(out.duration(), tl.duration());
    }

    #[test]
    fn an_l_cut_carries_the_outgoing_sound_on() {
        let (tl, _, s, _) = two_shots();
        let out = tl.apply_all(&split_edit_ops(&tl, 4 * SECOND, SECOND).unwrap()).unwrap();
        let a1 = sound(&out, s);
        assert_eq!(a1[0].1, Range::new(0, 5 * SECOND));
        assert_eq!(a1[0].2, Range::new(2 * SECOND, 7 * SECOND));
        assert_eq!(a1[1].1, Range::new(5 * SECOND, 8 * SECOND));
        assert_eq!(a1[1].2, Range::new(6 * SECOND, 9 * SECOND));
    }

    #[test]
    fn synced_sound_already_on_a_track_moves_without_copies() {
        let (mut tl, _, s, _) = two_shots();
        let ops = split_edit_ops(&tl, 4 * SECOND, -SECOND / 2).unwrap();
        tl = tl.apply_all(&ops[..4]).unwrap(); // the hand-over only
        let again = split_edit_ops(&tl, 4 * SECOND, SECOND / 4).unwrap();
        assert!(again.iter().all(|o| !matches!(o, Op::AddClip { .. } | Op::SetVolume { .. })));
        let out = tl.apply_all(&again).unwrap();
        assert_eq!(sound(&out, s)[0].1, Range::new(0, 4_250_000));
    }

    #[test]
    fn it_refuses_what_is_not_safe() {
        let (tl, _, _, _) = two_shots();
        assert_eq!(split_edit_ops(&tl, 3 * SECOND, SECOND), Err(SplitEditError::NoJoin));
        assert_eq!(split_edit_ops(&tl, 4 * SECOND, 0), Err(SplitEditError::TooFar));
        assert_eq!(split_edit_ops(&tl, 4 * SECOND, -5 * SECOND), Err(SplitEditError::TooFar));
        // b's recording starts at 5 s: a 6 s lead reaches before it was recorded.
        let mut short = tl.clone();
        let b = short.tracks[0].clips[1].id;
        short.apply(&Op::Trim { clip: b, source: Range::new(0, 4 * SECOND) }).unwrap();
        assert!(matches!(split_edit_ops(&short, 4 * SECOND, -SECOND), Err(SplitEditError::Edit(EditError::BadSource))));
        // The same footage running on is no cut.
        let mut cont = Timeline::new(30, 1);
        let a = cont.add_media(MediaKind::Video, "a.webm", 20 * SECOND);
        let v = cont.add_track(TrackKind::Video, "V1");
        cont.add_track(TrackKind::Audio, "A1");
        cont.apply(&Op::AddClip { track: v, media: a, at: 0, source: Range::new(0, 4 * SECOND) }).unwrap();
        cont.apply(&Op::AddClip { track: v, media: a, at: 4 * SECOND, source: Range::new(4 * SECOND, 8 * SECOND) }).unwrap();
        assert_eq!(split_edit_ops(&cont, 4 * SECOND, SECOND), Err(SplitEditError::ContinuousFootage));
    }
}
