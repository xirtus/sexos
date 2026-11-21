// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! One hot-plug slot's life, fed with what its registers say on each look.
//!
//! The rules follow the PCIe hot-plug model that Thunderbolt docks, eGPU
//! enclosures and server slots all share:
//! - a card that appears is believed after it stays present for `SETTLE_MS`;
//! - a slot with a power controller is powered then, and the link must be up
//!   for `LINK_MS` (the spec's 100 ms) before anything below is touched;
//! - the attention button starts a `BUTTON_MS` (5 s) window: a second press
//!   cancels, otherwise the devices are let go and the slot powered off;
//! - presence or link lost while up is a surprise removal: let go at once.

pub const SETTLE_MS: u64 = 200;
pub const LINK_MS: u64 = 100;
pub const LINK_TIMEOUT_MS: u64 = 1_000;
pub const BUTTON_MS: u64 = 5_000;

/// What the slot's registers say right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Look {
    pub present: bool,
    /// Data Link Layer Link Active; a port without that report passes `present`.
    pub link: bool,
    /// The attention button status bit was set since the last look.
    pub button: bool,
    /// Slot power is on (always true without a power controller).
    pub powered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Empty,
    Settling { since: u64 },
    Linking { since: u64 },
    LinkUp { since: u64 },
    Up,
    Button { since: u64 },
    /// Let go by the button and powered off; the card may still be in.
    Off,
    /// Present but the link never came up.
    Failed,
}

/// What the caller should do now. Power and indicators first, then events.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// Some(true): power the slot on; Some(false): off.
    pub power: Option<bool>,
    /// New power indicator value (`pcie::control::LED_*`).
    pub led: Option<u16>,
    /// Devices below are ready to enumerate.
    pub arrived: bool,
    /// Devices below are gone (or about to be): revoke and forget them.
    pub left: bool,
    /// The link never came up.
    pub failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub state: State,
    pub power_controller: bool,
}

impl Slot {
    /// A slot as BOOT found it: devices already below it were enumerated by BOOT.
    pub fn at_boot(look: Look, power_controller: bool) -> Slot {
        let state = if look.present && look.link { State::Up } else if look.present && !look.powered { State::Off } else { State::Empty };
        Slot { state, power_controller }
    }

    pub fn step(&mut self, now: u64, look: Look) -> Step {
        use State::*;
        let mut out = Step::default();
        let pc = self.power_controller;
        self.state = match self.state {
            Empty | Failed if !look.present => Empty,
            Empty => Settling { since: now },
            Failed => Failed,
            Settling { .. } if !look.present => Empty,
            Settling { since } if now.saturating_sub(since) >= SETTLE_MS => {
                if pc && !look.powered { out.power = Some(true); }
                out.led = Some(crate::pcie::control::LED_BLINK);
                Linking { since: now }
            }
            s @ Settling { .. } => s,
            Linking { .. } | LinkUp { .. } if !look.present => {
                if pc { out.power = Some(false); }
                out.led = Some(crate::pcie::control::LED_OFF);
                Empty
            }
            Linking { since } if look.link => { let _ = since; LinkUp { since: now } }
            Linking { since } if now.saturating_sub(since) >= LINK_TIMEOUT_MS => {
                if pc { out.power = Some(false); }
                out.led = Some(crate::pcie::control::LED_OFF);
                out.failed = true;
                Failed
            }
            s @ Linking { .. } => s,
            LinkUp { since } if !look.link => Linking { since },
            LinkUp { since } if now.saturating_sub(since) >= LINK_MS => {
                out.arrived = true;
                out.led = Some(crate::pcie::control::LED_ON);
                Up
            }
            s @ LinkUp { .. } => s,
            Up if !look.present || !look.link => {
                // Surprise removal: the devices are gone already.
                out.left = true;
                if pc && look.powered { out.power = Some(false); }
                out.led = Some(crate::pcie::control::LED_OFF);
                Empty
            }
            Up if look.button => { out.led = Some(crate::pcie::control::LED_BLINK); Button { since: now } }
            Up => Up,
            Button { .. } if !look.present => {
                out.left = true;
                out.led = Some(crate::pcie::control::LED_OFF);
                Empty
            }
            Button { .. } if look.button => { out.led = Some(crate::pcie::control::LED_ON); Up }
            Button { since } if now.saturating_sub(since) >= BUTTON_MS => {
                out.left = true;
                if pc { out.power = Some(false); }
                out.led = Some(crate::pcie::control::LED_OFF);
                Off
            }
            s @ Button { .. } => s,
            Off if !look.present => Empty,
            // Pressed again with the card still in: bring it back.
            Off if look.button => Settling { since: now },
            Off => Off,
        };
        out
    }
}
