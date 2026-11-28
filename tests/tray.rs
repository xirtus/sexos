//! Host test for the panel tray (`crates/sex-pdx/src/tray.rs`): wire, menu
//! layouts shared by Shell clicks and Display drawing, and the calendar.
#[path = "../crates/sex-pdx/src/tray.rs"]
mod tray;
use tray::*;

#[test]
fn status_and_view_round_trip() {
    let s = Status { volume: 63, muted: true, sound: true, battery: Some(87), charging: true, on_ac: Some(true), wifi: 10 };
    assert_eq!(Status::decode(s.wire()), s);
    assert_eq!(Status::decode(Status::UNKNOWN.wire()), Status::UNKNOWN);
    for v in [View { menu: Menu::Quick, month_offset: 0 }, View { menu: Menu::Calendar, month_offset: -3 }, View::default()] {
        assert_eq!(View::decode(v.wire()), Some(v));
    }
    assert_eq!(View::decode(7), None);
    let mut out = [0u8; 8];
    let n = label_decode(label_wire(b"DE"), &mut out);
    assert_eq!(&out[..n], b"DE");
}

#[test]
fn quick_menu_hits_what_it_draws() {
    let q = Quick::new(1280, 28, 1).unwrap();
    assert!(q.panel.0 + q.panel.2 <= 1280);
    let (sx, sy, sw, sh) = q.slider;
    assert_eq!(q.hit(sx, sy + sh / 2), QuickHit::Volume(0));
    assert_eq!(q.hit(sx + sw / 2, sy), QuickHit::Volume(50));
    assert_eq!(q.hit(sx + sw - 1, sy), QuickHit::Volume(99));
    assert_eq!(q.volume_at(sx + sw + 50), 100);
    let m = q.mute;
    assert_eq!(q.hit(m.0 + 1, m.1 + 1), QuickHit::Mute);
    let r = q.rows[3];
    assert_eq!(q.hit(r.0 + 5, r.1 + 5), QuickHit::Layout);
    assert_eq!(q.hit(q.rows[1].0 + 5, q.rows[1].1 + 5), QuickHit::Network);
    assert_eq!(q.hit(5, 500), QuickHit::Outside);
    assert!(Quick::new(200, 28, 1).is_none());
    let q2 = Quick::new(2560, 56, 2).unwrap();
    assert_eq!(q2.panel.2, 2 * q.panel.2);
}

#[test]
fn calendar_dates() {
    assert_eq!(weekday(2026, 11, 13), 4); // Friday
    assert_eq!(weekday(1970, 1, 1), 3); // Thursday
    assert_eq!(weekday(2000, 2, 29), 1); // Tuesday
    assert_eq!(weekday(2024, 1, 1), 0); // Monday
    assert_eq!(days_in_month(2024, 2), 29);
    assert_eq!(days_in_month(1900, 2), 28);
    assert_eq!(days_in_month(2000, 2), 29);
    assert_eq!(days_in_month(2026, 11), 30);
    assert_eq!(shift_month(2026, 11, 2), (2027, 1));
    assert_eq!(shift_month(2026, 1, -1), (2025, 12));
    assert_eq!(shift_month(2026, 5, -17), (2024, 12));
    let c = Calendar::new(1280, 28, 1).unwrap();
    // November 2026 starts on a Sunday: day 1 is the last column of the first row.
    let first = weekday(2026, 11, 1);
    assert_eq!(first, 6);
    let d1 = c.day_cell(first, 1);
    assert_eq!((d1.0, d1.1), (c.grid.0 + 6 * c.cell.0, c.grid.1));
    let d30 = c.day_cell(first, 30);
    assert!(d30.1 + d30.3 <= c.panel.1 + c.panel.3);
    // The Maya footer sits under the sixth week, inside the panel.
    assert!(c.maya.1 >= c.grid.1 + 6 * c.cell.1 && c.maya.1 + c.maya.3 <= c.panel.1 + c.panel.3);
    assert_eq!(c.hit(c.maya.0 + 1, c.maya.1 + 1), CalendarHit::Inside);
    // The oracle band is above the month header, inside the panel.
    assert!(c.oracle.1 >= c.panel.1 && c.oracle.1 + c.oracle.3 <= c.header.1);
    assert_eq!(c.hit(c.oracle.0 + 1, c.oracle.1 + 1), CalendarHit::Inside);
    assert!(c.panel.1 + c.panel.3 < 800);
    assert_eq!(c.hit(c.prev.0 + 1, c.prev.1 + 1), CalendarHit::Prev);
    assert_eq!(c.hit(c.next.0 + 1, c.next.1 + 1), CalendarHit::Next);
    assert_eq!(c.hit(c.header.0 + c.header.2 / 2, c.header.1 + 1), CalendarHit::Today);
    assert_eq!(c.hit(0, 0), CalendarHit::Outside);
}

