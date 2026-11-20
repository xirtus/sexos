// SPDX-License-Identifier: GPL-3.0-only
//! The NHI's inbound mailbox (BAR0 INMAIL_DATA / INMAIL_CMD): short commands to the
//! controller's firmware outside the control rings. The driver posts one and polls
//! until the firmware clears the request bit (Linux allows 500 ms).

use crate::regs::*;
use crate::Nhi;

/// The OS driver is going away (sleep or unload). The firmware stops sending ICM
/// events until the next DRIVER_READY.
pub const DRV_UNLOADS: u8 = 0x07;
pub const TIMEOUT_US: u64 = 500_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Busy, Done, Error }

pub fn post(n: &mut impl Nhi, command: u8, data: u32) {
    n.write(INMAIL_DATA, data);
    let word = n.read(INMAIL_CMD) & !(INMAIL_CMD_MASK | INMAIL_ERROR);
    n.write(INMAIL_CMD, word | INMAIL_OP_REQUEST | command as u32);
}

/// Where the last posted command is; a controller reading all ones stays Busy.
pub fn state(n: &mut impl Nhi) -> State {
    let word = n.read(INMAIL_CMD);
    if word & INMAIL_OP_REQUEST != 0 { State::Busy } else if word & INMAIL_ERROR != 0 { State::Error } else { State::Done }
}
