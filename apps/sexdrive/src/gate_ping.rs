// SPDX-License-Identifier: GPL-3.0-only
//! PKU gate proof (kernel study P6.2c): a pure function Files calls with no kernel
//! entry. It runs under SexDrive's keys on a gate stack and touches nothing else.

extern "C" fn ping(a: u64, b: u64, c: u64, d: u64) -> u64 {
    (a ^ b.rotate_left(16) ^ c.rotate_left(32) ^ d.rotate_left(48)).wrapping_add(0x5EC5_D21E)
}

/// Exports `ping` for any service holding a route to SexDrive.
pub fn export() {
    let r = sex_pdx::gate::create(ping, sex_pdx::gate::EXPORT_PING);
    sex_pdx::serial_record!("[sexdrive.gate.export] ok={} status={:#x}", r.is_ok() as u8, r.err().unwrap_or(0));
}
