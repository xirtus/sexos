// SPDX-License-Identifier: GPL-3.0-only
//! Qupid's media intelligence on the host: episodes from names, the next file,
//! skip cards from chapters, "Up next" countdown, 50%-300% projector audio DSP + EQ,
//! non-truncated stream telemetry + season-pack ranking, and ProviderPack hydration.
extern crate alloc;
#[path = "../crates/sex-episodes/src/audio_dsp.rs"] mod audio_dsp;
#[path = "../crates/sex-episodes/src/episodes.rs"] mod episodes;
#[path = "../crates/sex-episodes/src/pack.rs"] mod pack;
#[path = "../crates/sex-episodes/src/segments.rs"] mod segments;
#[path = "../crates/sex-episodes/src/streams.rs"] mod streams;

use audio_dsp::*;
use episodes::*;
use pack::*;
use segments::*;
use streams::*;

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

#[test] fn audio_dsp_unity_flat_is_bit_exact_and_osd_labels_match() {
    let mut cfg = AudioDspConfig::default();
    assert!(cfg.is_identity());
    assert_eq!(cfg.osd_button_label(), "Vol / EQ");

    let mut dsp = AudioDsp::new(48_000, cfg);
    let orig = [[0i16, 0], [4200, -4200], [31_000, -31_000], [-1234, 5678]];
    let mut buf = orig;
    dsp.process_stereo_frames(&mut buf);
    assert_eq!(buf, orig, "100% Flat must be bit-exact pass-through");

    assert_eq!(cfg.toggle_boost_200(), 200);
    assert_eq!(cfg.osd_button_label(), "Vol 200%");
    cfg.apply_preset(EqPreset::Projector);
    assert_eq!(cfg.osd_button_label(), "Vol 200% · Projector");
    assert_eq!(cfg.toggle_boost_200(), 100);
    assert_eq!(cfg.osd_button_label(), "EQ · Projector");
}

#[test] fn audio_dsp_200_and_300_boost_amplify_quiet_and_soft_limit_peaks() {
    let mut cfg = AudioDspConfig::default();
    cfg.boost_percent = 200;
    let mut dsp200 = AudioDsp::new(48_000, cfg);
    let mut quiet = [[4_000i16, -4_000i16]; 8];
    dsp200.process_stereo_frames(&mut quiet);
    assert!((quiet[7][0] - 8_000).abs() <= 2, "expected ~8000 at 200%, got {}", quiet[7][0]);
    assert!((quiet[7][1] + 8_000).abs() <= 2);

    cfg.boost_percent = 300;
    let mut dsp300 = AudioDsp::new(48_000, cfg);
    let mut hot = [[28_000i16, -28_000i16]; 16];
    dsp300.process_stereo_frames(&mut hot);
    for &[l, r] in &hot {
        assert!((29_000..=32_767).contains(&l), "300% hot peak must soft-limit without wrap: {l}");
        assert!((-32_767..=-29_000).contains(&r), "300% hot peak must soft-limit without wrap: {r}");
    }
}

#[test] fn audio_dsp_three_band_eq_and_dialogue_drc_lift_speech() {
    // Generate 1.8 kHz speech-band sine wave at -24 dBFS (~2060 peak).
    let n = 1024usize;
    let mut speech_flat = Vec::with_capacity(n);
    for i in 0..n {
        let phase = 2.0 * std::f32::consts::PI * 1800.0 * (i as f32) / 48_000.0;
        let s = (phase.sin() * 2000.0) as i16;
        speech_flat.push([s, s]);
    }
    let mut speech_boosted = speech_flat.clone();
    let mut eq_cfg = AudioDspConfig::default();
    eq_cfg.apply_preset(EqPreset::Dialogue); // Voice +6 dB + Dialogue DRC
    let mut dsp = AudioDsp::new(48_000, eq_cfg);
    dsp.process_stereo_frames(&mut speech_boosted);

    let peak_before = speech_flat[512..].iter().map(|f| f[0].abs()).max().unwrap();
    let peak_after = speech_boosted[512..].iter().map(|f| f[0].abs()).max().unwrap();
    assert!(
        peak_after > peak_before * 2,
        "Dialogue +6 dB voice EQ + DRC should more than double quiet 1.8 kHz speech ({peak_before} -> {peak_after})"
    );

    // Sub -54 dBFS room noise floor (`~20` i16 units) must NOT be pumped by DRC.
    let mut drc_only = AudioDspConfig::default();
    drc_only.dialogue_drc = true;
    let mut gate_dsp = AudioDsp::new(48_000, drc_only);
    let mut hiss = [[20i16, -20i16]; 256];
    gate_dsp.process_stereo_frames(&mut hiss);
    assert!((hiss[255][0] - 20).abs() <= 1, "noise gate should hold room hiss at unity");
}

