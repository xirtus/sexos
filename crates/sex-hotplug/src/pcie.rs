// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! PCI Express capability registers of a hot-plug port (PCIe base spec 7.5.3).

/// Capability id of the PCI Express capability.
pub const CAP_ID: u8 = 0x10;

/// Offsets inside the PCI Express capability.
pub const CAPS: u8 = 0x02;
pub const LINK_CAPS: u8 = 0x0c;
pub const LINK_STATUS: u8 = 0x12;
pub const SLOT_CAPS: u8 = 0x14;
pub const SLOT_CONTROL: u8 = 0x18;
pub const SLOT_STATUS: u8 = 0x1a;

/// PCIe capabilities register: slot implemented, device/port type.
pub const SLOT_IMPLEMENTED: u16 = 1 << 8;
pub const TYPE_ROOT_PORT: u8 = 4;
pub const TYPE_DOWNSTREAM: u8 = 6;

/// Link capabilities: the port reports Data Link Layer Link Active.
pub const LINK_ACTIVE_REPORTING: u32 = 1 << 20;
/// Link status: Data Link Layer Link Active.
pub const LINK_ACTIVE: u16 = 1 << 13;

/// Slot control.
pub mod control {
    pub const BUTTON_ENABLE: u16 = 1 << 0;
    pub const POWER_FAULT_ENABLE: u16 = 1 << 1;
    pub const MRL_ENABLE: u16 = 1 << 2;
    pub const PRESENCE_ENABLE: u16 = 1 << 3;
    pub const COMMAND_ENABLE: u16 = 1 << 4;
    pub const HOTPLUG_IRQ_ENABLE: u16 = 1 << 5;
    pub const ATTENTION_MASK: u16 = 3 << 6;
    pub const POWER_LED_MASK: u16 = 3 << 8;
    /// Power controller control: 1 = off.
    pub const POWER_OFF: u16 = 1 << 10;
    pub const LINK_CHANGE_ENABLE: u16 = 1 << 12;
    /// Indicator values (attention << 6, power << 8).
    pub const LED_ON: u16 = 1;
    pub const LED_BLINK: u16 = 2;
    pub const LED_OFF: u16 = 3;
}

/// Slot status. Bits 0..4 and 8 are write-one-to-clear.
pub mod status {
    pub const BUTTON: u16 = 1 << 0;
    pub const POWER_FAULT: u16 = 1 << 1;
    pub const MRL_CHANGED: u16 = 1 << 2;
    pub const PRESENCE_CHANGED: u16 = 1 << 3;
    pub const COMMAND_DONE: u16 = 1 << 4;
    pub const MRL_OPEN: u16 = 1 << 5;
    pub const PRESENT: u16 = 1 << 6;
    pub const LINK_CHANGED: u16 = 1 << 8;
    /// Every event bit: written back to clear them.
    pub const EVENTS: u16 = BUTTON | POWER_FAULT | MRL_CHANGED | PRESENCE_CHANGED | COMMAND_DONE | LINK_CHANGED;
}

/// What a slot can do (slot capabilities register).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotCaps {
    pub button: bool,
    pub power_controller: bool,
    pub mrl: bool,
    pub attention_led: bool,
    pub power_led: bool,
    pub surprise: bool,
    pub hotplug: bool,
    pub no_command_completed: bool,
    pub number: u16,
}

impl SlotCaps {
    pub fn decode(raw: u32) -> SlotCaps {
        SlotCaps {
            button: raw & 1 != 0,
            power_controller: raw & 1 << 1 != 0,
            mrl: raw & 1 << 2 != 0,
            attention_led: raw & 1 << 3 != 0,
            power_led: raw & 1 << 4 != 0,
            surprise: raw & 1 << 5 != 0,
            hotplug: raw & 1 << 6 != 0,
            no_command_completed: raw & 1 << 18 != 0,
            number: (raw >> 19) as u16,
        }
    }
}

/// A port that can take devices at runtime: a root port or a switch's
/// downstream port with a slot that says it is hot-plug capable.
pub fn is_hotplug_port(caps: u16, slot_caps: u32) -> bool {
    let kind = ((caps >> 4) & 0xf) as u8;
    matches!(kind, TYPE_ROOT_PORT | TYPE_DOWNSTREAM) && caps & SLOT_IMPLEMENTED != 0 && SlotCaps::decode(slot_caps).hotplug
}

/// Slot control with indicators and power set. `None` leaves a field alone.
pub fn control(current: u16, power_on: Option<bool>, power_led: Option<u16>, attention_led: Option<u16>) -> u16 {
    let mut v = current;
    if let Some(on) = power_on { v = if on { v & !control::POWER_OFF } else { v | control::POWER_OFF }; }
    if let Some(led) = power_led { v = (v & !control::POWER_LED_MASK) | (led & 3) << 8; }
    if let Some(led) = attention_led { v = (v & !control::ATTENTION_MASK) | (led & 3) << 6; }
    v
}

/// The hot-plug interrupt sources are left off: the kernel looks at the slot
/// on a slow tick instead (a few config reads), so a port without working MSI
/// still hot-plugs.
pub fn quiet_control(current: u16) -> u16 {
    use control::*;
    current & !(BUTTON_ENABLE | POWER_FAULT_ENABLE | MRL_ENABLE | PRESENCE_ENABLE | COMMAND_ENABLE | HOTPLUG_IRQ_ENABLE | LINK_CHANGE_ENABLE)
}
