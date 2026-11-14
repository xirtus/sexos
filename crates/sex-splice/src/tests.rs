use super::history::History;
use super::render::{changed_window, plan, Piece};
use super::suggest::{Author, Status, Suggestions};
use super::transcript::{ripple_ops, Transcript, Word};
use super::*;
use alloc::vec;
use std::string::ToString;

const S: Micros = SECOND;

/// A 20 s interview: one camera file on a picture track and its sound on a sound track.
fn interview() -> (Timeline, u32, u32, u32) {
    let mut tl = Timeline::new(30, 1);
    let m = tl.add_media(MediaKind::Video, "/home/clips/interview.mp4", 20 * S);
    let v = tl.add_track(TrackKind::Video, "Picture");
    let a = tl.add_track(TrackKind::Audio, "Dialogue");
    tl.apply(&Op::AddClip { track: v, media: m, at: 0, source: Range::new(0, 20 * S) }).unwrap();
    tl.apply(&Op::AddClip { track: a, media: m, at: 0, source: Range::new(0, 20 * S) }).unwrap();
    (tl, m, v, a)
}

fn words(media: u32, list: &[(&str, f64, f64)]) -> Transcript {
    let words = list
        .iter()
        .map(|(t, a, b)| Word { text: t.to_string(), source: Range::new((a * 1e6) as i64, (b * 1e6) as i64) })
        .collect();
    Transcript { media, words }
}

/// "So, um, there are three things. There are three things I learned. [4 s pause] First..."
fn script(m: u32) -> Transcript {
    words(
        m,
        &[
            ("So,", 0.5, 0.8),
            ("um,", 1.0, 1.4),
            ("there", 1.6, 1.8),
            ("are", 1.8, 2.0),
            ("three", 2.0, 2.3),
            ("things.", 2.3, 2.8),
            ("There", 3.2, 3.4),
            ("are", 3.4, 3.6),
            ("three", 3.6, 3.9),
            ("things", 3.9, 4.3),
            ("I", 4.4, 4.5),
            ("learned.", 4.5, 5.0),
            ("First,", 9.0, 9.4),
            ("listen.", 9.5, 10.0),
        ],
    )
}

fn spans(tl: &Timeline, track: u32) -> std::vec::Vec<(Micros, Micros, Micros, Micros)> {
    tl.tracks.iter().find(|t| t.id == track).unwrap().clips.iter().map(|c| (c.at, c.end(), c.source.start, c.source.end)).collect()
}

#[test]
fn split_keeps_both_halves_playing_the_same_source() {
    let (mut tl, _, v, _) = interview();
    let id = tl.tracks[0].clips[0].id;
    tl.apply(&Op::Split { clip: id, at: 7 * S }).unwrap();
    assert_eq!(spans(&tl, v), vec![(0, 7 * S, 0, 7 * S), (7 * S, 20 * S, 7 * S, 20 * S)]);
    assert_eq!(tl.apply(&Op::Split { clip: id, at: 7 * S }), Err(EditError::OutsideClip));
}

#[test]
fn overlap_is_refused_and_leaves_the_timeline_unchanged() {
    let (mut tl, m, v, _) = interview();
    let before = tl.clone();
    let e = tl.apply(&Op::AddClip { track: v, media: m, at: 5 * S, source: Range::new(0, S) });
    assert!(matches!(e, Err(EditError::Overlap { .. })));
    assert_eq!(tl, before);
    let moved = tl.apply_all(&[Op::RippleDelete { range: Range::new(0, 2 * S) }, Op::Move { clip: 999, at: 0 }]);
    assert_eq!(moved, Err(EditError::NoClip(999)));
    assert_eq!(tl, before);
}

#[test]
fn ripple_delete_cuts_every_track_and_closes_the_gap() {
    let (mut tl, _, v, a) = interview();
    tl.apply(&Op::AddMarker { at: 15 * S, label: "punchline".into() }).unwrap();
    tl.apply(&Op::RippleDelete { range: Range::new(5 * S, 9 * S) }).unwrap();
    let want = vec![(0, 5 * S, 0, 5 * S), (5 * S, 16 * S, 9 * S, 20 * S)];
    assert_eq!(spans(&tl, v), want);
    assert_eq!(spans(&tl, a), want);
    assert_eq!(tl.duration(), 16 * S);
    assert_eq!(tl.markers[0].at, 11 * S);
    // Every clip id is unique after the cut made new halves.
    let mut ids: std::vec::Vec<u32> = tl.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.id)).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 4);
}