#[test] fn audio_dsp_surround_51_and_71_downmix_lifts_center_dialogue() {
    let mut dsp = AudioDsp::default();
    // 5.1 frame: FL=0, FR=0, FC=10_000, LFE=10_000, BL=0, BR=0 vs pure surround BL=10_000.
    let center_51 = [0i16, 0, 10_000, 0, 0, 0];
    let surround_51 = [0i16, 0, 0, 0, 10_000, 0];
    let mut out_fc = [[0i16; 2]; 1];
    let mut out_bl = [[0i16; 2]; 1];
    assert_eq!(dsp.downmix_surround_to_stereo(&center_51, 6, &mut out_fc), 1);
    assert_eq!(dsp.downmix_surround_to_stereo(&surround_51, 6, &mut out_bl), 1);
    assert_eq!(out_fc[0][0], out_fc[0][1], "centre dialogue pans equally to L and R");
    assert!(out_fc[0][0] > out_bl[0][0] * 2, "centre channel (+3.2 dB lift) must outweigh surround");
}

#[test] fn streams_stremio_telemetry_never_truncates_seeds_codec_size_or_source() {
    let raw = r#"{
      "streams": [
        {
          "name": "Swarm\n4K DV",
          "title": "Cosmos.A.Space.Time.Odyssey.S01E04.2160p.WEB-DL.DDP5.1.DV.HDR10+.HEVC-OPENGRP.mkv\n👤 184 💾 2.45 GB ⚙️ OpenSwarm",
          "infoHash": "0123456789abcdef0123456789abcdef01234567",
          "fileIdx": 3,
          "behaviorHints": {
            "bingeGroup": "cosmos-2160p-hevc",
            "filename": "Cosmos.S01E04.2160p.DV.HDR10+.HEVC-OPENGRP.mkv"
          },
          "sources": [
            "tracker:udp://tracker.opentrackr.org:1337/announce",
            "udp://open.stealth.si:80/announce"
          ]
        }
      ]
    }"#;
    let parsed = parse_stremio_streams_json("Aggregator", raw).expect("valid stremio streams json");
    assert_eq!(parsed.len(), 1);
    let s = &parsed[0];
    assert_eq!(s.seeders, Some(184));
    assert_eq!(s.resolution_p, 2160);
    assert_eq!(s.video_codec, Some("HEVC"));
    assert_eq!(s.hdr_badge, Some("DV · HDR"));
    assert_eq!(s.source_indexer.as_deref(), Some("OpenSwarm"));
    assert_eq!(s.file_idx, Some(3));

    let (header, release) = s.display_lines();
    assert!(header.starts_with("👤 184 · 4K 2160p · HEVC · DV · HDR · 💾 2.45 GB · ⚙️ OpenSwarm"), "got: {header}");
    assert_eq!(release, "Cosmos.S01E04.2160p.DV.HDR10+.HEVC-OPENGRP.mkv");
    assert!(s.matches_episode(1, 4));
    assert!(!s.matches_episode(1, 5));

    let magnet = s
        .build_magnet_uri(&[
            String::from("udp://tracker.opentrackr.org:1337/announce"),
            String::from("udp://tracker.torrent.eu.org:451/announce"),
        ])
        .expect("magnet");
    assert!(magnet.starts_with("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"));
    assert!(magnet.ends_with("&so=3"));
    assert_eq!(magnet.matches("opentrackr.org").count(), 1, "duplicate trackers must be deduplicated");
}

