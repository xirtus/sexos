// SPDX-License-Identifier: GPL-3.0-only
#![no_std]
#![no_main]
mod allocator;
mod pager;
mod workspace;
use pager::{Backend, CleanupOutcome, Mapping, Outcome, Pager, ReleaseOutcome, WaitOutcome};
use sex_pdx::{pdx_listen_raw, pdx_try_listen_raw, serial_record, sys_yield};
use sex_pdx::paging::*;
const FRAME_COUNT: usize = 8;
const CLIENT_PAGE_QUOTA: usize = 4;
struct KernelBackend;
impl Backend for KernelBackend {
    fn query(&mut self, mapping: u64, token: u64) -> Result<FaultQuery, u64> { sex_pdx::paging_query(mapping, token) }
    fn zero(&mut self, virtual_address: u64, length: u64) {
        // Sext retains its authorized pool alias. Every new lease is fully zeroed.
        unsafe { core::ptr::write_bytes(virtual_address as *mut u8, 0, length as usize); }
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);
    }
    fn commit(&mut self, mapping: u64, index: usize, token: u64, rights: u8) -> u64 {
        sex_pdx::paging_commit(mapping, SLOT_FRAME_POOL, index as u64, token, rights)
    }
    fn reject(&mut self, mapping: u64, token: u64) -> u64 { sex_pdx::paging_reject(mapping, token) }
    fn reclaim(&mut self, mapping: u64, index: usize, token: u64) -> u64 {
        sex_pdx::paging_reclaim(mapping, SLOT_FRAME_POOL, index as u64, token)
    }
    fn cleanup(&mut self, mapping: u64) -> Result<CleanupResult, u64> {
        sex_pdx::paging_cleanup(mapping, SLOT_FRAME_POOL)
    }
}
#[derive(Clone, Copy)]
enum Work { Fault(FaultNotice), Release(ReleaseRequest), WaitCleanup(WaitCleanupRequest), Reply { target: u32, value: u64 } }
#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_record!("[sext.init.start]");
    let pool = sex_pdx::paging_info(SLOT_FRAME_POOL).expect("SEXT_FRAME_POOL_MISSING");
    let primary = sex_pdx::paging_info(SLOT_PAGER_MAPPING).expect("SEXT_MAPPING_MISSING");
    let secondary = sex_pdx::paging_info(SLOT_PAGER_MAPPING_SECOND).expect("SEXT_SECOND_MAPPING_MISSING");
    let mappings = [Some(Mapping { slot: SLOT_PAGER_MAPPING, info: primary }),
        Some(Mapping { slot: SLOT_PAGER_MAPPING_SECOND, info: secondary })];
    let mut pager = Pager::<FRAME_COUNT>::new_multi(pool, mappings, CLIENT_PAGE_QUOTA).expect("SEXT_INVALID_BOOT_AUTHORITY");
    let mut backend = KernelBackend;
    serial_record!("[sext.ready] clients=2 frames=8 quota=4 zero_owner=sext");
    for mapping in mappings.into_iter().flatten() {
        serial_record!("[sext.client.ready] target={} frames={} quota={} pool_bytes={} arena_bytes={} zero_owner=sext",
            mapping.info.target_pd, FRAME_COUNT, CLIENT_PAGE_QUOTA, pool.length, mapping.info.length);
    }
    let mut pending: [Option<Work>; 2] = [None; 2];
    let mut cleanup_pending: [Option<ExitNotice>; 2] = [None; 2];
    let mut app_pending:[Option<sex_pdx::app_lifecycle::ExitNotice>;4]=[None;4];
    let mut workspaces:[Option<workspace::Workspace>;4]=[None;4];
    let mut turn = 0usize;
    let mut retired: [Option<(u64, u32, u64)>; FRAME_COUNT] = [None; FRAME_COUNT];
    loop {
        // At most one bounded operation per client per pass. A contended query,
        // uncertain commit or ACK cannot prevent the other client's progress.
        let can_progress = cleanup_pending.iter().any(Option::is_some) || pending.iter().any(|work| match work {
            Some(Work::WaitCleanup(request)) => pager.wait_cleanup(*request) != WaitOutcome::Pending,
            Some(_) => true,
            None => false,
        });
        // A wait for another client's future death sleeps on the inbox. The
        // reliable kernel hint wakes it; no idle status-polling loop is needed.
        let incoming = if can_progress { pdx_try_listen_raw(0) } else { Some(pdx_listen_raw(0)) };
        if let Some(message) = incoming {
            let args = [message.arg0, message.arg1, message.arg2];
            let caller = message.caller_pd as u64;
            if message.type_id==sex_pdx::app_lifecycle::OP_APP_EXIT {
                if let Ok(n)=sex_pdx::app_lifecycle::ExitNotice::decode(message.type_id,caller,args) {
                    for (i,slot) in sex_pdx::app_lifecycle::SLOTS.into_iter().enumerate() {
                        if let Ok(c)=sex_pdx::app_lifecycle::catalog(slot) {
                            if c.target_pd==n.target_pd && c.task_id==n.task_id && workspaces[i].is_none() {
                                app_pending[i]=Some(n);
                            }
                        }
                    }
                }
            }

            // Kernel hints are remembered independently of a retained operation;
            // in particular an uncertain commit must not hide terminal cleanup.
            if let Ok(notice) = ExitNotice::decode(message.type_id, caller, args) {
                if let Some(client) = pager.client_index(notice.target_pd) {
                    if cleanup_pending[client].is_none() { cleanup_pending[client] = Some(notice); }
                }
            }
            let work = if message.type_id == OP_WAIT_CLEANUP {
                match WaitCleanupRequest::decode(message.type_id, caller, args) {
                    Ok(request) => Some(Work::WaitCleanup(request)),
                    Err(_) => Some(Work::Reply { target: message.caller_pd,
                        value: release_reply(message.arg1, ERR_PAGING_DENIED) }),
                }
            } else if message.type_id == OP_RELEASE_PAGE {
                match ReleaseRequest::decode(message.type_id, caller, args) {
                    Ok(request) => Some(Work::Release(request)),
                    Err(_) => {
                        serial_record!("[sext.release.denied] target={} request={} page={:#x} status={}",
                            message.caller_pd, message.arg1, message.arg0, ERR_PAGING_DENIED);
                        Some(Work::Reply { target: message.caller_pd,
                            value: release_reply(message.arg1, ERR_PAGING_DENIED) })
                    },
                }
            } else {
                FaultNotice::decode(message.type_id, caller, args).ok().map(Work::Fault)
            };
            if let Some(work) = work {
                if let Some(client) = pager.client_index(message.caller_pd) {
                    // Clients serialize fault/release/ACK. Duplicate arrivals
                    // cannot overwrite an operation with uncertain custody.
                    if pending[client].is_none() { pending[client] = Some(work); }
                } else if matches!(message.type_id, OP_RELEASE_PAGE | OP_WAIT_CLEANUP) {
                    let _ = sex_pdx::pdx_reply(message.caller_pd, release_reply(message.arg1, ERR_PAGING_DENIED));
                }
            }
        }
        for offset in 0..2 {
            let client = (turn + offset) % 2;
            if let Some(notice) = cleanup_pending[client] {
                let bindings = pager.bindings_for(notice.target_pd);
                match pager.cleanup(&mut backend, notice) {
                    CleanupOutcome::Retry => continue,
                    CleanupOutcome::Live | CleanupOutcome::Ignored => cleanup_pending[client] = None,
                    CleanupOutcome::Fatal(status) => panic!("SEXT_UNCERTAIN_CLEANUP status={}", status),
                    CleanupOutcome::Closed { result, released, replay } => {
                        if !replay {
                            let mapping = pager.mapping_for(notice.target_pd).unwrap();
                            for binding in bindings.into_iter().flatten() {
                                retired[binding.index] = Some((mapping.slot, binding.target, binding.token));
                            }
                            serial_record!("[sext.cleanup] target={} task={} bitmap={:#x} released={} remaining={} closed=1 ok=1",
                                notice.target_pd, result.task_id, result.frame_bitmap, released, pager.used_by(notice.target_pd));
                        }
                        cleanup_pending[client] = None;
                        pending[client] = None;
                        continue;
                    }
                }
            }
            let Some(work) = pending[client] else { continue; };
            match work {
                Work::Fault(notice) => match pager.step(&mut backend, notice) {
                    Outcome::Retry => {},
                    Outcome::Ignored => pending[client] = None,
                    Outcome::Committed { index, page, target, token } => {
                        serial_record!("[sext.fault.commit] target={} token={} page={:#x} frame={} used={} zeroed=4096 ok=1",
                            target, token, page, index, pager.used_by(target));
                        for workspace in workspaces.iter_mut().flatten() {
                            unsafe{workspace.record(target,token);}
                        }
                        let mapping = pager.mapping_for(target).unwrap();
                        let rights = mapping.info.rights & (sex_pdx::memory_region::READ | sex_pdx::memory_region::WRITE);
                        let exact = sex_pdx::paging_commit(mapping.slot, SLOT_FRAME_POOL, index as u64, token, rights);
                        let conflict = sex_pdx::paging_commit(mapping.slot, SLOT_FRAME_POOL, ((index + 1) % FRAME_COUNT) as u64, token, rights);
                        let zero = sex_pdx::paging_commit(mapping.slot, SLOT_FRAME_POOL, index as u64, 0, rights);
                        let unknown = sex_pdx::paging_commit(mapping.slot, SLOT_FRAME_POOL, index as u64, u64::MAX, rights);
                        assert!(exact == PAGING_OK && conflict == ERR_PAGING_DENIED && zero != PAGING_OK && unknown != PAGING_OK,
                            "SEXT_COMMIT_IDEMPOTENCE_FAILED");
                        serial_record!("[sext.commit.negative] token={} exact=1 conflicting_frame=1 zero_token=1 unknown_token=1 ok=1", token);
                        if let Some((old_mapping, old_target, old_token)) = retired[index] {
                            let old_commit = sex_pdx::paging_commit(old_mapping, SLOT_FRAME_POOL, index as u64, old_token, 6);
                            let old_reclaim = sex_pdx::paging_reclaim(old_mapping, SLOT_FRAME_POOL, index as u64, old_token);
                            let new_commit = sex_pdx::paging_commit(mapping.slot, SLOT_FRAME_POOL, index as u64, token, rights);
                            assert!(old_commit == ERR_PAGING_DENIED
                                && matches!(old_reclaim, PAGING_OK | ERR_PAGING_DENIED) && new_commit == PAGING_OK,
                                "SEXT_STALE_REUSE_AUTHORITY");
                            serial_record!("[sext.reuse.negative] frame={} old_token={} token={} old_target={} target={} stale_commit=1 stale_reclaim=1 new_commit=1 ok=1",
                                index, old_token, token, old_target, target);
                            if let Some(expected) = pager.closed_result(old_target) {
                                // Exercise the kernel's closed-range cache after
                                // another owner has reused backing. Never apply
                                // the returned historical bitmap to the allocator.
                                let repeated = sex_pdx::paging_cleanup(old_mapping, SLOT_FRAME_POOL)
                                    .expect("SEXT_CLEANUP_REPLAY_FAILED");
                                let preserved = sex_pdx::paging_commit(mapping.slot, SLOT_FRAME_POOL, index as u64, token, rights);
                                assert!(repeated == expected && preserved == PAGING_OK, "SEXT_CLEANUP_REPLAY_CHANGED_OWNER");
                                serial_record!("[sext.cleanup.replay] target={} task={} bitmap={:#x} frame={} new_target={} new_token={} preserved=1 ok=1",
                                    old_target, repeated.task_id, repeated.frame_bitmap, index, target, token);
                            }
                        }
                        pending[client] = None;
                    }
                    Outcome::Rejected { reason, page, target, token } => {
                        serial_record!("[sext.fault.reject] target={} token={} page={:#x} reason={:?} used={} quota={}",
                            target, token, page, reason, pager.used_by(target), CLIENT_PAGE_QUOTA);
                        pending[client] = None;
                    }
                    Outcome::Fatal(status) => panic!("SEXT_UNCERTAIN_COMMIT status={}", status),
                },
                Work::Release(request) => match pager.release(&mut backend, request) {
                    ReleaseOutcome::Retry => {},
                    ReleaseOutcome::Fatal(status) => panic!("SEXT_UNCERTAIN_RECLAIM status={}", status),
                    ReleaseOutcome::Done { status, binding, replay } => {
                        if let Some(binding) = binding {
                            serial_record!("[sext.release] target={} request={} page={:#x} token={} frame={} status={} replay={} used={}",
                                request.caller_pd, request.request_id, request.page, binding.token, binding.index,
                                status, replay as u8, pager.used_by(request.caller_pd));
                            if status == PAGING_OK && !replay {
                                let mapping = pager.mapping_for(request.caller_pd).unwrap();
                                retired[binding.index] = Some((mapping.slot, request.caller_pd, binding.token));
                                let stale = sex_pdx::paging_commit(mapping.slot, SLOT_FRAME_POOL, binding.index as u64, binding.token, 6);
                                let duplicate = sex_pdx::paging_reclaim(mapping.slot, SLOT_FRAME_POOL, binding.index as u64, binding.token);
                                assert!(stale != PAGING_OK && duplicate == PAGING_OK, "SEXT_RECLAIM_TOMBSTONE_FAILED");
                                serial_record!("[sext.reclaim.negative] target={} token={} frame={} stale_commit=1 duplicate_reclaim=1 ok=1",
                                    request.caller_pd, binding.token, binding.index);
                            }
                        } else {
                            serial_record!("[sext.release.denied] target={} request={} page={:#x} status={}",
                                request.caller_pd, request.request_id, request.page, status);
                        }
                        pending[client] = Some(Work::Reply { target: request.caller_pd,
                            value: release_reply(request.request_id, status) });
                    }
                },
                Work::WaitCleanup(request) => match pager.wait_cleanup(request) {
                    WaitOutcome::Pending => {},
                    WaitOutcome::Reply(status) => {
                        serial_record!("[sext.cleanup.wait] caller={} target={} request={} status={}",
                            request.caller_pd, request.target_pd, request.request_id, status);
                        pending[client] = Some(Work::Reply { target: request.caller_pd,
                            value: release_reply(request.request_id, status) });
                    }
                },
                Work::Reply { target, value } => {
                    if sex_pdx::pdx_reply(target, value) == 0 { pending[client] = None; }
                }
            }
        }
        // Generic private retirement follows demand-range cleanup and all five
        // independent service ACKs. Kernel retries hints; no idle status spin.
        for i in 0..sex_pdx::app_lifecycle::SLOTS.len() {
            let Some(n)=app_pending[i] else{continue};
            match sex_pdx::app_lifecycle::retire(sex_pdx::app_lifecycle::SLOTS[i]) {
                Ok(pool)=>{
                    assert_eq!(pool.task_id,n.task_id,"APP_POOL_TASK");
                    let mut workspace=unsafe{workspace::Workspace::admit(pool.base,pool.length)};
                    // Exercise overwrite/reuse of the same real backing before
                    // retaining subsequent live-client journal records there.
                    unsafe{workspace.record(n.target_pd,n.task_id as u64);}
                    workspace.rewind();
                    let records=unsafe{workspace.record(14,n.task_id as u64)};
                    workspaces[i]=Some(workspace);app_pending[i]=None;
                    serial_record!("[sext.app.pool.ready] target={} task={} bytes={} zeroed={} workspace_records={} reused=1 zero_owner=sext ok=1",n.target_pd,n.task_id,pool.length,pool.length,records);
                },
                Err(sex_pdx::app_lifecycle::RETRY)=>{},
                Err(status)=>panic!("SEXT_APP_RETIRE status={}",status),
            }
        }
        turn ^= 1;
        if pending.iter().any(Option::is_some) || cleanup_pending.iter().any(Option::is_some) { sys_yield(); }
    }
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_record!("[sext.fatal] {}", info);
    loop { sys_yield(); }
}
