// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Bluetooth bonds for Cufflinks: one file Files keeps for it (`sex_pdx::bt_bonds`).
use sex_pdx::bt_bonds::{Store, PATH};

static mut STORE: Store = Store::new();

/// One request; the word to reply with (none for PUT).
pub fn handle(caller: u32, arg0: u64, arg1: u64) -> Option<u64> {
    let store = unsafe { &mut *core::ptr::addr_of_mut!(STORE) };
    let reply = store.handle(caller, arg0, arg1,
        &mut |out| crate::fs_service::read_own(PATH, out).filter(|n| *n > 0),
        &mut |bytes| crate::fs_service::replace_own(PATH, bytes));
    if sex_pdx::bt_bonds::decode_request(arg0).0 == sex_pdx::bt_bonds::SAVE {
        sex_pdx::serial_record!("[files.bt.bonds.save] caller={} bytes={} reply={:#x}", caller, arg1, reply.unwrap_or(0));
    }
    reply
}
