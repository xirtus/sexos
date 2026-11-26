// SPDX-License-Identifier: GPL-3.0-only
//! Pure, host-testable math and argument parsers for Spindle's `sweep` and `power`/`profile` commands.
//! Zero static buffers, zero OS dependencies.

pub const DEFAULT_ITERS: usize = 64;
pub const MAX_ITERS: usize = 10_000;

/// Summary statistics for a slice of cycle/latency samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Percentiles {
    pub count: usize,
    pub min: u64,
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
    pub avg: u64,
}

/// Computes `min`, `p50`, `p95`, `p99`, `max`, and `avg` in place by sorting `samples`.
/// Uses index `((n - 1) * pct) / 100` for percentiles.
pub fn compute_percentiles(samples: &mut [u64]) -> Percentiles {
    let n = samples.len();
    if n == 0 {
        return Percentiles::default();
    }
    samples.sort_unstable();
    let mut sum: u128 = 0;
    for &s in samples.iter() {
        sum += s as u128;
    }
    let idx = |pct: usize| ((n - 1) * pct) / 100;
    Percentiles {
        count: n,
        min: samples[0],
        p50: samples[idx(50)],
        p95: samples[idx(95)],
        p99: samples[idx(99)],
        max: samples[n - 1],
        avg: (sum / n as u128) as u64,
    }
}

/// Converts TSC `cycles` to nanoseconds given `khz` (cycles per millisecond).
/// Returns 0 when `khz == 0`.
pub fn cycles_to_ns(cycles: u64, khz: u64) -> u64 {
    if khz == 0 {
        return 0;
    }
    ((cycles as u128 * 1_000_000) / (khz as u128)) as u64
}

/// Computes throughput in KiB/s from `bytes` transferred in `elapsed_ms` milliseconds.
pub fn throughput_kib_s(bytes: u64, elapsed_ms: u64) -> u64 {
    if elapsed_ms == 0 {
        return 0;
    }
    ((bytes as u128 * 1000) / (1024 * elapsed_ms as u128)) as u64
}

/// Computes throughput as `(whole_mib_s, tenths_mib_s)` without floating point.
pub fn throughput_mib_tenths(bytes: u64, elapsed_ms: u64) -> (u64, u64) {
    if elapsed_ms == 0 {
        return (0, 0);
    }
    let tenths = ((bytes as u128 * 10_000) / (1024 * 1024 * elapsed_ms as u128)) as u64;
    (tenths / 10, tenths % 10)
}

/// Converts ACPI decikelvin (tenths of a kelvin, 0 °C = 273.15 K ≈ 2732 dK) to `(whole_c, tenths_c)`.
pub fn decikelvin_to_celsius_tenths(dk: u32) -> (i32, u32) {
    let diff = dk as i32 - 2732;
    let abs = diff.unsigned_abs();
    (diff / 10, abs % 10)
}

/// Parsed `sweep` subcommand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SweepKind<'a> {
    Sched(usize),
    Ipc(usize),
    Nvme(Option<&'a [u8]>),
    All(Option<&'a [u8]>),
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = s {
        s = rest;
    }
    while let [rest @ .., b' ' | b'\t'] = s {
        s = rest;
    }
    s
}

fn parse_iters(word: &[u8]) -> Option<usize> {
    let s = core::str::from_utf8(word).ok()?;
    let n = s.parse::<usize>().ok()?;
    if (1..=MAX_ITERS).contains(&n) {
        Some(n)
    } else {
        None
    }
}

/// Parses arguments to `sweep`: `sched [N]`, `ipc [N]`, `nvme [PATH]`, or `all [PATH]`.
pub fn parse_sweep_args(args: &[u8]) -> Option<SweepKind<'_>> {
    let args = trim(args);
    if args.is_empty() {
        return None;
    }
    let mut words = args.split(|b| b.is_ascii_whitespace()).filter(|w| !w.is_empty());
    let sub = words.next()?;
    let arg1 = words.next();
    if words.next().is_some() {
        return None;
    }
    if sub.eq_ignore_ascii_case(b"sched") {
        let n = match arg1 {
            None => DEFAULT_ITERS,
            Some(w) => parse_iters(w)?,
        };
        Some(SweepKind::Sched(n))
    } else if sub.eq_ignore_ascii_case(b"ipc") {
        let n = match arg1 {
            None => DEFAULT_ITERS,
            Some(w) => parse_iters(w)?,
        };
        Some(SweepKind::Ipc(n))
    } else if sub.eq_ignore_ascii_case(b"nvme") {
        Some(SweepKind::Nvme(arg1))
    } else if sub.eq_ignore_ascii_case(b"all") {
        Some(SweepKind::All(arg1))
    } else {
        None
    }
}

/// Canonical profile mapping from CLI name to `(CPU HWP/CPPC profile, AWCC thermal profile)`.
/// Constants match `sex_pdx::power::{PROFILE_*, THERMAL_*}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileSpec {
    pub name: &'static str,
    pub cpu_profile: u8,
    pub thermal_kind: u8,
}

/// Parses a power/AWCC thermal profile name (case-insensitive).
pub fn parse_profile_name(arg: &[u8]) -> Option<ProfileSpec> {
    let w = trim(arg);
    if w.is_empty() {
        return None;
    }
    if w.eq_ignore_ascii_case(b"quiet") {
        Some(ProfileSpec { name: "quiet", cpu_profile: 2, thermal_kind: 1 })
    } else if w.eq_ignore_ascii_case(b"balanced") || w.eq_ignore_ascii_case(b"bal") {
        Some(ProfileSpec { name: "balanced", cpu_profile: 1, thermal_kind: 2 })
    } else if w.eq_ignore_ascii_case(b"balanced_performance")
        || w.eq_ignore_ascii_case(b"balanced-performance")
        || w.eq_ignore_ascii_case(b"bal_perf")
    {
        Some(ProfileSpec { name: "balanced_performance", cpu_profile: 0, thermal_kind: 3 })
    } else if w.eq_ignore_ascii_case(b"performance") || w.eq_ignore_ascii_case(b"perf") {
        Some(ProfileSpec { name: "performance", cpu_profile: 0, thermal_kind: 4 })
    } else if w.eq_ignore_ascii_case(b"cool") {
        Some(ProfileSpec { name: "cool", cpu_profile: 1, thermal_kind: 5 })
    } else if w.eq_ignore_ascii_case(b"low_power")
        || w.eq_ignore_ascii_case(b"low-power")
        || w.eq_ignore_ascii_case(b"saver")
        || w.eq_ignore_ascii_case(b"powersave")
    {
        Some(ProfileSpec { name: "low_power", cpu_profile: 2, thermal_kind: 6 })
    } else if w.eq_ignore_ascii_case(b"gmode") || w.eq_ignore_ascii_case(b"g-mode") {
        Some(ProfileSpec { name: "gmode", cpu_profile: 0, thermal_kind: 7 })
    } else if w.eq_ignore_ascii_case(b"custom") {
        Some(ProfileSpec { name: "custom", cpu_profile: 1, thermal_kind: 0 })
    } else {
        None
    }
}
