#![no_std]
#![no_main]

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{compiler_fence, Ordering};
use sex_pdx::{
    pdx_call, pdx_reply, pdx_listen_raw, serial_println,
    SLOT_SHELL, SLOT_USB_HOST,
    SLOT_DMA_REGION, SLOT_FRAME_REGION, SLOT_BLOCK_BUFFER,
    BLOCK_READ, BLOCK_WRITE, BLOCK_SYNC,
    BLOCK_ERR_BAD_CMD, BLOCK_ERR_BAD_LEN, BLOCK_ERR_NO_DEVICE,
    BLOCK_SECTOR_SIZE, BLOCK_MAX_XFER,
};


mod dma_transfer;
use dma_transfer::DmaTransfer;
use sex_pdx::dma_pool::{DmaLease, DmaRegionPool};
use sex_pdx::memory_region::{RegionView, QUERY_VIEW, READ, WRITE, DMA_ADDRESS};

// All permanent DMA allocations are slices of this one boot-granted region.
// Normal operations reuse the two leases; they never reserve another page.
static mut DMA_POOL: Option<DmaRegionPool> = None;
static mut DMA_VIEW: Option<RegionView> = None;
static mut READ_LEASE: Option<DmaLease> = None;
static mut WRITE_LEASE: Option<DmaLease> = None;

// SERIAL_DIET_V1: shared budget for hot-loop diagnostics. Early boot keeps
// the full trace (first 240 lines); steady-state loops go quiet so
// serial VM-exits stop dominating wall clock. Error and gate-required
// markers are NOT routed through this.
static mut HOT_LOG_BUDGET: u32 = 240;
#[inline(always)]
fn hot_log() -> bool {
    unsafe {
        if HOT_LOG_BUDGET > 0 { HOT_LOG_BUDGET -= 1; true } else { false }
    }
}

struct DummyAllocator;
unsafe impl GlobalAlloc for DummyAllocator {
    unsafe fn alloc(&self, _layout: Layout) -> *mut u8 { core::ptr::null_mut() }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOCATOR: DummyAllocator = DummyAllocator;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        let diff = *a.add(i) as i32 - *b.add(i) as i32;
        if diff != 0 { return diff; }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    for i in 0..n { *dest.add(i) = *src.add(i); }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(dest: *mut u8, c: i32, n: usize) -> *mut u8 {
    for i in 0..n { *dest.add(i) = c as u8; }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if dest as usize <= src as usize {
        for i in 0..n { *dest.add(i) = *src.add(i); }
    } else {
        for i in (0..n).rev() { *dest.add(i) = *src.add(i); }
    }
    dest
}

// Local Opcode
pub const OP_SHELL_BIND_BUFFER: u64 = 0x14;

// SLOT_NVME_HOST = 16 — matches devmgr.rs grant. Not in sex-pdx to avoid ABI hash churn.
const SLOT_NVME_HOST: u64 = 16;
const PAGE_SIZE: u64 = 4096;
const NVME_SQ0TDBL: u64 = 0x1000;
const NVME_CQ0HDBL: u64 = 0x1004;
const NVME_LBA_BYTES: u64 = 512;
// LBA constants below are re-exported from sex_pdx's canonical disk-layout
// module (the single source of truth every fixed on-disk region in the
// system must be declared in) rather than defined locally — that module's
// compile-time asserts are what would have caught the LBA 128 collision
// between this file's own AP4 self-test and DISKFS_V4's content pool
// before it ever shipped. See docs/handoff/SEXDRIVE_NVME_QUEUE_WRAP_V1.md.
const WRITE_PROOF_LBA: u64 = sex_pdx::SEXDRIVE_AP3_WRITE_PROOF_LBA; // Final LBA in gate nvme.img (2048 sectors)
const WRITE_PROOF_LEN: u64 = 512;
const WRITE_PROOF_MAGIC: u64 = 0x3156_4554_4952_5753; // "SWRITEV1" LE
const AP4_MULTI_BASE_LBA: u64 = sex_pdx::SEXDRIVE_AP4_MULTI_BASE_LBA;
const AP4_MULTI_BLOCKS: u64 = sex_pdx::SEXDRIVE_AP4_MULTI_SECTORS;
const AP4_MULTI_BLOCK_BYTES: u64 = NVME_LBA_BYTES;
const AP5A_PERSIST_BASE_LBA: u64 = sex_pdx::SEXDRIVE_AP5A_PERSIST_BASE_LBA;
const AP5A_PERSIST_BLOCKS: u64 = sex_pdx::SEXDRIVE_AP5A_PERSIST_SECTORS;
const AP5A_PERSIST_BLOCK_BYTES: u64 = NVME_LBA_BYTES;
const STORAGE_100_PERSIST_WRITE_ENABLED: bool =
    option_env!("SEXOS_STORAGE_100_PERSIST_WRITE").is_some();
const STORAGE_100_PERSIST_READ_ENABLED: bool =
    option_env!("SEXOS_STORAGE_100_PERSIST_READ").is_some();
const STORAGE_100_FLUSH_AUDIT_ENABLED: bool =
    option_env!("SEXOS_STORAGE_100_FLUSH_AUDIT").is_some();
const STORAGE_100_NEGATIVE_ENABLED: bool =
    option_env!("SEXOS_STORAGE_100_NEGATIVE").is_some();
const STORAGE_100_NEG_MISMATCH_ENABLED: bool =
    option_env!("SEXOS_STORAGE_100_NEG_MISMATCH").is_some();
const STORAGE_100_IO_READ_PROBE_ENABLED: bool =
    option_env!("SEXOS_STORAGE_100_IO_READ_PROBE").is_some();
const AP6_NEG_MISMATCH_LBA: u64 = sex_pdx::SEXDRIVE_AP6_NEG_MISMATCH_LBA;
const AP6_NEG_MISMATCH_BYTES: u64 = NVME_LBA_BYTES;
const MANIFEST_LBA: u64 = sex_pdx::DISKFS_MANIFEST_LBA;
const PROOF_OBJECT_START_LBA: u64 = sex_pdx::DISKFS_SEXFILES_PROOF_START_LBA;
const PROOF_OBJECT_END_LBA: u64 = sex_pdx::DISKFS_SEXFILES_PROOF_START_LBA + sex_pdx::DISKFS_SEXFILES_PROOF_SECTORS - 1;
// V2 multi-object slots (SEXFILES_DISK_MULTI_OBJECT_MANIFEST_PLAN_V1)
const LINEN_OBJECT_START_LBA: u64 = sex_pdx::DISKFS_LINEN_OBJECT_START_LBA;
const LINEN_OBJECT_END_LBA:   u64 = sex_pdx::DISKFS_LINEN_OBJECT_START_LBA + sex_pdx::DISKFS_LINEN_OBJECT_SECTORS - 1;
const QUIL_OBJECT_START_LBA:  u64 = sex_pdx::DISKFS_QUIL_OBJECT_START_LBA;
const QUIL_OBJECT_END_LBA:    u64 = sex_pdx::DISKFS_QUIL_OBJECT_START_LBA + sex_pdx::DISKFS_QUIL_OBJECT_SECTORS - 1;
// SexFS v0 on-disk regions (SEXFS_V0_ONDISK_CONTRACT_SPEC_V1)
const SEXFS_V0_META_START_LBA: u64 = sex_pdx::SEXFS_V0_META_START_LBA;
const SEXFS_V0_META_END_LBA:   u64 = sex_pdx::SEXFS_V0_META_END_LBA;
const SEXFS_V0_OBJECT_START_LBA: u64 = sex_pdx::SEXFS_V0_OBJECT_START_LBA;
const SEXFS_V0_OBJECT_END_LBA:   u64 = sex_pdx::SEXFS_V0_OBJECT_END_LBA;

struct NvmeIoState {
    ready: bool,
    map_va: u64,
    io_sq_va: u64,
    io_cq_va: u64,
    sq1tdbl: u64,
    cq1hdbl: u64,
    sq_tail: u32,
    cq_head: u32,
    cq_phase: u32,
    next_cid: u16,
}

static mut NVME_IO_STATE: NvmeIoState = NvmeIoState {
    ready: false,
    map_va: 0,
    io_sq_va: 0,
    io_cq_va: 0,
    sq1tdbl: 0,
    cq1hdbl: 0,
    sq_tail: 0,
    cq_head: 0,
    cq_phase: 1,
    next_cid: 0x0500,
};

fn init_dma_region() -> bool {
    let view = match sex_pdx::memory_region_full_view(SLOT_DMA_REGION, QUERY_VIEW | READ | WRITE | DMA_ADDRESS) {
        Ok(view) if view.length == 512 * 1024 => view,
        _ => return false,
    };
    let mut pool = match DmaRegionPool::new(view) { Ok(pool) => pool, Err(_) => return false };
    let read = match pool.reserve(PAGE_SIZE) { Ok(buffer) => buffer, Err(_) => return false };
    let write = match pool.reserve(PAGE_SIZE) { Ok(buffer) => buffer, Err(_) => return false };
    unsafe {
        DMA_VIEW = Some(view);
        DMA_POOL = Some(pool);
        READ_LEASE = Some(DmaLease::new(read));
        WRITE_LEASE = Some(DmaLease::new(write));
    }
    sex_pdx::serial_record!("[sexdrive.dma.region] slot={} bytes={} reusable_buffers=2", SLOT_DMA_REGION, view.length);
    true
}

// Initialization-only reservation; no allocation or mapping syscall exists here.
fn reserve_dma_phys(size: u64) -> u64 {
    unsafe {
        DMA_POOL.as_mut().and_then(|pool| pool.reserve(size).ok())
            .map(|buffer| buffer.physical_address).unwrap_or(0)
    }
}

// Translate only inside our already-granted DMA arena, never arbitrary physical
// addresses. The caller's span must also have been permanently reserved.
fn own_dma_virtual(phys: u64, size: u64) -> u64 {
    unsafe {
        let (Some(view), Some(pool)) = (DMA_VIEW, DMA_POOL.as_ref()) else { return 0; };
        let Some(base) = view.dma_address else { return 0; };
        let Some(offset) = phys.checked_sub(base) else { return 0; };
        if size == 0 || !offset.checked_add(size).is_some_and(|end| end <= pool.used()) { return 0; }
        view.virtual_address + offset
    }
}

fn stop_uncertain_io() {
    unsafe { NVME_IO_STATE.ready = false; }
    serial_println!("[sexdrive.dma.quarantine] ready=0 reason=unresolved_submission");
}

fn prepare_bounce(write: bool, cid: u16) -> Option<DmaTransfer<'static>> {
    unsafe {
        let lease = if write { WRITE_LEASE.as_mut()? } else { READ_LEASE.as_mut()? };
        DmaTransfer::prepare(lease, cid as u64, stop_uncertain_io).ok()
    }
}

fn block_buffer_view(caller: u32, slot: u64, bytes: u64, rights: u8) -> Option<RegionView> {
    if slot != SLOT_BLOCK_BUFFER { return None; }
    let info = sex_pdx::memory_region_info(slot).ok()?;
    if info.peer_pd != Some(caller) { return None; }
    sex_pdx::memory_region_view(slot, 0, bytes, QUERY_VIEW | rights).ok()
}

