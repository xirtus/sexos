// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Screen sharing's wire, picker layout, bank and Shell custody:
//! `cargo test -p sex-pdx --test screen_share`.
use sex_pdx::screen_share::*;


#[test]
fn views_round_trip_and_bad_ones_fail() {
    for v in [View::Hidden, View::Picking { asker: 358 }, View::Sharing { kind: SCREEN, surface: 0, target: 41, session: 3 },
        View::Sharing { kind: WINDOW, surface: 300, target: 42, session: 9 }] {
        assert_eq!(View::decode(v.wire()), Some(v));
    }
    assert_eq!(View::decode([3, 0, 41 << 32 | 1]), None, "a window share names its window");
    assert_eq!(View::decode([2, 0, 1]), None, "a share names its program");
    assert_eq!(View::decode([4, 1, 1]), None);
    assert_eq!(View::decode([0, 1, 0]), None);
}

#[test]
fn picker_and_stop_pill_fit_the_screen() {
    let p = Picker::new(1280, 30, 1).unwrap();
    assert!(p.pill.0 + p.pill.2 <= 1280 && p.pill.1 >= 30);
    // The pill stays above the overview's first row (32 px margin under the bar).
    assert!(p.pill.1 + p.pill.3 <= 30 + 32);
    assert!(inside(p.pill, p.button.0, p.button.1) && inside(p.pill, p.button.0 + p.button.2 - 1, p.button.1 + p.button.3 - 1));
    assert!(p.on_button(p.button.0 + 5, p.button.1 + 5) && !p.on_button(p.pill.0 + 5, p.pill.1 + 5));
    let s = Picker::new(2560, 60, 2).unwrap();
    assert_eq!(s.pill.2, 1280);
    assert!(Picker::new(100, 30, 1).is_none());
    let (x, y, w, h) = stop_pill(1280, 800, 1);
    assert!(x + w <= 1280 && y + h < 800 && w == 300);
}

#[test]
fn fit_keeps_the_shape() {
    assert_eq!(fit(800, 500), (800, 500));
    assert_eq!(fit(800, 600), (720, 540));
    assert_eq!(fit(1920, 1080), (960, 540));
    assert_eq!(fit(1280, 800), (864, 540));
    assert_eq!(fit(3000, 500), (960, 160));
    assert_eq!(fit(0, 10), (0, 0));
}

#[test]
fn bank_publishes_reads_and_forgets() {
    let mut mem = vec![0u32; BYTES as usize / 4];
    let bank = unsafe { Bank::at(mem.as_mut_ptr() as *mut u8) };
    let mut out = vec![0u32; MAX_PIXELS];
    assert!(bank.info().is_none() && bank.read(0, &mut out).is_none());
    assert!(bank.publish(4, 4, |_| {}).is_none(), "nothing is written before a share");
    bank.start(41, WINDOW, 7, b"Notes - Quil");
    let info = bank.info().unwrap();
    assert_eq!((info.target, info.kind, info.session, info.title()), (41, WINDOW, 7, &b"Notes - Quil"[..]));
    let s1 = bank.publish(3, 2, |px| for (i, p) in px.iter_mut().enumerate() { *p = i as u32 + 1 }).unwrap();
    let f = bank.read(0, &mut out).unwrap();
    assert_eq!((f.seq, f.width, f.height), (s1, 3, 2));
    assert_eq!(&out[..6], &[1, 2, 3, 4, 5, 6]);
    assert!(bank.read(f.seq, &mut out).is_none(), "no new picture");
    let s2 = bank.publish(2, 2, |px| px.fill(9)).unwrap();
    assert!(s2 > s1);
    let f = bank.read(s1, &mut out).unwrap();
    assert_eq!((f.width, f.height, out[0]), (2, 2, 9));
    assert!(bank.read(0, &mut out[..3]).is_none(), "too small to take it");
    assert!(bank.publish(MAX_W + 1, 1, |_| {}).is_none());
    bank.stop();
    assert!(bank.info().is_none() && bank.read(0, &mut out).is_none());
    assert!(mem[HEADER / 4..].iter().all(|&p| p == 0), "the pictures are wiped");
}

#[test]
fn custody_asks_picks_and_stops() {
    let (a, b) = ((41u32, 1u32), (42u32, 1u32));
    let mut c = Custody::new();
    assert_eq!(c.ask(a, false), (false, Some((a, [DENIED, 0, 0]))), "only right after a key or click");
    assert_eq!(c.ask(a, true), (true, None));
    assert_eq!(c.ask(b, true), (false, Some((b, [BUSY, 0, 0]))));
    assert_eq!(c.ask(a, true), (true, None), "asking again keeps the picker");
    let (reply, view) = c.picked(None, 41);
    assert_eq!((reply, view), (Some((a, [DENIED, 0, 0])), Some(View::Hidden)));
    assert_eq!(c.picked(Some((SCREEN, 0)), 41), (None, None), "no picker, no share");
    c.ask(a, true);
    let (reply, view) = c.picked(Some((WINDOW, 300)), 41);
    assert_eq!(reply, Some((a, [STARTED, WINDOW, 1])));
    assert_eq!(view, Some(View::Sharing { kind: WINDOW, surface: 300, target: 41, session: 1 }));
    assert_eq!(c.stop(true), (Some((a, [STOPPED, WINDOW, 1])), Some(View::Hidden)));
    assert_eq!(c.stop(true), (None, None));
    c.ask(b, true);
    let (reply, _) = c.picked(Some((SCREEN, 9)), 42);
    assert_eq!(reply, Some((b, [STARTED, SCREEN, 2])));
    assert_eq!(c.sharing.map(|s| s.surface), Some(0), "the screen names no window");
    // Another program may not pick while one shares; the sharer may pick anew.
    assert_eq!(c.ask(a, true), (false, Some((a, [BUSY, 0, 0]))));
    assert_eq!(c.ask(b, true), (true, None));
    assert_eq!(c.picked(None, 42), (Some((b, [DENIED, 0, 0])), Some(View::Sharing { kind: SCREEN, surface: 0, target: 42, session: 2 })), "saying no keeps the share on");
    assert_eq!(c.stop(false), (None, Some(View::Hidden)));
}