#[test]
fn speed_changes_duration_and_source_mapping() {
    let (mut tl, _, v, _) = interview();
    let id = tl.tracks[0].clips[0].id;
    tl.apply(&Op::SetSpeed { clip: id, speed: 2000 }).unwrap();
    assert_eq!(spans(&tl, v)[0].1, 10 * S);
    let c = tl.clip(id).unwrap().1;
    assert_eq!(c.source_at(3 * S), 6 * S);
    assert_eq!(c.timeline_at(6 * S), Some(3 * S));
    assert_eq!(tl.apply(&Op::SetSpeed { clip: id, speed: 0 }), Err(EditError::BadSpeed));
}

#[test]
fn deleting_a_sentence_removes_its_picture_and_sound() {
    let (tl, m, v, a) = interview();
    let t = script(m);
    // Delete the first, abandoned "there are three things." (words 2..=5).
    let cut = t.cut_words(2, 5).unwrap();
    assert_eq!(cut.range, Range::new(1_600_000, 3_200_000));
    let tl2 = tl.apply_all(&ripple_ops(&tl, &[cut])).unwrap();
    let want = vec![(0, 1_600_000, 0, 1_600_000), (1_600_000, 18_400_000, 3_200_000, 20 * S)];
    assert_eq!(spans(&tl2, v), want);
    assert_eq!(spans(&tl2, a), want);
    // The words that remain play in order; the deleted ones are gone.
    let playing: std::vec::Vec<&str> = t.playing(&tl2).iter().map(|(i, _)| t.words[*i].text.as_str()).collect();
    assert_eq!(playing, vec!["So,", "um,", "There", "are", "three", "things", "I", "learned.", "First,", "listen."]);
    // "There" now starts exactly where the cut joined.
    assert_eq!(t.playing(&tl2)[2].1, 1_600_000);
}

#[test]
fn retakes_fillers_and_pauses_are_found() {
    let (_, m, _, _) = interview();
    let t = script(m);
    assert_eq!(t.repeated_takes(2), vec![(2, 5)]);
    let f = t.filler_cuts();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].range, Range::new(1_000_000, 1_600_000));
    // The 4 s wait before "First," shrinks to 0.8 s, split around the middle.
    let p = t.pause_cuts(1_500_000, 800_000);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].range, Range::new(5_400_000, 8_600_000));
}

#[test]
fn cuts_found_before_other_edits_still_land_on_their_words() {
    let (tl, m, v, _) = interview();
    let t = script(m);
    // First cut the pause, then the filler which lies earlier, then a retake: each found
    // against the original transcript, each applied to the timeline as it is by then.
    let pause = t.pause_cuts(1_500_000, 800_000);
    let tl = tl.apply_all(&ripple_ops(&tl, &pause)).unwrap();
    let tl = tl.apply_all(&ripple_ops(&tl, &t.filler_cuts())).unwrap();
    let take = t.cut_words(2, 5).unwrap();
    let tl = tl.apply_all(&ripple_ops(&tl, &[take])).unwrap();
    // Cutting an already removed span again changes nothing.
    let again = tl.apply_all(&ripple_ops(&tl, &[take])).unwrap();
    assert_eq!(again, tl);
    let playing: std::vec::Vec<&str> = t.playing(&tl).iter().map(|(i, _)| t.words[*i].text.as_str()).collect();
    assert_eq!(playing, vec!["So,", "There", "are", "three", "things", "I", "learned.", "First,", "listen."]);
    // 20 s - 3.2 s pause - 0.6 s filler - 1.6 s retake.
    assert_eq!(tl.duration(), 14_600_000);
    // The filler and retake cuts touch, so three pieces of source remain.
    assert_eq!(spans(&tl, v), vec![(0, S, 0, S), (S, 3_200_000, 3_200_000, 5_400_000), (3_200_000, 14_600_000, 8_600_000, 20 * S)]);
}

#[test]
fn suggestions_wait_and_apply_against_the_current_timeline() {
    let (tl, m, _, _) = interview();
    let t = script(m);
    let mut s = Suggestions::default();
    let retake = s.stage(Author::Assistant, "Keep the second, complete take of the opening line.", vec![t.cut_words(2, 5).unwrap()]);
    let filler = s.stage(Author::Rule, "Remove the \"um\".", t.filler_cuts());
    assert_eq!(s.pending().count(), 2);
    // The editor shortens the long pause by hand first.
    let tl = tl.apply_all(&ripple_ops(&tl, &t.pause_cuts(1_500_000, 800_000))).unwrap();
    let tl = s.apply(retake, &tl).unwrap().unwrap();
    assert!(s.skip(filler));
    assert_eq!(s.get(retake).unwrap().status, Status::Applied);
    assert_eq!(s.get(filler).unwrap().status, Status::Skipped);
    assert!(s.apply(retake, &tl).unwrap().is_none(), "applied twice");
    let playing: std::vec::Vec<&str> = t.playing(&tl).iter().map(|(i, _)| t.words[*i].text.as_str()).collect();
    assert_eq!(&playing[..3], &["So,", "um,", "There"]);
}