#[test]
fn text_lines() {
    let mut out = [0u8; 32];
    let s = Status { battery: Some(7), charging: false, on_ac: Some(true), ..Status::UNKNOWN };
    let n = battery_text(&s, &mut out);
    assert_eq!(&out[..n], b"Battery 7%, plugged in");
    let n = battery_text(&Status { battery: Some(100), charging: true, ..Status::UNKNOWN }, &mut out);
    assert_eq!(&out[..n], b"Battery 100%, charging");
    let n = battery_text(&Status::UNKNOWN, &mut out);
    assert_eq!(&out[..n], b"On AC power");
    assert_eq!(wifi_text(10), "Wi-Fi: connected");
    assert_eq!(wifi_text(WIFI_UNKNOWN), "Network: unknown");
    assert_eq!(wifi_text(0), "Network: wired (no Wi-Fi)");
}

#[test]
fn oracle_band_lays_out_the_cross_and_wave_spell_without_overlap() {
    // The wave spell's U: 1-5 down the left, 6-9 along the bottom, 10-12 up the right, 13 beside 12.
    let slots: Vec<_> = (1..=13).map(wave_slot).collect();
    assert_eq!(slots, [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (1, 4), (2, 4), (3, 4), (4, 4), (4, 3), (4, 2), (4, 1), (3, 1)]);
    for scale in 1..=3 {
        let c = Calendar::new(1920 * scale, 28 * scale, scale).unwrap();
        let o = Oracle::new(c.oracle, scale);
        let within = |r: Rect| r.0 >= c.oracle.0 && r.1 >= c.oracle.1 && r.0 + r.2 <= c.oracle.0 + c.oracle.2 && r.1 + r.3 <= c.oracle.1 + c.oracle.3;
        let overlap = |a: Rect, b: Rect| a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
        let mut all: Vec<Rect> = o.cross.to_vec();
        all.extend(o.wave);
        all.extend([o.wave_label, o.kin_label, o.words]);
        for (i, &a) in all.iter().enumerate() {
            assert!(within(a), "scale {scale}: part {i} {a:?} outside {:?}", c.oracle);
            for &b in &all[i + 1..] { assert!(!overlap(a, b), "scale {scale}: {a:?} overlaps {b:?}"); }
        }
        // Guide above destiny above occult; antipode left of it, analog right.
        let d = o.cross[Oracle::DESTINY];
        assert_eq!((o.cross[Oracle::GUIDE].0, o.cross[Oracle::GUIDE].1 + d.3), (d.0, d.1));
        assert_eq!((o.cross[Oracle::OCCULT].0, o.cross[Oracle::OCCULT].1), (d.0, d.1 + d.3));
        assert_eq!(o.cross[Oracle::ANTIPODE].0 + d.2, d.0);
        assert_eq!(o.cross[Oracle::ANALOG].0, d.0 + d.2);
        // The words sit under the cross and the wave spell.
        assert!(o.words.1 >= o.wave[4].1 + o.wave[4].3);
    }
}
