#![allow(dead_code, unused_variables, unused_assignments, static_mut_refs)]
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/memory_region.rs"]
pub mod memory_region;
#[path = "../crates/sex-pdx/src/dma_pool.rs"]
pub mod dma_pool;
#[path = "../apps/sexdrive/src/dma_transfer.rs"]
mod dma_transfer;
#[path="../apps/sexdrive/src/completion_wait.rs"]mod completion_wait;
pub fn get_ticks()->u64{driver_read::mock_ticks()}
pub fn sys_wait_self_ticks(ticks:u64)->Result<(),u64>{driver_read::mock_wait(ticks)}

use dma_pool::{DmaError, DmaLease, DmaRegionPool};
use dma_transfer::DmaTransfer;
use memory_region::{RegionView, QUERY_VIEW, READ, WRITE};
#[macro_export]macro_rules! serial_record{($($arg:tt)*)=>{}}

fn pool() -> DmaRegionPool {
    DmaRegionPool::new(RegionView { region_id: 7, virtual_address: 0x100000,
        dma_address: Some(0x200000), length: 512 * 1024 }).unwrap()
}
fn no_action() {}

#[test]
fn pre_submit_failure_returns_the_same_buffer() {
    let mut pool = pool();
    let backing = pool.reserve(4096).unwrap();
    let mut lease = DmaLease::new(backing);
    drop(DmaTransfer::prepare(&mut lease, 7, no_action).unwrap());
    assert_eq!(lease.prepare(8).unwrap(), backing);
}

#[test]
fn timeout_quarantines_and_invokes_readiness_stop() {
    static STOPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    fn stop() { STOPPED.store(true, std::sync::atomic::Ordering::SeqCst); }
    let mut lease = DmaLease::new(pool().reserve(4096).unwrap());
    { let mut transfer = DmaTransfer::prepare(&mut lease, 9, stop).unwrap(); transfer.submit().unwrap(); }
    assert!(STOPPED.load(std::sync::atomic::Ordering::SeqCst));
    assert!(lease.is_quarantined());
    assert_eq!(lease.prepare(10), Err(DmaError::Busy));
}

#[test]
fn foreign_cid_cannot_release_uncertain_dma() {
    let mut lease = DmaLease::new(pool().reserve(4096).unwrap());
    {
        let mut transfer = DmaTransfer::prepare(&mut lease, 19, no_action).unwrap();
        transfer.submit().unwrap();
        assert_eq!(transfer.complete(20, 1, 1), Err(DmaError::InvalidTransition));
    }
    assert!(lease.is_quarantined());
}

#[test]
fn stale_phase_cannot_release_wrapped_cid() {
    let mut lease = DmaLease::new(pool().reserve(4096).unwrap());
    {
        let mut transfer = DmaTransfer::prepare(&mut lease, 0, no_action).unwrap();
        transfer.submit().unwrap();
        assert_eq!(transfer.complete(0, 0, 1), Err(DmaError::InvalidTransition));
    }
    assert!(lease.is_quarantined());
}

#[test]
fn matching_error_cqe_is_completed_dma_not_a_timeout() {
    let mut lease = DmaLease::new(pool().reserve(4096).unwrap());
    {
        let mut transfer = DmaTransfer::prepare(&mut lease, 11, || panic!("completion is certain")).unwrap();
        transfer.submit().unwrap();
        let error_cqe = 11u32 | (1 << 16) | (2 << 17);
        assert_ne!(error_cqe >> 17, 0);
        transfer.complete((error_cqe & 0xffff) as u16, (error_cqe >> 16) & 1, 1).unwrap();
    }
    assert!(!lease.is_quarantined());
    assert!(lease.prepare(12).is_ok());
}