#[test] fn streams_season_pack_matching_and_viability_ranking() {
    let raw = r#"{
      "streams": [
        {
          "name": "Dead Remux",
          "title": "Show.S02E03.2160p.BluRay.REMUX.AVC-BIG.mkv\n👤 0 💾 58.0 GB",
          "infoHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        },
        {
          "name": "Oversized Remux",
          "title": "Show.S02E03.2160p.BluRay.REMUX.H264-BIG.mkv\n👤 90 💾 54.0 GB",
          "infoHash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        },
        {
          "name": "Compact AV1 Pack",
          "title": "Show.S01-S03.1080p.AV1.10bit.Opus-CLEAN\n👤 145 💾 1.35 GB",
          "infoHash": "cccccccccccccccccccccccccccccccccccccccc"
        },
        {
          "name": "Legacy x264",
          "title": "Show.S02E03.1080p.WEB.x264-OTHER.mkv\n👤 110 💾 3.80 GB",
          "infoHash": "dddddddddddddddddddddddddddddddddddddddd"
        }
      ]
    }"#;
    let all = parse_stremio_streams_json("OpenIndex", raw).unwrap();
    assert_eq!(all.len(), 4);
    assert!(all[2].matches_episode(2, 3), "S01-S03 pack must match S02E03");
    assert!(!all[2].matches_episode(4, 1), "S01-S03 pack must not match S04E01");

    let ranked = rank_streams(
        &all,
        &RankOptions {
            drop_zero_seed_torrents: true,
            prefer_modern_codecs: true,
            max_preferred_size_bytes: Some(8 * 1024 * 1024 * 1024), // 8 GB ceiling
            previous_release: Some("Show.S02E02.1080p.AV1.10bit.Opus-CLEAN.mkv"),
        },
    );
    assert_eq!(ranked.len(), 3, "0-seed dead stream must be dropped");
    assert_eq!(ranked[0].video_codec, Some("AV1"), "streamable AV1 match must rank #1");
    assert_eq!(ranked[2].info_hash, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "54 GB remux penalized last");
}

#[test] fn streams_rss_btih_array_and_movies_json_parsers() {
    let rss = r#"<?xml version="1.0" encoding="UTF-8"?>
    <rss version="2.0" xmlns:open="https://example.org/xmlns">
      <channel>
        <item>
          <title>[OpenEncode] Big Buck Bunny - 01 (1080p AV1) [1234ABCD].mkv</title>
          <link>magnet:?xt=urn:btih:1111222233334444555566667777888899990000&amp;tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce</link>
          <open:seeders>73</open:seeders>
          <open:size>640 MiB</open:size>
        </item>
      </channel>
    </rss>"#;
    let rss_items = parse_rss_swarm_xml("ArchiveRSS", rss);
    assert_eq!(rss_items.len(), 1);
    assert_eq!(rss_items[0].seeders, Some(73));
    assert_eq!(rss_items[0].video_codec, Some("AV1"));
    assert_eq!(rss_items[0].size_bytes, Some(640 * 1024 * 1024));

    let btih_json = r#"[
      {"name":"No results returned","info_hash":"0000000000000000000000000000000000000000","seeders":"0","size":"0"},
      {"name":"Sintel.2010.2160p.HEVC.10bit-OPEN","info_hash":"ABCDEF0123456789ABCDEF0123456789ABCDEF01","seeders":"95","leechers":"4","size":"1610612736"}
    ]"#;
    let btih_items = parse_btih_array_json("OpenBTIH", btih_json).unwrap();
    assert_eq!(btih_items.len(), 1, "all-zero sentinel row must be ignored");
    assert_eq!(btih_items[0].seeders, Some(95));
    assert_eq!(btih_items[0].video_codec, Some("HEVC"));
    assert_eq!(format_bytes(btih_items[0].size_bytes.unwrap()), "1.50 GB");

    let movies_json = r#"{
      "status": "ok",
      "data": {
        "movies": [
          {
            "title_long": "Tears of Steel (2012)",
            "torrents": [
              {"hash": "fedcba9876543210fedcba9876543210fedcba98", "quality": "1080p", "type": "web", "video_codec": "x265", "seeds": 62, "peers": 5, "size_bytes": 943718400}
            ]
          }
        ]
      }
    }"#;
    let movie_items = parse_movies_torrents_json("OpenMovies", movies_json).unwrap();
    assert_eq!(movie_items.len(), 1);
    assert_eq!(movie_items[0].seeders, Some(62));
    assert_eq!(movie_items[0].video_codec, Some("HEVC"));
}

