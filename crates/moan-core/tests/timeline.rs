// SPDX-License-Identifier: GPL-3.0-only
use moan_core::{render, Clip, Error, Track};
fn track(source: &[[i16; 2]], start: usize) -> Track<'_> {
    let mut t = Track::new();
    t.clips.push(Clip::new(source, start).unwrap());
    t
}
#[test]
fn placement_and_silence() {
    let src = [[123, -456]; 3];
    let tracks = [track(&src, 2)];
    let mut out = [[999; 2]; 7];
    render(&tracks, 0, &mut out);
    assert_eq!(
        out,
        [
            [0, 0],
            [0, 0],
            [123, -456],
            [123, -456],
            [123, -456],
            [0, 0],
            [0, 0]
        ]
    );
}
#[test]
fn split_is_lossless_and_shares_source() {
    let src = [[100, -100], [200, -200], [300, -300], [400, -400]];
    let mut t = track(&src, 0);
    let right = t.clips[0].split(2).unwrap();
    t.clips.push(right);
    let mut out = [[0; 2]; 4];
    render(&[t], 0, &mut out);
    assert_eq!(out, src);
}
#[test]
fn trim_leaves_gap_and_keeps_gain() {
    let src = [[100; 2], [200; 2], [300; 2], [400; 2]];
    let mut t = track(&src, 1);
    t.clips[0].set_gain(50).unwrap();
    t.clips[0].trim(1, 3).unwrap();
    let mut out = [[0; 2]; 6];
    render(&[t], 0, &mut out);
    assert_eq!(out, [[0; 2], [0; 2], [100; 2], [150; 2], [0; 2], [0; 2]]);
}
#[test]
fn linear_fades_and_overlapping_envelopes() {
    let src = [[12000; 2]; 5];
    let mut t = track(&src, 0);
    t.clips[0].set_fades(3, 3).unwrap();
    let mut out = [[0; 2]; 5];
    render(&[t], 0, &mut out);
    assert_eq!(out, [[0; 2], [6000; 2], [12000; 2], [6000; 2], [0; 2]]);
    let src = [[12000; 2]; 3];
    let mut t = track(&src, 0);
    t.clips[0].set_fades(3, 3).unwrap();
    let mut out = [[0; 2]; 3];
    render(&[t], 0, &mut out);
    assert_eq!(out, [[0; 2], [3000; 2], [0; 2]]);
}
#[test]
fn pan_volume_and_clip_gain() {
    let src = [[4000, -8000]];
    let mut t = track(&src, 0);
    t.clips[0].set_gain(200).unwrap();
    t.set_volume(50).unwrap();
    t.set_pan(50).unwrap();
    let mut out = [[0; 2]];
    render(&[t], 0, &mut out);
    assert_eq!(out, [[2000, -8000]]);
}
#[test]
fn final_clipping_preserves_cancellation() {
    let pos = [[30000; 2]];
    let neg = [[-30000; 2]];
    let mut out = [[0; 2]];
    render(
        &[track(&pos, 0), track(&pos, 0), track(&neg, 0)],
        0,
        &mut out,
    );
    assert_eq!(out, pos);
    render(&[track(&pos, 0), track(&pos, 0)], 0, &mut out);
    assert_eq!(out, [[32767; 2]]);
    render(&[track(&neg, 0), track(&neg, 0)], 0, &mut out);
    assert_eq!(out, [[-32768; 2]]);
}
#[test]
fn mute_and_solo() {
    let src = [[100; 2]];
    let mut a = track(&src, 0);
    let b = track(&src, 0);
    a.solo = true;
    let mut tracks = [a, b];
    let mut out = [[0; 2]];
    render(&tracks, 0, &mut out);
    assert_eq!(out, src);
    tracks[0].muted = true;
    render(&tracks, 0, &mut out);
    assert_eq!(out, src);
    tracks[1].muted = true;
    render(&tracks, 0, &mut out);
    assert_eq!(out, [[0; 2]]);
}
#[test]
fn chunks_equal_one_render_across_edit_and_fade_boundaries() {
    let src = [[1000, -2000]; 100];
    let mut t = track(&src, 7);
    t.clips[0].set_fades(17, 23).unwrap();
    let r = t.clips[0].split(40).unwrap();
    t.clips.push(r);
    let tracks = [t];
    let mut full = [[0; 2]; 120];
    render(&tracks, 0, &mut full);
    let mut chunks = [[0; 2]; 120];
    for (i, chunk) in chunks.chunks_mut(13).enumerate() {
        render(&tracks, i * 13, chunk);
    }
    assert_eq!(full, chunks);
}
#[test]
fn invalid_edits_are_atomic_and_timeline_overflow_is_silent() {
    let src = [[123; 2]; 2];
    let mut t = track(&src, 0);
    assert_eq!(t.clips[0].trim(2, 1), Err(Error::Range));
    assert!(matches!(t.clips[0].split(0), Err(Error::Range)));
    assert!(matches!(t.clips[0].split(2), Err(Error::Range)));
    assert_eq!(t.clips[0].set_fades(3, 0), Err(Error::Range));
    assert_eq!(t.clips[0].set_gain(401), Err(Error::Gain));
    assert_eq!(t.set_pan(-101), Err(Error::Pan));
    assert_eq!(t.set_volume(401), Err(Error::Gain));
    let mut out = [[0; 2]; 2];
    render(&[t], 0, &mut out);
    assert_eq!(out, src);
    assert!(matches!(Clip::new(&src, usize::MAX), Err(Error::Range)));
    render(&[], usize::MAX, &mut out);
    assert_eq!(out, [[0; 2]; 2]);
}
#[test]
fn empty_clip_and_one_frame_fade() {
    let mut t = track(&[], 0);
    assert!(t.clips[0].is_empty());
    t.clips[0].trim(0, 0).unwrap();
    let src = [[100; 2]];
    let mut t = track(&src, 0);
    t.clips[0].set_fades(1, 0).unwrap();
    let mut out = [[999; 2]];
    render(&[t], 0, &mut out);
    assert_eq!(out, [[0; 2]]);
}
