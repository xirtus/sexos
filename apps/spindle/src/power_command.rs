// SPDX-License-Identifier: GPL-3.0-only
//! AWCC & ACPI power/thermal/fan CLI commands (`power`, `profile [NAME]`, `fan`).
//!
//! Reads live sensors and CPU profile via `sex_pdx::power` (syscall 84, `SENSE|PROFILE`),
//! switches CPU + Alienware Command Center thermal profiles via `CTL_PROFILE` / `CTL_THERMAL_PROFILE`,
//! and surfaces AWCC fan RPMs, temperatures, and available profiles from `sexacpi`.

use crate::sweep_math::{decikelvin_to_celsius_tenths, parse_profile_name};
use crate::Scrollback;
use sex_pdx::power as p;

struct Text {
    buf: [u8; 160],
    len: usize,
}
impl core::fmt::Write for Text {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let take = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
        self.len += take;
        Ok(())
    }
}
macro_rules! say { ($sb:expr, $($arg:tt)*) => {{
    let mut t = Text { buf: [0; 160], len: 0 };
    let _ = core::fmt::Write::write_fmt(&mut t, format_args!($($arg)*));
    $sb.push(&t.buf[..t.len.min(110)]);
    sex_pdx::serial_record!("[spindle.power.line] {}", core::str::from_utf8(&t.buf[..t.len]).unwrap_or("?"));
}}}

fn cpu_profile_name(v: Option<u8>) -> &'static str {
    match v {
        Some(p::PROFILE_PERFORMANCE) => "performance",
        Some(p::PROFILE_BALANCED) => "balanced",
        Some(p::PROFILE_SAVER) => "saver",
        _ => "unknown",
    }
}

/// `power`: shows CPU profile, AC, battery, ACPI temps, and AWCC fans/temps/profile.
pub fn power_command(sb: &mut Scrollback) {
    match p::info() {
        Ok(info) => {
            let hwp = if info.flags & p::HWP != 0 { "hwp" } else if info.flags & p::CPPC != 0 { "cppc" } else { "fixed" };
            let mwait = if info.flags & p::MWAIT_IDLE != 0 { "mwait" } else { "hlt" };
            let aml = if info.flags & p::AML != 0 { "ok" } else if info.flags & p::AML_QUARANTINED != 0 { "quarantined" } else { "none" };
            say!(
                sb,
                "power: cpu={} idle=C{}({}us,{}) aml={} bat={} thermal={} lid={}",
                hwp,
                info.idle_state,
                info.idle_latency_us,
                mwait,
                aml,
                info.batteries,
                info.thermal_zones,
                info.lids
            );
        }
        Err(e) => {
            say!(sb, "power: info unavailable (status={:#x})", e);
        }
    }

    match p::sensors() {
        Ok(s) => {
            let ac = match s.on_ac {
                Some(true) => "AC online",
                Some(false) => "on battery",
                None => "AC unknown",
            };
            let lid = match s.lid_open {
                Some(true) => "open",
                Some(false) => "closed",
                None => "n/a",
            };
            say!(
                sb,
                "  profile: {}  power: {}  lid: {}",
                cpu_profile_name(s.profile),
                ac,
                lid
            );
            if let Some(pct) = s.battery_percent {
                let state = if s.charging { "charging" } else { "discharging" };
                match (s.battery_minutes, s.battery_health) {
                    (Some(min), Some(h)) => say!(sb, "  battery: {}% ({}, {} min, health {}%)", pct, state, min, h),
                    (Some(min), None) => say!(sb, "  battery: {}% ({}, {} min)", pct, state, min),
                    (None, Some(h)) => say!(sb, "  battery: {}% ({}, health {}%)", pct, state, h),
                    (None, None) => say!(sb, "  battery: {}% ({})", pct, state),
                }
            }
            if let Some(dk) = s.hottest_decikelvin {
                let (c, t) = decikelvin_to_celsius_tenths(dk);
                if let Some(crit) = s.critical_decikelvin {
                    let (cc, ct) = decikelvin_to_celsius_tenths(crit as u32);
                    say!(sb, "  thermal: hottest {}.{} C (critical {}.{} C)", c, t, cc, ct);
                } else {
                    say!(sb, "  thermal: hottest {}.{} C", c, t);
                }
            }
            if let Some(b) = s.brightness {
                say!(sb, "  panel:   brightness {}%", b);
            }
        }
        Err(e) => {
            say!(sb, "  sensors: unavailable (status={:#x})", e);
        }
    }

    // Surface latest AWCC fan RPMs, temperatures, and thermal profile from sexacpi.
    let before = sb.total_lines;
    crate::hw_command::newest(b"[sexacpi.awcc]", sb);
    crate::hw_command::newest(b"[sexacpi.awcc.reading]", sb);
    crate::hw_command::newest(b"[sexacpi.awcc.set]", sb);
    if sb.total_lines == before {
        sb.push(b"  awcc:    no Alienware Command Center readings in log yet");
    }
}

/// `profile [NAME]`: queries or switches CPU + AWCC thermal profile.
pub fn profile_command(args: &[u8], sb: &mut Scrollback) {
    let mut trimmed = args;
    while let [b' ' | b'\t', rest @ ..] = trimmed { trimmed = rest; }
    while let [rest @ .., b' ' | b'\t'] = trimmed { trimmed = rest; }

    if trimmed.is_empty() {
        let cur = p::sensors().ok().and_then(|s| s.profile);
        say!(sb, "profile: current cpu={}", cpu_profile_name(cur));
        crate::hw_command::newest(b"[sexacpi.awcc.reading]", sb);
        sb.push(b"Usage: profile quiet|balanced|balanced_performance|performance|cool|low_power|gmode");
        return;
    }

    let Some(spec) = parse_profile_name(trimmed) else {
        sb.push(b"Unknown profile. Valid: quiet, balanced, balanced_performance, performance, cool, low_power, gmode");
        return;
    };

    let cpu_res = p::control(p::CTL_PROFILE, spec.cpu_profile);
    let fw_res = p::control(p::CTL_THERMAL_PROFILE, spec.thermal_kind);
    sex_pdx::serial_record!(
        "[spindle.power.profile] name={} cpu={} cpu_ok={} thermal={} fw_ok={}",
        spec.name,
        spec.cpu_profile,
        cpu_res.is_ok() as u8,
        spec.thermal_kind,
        fw_res.is_ok() as u8
    );
    match (cpu_res, fw_res) {
        (Ok(()), Ok(())) => say!(sb, "profile: set {} (CPU + AWCC firmware queued)", spec.name),
        (Ok(()), Err(e)) => say!(sb, "profile: set CPU {} (AWCC firmware status={:#x})", spec.name, e),
        (Err(e), _) => say!(sb, "profile: failed (status={:#x})", e),
    }
}

/// `fan`: shows AWCC fan inventory and live RPM / temperature readings.
pub fn fan_command(sb: &mut Scrollback) {
    let before = sb.total_lines;
    crate::hw_command::newest(b"[sexacpi.awcc]", sb);
    crate::hw_command::newest(b"[sexacpi.awcc.reading]", sb);
    crate::hw_command::newest(b"[sexacpi.awcc.set]", sb);
    if sb.total_lines == before {
        sb.push(b"fan: no AWCC fan controller detected in boot log.");
    } else {
        sb.push(b"Tip: switch fan curves with `profile quiet|balanced|performance|gmode`");
    }
}
