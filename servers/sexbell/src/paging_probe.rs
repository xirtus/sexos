// SPDX-License-Identifier: GPL-3.0-only
//! Second boot-authorized client of sext; one page stays live during reuse.
use sex_pdx::{paging::SLOT_DEMAND_REGION, serial_record};
static mut DEFERRED: [Option<sex_pdx::PdxMessage>; 32] = [None; 32];
static mut DEFERRED_COUNT: usize = 0;
static mut DEFERRED_READ: usize = 0;
pub fn next_message() -> sex_pdx::PdxMessage {
    unsafe {
        if DEFERRED_READ < DEFERRED_COUNT {
            let index = DEFERRED_READ;
            DEFERRED_READ += 1;
            return DEFERRED[index].take().expect("BELL_DEFERRED_MESSAGE");
        }
    }
    sex_pdx::pdx_listen_raw(0)
}
pub fn run() {
    let info = sex_pdx::paging_info(SLOT_DEMAND_REGION).expect("BELL_DEMAND_REGION_MISSING");
    serial_record!("[paging.client.begin] pd={} base={:#x} bytes={}", info.target_pd, info.base, info.length);
    assert!(info.base != 0 && info.base % 4096 == 0 && info.length == 32768);
    for page in 0..4u64 {
        let address = (info.base + page * 4096) as *mut u64;
        unsafe {
            assert_eq!(core::ptr::read_volatile(address), 0, "BELL_DEMAND_NOT_ZERO");
            assert_eq!(core::ptr::read_volatile(address.add(511)), 0, "BELL_DEMAND_TAIL_NOT_ZERO");
            core::ptr::write_volatile(address, 0xbe11_0000_0000_0000 | page);
            assert_eq!(core::ptr::read_volatile(address), 0xbe11_0000_0000_0000 | page);
            // Dirty the tail too: checking only the first word misses leaks.
            core::ptr::write_volatile(address.add(511), 0xbe11_ffff_ffff_0000 | page);
        }
        serial_record!("[paging.client.page] pd={} page={} first=read zero=1 value=1 tail=1 ok=1", info.target_pd, page);
    }
    for page in 0..3u64 {
        assert_eq!(sex_pdx::pdx_call(sex_pdx::SLOT_SEXT, 0x104, info.base + page * 4096, page + 1, 0).0, 0);
        loop {
            let reply = sex_pdx::pdx_listen_raw(0);
            if reply.type_id == 1 && reply.caller_pd == 14 {
                assert_eq!(reply.arg0, (page + 1) << 8, "BELL_RELEASE_FAILED");
                break;
            }
            unsafe {
                assert!(DEFERRED_COUNT < 32, "BELL_DEFERRED_OVERFLOW");
                DEFERRED[DEFERRED_COUNT] = Some(reply);
                DEFERRED_COUNT += 1;
            }
        }
    }
    serial_record!("[paging.client.released] pd={} pages=3 retained=1 ok=1", info.target_pd);
}

pub fn check_retained() {
    let info = sex_pdx::paging_info(SLOT_DEMAND_REGION).expect("BELL_DEMAND_REGION_MISSING");
    unsafe {
        assert_eq!(core::ptr::read_volatile((info.base + 3 * 4096) as *const u64), 0xbe11_0000_0000_0003);
        assert_eq!(core::ptr::read_volatile((info.base + 3 * 4096 + 4088) as *const u64), 0xbe11_ffff_ffff_0003);
    }
}

#[cfg(paging_exit_cleanup_proof)]
pub fn exit_reuse() {
    // This runs after the second normal Bell reply, so Kaleidoscope is free to
    // finish or hit its quota. Wait for sext's authoritative cleanup ACK.
    assert_eq!(sex_pdx::pdx_call(sex_pdx::SLOT_SEXT, 0x106, 15, 4, 0).0, 0);
    loop {
        let reply = sex_pdx::pdx_listen_raw(0);
        if reply.type_id == 1 && reply.caller_pd == 14 {
            assert_eq!(reply.arg0, 4 << 8, "BELL_CLEANUP_WAIT_ACK");
            break;
        }
        unsafe {
            assert!(DEFERRED_COUNT < 32, "BELL_DEFERRED_OVERFLOW");
            DEFERRED[DEFERRED_COUNT] = Some(reply);
            DEFERRED_COUNT += 1;
        }
    }
    let info = sex_pdx::paging_info(SLOT_DEMAND_REGION).expect("BELL_DEMAND_REGION_MISSING");
    for page in 0..3u64 {
        let address = (info.base + page * 4096) as *mut u64;
        unsafe {
            assert_eq!(core::ptr::read_volatile(address), 0, "BELL_EXIT_REUSE_NOT_ZERO");
            assert_eq!(core::ptr::read_volatile(address.add(511)), 0, "BELL_EXIT_REUSE_TAIL_NOT_ZERO");
            core::ptr::write_volatile(address, 0xc1ea_0000_0000_0000 | page);
            assert_eq!(core::ptr::read_volatile(address), 0xc1ea_0000_0000_0000 | page);
        }
        check_retained();
        serial_record!("[paging.client.exit_reuse.page] pd=10 page={} zero=1 tail=1 value=1 neighbor=1 ok=1", page);
    }
    serial_record!("[paging.client.exit_reuse] pd=10 pages=3 retained=1 dead_target=15 ok=1");
}
