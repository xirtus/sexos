// SPDX-License-Identifier: GPL-3.0-only
//! Host test for desktop widgets (slice S5b): the words and layout in
//! crates/sex-pdx/src/desktop_widgets.rs.
#[path = "../crates/sex-pdx/src/desktop_widgets.rs"]
mod desktop_widgets;
use desktop_widgets::*;

fn s(l: Line) -> String { String::from_utf8(l.as_bytes().to_vec()).unwrap() }

#[test]
fn dates() {
    assert_eq!(civil_date(0), (1970, 1, 1));
    assert_eq!(civil_date(-1), (1969, 12, 31));
    assert_eq!(civil_date(11_016), (2000, 2, 29));
    assert_eq!(civil_date(20_774), (2026, 11, 17));
}

#[test]
fn clock_card() {
    // 2026-11-17 14:05:09 UTC, a Tuesday.
    let t = 20_774 * 86_400 + 14 * 3600 + 5 * 60 + 9;
    let (time, date) = clock_lines(t, true);
    assert_eq!((s(time), s(date)), ("14:05".into(), "Tuesday 17 November".into()));
    assert_eq!(s(clock_lines(t, false).0), "2:05 PM");
    assert_eq!(s(clock_lines(20_774 * 86_400 + 5 * 60, false).0), "12:05 AM");
    assert_eq!(s(clock_lines(0, true).1), "Thursday 1 January");
    assert_eq!(s(clock_lines(-1, true).0), "23:59");
}

#[test]
fn processor_load() {
    let a = Sample { tsc: 1_000, cores: 2, idle: 0 };
    assert_eq!(cpu_percent(a, Sample { tsc: 2_000, cores: 2, idle: 1_500 }), Some(25));
    assert_eq!(cpu_percent(a, Sample { tsc: 2_000, cores: 2, idle: 2_000 }), Some(0));
    assert_eq!(cpu_percent(a, Sample { tsc: 2_000, cores: 2, idle: 9_999 }), Some(0), "idle past the span is clamped");
    assert_eq!(cpu_percent(a, Sample { tsc: 2_000, cores: 2, idle: 0 }), Some(100));
    assert_eq!(cpu_percent(a, a), None);
    assert_eq!(cpu_percent(a, Sample { tsc: 5, cores: 2, idle: 0 }), None, "the clock went back");
}

#[test]
fn system_card() {
    let (a, b) = system_lines(Some(7), Some(300_000));
    assert_eq!((s(a), s(b)), ("Processor 7%".into(), "Memory free 1.1 GB".into()));
    let (a, b) = system_lines(None, Some(1000));
    assert_eq!((s(a), s(b)), ("Processor -".into(), "Memory free 4 MB".into()));
    assert_eq!(s(system_lines(Some(100), None).1), "Memory free -");
}

#[test]
fn layout_column_top_right() {
    let l = Layout::new(ALL, 1280, 800, 32, 1).unwrap();
    assert_eq!(l.clock, Some((1280 - 24 - 240, 32 + 24, 240, CLOCK_H)));
    assert_eq!(l.system, Some((1280 - 24 - 240, 32 + 24 + CLOCK_H + GAP, 240, SYSTEM_H)));
    assert_eq!(l.column, (1016, 56, 240, CLOCK_H + GAP + SYSTEM_H));
    let only = Layout::new(SYSTEM, 1280, 800, 32, 1).unwrap();
    assert_eq!((only.clock, only.system.unwrap().1), (None, 56));
    assert_eq!(Layout::new(0, 1280, 800, 32, 1), None);
    assert_eq!(Layout::new(ALL, 200, 800, 32, 1), None, "too narrow");
    let big = Layout::new(ALL, 3840, 2160, 64, 3).unwrap();
    assert_eq!(big.scale, 2, "cards stop growing at scale 2");
    assert_eq!(big.clock.unwrap().2, 480);
}
