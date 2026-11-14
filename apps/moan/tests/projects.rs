// SPDX-License-Identifier: GPL-3.0-only
use moan::{
    project::{self, Error},
    Session, Studio,
};
use moan_core::{
    effects::{Compressor, Equalizer, Reverb},
    render, render_processed,
};
use weft::{Strings, Theme, Ui};
fn fixture() -> Session {
    let mut s = Session::default();
    s.add((0..100).map(|i| [i * 100 - 4000, 4000 - i * 70]).collect())
        .unwrap();
    s.cursor = 40;
    s.split().unwrap();
    s.tracks[0].set_volume(75).unwrap();
    s.tracks[0].set_pan(-20).unwrap();
    s.tracks[0].muted = true;
    s.tracks[0].solo = true;
    s.tracks[0].clips[0].set_gain(150).unwrap();
    s.tracks[0].clips[0].set_fades(5, 10).unwrap();
    s.tracks[0].effects.eq = Some(Equalizer::new(140, 100, 80).unwrap());
    s.tracks[0].effects.compressor = Some(Compressor::new(6000, 4, 48, 4800).unwrap());
    s.tracks[0].effects.reverb = Some(Reverb::new(65, 25).unwrap());
    s.cursor = 200;
    s.add(vec![[123, -456]; 100]).unwrap();
    s.from = 20;
    s.to = 80;
    s.mark_saved();
    s
}
fn checksum(bytes: &mut [u8]) {
    let hash = bytes[..24]
        .iter()
        .chain(&bytes[32..])
        .fold(0xcbf29ce484222325u64, |v, b| {
            (v ^ *b as u64).wrapping_mul(0x100000001b3)
        });
    bytes[24..32].copy_from_slice(&hash.to_le_bytes());
}
#[test]
fn editable_project_roundtrip_preserves_pcm_clips_mixer_effects_and_shared_sources() {
    let original = fixture();
    let bytes = original.encode_project().unwrap();
    let decoded = Session::decode_project(&bytes).unwrap();
    assert_eq!(decoded.encode_project().unwrap(), bytes);
    assert_eq!(
        (decoded.selected, decoded.cursor, decoded.from, decoded.to),
        (1, 200, 20, 80)
    );
    assert_eq!(decoded.tracks[0].clips[0].fades(), (5, 10));
    assert_eq!(decoded.tracks[0].clips[0].gain(), 150);
    assert_eq!(decoded.tracks[0].clips[1].source_offset(), 40);
    assert_eq!(
        decoded.tracks[0].clips[0].source_frames().as_ptr(),
        decoded.tracks[0].clips[1].source_frames().as_ptr()
    );
    assert_eq!(
        (
            decoded.tracks[0].volume(),
            decoded.tracks[0].pan(),
            decoded.tracks[0].muted,
            decoded.tracks[0].solo
        ),
        (75, -20, true, true)
    );
    assert_eq!(
        decoded.tracks[0].effects.eq.as_ref().unwrap().gains(),
        [140, 100, 80]
    );
    assert_eq!(
        decoded.tracks[0]
            .effects
            .compressor
            .as_ref()
            .unwrap()
            .parameters(),
        (6000, 4, 48, 4800)
    );
    assert_eq!(
        decoded.tracks[0]
            .effects
            .reverb
            .as_ref()
            .unwrap()
            .parameters(),
        (65, 25)
    );
    let mut a = [[0; 2]; 500];
    let mut b = a;
    render(&original.tracks, 0, &mut a);
    render(&decoded.tracks, 0, &mut b);
    assert_eq!(a, b);
    let mut a_tracks = original.tracks.clone();
    let mut b_tracks = decoded.tracks.clone();
    a_tracks[0].muted = false;
    b_tracks[0].muted = false;
    render_processed(&mut a_tracks, 0, &mut a);
    render_processed(&mut b_tracks, 0, &mut b);
    assert_eq!(a, b);
    assert_eq!(
        u16::from_le_bytes(bytes[32..34].try_into().unwrap()),
        2,
        "split clips must not duplicate their source PCM"
    );
}
#[test]
fn reopening_starts_clean_and_first_edit_undo_restores_loaded_baseline() {
    let bytes = fixture().encode_project().unwrap();
    let mut s = Session::decode_project(&bytes).unwrap();
    assert!(!s.is_dirty());
    assert!(!s.undo());
    assert!(!s.redo());
    s.gain(50).unwrap();
    assert!(s.is_dirty());
    assert!(s.undo());
    assert!(!s.is_dirty());
    assert_eq!(s.encode_project().unwrap(), bytes);
    assert!(s.redo());
    assert!(s.is_dirty());
    s.mark_saved();
    assert!(!s.is_dirty());
    s.undo();
    assert!(s.is_dirty());
}
#[test]
fn empty_and_empty_track_projects_roundtrip() {
    let mut s = Session::default();
    assert_eq!(
        Session::decode_project(&s.encode_project().unwrap())
            .unwrap()
            .tracks
            .len(),
        0
    );
    s.add(vec![[1; 2]; 4]).unwrap();
    s.from = 0;
    s.to = 4;
    s.cut().unwrap();
    let decoded = Session::decode_project(&s.encode_project().unwrap()).unwrap();
    assert_eq!(decoded.tracks.len(), 1);
    assert!(decoded.tracks[0].clips.is_empty());
}
#[test]
fn every_truncation_trailer_and_checksum_corruption_is_rejected() {
    let bytes = fixture().encode_project().unwrap();
    for n in 0..bytes.len() {
        assert!(
            Session::decode_project(&bytes[..n]).is_err(),
            "truncation {n}"
        );
    }
    let mut corrupt = bytes.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(matches!(
        Session::decode_project(&corrupt),
        Err(Error::Checksum)
    ));
    let mut trailer = bytes.clone();
    trailer.push(0);
    assert!(Session::decode_project(&trailer).is_err());
    let mut version = bytes;
    version[8..10].copy_from_slice(&2u16.to_le_bytes());
    assert!(matches!(
        Session::decode_project(&version),
        Err(Error::Version)
    ));
}
#[test]
fn bounded_header_counts_and_positions_fail_before_pcm_allocation() {
    let original = fixture().encode_project().unwrap();
    for (at, value) in [(32, u16::MAX), (34, 9), (36, 99), (38, 1)] {
        let mut bytes = original.clone();
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
        checksum(&mut bytes);
        assert!(Session::decode_project(&bytes).is_err());
    }
    let mut bytes = original.clone();
    bytes[40..48].copy_from_slice(&u64::MAX.to_le_bytes());
    checksum(&mut bytes);
    assert!(Session::decode_project(&bytes).is_err());
    let mut bytes = original;
    bytes[64..68].copy_from_slice(&u32::MAX.to_le_bytes());
    checksum(&mut bytes);
    assert!(Session::decode_project(&bytes).is_err());
}
#[test]
fn malformed_source_ranges_gain_and_effects_are_rejected() {
    let original = fixture().encode_project().unwrap();
    let track = 64 + (4 + 100 * 4) * 2;
    let clip = track + 30;
    for (at, value) in [
        (track + 2, 401u16),
        (track + 14, 0),
        (track + 16, 21),
        (track + 26, 91),
        (clip, 99),
        (clip + 20, 401),
        (clip + 22, 1),
    ] {
        let mut bytes = original.clone();
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
        checksum(&mut bytes);
        assert!(Session::decode_project(&bytes).is_err(), "offset {at}");
    }
    for at in [clip + 12, clip + 16, clip + 24, clip + 28] {
        let mut bytes = original.clone();
        bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        checksum(&mut bytes);
        assert!(Session::decode_project(&bytes).is_err());
    }
}
#[test]
fn bad_open_never_partially_replaces_the_existing_project() {
    let mut ui = Ui::new(640, 400, Theme::dark(), Strings::new("en"));
    let mut studio = Studio::build(&mut ui);
    studio.session = fixture();
    let before = studio.session.encode_project().unwrap();
    assert!(studio.load_project(&mut ui, b"broken").is_err());
    assert_eq!(studio.session.encode_project().unwrap(), before);
    studio.load_project(&mut ui, &before).unwrap();
    assert!(!studio.session.is_dirty());
    studio.session.gain(50).unwrap();
    studio.session.undo();
    assert!(!studio.session.is_dirty());
}
#[test]
fn byte_mutations_with_valid_checksums_do_not_panic_or_escape_bounds() {
    let bytes = fixture().encode_project().unwrap();
    for at in (32..bytes.len()).step_by(7) {
        let mut candidate = bytes.clone();
        candidate[at] ^= 0xFF;
        checksum(&mut candidate);
        if let Ok(s) = Session::decode_project(&candidate) {
            assert!(s.tracks.len() <= 8);
            assert!(s.duration() <= project::MAX_TIMELINE);
        }
    }
}
