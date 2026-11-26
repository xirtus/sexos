// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for Spindle's `sweep` and `power`/`profile` math and argument parsers.

#[path = "../apps/spindle/src/sweep_math.rs"]
mod sweep_math;

use sweep_math::*;

#[test]
fn percentiles_empty_and_single() {
    let mut empty: [u64; 0] = [];
    assert_eq!(compute_percentiles(&mut empty), Percentiles::default());

    let mut one = [42u64];
    assert_eq!(
        compute_percentiles(&mut one),
        Percentiles {
            count: 1,
            min: 42,
            p50: 42,
            p95: 42,
            p99: 42,
            max: 42,
            avg: 42,
        }
    );
}

#[test]
fn percentiles_unsorted_hundred_samples() {
    let mut samples: Vec<u64> = (1..=100).rev().collect();
    let p = compute_percentiles(&mut samples);
    assert_eq!(p.count, 100);
    assert_eq!(p.min, 1);
    assert_eq!(p.max, 100);
    // idx(50) = 99*50/100 = 49 -> value 50
    assert_eq!(p.p50, 50);
    // idx(95) = 99*95/100 = 94 -> value 95
    assert_eq!(p.p95, 95);
    // idx(99) = 99*99/100 = 98 -> value 99
    assert_eq!(p.p99, 99);
    // sum(1..=100) = 5050 -> avg = 50
    assert_eq!(p.avg, 50);
}

#[test]
fn cycles_to_ns_conversion_and_overflow_safety() {
    assert_eq!(cycles_to_ns(1000, 0), 0);
    // 2.5 GHz = 2_500_000 cycles/ms -> 2500 cycles = 1000 ns
    assert_eq!(cycles_to_ns(2500, 2_500_000), 1000);
    // 3.0 GHz = 3_000_000 cycles/ms -> 900 cycles = 300 ns
    assert_eq!(cycles_to_ns(900, 3_000_000), 300);
    // Large cycle count (10 seconds at 3 GHz) does not overflow
    assert_eq!(cycles_to_ns(30_000_000_000, 3_000_000), 10_000_000_000);
}

#[test]
fn throughput_and_temperature_conversions() {
    assert_eq!(throughput_kib_s(1024 * 1024, 0), 0);
    assert_eq!(throughput_kib_s(1024 * 1024, 1000), 1024);
    assert_eq!(throughput_mib_tenths(1024 * 1024, 1000), (1, 0));
    assert_eq!(throughput_mib_tenths(15 * 1024 * 1024, 100), (150, 0));
    assert_eq!(throughput_mib_tenths(125 * 1024 * 1024, 1000), (125, 0));
    assert_eq!(throughput_mib_tenths(1536 * 1024, 1000), (1, 5));

    assert_eq!(decikelvin_to_celsius_tenths(2732), (0, 0));
    assert_eq!(decikelvin_to_celsius_tenths(3132), (40, 0));
    assert_eq!(decikelvin_to_celsius_tenths(3587), (85, 5));
    assert_eq!(decikelvin_to_celsius_tenths(2682), (-5, 0));
}

#[test]
fn sweep_arg_parsing() {
    assert_eq!(parse_sweep_args(b"sched"), Some(SweepKind::Sched(DEFAULT_ITERS)));
    assert_eq!(parse_sweep_args(b"  SCHED  1000 "), Some(SweepKind::Sched(1000)));
    assert_eq!(parse_sweep_args(b"ipc"), Some(SweepKind::Ipc(DEFAULT_ITERS)));
    assert_eq!(parse_sweep_args(b"ipc 1"), Some(SweepKind::Ipc(1)));
    assert_eq!(parse_sweep_args(b"ipc 10000"), Some(SweepKind::Ipc(10000)));
    assert_eq!(parse_sweep_args(b"nvme"), Some(SweepKind::Nvme(None)));
    assert_eq!(
        parse_sweep_args(b"nvme /media/Ventoy/sexos.iso"),
        Some(SweepKind::Nvme(Some(b"/media/Ventoy/sexos.iso")))
    );
    assert_eq!(parse_sweep_args(b"all"), Some(SweepKind::All(None)));
    assert_eq!(
        parse_sweep_args(b"all /media/windows/bootmgr"),
        Some(SweepKind::All(Some(b"/media/windows/bootmgr")))
    );

    // Invalid inputs
    assert_eq!(parse_sweep_args(b""), None);
    assert_eq!(parse_sweep_args(b"   "), None);
    assert_eq!(parse_sweep_args(b"sched 0"), None);
    assert_eq!(parse_sweep_args(b"sched 10001"), None);
    assert_eq!(parse_sweep_args(b"ipc abc"), None);
    assert_eq!(parse_sweep_args(b"sched 10 extra"), None);
    assert_eq!(parse_sweep_args(b"unknown"), None);
}

#[test]
fn profile_name_parsing() {
    assert_eq!(
        parse_profile_name(b"quiet"),
        Some(ProfileSpec { name: "quiet", cpu_profile: 2, thermal_kind: 1 })
    );
    assert_eq!(
        parse_profile_name(b"  BALANCED "),
        Some(ProfileSpec { name: "balanced", cpu_profile: 1, thermal_kind: 2 })
    );
    assert_eq!(
        parse_profile_name(b"balanced_performance"),
        Some(ProfileSpec { name: "balanced_performance", cpu_profile: 0, thermal_kind: 3 })
    );
    assert_eq!(
        parse_profile_name(b"balanced-performance"),
        Some(ProfileSpec { name: "balanced_performance", cpu_profile: 0, thermal_kind: 3 })
    );
    assert_eq!(
        parse_profile_name(b"performance"),
        Some(ProfileSpec { name: "performance", cpu_profile: 0, thermal_kind: 4 })
    );
    assert_eq!(
        parse_profile_name(b"perf"),
        Some(ProfileSpec { name: "performance", cpu_profile: 0, thermal_kind: 4 })
    );
    assert_eq!(
        parse_profile_name(b"low_power"),
        Some(ProfileSpec { name: "low_power", cpu_profile: 2, thermal_kind: 6 })
    );
    assert_eq!(
        parse_profile_name(b"saver"),
        Some(ProfileSpec { name: "low_power", cpu_profile: 2, thermal_kind: 6 })
    );
    assert_eq!(
        parse_profile_name(b"cool"),
        Some(ProfileSpec { name: "cool", cpu_profile: 1, thermal_kind: 5 })
    );
    assert_eq!(
        parse_profile_name(b"gmode"),
        Some(ProfileSpec { name: "gmode", cpu_profile: 0, thermal_kind: 7 })
    );
    assert_eq!(
        parse_profile_name(b"custom"),
        Some(ProfileSpec { name: "custom", cpu_profile: 1, thermal_kind: 0 })
    );

    assert_eq!(parse_profile_name(b""), None);
    assert_eq!(parse_profile_name(b"turbo"), None);
    assert_eq!(parse_profile_name(b"quiet extra"), None);
}
