//! Host test for Cufflinks's headset buttons and volume (`servers/cufflinks/src/remote.rs`),
//! the exact file the server compiles: buttons become key snapshots (released when the
//! headset forgets to), the headset's volume buttons become one volume key each, the
//! master volume goes to the headset, and our own setting echoed back presses nothing.
#[path = "../servers/cufflinks/src/remote.rs"]
mod remote;
use remote::*;

const H: u16 = 0x000b;

#[test]
fn volume_scales_round_trip() {
    for m in 0..=100u8 { assert_eq!(to_master(to_headset(m)), m, "{m}"); }
    assert_eq!((to_headset(0), to_headset(100), to_headset(250)), (0, 127, 127));
    assert_eq!((to_master(0), to_master(127), to_master(255)), (0, 100, 100));
}

#[test]
fn buttons_are_keys_and_a_lost_release_times_out() {
    let mut r = Remote::new();
    assert_eq!(r.key(H, PLAY_PAUSE, true, 0), None, "not ready yet");
    assert!(r.ready(H));
    assert!(!r.ready(0x000c), "the first headset is followed");
    assert_eq!(r.key(0x000c, PLAY_PAUSE, true, 0), None);
    assert_eq!(r.key(H, PLAY_PAUSE, true, 10), Some(Do::Keys(PLAY_PAUSE as u64)));
    assert_eq!(r.key(H, PLAY_PAUSE, false, 20), Some(Do::Keys(0)));
    assert_eq!(r.key(H, NEXT, false, 30), None, "a release of nothing");
    assert_eq!(r.key(H, STOP, true, 100), Some(Do::Keys(STOP as u64)));
    assert_eq!(r.tick(100 + RELEASE_TICKS - 1, None), None);
    assert_eq!(r.tick(100 + RELEASE_TICKS, None), Some(Do::Keys(0)));
    assert_eq!(r.tick(100 + 2 * RELEASE_TICKS, None), None);
    assert!(r.gone(H) && r.handle.is_none());
    assert!(r.ready(0x000c));
}

#[test]
fn volume_both_ways_without_a_loop() {
    let mut r = Remote::new();
    r.ready(H);
    // No absolute volume yet: the master is not sent.
    assert_eq!(r.tick(0, Some(40)), None);
    // The registration's answer: the headset does absolute volume; the system's volume wins.
    assert_eq!(r.volume(H, 100, false, Some(40)), None);
    assert_eq!(r.tick(1, Some(40)), Some(Do::SetVolume(to_headset(40))));
    assert_eq!(r.tick(2, Some(40)), None, "sent once");
    // The headset's own button: one volume key toward it.
    assert_eq!(r.volume(H, to_headset(40) + 8, true, Some(40)), Some(Do::Tap(VOLUME_UP)));
    assert_eq!(r.volume(H, to_headset(40) - 8, true, Some(40)), Some(Do::Tap(VOLUME_DOWN)));
    // The Shell stepped the master: it goes to the headset, whose echo presses nothing.
    assert_eq!(r.tick(3, Some(45)), Some(Do::SetVolume(to_headset(45))));
    assert_eq!(r.volume(H, to_headset(45), true, Some(45)), None);
    // A nudge under half a step: the headset is put back on the system's volume.
    assert_eq!(r.volume(H, to_headset(45) + 1, true, Some(45)), Some(Do::SetVolume(to_headset(45))));
    // At the ends nothing to press, and nothing without Bell's volume.
    assert_eq!(r.volume(H, 127, true, Some(100)), None);
    assert_eq!(r.volume(H, 0, true, Some(0)), None);
    assert_eq!(r.volume(H, 127, true, None), None);
    // A headset that stops doing absolute volume gets nothing more.
    r.no_volume(H);
    assert_eq!(r.tick(4, Some(80)), None);
}
