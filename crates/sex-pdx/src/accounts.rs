// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Multipass, the accounts service: the protocol apps use to ask for accounts and to act
//! for them. Holding the BOOT route to Multipass is the right to ask; what an app may do
//! for which stamp is a grant the person gives (Tatami, or a Collar prompt).
//!
//! Tokens and passwords never cross this protocol. There is a `TOKEN` kind only so the
//! refusal is part of the contract: Multipass always answers it with `TOKEN_NEVER_LEAVES`.
//!
//! Requests and answers carry their bodies in the region shared with Multipass
//! (`REGION_BYTES`), as UTF-8 lines or the raw HTTP body of an act-for answer.
//! Slot and region numbers are assigned when `servers/multipass` joins the boot catalog.

pub const OP: u64 = 0x2B0;
pub const REGION_BYTES: usize = 256 * 1024;

/// List the caller's granted stamps: one line each, `id \t provider \t address \t verified \t signed_in`.
pub const LIST: u8 = 1;
/// Ask the person to grant this app a stamp (opens the Multipass picker). Value: scope.
pub const REQUEST_GRANT: u8 = 2;
/// Act for a stamp. Region: `method \n path \n content-type \n header: value ... \n\n body`.
/// Answer: HTTP status in the value, response body in the region.
pub const ACT: u8 = 3;
/// Always refused with `TOKEN_NEVER_LEAVES`.
pub const TOKEN: u8 = 4;
/// Open an authenticated mail flow (IMAP or SMTP) for a stamp: Multipass runs the SASL step
/// with the secret from Shibari and hands the caller the flow, already signed in.
pub const OPEN_MAIL_FLOW: u8 = 5;
/// Drop this app's grant for a stamp.
pub const RELEASE: u8 = 6;

pub const SCOPE_READ: u32 = 1;
pub const SCOPE_WRITE: u32 = 2;

pub const OK: u8 = 0;
pub const WAIT: u8 = 1;
pub const NO_GRANT: u8 = 2;
pub const NOT_SIGNED_IN: u8 = 3;
pub const OFFLINE: u8 = 4;
pub const INVALID: u8 = 5;
pub const TOKEN_NEVER_LEAVES: u8 = 6;
pub const DECLINED: u8 = 7;
pub const FAILED: u8 = 8;

const TAG: u64 = 0xAC;
/// Request word: sequence, kind and the stamp it is about.
pub const fn request(seq: u32, kind: u8, stamp: u32) -> u64 {
    (seq as u64 & 0xFF_FFFF) | (kind as u64) << 24 | (stamp as u64) << 32
}
pub const fn decode_request(a: u64) -> (u32, u8, u32) { ((a & 0xFF_FFFF) as u32, (a >> 24) as u8, (a >> 32) as u32) }
pub const fn reply(seq: u32, status: u8, value: u32) -> u64 {
    TAG << 56 | ((seq as u64) & 0xFF_FFFF) << 32 | (status as u64) << 24 | (value as u64 & 0xFF_FFFF)
}
pub fn decode_reply(word: u64) -> Option<(u32, u8, u32)> {
    if word >> 56 != TAG { return None; }
    Some((((word >> 32) & 0xFF_FFFF) as u32, (word >> 24) as u8, (word & 0xFF_FFFF) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn words_round_trip() {
        assert_eq!(decode_request(request(7, ACT, 42)), (7, ACT, 42));
        assert_eq!(decode_reply(reply(9, TOKEN_NEVER_LEAVES, 0)), Some((9, TOKEN_NEVER_LEAVES, 0)));
        assert_eq!(decode_reply(reply(1, OK, 207)), Some((1, OK, 207)));
        assert_eq!(decode_reply(0), None);
    }
}
