//! Host test for session restore (`crates/sex-pdx/src/session_restore.rs`).
#[path = "../crates/sex-pdx/src/session_restore.rs"]
mod session_restore;
use session_restore::*;

#[test]
fn lines_round_trip() {
    let mut out = [0u8; 128];
    for e in [
        Entry { scene: 0, network: false, path: b"/bin/view", args: b"@/pic.png" },
        Entry { scene: 3, network: true, path: b"/bin/peek", args: b"http://x/y z" },
        Entry { scene: 9, network: false, path: b"/bin/fortune", args: b"" },
    ] {
        let n = e.encode(&mut out).unwrap();
        assert_eq!(out[n - 1], b'\n');
        assert_eq!(Entry::decode(&out[..n - 1]), Some(e));
    }
    assert_eq!(Entry { scene: 10, network: false, path: b"/a", args: b"" }.encode(&mut out), None);
    assert_eq!(Entry { scene: 1, network: false, path: b"/a", args: b"x\ny" }.encode(&mut out), None);
    assert_eq!(Entry { scene: 1, network: false, path: b"/a", args: b"" }.encode(&mut [0u8; 4]), None);
}

#[test]
fn damaged_lines_are_skipped() {
    let text = b"1 0 /bin/view @/a.png\ngarbage\n2 1 /bin/peek http://h/\nx 0 /bin/no\n3 0 relative\n\n";
    let got: Vec<_> = entries(text).collect();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].scene, 1);
    assert_eq!(got[1].path, b"/bin/peek");
    assert!(got[1].network);
    let many: Vec<u8> = (0..20).flat_map(|_| b"0 0 /bin/a\n".iter().copied()).collect();
    assert_eq!(entries(&many).count(), MAX_RUNS);
}

#[test]
fn scene_rides_in_run_flags() {
    for s in 0..9u8 { assert_eq!(scene_of(scene_flags(s) | 1), Some(s)); }
    assert_eq!(scene_of(1), None);
}