#[test] fn pack_public_defaults_are_clean_and_overlay_merges_idempotently() {
    let mut cfg = QupidCatalogConfig::load_default();
    assert!(cfg.stremio_addons.iter().any(|a| a.id == "com.linvo.cinemeta"));
    assert!(cfg.swarm_indexers.iter().any(|i| i.id == "internet-archive"));
    assert!(cfg.kodi_repositories.iter().any(|r| r.id == "repository.xbmc.org"));
    #[cfg(qupid_seed_pack)]
    {
        assert!(ProviderPack::has_baked_seed());
        assert!(cfg.stremio_addons.len() >= 2, "baked seed pack must add stream addons");
        assert!(cfg.swarm_indexers.len() >= 5, "baked seed pack must add swarm indexers");
        assert!(cfg.kodi_repositories.len() >= 2, "baked seed pack must add Kodi repos");
    }

    let overlay = r#"{
      "schemaVersion": 1,
      "name": "External Research Pack",
      "trackers": [
        "udp://tracker.opentrackr.org:1337/announce",
        "wss://tracker.openwebtorrent.com"
      ],
      "stremioAddons": [
        {
          "id": "org.example.openstreams",
          "name": "OpenStreams",
          "version": "1.2.0",
          "transportUrl": "https://streams.example.org/manifest.json",
          "types": ["movie", "series"],
          "resources": ["stream"],
          "idPrefixes": ["tt"]
        }
      ],
      "swarmIndexers": [
        {
          "id": "open-mirror-movies",
          "name": "Open Mirror Movies",
          "kind": "json_movies_torrents",
          "baseUrls": ["https://m1.example.org", "https://m2.example.org/"],
          "pathTemplate": "/api/list.json?query_term={query}&sort_by=seeds",
          "tabGroup": "movies_tv",
          "adult": false,
          "imdbFallback": true
        }
      ],
      "kodiRepositories": [
        {
          "id": "repository.example.video",
          "name": "Example Video Repo",
          "zipUrl": "https://repo.example.org/repository.example.video-1.0.0.zip",
          "installAddons": ["plugin.video.example"]
        }
      ]
    }"#;

    cfg.apply_pack_json(overlay).expect("valid overlay pack");
    cfg.apply_pack_json(overlay).expect("idempotent second apply");

    assert_eq!(cfg.active_pack_name, "External Research Pack");
    assert_eq!(cfg.trackers.iter().filter(|t| t.contains("opentrackr")).count(), 1);
    assert!(cfg.trackers.iter().any(|t| t == "wss://tracker.openwebtorrent.com"));

    let addon = cfg.stremio_addons.iter().find(|a| a.id == "org.example.openstreams").unwrap();
    assert_eq!(addon.stream_url("series", "tt1234567:1:4"), "https://streams.example.org/stream/series/tt1234567%3A1%3A4.json");

    let ix = cfg.swarm_indexers.iter().find(|i| i.id == "open-mirror-movies").unwrap();
    assert_eq!(
        ix.build_search_urls("open movie", None),
        vec![
            String::from("https://m1.example.org/api/list.json?query_term=open+movie&sort_by=seeds"),
            String::from("https://m2.example.org/api/list.json?query_term=open+movie&sort_by=seeds"),
        ]
    );
    assert_eq!(ix.build_imdb_urls("tt0062622").len(), 2);
    assert!(cfg.kodi_repositories.iter().any(|r| r.id == "repository.example.video"));
}
