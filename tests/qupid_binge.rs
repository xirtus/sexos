// SPDX-License-Identifier: GPL-3.0-only
//! Qupid's "Keep watching" logic on the host: episodes from names, the next file,
//! skip cards from chapters and the "Up next" countdown.
extern crate alloc;
#[path = "../crates/sex-media/src/episodes.rs"] mod episodes;
#[path = "../crates/sex-media/src/segments.rs"] mod segments;
use episodes::*;
use segments::*;

fn ep(path: &str) -> (String, u32, u32) {
    let e = parse(path).unwrap_or_else(|| panic!("no episode in {path}"));
    (e.show, e.season, e.episode)
}

#[test] fn names_give_show_season_episode() {
    assert_eq!(ep("/tv/The.Expanse.S02E05.1080p.WEB-DL.x264-NTb.mkv"), ("the expanse".into(), 2, 5));
    assert_eq!(ep("the expanse s2e6.mp4"), ("the expanse".into(), 2, 6));
    assert_eq!(ep("Firefly 1x03 Bushwhacked.avi"), ("firefly".into(), 1, 3));
    assert_eq!(ep("[SubsPlease] Frieren - 07 (1080p) [ABCD1234].mkv"), ("frieren".into(), 1, 7));
    assert_eq!(ep("[Group] Show Name - 12v2 [720p].mkv"), ("show name".into(), 1, 12));
    assert_eq!(ep("Cosmos Episode 4.webm"), ("cosmos".into(), 1, 4));
    assert_eq!(ep("Cosmos.Ep.05.webm"), ("cosmos".into(), 1, 5));
    assert_eq!(ep("Show.S01.E09.mkv"), ("show".into(), 1, 9));
}

#[test] fn films_and_years_are_not_episodes() {
    assert_eq!(parse("Blade Runner - 1982 [1080p].mkv"), None);
    assert_eq!(parse("holiday.mp4"), None);
    assert_eq!(parse("1080p sample.mkv"), None);
    assert_eq!(parse("Sexy Beast (2000).mkv"), None);
}

#[test] fn release_groups() {
    assert_eq!(release_group("[SubsPlease] Frieren - 07 (1080p).mkv").as_deref(), Some("subsplease"));
    assert_eq!(release_group("The.Expanse.S02E05.1080p.WEB-DL.x264-NTb.mkv").as_deref(), Some("ntb"));
    assert_eq!(release_group("Show.S01E01.1080p.WEB.H264-GROUP [rarbg].mkv").as_deref(), Some("group"));
    assert_eq!(release_group("[1080p] Show - 01.mkv"), None);
    assert_eq!(release_group("Show - 01.mkv"), None);
    assert_eq!(resolution("x.2160p.mkv"), 2160);
    assert_eq!(codec("Show.S01E01.HEVC.mkv"), Some("hevc"));
}

#[test] fn next_episode_in_order_skipping_specials() {
    let files = [
        "Show.S01E03.mkv", "Show.S00E01.Special.mkv", "Show.S01E01.mkv", "Other.S01E02.mkv",
        "Show.S02E01.mkv", "notes.txt",
    ];
    assert_eq!(next_episode("Show.S01E01.mkv", &files), Some(0));
    assert_eq!(next_episode("Show.S01E03.mkv", &files), Some(4));
    assert_eq!(next_episode("Show.S02E01.mkv", &files), None);
    assert_eq!(next_episode("film.mkv", &files), None);
    // Watching a special: the next special or the first episode after it.
    assert_eq!(next_episode("Show.S00E01.Special.mkv", &files), Some(2));
}

#[test] fn next_episode_prefers_the_same_release() {
    let files = [
        "Show.S01E02.720p.WEB.x264-OTHER.mkv",
        "Show.S01E02.1080p.WEB-DL.DDP5.1.HEVC-NTb.mkv",
        "Show.S01E02.2160p.REMUX.mkv",
    ];
    assert_eq!(next_episode("Show.S01E01.1080p.WEB-DL.DDP5.1.HEVC-NTb.mkv", &files), Some(1));
    assert_eq!(next_episode("Show.S01E01.720p.WEB.x264-OTHER.mkv", &files), Some(0));
    assert!(similarity("A.1080p.HEVC-NTb.mkv", "B.1080p.HEVC-NTb.mkv") > similarity("A.1080p.HEVC-NTb.mkv", "B.720p.x264-X.mkv"));
}

fn ch(title: &str, s: u64) -> Chapter { Chapter { title: title.into(), start_ms: s * 1000 } }

#[test] fn chapters_become_skip_segments() {
    assert_eq!(classify("Opening"), Some(Kind::Intro));
    assert_eq!(classify("OP"), Some(Kind::Intro));
    assert_eq!(classify("Previously on"), Some(Kind::Recap));
    assert_eq!(classify("End Credits"), Some(Kind::Credits));
    assert_eq!(classify("ED"), Some(Kind::Credits));
    assert_eq!(classify("Next Episode Preview"), Some(Kind::Preview));
    assert_eq!(classify("Part A"), None);
    assert_eq!(classify("Weekend"), None);
    let chapters = [ch("Part B", 400), ch("Prologue", 0), ch("Opening", 90), ch("Part A", 180), ch("Ending", 1300), ch("Preview", 1390)];
    let segs = from_chapters(&chapters, 1420_000);
    assert_eq!(segs, vec![
        Segment { kind: Kind::Intro, start_ms: 90_000, end_ms: 180_000 },
        Segment { kind: Kind::Credits, start_ms: 1_300_000, end_ms: 1_390_000 },
        Segment { kind: Kind::Preview, start_ms: 1_390_000, end_ms: 1_420_000 },
    ]);
    assert_eq!(skip_at(&segs, 89_000), None);
    assert_eq!(skip_at(&segs, 100_000).map(|s| s.kind.prompt()), Some("Skip intro"));
    assert_eq!(skip_at(&segs, 179_000), None, "too close to its end to bother");
    // "Ending" and "Ending (cont.)" are one segment.
    let joined = from_chapters(&[ch("Ending", 10), ch("Ending (cont.)", 20)], 30_000);
    assert_eq!(joined, vec![Segment { kind: Kind::Credits, start_ms: 10_000, end_ms: 30_000 }]);
}

#[test] fn up_next_starts_at_end_credits_or_before_the_end() {
    let segs = from_chapters(&[ch("Intro", 0), ch("Story", 60), ch("Credits", 1300)], 1400_000);
    assert_eq!(up_next_from(&segs, 1400_000), Some(1_300_000));
    // Credits chapters early in a video (a cold open's credits) do not count.
    let early = from_chapters(&[ch("Credits", 30), ch("Story", 90)], 1400_000);
    assert_eq!(up_next_from(&early, 1400_000), Some(1_370_000));
    assert_eq!(up_next_from(&[], 40_000), None, "short clips need no card");
}

#[test] fn up_next_counts_down_and_can_be_dismissed() {
    let mut card = UpNextCard::new(&[], 600_000);
    assert_eq!(card.at(500_000), UpNext::Hidden);
    assert_eq!(card.at(570_000), UpNext::Counting(10_000));
    assert_eq!(card.at(574_000), UpNext::Counting(6_000));
    assert_eq!(card.at(580_000), UpNext::Go);
    card.dismiss();
    assert_eq!(card.at(585_000), UpNext::Hidden, "watching the credits");
    assert_eq!(card.at(100_000), UpNext::Hidden);
    assert_eq!(card.at(571_000), UpNext::Counting(9_000), "seeking back re-arms it");
}
