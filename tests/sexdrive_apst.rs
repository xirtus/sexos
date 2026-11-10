// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! APST table from Identify Controller data, with Linux's default policy.
#[path="../apps/sexdrive/src/apst.rs"]mod apst;
use apst::*;

fn id(npss: u8, apsta: bool, states: &[(bool, u32, u32)]) -> Vec<u8> {
    let mut b = vec![0u8; 4096];
    b[ID_NPSS] = npss; b[ID_APSTA] = apsta as u8;
    for (i, &(nop, enlat, exlat)) in states.iter().enumerate() {
        let d = ID_PSD + i * PSD_BYTES;
        b[d + 3] = if nop { 2 } else { 0 };
        b[d + 4..d + 8].copy_from_slice(&enlat.to_le_bytes());
        b[d + 8..d + 12].copy_from_slice(&exlat.to_le_bytes());
    }
    b
}

// A Samsung 980 PRO-like table: PS0-2 operational, PS3 (enlat 2000, exlat 1200), PS4 (500, 9500).
fn samsung() -> Vec<u8> { id(4, true, &[(false,0,0),(false,0,0),(false,0,0),(true,2000,1200),(true,500,9500)]) }

#[test]fn laptop_ssd_gets_both_low_power_states(){
    let (s, n) = parse(&samsung()).unwrap();
    let p = plan(&s, n, false).unwrap();
    // PS4 total 10 ms <= 15 ms: 100 ms idle. PS3 total 3.2 ms: also 100 ms. PS0-2 go to PS3 (the shallowest usable).
    let to = |state: u64, ms: u64| (state << 3) | (ms << 8);
    assert_eq!(p.table[4], 0);
    assert_eq!(p.table[3], to(4, 100));
    for i in 0..3 { assert_eq!(p.table[i], to(3, 100)); }
    assert_eq!((p.deepest, p.worst_latency_us), (4, 10_000));
}
#[test]fn slow_deep_state_waits_two_seconds_and_too_slow_is_skipped(){
    let (s, n) = parse(&id(3, true, &[(false,0,0),(true,5000,20000),(true,1000,200000),(true,10,10)])).unwrap();
    // PS3 (20 us) tier 0; PS2 exit 200 ms > ceiling skipped; PS1 total 25 ms -> tier 1, 2 s.
    let p = plan(&s, n, false).unwrap();
    assert_eq!(p.table[0], (1 << 3) | (2000 << 8));
    assert_eq!(p.table[1], 3 << 3 | 100 << 8);
    assert_eq!(p.table[2], 3 << 3 | 100 << 8);
    assert_eq!(p.deepest, 3);
}
#[test]fn quirk_skips_the_deepest_state(){
    let (s, n) = parse(&samsung()).unwrap();
    let p = plan(&s, n, true).unwrap();
    assert_eq!(p.deepest, 3);
    assert_eq!(p.table[3], 0);
    assert!(skip_deepest(0x144d, 0xa802) && !skip_deepest(0x144d, 0xa80a));
}
#[test]fn no_apst_support_or_no_idle_state_means_no_table(){
    assert!(parse(&id(4, false, &[])).is_none());
    let (s, n) = parse(&id(2, true, &[(false,0,0),(false,0,0),(false,0,0)])).unwrap();
    assert!(plan(&s, n, false).is_none());
    assert!(parse(&[0u8; 100]).is_none());
}
#[test]fn expensive_deep_state_does_not_hide_shallow_ones(){
    // PS4 is 60+60 ms (over the 100 ms ceiling); PS3 is 2 ms and must still be used.
    let mut s=[PowerState{non_operational:false,entry_us:0,exit_us:0};ENTRIES];
    s[3]=PowerState{non_operational:true,entry_us:1_000,exit_us:1_000};
    s[4]=PowerState{non_operational:true,entry_us:60_000,exit_us:60_000};
    let p=plan(&s,4,false).expect("PS3 is usable");
    assert_eq!(p.deepest,3);assert_eq!(p.table[0],(3<<3)|(100<<8));assert_eq!(p.table[3],0);assert_eq!(p.table[4],0);
}
