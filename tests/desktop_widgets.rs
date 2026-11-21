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
    let l = Layout::new(CLOCK | SYSTEM, 1280, 800, 32, 1).unwrap();
    assert_eq!(l.clock, Some((1280 - 24 - 240, 32 + 24, 240, CLOCK_H)));
    assert_eq!(l.system, Some((1280 - 24 - 240, 32 + 24 + CLOCK_H + GAP, 240, SYSTEM_H)));
    assert_eq!(l.column, (1016, 56, 240, CLOCK_H + GAP + SYSTEM_H));
    assert_eq!(l.weather, None);
    // The weather card goes between the clock and the system card.
    let all = Layout::new(ALL, 1280, 800, 32, 1).unwrap();
    assert_eq!(all.weather, Some((1016, 56 + CLOCK_H + GAP, 240, WEATHER_H)));
    assert_eq!(all.system.unwrap().1, 56 + CLOCK_H + GAP + WEATHER_H + GAP);
    assert_eq!(all.column.3, CLOCK_H + WEATHER_H + SYSTEM_H + 2 * GAP);
    let only = Layout::new(SYSTEM, 1280, 800, 32, 1).unwrap();
    assert_eq!((only.clock, only.system.unwrap().1), (None, 56));
    assert_eq!(Layout::new(0, 1280, 800, 32, 1), None);
    assert_eq!(Layout::new(ALL, 200, 800, 32, 1), None, "too narrow");
    let big = Layout::new(ALL, 3840, 2160, 64, 3).unwrap();
    assert_eq!(big.scale, 2, "cards stop growing at scale 2");
    assert_eq!(big.clock.unwrap().2, 480);
}

#[test]
fn weather_wire() {
    let w = Weather { now: -73, high: 12, low: -150, code: 95 };
    for r in [Report::Ok(w), Report::Unavailable(NO_PLACE), Report::Unavailable(NO_NETWORK), Report::Unavailable(NO_FORECAST)] {
        assert_eq!(Report::unpack(r.pack()), Some(r));
    }
    assert_eq!(Report::unpack(0), None);
    assert_eq!(Report::unpack(3 << 56), None);
    assert_eq!(Report::unpack(2 << 56 | 9 << 48), None, "unknown reason");
    assert_eq!(Report::unpack(2 << 56 | 2 << 48 | 5), None, "stray bits");
}

#[test]
fn every_zone_city_has_a_place() {
    // The zones Settings offers, read from sex-config's table.
    let src = include_str!("../crates/sex-config/src/lib.rs");
    let table = &src[src.find("pub const ZONES: &[&str] = &[").unwrap()..];
    let table = &table[..table.find("];").unwrap()];
    let zones: Vec<&str> = table.split('"').skip(1).step_by(2).collect();
    assert!(zones.len() > 50, "{zones:?}");
    for z in zones { assert_eq!(place(z).is_some(), z != "UTC", "{z}"); }
}

#[test]
fn forecast_request() {
    let mut out = [0u8; 192];
    let n = forecast_path(5252, 1340, &mut out);
    assert_eq!(std::str::from_utf8(&out[..n]).unwrap(),
        "/v1/forecast?latitude=52.52&longitude=13.40&current=temperature_2m,weather_code&daily=temperature_2m_max,temperature_2m_min&forecast_days=1&timezone=auto");
    let n = forecast_path(-621, -13, &mut out);
    assert!(std::str::from_utf8(&out[..n]).unwrap().starts_with("/v1/forecast?latitude=-6.21&longitude=-0.13&"));
    assert_eq!(place("Europe/Berlin"), Some((5252, 1340)));
    assert_eq!(place("UTC"), None);
}

#[test]
fn forecast_answer() {
    // Open-Meteo's answer, as it comes (units first, then values).
    let body = r#"{"latitude":52.52,"longitude":13.419998,"generationtime_ms":0.05,"utc_offset_seconds":3600,"timezone":"Europe/Berlin","timezone_abbreviation":"GMT+1","elevation":38.0,"current_units":{"time":"iso8601","interval":"seconds","temperature_2m":"°C","weather_code":"wmo code"},"current":{"time":"2026-11-18T23:00","interval":900,"temperature_2m":6.86,"weather_code":3},"daily_units":{"time":"iso8601","temperature_2m_max":"°C","temperature_2m_min":"°C"},"daily":{"time":["2026-11-18"],"temperature_2m_max":[9.4],"temperature_2m_min":[-2.25]}}"#.as_bytes();
    assert_eq!(parse_forecast(body), Some(Weather { now: 69, high: 94, low: -23, code: 3 }));
    assert_eq!(parse_forecast(b"{}"), None);
    assert_eq!(parse_forecast(br#"{"current":{"temperature_2m":null,"weather_code":3},"daily":{"temperature_2m_max":[1],"temperature_2m_min":[0]}}"#), None);
    assert_eq!(parse_forecast(br#"{"current":{"temperature_2m":999,"weather_code":3},"daily":{"temperature_2m_max":[1],"temperature_2m_min":[0]}}"#), None, "out of this world");
    let cold = br#"{"current":{"temperature_2m":-0.04,"weather_code":71},"daily":{"temperature_2m_max":[0],"temperature_2m_min":[-12]}}"#;
    assert_eq!(parse_forecast(cold), Some(Weather { now: 0, high: 0, low: -120, code: 71 }));
}

#[test]
fn weather_card() {
    let w = Weather { now: 69, high: 94, low: -23, code: 3 };
    let (a, b) = weather_lines(Some(Report::Ok(w)), "Europe/Berlin");
    assert_eq!((s(a), s(b)), ("Berlin 7°".into(), "Cloudy, 9° / −2°".into()));
    let (a, b) = weather_lines(Some(Report::Ok(Weather { now: 200, high: 250, low: -50, code: 0 })), "America/New_York");
    assert_eq!((s(a), s(b)), ("New York 68°".into(), "Clear, 77° / 23°".into()));
    assert_eq!(s(weather_lines(None, "Europe/Berlin").1), "Looking it up...");
    assert_eq!(s(weather_lines(Some(Report::Unavailable(NO_PLACE)), "UTC").1), "Set a time zone city");
    assert_eq!(s(weather_lines(Some(Report::Unavailable(NO_NETWORK)), "UTC").1), "Open-Meteo out of reach");
    assert_eq!(sky(96), "Thunderstorm, hail");
    assert_eq!(sky(42), "Weather");
}
