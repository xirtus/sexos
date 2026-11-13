// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Hotplug live status: is a monitor attached to a port right now. Combo
//! ports report in the south display ISR; Type-C ports (USB-C/Thunderbolt DP)
//! in the north DE HPD ISR, as either DP-alt or Thunderbolt.

use crate::regs::*;
use crate::Mmio;

pub fn live(m: &mut impl Mmio, port: u32) -> bool {
    if is_type_c(port) {
        let north = m.read(DE_HPD_ISR);
        north & (tc_hotplug_bit(port) | tbt_hotplug_bit(port)) != 0 || m.read(SDEISR) & sde_hotplug_bit(port) != 0
    } else {
        m.read(SDEISR) & sde_hotplug_bit(port) != 0
    }
}

/// Bit `i` set when `PORTS[i]` has a monitor.
pub fn scan(m: &mut impl Mmio) -> u32 {
    PORTS.iter().enumerate().fold(0, |acc, (i, &p)| if live(m, p) { acc | 1 << i } else { acc })
}

/// Ports whose state changed between two scans: (arrived, left).
pub fn diff(old: u32, new: u32) -> (u32, u32) { (new & !old, old & !new) }
