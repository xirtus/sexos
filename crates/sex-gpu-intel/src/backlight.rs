// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! eDP backlight through the south display PWM (first controller). Workstream
//! H2 owns the policy (keys, ACPI `_BCM` first); this is the register path it
//! falls back to when firmware gives no ACPI brightness control.

use crate::regs::*;
use crate::Mmio;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backlight { pub enabled: bool, pub period: u32, pub duty: u32 }
impl Backlight {
    pub fn percent(&self) -> u32 { if self.period == 0 { 0 } else { ((self.duty as u64 * 100 + self.period as u64 / 2) / self.period as u64) as u32 } }
}

pub fn read(m: &mut impl Mmio) -> Backlight {
    Backlight { enabled: m.read(BLC_PWM_CTL) & BLC_PWM_ENABLE != 0, period: m.read(BLC_PWM_FREQ), duty: m.read(BLC_PWM_DUTY) }
}

/// Set brightness 1..=100 % of the firmware-programmed period. Never writes 0
/// so a slip of the keys cannot leave a black, "dead" panel; turning the
/// panel off is the lid/blank path's job.
pub fn set_percent(m: &mut impl Mmio, pct: u32) -> Option<Backlight> {
    let b = read(m);
    if !b.enabled || b.period == 0 { return None; }
    let duty = ((b.period as u64 * pct.clamp(1, 100) as u64) / 100).max(1) as u32;
    m.write(BLC_PWM_DUTY, duty);
    Some(Backlight { duty, ..b })
}