#[test]
fn twenty_thousand_transfers_keep_pool_usage_constant() {
    let mut pool = pool();
    let read = pool.reserve(4096).unwrap();
    let write = pool.reserve(4096).unwrap();
    let mut leases = [DmaLease::new(read), DmaLease::new(write)];
    for tag in 0..20_000u16 {
        let mut transfer = DmaTransfer::prepare(&mut leases[tag as usize % 2], tag as u64, no_action).unwrap();
        assert_eq!(transfer.buffer.length, 4096);
        transfer.submit().unwrap();
        transfer.complete(tag, (tag / 16) as u32 % 2, (tag / 16) as u32 % 2).unwrap();
    }
    assert_eq!(pool.used(), 8192);
}

#[test]
fn completion_without_submission_does_not_release_as_completed() {
    let mut lease = DmaLease::new(pool().reserve(4096).unwrap());
    {
        let mut transfer = DmaTransfer::prepare(&mut lease, 12, no_action).unwrap();
        assert_eq!(transfer.complete(12, 1, 1), Err(DmaError::InvalidTransition));
    }
    // Prepared-only abort is safe because no command was made visible.
    assert!(lease.prepare(13).is_ok());
}

// Test the extracted production driver peer-check body against the same region
// resolver used by the kernel. Only syscall transport is substituted here.
const SLOT_BLOCK_BUFFER: u64 = 17;
pub const DISKFS_V4_POOL_BASE_LBA:u64=400;
pub const DISKFS_V4_POOL_SECTORS:u64=1408;
fn grant() -> memory_region::RegionGrant {
    memory_region::RegionGrant { region_id: 5, virtual_base: 0x400000,
        dma_base: None, length: 4096,
        rights: memory_region::QUERY_VIEW | READ | WRITE, peer_pd: Some(27) }
}
pub fn memory_region_info(slot: u64) -> Result<memory_region::MemoryRegionInfo, u64> {
    if slot != SLOT_BLOCK_BUFFER { return Err(1); }
    memory_region::resolve_info(Some(&grant())).map_err(|_| 1)
}
pub fn memory_region_view(slot: u64, offset: u64, length: u64, rights: u8) -> Result<RegionView, u64> {
    if slot != SLOT_BLOCK_BUFFER { return Err(1); }
    memory_region::resolve_view(Some(&grant()), memory_region::ViewRequest { offset, length, rights }).map_err(|_| 1)
}
include!(env!("SEXDRIVE_BLOCK_VIEW_SOURCE"));

#[test]
fn actual_driver_peer_check_accepts_only_authenticated_peer_and_local_slot() {
    assert!(block_buffer_view(27, 17, 512, WRITE).is_some());
    assert!(block_buffer_view(6, 17, 512, WRITE).is_none());
    assert!(block_buffer_view(27, 0x1_0000_0011, 512, WRITE).is_none());
    assert!(block_buffer_view(27, 17, 4097, WRITE).is_none());
    assert!(block_buffer_view(27, 17, 512, memory_region::DMA_ADDRESS).is_none());
}