#[test]
fn history_undo_redo_and_revert() {
    let (tl, m, _, _) = interview();
    let t = script(m);
    let mut h = History::new(tl.clone());
    let cut = h.current().apply_all(&ripple_ops(h.current(), &t.filler_cuts())).unwrap();
    h.commit("Remove filler", cut.clone());
    assert!(h.undo());
    assert_eq!(h.current(), &tl);
    assert!(h.redo());
    assert_eq!(h.current(), &cut);
    assert!(!h.redo());
    assert!(h.revert_to(0));
    assert_eq!(h.current(), h.original());
    assert_eq!(h.versions().len(), 3);
    assert_eq!(h.versions()[2].summary, "Back to: Original");
}

#[test]
fn render_plan_has_gaps_and_skips_disabled_clips() {
    let (mut tl, m, v, a) = interview();
    tl.apply(&Op::RippleDelete { range: Range::new(5 * S, 9 * S) }).unwrap();
    let second = tl.tracks[0].clips[1].id;
    tl.apply(&Op::Move { clip: second, at: 6 * S }).unwrap();
    tl.tracks[1].muted = true;
    let p = plan(&tl);
    assert_eq!(p[0].track, v);
    assert_eq!(p[0].pieces.len(), 3);
    assert!(matches!(p[0].pieces[1], Piece::Gap { timeline } if timeline == Range::new(5 * S, 6 * S)));
    assert!(matches!(p[0].pieces[2], Piece::Play { media, source, .. } if media == m && source.start == 9 * S));
    assert_eq!(p[1].track, a);
    assert_eq!(p[1].pieces, vec![Piece::Gap { timeline: Range::new(0, 17 * S) }]);
}

#[test]
fn preview_renders_only_around_the_change() {
    let (tl, m, _, _) = interview();
    let t = script(m);
    let after = tl.apply_all(&ripple_ops(&tl, &t.filler_cuts())).unwrap();
    // The cut removed 1.0-1.6 s; everything after plays the same, 0.6 s earlier.
    assert_eq!(changed_window(&tl, &after, 0), Some(Range::new(S, S)));
    assert_eq!(changed_window(&tl, &after, 2 * S), Some(Range::new(0, 3 * S)));
    assert_eq!(changed_window(&tl, &tl.clone(), S), None);
    // Changing the volume of the sound from 12 s on changes 12 s to the end.
    let mut louder = tl.clone();
    let aid = louder.tracks[1].clips[0].id;
    louder.apply(&Op::Split { clip: aid, at: 12 * S }).unwrap();
    assert_eq!(changed_window(&tl, &louder, 0), None, "a split alone changes nothing you see or hear");
    let tail = louder.tracks[1].clips[1].id;
    louder.apply(&Op::SetVolume { clip: tail, volume: 1500 }).unwrap();
    assert_eq!(changed_window(&tl, &louder, 0), Some(Range::new(12 * S, 20 * S)));
}

#[test]
fn otio_export_names_each_clip_and_gap_in_frames() {
    let (mut tl, _, _, _) = interview();
    tl.apply(&Op::RippleDelete { range: Range::new(5 * S, 9 * S) }).unwrap();
    let second = tl.tracks[0].clips[1].id;
    tl.apply(&Op::Move { clip: second, at: 6 * S }).unwrap();
    let doc = otio::export(&tl, "Interview \"cut 1\"");
    assert!(doc.starts_with("{\"OTIO_SCHEMA\":\"Timeline.1\",\"name\":\"Interview \\\"cut 1\\\"\""));
    assert!(doc.contains("\"kind\":\"Video\""));
    assert!(doc.contains("\"kind\":\"Audio\""));
    // Second picture clip: source 9 s (270 frames) for 11 s (330 frames) after a 1 s gap.
    assert!(doc.contains("{\"OTIO_SCHEMA\":\"Gap.1\",\"name\":\"\",\"source_range\":{\"OTIO_SCHEMA\":\"TimeRange.1\",\"start_time\":{\"OTIO_SCHEMA\":\"RationalTime.1\",\"rate\":30.0,\"value\":0.0},\"duration\":{\"OTIO_SCHEMA\":\"RationalTime.1\",\"rate\":30.0,\"value\":30.0}}}"));
    assert!(doc.contains("\"value\":270.0},\"duration\":{\"OTIO_SCHEMA\":\"RationalTime.1\",\"rate\":30.0,\"value\":330.0}"));
    assert!(doc.contains("\"target_url\":\"/home/clips/interview.mp4\""));
    assert_eq!(doc.matches("Clip.2").count(), 4);
}
