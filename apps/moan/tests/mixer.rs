use moan::Session;
use moan_core::{Clip, Frame, Track};
use std::sync::Arc;
fn rehash(bytes: &mut [u8]) {
    let sum = bytes[..24]
        .iter()
        .chain(&bytes[32..])
        .fold(0xcbf29ce484222325u64, |sum, b| {
            (sum ^ *b as u64).wrapping_mul(0x100000001b3)
        });
    bytes[24..32].copy_from_slice(&sum.to_le_bytes());
}
#[test]
fn master_v3_preserves_legacy_formats_and_exact_gain_undo() {
    let mut session = Session::default();
    session.add(vec![[12000, -24000]; 20]).unwrap();
    let original = session.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes(original[8..10].try_into().unwrap()), 1);
    session.mark_saved();
    session.set_master_volume(50).unwrap();
    assert!(session.is_dirty());
    let mut block = [[0; 2]; 20];
    session.render_processed(0, &mut block);
    assert!(block.iter().all(|f| *f == [6000, -12000]));
    let encoded = session.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes(encoded[8..10].try_into().unwrap()), 3);
    let mut restored = Session::decode_project(&encoded).unwrap();
    assert_eq!(restored.master_volume(), 50);
    restored.render_processed(0, &mut block);
    assert_eq!(block[0], [6000, -12000]);
    for gain in [0, 200] {
        restored.set_master_volume(gain).unwrap();
        let bytes = restored.encode_project().unwrap();
        assert_eq!(
            Session::decode_project(&bytes).unwrap().master_volume(),
            gain
        );
    }
    let mut bad = encoded.clone();
    bad[38..40].copy_from_slice(&201u16.to_le_bytes());
    rehash(&mut bad);
    assert!(Session::decode_project(&bad).is_err());
    assert!(session.undo());
    assert_eq!(session.master_volume(), 100);
    assert!(!session.is_dirty());
    assert_eq!(session.encode_project().unwrap(), original);
    assert!(session.redo());
    assert_eq!(session.master_volume(), 50);
    let before = session.encode_project().unwrap();
    assert!(session.set_master_volume(201).is_err());
    assert_eq!(session.encode_project().unwrap(), before);
}
#[test]
fn recording_and_plugin_import_share_pcm_without_an_arc_slice_copy() {
    let pcm = vec![[12, -34]; 48000];
    let ptr = pcm.as_ptr();
    let mut session = Session::default();
    session.add(pcm).unwrap();
    assert_eq!(session.tracks[0].clips[0].source_frames().as_ptr(), ptr);
    session.set_master_volume(70).unwrap();
    assert!(session.undo());
    assert_eq!(session.tracks[0].clips[0].source_frames().as_ptr(), ptr);
}
#[test]
fn offline_region_keeps_clip_edges_and_excludes_track_and_master_mix() {
    let mut session = Session::default();
    session.cursor = 48000;
    session.add(vec![[1000, -2000]; 100]).unwrap();
    session.tracks[0].clips[0].set_gain(50).unwrap();
    session.tracks[0].clips[0].set_fades(1, 1).unwrap();
    session.tracks[0].set_volume(30).unwrap();
    session.tracks[0].set_pan(100).unwrap();
    session.set_master_volume(20).unwrap();
    let before = session.encode_project().unwrap();
    let (start, pcm) = session.offline_region().unwrap();
    assert_eq!(start, 48000);
    assert_eq!(pcm[0], [0; 2]);
    assert_eq!(pcm[1], [500, -1000]);
    assert_eq!(pcm[99], [0; 2]);
    assert_eq!(session.encode_project().unwrap(), before);
    // No room for the promised new track: fail before creating an input job.
    for _ in 1..8 {
        session.add(vec![[0; 2]; 10]).unwrap();
    }
    assert!(session.offline_region().is_err());
}
#[test]
fn buffer_clip_trims_and_serializes_like_legacy_owned_audio() {
    let samples: Vec<Frame> = (0..20).map(|n| [n, -n]).collect();
    let mut a = Session::default();
    a.add(samples.clone()).unwrap();
    let mut b = Session::default();
    let mut track = Track::new();
    track
        .clips
        .push(Clip::owned(Arc::from(samples), 0).unwrap());
    b.tracks.push(track);
    b.to = 20;
    assert_eq!(a.encode_project().unwrap(), b.encode_project().unwrap());
}