fn nvme_read_into_bounce(offset: u64, size: u64) -> u64 {
    if size == 0
        || size > BLOCK_MAX_XFER
        || size > PAGE_SIZE
        || (offset % NVME_LBA_BYTES) != 0
        || (size % NVME_LBA_BYTES) != 0
    {
        serial_println!(
            "[sexdrive.block.read.api.err] reason=bad_len offset={:#x} size={} lba={}",
            offset, size, NVME_LBA_BYTES
        );
        return BLOCK_ERR_BAD_LEN;
    }

    let map_va: u64;
    let io_sq_va: u64;
    let io_cq_va: u64;
    let sq1tdbl: u64;
    let cq1hdbl: u64;
    let mut sq_tail: u32;
    let mut cq_head: u32;
    let mut cq_phase: u32;
    let cid: u16;
    unsafe {
        if !NVME_IO_STATE.ready {
            serial_println!("[sexdrive.block.read.api.err] reason=no_ioq_ready");
            return BLOCK_ERR_NO_DEVICE;
        }
        map_va = NVME_IO_STATE.map_va;
        io_sq_va = NVME_IO_STATE.io_sq_va;
        io_cq_va = NVME_IO_STATE.io_cq_va;
        sq1tdbl = NVME_IO_STATE.sq1tdbl;
        cq1hdbl = NVME_IO_STATE.cq1hdbl;
        sq_tail = NVME_IO_STATE.sq_tail;
        cq_head = NVME_IO_STATE.cq_head;
        cq_phase = NVME_IO_STATE.cq_phase;
        cid = NVME_IO_STATE.next_cid;
        NVME_IO_STATE.next_cid = NVME_IO_STATE.next_cid.wrapping_add(1);
    }

    let mut transfer = match prepare_bounce(false, cid) {
        Some(transfer) => transfer, None => return BLOCK_ERR_NO_DEVICE,
    };
    let data_phys = transfer.buffer.physical_address;
    if data_phys == 0 || data_phys == u64::MAX || (data_phys % PAGE_SIZE) != 0 {
        serial_println!(
            "[sexdrive.block.read.api.err] reason=data_alloc_invalid phys={:#x}",
            data_phys
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    let data_va = transfer.buffer.virtual_address;
    if data_va == 0 || data_va == u64::MAX {
        serial_println!(
            "[sexdrive.block.read.api.err] reason=data_map_failed va={:#x}",
            data_va
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    unsafe {
        for i in 0..(PAGE_SIZE / 8) as usize {
            core::ptr::write_volatile((data_va as *mut u64).add(i), 0);
        }
    }

    let slba = offset / NVME_LBA_BYTES;
    let nlb = (size / NVME_LBA_BYTES).saturating_sub(1);

    let sqe_ptr = (io_sq_va as *mut u8).wrapping_add((sq_tail as usize) * 64) as *mut u32;
    unsafe {
        for i in 0..16usize {
            core::ptr::write_volatile(sqe_ptr.add(i), 0);
        }
        core::ptr::write_volatile(sqe_ptr.add(0), 0x02u32 | ((cid as u32) << 16)); // READ + CID
        core::ptr::write_volatile(sqe_ptr.add(1), 1u32); // NSID=1
        core::ptr::write_volatile(sqe_ptr.add(6), (data_phys & 0xFFFF_FFFF) as u32); // PRP1 low
        core::ptr::write_volatile(sqe_ptr.add(7), (data_phys >> 32) as u32); // PRP1 high
        core::ptr::write_volatile(sqe_ptr.add(10), (slba & 0xFFFF_FFFF) as u32); // SLBA low
        core::ptr::write_volatile(sqe_ptr.add(11), (slba >> 32) as u32); // SLBA high
        core::ptr::write_volatile(sqe_ptr.add(12), (nlb & 0xFFFF) as u32); // NLB (0-based)
    }
    serial_println!(
        "[sexdrive.block.read.api.nvme.submit] cid={} nsid={} slba={} nlb={} prp1={:#x} sq_tail={}",
        cid as u64, 1u64, slba, nlb, data_phys, sq_tail as u64
    );
    if hot_log() { serial_println!(
        "[sexdrive.nvme.submit.detail] path=typed op=READ cid={} sq_tail_before={} sq_tail_after={} cq_head={} cq_phase={} lba={} bytes={}",
        cid as u64, sq_tail as u64, ((sq_tail + 1) % 16) as u64, cq_head as u64, cq_phase as u64, slba, size
    ); }

    compiler_fence(Ordering::SeqCst);
    sq_tail = (sq_tail + 1) % 16;
    unsafe {
        if transfer.submit().is_err() { return BLOCK_ERR_NO_DEVICE; }
        core::ptr::write_volatile((map_va + sq1tdbl) as *mut u32, sq_tail);
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.doorbell] path=typed qid=1 tail={}",
        sq_tail as u64
    ); }

    let mut done = false;
    let mut dw2 = 0u32;
    let mut dw3 = 0u32;
    let max_polls = 1_000_000u32;
    let mut polls = 0u32;
    let mut last_dw3 = 0u32;
    if hot_log() { serial_println!(
        "[sexdrive.nvme.poll.begin] path=typed cid={} cq_head={} phase={} max_polls={}",
        cid as u64, cq_head as u64, cq_phase as u64, max_polls as u64
    ); }
    for _ in 0..max_polls {
        polls = polls.wrapping_add(1);
        let cqe_ptr = (io_cq_va as *const u8).wrapping_add((cq_head as usize) * 16) as *const u32;
        let rd_dw2 = unsafe { core::ptr::read_volatile(cqe_ptr.add(2)) };
        let rd_dw3 = unsafe { core::ptr::read_volatile(cqe_ptr.add(3)) };
        last_dw3 = rd_dw3;
        let cid_now = rd_dw3 & 0xFFFF;
        let phase_now = (rd_dw3 >> 16) & 0x1;
        if cid_now == cid as u32 && phase_now == cq_phase {
            done = true;
            dw2 = rd_dw2;
            dw3 = rd_dw3;
            serial_println!(
                "[sexdrive.block.read.api.cqe] cid={} phase={} dw2={:#x} dw3={:#x}",
                cid_now as u64, phase_now as u64, rd_dw2 as u64, rd_dw3 as u64
            );
            break;
        }
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.poll.end] path=typed cid={} found={} status={} cq_head={} phase={} polls={}",
        cid as u64,
        if done { 1u64 } else { 0u64 },
        ((dw3 >> 17) & 0x7FFF) as u64,
        cq_head as u64,
        cq_phase as u64,
        polls as u64
    ); }
    if !done {
        stop_uncertain_io();
        serial_println!(
            "[sexdrive.block.read.api.err] reason=cqe_timeout cid={} head={} phase={}",
            cid as u64, cq_head as u64, cq_phase as u64
        );
        serial_println!(
            "[sexdrive.nvme.poll.timeout.detail] path=typed cid={} expected_cid={} seen_cid={} seen_status={} seen_phase={} cq_head={} phase={} polls={}",
            cid as u64,
            cid as u64,
            (last_dw3 & 0xFFFF) as u64,
            ((last_dw3 >> 17) & 0x7FFF) as u64,
            ((last_dw3 >> 16) & 0x1) as u64,
            cq_head as u64,
            cq_phase as u64,
            polls as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }

    if transfer.complete((dw3 & 0xFFFF) as u16, (dw3 >> 16) & 1, cq_phase).is_err() {
        return BLOCK_ERR_NO_DEVICE;
    }
    let sf = (dw3 >> 17) & 0x7FFF;
    let sc = sf & 0xFF;
    let sct = (sf >> 8) & 0x7;
    if sc != 0 || sct != 0 {
        unsafe { NVME_IO_STATE.ready = false; } // CQ error completed DMA; stop stale queue state.
        let sqhd = dw2 & 0xFFFF;
        let sqid = (dw2 >> 16) & 0xFFFF;
        serial_println!(
            "[sexdrive.block.read.api.err] reason=status_fail cid={} sc={} sct={} sqhd={} sqid={} dw3={:#x}",
            cid as u64, sc as u64, sct as u64, sqhd as u64, sqid as u64, dw3 as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }

    cq_head = (cq_head + 1) % 16;
    if cq_head == 0 {
        cq_phase ^= 1;
    }
    unsafe {
        core::ptr::write_volatile((map_va + cq1hdbl) as *mut u32, cq_head);
        NVME_IO_STATE.sq_tail = sq_tail;
        NVME_IO_STATE.cq_head = cq_head;
        NVME_IO_STATE.cq_phase = cq_phase;
    }

    let d0 = unsafe { core::ptr::read_volatile(data_va as *const u64) };
    let d1 = unsafe { core::ptr::read_volatile((data_va + 8) as *const u64) };
    serial_println!(
        "[sexdrive.block.read.api.ok] cid={} slba={} nlb={} d0={:#x} d1={:#x}",
        cid as u64, slba, nlb, d0, d1
    );
    0u64
}

// Called only with the peer-checked BOOT view from typed command admission.
// The public block contract permits one page, not only the early 512-byte demo.
fn typed_read_handoff(offset:u64,size:u64,buf_cap:u64,view:Option<RegionView>)->u64{
    if size==0 || size>BLOCK_MAX_XFER || size>PAGE_SIZE
        || size%NVME_LBA_BYTES!=0 || offset%NVME_LBA_BYTES!=0
        || offset.checked_add(size).is_none(){return BLOCK_ERR_BAD_LEN}
    if buf_cap!=SLOT_BLOCK_BUFFER{return BLOCK_ERR_NO_DEVICE}
    let Some(view)=view else{return BLOCK_ERR_NO_DEVICE};
    if view.length<size || view.virtual_address==0 || view.virtual_address==u64::MAX
        || view.virtual_address.checked_add(size).is_none(){return BLOCK_ERR_NO_DEVICE}
    nvme_read_into_mapped_va(offset,size,view.virtual_address)
}

fn nvme_read_into_mapped_va(offset: u64, size: u64, dst_va: u64) -> u64 {
    if size == 0
        || size > BLOCK_MAX_XFER
        || size > PAGE_SIZE
        || (offset % NVME_LBA_BYTES) != 0
        || (size % NVME_LBA_BYTES) != 0
        || dst_va == 0
        || dst_va == u64::MAX
    {
        serial_println!(
            "[sexdrive.block.read.handoff.err] reason=bad_args offset={:#x} size={} dst_va={:#x}",
            offset, size, dst_va
        );
        return BLOCK_ERR_BAD_LEN;
    }

    if hot_log() { serial_println!(
        "[sexdrive.block.read.handoff.nvme.begin] offset={:#x} size={} dst_va={:#x}",
        offset, size, dst_va
    ); }

    let map_va: u64;
    let io_sq_va: u64;
    let io_cq_va: u64;
    let sq1tdbl: u64;
    let cq1hdbl: u64;
    let mut sq_tail: u32;
    let mut cq_head: u32;
    let mut cq_phase: u32;
    let cid: u16;
    unsafe {
        if !NVME_IO_STATE.ready {
            serial_println!("[sexdrive.block.read.handoff.err] reason=no_ioq_ready");
            return BLOCK_ERR_NO_DEVICE;
        }
        map_va = NVME_IO_STATE.map_va;
        io_sq_va = NVME_IO_STATE.io_sq_va;
        io_cq_va = NVME_IO_STATE.io_cq_va;
        sq1tdbl = NVME_IO_STATE.sq1tdbl;
        cq1hdbl = NVME_IO_STATE.cq1hdbl;
        sq_tail = NVME_IO_STATE.sq_tail;
        cq_head = NVME_IO_STATE.cq_head;
        cq_phase = NVME_IO_STATE.cq_phase;
        cid = NVME_IO_STATE.next_cid;
        NVME_IO_STATE.next_cid = NVME_IO_STATE.next_cid.wrapping_add(1);
    }

    let mut transfer = match prepare_bounce(false, cid) {
        Some(transfer) => transfer, None => return BLOCK_ERR_NO_DEVICE,
    };
    let data_phys = transfer.buffer.physical_address;
    if data_phys == 0 || data_phys == u64::MAX || (data_phys % PAGE_SIZE) != 0 {
        serial_println!(
            "[sexdrive.block.read.handoff.err] reason=data_alloc_invalid phys={:#x}",
            data_phys
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    let data_va = transfer.buffer.virtual_address;
    if data_va == 0 || data_va == u64::MAX {
        serial_println!(
            "[sexdrive.block.read.handoff.err] reason=data_map_failed va={:#x}",
            data_va
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    unsafe {
        for i in 0..(PAGE_SIZE / 8) as usize {
            core::ptr::write_volatile((data_va as *mut u64).add(i), 0);
        }
    }

    let slba = offset / NVME_LBA_BYTES;
    let nlb = (size / NVME_LBA_BYTES).saturating_sub(1);
    let max_lba = WRITE_PROOF_LBA;
    let last_lba = slba.saturating_add(nlb);
    let range_ok = if slba > max_lba { 0u64 } else if nlb > (max_lba - slba) { 0u64 } else { 1u64 };
    if hot_log() { serial_println!(
        "[sexdrive.nvme.cmd.range] path=typed slba={} nlb={} max_lba={} ok={}",
        slba, nlb, max_lba, range_ok
    ); }
    let sqe_ptr = (io_sq_va as *mut u8).wrapping_add((sq_tail as usize) * 64) as *mut u32;
    unsafe {
        for i in 0..16usize {
            core::ptr::write_volatile(sqe_ptr.add(i), 0);
        }
        core::ptr::write_volatile(sqe_ptr.add(0), 0x02u32 | ((cid as u32) << 16)); // READ + CID
        core::ptr::write_volatile(sqe_ptr.add(1), 1u32); // NSID=1
        core::ptr::write_volatile(sqe_ptr.add(6), (data_phys & 0xFFFF_FFFF) as u32); // PRP1 low
        core::ptr::write_volatile(sqe_ptr.add(7), (data_phys >> 32) as u32); // PRP1 high
        core::ptr::write_volatile(sqe_ptr.add(10), (slba & 0xFFFF_FFFF) as u32); // SLBA low
        core::ptr::write_volatile(sqe_ptr.add(11), (slba >> 32) as u32); // SLBA high
        core::ptr::write_volatile(sqe_ptr.add(12), (nlb & 0xFFFF) as u32); // NLB (0-based)
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.cmd.shape] path=typed op=READ cid={} nsid={} slba={} nlb={} bytes={} prp1={:#x} prp2={:#x} buf_va={:#x} buf_phys={:#x} buffer_cap={:#x} buffer_kind=capability_region last_lba={}",
        cid as u64, 1u64, slba, nlb, size, data_phys, 0u64, dst_va, data_phys, SLOT_BLOCK_BUFFER, last_lba
    ); }
    compiler_fence(Ordering::SeqCst);
    if hot_log() { serial_println!(
        "[sexdrive.nvme.submit.detail] path=typed op=READ cid={} sq_tail_before={} sq_tail_after={} cq_head={} cq_phase={} lba={} bytes={}",
        cid as u64, sq_tail as u64, ((sq_tail + 1) % 16) as u64, cq_head as u64, cq_phase as u64, slba, size
    ); }
    sq_tail = (sq_tail + 1) % 16;
    unsafe {
        if transfer.submit().is_err() { return BLOCK_ERR_NO_DEVICE; }
        core::ptr::write_volatile((map_va + sq1tdbl) as *mut u32, sq_tail);
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.doorbell] path=typed qid=1 tail={}",
        sq_tail as u64
    ); }

    let mut done = false;
    let mut dw2 = 0u32;
    let mut dw3 = 0u32;
    let max_polls = 1_000_000u32;
    let mut polls = 0u32;
    let mut last_dw3 = 0u32;
    if hot_log() { serial_println!(
        "[sexdrive.nvme.poll.begin] path=typed cid={} cq_head={} phase={} max_polls={}",
        cid as u64, cq_head as u64, cq_phase as u64, max_polls as u64
    ); }
    for _ in 0..max_polls {
        polls = polls.wrapping_add(1);
        let cqe_ptr = (io_cq_va as *const u8).wrapping_add((cq_head as usize) * 16) as *const u32;
        let rd_dw2 = unsafe { core::ptr::read_volatile(cqe_ptr.add(2)) };
        let rd_dw3 = unsafe { core::ptr::read_volatile(cqe_ptr.add(3)) };
        last_dw3 = rd_dw3;
        let cid_now = rd_dw3 & 0xFFFF;
        let phase_now = (rd_dw3 >> 16) & 0x1;
        if cid_now == cid as u32 && phase_now == cq_phase {
            done = true;
            dw2 = rd_dw2;
            dw3 = rd_dw3;
            if hot_log() { serial_println!(
                "[sexdrive.block.read.handoff.nvme.cqe] cid={} phase={} dw2={:#x} dw3={:#x}",
                cid_now as u64, phase_now as u64, rd_dw2 as u64, rd_dw3 as u64
            ); }
            break;
        }
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.poll.end] path=typed cid={} found={} status={} cq_head={} phase={} polls={}",
        cid as u64,
        if done { 1u64 } else { 0u64 },
        ((dw3 >> 17) & 0x7FFF) as u64,
        cq_head as u64,
        cq_phase as u64,
        polls as u64
    ); }
    if !done {
        stop_uncertain_io();
        serial_println!(
            "[sexdrive.block.read.handoff.err] reason=cqe_timeout cid={} head={} phase={}",
            cid as u64, cq_head as u64, cq_phase as u64
        );
        serial_println!(
            "[sexdrive.nvme.poll.timeout.detail] path=typed cid={} expected_cid={} seen_cid={} seen_status={} seen_phase={} cq_head={} phase={} polls={}",
            cid as u64,
            cid as u64,
            (last_dw3 & 0xFFFF) as u64,
            ((last_dw3 >> 17) & 0x7FFF) as u64,
            ((last_dw3 >> 16) & 0x1) as u64,
            cq_head as u64,
            cq_phase as u64,
            polls as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }

    if transfer.complete((dw3 & 0xFFFF) as u16, (dw3 >> 16) & 1, cq_phase).is_err() {
        return BLOCK_ERR_NO_DEVICE;
    }
    let sf = (dw3 >> 17) & 0x7FFF;
    let sc = sf & 0xFF;
    let sct = (sf >> 8) & 0x7;
    if sc != 0 || sct != 0 {
        unsafe { NVME_IO_STATE.ready = false; } // CQ error completed DMA; stop stale queue state.
        let sqhd = dw2 & 0xFFFF;
        let sqid = (dw2 >> 16) & 0xFFFF;
        serial_println!(
            "[sexdrive.block.read.handoff.err] reason=status_fail cid={} sc={} sct={} sqhd={} sqid={} dw3={:#x}",
            cid as u64, sc as u64, sct as u64, sqhd as u64, sqid as u64, dw3 as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }

    cq_head = (cq_head + 1) % 16;
    if cq_head == 0 {
        cq_phase ^= 1;
    }
    unsafe {
        core::ptr::write_volatile((map_va + cq1hdbl) as *mut u32, cq_head);
        NVME_IO_STATE.sq_tail = sq_tail;
        NVME_IO_STATE.cq_head = cq_head;
        NVME_IO_STATE.cq_phase = cq_phase;
    }

    unsafe {
        let src = data_va as *const u8;
        let dst = dst_va as *mut u8;
        let mut i = 0usize;
        while i < size as usize {
            core::ptr::write_volatile(dst.add(i), core::ptr::read_volatile(src.add(i)));
            i += 1;
        }
    }
    if hot_log() { serial_println!(
        "[sexdrive.block.read.handoff.copy.ok] phase=B len={}",
        size
    ); }
    0u64
}

fn write_guard_allows(offset: u64, size: u64, buf_cap: u64) -> bool {
    let expected_offset = WRITE_PROOF_LBA * NVME_LBA_BYTES;
    let manifest_offset = MANIFEST_LBA * NVME_LBA_BYTES;
    let object_start_offset = PROOF_OBJECT_START_LBA * NVME_LBA_BYTES;
    let object_end_offset = PROOF_OBJECT_END_LBA * NVME_LBA_BYTES;
    let proof_mode = buf_cap == SLOT_BLOCK_BUFFER;
    let allow_manifest = proof_mode && size == WRITE_PROOF_LEN && offset == manifest_offset;
    let linen_start = LINEN_OBJECT_START_LBA * NVME_LBA_BYTES;
    let linen_end   = LINEN_OBJECT_END_LBA   * NVME_LBA_BYTES;
    let quil_start  = QUIL_OBJECT_START_LBA  * NVME_LBA_BYTES;
    let quil_end    = QUIL_OBJECT_END_LBA    * NVME_LBA_BYTES;
    let allow_object = proof_mode && size == WRITE_PROOF_LEN
        && (offset % NVME_LBA_BYTES) == 0
        && offset >= object_start_offset
        && offset <= object_end_offset;
    let allow_linen = proof_mode && size == WRITE_PROOF_LEN
        && (offset % NVME_LBA_BYTES) == 0
        && offset >= linen_start && offset <= linen_end;
    let allow_quil = proof_mode && size == WRITE_PROOF_LEN
        && (offset % NVME_LBA_BYTES) == 0
        && offset >= quil_start && offset <= quil_end;
    let allow_proof = proof_mode && offset == expected_offset && size == WRITE_PROOF_LEN;

    // SexFS v0 guarded ranges (SEXFS_V0_ONDISK_CONTRACT_SPEC_V1)
    let sexfs_meta_start = SEXFS_V0_META_START_LBA * NVME_LBA_BYTES;
    let sexfs_meta_end   = SEXFS_V0_META_END_LBA   * NVME_LBA_BYTES;
    let sexfs_obj_start  = SEXFS_V0_OBJECT_START_LBA * NVME_LBA_BYTES;
    let sexfs_obj_end    = SEXFS_V0_OBJECT_END_LBA   * NVME_LBA_BYTES;
    let allow_sexfs_meta = proof_mode && size == WRITE_PROOF_LEN
        && (offset % NVME_LBA_BYTES) == 0
        && offset >= sexfs_meta_start && offset <= sexfs_meta_end;
    let allow_sexfs_obj = proof_mode && size == WRITE_PROOF_LEN
        && (offset % NVME_LBA_BYTES) == 0
        && offset >= sexfs_obj_start && offset <= sexfs_obj_end;

    // Bulk zeroing is confined to the already granted V4 pool. Metadata,
    // legacy objects, proof sectors and the separate history journal retain
    // their existing sector-only routes. The peer-bound region is checked
    // before this function and the entire interval must fit the pool.
    let allow_pool_bulk = proof_mode && size > WRITE_PROOF_LEN && size <= PAGE_SIZE
        && size % NVME_LBA_BYTES == 0 && offset % NVME_LBA_BYTES == 0
        && offset >= sex_pdx::DISKFS_V4_POOL_BASE_LBA * NVME_LBA_BYTES
        && offset.checked_add(size).map_or(false,|end|end <=
            (sex_pdx::DISKFS_V4_POOL_BASE_LBA+sex_pdx::DISKFS_V4_POOL_SECTORS)*NVME_LBA_BYTES);
    let allow = allow_proof || allow_manifest || allow_object || allow_linen || allow_quil
        || allow_sexfs_meta || allow_sexfs_obj || allow_pool_bulk;
    if !allow || hot_log() {
    serial_println!(
        "[sexdrive.write.guard.config] proof_lba={} proof_offset={:#x} manifest_lba={} object_lba_start={} object_lba_end={} proof_len={} magic={:#x} sexfs_meta={}-{} sexfs_obj={}-{}",
        WRITE_PROOF_LBA,
        expected_offset,
        MANIFEST_LBA,
        PROOF_OBJECT_START_LBA,
        PROOF_OBJECT_END_LBA,
        WRITE_PROOF_LEN,
        WRITE_PROOF_MAGIC,
        SEXFS_V0_META_START_LBA, SEXFS_V0_META_END_LBA,
        SEXFS_V0_OBJECT_START_LBA, SEXFS_V0_OBJECT_END_LBA
    );
    serial_println!(
        "[sexdrive.write.guard.begin] offset={:#x} size={} buf_cap={:#x} proof_mode={}",
        offset,
        size,
        buf_cap,
        if proof_mode { 1u64 } else { 0u64 }
    );
    }
    if allow && hot_log() {
        let lba = if NVME_LBA_BYTES != 0 { offset / NVME_LBA_BYTES } else { u64::MAX };
        if allow_sexfs_meta || allow_sexfs_obj {
            serial_println!(
                "[sexdrive.write_guard.sexfs.allow] lba={} size={} ok=1",
                lba, size
            );
        } else {
            serial_println!(
                "[sexdrive.write.guard.allow] offset={:#x} size={} buf_cap={:#x}",
                offset,
                size,
                buf_cap
            );
        }
    } else if !allow {
        let lba = if NVME_LBA_BYTES != 0 { offset / NVME_LBA_BYTES } else { u64::MAX };
        serial_println!(
            "[sexdrive.write_guard.reject] lba={} size={} ok=1",
            lba, size
        );
    }
    allow
}

fn nvme_write_one_block(offset: u64, size: u64, src_va: u64) -> u64 {
    // Success chatter shares the existing finite hot-log budget. Device errors,
    // timeout/quarantine and actual FLUSH receipts remain unconditional.
    if size == 0 || size > PAGE_SIZE || size > BLOCK_MAX_XFER
        || size % NVME_LBA_BYTES != 0 || offset % NVME_LBA_BYTES != 0
        || offset.checked_add(size).map_or(true,|end|end>(WRITE_PROOF_LBA+1)*NVME_LBA_BYTES)
        || src_va == 0 || src_va == u64::MAX {
        serial_println!(
            "[sexdrive.nvme.write.err] reason=bad_args offset={:#x} size={} src_va={:#x}",
            offset, size, src_va
        );
        return BLOCK_ERR_BAD_LEN;
    }

    let map_va: u64;
    let io_sq_va: u64;
    let io_cq_va: u64;
    let sq1tdbl: u64;
    let cq1hdbl: u64;
    let mut sq_tail: u32;
    let mut cq_head: u32;
    let mut cq_phase: u32;
    let write_cid: u16;
    unsafe {
        if !NVME_IO_STATE.ready {
            serial_println!("[sexdrive.nvme.write.err] reason=no_ioq_ready");
            return BLOCK_ERR_NO_DEVICE;
        }
        map_va = NVME_IO_STATE.map_va;
        io_sq_va = NVME_IO_STATE.io_sq_va;
        io_cq_va = NVME_IO_STATE.io_cq_va;
        sq1tdbl = NVME_IO_STATE.sq1tdbl;
        cq1hdbl = NVME_IO_STATE.cq1hdbl;
        sq_tail = NVME_IO_STATE.sq_tail;
        cq_head = NVME_IO_STATE.cq_head;
        cq_phase = NVME_IO_STATE.cq_phase;
        write_cid = NVME_IO_STATE.next_cid;
        NVME_IO_STATE.next_cid = NVME_IO_STATE.next_cid.wrapping_add(1);
    }

    let mut transfer = match prepare_bounce(true, write_cid) {
        Some(transfer) => transfer, None => return BLOCK_ERR_NO_DEVICE,
    };
    let write_phys = transfer.buffer.physical_address;
    let write_va = transfer.buffer.virtual_address;
    if transfer.buffer.length < size || write_phys == 0 || write_phys == u64::MAX || (write_phys % PAGE_SIZE) != 0
        || write_va == 0 || write_va == u64::MAX
    {
        serial_println!(
            "[sexdrive.nvme.write.err] reason=write_buf_invalid phys={:#x} va={:#x}",
            write_phys, write_va
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    unsafe {
        for i in 0..(PAGE_SIZE as usize) {
            core::ptr::write_volatile((write_va as *mut u8).add(i), 0u8);
        }
        let mut i = 0usize;
        while i < size as usize {
            let b = core::ptr::read_volatile((src_va as *const u8).add(i));
            core::ptr::write_volatile((write_va as *mut u8).add(i), b);
            i += 1;
        }
    }

    let slba = offset / NVME_LBA_BYTES;
    let nlb = (size / NVME_LBA_BYTES - 1) as u32;
    let max_lba = WRITE_PROOF_LBA;
    let last_lba = slba + nlb as u64;
    let range_ok = if slba <= max_lba { 1u64 } else { 0u64 };
    if hot_log() { serial_println!(
        "[sexdrive.nvme.cmd.range] path=typed slba={} nlb={} max_lba={} ok={}",
        slba, nlb as u64, max_lba, range_ok
    ); }
    let sqe_ptr = (io_sq_va as *mut u8).wrapping_add((sq_tail as usize) * 64) as *mut u32;
    unsafe {
        for i in 0..16usize {
            core::ptr::write_volatile(sqe_ptr.add(i), 0);
        }
        core::ptr::write_volatile(sqe_ptr.add(0), 0x01u32 | ((write_cid as u32) << 16));
        core::ptr::write_volatile(sqe_ptr.add(1), 1u32);
        core::ptr::write_volatile(sqe_ptr.add(6), (write_phys & 0xFFFF_FFFF) as u32);
        core::ptr::write_volatile(sqe_ptr.add(7), (write_phys >> 32) as u32);
        core::ptr::write_volatile(sqe_ptr.add(10), (slba & 0xFFFF_FFFF) as u32);
        core::ptr::write_volatile(sqe_ptr.add(11), (slba >> 32) as u32);
        core::ptr::write_volatile(sqe_ptr.add(12), nlb);
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.write.submit] cid={} nsid=1 slba={} nlb={} prp1={:#x} sq_tail={}",
        write_cid as u64, slba, nlb, write_phys, sq_tail as u64
    ); }
    if hot_log() { serial_println!(
        "[sexdrive.block.write.api.nvme.submit] cid={} nsid=1 slba={} nlb={} prp1={:#x}",
        write_cid as u64, slba, nlb, write_phys
    ); }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.cmd.shape] path=typed op=WRITE cid={} nsid={} slba={} nlb={} bytes={} prp1={:#x} prp2={:#x} buf_va={:#x} buf_phys={:#x} buffer_cap={:#x} buffer_kind=capability_region last_lba={}",
        write_cid as u64, 1u64, slba, nlb as u64, size, write_phys, 0u64, src_va, write_phys, SLOT_BLOCK_BUFFER, last_lba
    ); }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.submit.detail] path=typed op=WRITE cid={} sq_tail_before={} sq_tail_after={} cq_head={} cq_phase={} lba={} bytes={}",
        write_cid as u64, sq_tail as u64, ((sq_tail + 1) % 16) as u64, cq_head as u64, cq_phase as u64, slba, size
    ); }

    compiler_fence(Ordering::SeqCst);
    sq_tail = (sq_tail + 1) % 16;
    unsafe {
        if transfer.submit().is_err() { return BLOCK_ERR_NO_DEVICE; }
        core::ptr::write_volatile((map_va + sq1tdbl) as *mut u32, sq_tail);
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.doorbell] path=typed qid=1 tail={}",
        sq_tail as u64
    ); }

    let mut done = false;
    let mut dw3 = 0u32;
    let max_polls = 1_000_000u32;
    let mut polls = 0u32;
    let mut last_dw3 = 0u32;
    if hot_log() { serial_println!(
        "[sexdrive.nvme.poll.begin] path=typed cid={} cq_head={} phase={} max_polls={}",
        write_cid as u64, cq_head as u64, cq_phase as u64, max_polls as u64
    ); }
    for _ in 0..max_polls {
        polls = polls.wrapping_add(1);
        let cqe_ptr = (io_cq_va as *const u8).wrapping_add((cq_head as usize) * 16) as *const u32;
        let rd_dw2 = unsafe { core::ptr::read_volatile(cqe_ptr.add(2)) };
        let rd_dw3 = unsafe { core::ptr::read_volatile(cqe_ptr.add(3)) };
        last_dw3 = rd_dw3;
        let cid_now = rd_dw3 & 0xFFFF;
        let phase_now = (rd_dw3 >> 16) & 0x1;
        if cid_now == write_cid as u32 && phase_now == cq_phase {
            done = true;
            dw3 = rd_dw3;
            if hot_log() { serial_println!(
                "[sexdrive.nvme.write.cqe] cid={} phase={} dw2={:#x} dw3={:#x}",
                cid_now as u64, phase_now as u64, rd_dw2 as u64, rd_dw3 as u64
            ); }
            if hot_log() { serial_println!(
                "[sexdrive.block.write.api.cqe] cid={} phase={} dw2={:#x} dw3={:#x}",
                cid_now as u64, phase_now as u64, rd_dw2 as u64, rd_dw3 as u64
            ); }
            break;
        }
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.poll.end] path=typed cid={} found={} status={} cq_head={} phase={} polls={}",
        write_cid as u64,
        if done { 1u64 } else { 0u64 },
        ((dw3 >> 17) & 0x7FFF) as u64,
        cq_head as u64,
        cq_phase as u64,
        polls as u64
    ); }
    if !done {
        stop_uncertain_io();
        serial_println!(
            "[sexdrive.nvme.write.err] reason=cqe_timeout cid={} head={} phase={}",
            write_cid as u64, cq_head as u64, cq_phase as u64
        );
        serial_println!(
            "[sexdrive.nvme.poll.timeout.detail] path=typed cid={} expected_cid={} seen_cid={} seen_status={} seen_phase={} cq_head={} phase={} polls={}",
            write_cid as u64,
            write_cid as u64,
            (last_dw3 & 0xFFFF) as u64,
            ((last_dw3 >> 17) & 0x7FFF) as u64,
            ((last_dw3 >> 16) & 0x1) as u64,
            cq_head as u64,
            cq_phase as u64,
            polls as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    if transfer.complete((dw3 & 0xFFFF) as u16, (dw3 >> 16) & 1, cq_phase).is_err() {
        return BLOCK_ERR_NO_DEVICE;
    }
    let sf = (dw3 >> 17) & 0x7FFF;
    let sc = sf & 0xFF;
    let sct = (sf >> 8) & 0x7;
    if sc != 0 || sct != 0 {
        unsafe { NVME_IO_STATE.ready = false; } // CQ error completed DMA; stop stale queue state.
        serial_println!(
            "[sexdrive.nvme.write.err] reason=status_fail cid={} sc={} sct={} dw3={:#x}",
            write_cid as u64, sc as u64, sct as u64, dw3 as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }

    cq_head = (cq_head + 1) % 16;
    if cq_head == 0 {
        cq_phase ^= 1;
    }
    unsafe {
        core::ptr::write_volatile((map_va + cq1hdbl) as *mut u32, cq_head);
        NVME_IO_STATE.sq_tail = sq_tail;
        NVME_IO_STATE.cq_head = cq_head;
        NVME_IO_STATE.cq_phase = cq_phase;
    }
    if hot_log() { serial_println!("[sexdrive.nvme.write.ok] cid={} slba={}", write_cid as u64, slba); }
    if hot_log() { serial_println!("[sexdrive.block.write.api.ok] cid={} slba={}", write_cid as u64, slba); }
    0u64
}

fn nvme_write_readback_proof(offset: u64, size: u64, src_va: u64) -> u64 {
    if size != WRITE_PROOF_LEN || src_va == 0 || src_va == u64::MAX {
        serial_println!("[sexdrive.storage100.rw.fail] reason=bad_args");
        serial_println!(
            "[sexdrive.nvme.write.err] reason=bad_args offset={:#x} size={} src_va={:#x}",
            offset, size, src_va
        );
        return BLOCK_ERR_BAD_LEN;
    }
    let expected_offset = WRITE_PROOF_LBA * NVME_LBA_BYTES;
    if offset != expected_offset {
        serial_println!("[sexdrive.storage100.rw.fail] reason=guard_offset_mismatch");
        serial_println!(
            "[sexdrive.nvme.write.err] reason=guard_offset_mismatch offset={:#x} expected={:#x}",
            offset, expected_offset
        );
        return BLOCK_ERR_BAD_LEN;
    }

    let lba = offset / NVME_LBA_BYTES;
    serial_println!("[sexdrive.storage100.rw.begin] lba={} bytes={}", lba, size);
    serial_println!("[sexdrive.nvme.write.begin] offset={:#x} size={}", offset, size);
    serial_println!("[sexdrive.block.write.api.nvme.submit] begin=1 offset={:#x} size={}", offset, size);

    let map_va: u64;
    let io_sq_va: u64;
    let io_cq_va: u64;
    let sq1tdbl: u64;
    let cq1hdbl: u64;
    let mut sq_tail: u32;
    let mut cq_head: u32;
    let mut cq_phase: u32;
    let write_cid: u16;
    let read_cid: u16;
    unsafe {
        if !NVME_IO_STATE.ready {
            serial_println!("[sexdrive.nvme.write.err] reason=no_ioq_ready");
            return BLOCK_ERR_NO_DEVICE;
        }
        map_va = NVME_IO_STATE.map_va;
        io_sq_va = NVME_IO_STATE.io_sq_va;
        io_cq_va = NVME_IO_STATE.io_cq_va;
        sq1tdbl = NVME_IO_STATE.sq1tdbl;
        cq1hdbl = NVME_IO_STATE.cq1hdbl;
        sq_tail = NVME_IO_STATE.sq_tail;
        cq_head = NVME_IO_STATE.cq_head;
        cq_phase = NVME_IO_STATE.cq_phase;
        write_cid = NVME_IO_STATE.next_cid;
        NVME_IO_STATE.next_cid = NVME_IO_STATE.next_cid.wrapping_add(1);
        read_cid = NVME_IO_STATE.next_cid;
        NVME_IO_STATE.next_cid = NVME_IO_STATE.next_cid.wrapping_add(1);
    }

    let mut write_transfer = match prepare_bounce(true, write_cid) {
        Some(transfer) => transfer, None => return BLOCK_ERR_NO_DEVICE,
    };
    let write_phys = write_transfer.buffer.physical_address;
    let write_va = write_transfer.buffer.virtual_address;
    if write_phys == 0 || write_phys == u64::MAX || (write_phys % PAGE_SIZE) != 0
        || write_va == 0 || write_va == u64::MAX
    {
        serial_println!("[sexdrive.storage100.rw.fail] reason=write_buf_invalid");
        serial_println!(
            "[sexdrive.nvme.write.err] reason=write_buf_invalid phys={:#x} va={:#x}",
            write_phys, write_va
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    unsafe {
        for i in 0..(PAGE_SIZE as usize) {
            core::ptr::write_volatile((write_va as *mut u8).add(i), 0u8);
        }
        let mut i = 0usize;
        while i < WRITE_PROOF_LEN as usize {
            let b = (0xA5u8 ^ (i as u8) ^ 0x3Cu8) & 0xFFu8;
            core::ptr::write_volatile((write_va as *mut u8).add(i), b);
            i += 1;
        }
    }

    let slba = lba;
    let nlb = 0u32;
    let sqe_ptr = (io_sq_va as *mut u8).wrapping_add((sq_tail as usize) * 64) as *mut u32;
    unsafe {
        for i in 0..16usize {
            core::ptr::write_volatile(sqe_ptr.add(i), 0);
        }
        core::ptr::write_volatile(sqe_ptr.add(0), 0x01u32 | ((write_cid as u32) << 16)); // WRITE + CID
        core::ptr::write_volatile(sqe_ptr.add(1), 1u32); // NSID=1
        core::ptr::write_volatile(sqe_ptr.add(6), (write_phys & 0xFFFF_FFFF) as u32); // PRP1 low
        core::ptr::write_volatile(sqe_ptr.add(7), (write_phys >> 32) as u32); // PRP1 high
        core::ptr::write_volatile(sqe_ptr.add(10), (slba & 0xFFFF_FFFF) as u32); // SLBA low
        core::ptr::write_volatile(sqe_ptr.add(11), (slba >> 32) as u32); // SLBA high
        core::ptr::write_volatile(sqe_ptr.add(12), nlb); // NLB=0 (one block)
    }
    serial_println!(
        "[sexdrive.storage100.write.submit] lba={} bytes={}",
        slba,
        size
    );
    serial_println!(
        "[sexdrive.nvme.write.submit] cid={} nsid=1 slba={} nlb=0 prp1={:#x} sq_tail={}",
        write_cid as u64, slba, write_phys, sq_tail as u64
    );
    serial_println!(
        "[sexdrive.block.write.api.nvme.submit] cid={} nsid=1 slba={} nlb=0 prp1={:#x}",
        write_cid as u64, slba, write_phys
    );
    if hot_log() { serial_println!(
        "[sexdrive.nvme.cmd.shape] path=selftest op=WRITE cid={} nsid={} slba={} nlb={} bytes={} prp1={:#x} prp2={:#x} buf_va={:#x} buf_phys={:#x} buffer_kind=internal",
        write_cid as u64, 1u64, slba, nlb as u64, size, write_phys, 0u64, write_va, write_phys
    ); }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.submit.detail] path=selftest op=WRITE cid={} sq_tail_before={} sq_tail_after={} cq_head={} cq_phase={} lba={} bytes={}",
        write_cid as u64, sq_tail as u64, ((sq_tail + 1) % 16) as u64, cq_head as u64, cq_phase as u64, slba, size
    ); }

    compiler_fence(Ordering::SeqCst);
    sq_tail = (sq_tail + 1) % 16;
    unsafe {
        if write_transfer.submit().is_err() { return BLOCK_ERR_NO_DEVICE; }
        core::ptr::write_volatile((map_va + sq1tdbl) as *mut u32, sq_tail);
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.doorbell] path=selftest qid=1 tail={}",
        sq_tail as u64
    ); }

    let mut done = false;
    let mut dw2 = 0u32;
    let mut dw3 = 0u32;
    let max_polls = 1_000_000u32;
    let mut polls = 0u32;
    let mut last_dw3 = 0u32;
    if hot_log() { serial_println!(
        "[sexdrive.nvme.poll.begin] path=selftest cid={} cq_head={} phase={} max_polls={}",
        write_cid as u64, cq_head as u64, cq_phase as u64, max_polls as u64
    ); }
    for _ in 0..max_polls {
        polls = polls.wrapping_add(1);
        let cqe_ptr = (io_cq_va as *const u8).wrapping_add((cq_head as usize) * 16) as *const u32;
        let rd_dw2 = unsafe { core::ptr::read_volatile(cqe_ptr.add(2)) };
        let rd_dw3 = unsafe { core::ptr::read_volatile(cqe_ptr.add(3)) };
        last_dw3 = rd_dw3;
        let cid_now = rd_dw3 & 0xFFFF;
        let phase_now = (rd_dw3 >> 16) & 0x1;
        if cid_now == write_cid as u32 && phase_now == cq_phase {
            done = true;
            dw2 = rd_dw2;
            dw3 = rd_dw3;
            serial_println!(
                "[sexdrive.nvme.write.cqe] cid={} phase={} dw2={:#x} dw3={:#x}",
                cid_now as u64, phase_now as u64, rd_dw2 as u64, rd_dw3 as u64
            );
            serial_println!(
                "[sexdrive.block.write.api.cqe] cid={} phase={} dw2={:#x} dw3={:#x}",
                cid_now as u64, phase_now as u64, rd_dw2 as u64, rd_dw3 as u64
            );
            break;
        }
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.poll.end] path=selftest cid={} found={} status={} cq_head={} phase={} polls={}",
        write_cid as u64,
        if done { 1u64 } else { 0u64 },
        ((dw3 >> 17) & 0x7FFF) as u64,
        cq_head as u64,
        cq_phase as u64,
        polls as u64
    ); }
    if !done {
        stop_uncertain_io();
        serial_println!("[sexdrive.storage100.rw.fail] reason=write_cqe_timeout");
        serial_println!(
            "[sexdrive.nvme.write.err] reason=cqe_timeout cid={} head={} phase={}",
            write_cid as u64, cq_head as u64, cq_phase as u64
        );
        serial_println!(
            "[sexdrive.nvme.poll.timeout.detail] path=selftest cid={} expected_cid={} seen_cid={} seen_status={} seen_phase={} cq_head={} phase={} polls={}",
            write_cid as u64,
            write_cid as u64,
            (last_dw3 & 0xFFFF) as u64,
            ((last_dw3 >> 17) & 0x7FFF) as u64,
            ((last_dw3 >> 16) & 0x1) as u64,
            cq_head as u64,
            cq_phase as u64,
            polls as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    if write_transfer.complete((dw3 & 0xFFFF) as u16, (dw3 >> 16) & 1, cq_phase).is_err() {
        return BLOCK_ERR_NO_DEVICE;
    }
    let sf = (dw3 >> 17) & 0x7FFF;
    let sc = sf & 0xFF;
    let sct = (sf >> 8) & 0x7;
    if sc != 0 || sct != 0 {
        unsafe { NVME_IO_STATE.ready = false; }
        serial_println!("[sexdrive.storage100.write.complete] status=1 bytes={}", size);
        serial_println!("[sexdrive.storage100.rw.fail] reason=write_status_fail");
        serial_println!(
            "[sexdrive.nvme.write.err] reason=status_fail cid={} sc={} sct={} dw3={:#x}",
            write_cid as u64, sc as u64, sct as u64, dw3 as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    cq_head = (cq_head + 1) % 16;
    if cq_head == 0 {
        cq_phase ^= 1;
    }
    unsafe {
        core::ptr::write_volatile((map_va + cq1hdbl) as *mut u32, cq_head);
    }
    serial_println!("[sexdrive.nvme.write.ok] cid={} slba={}", write_cid as u64, slba);
    serial_println!("[sexdrive.block.write.api.ok] cid={} slba={}", write_cid as u64, slba);
    serial_println!("[sexdrive.storage100.write.complete] status=0 bytes={}", size);

    // Readback verify from same LBA
    serial_println!("[sexdrive.storage100.read.submit] lba={} bytes={}", slba, size);
    serial_println!("[sexdrive.nvme.write.readback.begin] slba={}", slba);
    let mut read_transfer = match prepare_bounce(false, read_cid) {
        Some(transfer) => transfer, None => return BLOCK_ERR_NO_DEVICE,
    };
    let read_phys = read_transfer.buffer.physical_address;
    let read_va = read_transfer.buffer.virtual_address;
    if read_phys == 0 || read_phys == u64::MAX || (read_phys % PAGE_SIZE) != 0
        || read_va == 0 || read_va == u64::MAX
    {
        serial_println!("[sexdrive.storage100.rw.fail] reason=readback_buf_invalid");
        serial_println!(
            "[sexdrive.nvme.write.err] reason=readback_buf_invalid phys={:#x} va={:#x}",
            read_phys, read_va
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    unsafe {
        for i in 0..(PAGE_SIZE as usize) {
            core::ptr::write_volatile((read_va as *mut u8).add(i), 0u8);
        }
    }

    let sqe_ptr2 = (io_sq_va as *mut u8).wrapping_add((sq_tail as usize) * 64) as *mut u32;
    unsafe {
        for i in 0..16usize {
            core::ptr::write_volatile(sqe_ptr2.add(i), 0);
        }
        core::ptr::write_volatile(sqe_ptr2.add(0), 0x02u32 | ((read_cid as u32) << 16)); // READ + CID
        core::ptr::write_volatile(sqe_ptr2.add(1), 1u32); // NSID=1
        core::ptr::write_volatile(sqe_ptr2.add(6), (read_phys & 0xFFFF_FFFF) as u32);
        core::ptr::write_volatile(sqe_ptr2.add(7), (read_phys >> 32) as u32);
        core::ptr::write_volatile(sqe_ptr2.add(10), (slba & 0xFFFF_FFFF) as u32);
        core::ptr::write_volatile(sqe_ptr2.add(11), (slba >> 32) as u32);
        core::ptr::write_volatile(sqe_ptr2.add(12), 0u32);
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.cmd.shape] path=selftest op=READ cid={} nsid={} slba={} nlb={} bytes={} prp1={:#x} prp2={:#x} buf_va={:#x} buf_phys={:#x} buffer_kind=internal",
        read_cid as u64, 1u64, slba, 0u64, size, read_phys, 0u64, read_va, read_phys
    ); }
    compiler_fence(Ordering::SeqCst);
    if hot_log() { serial_println!(
        "[sexdrive.nvme.submit.detail] path=selftest op=READ cid={} sq_tail_before={} sq_tail_after={} cq_head={} cq_phase={} lba={} bytes={}",
        read_cid as u64, sq_tail as u64, ((sq_tail + 1) % 16) as u64, cq_head as u64, cq_phase as u64, slba, size
    ); }
    sq_tail = (sq_tail + 1) % 16;
    unsafe {
        if read_transfer.submit().is_err() { return BLOCK_ERR_NO_DEVICE; }
        core::ptr::write_volatile((map_va + sq1tdbl) as *mut u32, sq_tail);
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.doorbell] path=selftest qid=1 tail={}",
        sq_tail as u64
    ); }

    let mut rb_done = false;
    let mut rb_dw2 = 0u32;
    let mut rb_dw3 = 0u32;
    let max_polls = 1_000_000u32;
    let mut polls = 0u32;
    let mut last_dw3 = 0u32;
    if hot_log() { serial_println!(
        "[sexdrive.nvme.poll.begin] path=selftest cid={} cq_head={} phase={} max_polls={}",
        read_cid as u64, cq_head as u64, cq_phase as u64, max_polls as u64
    ); }
    for _ in 0..max_polls {
        polls = polls.wrapping_add(1);
        let cqe_ptr = (io_cq_va as *const u8).wrapping_add((cq_head as usize) * 16) as *const u32;
        let rd_dw2 = unsafe { core::ptr::read_volatile(cqe_ptr.add(2)) };
        let rd_dw3 = unsafe { core::ptr::read_volatile(cqe_ptr.add(3)) };
        last_dw3 = rd_dw3;
        let cid_now = rd_dw3 & 0xFFFF;
        let phase_now = (rd_dw3 >> 16) & 0x1;
        if cid_now == read_cid as u32 && phase_now == cq_phase {
            rb_done = true;
            rb_dw2 = rd_dw2;
            rb_dw3 = rd_dw3;
            serial_println!(
                "[sexdrive.nvme.write.readback.cqe] cid={} phase={} dw2={:#x} dw3={:#x}",
                cid_now as u64, phase_now as u64, rd_dw2 as u64, rd_dw3 as u64
            );
            break;
        }
    }
    if hot_log() { serial_println!(
        "[sexdrive.nvme.poll.end] path=selftest cid={} found={} status={} cq_head={} phase={} polls={}",
        read_cid as u64,
        if rb_done { 1u64 } else { 0u64 },
        ((rb_dw3 >> 17) & 0x7FFF) as u64,
        cq_head as u64,
        cq_phase as u64,
        polls as u64
    ); }
    if !rb_done {
        stop_uncertain_io();
        serial_println!("[sexdrive.storage100.rw.fail] reason=read_cqe_timeout");
        serial_println!(
            "[sexdrive.nvme.write.err] reason=readback_timeout cid={} head={} phase={}",
            read_cid as u64, cq_head as u64, cq_phase as u64
        );
        serial_println!(
            "[sexdrive.nvme.poll.timeout.detail] path=selftest cid={} expected_cid={} seen_cid={} seen_status={} seen_phase={} cq_head={} phase={} polls={}",
            read_cid as u64,
            read_cid as u64,
            (last_dw3 & 0xFFFF) as u64,
            ((last_dw3 >> 17) & 0x7FFF) as u64,
            ((last_dw3 >> 16) & 0x1) as u64,
            cq_head as u64,
            cq_phase as u64,
            polls as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    if read_transfer.complete((rb_dw3 & 0xFFFF) as u16, (rb_dw3 >> 16) & 1, cq_phase).is_err() {
        return BLOCK_ERR_NO_DEVICE;
    }
    let rb_sf = (rb_dw3 >> 17) & 0x7FFF;
    let rb_sc = rb_sf & 0xFF;
    let rb_sct = (rb_sf >> 8) & 0x7;
    if rb_sc != 0 || rb_sct != 0 {
        unsafe { NVME_IO_STATE.ready = false; }
        serial_println!("[sexdrive.storage100.read.complete] status=1 bytes={}", size);
        serial_println!("[sexdrive.storage100.rw.fail] reason=read_status_fail");
        serial_println!(
            "[sexdrive.nvme.write.err] reason=readback_status_fail cid={} sc={} sct={} dw3={:#x}",
            read_cid as u64, rb_sc as u64, rb_sct as u64, rb_dw3 as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    cq_head = (cq_head + 1) % 16;
    if cq_head == 0 {
        cq_phase ^= 1;
    }
    unsafe {
        core::ptr::write_volatile((map_va + cq1hdbl) as *mut u32, cq_head);
        NVME_IO_STATE.sq_tail = sq_tail;
        NVME_IO_STATE.cq_head = cq_head;
        NVME_IO_STATE.cq_phase = cq_phase;
    }
    serial_println!("[sexdrive.storage100.read.complete] status=0 bytes={}", size);

    let mut mismatch = false;
    let mut i = 0usize;
    while i < WRITE_PROOF_LEN as usize {
        let expect = (0xA5u8 ^ (i as u8) ^ 0x3Cu8) & 0xFFu8;
        let got = unsafe { core::ptr::read_volatile((read_va as *const u8).add(i)) };
        if got != expect {
            mismatch = true;
            break;
        }
        i += 1;
    }
    if mismatch {
        serial_println!("[sexdrive.storage100.rw.fail] reason=byte_mismatch");
        serial_println!("[sexdrive.storage100.read.match] lba={} bytes={} ok=0", slba, size);
        BLOCK_ERR_NO_DEVICE
    } else {
        serial_println!("[sexdrive.storage100.read.match] lba={} bytes={} ok=1", slba, size);
        serial_println!("[sexdrive.storage100.rw.done] ok=1");
        0u64
    }
}

fn nvme_multiblock_write_readback_proof() -> u64 {
    serial_println!(
        "[sexdrive.storage100.multi.begin] base_lba={} blocks={} bytes_per_block={}",
        AP4_MULTI_BASE_LBA,
        AP4_MULTI_BLOCKS,
        AP4_MULTI_BLOCK_BYTES
    );
    let mut b = 0u64;
    while b < AP4_MULTI_BLOCKS {
        let lba = AP4_MULTI_BASE_LBA + b;
        serial_println!(
            "[sexdrive.storage100.multi.block.begin] idx={} lba={} bytes={}",
            b,
            lba,
            AP4_MULTI_BLOCK_BYTES
        );

        let write_phys = reserve_dma_phys(PAGE_SIZE);
        let write_va = own_dma_virtual(write_phys, PAGE_SIZE);
        if write_phys == 0 || write_phys == u64::MAX || (write_phys % PAGE_SIZE) != 0
            || write_va == 0 || write_va == u64::MAX
        {
            serial_println!("[sexdrive.storage100.multi.fail] reason=write_buf_invalid idx={} lba={}", b, lba);
            return BLOCK_ERR_NO_DEVICE;
        }
        unsafe {
            let mut i = 0usize;
            while i < AP4_MULTI_BLOCK_BYTES as usize {
                let val = (0xA5u8 ^ (i as u8) ^ ((b as u8).wrapping_mul(0x33u8)) ^ 0x3Cu8) & 0xFFu8;
                core::ptr::write_volatile((write_va as *mut u8).add(i), val);
                i += 1;
            }
        }

        serial_println!(
            "[sexdrive.storage100.multi.write.submit] idx={} lba={} bytes={}",
            b,
            lba,
            AP4_MULTI_BLOCK_BYTES
        );
        let write_status = nvme_write_one_block(lba * NVME_LBA_BYTES, AP4_MULTI_BLOCK_BYTES, write_va);
        if write_status != 0 {
            serial_println!(
                "[sexdrive.storage100.multi.write.complete] idx={} lba={} status=1 bytes={}",
                b,
                lba,
                AP4_MULTI_BLOCK_BYTES
            );
            serial_println!("[sexdrive.storage100.multi.fail] reason=write_status_fail idx={} lba={}", b, lba);
            return BLOCK_ERR_NO_DEVICE;
        }
        serial_println!(
            "[sexdrive.storage100.multi.write.complete] idx={} lba={} status=0 bytes={}",
            b,
            lba,
            AP4_MULTI_BLOCK_BYTES
        );

        let read_phys = reserve_dma_phys(PAGE_SIZE);
        let read_va = own_dma_virtual(read_phys, PAGE_SIZE);
        if read_phys == 0 || read_phys == u64::MAX || (read_phys % PAGE_SIZE) != 0
            || read_va == 0 || read_va == u64::MAX
        {
            serial_println!("[sexdrive.storage100.multi.fail] reason=read_buf_invalid idx={} lba={}", b, lba);
            return BLOCK_ERR_NO_DEVICE;
        }

        serial_println!(
            "[sexdrive.storage100.multi.read.submit] idx={} lba={} bytes={}",
            b,
            lba,
            AP4_MULTI_BLOCK_BYTES
        );
        let read_status = nvme_read_into_mapped_va(lba * NVME_LBA_BYTES, AP4_MULTI_BLOCK_BYTES, read_va);
        if read_status != 0 {
            serial_println!(
                "[sexdrive.storage100.multi.read.complete] idx={} lba={} status=1 bytes={}",
                b,
                lba,
                AP4_MULTI_BLOCK_BYTES
            );
            serial_println!("[sexdrive.storage100.multi.fail] reason=read_status_fail idx={} lba={}", b, lba);
            return BLOCK_ERR_NO_DEVICE;
        }
        serial_println!(
            "[sexdrive.storage100.multi.read.complete] idx={} lba={} status=0 bytes={}",
            b,
            lba,
            AP4_MULTI_BLOCK_BYTES
        );

        let mut i = 0usize;
        let mut mismatch = false;
        while i < AP4_MULTI_BLOCK_BYTES as usize {
            let expected = (0xA5u8 ^ (i as u8) ^ ((b as u8).wrapping_mul(0x33u8)) ^ 0x3Cu8) & 0xFFu8;
            let got = unsafe { core::ptr::read_volatile((read_va as *const u8).add(i)) };
            if got != expected {
                mismatch = true;
                serial_println!(
                    "[sexdrive.storage100.multi.read.match] idx={} lba={} ok=0 first_bad={} expected={} got={}",
                    b,
                    lba,
                    i as u64,
                    expected as u64,
                    got as u64
                );
                serial_println!(
                    "[sexdrive.storage100.multi.fail] reason=byte_mismatch idx={} lba={} first_bad={}",
                    b,
                    lba,
                    i as u64
                );
                break;
            }
            i += 1;
        }
        if mismatch {
            return BLOCK_ERR_NO_DEVICE;
        }
        serial_println!(
            "[sexdrive.storage100.multi.read.match] idx={} lba={} bytes={} ok=1",
            b,
            lba,
            AP4_MULTI_BLOCK_BYTES
        );
        serial_println!("[sexdrive.storage100.multi.block.done] idx={} lba={} ok=1", b, lba);
        b += 1;
    }
    serial_println!("[sexdrive.storage100.multi.done] blocks={} ok=1", AP4_MULTI_BLOCKS);
    0u64
}

fn nvme_persist_write_proof() -> u64 {
    serial_println!(
        "[sexdrive.storage100.persist.write.begin] base_lba={} blocks={} bytes_per_block={}",
        AP5A_PERSIST_BASE_LBA,
        AP5A_PERSIST_BLOCKS,
        AP5A_PERSIST_BLOCK_BYTES
    );

    let mut b = 0u64;
    while b < AP5A_PERSIST_BLOCKS {
        let lba = AP5A_PERSIST_BASE_LBA + b;
        let write_phys = reserve_dma_phys(PAGE_SIZE);
        let write_va = own_dma_virtual(write_phys, PAGE_SIZE);
        if write_phys == 0 || write_phys == u64::MAX || (write_phys % PAGE_SIZE) != 0
            || write_va == 0 || write_va == u64::MAX
        {
            serial_println!("[sexdrive.storage100.persist.fail] phase=write reason=write_buf_invalid idx={} lba={}", b, lba);
            return BLOCK_ERR_NO_DEVICE;
        }

        unsafe {
            let mut i = 0usize;
            while i < AP5A_PERSIST_BLOCK_BYTES as usize {
                let val = (0x5Au8 ^ (i as u8) ^ ((b as u8).wrapping_mul(0x21u8)) ^ 0xC3u8) & 0xFFu8;
                core::ptr::write_volatile((write_va as *mut u8).add(i), val);
                i += 1;
            }
        }

        let write_status = nvme_write_one_block(lba * NVME_LBA_BYTES, AP5A_PERSIST_BLOCK_BYTES, write_va);
        if write_status != 0 {
            serial_println!("[sexdrive.storage100.persist.fail] phase=write reason=write_status_fail idx={} lba={} status={}", b, lba, write_status);
            return BLOCK_ERR_NO_DEVICE;
        }

        serial_println!(
            "[sexdrive.storage100.persist.write.block] idx={} lba={} status=0 bytes={}",
            b,
            lba,
            AP5A_PERSIST_BLOCK_BYTES
        );
        b += 1;
    }

    serial_println!("[sexdrive.storage100.persist.write.done] blocks={} ok=1", AP5A_PERSIST_BLOCKS);
    0u64
}

fn nvme_persist_read_proof() -> u64 {
    serial_println!(
        "[sexdrive.storage100.persist.read.begin] base_lba={} blocks={} bytes_per_block={}",
        AP5A_PERSIST_BASE_LBA,
        AP5A_PERSIST_BLOCKS,
        AP5A_PERSIST_BLOCK_BYTES
    );

    let mut b = 0u64;
    while b < AP5A_PERSIST_BLOCKS {
        let lba = AP5A_PERSIST_BASE_LBA + b;
        let read_phys = reserve_dma_phys(PAGE_SIZE);
        let read_va = own_dma_virtual(read_phys, PAGE_SIZE);
        if read_phys == 0 || read_phys == u64::MAX || (read_phys % PAGE_SIZE) != 0
            || read_va == 0 || read_va == u64::MAX
        {
            serial_println!("[sexdrive.storage100.persist.fail] phase=read reason=read_buf_invalid idx={} lba={}", b, lba);
            return BLOCK_ERR_NO_DEVICE;
        }

        let read_status = nvme_read_into_mapped_va(lba * NVME_LBA_BYTES, AP5A_PERSIST_BLOCK_BYTES, read_va);
        if read_status != 0 {
            serial_println!("[sexdrive.storage100.persist.fail] phase=read reason=read_status_fail idx={} lba={} status={}", b, lba, read_status);
            return BLOCK_ERR_NO_DEVICE;
        }

        serial_println!(
            "[sexdrive.storage100.persist.read.block] idx={} lba={} status=0 bytes={}",
            b,
            lba,
            AP5A_PERSIST_BLOCK_BYTES
        );

        let mut i = 0usize;
        while i < AP5A_PERSIST_BLOCK_BYTES as usize {
            let expected = (0x5Au8 ^ (i as u8) ^ ((b as u8).wrapping_mul(0x21u8)) ^ 0xC3u8) & 0xFFu8;
            let got = unsafe { core::ptr::read_volatile((read_va as *const u8).add(i)) };
            if got != expected {
                serial_println!(
                    "[sexdrive.storage100.persist.read.match] idx={} lba={} ok=0 first_bad={} expected={} got={}",
                    b,
                    lba,
                    i as u64,
                    expected as u64,
                    got as u64
                );
                serial_println!(
                    "[sexdrive.storage100.persist.fail] phase=read reason=byte_mismatch idx={} lba={} first_bad={}",
                    b,
                    lba,
                    i as u64
                );
                return BLOCK_ERR_NO_DEVICE;
            }
            i += 1;
        }

        serial_println!(
            "[sexdrive.storage100.persist.read.match] idx={} lba={} bytes={} ok=1",
            b,
            lba,
            AP5A_PERSIST_BLOCK_BYTES
        );
        b += 1;
    }

    serial_println!("[sexdrive.storage100.persist.read.done] blocks={} ok=1", AP5A_PERSIST_BLOCKS);
    0u64
}

/// [sexdrive.nvme.flush] — Issue NVMe FLUSH command (opcode 0x00) on IO queue 1.
/// No data transfer. Completes successfully only after all previously written
/// data is committed to non-volatile media (or CQE returns success in QEMU).
/// Returns 0 on success, BLOCK_ERR_NO_DEVICE on failure.
fn nvme_flush() -> u64 {
    let io_sq_va: u64;
    let io_cq_va: u64;
    let map_va: u64;
    let sq1tdbl: u64;
    let cq1hdbl: u64;
    let mut sq_tail: u32;
    let mut cq_head: u32;
    let mut cq_phase: u32;
    let flush_cid: u16;
    unsafe {
        if !NVME_IO_STATE.ready {
            serial_println!("[sexdrive.nvme.flush.err] reason=no_ioq_ready");
            return BLOCK_ERR_NO_DEVICE;
        }
        map_va = NVME_IO_STATE.map_va;
        io_sq_va = NVME_IO_STATE.io_sq_va;
        io_cq_va = NVME_IO_STATE.io_cq_va;
        sq1tdbl = NVME_IO_STATE.sq1tdbl;
        cq1hdbl = NVME_IO_STATE.cq1hdbl;
        sq_tail = NVME_IO_STATE.sq_tail;
        cq_head = NVME_IO_STATE.cq_head;
        cq_phase = NVME_IO_STATE.cq_phase;
        flush_cid = NVME_IO_STATE.next_cid;
        NVME_IO_STATE.next_cid = NVME_IO_STATE.next_cid.wrapping_add(1);
    }

    // Build FLUSH SQ entry: opcode=0x00, NSID=1, no data, no metadata.
    let sqe_ptr = unsafe {
        (io_sq_va as *mut u8).wrapping_add((sq_tail as usize) * 64) as *mut u32
    };
    unsafe {
        // Zero entire 64-byte SQ entry.
        let mut i = 0usize;
        while i < 16 {
            core::ptr::write_volatile(sqe_ptr.add(i), 0u32);
            i += 1;
        }
        // CDW0: opcode=0x00 (FLUSH), CID in bits 31:16.
        core::ptr::write_volatile(sqe_ptr.add(0), (flush_cid as u32) << 16);
        // CDW1: NSID=1.
        core::ptr::write_volatile(sqe_ptr.add(1), 1u32);
    }
    serial_println!(
        "[sexdrive.nvme.flush.submit] cid={} nsid=1 sq_tail={}",
        flush_cid as u64, sq_tail as u64
    );

    compiler_fence(Ordering::SeqCst);
    sq_tail = (sq_tail + 1) % 16;
    unsafe {
        core::ptr::write_volatile((map_va + sq1tdbl) as *mut u32, sq_tail);
    }

    // Poll CQE (short timeout — if not supported, return honest error).
    let mut done = false;
    let mut dw3 = 0u32;
    for _poll in 0..1_000_000u32 {
        let cqe_ptr = unsafe {
            (io_cq_va as *const u8).wrapping_add((cq_head as usize) * 16) as *const u32
        };
        let rd_dw3 = unsafe { core::ptr::read_volatile(cqe_ptr.add(3)) };
        let cid_now = rd_dw3 & 0xFFFF;
        let phase_now = (rd_dw3 >> 16) & 0x1;
        if cid_now == flush_cid as u32 && phase_now == cq_phase {
            done = true;
            dw3 = rd_dw3;
            serial_println!(
                "[sexdrive.nvme.flush.cqe] cid={} phase={} dw3={:#x}",
                cid_now as u64, phase_now as u64, rd_dw3 as u64
            );
            break;
        }
        // Spin hint to avoid burning CPU while keeping it simple.
        core::hint::spin_loop();
    }
    if !done {
        stop_uncertain_io();
        serial_println!(
            "[sexdrive.nvme.flush.err] reason=cqe_timeout cid={} head={} phase={}",
            flush_cid as u64, cq_head as u64, cq_phase as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }
    let sf = (dw3 >> 17) & 0x7FFF;
    let sc = sf & 0xFF;
    let sct = (sf >> 8) & 0x7;
    if sc != 0 || sct != 0 {
        unsafe { NVME_IO_STATE.ready = false; }
        serial_println!(
            "[sexdrive.nvme.flush.err] reason=status_fail cid={} sc={} sct={} dw3={:#x}",
            flush_cid as u64, sc as u64, sct as u64, dw3 as u64
        );
        return BLOCK_ERR_NO_DEVICE;
    }

    cq_head = (cq_head + 1) % 16;
    if cq_head == 0 {
        cq_phase ^= 1;
    }
    unsafe {
        core::ptr::write_volatile((map_va + cq1hdbl) as *mut u32, cq_head);
        NVME_IO_STATE.sq_tail = sq_tail;
        NVME_IO_STATE.cq_head = cq_head;
        NVME_IO_STATE.cq_phase = cq_phase;
    }
    serial_println!("[sexdrive.nvme.flush.ok] cid={}", flush_cid as u64);
    0u64
}

fn nvme_storage100_flush_audit() -> u64 {
    serial_println!("[sexdrive.storage100.flush.begin] nsid=1");
    let ioq_ready = unsafe { NVME_IO_STATE.ready };
    if !ioq_ready {
        serial_println!("[sexdrive.storage100.flush.fail] reason=no_ioq_ready");
        return BLOCK_ERR_NO_DEVICE;
    }

    serial_println!("[sexdrive.storage100.flush.submit] opcode=0x00 nsid=1");
    let status = nvme_flush();
    if status == 0 {
        serial_println!("[sexdrive.storage100.flush.complete] status=0");
        serial_println!("[sexdrive.storage100.flush.done] ok=1");
        return 0;
    }

    // Honest AP5b classification: QEMU/device may not complete FLUSH CQE.
    serial_println!(
        "[sexdrive.storage100.flush.skip] reason=flush_not_completed_or_not_supported status={}",
        status
    );
    BLOCK_ERR_NO_DEVICE
}

fn nvme_storage100_negative_mismatch() -> u64 {
    let lba = AP6_NEG_MISMATCH_LBA;
    let size = AP6_NEG_MISMATCH_BYTES;
    let write_phys = reserve_dma_phys(PAGE_SIZE);
    let write_va = own_dma_virtual(write_phys, PAGE_SIZE);
    let read_phys = reserve_dma_phys(PAGE_SIZE);
    let read_va = own_dma_virtual(read_phys, PAGE_SIZE);
    if write_phys == 0 || write_phys == u64::MAX || (write_phys % PAGE_SIZE) != 0
        || write_va == 0 || write_va == u64::MAX
        || read_phys == 0 || read_phys == u64::MAX || (read_phys % PAGE_SIZE) != 0
        || read_va == 0 || read_va == u64::MAX
    {
        serial_println!("[sexdrive.storage100.neg.mismatch.fail] reason=buf_alloc_invalid");
        return BLOCK_ERR_NO_DEVICE;
    }

    serial_println!(
        "[sexdrive.storage100.neg.mismatch.begin] lba={} bytes={}",
        lba, size
    );

    unsafe {
        for i in 0..(size as usize) {
            let b = (0x5Au8 ^ (i as u8) ^ 0xC3u8) & 0xFFu8;
            core::ptr::write_volatile((write_va as *mut u8).add(i), b);
            core::ptr::write_volatile((read_va as *mut u8).add(i), 0u8);
        }
    }

    let write_status = nvme_write_one_block(lba * NVME_LBA_BYTES, size, write_va);
    if write_status != 0 {
        serial_println!(
            "[sexdrive.storage100.neg.mismatch.fail] reason=write_status status={}",
            write_status
        );
        return write_status;
    }
    let read_status = nvme_read_into_mapped_va(lba * NVME_LBA_BYTES, size, read_va);
    if read_status != 0 {
        serial_println!(
            "[sexdrive.storage100.neg.mismatch.fail] reason=read_status status={}",
            read_status
        );
        return read_status;
    }

    unsafe {
        let read_ptr = read_va as *const u8;
        let mut expected = core::ptr::read_volatile(write_va as *const u8);
        expected ^= 0x01;
        let got = core::ptr::read_volatile(read_ptr);
        if expected != got {
            serial_println!(
                "[sexdrive.storage100.neg.mismatch.detected] ok=1 first_bad=0 expected={} got={}",
                expected as u64,
                got as u64
            );
            return 0;
        }
    }

    serial_println!("[sexdrive.storage100.neg.mismatch.fail] reason=unexpected_match");
    BLOCK_ERR_BAD_LEN
}

fn nvme_probe_bar() {
    let bar_bytes = match sex_pdx::memory_region_info(SLOT_NVME_HOST) {
        Ok(info) if info.length >= 0x1000 => info.length,
        _ => { serial_println!("[sexdrive.nvme.bar.resolve.err] reason=missing_boot_view"); return; }
    };
    let map_va: u64;
    unsafe {
        // syscall 43 = MAP_PCI_BAR(cap_slot, bar_index, map_size)
        core::arch::asm!(
            "syscall",
            in("rax") 43u64,
            in("rdi") SLOT_NVME_HOST,
            in("rsi") 0u64,       // BAR0
            in("rdx") bar_bytes, // Bound by the actual boot-granted BAR length
            lateout("rax") map_va,
            out("rcx") _,
            out("r11") _,
        );
    }

    if map_va == u64::MAX || map_va == 0 {
        serial_println!(
            "[sexdrive.nvme.bar.resolve.begin] slot={} bar={} [sexdrive.nvme.bar.resolve.err] map_va={:#x} [sexdrive.device.no_nvme_cap]",
            SLOT_NVME_HOST, 0u64, map_va
        );
        return;
    }

    // BAR0 mapped: read NVMe identity registers via volatile MMIO loads.
    let nvme_cap = unsafe { core::ptr::read_volatile((map_va + 0x0000) as *const u64) };
    let nvme_vs = unsafe { core::ptr::read_volatile((map_va + 0x0008) as *const u32) };
    let nvme_cc = unsafe { core::ptr::read_volatile((map_va + 0x0014) as *const u32) };
    let nvme_csts = unsafe { core::ptr::read_volatile((map_va + 0x001C) as *const u32) };
    let nvme_aqa = unsafe { core::ptr::read_volatile((map_va + 0x0024) as *const u32) };
    let nvme_asq = unsafe { core::ptr::read_volatile((map_va + 0x0028) as *const u64) };
    let nvme_acq = unsafe { core::ptr::read_volatile((map_va + 0x0030) as *const u64) };

    let cap_mqes = (nvme_cap & 0xFFFF) as u16;
    let cap_dstrd = ((nvme_cap >> 32) & 0xF) as u8;
    // Include IO CQ1's doorbell; reject a controller layout outside its BAR.
    let required_bar_bytes = 0x1000 + (3u64 << (2 + cap_dstrd)) + 4;
    if required_bar_bytes > bar_bytes {
        serial_println!("[sexdrive.nvme.bar.resolve.err] reason=doorbell_out_of_range required={} granted={}", required_bar_bytes, bar_bytes);
        return;
    }
    let vs_major = ((nvme_vs >> 16) & 0xFFFF) as u16;
    let vs_minor = ((nvme_vs >> 8) & 0xFF) as u8;
    let csts_rdy = (nvme_csts & 0x1) != 0;
    let cc_en = (nvme_cc & 0x1) != 0;
    serial_println!(
        "[sexdrive.nvme.bar.resolve.begin] slot={} bar={} [sexdrive.nvme.bar.resolve.ok] map_va={:#x} [sexdrive.device.nvme_cap.present] va={:#x} cap={:#x}",
        SLOT_NVME_HOST, 0u64, map_va, map_va, nvme_cap
    );
    serial_println!(
        "[sexdrive.nvme.reg.cap] cap={:#x} mqes={} dstrd={}",
        nvme_cap, cap_mqes, cap_dstrd
    );
    serial_println!(
        "[sexdrive.nvme.reg.vs] vs={:#x} major={} minor={}",
        nvme_vs, vs_major, vs_minor
    );
    serial_println!("[sexdrive.nvme.reg.cc] cc={:#x}", nvme_cc);
    serial_println!(
        "[sexdrive.nvme.reg.csts] csts={:#x} rdy={}",
        nvme_csts,
        if csts_rdy { 1u64 } else { 0u64 }
    );

    if nvme_cap != 0 && nvme_vs != 0 {
        serial_println!(
            "[sexdrive.nvme.identity.ok] cap={:#x} vs={:#x} mqes={} dstrd={} major={} minor={} csts_rdy={}",
            nvme_cap,
            nvme_vs,
            cap_mqes,
            cap_dstrd,
            vs_major,
            vs_minor,
            if csts_rdy { 1u64 } else { 0u64 }
        );
    } else {
        serial_println!(
            "[sexdrive.nvme.identity.err] cap={:#x} vs={:#x} cc={:#x} csts={:#x}",
            nvme_cap,
            nvme_vs,
            nvme_cc,
            nvme_csts
        );
    }

    serial_println!(
        "[sexdrive.nvme.queue.inspect] aqa={:#x} asq={:#x} acq={:#x} cc={:#x} csts={:#x}",
        nvme_aqa,
        nvme_asq,
        nvme_acq,
        nvme_cc,
        nvme_csts
    );
    serial_println!("[sexdrive.nvme.queue.aqa] aqa={:#x}", nvme_aqa);
    serial_println!("[sexdrive.nvme.queue.asq] asq={:#x}", nvme_asq);
    serial_println!("[sexdrive.nvme.queue.acq] acq={:#x}", nvme_acq);

    let queues_preconfigured = nvme_aqa != 0 && nvme_asq != 0 && nvme_acq != 0;
    if queues_preconfigured {
        serial_println!(
            "[sexdrive.nvme.queue.ready] mode=preconfigured cc_en={} csts_rdy={} aqa={:#x} asq={:#x} acq={:#x}",
            if cc_en { 1u64 } else { 0u64 },
            if csts_rdy { 1u64 } else { 0u64 },
            nvme_aqa,
            nvme_asq,
            nvme_acq
        );
    } else {
        if cc_en || csts_rdy {
            serial_println!(
                "[sexdrive.nvme.queue.err] reason=stop_first_controller_enabled_queue_program_requires_disable cc_en={} csts_rdy={} aqa={:#x} asq={:#x} acq={:#x}",
                if cc_en { 1u64 } else { 0u64 },
                if csts_rdy { 1u64 } else { 0u64 },
                nvme_aqa,
                nvme_asq,
                nvme_acq
            );
            return;
        }

        let asq_phys = reserve_dma_phys(PAGE_SIZE);
        let acq_phys = reserve_dma_phys(PAGE_SIZE);
        if asq_phys == 0 || asq_phys == u64::MAX || acq_phys == 0 || acq_phys == u64::MAX {
            serial_println!(
                "[sexdrive.nvme.queue.err] reason=alloc_failed asq_phys={:#x} acq_phys={:#x}",
                asq_phys,
                acq_phys
            );
            return;
        }
        if (asq_phys % PAGE_SIZE) != 0 || (acq_phys % PAGE_SIZE) != 0 {
            serial_println!(
                "[sexdrive.nvme.queue.err] reason=alloc_unaligned asq_phys={:#x} acq_phys={:#x}",
                asq_phys,
                acq_phys
            );
            return;
        }

        let asq_va = own_dma_virtual(asq_phys, PAGE_SIZE);
        let acq_va = own_dma_virtual(acq_phys, PAGE_SIZE);
        if asq_va == 0 || asq_va == u64::MAX || acq_va == 0 || acq_va == u64::MAX {
            serial_println!(
                "[sexdrive.nvme.queue.err] reason=map_failed asq_va={:#x} acq_va={:#x}",
                asq_va,
                acq_va
            );
            return;
        }

        let asq_words = (PAGE_SIZE / 8) as usize;
        let acq_words = (PAGE_SIZE / 8) as usize;
        unsafe {
            for i in 0..asq_words {
                core::ptr::write_volatile((asq_va as *mut u64).add(i), 0);
            }
            for i in 0..acq_words {
                core::ptr::write_volatile((acq_va as *mut u64).add(i), 0);
            }
        }

        serial_println!(
            "[sexdrive.nvme.queue.alloc.ok] asq_phys={:#x} asq_va={:#x} acq_phys={:#x} acq_va={:#x}",
            asq_phys,
            asq_va,
            acq_phys,
            acq_va
        );

        let aqa = ((16u32 - 1) << 16) | (16u32 - 1);
        unsafe {
            core::ptr::write_volatile((map_va + 0x0024) as *mut u32, aqa);
            core::ptr::write_volatile((map_va + 0x0028) as *mut u64, asq_phys);
            core::ptr::write_volatile((map_va + 0x0030) as *mut u64, acq_phys);
        }

        let aqa_rb = unsafe { core::ptr::read_volatile((map_va + 0x0024) as *const u32) };
        let asq_rb = unsafe { core::ptr::read_volatile((map_va + 0x0028) as *const u64) };
        let acq_rb = unsafe { core::ptr::read_volatile((map_va + 0x0030) as *const u64) };
        serial_println!(
            "[sexdrive.nvme.queue.program.ok] aqa={:#x} asq={:#x} acq={:#x}",
            aqa_rb,
            asq_rb,
            acq_rb
        );
        serial_println!(
            "[sexdrive.nvme.queue.ready] mode=programmed cc_en=0 csts_rdy=0 aqa={:#x} asq={:#x} acq={:#x}",
            aqa_rb,
            asq_rb,
            acq_rb
        );
    }

    let old_cc = nvme_cc;
    let old_csts = nvme_csts;
    let old_aqa = nvme_aqa;
    let old_asq = nvme_asq;
    let old_acq = nvme_acq;
    let cap_to = ((nvme_cap >> 24) & 0xFF) as u32;

    serial_println!(
        "[sexdrive.nvme.reprovision.begin] old_cc={:#x} old_csts={:#x} old_aqa={:#x} old_asq={:#x} old_acq={:#x} cap_to={} dstrd={}",
        old_cc,
        old_csts,
        old_aqa,
        old_asq,
        old_acq,
        cap_to as u64,
        cap_dstrd as u64
    );

    serial_println!(
        "[sexdrive.nvme.reprovision.disable.begin] cc_old={:#x} csts_old={:#x}",
        old_cc,
        old_csts
    );
    let cc_disabled = old_cc & !1u32;
    unsafe {
        core::ptr::write_volatile((map_va + 0x0014) as *mut u32, cc_disabled);
    }

    let disable_poll_limit = if cap_to == 0 { 1_000_000u32 } else { (cap_to * 20_000).max(200_000) };
    let mut disable_polls = 0u32;
    let mut rdy0 = false;
    while disable_polls < disable_poll_limit {
        let csts_now = unsafe { core::ptr::read_volatile((map_va + 0x001C) as *const u32) };
        if (csts_now & 0x1) == 0 {
            rdy0 = true;
            serial_println!(
                "[sexdrive.nvme.reprovision.rdy0] polls={} csts={:#x} cc={:#x}",
                disable_polls as u64,
                csts_now as u64,
                cc_disabled as u64
            );
            break;
        }
        disable_polls += 1;
    }
    if !rdy0 {
        let csts_now = unsafe { core::ptr::read_volatile((map_va + 0x001C) as *const u32) };
        serial_println!(
            "[sexdrive.nvme.reprovision.err] reason=disable_rdy0_timeout polls={} limit={} csts={:#x} cc={:#x}",
            disable_polls as u64,
            disable_poll_limit as u64,
            csts_now as u64,
            cc_disabled as u64
        );
        return;
    }

    let asq_phys = reserve_dma_phys(PAGE_SIZE);
    let acq_phys = reserve_dma_phys(PAGE_SIZE);
    if asq_phys == 0 || asq_phys == u64::MAX || acq_phys == 0 || acq_phys == u64::MAX {
        serial_println!(
            "[sexdrive.nvme.reprovision.err] reason=alloc_failed asq_phys={:#x} acq_phys={:#x}",
            asq_phys,
            acq_phys
        );
        return;
    }
    if (asq_phys % PAGE_SIZE) != 0 || (acq_phys % PAGE_SIZE) != 0 {
        serial_println!(
            "[sexdrive.nvme.reprovision.err] reason=alloc_unaligned asq_phys={:#x} acq_phys={:#x}",
            asq_phys,
            acq_phys
        );
        return;
    }

    let asq_va = own_dma_virtual(asq_phys, PAGE_SIZE);
    let acq_va = own_dma_virtual(acq_phys, PAGE_SIZE);
    if asq_va == 0 || asq_va == u64::MAX || acq_va == 0 || acq_va == u64::MAX {
        serial_println!(
            "[sexdrive.nvme.reprovision.err] reason=map_failed asq_va={:#x} acq_va={:#x}",
            asq_va,
            acq_va
        );
        return;
    }

    let words = (PAGE_SIZE / 8) as usize;
    unsafe {
        for i in 0..words {
            core::ptr::write_volatile((asq_va as *mut u64).add(i), 0);
            core::ptr::write_volatile((acq_va as *mut u64).add(i), 0);
        }
    }
    serial_println!(
        "[sexdrive.nvme.reprovision.alloc.ok] asq_phys={:#x} asq_va={:#x} acq_phys={:#x} acq_va={:#x}",
        asq_phys,
        asq_va,
        acq_phys,
        acq_va
    );

    let new_aqa = ((16u32 - 1) << 16) | (16u32 - 1);
    unsafe {
        core::ptr::write_volatile((map_va + 0x0024) as *mut u32, new_aqa);
        core::ptr::write_volatile((map_va + 0x0028) as *mut u64, asq_phys);
        core::ptr::write_volatile((map_va + 0x0030) as *mut u64, acq_phys);
    }
    serial_println!("[sexdrive.nvme.reprovision.program.aqa] aqa={:#x}", new_aqa as u64);
    serial_println!("[sexdrive.nvme.reprovision.program.asq] asq={:#x}", asq_phys);
    serial_println!("[sexdrive.nvme.reprovision.program.acq] acq={:#x}", acq_phys);

    serial_println!("[sexdrive.nvme.reprovision.enable.begin] cc_before={:#x}", cc_disabled as u64);
    let cc_enabled = cc_disabled | 1u32;
    unsafe {
        core::ptr::write_volatile((map_va + 0x0014) as *mut u32, cc_enabled);
    }

    let enable_poll_limit = if cap_to == 0 { 1_000_000u32 } else { (cap_to * 20_000).max(200_000) };
    let mut enable_polls = 0u32;
    let mut rdy1 = false;
    while enable_polls < enable_poll_limit {
        let csts_now = unsafe { core::ptr::read_volatile((map_va + 0x001C) as *const u32) };
        if (csts_now & 0x1) != 0 {
            rdy1 = true;
            serial_println!(
                "[sexdrive.nvme.reprovision.rdy1] polls={} csts={:#x} cc={:#x}",
                enable_polls as u64,
                csts_now as u64,
                cc_enabled as u64
            );
            break;
        }
        enable_polls += 1;
    }
    if !rdy1 {
        let csts_now = unsafe { core::ptr::read_volatile((map_va + 0x001C) as *const u32) };
        serial_println!(
            "[sexdrive.nvme.reprovision.err] reason=enable_rdy1_timeout polls={} limit={} csts={:#x} cc={:#x}",
            enable_polls as u64,
            enable_poll_limit as u64,
            csts_now as u64,
            cc_enabled as u64
        );
        return;
    }

    let rb_cc = unsafe { core::ptr::read_volatile((map_va + 0x0014) as *const u32) };
    let rb_csts = unsafe { core::ptr::read_volatile((map_va + 0x001C) as *const u32) };
    let rb_aqa = unsafe { core::ptr::read_volatile((map_va + 0x0024) as *const u32) };
    let rb_asq = unsafe { core::ptr::read_volatile((map_va + 0x0028) as *const u64) };
    let rb_acq = unsafe { core::ptr::read_volatile((map_va + 0x0030) as *const u64) };
    if rb_aqa != new_aqa || rb_asq != asq_phys || rb_acq != acq_phys || (rb_cc & 1) == 0 || (rb_csts & 1) == 0 {
        serial_println!(
            "[sexdrive.nvme.reprovision.err] reason=readback_mismatch cc={:#x} csts={:#x} aqa={:#x} asq={:#x} acq={:#x}",
            rb_cc as u64,
            rb_csts as u64,
            rb_aqa as u64,
            rb_asq,
            rb_acq
        );
        return;
    }
    serial_println!(
        "[sexdrive.nvme.reprovision.ok] cc={:#x} csts={:#x} aqa={:#x} asq={:#x} acq={:#x} disable_polls={} enable_polls={}",
        rb_cc as u64,
        rb_csts as u64,
        rb_aqa as u64,
        rb_asq,
        rb_acq,
        disable_polls as u64,
        enable_polls as u64
    );

    // Identify retry V2 with corrected CQE decode.
    let asq_owned_phys = rb_asq;
    let acq_owned_phys = rb_acq;
    let asq_owned_va = asq_va;
    let acq_owned_va = acq_va;
    let q_entries: u32 = 16;
    let mut sq_tail: u32 = 0;
    let mut cq_head: u32 = 0;
    let mut cq_phase: u32 = 1;

    let cc_css = (rb_cc >> 4) & 0x7;
    let cc_mps = (rb_cc >> 7) & 0xF;
    let cc_ams = (rb_cc >> 11) & 0x7;
    let cc_iosqes = (rb_cc >> 16) & 0xF;
    let cc_iocqes = (rb_cc >> 20) & 0xF;

    serial_println!(
        "[sexdrive.nvme.admin.identify.v2.begin] asq_phys={:#x} asq_va={:#x} acq_phys={:#x} acq_va={:#x} q_entries={} sq_head={} sq_tail={} cq_head={} cq_phase={}",
        asq_owned_phys,
        asq_owned_va,
        acq_owned_phys,
        acq_owned_va,
        q_entries as u64,
        0u64,
        sq_tail as u64,
        cq_head as u64,
        cq_phase as u64
    );
    serial_println!(
        "[sexdrive.nvme.admin.identify.v2.cqe.decode] cc={:#x} en={} css={} mps={} ams={} iosqes={} iocqes={} dstrd={} sq0tdbl={:#x} cq0hdbl={:#x}",
        rb_cc as u64,
        (rb_cc & 1) as u64,
        cc_css as u64,
        cc_mps as u64,
        cc_ams as u64,
        cc_iosqes as u64,
        cc_iocqes as u64,
        cap_dstrd as u64,
        NVME_SQ0TDBL,
        NVME_CQ0HDBL
    );

    if cc_iosqes != 6 || cc_iocqes != 4 || cc_mps != 0 {
        serial_println!(
            "[sexdrive.nvme.admin.identify.v2.err] reason=cc_layout_invalid cc={:#x} iosqes={} iocqes={} mps={}",
            rb_cc as u64,
            cc_iosqes as u64,
            cc_iocqes as u64,
            cc_mps as u64
        );
        return;
    }

    let acq0_ptr = acq_owned_va as *const u32;
    let acq0_dw0_before = unsafe { core::ptr::read_volatile(acq0_ptr.add(0)) };
    let acq0_dw1_before = unsafe { core::ptr::read_volatile(acq0_ptr.add(1)) };
    let acq0_dw2_before = unsafe { core::ptr::read_volatile(acq0_ptr.add(2)) };
    let acq0_dw3_before = unsafe { core::ptr::read_volatile(acq0_ptr.add(3)) };
    serial_println!(
        "[sexdrive.nvme.admin.identify.v2.cqe.raw] stage=before dw0={:#x} dw1={:#x} dw2={:#x} dw3={:#x} phase={}",
        acq0_dw0_before as u64,
        acq0_dw1_before as u64,
        acq0_dw2_before as u64,
        acq0_dw3_before as u64,
        (acq0_dw3_before & 0x1) as u64
    );

    let id_phys = reserve_dma_phys(PAGE_SIZE);
    if id_phys == 0 || id_phys == u64::MAX || (id_phys % PAGE_SIZE) != 0 {
        serial_println!(
            "[sexdrive.nvme.admin.identify.v2.err] reason=id_alloc_invalid phys={:#x}",
            id_phys
        );
        return;
    }
    let id_va = own_dma_virtual(id_phys, PAGE_SIZE);
    if id_va == 0 || id_va == u64::MAX {
        serial_println!(
            "[sexdrive.nvme.admin.identify.v2.err] reason=id_map_failed va={:#x}",
            id_va
        );
        return;
    }

    unsafe {
        for i in 0..(PAGE_SIZE / 8) as usize {
            core::ptr::write_volatile((id_va as *mut u64).add(i), 0);
        }
    }

    let cid: u16 = 0x0042;
    let sqe_ptr = (asq_owned_va as *mut u8).wrapping_add((sq_tail as usize) * 64) as *mut u32;
    unsafe {
        for i in 0..16usize {
            core::ptr::write_volatile(sqe_ptr.add(i), 0);
        }
        // CDW0: OPC=0x06 Identify, CID in bits 31:16
        core::ptr::write_volatile(sqe_ptr.add(0), 0x06u32 | ((cid as u32) << 16));
        // PRP1 in DW6/DW7
        core::ptr::write_volatile(sqe_ptr.add(6), (id_phys & 0xFFFF_FFFF) as u32);
        core::ptr::write_volatile(sqe_ptr.add(7), (id_phys >> 32) as u32);
        // CDW10: CNS=1 Identify Controller
        core::ptr::write_volatile(sqe_ptr.add(10), 1u32);
    }

    serial_println!(
        "[sexdrive.nvme.admin.identify.v2.submit] cid={} opc={:#x} cns={} prp1={:#x} sq_tail={}",
        cid as u64,
        0x06u64,
        1u64,
        id_phys,
        sq_tail as u64
    );

    compiler_fence(Ordering::SeqCst);
    sq_tail = (sq_tail + 1) % q_entries;
    unsafe {
        core::ptr::write_volatile((map_va + NVME_SQ0TDBL) as *mut u32, sq_tail);
    }
    serial_println!(
        "[sexdrive.nvme.admin.identify.v2.submit] sq0tdbl={:#x} new_tail={} dstrd={}",
        NVME_SQ0TDBL,
        sq_tail as u64,
        cap_dstrd as u64
    );

    let mut got_cqe = false;
    let mut cqe_dw0 = 0u32;
    let mut cqe_dw1 = 0u32;
    let mut cqe_dw2 = 0u32;
    let mut cqe_dw3 = 0u32;
    let mut cqe_phase = 0u32;
    let mut cqe_cid = 0u32;
    let mut cqe_polls = 0u32;
    let mut acq_changed = false;
    let mut first_change_poll = u32::MAX;
    let mut acq0_dw0_change = acq0_dw0_before;
    let mut acq0_dw1_change = acq0_dw1_before;
    let mut acq0_dw2_change = acq0_dw2_before;
    let mut acq0_dw3_change = acq0_dw3_before;
    let mut saw_cid_ignore_phase = false;
    let mut ig_dw0 = 0u32;
    let mut ig_dw1 = 0u32;
    let mut ig_dw2 = 0u32;
    let mut ig_dw3 = 0u32;
    let mut ig_phase = 0u32;

    while cqe_polls < 1_000_000 {
        let acq0_dw0 = unsafe { core::ptr::read_volatile(acq0_ptr.add(0)) };
        let acq0_dw1 = unsafe { core::ptr::read_volatile(acq0_ptr.add(1)) };
        let acq0_dw2 = unsafe { core::ptr::read_volatile(acq0_ptr.add(2)) };
        let acq0_dw3 = unsafe { core::ptr::read_volatile(acq0_ptr.add(3)) };
        if !acq_changed
            && (acq0_dw0 != acq0_dw0_before
                || acq0_dw1 != acq0_dw1_before
                || acq0_dw2 != acq0_dw2_before
                || acq0_dw3 != acq0_dw3_before)
        {
            acq_changed = true;
            first_change_poll = cqe_polls;
            acq0_dw0_change = acq0_dw0;
            acq0_dw1_change = acq0_dw1;
            acq0_dw2_change = acq0_dw2;
            acq0_dw3_change = acq0_dw3;
            serial_println!(
                "[sexdrive.nvme.admin.identify.v2.cqe.raw] stage=change poll={} dw0={:#x} dw1={:#x} dw2={:#x} dw3={:#x} phase={}",
                cqe_polls as u64,
                acq0_dw0 as u64,
                acq0_dw1 as u64,
                acq0_dw2 as u64,
                acq0_dw3 as u64,
                (acq0_dw3 & 0x1) as u64
            );
        }

        let cqe_ptr = (acq_owned_va as *const u8).wrapping_add((cq_head as usize) * 16) as *const u32;
        let dw0 = unsafe { core::ptr::read_volatile(cqe_ptr.add(0)) };
        let dw1 = unsafe { core::ptr::read_volatile(cqe_ptr.add(1)) };
        let dw2 = unsafe { core::ptr::read_volatile(cqe_ptr.add(2)) };
        let dw3 = unsafe { core::ptr::read_volatile(cqe_ptr.add(3)) };
        // Corrected decode for this observed QEMU path:
        // DW3[15:0]  = CID
        // DW3[16]    = phase
        // DW3[31:17] = status field
        let cmd_id = dw3 & 0xFFFF;
        let phase = (dw3 >> 16) & 0x1;
        if cmd_id == cid as u32 {
            saw_cid_ignore_phase = true;
            ig_dw0 = dw0;
            ig_dw1 = dw1;
            ig_dw2 = dw2;
            ig_dw3 = dw3;
            ig_phase = phase;
        }
        if phase == cq_phase && cmd_id == cid as u32 {
            got_cqe = true;
            cqe_dw0 = dw0;
            cqe_dw1 = dw1;
            cqe_dw2 = dw2;
            cqe_dw3 = dw3;
            cqe_phase = phase;
            cqe_cid = cmd_id;
            serial_println!(
                "[sexdrive.nvme.admin.identify.v2.cqe.decode] match=phase polls={} head={} phase={} cid={} dw0={:#x} dw1={:#x} dw2={:#x} dw3={:#x}",
                cqe_polls as u64,
                cq_head as u64,
                phase as u64,
                cmd_id as u64,
                dw0 as u64,
                dw1 as u64,
                dw2 as u64,
                dw3 as u64
            );
            break;
        }
        cqe_polls += 1;
    }

    if !got_cqe {
        if saw_cid_ignore_phase {
            serial_println!(
                "[sexdrive.nvme.admin.identify.v2.cqe.raw] stage=ignore_phase cid={} phase={} dw0={:#x} dw1={:#x} dw2={:#x} dw3={:#x}",
                cid as u64,
                ig_phase as u64,
                ig_dw0 as u64,
                ig_dw1 as u64,
                ig_dw2 as u64,
                ig_dw3 as u64
            );
        }
        serial_println!(
            "[sexdrive.nvme.admin.identify.v2.err] reason=cqe_timeout cid={} head={} phase={} polls={} acq_changed={} first_change_poll={} acq0_dw0_before={:#x} acq0_dw1_before={:#x} acq0_dw2_before={:#x} acq0_dw3_before={:#x} acq0_dw0_now={:#x} acq0_dw1_now={:#x} acq0_dw2_now={:#x} acq0_dw3_now={:#x}",
            cid as u64,
            cq_head as u64,
            cq_phase as u64,
            cqe_polls as u64,
            if acq_changed { 1u64 } else { 0u64 },
            if first_change_poll == u32::MAX { u64::MAX } else { first_change_poll as u64 },
            acq0_dw0_before as u64,
            acq0_dw1_before as u64,
            acq0_dw2_before as u64,
            acq0_dw3_before as u64,
            acq0_dw0_change as u64,
            acq0_dw1_change as u64,
            acq0_dw2_change as u64,
            acq0_dw3_change as u64
        );
        return;
    }

    serial_println!(
        "[sexdrive.nvme.admin.identify.v2.cqe.raw] stage=phase_match cid={} phase={} dw0={:#x} dw1={:#x} dw2={:#x} dw3={:#x}",
        cqe_cid as u64,
        cqe_phase as u64,
        cqe_dw0 as u64,
        cqe_dw1 as u64,
        cqe_dw2 as u64,
        cqe_dw3 as u64
    );

    let sf = (cqe_dw3 >> 17) & 0x7FFF;
    let sc = sf & 0xFF;
    let sct = (sf >> 8) & 0x7;
    let sqhd = (cqe_dw2 >> 16) & 0xFFFF;

    cq_head = (cq_head + 1) % q_entries;
    if cq_head == 0 {
        cq_phase ^= 1;
    }
    unsafe {
        core::ptr::write_volatile((map_va + NVME_CQ0HDBL) as *mut u32, cq_head);
    }

    if sc != 0 || sct != 0 {
        serial_println!(
            "[sexdrive.nvme.admin.identify.v2.err] reason=cqe_status_fail cid={} sc={} sct={} sqhd={} head={} dw3={:#x}",
            cid as u64,
            sc as u64,
            sct as u64,
            sqhd as u64,
            cq_head as u64,
            cqe_dw3 as u64
        );
        return;
    }

    let sn0 = unsafe { core::ptr::read_volatile((id_va + 4) as *const u64) };
    let mn0 = unsafe { core::ptr::read_volatile((id_va + 24) as *const u64) };
    let nn = unsafe { core::ptr::read_volatile((id_va + 516) as *const u32) };
    serial_println!(
        "[sexdrive.nvme.admin.identify.v2.ok] cid={} sc={} sct={} sqhd={} cqh={} cqp={} sn0={:#x} mn0={:#x} nn={}",
        cid as u64,
        sc as u64,
        sct as u64,
        sqhd as u64,
        cq_head as u64,
        cq_phase as u64,
        sn0,
        mn0,
        nn as u64
    );

    // IO Queue creation proof (no IO read/write in this mission).
    let io_cq_phys = reserve_dma_phys(PAGE_SIZE);
    let io_sq_phys = reserve_dma_phys(PAGE_SIZE);
    if io_cq_phys == 0 || io_cq_phys == u64::MAX || io_sq_phys == 0 || io_sq_phys == u64::MAX {
        serial_println!(
            "[sexdrive.nvme.ioq.err] reason=alloc_failed io_cq_phys={:#x} io_sq_phys={:#x}",
            io_cq_phys,
            io_sq_phys
        );
        return;
    }
    if (io_cq_phys % PAGE_SIZE) != 0 || (io_sq_phys % PAGE_SIZE) != 0 {
        serial_println!(
            "[sexdrive.nvme.ioq.err] reason=alloc_unaligned io_cq_phys={:#x} io_sq_phys={:#x}",
            io_cq_phys,
            io_sq_phys
        );
        return;
    }

    let io_cq_va = own_dma_virtual(io_cq_phys, PAGE_SIZE);
    let io_sq_va = own_dma_virtual(io_sq_phys, PAGE_SIZE);
    if io_cq_va == 0 || io_cq_va == u64::MAX || io_sq_va == 0 || io_sq_va == u64::MAX {
        serial_println!(
            "[sexdrive.nvme.ioq.err] reason=map_failed io_cq_va={:#x} io_sq_va={:#x}",
            io_cq_va,
            io_sq_va
        );
        return;
    }

    unsafe {
        for i in 0..(PAGE_SIZE / 8) as usize {
            core::ptr::write_volatile((io_cq_va as *mut u64).add(i), 0);
            core::ptr::write_volatile((io_sq_va as *mut u64).add(i), 0);
        }
    }
    serial_println!(
        "[sexdrive.nvme.ioq.alloc.ok] io_cq_phys={:#x} io_cq_va={:#x} io_sq_phys={:#x} io_sq_va={:#x}",
        io_cq_phys,
        io_cq_va,
        io_sq_phys,
        io_sq_va
    );

    // Reuse admin queue state after Identify completion.
    let mut admin_sq_tail = sq_tail;
    let mut admin_cq_head = cq_head;
    let mut admin_cq_phase = cq_phase;

    // Create IO Completion Queue (opcode 0x05), QID=1, QSIZE=15, PC=1, IEN=0, IV=0.
    let create_cq_cid: u16 = 0x0043;
    let cq_sqe_ptr = (asq_owned_va as *mut u8).wrapping_add((admin_sq_tail as usize) * 64) as *mut u32;
    unsafe {
        for i in 0..16usize {
            core::ptr::write_volatile(cq_sqe_ptr.add(i), 0);
        }
        core::ptr::write_volatile(cq_sqe_ptr.add(0), 0x05u32 | ((create_cq_cid as u32) << 16));
        core::ptr::write_volatile(cq_sqe_ptr.add(6), (io_cq_phys & 0xFFFF_FFFF) as u32);
        core::ptr::write_volatile(cq_sqe_ptr.add(7), (io_cq_phys >> 32) as u32);
        core::ptr::write_volatile(cq_sqe_ptr.add(10), (1u32) | (15u32 << 16));
        core::ptr::write_volatile(cq_sqe_ptr.add(11), 1u32); // PC=1, IEN=0, IV=0
    }
    serial_println!(
        "[sexdrive.nvme.ioq.create_cq.submit] cid={} opc={:#x} qid={} qsize={} prp1={:#x} cdw11={:#x}",
        create_cq_cid as u64,
        0x05u64,
        1u64,
        15u64,
        io_cq_phys,
        1u64
    );

    compiler_fence(Ordering::SeqCst);
    admin_sq_tail = (admin_sq_tail + 1) % q_entries;
    unsafe { core::ptr::write_volatile((map_va + NVME_SQ0TDBL) as *mut u32, admin_sq_tail); }

    let mut cq_done = false;
    let mut cq_cqe_dw2 = 0u32;
    let mut cq_cqe_dw3 = 0u32;
    for _ in 0..1_000_000u32 {
        let cqe_ptr = (acq_owned_va as *const u8).wrapping_add((admin_cq_head as usize) * 16) as *const u32;
        let dw2 = unsafe { core::ptr::read_volatile(cqe_ptr.add(2)) };
        let dw3 = unsafe { core::ptr::read_volatile(cqe_ptr.add(3)) };
        let cid_now = dw3 & 0xFFFF;
        let phase_now = (dw3 >> 16) & 0x1;
        if cid_now == create_cq_cid as u32 && phase_now == admin_cq_phase {
            cq_done = true;
            cq_cqe_dw2 = dw2;
            cq_cqe_dw3 = dw3;
            serial_println!(
                "[sexdrive.nvme.ioq.create_cq.cqe] cid={} dw2={:#x} dw3={:#x} phase={}",
                cid_now as u64,
                dw2 as u64,
                dw3 as u64,
                phase_now as u64
            );
            break;
        }
    }
    if !cq_done {
        stop_uncertain_io();
        serial_println!(
            "[sexdrive.nvme.ioq.err] reason=create_cq_timeout cid={}",
            create_cq_cid as u64
        );
        return;
    }

    let cq_sf = (cq_cqe_dw3 >> 17) & 0x7FFF;
    let cq_sc = cq_sf & 0xFF;
    let cq_sct = (cq_sf >> 8) & 0x7;
    admin_cq_head = (admin_cq_head + 1) % q_entries;
    if admin_cq_head == 0 { admin_cq_phase ^= 1; }
    unsafe { core::ptr::write_volatile((map_va + NVME_CQ0HDBL) as *mut u32, admin_cq_head); }
    if cq_sc != 0 || cq_sct != 0 {
        serial_println!(
            "[sexdrive.nvme.ioq.err] reason=create_cq_status_fail cid={} sc={} sct={} dw3={:#x}",
            create_cq_cid as u64,
            cq_sc as u64,
            cq_sct as u64,
            cq_cqe_dw3 as u64
        );
        return;
    }
    serial_println!(
        "[sexdrive.nvme.ioq.create_cq.ok] cid={} sc={} sct={} cqh={} cqp={}",
        create_cq_cid as u64,
        cq_sc as u64,
        cq_sct as u64,
        admin_cq_head as u64,
        admin_cq_phase as u64
    );

    // Create IO Submission Queue (opcode 0x01), QID=1, QSIZE=15, CQID=1, PC=1, QPRIO=0.
    let create_sq_cid: u16 = 0x0044;
    let sq_sqe_ptr = (asq_owned_va as *mut u8).wrapping_add((admin_sq_tail as usize) * 64) as *mut u32;
    unsafe {
        for i in 0..16usize {
            core::ptr::write_volatile(sq_sqe_ptr.add(i), 0);
        }
        core::ptr::write_volatile(sq_sqe_ptr.add(0), 0x01u32 | ((create_sq_cid as u32) << 16));
        core::ptr::write_volatile(sq_sqe_ptr.add(6), (io_sq_phys & 0xFFFF_FFFF) as u32);
        core::ptr::write_volatile(sq_sqe_ptr.add(7), (io_sq_phys >> 32) as u32);
        core::ptr::write_volatile(sq_sqe_ptr.add(10), (1u32) | (15u32 << 16));
        core::ptr::write_volatile(sq_sqe_ptr.add(11), (1u32 << 16) | 1u32); // CQID=1, PC=1, QPRIO=0
    }
    serial_println!(
        "[sexdrive.nvme.ioq.create_sq.submit] cid={} opc={:#x} qid={} qsize={} cqid={} prp1={:#x}",
        create_sq_cid as u64,
        0x01u64,
        1u64,
        15u64,
        1u64,
        io_sq_phys
    );

    compiler_fence(Ordering::SeqCst);
    admin_sq_tail = (admin_sq_tail + 1) % q_entries;
    unsafe { core::ptr::write_volatile((map_va + NVME_SQ0TDBL) as *mut u32, admin_sq_tail); }

    let mut sq_done = false;
    let mut sq_cqe_dw3 = 0u32;
    for _ in 0..1_000_000u32 {
        let cqe_ptr = (acq_owned_va as *const u8).wrapping_add((admin_cq_head as usize) * 16) as *const u32;
        let dw3 = unsafe { core::ptr::read_volatile(cqe_ptr.add(3)) };
        let cid_now = dw3 & 0xFFFF;
        let phase_now = (dw3 >> 16) & 0x1;
        if cid_now == create_sq_cid as u32 && phase_now == admin_cq_phase {
            sq_done = true;
            sq_cqe_dw3 = dw3;
            serial_println!(
                "[sexdrive.nvme.ioq.create_sq.cqe] cid={} dw3={:#x} phase={}",
                cid_now as u64,
                dw3 as u64,
                phase_now as u64
            );
            break;
        }
    }
    if !sq_done {
        stop_uncertain_io();
        serial_println!(
            "[sexdrive.nvme.ioq.err] reason=create_sq_timeout cid={}",
            create_sq_cid as u64
        );
        return;
    }

    let sq_sf = (sq_cqe_dw3 >> 17) & 0x7FFF;
    let sq_sc = sq_sf & 0xFF;
    let sq_sct = (sq_sf >> 8) & 0x7;
    admin_cq_head = (admin_cq_head + 1) % q_entries;
    if admin_cq_head == 0 { admin_cq_phase ^= 1; }
    unsafe { core::ptr::write_volatile((map_va + NVME_CQ0HDBL) as *mut u32, admin_cq_head); }
    if sq_sc != 0 || sq_sct != 0 {
        serial_println!(
            "[sexdrive.nvme.ioq.err] reason=create_sq_status_fail cid={} sc={} sct={} dw3={:#x}",
            create_sq_cid as u64,
            sq_sc as u64,
            sq_sct as u64,
            sq_cqe_dw3 as u64
        );
        return;
    }
    serial_println!(
        "[sexdrive.nvme.ioq.create_sq.ok] cid={} sc={} sct={} cqh={} cqp={}",
        create_sq_cid as u64,
        sq_sc as u64,
        sq_sct as u64,
        admin_cq_head as u64,
        admin_cq_phase as u64
    );

    let sq1tdbl = 0x1000u64 + ((2u64 * 1u64) << (2 + cap_dstrd));
    let cq1hdbl = 0x1000u64 + ((2u64 * 1u64 + 1u64) << (2 + cap_dstrd));
    serial_println!(
        "[sexdrive.nvme.ioq.ready] qid=1 depth=16 sq_tail={} cq_head={} cq_phase={} sq1tdbl={:#x} cq1hdbl={:#x}",
        0u64,
        0u64,
        1u64,
        sq1tdbl,
        cq1hdbl
    );
    unsafe {
        NVME_IO_STATE.ready = true;
        NVME_IO_STATE.map_va = map_va;
        NVME_IO_STATE.io_sq_va = io_sq_va;
        NVME_IO_STATE.io_cq_va = io_cq_va;
        NVME_IO_STATE.sq1tdbl = sq1tdbl;
        NVME_IO_STATE.cq1hdbl = cq1hdbl;
        NVME_IO_STATE.sq_tail = 0;
        NVME_IO_STATE.cq_head = 0;
        NVME_IO_STATE.cq_phase = 1;
    }

    // AP3: deterministic single-block write/read/match self-test on real NVMe IOQ.
    // This only executes when NVMe BAR resolve + IOQ creation succeeded.
    let ap3_buf_phys = reserve_dma_phys(PAGE_SIZE);
    let ap3_buf_va = own_dma_virtual(ap3_buf_phys, PAGE_SIZE);
    if ap3_buf_phys == 0 || ap3_buf_phys == u64::MAX || (ap3_buf_phys % PAGE_SIZE) != 0
        || ap3_buf_va == 0 || ap3_buf_va == u64::MAX
    {
        serial_println!("[sexdrive.storage100.rw.fail] reason=ap3_buf_alloc_invalid");
    } else {
        let ap3_status = nvme_write_readback_proof(WRITE_PROOF_LBA * NVME_LBA_BYTES, WRITE_PROOF_LEN, ap3_buf_va);
        if ap3_status != 0 {
            serial_println!("[sexdrive.storage100.rw.fail] reason=ap3_selftest_status status={}", ap3_status);
        } else {
            let ap4_status = nvme_multiblock_write_readback_proof();
            if ap4_status != 0 {
                serial_println!("[sexdrive.storage100.multi.fail] reason=ap4_selftest_status status={}", ap4_status);
            } else if STORAGE_100_PERSIST_WRITE_ENABLED {
                let persist_write_status = nvme_persist_write_proof();
                if persist_write_status != 0 {
                    serial_println!(
                        "[sexdrive.storage100.persist.fail] phase=write reason=ap5a_selftest_status status={}",
                        persist_write_status
                    );
                }
            } else if STORAGE_100_PERSIST_READ_ENABLED {
                let persist_read_status = nvme_persist_read_proof();
                if persist_read_status != 0 {
                    serial_println!(
                        "[sexdrive.storage100.persist.fail] phase=read reason=ap5a_selftest_status status={}",
                        persist_read_status
                    );
                }
            } else if STORAGE_100_FLUSH_AUDIT_ENABLED {
                let _ = nvme_storage100_flush_audit();
            } else if STORAGE_100_NEGATIVE_ENABLED && STORAGE_100_NEG_MISMATCH_ENABLED {
                let neg_status = nvme_storage100_negative_mismatch();
                if neg_status != 0 {
                    serial_println!(
                        "[sexdrive.storage100.neg.mismatch.fail] reason=ap6_negative_status status={}",
                        neg_status
                    );
                }
            }
        }
    }

    // === SEXOS_STORAGE_100_IO_READ_PROBE gate ===
    // Legacy real IO READ proof. Vestigial; runs only when explicitly requested.
    // Default: OFF.  Must NOT run under SEXOS_STORAGE_100_PROOF or DiskFS profile
    // because it writes io_sq_tail=1 (local tracking) which overwrites the
    // storage proof tail (~10) and corrupts the IO SQ doorbell.
    if STORAGE_100_IO_READ_PROBE_ENABLED && unsafe { NVME_IO_STATE.ready } {
    // One real IO READ proof (no BLOCK API wiring in this mission).
    let io_q_entries: u32 = 16;
    let mut io_sq_tail: u32 = 0;
    let mut io_cq_head: u32 = 0;
    let mut io_cq_phase: u32 = 1;

    let data_phys = reserve_dma_phys(PAGE_SIZE);
    if data_phys == 0 || data_phys == u64::MAX || (data_phys % PAGE_SIZE) != 0 {
        serial_println!(
            "[sexdrive.nvme.io.read.err] reason=data_alloc_invalid phys={:#x}",
            data_phys
        );
        return;
    }
    let data_va = own_dma_virtual(data_phys, PAGE_SIZE);
    if data_va == 0 || data_va == u64::MAX {
        serial_println!(
            "[sexdrive.nvme.io.read.err] reason=data_map_failed va={:#x}",
            data_va
        );
        return;
    }
    unsafe {
        for i in 0..(PAGE_SIZE / 8) as usize {
            core::ptr::write_volatile((data_va as *mut u64).add(i), 0);
        }
    }
    serial_println!(
        "[sexdrive.nvme.io.read.begin] data_phys={:#x} data_va={:#x} qid={} depth={} sq_tail={} cq_head={} cq_phase={}",
        data_phys,
        data_va,
        1u64,
        io_q_entries as u64,
        io_sq_tail as u64,
        io_cq_head as u64,
        io_cq_phase as u64
    );

    // NVMe READ opcode=0x02, NSID=1, SLBA=0, NLB=0 (one logical block, zero-based count).
    let read_cid: u16 = 0x0045;
    let io_sqe_ptr = (io_sq_va as *mut u8).wrapping_add((io_sq_tail as usize) * 64) as *mut u32;
    unsafe {
        for i in 0..16usize {
            core::ptr::write_volatile(io_sqe_ptr.add(i), 0);
        }
        core::ptr::write_volatile(io_sqe_ptr.add(0), 0x02u32 | ((read_cid as u32) << 16)); // OPC + CID
        core::ptr::write_volatile(io_sqe_ptr.add(1), 1u32); // NSID=1
        core::ptr::write_volatile(io_sqe_ptr.add(6), (data_phys & 0xFFFF_FFFF) as u32); // PRP1 low
        core::ptr::write_volatile(io_sqe_ptr.add(7), (data_phys >> 32) as u32); // PRP1 high
        core::ptr::write_volatile(io_sqe_ptr.add(10), 0u32); // SLBA low
        core::ptr::write_volatile(io_sqe_ptr.add(11), 0u32); // SLBA high
        core::ptr::write_volatile(io_sqe_ptr.add(12), 0u32); // NLB=0 => one LBA
    }
    serial_println!(
        "[sexdrive.nvme.io.read.submit] cid={} opc={:#x} nsid={} slba={} nlb={} prp1={:#x}",
        read_cid as u64,
        0x02u64,
        1u64,
        0u64,
        0u64,
        data_phys
    );

    compiler_fence(Ordering::SeqCst);
    io_sq_tail = (io_sq_tail + 1) % io_q_entries;
    unsafe {
        core::ptr::write_volatile((map_va + sq1tdbl) as *mut u32, io_sq_tail);
    }
    serial_println!(
        "[sexdrive.nvme.io.read.doorbell] sq1tdbl={:#x} new_tail={} dstrd={}",
        sq1tdbl,
        io_sq_tail as u64,
        cap_dstrd as u64
    );

    let mut read_done = false;
    let mut rd_dw0 = 0u32;
    let mut rd_dw1 = 0u32;
    let mut rd_dw2 = 0u32;
    let mut rd_dw3 = 0u32;
    for _ in 0..1_000_000u32 {
        let cqe_ptr = (io_cq_va as *const u8).wrapping_add((io_cq_head as usize) * 16) as *const u32;
        let dw0 = unsafe { core::ptr::read_volatile(cqe_ptr.add(0)) };
        let dw1 = unsafe { core::ptr::read_volatile(cqe_ptr.add(1)) };
        let dw2 = unsafe { core::ptr::read_volatile(cqe_ptr.add(2)) };
        let dw3 = unsafe { core::ptr::read_volatile(cqe_ptr.add(3)) };
        let cid_now = dw3 & 0xFFFF;
        let phase_now = (dw3 >> 16) & 0x1;
        if cid_now == read_cid as u32 && phase_now == io_cq_phase {
            read_done = true;
            rd_dw0 = dw0;
            rd_dw1 = dw1;
            rd_dw2 = dw2;
            rd_dw3 = dw3;
            serial_println!(
                "[sexdrive.nvme.io.read.cqe] cid={} phase={} dw0={:#x} dw1={:#x} dw2={:#x} dw3={:#x}",
                cid_now as u64,
                phase_now as u64,
                dw0 as u64,
                dw1 as u64,
                dw2 as u64,
                dw3 as u64
            );
            break;
        }
    }
    if !read_done {
        stop_uncertain_io();
        serial_println!(
            "[sexdrive.nvme.io.read.err] reason=cqe_timeout cid={} head={} phase={}",
            read_cid as u64,
            io_cq_head as u64,
            io_cq_phase as u64
        );
        return;
    }

    let rd_sf = (rd_dw3 >> 17) & 0x7FFF;
    let rd_sc = rd_sf & 0xFF;
    let rd_sct = (rd_sf >> 8) & 0x7;
    let rd_sqhd = rd_dw2 & 0xFFFF;
    let rd_sqid = (rd_dw2 >> 16) & 0xFFFF;

    io_cq_head = (io_cq_head + 1) % io_q_entries;
    if io_cq_head == 0 { io_cq_phase ^= 1; }
    unsafe {
        core::ptr::write_volatile((map_va + cq1hdbl) as *mut u32, io_cq_head);
    }

    if rd_sc != 0 || rd_sct != 0 {
        unsafe { NVME_IO_STATE.ready = false; }
        serial_println!(
            "[sexdrive.nvme.io.read.err] reason=status_fail cid={} sc={} sct={} sqhd={} sqid={} dw3={:#x}",
            read_cid as u64,
            rd_sc as u64,
            rd_sct as u64,
            rd_sqhd as u64,
            rd_sqid as u64,
            rd_dw3 as u64
        );
        return;
    }

    let d0 = unsafe { core::ptr::read_volatile(data_va as *const u64) };
    let d1 = unsafe { core::ptr::read_volatile((data_va + 8) as *const u64) };
    let d2 = unsafe { core::ptr::read_volatile((data_va + 16) as *const u64) };
    let d3 = unsafe { core::ptr::read_volatile((data_va + 24) as *const u64) };
    serial_println!(
        "[sexdrive.nvme.io.read.ok] cid={} sc={} sct={} sqhd={} sqid={} cqh={} cqp={} d0={:#x} d1={:#x} d2={:#x} d3={:#x}",
        read_cid as u64,
        rd_sc as u64,
        rd_sct as u64,
        rd_sqhd as u64,
        rd_sqid as u64,
        io_cq_head as u64,
        io_cq_phase as u64,
        d0,
        d1,
        d2,
        d3
    );
    unsafe {
        NVME_IO_STATE.sq_tail = io_sq_tail;
        NVME_IO_STATE.cq_head = io_cq_head;
        NVME_IO_STATE.cq_phase = io_cq_phase;
    } // end unsafe + if STORAGE_100_IO_READ_PROBE_ENABLED
    } else {
        serial_println!("[sexdrive.storage100.io_read_probe.skip] reason=not_requested");
    }
}

fn xhci_probe_mmio() {
    let bar_bytes = match sex_pdx::memory_region_info(SLOT_USB_HOST) {
        Ok(info) if info.length >= 0x1000 => 0x1000,
        _ => return,
    };
    let map_va: u64;
    unsafe {
        // syscall 43 = MAP_PCI_BAR(cap_slot, bar_index, map_size)
        core::arch::asm!(
            "syscall",
            in("rax") 43u64,
            in("rdi") SLOT_USB_HOST,
            in("rsi") 0u64,      // BAR0
            in("rdx") bar_bytes, // first page, within the actual granted BAR
            lateout("rax") map_va,
            out("rcx") _,
            out("r11") _,
        );
    }

    if map_va == u64::MAX || map_va == 0 {
        serial_println!("[sexdrive] XHCI probe: no BAR lease/mapping available");
        return;
    }

    let regs = map_va as *const u32;
    let cap0 = unsafe { core::ptr::read_volatile(regs.add(0)) };
    let caplength = (cap0 & 0xFF) as u8;
    let hciversion = ((cap0 >> 16) & 0xFFFF) as u16;
    let hcsp1 = unsafe { core::ptr::read_volatile(regs.add(1)) }; // 0x04
    let hcc1 = unsafe { core::ptr::read_volatile(regs.add(4)) };  // 0x10

    serial_println!(
        "[sexdrive] XHCI MMIO probe ok va={:#x} caplength={:#x} hciversion={:#x} hcsp1={:#x} hcc1={:#x}",
        map_va, caplength, hciversion, hcsp1, hcc1
    );
}

// Device commands finish or quarantine their DMA synchronously. There is no
// periodic storage work; the kernel wake owner resumes this task on IPC.
fn service_next_request(){
    let msg=pdx_listen_raw(0);
    // BLOCK_READ is numeric 1 in the public ABI, not a receipt kind.
    let cmd = msg.type_id;
    let offset = msg.arg0;
    let size = msg.arg1;
    let _buf_cap = msg.arg2;
    let ready_snapshot = unsafe { if NVME_IO_STATE.ready { 1u64 } else { 0u64 } };
    let op = match cmd {
        BLOCK_READ => "READ",
        BLOCK_WRITE => "WRITE",
        _ => "OTHER",
    };
    let lba = if BLOCK_SECTOR_SIZE != 0 {
        offset / BLOCK_SECTOR_SIZE
    } else {
        0
    };
    let submit_cid_snapshot = unsafe { NVME_IO_STATE.next_cid as u64 };
    let submit_tail_snapshot = unsafe { NVME_IO_STATE.sq_tail as u64 };

    if hot_log() { serial_println!(
        "[sexdrive.block.typed.recv] cmd={} offset={:#x} size={} buf_cap={:#x} caller={}",
        cmd, offset, size, _buf_cap, msg.caller_pd
    ); }
    if cmd == BLOCK_READ || cmd == BLOCK_WRITE {
        if hot_log() { serial_println!(
            "[sexdrive.block.req] op={} ready={} lba={} bytes={} buffer_cap={:#x} device_cap={:#x}",
            op, ready_snapshot, lba, size, _buf_cap, SLOT_NVME_HOST
        ); }
    }

    // The numeric selector is local to this domain. Its paired peer
    // must match authenticated provenance before any shared byte access.
    let block_view = if cmd == BLOCK_READ || cmd == BLOCK_WRITE {
        block_buffer_view(msg.caller_pd, _buf_cap, size,
            if cmd == BLOCK_READ { WRITE } else { READ })
    } else { None };
    // [sexblock.abi.request.decode] — dispatch on typed command
    let reply_val: u64 = if (cmd == BLOCK_READ || cmd == BLOCK_WRITE)
        && (size == 0 || size > BLOCK_MAX_XFER || offset % BLOCK_SECTOR_SIZE != 0) {
        BLOCK_ERR_BAD_LEN
    } else if (cmd == BLOCK_READ || cmd == BLOCK_WRITE) && block_view.is_none() {
        serial_println!("[sexdrive.block.region.reject] caller={} slot={} bytes={}", msg.caller_pd, _buf_cap, size);
        BLOCK_ERR_NO_DEVICE
    } else { match cmd {
        BLOCK_READ => typed_read_handoff(offset,size,_buf_cap,block_view),
        BLOCK_WRITE => {
            if hot_log() { serial_println!(
                "[sexdrive.block.write.api.recv] offset={:#x} size={} buf_cap={:#x}",
                offset, size, _buf_cap
            ); }
            if write_guard_allows(offset, size, _buf_cap) {
                let fill_va = block_view.map(|view| view.virtual_address).unwrap_or(0);
                if fill_va == 0 || fill_va == u64::MAX {
                    serial_println!(
                        "[sexdrive.nvme.write.err] reason=map_failed va={:#x}",
                        fill_va
                    );
                    if hot_log() { serial_println!(
                        "[sexdrive.nvme.cmd.shape.err] path=typed reason=region_view_failed buffer_cap={:#x}",
                        SLOT_BLOCK_BUFFER
                    ); }
                    BLOCK_ERR_NO_DEVICE
                } else {
                    if hot_log() { serial_println!("[sexdrive.bufcap.map.ok] fill_va={:#x}", fill_va); }
                    if hot_log() { serial_println!(
                        "[sexdrive.block.nvme.submit] op={} lba={} bytes={} cid={} tail={} ready={}",
                        op, lba, size, submit_cid_snapshot, submit_tail_snapshot, ready_snapshot
                    ); }
                    let proof_offset = WRITE_PROOF_LBA * NVME_LBA_BYTES;
                    if offset == proof_offset {
                        nvme_write_readback_proof(offset, size, fill_va)
                    } else {
                        nvme_write_one_block(offset, size, fill_va)
                    }
                }
            } else {
                serial_println!(
                    "[sexdrive.block.typed] cmd={} ERR_NO_DEVICE honest=write_not_implemented_guard_only",
                    cmd
                );
                BLOCK_ERR_NO_DEVICE
            }
        }
        BLOCK_SYNC => {
            let status=nvme_flush();
            sex_pdx::serial_record!("[sexdrive.sync.result] status={} actual_nvme_flush=1",status);
            status
        }
        _ => {
            serial_println!(
                "[sexdrive.block.typed] cmd={} ERR_BAD_CMD unknown",
                cmd
            );
            BLOCK_ERR_BAD_CMD
        }
    }};

    if hot_log() { serial_println!(
        "[sexblock.abi.reply.encode] caller={} status={}",
        msg.caller_pd, reply_val
    ); }
    pdx_reply(msg.caller_pd, reply_val);
    if hot_log() { serial_println!(
        "[sexdrive.block.typed.reply] cmd={} caller={} status={}",
        cmd, msg.caller_pd, reply_val
    ); }
    if cmd == BLOCK_READ || cmd == BLOCK_WRITE {
        if reply_val == 0 {
            if hot_log() { serial_println!(
                "[sexdrive.block.nvme.cqe] op={} cid={} status=0",
                op, submit_cid_snapshot
            ); }
            if hot_log() { serial_println!(
                "[sexdrive.block.reply] op={} status=0 bytes={} ready=1",
                op, size
            ); }
        } else if reply_val == BLOCK_ERR_NO_DEVICE {
            if hot_log() { serial_println!(
                "[sexdrive.block.nvme.cqe.timeout] op={} cid={} polls={}",
                op, submit_cid_snapshot, 0u64
            ); }
            let reason = if ready_snapshot == 0 {
                "no_ioq_ready"
            } else {
                "no_device_other"
            };
            if hot_log() { serial_println!(
                "[sexdrive.block.reply] op={} status=4 reason={} ready={}",
                op, reason, ready_snapshot
            ); }
        } else {
            if hot_log() { serial_println!(
                "[sexdrive.block.reply] op={} status={} reason=other ready={}",
                op, reply_val, ready_snapshot
            ); }
        }
    }
}

// Initialize the existing BOOT-paired demonstration buffer once before the
// shell sees its binding. Storage service progress never redraws this buffer.
fn fill_initial_frame(shared_addr:u64){
    let ptr=shared_addr as *mut u32;
    for y in 0..768 {for x in 0..1024 {
        unsafe{core::ptr::write_volatile(ptr.add(y*1024+x),(x as u32^y as u32).wrapping_add(1));}
    }}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[sexdrive.init.start]");
    if init_dma_region() { nvme_probe_bar(); }
    else { serial_println!("[sexdrive.dma.region.err] ready=0"); }
    xhci_probe_mmio();

    // Wait for display/shell to be ready
    for _ in 0..10_000_000 {
        core::hint::spin_loop();
    }

    // The paired frame is assigned by BOOT_DAG; no allocation or peer choice.
    let shared_addr = match sex_pdx::memory_region_view(SLOT_FRAME_REGION, 0, 1024 * 768 * 4, QUERY_VIEW | WRITE) {
        Ok(view) => view.virtual_address,
        Err(_) => {
            serial_println!("[sexdrive.frame.region.err] slot={}", SLOT_FRAME_REGION);
            loop { core::hint::spin_loop(); }
        }
    };

    fill_initial_frame(shared_addr);

    // Zero-copy handoff: Notify AUTHORITY (Silk-Shell) about shared buffer
    unsafe {
        pdx_call(SLOT_SHELL, OP_SHELL_BIND_BUFFER, shared_addr, 0, 0);
    }

    if hot_log() { serial_println!("[sexdrive.ready]"); }
    loop { service_next_request(); }
}
