//! Thin wrappers over sex-pdx for sexfiles server.
//! Uses standard pdx_listen_raw(0) / pdx_reply pattern.

pub use sex_pdx::{
    pdx_listen_raw, pdx_reply, serial_println, BLOCK_ERR_BAD_CMD, BLOCK_ERR_BAD_LEN,
    BLOCK_ERR_NO_DEVICE, SLOT_BLOCK,
};

/// SEXFILES_DEFER_V1: client requests that arrive while a nested
/// reply-wait loop (diskfs_block_call) is listening on slot 0. The old
/// loop DISCARDED them as "stale startup messages" — every NVMe block
/// roundtrip made on behalf of client A silently ate any request client
/// B sent during the wait (root cause of the vanished-request hangs).
/// Stash them here; the trampoline main loop drains before listening.
/// Single-threaded PD — plain statics, no atomics needed.
static mut DEFERRED: crate::defer_queue::Deferred = crate::defer_queue::Deferred::new();

pub fn defer_stash(type_id: u64, caller_pd: u32, a0: u64, a1: u64, a2: u64) -> bool {
    unsafe {
        if !DEFERRED.push((type_id, caller_pd, a0, a1, a2)) {
            serial_println!(
                "[sexfiles.defer.drop] type={:#x} caller={} reason=stash_full",
                type_id,
                caller_pd
            );
            return false;
        }
        serial_println!(
            "[sexfiles.defer.stash] type={:#x} caller={} depth={}",
            type_id,
            caller_pd,
            DEFERRED.len()
        );
        true
    }
}

/// Message shape shared by the live-listen and defer-replay paths in the
/// trampoline loop (mirrors the PdxMessage fields the loop uses).
pub struct ReplayMsg {
    pub type_id: u64,
    pub caller_pd: u32,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
}

pub fn defer_pop() -> Option<(u64, u32, u64, u64, u64)> {
    unsafe {
        let msg = DEFERRED.pop()?;
        serial_println!(
            "[sexfiles.defer.replay] type={:#x} caller={} depth={}",
            msg.0,
            msg.1,
            DEFERRED.len()
        );
        Some(msg)
    }
}