// Compile the full production read path. Substitute only its device buffers and
// serial transport; its validation, CQ polling, guard, copy and cursor updates
// are the driver's actual implementation. The host does not simulate real DMA.
mod driver_read {
    use super::*;
    use core::sync::atomic::{compiler_fence, Ordering};
    macro_rules! serial_println { ($($tokens:tt)*) => {}; }
    const PAGE_SIZE: u64 = 4096;
    const BLOCK_MAX_XFER: u64 = 4096;
    const BLOCK_ERR_BAD_LEN: u64 = 2;
    const BLOCK_ERR_NO_DEVICE: u64 = 4;
    const NVME_LBA_BYTES: u64 = 512;
    const WRITE_PROOF_LBA: u64 = 2047;
    const WRITE_PROOF_LEN: u64 = 512;
    const MANIFEST_LBA:u64=2046;
    const PROOF_OBJECT_START_LBA:u64=2038;const PROOF_OBJECT_END_LBA:u64=2045;
    const LINEN_OBJECT_START_LBA:u64=2030;const LINEN_OBJECT_END_LBA:u64=2037;
    const QUIL_OBJECT_START_LBA:u64=2022;const QUIL_OBJECT_END_LBA:u64=2029;
    const SEXFS_V0_META_START_LBA:u64=0;const SEXFS_V0_META_END_LBA:u64=47;
    const SEXFS_V0_OBJECT_START_LBA:u64=128;const SEXFS_V0_OBJECT_END_LBA:u64=2019;
    const WRITE_PROOF_MAGIC:u64=0x3156455449525753;
    fn hot_log() -> bool { false }
    struct NvmeState {
        ready: bool, map_va: u64, io_sq_va: u64, io_cq_va: u64,
        sq1tdbl: u64, cq1hdbl: u64, sq_tail: u32, cq_head: u32,
        cq_phase: u32, next_cid: u16,
    }
    static mut NVME_IO_STATE: NvmeState = NvmeState { ready: false,
        map_va: 0, io_sq_va: 0, io_cq_va: 0, sq1tdbl: 0, cq1hdbl: 0,
        sq_tail: 0, cq_head: 0, cq_phase: 1, next_cid: 1 };
    static mut READ_LEASE: Option<DmaLease> = None;
    static mut WRITE_LEASE: Option<DmaLease> = None;
    static mut NOW:u64=0;static mut WAITS:u32=0;static mut DELAY:u32=0;
    static mut CLOCK_MODE:u8=0;static mut COMPLETION:u32=0;static mut CQ_PTR:*mut u32=core::ptr::null_mut();
    static mut DEST_PTR:*const u8=core::ptr::null();static mut DEST_LEN:usize=0;
    static mut DOORBELLS:*const u32=core::ptr::null();
    pub fn mock_ticks()->u64{unsafe{NOW}}
    pub fn mock_wait(ticks:u64)->Result<(),u64>{unsafe{
        assert_eq!(ticks,1);WAITS+=1;
        assert!(NVME_IO_STATE.ready);assert_eq!(NVME_IO_STATE.cq_head,0,"no early CQ consumption");
        assert_eq!(*DOORBELLS,1,"one submitted SQ doorbell, no replay");assert_eq!(*DOORBELLS.add(1),0,"no CQ doorbell before completion");
        if !DEST_PTR.is_null(){assert!(core::slice::from_raw_parts(DEST_PTR,DEST_LEN).iter().all(|b|*b==0xa5),"no early READ handoff");}
        match CLOCK_MODE{1=>NOW=NOW.saturating_sub(1),2=>{},3=>return Err(9),4=>NOW+=completion_wait::MAX_TICKS,_=>NOW+=1}
        if DELAY!=0&&WAITS==DELAY{core::ptr::write_volatile(CQ_PTR.add(3),COMPLETION);}
        Ok(())
    }}
    include!(env!("SEXDRIVE_READ_PATH_SOURCE"));
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn exercise(write: bool, cqe: u32, expected: u64, quarantined: bool, ready: bool) {
        exercise_size(write,cqe,expected,quarantined,ready,512);
    }
    fn exercise_size(write:bool,cqe:u32,expected:u64,quarantined:bool,ready:bool,size:usize){
        exercise_wait(write,cqe,expected,quarantined,ready,size,0,0,0);
    }
    fn exercise_wait(write:bool,cqe:u32,expected:u64,quarantined:bool,ready:bool,size:usize,delay:u32,clock_mode:u8,completion:u32){
        let _lock = LOCK.lock().unwrap();
        let mut sq = [0u64; 128];
        let mut cq = [0u32; 64];
        let mut doorbells = [0u32; 2];
        let mut buffer = [0u64; 512];
        let mut destination = vec![0xa5u8; size];
        cq[3] = cqe;
        unsafe {
            NOW=10;WAITS=0;DELAY=delay;CLOCK_MODE=clock_mode;COMPLETION=completion;CQ_PTR=cq.as_mut_ptr();
            DEST_PTR=destination.as_ptr();DEST_LEN=size;DOORBELLS=doorbells.as_ptr();
            let lease = Some(DmaLease::new(dma_pool::DmaBuffer {
                virtual_address: buffer.as_mut_ptr() as u64,
                physical_address: 0x200000, length: 4096,
            }));
            if write { WRITE_LEASE = lease; } else { READ_LEASE = lease; }
            NVME_IO_STATE = NvmeState { ready: true,
                map_va: doorbells.as_mut_ptr() as u64, io_sq_va: sq.as_mut_ptr() as u64,
                io_cq_va: cq.as_ptr() as u64, sq1tdbl: 0, cq1hdbl: 4,
                sq_tail: 0, cq_head: 0, cq_phase: 1, next_cid: 42 };
        }
        let result = if write { nvme_write_one_block(0, size as u64, destination.as_ptr() as u64) }
            else { typed_read_handoff(0,size as u64,SLOT_BLOCK_BUFFER,Some(RegionView{
                region_id:5,virtual_address:destination.as_mut_ptr()as u64,dma_address:None,length:size as u64})) };
        assert_eq!(result, expected);
        assert_eq!(doorbells[0], 1, "real production body must submit");
        unsafe {
            let lease = if write { WRITE_LEASE.as_ref() } else { READ_LEASE.as_ref() }.unwrap();
            assert_eq!(lease.is_quarantined(), quarantined);
            assert_eq!(NVME_IO_STATE.ready, ready);
            if expected == 0 {
                assert_eq!(destination, if write { vec![0xa5; size] } else { vec![0; size] });
                assert_eq!(*((sq.as_ptr() as *const u32).add(12)), (size/512-1)as u32);
                if write {
                    assert!(std::slice::from_raw_parts(buffer.as_ptr() as *const u8,size).iter().all(|&b|b==0xA5));
                }
                if write { assert_eq!(buffer[0], 0xa5a5_a5a5_a5a5_a5a5); }
                assert_eq!(NVME_IO_STATE.cq_head, 1);
                assert_eq!(doorbells[1], 1);
            } else {
                assert_eq!(destination, vec![0xa5; size], "failure must not hand off uncompleted data");
            }
            assert_eq!(NVME_IO_STATE.next_cid,43,"never resubmit or reserve another CID");
            if quarantined{
                assert_eq!(NVME_IO_STATE.cq_head,0);assert_eq!(doorbells[1],0);
                let lease=if write{WRITE_LEASE.as_mut()}else{READ_LEASE.as_mut()}.unwrap();assert_eq!(lease.prepare(99),Err(DmaError::Busy));
                let before=sq;let result=if write{nvme_write_one_block(0,size as u64,destination.as_ptr()as u64)}else{nvme_read_into_mapped_va(0,size as u64,destination.as_mut_ptr()as u64)};
                assert_eq!(result,BLOCK_ERR_NO_DEVICE);assert_eq!(NVME_IO_STATE.next_cid,43);assert_eq!(sq,before);assert_eq!(doorbells[0],1);
            }
            if delay!=0&&expected==0{assert_eq!(WAITS,delay);}
            if !quarantined{let lease=if write{WRITE_LEASE.as_mut()}else{READ_LEASE.as_mut()}.unwrap();assert!(lease.prepare(99).is_ok());}
            READ_LEASE = None;
            WRITE_LEASE = None;
            CQ_PTR=core::ptr::null_mut();DOORBELLS=core::ptr::null();DEST_PTR=core::ptr::null();
            NVME_IO_STATE.ready = false;
        }
    }

