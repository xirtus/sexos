use moan::{project, Session};
use moan_core::{
    midi::{Instrument, Pattern},
    RATE,
};
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
fn midi_project_preserves_instrument_notes_trim_shared_cache_and_undo() {
    let mut s = Session::default();
    s.add_midi(Pattern::preset(Instrument::VelvetSampler))
        .unwrap();
    s.cursor = RATE as usize;
    s.split().unwrap();
    let original = s.tracks[0].clips[0].midi_pattern().unwrap().clone();
    s.edit_midi(|p| p.notes[0].key = 72).unwrap();
    assert_eq!(
        s.tracks[0].clips[1].midi_pattern().unwrap().notes[0].key,
        72
    );
    s.undo();
    assert_eq!(s.tracks[0].clips[0].midi_pattern(), Some(&original));
    s.redo();
    let bytes = s.encode_project().unwrap();
    assert_eq!(u16::from_le_bytes(bytes[8..10].try_into().unwrap()), 2);
    let restored = Session::decode_project(&bytes).unwrap();
    assert!(!restored.is_dirty());
    assert_eq!(restored.tracks[0].clips.len(), 2);
    assert_eq!(
        restored.tracks[0].clips[0].midi_pattern(),
        s.tracks[0].clips[0].midi_pattern()
    );
    assert_eq!(
        restored.tracks[0].clips[0].source_frames().as_ptr(),
        restored.tracks[0].clips[1].source_frames().as_ptr()
    );
    assert_eq!(restored.encode_project().unwrap(), bytes);
}
#[test]
fn audio_only_stays_exact_v1_and_mixed_sources_round_trip() {
    let mut s = Session::default();
    s.tone().unwrap();
    let original = s.encode_project().unwrap();
    assert_eq!(&original[8..10], &1u16.to_le_bytes());
    assert_eq!(
        Session::decode_project(&original)
            .unwrap()
            .encode_project()
            .unwrap(),
        original
    );
    s.add_midi(Pattern::preset(Instrument::SilkTriangle))
        .unwrap();
    let bytes = s.encode_project().unwrap();
    let back = Session::decode_project(&bytes).unwrap();
    assert!(back.tracks[0].clips[0].midi_pattern().is_none());
    assert!(back.tracks[1].clips[0].midi_pattern().is_some());
    assert_eq!(back.encode_project().unwrap(), bytes);
}
#[test]
fn v2_corrupt_metadata_and_false_pcm_cache_reject_with_valid_checksum() {
    let mut s = Session::default();
    s.add_midi(Pattern::preset(Instrument::SilkTriangle))
        .unwrap();
    let bytes = s.encode_project().unwrap();
    let n = RATE as usize * 4;
    let meta = project::HEADER + 4 + n * 4;
    for (offset, value) in [
        (meta, 2u16),
        (meta + 2, 255),
        (meta + 4, 0),
        (meta + 6, 129),
        (meta + 8 + 8, 256),
        (meta + 8 + 10, 0),
    ] {
        let mut b = bytes.clone();
        b[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        checksum(&mut b);
        assert!(Session::decode_project(&b).is_err(), "offset {offset}");
    }
    let mut b = bytes.clone();
    b[project::HEADER + 4 + 1000] ^= 1;
    checksum(&mut b);
    assert!(matches!(
        Session::decode_project(&b),
        Err(project::Error::Source)
    ));
    let mut b = bytes;
    b[meta + 8..meta + 12].copy_from_slice(&u32::MAX.to_le_bytes());
    checksum(&mut b);
    assert!(Session::decode_project(&b).is_err());
}
