// SPDX-License-Identifier: GPL-3.0-only
use silkbar_model::civil_clock::{Applied, CivilClock, Reject, State, Update};
use silkbar_model::*;

#[test]
fn test_contract_and_invariants() {
    assert!(validate_invariants(), "validate_invariants must pass");
    assert_eq!(validate_silkbar_contract(), 0, "validate_silkbar_contract must return 0");
    assert!(validate_contract(), "validate_contract must return true");
    assert!(validate_deterministic_vectors(), "validate_deterministic_vectors must pass");
    let fp = contract_fingerprint();
    assert_ne!(fp, 0, "contract_fingerprint must be non-zero");
    assert_eq!(fp, contract_fingerprint(), "contract_fingerprint must be deterministic");
}

#[test]
fn test_desktop_width_geometry() {
    // Normal 1280 width desktop
    let bar_1280 = for_desktop_width(1280).expect("1280 width should be supported");
    assert_eq!(bar_1280.workspaces.len(), 9);
    assert_eq!(bar_1280.layout.len(), LAYOUT_COUNT);

    // Compact 1024 width desktop
    let bar_1024 = for_desktop_width(1024).expect("1024 width should be supported");
    assert_eq!(bar_1024.workspaces.len(), 9);

    // Too narrow
    assert!(for_desktop_width(300).is_none(), "under-minimum width should return None");
}

#[test]
fn test_hit_testing() {
    let bar = for_desktop_width(1280).unwrap();

    // Hit outside panel
    assert!(hit_test_action(&bar, 0, 0) == Action::None);
    assert!(hit_test_action(&bar, 5000, 5000) == Action::None);

    // Hit launcher
    let act_launcher = hit_test_action(&bar, LAUNCHER_X + 2, LAUNCHER_Y + 2);
    assert!(act_launcher == Action::OpenLauncher);

    // Hit workspace 0 -> Action::SwitchWorkspace(1)
    let act_ws0 = hit_test_action(&bar, WS_X0 + 2, WS_Y + 2);
    assert!(act_ws0 == Action::SwitchWorkspace(1));

    // Hit workspace 8 -> Action::SwitchWorkspace(9)
    let act_ws8 = hit_test_action(&bar, WS_X8 + 2, WS_Y + 2);
    assert!(act_ws8 == Action::SwitchWorkspace(9));

    // Hit clock
    let act_clock = hit_test_action(&bar, CHIP_X3 + 5, CHIP_Y + 5);
    assert!(act_clock == Action::OpenClock);
}

#[test]
fn test_color_lighten_darken() {
    let c = 0x808080;
    let l = lighten(c);
    let d = darken(c);
    assert!(l > c, "lighten must increase channel values");
    assert!(d < c, "darken must decrease channel values");

    // Extreme bounds: clamping must not panic
    let _ = lighten(0xFFFFFF);
    let _ = darken(0x000000);
}

#[test]
fn test_updates_and_queue() {
    let mut bar = for_desktop_width(1280).unwrap();

    // Set workspace active
    let ok = apply_raw_update(&mut bar, UpdateKind::SetWorkspaceActive as u32, 4, 1, 0);
    assert!(ok, "apply valid workspace active update");
    assert!(bar.workspaces[4].active);

    // Update urgency
    let ok_urg = apply_raw_update(&mut bar, UpdateKind::SetWorkspaceUrgent as u32, 4, 1, 0);
    assert!(ok_urg);
    assert!(bar.workspaces[4].urgent);

    // Invalid workspace index
    let bad = apply_raw_update(&mut bar, UpdateKind::SetWorkspaceActive as u32, 20, 1, 0);
    assert!(!bad, "out of bounds workspace index must be rejected");

    // Queue test
    let mut queue = SilkBarUpdateQueue::empty();
    assert!(queue.pop().is_none());

    let u = SilkBarUpdate::new(UpdateKind::SetWorkspaceActive as u32, 3, 1, 0);
    assert!(queue.push(u));

    let popped = queue.pop().expect("pop should return pushed update");
    assert_eq!(popped.kind, UpdateKind::SetWorkspaceActive as u32);
    assert_eq!(popped.index, 3);
    assert!(queue.pop().is_none());
}

#[test]
fn test_civil_clock_state_machine() {
    let mut clock = CivilClock::new();
    assert_eq!(clock.state(), State::Unknown);
    assert_eq!(clock.seconds(), 0);
    assert_eq!(clock.sequence(), 0);

    let caller = (1, 100);
    let expected = (1, 100);

    // Wrong publisher
    let bad_caller = (2, 200);
    let u1 = Update { state: State::Snapshot, seconds: 12345, sequence: 10 };
    let res = clock.receive(bad_caller, expected, [11, (1 << 36) | (12345), 10], 10000, 20000, 10);
    assert_eq!(res, Err(Reject::Publisher));

    // Valid update
    let enc = u1.encode();
    let res_ok = clock.receive(caller, expected, [enc.kind as u64, ((enc.index as u64) << 32) | enc.a as u64, enc.b as u64], 10000, 20000, 10);
    assert_eq!(res_ok, Ok(Applied::New));
    assert_eq!(clock.state(), State::Snapshot);
    assert_eq!(clock.seconds(), 12345);
    assert_eq!(clock.sequence(), 10);

    // Duplicate update (idempotent)
    let res_dup = clock.receive(caller, expected, [enc.kind as u64, ((enc.index as u64) << 32) | enc.a as u64, enc.b as u64], 10000, 20000, 15);
    assert_eq!(res_dup, Ok(Applied::Duplicate));

    // Stale sequence (sequence 5 < current sequence 10)
    let u_stale = Update { state: State::Snapshot, seconds: 12345, sequence: 5 };
    let enc_stale = u_stale.encode();
    let res_stale = clock.receive(caller, expected, [enc_stale.kind as u64, ((enc_stale.index as u64) << 32) | enc_stale.a as u64, enc_stale.b as u64], 10000, 20000, 20);
    assert_eq!(res_stale, Err(Reject::Stale));

    // Rollback detection (seconds decrease below high water)
    let u_back = Update { state: State::Snapshot, seconds: 12000, sequence: 15 };
    let enc_back = u_back.encode();
    let res_back = clock.receive(caller, expected, [enc_back.kind as u64, ((enc_back.index as u64) << 32) | enc_back.a as u64, enc_back.b as u64], 10000, 20000, 25);
    assert_eq!(res_back, Ok(Applied::New));
    assert_eq!(clock.state(), State::Rollback);

    // Terminal state: Rollback rejects further updates
    let u_next = Update { state: State::Snapshot, seconds: 15000, sequence: 20 };
    let enc_next = u_next.encode();
    let res_term = clock.receive(caller, expected, [enc_next.kind as u64, ((enc_next.index as u64) << 32) | enc_next.a as u64, enc_next.b as u64], 10000, 20000, 30);
    assert_eq!(res_term, Err(Reject::Terminal));
}