    #[test]fn actual_matching_last_queue_slot_advances_head_and_phase_exactly_once(){
        let _lock=LOCK.lock().unwrap();let mut sq=[0u64;128];let mut cq=[0u32;64];let mut db=[0u32;2];let mut buffer=[0u64;512];let source=[0xa5u8;512];
        unsafe{
            NOW=10;cq[15*4+3]=42|(1<<16);WRITE_LEASE=Some(DmaLease::new(dma_pool::DmaBuffer{virtual_address:buffer.as_mut_ptr()as u64,physical_address:0x200000,length:4096}));
            NVME_IO_STATE=NvmeState{ready:true,map_va:db.as_mut_ptr()as u64,io_sq_va:sq.as_mut_ptr()as u64,io_cq_va:cq.as_ptr()as u64,sq1tdbl:0,cq1hdbl:4,sq_tail:15,cq_head:15,cq_phase:1,next_cid:42};
        }
        assert_eq!(nvme_write_one_block(0,512,source.as_ptr()as u64),0);assert_eq!(db,[0,0]);
        unsafe{assert_eq!((NVME_IO_STATE.sq_tail,NVME_IO_STATE.cq_head,NVME_IO_STATE.cq_phase,NVME_IO_STATE.next_cid),(0,0,0,43));assert!(!WRITE_LEASE.as_ref().unwrap().is_quarantined());WRITE_LEASE=None;NVME_IO_STATE.ready=false;}
    }
    #[test]fn actual_flush_waits_for_exact_completion_and_fences_on_timeout_without_replay(){
        let _lock=LOCK.lock().unwrap();
        for success in [false,true]{
            let mut sq=[0u64;128];let mut cq=[0u32;64];let mut db=[0u32;2];
            unsafe{
                NOW=10;WAITS=0;DELAY=if success{3}else{0};CLOCK_MODE=0;COMPLETION=42|(1<<16);cq[3]=26;
                CQ_PTR=cq.as_mut_ptr();DOORBELLS=db.as_ptr();DEST_PTR=core::ptr::null();DEST_LEN=0;
                NVME_IO_STATE=NvmeState{ready:true,map_va:db.as_mut_ptr()as u64,io_sq_va:sq.as_mut_ptr()as u64,io_cq_va:cq.as_ptr()as u64,sq1tdbl:0,cq1hdbl:4,sq_tail:0,cq_head:0,cq_phase:1,next_cid:42};
            }
            assert_eq!(nvme_flush(),if success{0}else{BLOCK_ERR_NO_DEVICE});assert_eq!(db[0],1);
            unsafe{assert_eq!(NVME_IO_STATE.ready,success);assert_eq!(NVME_IO_STATE.next_cid,43);assert_eq!(NVME_IO_STATE.cq_head,success as u32);assert_eq!(db[1],success as u32);if !success{assert_eq!(nvme_flush(),BLOCK_ERR_NO_DEVICE);assert_eq!(NVME_IO_STATE.next_cid,43);assert_eq!(db[0],1);}CQ_PTR=core::ptr::null_mut();DOORBELLS=core::ptr::null();}
        }
    }
    #[test]fn actual_delayed_read_and_write_complete_after_wait_without_submission_replay(){
        for write in [false,true]{exercise_wait(write,26,0,false,true,512,3,0,42|(1<<16));}
    }
    #[test]fn actual_stale_phase_or_wrong_cid_never_releases_dma_on_foreign_receipt(){
        for cqe in [42,41|(1<<16)]{exercise_wait(false,cqe,BLOCK_ERR_NO_DEVICE,true,false,512,0,0,0);}
    }
    #[test]fn actual_wait_deadline_rollback_stalled_clock_and_denied_full_quarantine(){
        for mode in [1,2,3,4]{exercise_wait(true,26,BLOCK_ERR_NO_DEVICE,true,false,512,0,mode,0);}
    }
    #[test]fn delayed_matching_error_cqe_completes_dma_then_stops_queue_without_copy(){
        exercise_wait(false,26,BLOCK_ERR_NO_DEVICE,false,false,512,2,0,42|(1<<16)|(2<<17));
    }
    #[test]
    fn production_write_completion_copies_payload_and_updates_queue() {
        exercise(true, 42 | (1 << 16), 0, false, true);
    }

    #[test]
    fn production_write_timeout_never_reuses_buffer() {
        exercise(true, 0, BLOCK_ERR_NO_DEVICE, true, false);
    }

    #[test]
    fn production_write_error_completes_dma_but_stops_queue() {
        exercise(true, 42 | (1 << 16) | (2 << 17), BLOCK_ERR_NO_DEVICE, false, false);
    }

    #[test]
    fn production_read_completion_updates_queue_and_copies() {
        exercise(false, 42 | (1 << 16), 0, false, true);
    }

    #[test]
    fn production_read_timeout_quarantines_and_keeps_destination() {
        exercise(false, 0, BLOCK_ERR_NO_DEVICE, true, false);
    }

    #[test]
    fn production_read_stale_phase_is_not_completion() {
        exercise(false, 42, BLOCK_ERR_NO_DEVICE, true, false);
    }

    #[test]
    fn production_read_error_completes_dma_but_stops_queue() {
        exercise(false, 42 | (1 << 16) | (2 << 17), BLOCK_ERR_NO_DEVICE, false, false);
    }
    #[test]
    fn production_four_kib_write_uses_eight_lbas_and_the_same_dma_lease(){
        exercise_size(true,42|(1<<16),0,false,true,4096);
    }
    #[test]
    fn typed_page_read_uses_eight_lbas_and_same_lease(){
        exercise_size(false,42|(1<<16),0,false,true,4096);
    }
    #[test]
    fn typed_page_read_error_and_timeout_never_publish_bytes(){
        exercise_size(false,0,BLOCK_ERR_NO_DEVICE,true,false,4096);
        exercise_size(false,42|(1<<16)|(2<<17),BLOCK_ERR_NO_DEVICE,false,false,4096);
    }
    #[test]
    fn typed_read_rejects_bad_length_offset_slot_or_view_before_queue_access(){
        let _lock=LOCK.lock().unwrap();
        let view=RegionView{region_id:5,virtual_address:0x400000,dma_address:None,length:4096};
        unsafe{NVME_IO_STATE.ready=false;}
        for size in [0,1,513,4097,8192,u64::MAX]{assert_eq!(typed_read_handoff(0,size,17,Some(view)),BLOCK_ERR_BAD_LEN);}
        assert_eq!(typed_read_handoff(1,4096,17,Some(view)),BLOCK_ERR_BAD_LEN);
        assert_eq!(typed_read_handoff(u64::MAX-511,4096,17,Some(view)),BLOCK_ERR_BAD_LEN);
        assert_eq!(typed_read_handoff(0,4096,99,Some(view)),BLOCK_ERR_NO_DEVICE);
        assert_eq!(typed_read_handoff(0,4096,17,None),BLOCK_ERR_NO_DEVICE);
        assert_eq!(typed_read_handoff(0,4096,17,Some(RegionView{length:512,..view})),BLOCK_ERR_NO_DEVICE);
        assert_eq!(typed_read_handoff(0,4096,17,Some(RegionView{virtual_address:u64::MAX-511,..view})),BLOCK_ERR_NO_DEVICE);
    }
    #[test]
    fn production_bulk_guard_checks_whole_pool_interval_and_legacy_limits(){
        for size in [1024,2048,4096]{
            assert!(write_guard_allows(400*512,size,17));
            assert!(write_guard_allows(1808*512-size,size,17));
            assert!(!write_guard_allows(1808*512-size+512,size,17));
            assert!(!write_guard_allows(399*512,size,17));
            assert!(!write_guard_allows(1808*512,size,17));
            assert!(!write_guard_allows(2046*512,size,17));
            assert!(!write_guard_allows(400*512,size,99));
        }
        for size in [0,513,8192,u64::MAX]{assert!(!write_guard_allows(400*512,size,17));}
        assert!(!write_guard_allows(u64::MAX-511,4096,17));
        assert!(!write_guard_allows(400*512+1,4096,17));
        assert!(write_guard_allows(2046*512,512,17));
        assert!(write_guard_allows(1808*512,512,17));
    }

}
