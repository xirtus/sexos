//! Host test for programs starting programs (`crates/sex-pdx/src/launch.rs`).
#[path = "../crates/sex-pdx/src/launch.rs"]
mod launch;
#[path = "../crates/sex-pdx/src/program_store.rs"]
mod program_store;
mod package { pub const GRANT_TOKEN: &[u8] = b"+packages"; }
use launch::*;

#[test]
fn events_round_trip() {
    for e in [
        Event::Started { cookie: 7, row: 40, task: 9001 },
        Event::Refused { cookie: u32::MAX, code: program_store::NO_FREE_ROW },
        Event::Done { cookie: 3, row: 41, task: 12, status: Some(0) },
        Event::Done { cookie: 3, row: 41, task: 12, status: Some(u32::MAX) },
        Event::Done { cookie: 3, row: 41, task: 12, status: None },
    ] {
        assert_eq!(Event::decode(OP_EVENT, e.wire()), Some(e));
    }
    assert_eq!(Event::decode(OP_EVENT, [9 << 32, 0, 0]), None);
    assert_eq!(Event::decode(OP_LAUNCH, Event::Started { cookie: 1, row: 1, task: 1 }.wire()), None);
}

#[test]
fn files_open_in_the_program_for_their_type() {
    assert_eq!(program_for(b"/home/report.PDF"), Some(&b"pdf"[..]));
    assert_eq!(program_for(b"/media/stick/clip.webm"), Some(&b"play"[..]));
    assert_eq!(program_for(b"/music/song.flac"), Some(&b"play"[..]));
    assert_eq!(program_for(b"/Screenshot-3.png"), Some(&b"view"[..]));
    assert_eq!(program_for(b"/notes/todo.md"), Some(&b"jot"[..]));
    assert_eq!(program_for(b"/bin/hello"), None);
    assert_eq!(program_for(b"/dir.d/noext"), None);
    assert_eq!(program_for(b"/x.verylongext"), None);
    assert_eq!(program_for(b"/trailing."), None);
}

#[test]
fn open_hands_over_exactly_one_plain_path() {
    let mut out = [0u8; 64];
    let n = open_args(b"/home/a.pdf", &mut out).unwrap();
    assert_eq!(&out[..n], b"@/home/a.pdf");
    for bad in [&b"home/a.pdf"[..], b"/a b.pdf", b"/x/../etc", b"/./a", b""] { assert_eq!(open_args(bad, &mut out), None); }
    assert_eq!(open_args(&[b'/'; 64], &mut out), None);
    let n = bin_path(b"pdf", &mut out).unwrap();
    assert_eq!(&out[..n], b"/bin/pdf");
    assert_eq!(bin_path(b"a/b", &mut out), None);
    assert_eq!(bin_path(b"", &mut out), None);
}

#[test]
fn a_starter_passes_on_only_what_it_holds() {
    let holds = |t: &[u8]| t.starts_with(b"@/home");
    assert!(args_allowed(b"--thumb @/home/a.pdf @/home/out 112", false, holds));
    assert!(!args_allowed(b"@/etc/secret", false, holds));
    assert!(!args_allowed(b"@/home/a.pdf +packages", false, holds));
    assert!(args_allowed(b"+packages", true, holds));
    assert!(args_allowed(b"", false, holds));
}
