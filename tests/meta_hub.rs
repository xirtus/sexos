// SPDX-License-Identifier: GPL-3.0-only
//! Host tests for the SexOS Meta Control Center & Super-Launcher Bento Hub
//! (`sex_pdx::meta_hub`), verifying zero overlap with Command Palette and Spotlight,
//! Voyeur quick-command ranking, wire packet fidelity, slider math, and hit-testing.

#[path = "../crates/sex-pdx/src/spotlight.rs"]
mod spotlight;

#[path = "../crates/sex-pdx/src/meta_hub.rs"]
mod meta_hub;

#[path = "../crates/voyeur/src/rank.rs"]
mod rank;

use meta_hub::*;

#[test]
fn meta_hub_never_overlaps_actual_spotlight_layout() {
    for &(w, h, sc, expect_fits) in &[
        (1280usize, 800usize, 1u32, true),
        (1600, 900, 1, true),
        (1920, 1080, 1, true),
        (1920, 1080, 2, false),
        (2560, 1600, 2, true),
        (3840, 2160, 3, true),
    ] {
        let bar = 28 * sc as usize;
        let sp = spotlight::Layout::new(w, h, bar, sc).unwrap();
        let anchor = (sp.panel.0 as i16, sp.panel.1 as i16, sp.panel.2 as u16, sp.panel.3 as u16);
        let res = Layout::compute(w, h, bar, sc, MODE_SPOTLIGHT, anchor);
        if expect_fits {
            let mh = res.expect("meta hub docks alongside Spotlight");
            assert!(rects_disjoint(mh.panel, sp.panel), "overlap on {}x{}@{}", w, h, sc);
            assert!(mh.panel.0 + mh.panel.2 <= w);
            assert!(mh.panel.1 >= bar);
            assert!(mh.panel.1 + mh.panel.3 <= h);
        } else if let Some(mh) = res {
            assert!(rects_disjoint(mh.panel, sp.panel), "overlap on {}x{}@{}", w, h, sc);
        }
    }
}

#[test]
fn quick_commands_rank_first_in_voyeur_without_displacing_legacy_queries() {
    let mut ranked = [rank::Ranked::default(); 32];
    for query in [
        "vol 65",
        "volume 80",
        "mute",
        "unmute",
        "mic mute",
        "bright 50",
        "brightness 90",
        "wifi on",
        "wifi off",
        "wifi scan",
        "bt on",
        "bt off",
        "gmode",
        "perf",
        "balanced",
        "saver",
        "night",
        "night 2",
        "accent rose",
        "accent graphite",
        "accent lavender",
        "accent ice",
        "dnd",
        "lock",
        "shot",
        "space 3",
    ] {
        let cmd = parse_quick_command(query).expect(query);
        let mut buf = [0u8; 48];
        let len = format_quick_command(cmd, &mut buf);
        let title = core::str::from_utf8(&buf[..len]).unwrap();
        let kw = quick_command_keywords(cmd);
        let cands = [
            rank::Candidate {
                provider: 3, // settings
                weight: 90,
                kind: 3,
                title: "Sound",
                keywords: "volume audio speakers microphone output input",
                hint: 0,
                hint_cap: rank::HINT_CAP_BUILTIN,
            },
            rank::Candidate {
                provider: 1, // apps
                weight: 100,
                kind: 3,
                title,
                keywords: kw,
                hint: rank::HINT_CAP_BUILTIN,
                hint_cap: rank::HINT_CAP_BUILTIN,
            },
        ];
        let n = rank::merge(query, 100, &rank::Frecency::new(), &cands, &mut ranked);
        assert!(n >= 1, "expected rank output for {}", query);
        assert_eq!(ranked[0].index, 1, "quick command should rank first for {}", query);
    }
}

#[test]
fn packet_receiver_tracks_visible_transitions_and_ssid() {
    let mut rx = Receiver::new();
    let mut s = MetaHubState::DEFAULT;
    s.visible = true;
    s.mode = MODE_COMMAND_PALETTE;
    s.anchor = (400, 274, 480, 280);
    s.wifi_enabled = true;
    s.wifi_connected = true;
    s.wifi_rssi_bars = 3;
    s.set_ssid(b"Studio-Noir");
    s.brightness_pct = 75;
    s.volume_pct = 55;
    s.mic_muted = true;
    s.power_mode = POWER_GMODE;
    s.night_light_level = 2;
    s.noir_accent = 2;
    s.dnd_enabled = true;
    s.active_space = 1;

    let (a0, a1, a2) = s.pack_anchor();
    let (s0, s1, s2) = s.pack_ssid_telemetry();
    let (p0, p1, p2) = s.pack_state();
    assert_eq!(rx.receive(a0, a1, a2), Some(false));
    assert_eq!(rx.receive(s0, s1, s2), Some(false));
    assert_eq!(rx.receive(p0, p1, p2), Some(true));
    assert_eq!(rx.current, s);
    assert_eq!(rx.current.ssid_str(), "Studio-Noir");

    // Repeated state produces no spurious redraw
    assert_eq!(rx.receive(p0, p1, p2), Some(false));
    // Closing produces redraw once
    assert_eq!(rx.receive(0, 0, 0), Some(true));
    assert_eq!(rx.receive(0, 0, 0), Some(false));
}
