// SPDX-License-Identifier: GPL-3.0-only
//! Real instruction retry in the client's BOOT_DAG demand region.
use sex_pdx::paging::SLOT_DEMAND_REGION;

unsafe fn load(address: u64) -> u64 {
    let result: u64;
    core::arch::asm!("mov {result}, qword ptr [{address}]",
        address = in(reg) address, result = lateout(reg) result, options(nostack));
    result
}

unsafe fn store(address: u64, value: u64) {
    core::arch::asm!("mov qword ptr [{address}], {value}",
        address = in(reg) address, value = in(reg) value, options(nostack));
}

pub fn run() {
    // Bell serves this ordinary request only after its paging probe finishes.
    wait_bell();
    let info = sex_pdx::paging_info(SLOT_DEMAND_REGION).expect("PAGING_CLIENT_CAP_MISSING");
    assert!(info.base != 0 && info.base % 4096 == 0 && info.length == 8 * 4096);
    let invalid = sex_pdx::paging_info(0).is_err();
    let alias = sex_pdx::paging_info((1u64 << 32) | SLOT_DEMAND_REGION).is_err();
    let query = sex_pdx::paging_query(SLOT_DEMAND_REGION, 1).is_err();
    let commit = sex_pdx::paging_commit(SLOT_DEMAND_REGION, SLOT_DEMAND_REGION, 0, 1, 6) != 0;
    let reject = sex_pdx::paging_reject(SLOT_DEMAND_REGION, 1) != 0;
    let reclaim = sex_pdx::paging_reclaim(SLOT_DEMAND_REGION, SLOT_DEMAND_REGION, 0, 1) != 0;
    assert!(invalid && alias && query && commit && reject && reclaim, "PAGING_CLIENT_AUTHORITY_DENIAL_FAILED");
    sex_pdx::serial_record!("[paging.client.negative] invalid_slot=1 high_alias=1 wrong_query=1 wrong_commit=1 wrong_reject=1 wrong_reclaim=1 ok=1");
    sex_pdx::serial_record!("[paging.client.begin] pd={} base={:#x} bytes={}",
        info.target_pd, info.base, info.length);
    for page in 0..4u64 {
        let address = info.base + page * 4096;
        let value = 0x51e7_0000_0000_0000 | page;
        let write_first = page & 1 == 1;
        unsafe {
            if !write_first {
                // This load must fault, then retry at the same RIP and see zero.
                assert_eq!(load(address), 0, "PAGING_FRESH_FRAME_NOT_ZERO");
            }
            // Odd pages first fault on the store. Its original registers and
            // address must survive the trip through sext and the scheduler.
            store(address, value);
            assert_eq!(load(address), value, "PAGING_RETRY_VALUE_MISMATCH");
            assert_eq!(load(address + 4088), 0, "PAGING_PAGE_TAIL_NOT_ZERO");
        }
        sex_pdx::serial_record!("[paging.client.page] pd={} page={} first={} zero=1 value=1 tail=1 ok=1",
            info.target_pd, page, if write_first { "write" } else { "read" });
        sex_pdx::sys_yield();
    }
    // Re-read the earlier mappings after several independent faults/switches.
    for page in 0..4u64 {
        assert_eq!(unsafe { load(info.base + page * 4096) }, 0x51e7_0000_0000_0000 | page);
    }
    sex_pdx::serial_record!("[paging.client.recovered] pd={} pages=4 retries=4 retained=4 ok=1", info.target_pd);
    // Release then refault the same VA. A stale release request must replay its
    // old result without removing the replacement mapping at this address.
    release(info.base, 1, 0);
    assert_eq!(unsafe { load(info.base) }, 0, "PAGING_RECYCLED_FRAME_NOT_ZERO");
    unsafe { store(info.base, 0x51e7_0000_0000_0100); }
    release(info.base, 1, 0); // exact duplicate after remapping: no second reclaim
    release(info.base + 4096, 1, 3); // conflicting same-ID page is denied
    release(info.base, 0, 3); // zero request IDs are denied
    assert_eq!(unsafe { load(info.base) }, 0x51e7_0000_0000_0100);
    for page in 1..4u64 {
        assert_eq!(unsafe { load(info.base + page * 4096) }, 0x51e7_0000_0000_0000 | page);
    }
    sex_pdx::serial_record!("[paging.client.reuse] pd={} page=0 refault=1 zero=1 replay_safe=1 neighbors=3 ok=1", info.target_pd);
    wait_bell();
    #[cfg(paging_exit_cleanup_proof)]
    {
        sex_pdx::serial_record!("[paging.client.finish] pd={} resident=4 reason=proof", info.target_pd);
        sex_pdx::task_finish(0);
    }
    #[cfg(paging_quota_proof)]
    {
        sex_pdx::serial_record!("[paging.client.quota.begin] pd={} page=4 in_range=1", info.target_pd);
        let _ = unsafe { load(info.base + 4 * 4096) };
        panic!("PAGING_QUOTA_FAULT_UNEXPECTEDLY_RETURNED");
    }
}

fn release(page: u64, request: u64, expected: u64) {
    assert_eq!(sex_pdx::pdx_call(sex_pdx::SLOT_SEXT, 0x104, page, request, 0).0, 0);
    loop {
        let reply = sex_pdx::pdx_listen_raw(0);
        if reply.type_id == 1 && reply.caller_pd == 14 {
            assert_eq!(reply.arg0 >> 8, request, "PAGING_RELEASE_REPLY_ID");
            assert_eq!(reply.arg0 & 255, expected, "PAGING_RELEASE_STATUS");
            return;
        }
    }
}

fn wait_bell() {
    assert_eq!(sex_pdx::pdx_call(sex_pdx::SLOT_BELL, sex_pdx::OP_BELL_SUBSCRIBE, 0, 0, 0).0, 0);
    loop {
        let message = sex_pdx::pdx_listen_raw(0);
        if message.type_id == 1 && message.caller_pd == 10 {
            assert_eq!(message.arg0, u64::MAX);
            return;
        }
    }
}
