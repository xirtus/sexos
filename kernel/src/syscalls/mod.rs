pub mod spawn;
pub mod fs;
pub mod storage;
pub mod pipe;
pub mod fork;
pub mod translator;
pub mod store;

use crate::interrupts::SyscallRegs;
use crate::ipc::messages::MessageType;
use core::sync::atomic::{AtomicUsize, Ordering};
use x86_64::VirtAddr;

const SNAPSHOT_MAX: usize = 64;
const RAW_PRINT_MAX: usize = 4096;

/// Reject addresses whose high bits do not match bit 47 (x86_64 canonical form).
#[inline]
fn is_canonical_addr(addr: u64) -> bool {
    let sign = (addr >> 47) & 1;
    let high = addr >> 48;
    (sign == 0 && high == 0) || (sign == 1 && high == 0xFFFF)
}

/// Validate that every 4 KiB page touched by `[start, start + len)` is mapped,
/// user-accessible, and has the requested read/write permissions under the
/// current PD's user-mode PKRU mask.
///
/// Does NOT hold GLOBAL_VAS or any allocator lock; it only reads the existing
/// page tables via `read_pte_flags`.
fn validate_user_bytes(start: u64, len: usize, writable: bool) -> bool {
    if len == 0 {
        return true;
    }
    if !is_canonical_addr(start) {
        return false;
    }
    // Validate the inclusive end address independently before any page
    // alignment, masking, or translation.  Reject arithmetic wraparound
    // and any range that touches a non-canonical byte.
    let inclusive_end = match start.checked_add((len as u64) - 1) {
        Some(e) => e,
        None => return false,
    };
    if !is_canonical_addr(inclusive_end) {
        return false;
    }

    let pd = crate::core_local::CoreLocal::get().current_pd_ref();
    let pkru = pd.current_pkru_mask.load(Ordering::Acquire);

    let first_page = start & !0xFFFu64;
    let last_page = inclusive_end & !0xFFFu64;
    let mut va = first_page;
    loop {
        match crate::memory::manager::read_pte_flags(x86_64::VirtAddr::new(va)) {
            Ok(pte) => {
                if (pte & 0x1) == 0 {
                    return false; // not present
                }
                if (pte & 0x4) == 0 {
                    return false; // not user-accessible
                }
                if writable && (pte & 0x2) == 0 {
                    return false; // not writable
                }
                let pkey = ((pte >> 59) & 0xF) as u32;
                let shift = pkey * 2;
                if ((pkru >> shift) & 1) != 0 {
                    return false; // access disabled for this pkey
                }
                if writable && ((pkru >> (shift + 1)) & 1) != 0 {
                    return false; // write disabled for this pkey
                }
            }
            Err(_) => return false,
        }
        if va == last_page {
            break;
        }
        va = match va.checked_add(4096) {
            Some(v) => v,
            None => return false,
        };
    }
    true
}

static mut SNAP_RING: [sex_pdx::SceneSnapshot; SNAPSHOT_MAX] = [sex_pdx::SceneSnapshot {
    layers_ptr: 0, layers_len: 0, cursor_x: 0, cursor_y: 0,
    is_incremental: 0, damage_rects_ptr: 0, damage_rects_len: 0,
}; SNAPSHOT_MAX];

static SNAP_IDX: AtomicUsize = AtomicUsize::new(0);
static mut SNAP_GEN: [u32; SNAPSHOT_MAX] = [0u32; SNAPSHOT_MAX];

// SNAPSHOT_HANDLE = (gen << 32 | idx)
// prevents stale slot reuse attacks after ring wrap
unsafe fn snapshot_ingest(src: *const sex_pdx::SceneSnapshot) -> Result<u64, u64> {
    let size = core::mem::size_of::<sex_pdx::SceneSnapshot>();
    if src.is_null()
        || (src as usize) % core::mem::align_of::<sex_pdx::SceneSnapshot>() != 0
        || !validate_user_bytes(src as u64, size, false)
    {
        return Err(sex_pdx::ERR_CAP_INVALID);
    }
    let idx = SNAP_IDX.fetch_add(1, Ordering::AcqRel) % SNAPSHOT_MAX;
    SNAP_GEN[idx] = SNAP_GEN[idx].wrapping_add(1);
    core::ptr::copy_nonoverlapping(src, &mut SNAP_RING[idx], 1);
    Ok(((SNAP_GEN[idx] as u64) << 32) | (idx as u64))
}

// Single-path: validate → copy → return status. One unsafe block.
fn snapshot_resolve(handle: u64, out_ptr: *mut sex_pdx::SceneSnapshot) -> u64 {
    let idx = (handle & 0xFFFF_FFFF) as usize;
    let gen = (handle >> 32) as u32;
    let size = core::mem::size_of::<sex_pdx::SceneSnapshot>();
    if idx >= SNAPSHOT_MAX
        || out_ptr.is_null()
        || !validate_user_bytes(out_ptr as u64, size, true)
    {
        return sex_pdx::ERR_CAP_INVALID;
    }
    unsafe {
        if SNAP_GEN[idx] != gen {
            return sex_pdx::ERR_CAP_INVALID; // stale handle
        }
        *out_ptr = SNAP_RING[idx];
    }
    0u64
}

#[repr(C)]
pub struct DisplayInfo {
    pub virt_addr: u64,
    pub width:     u32,
    pub height:    u32,
    pub pitch:     u32, // Pixels per row
}

/// Exact runtime region ABI, also exercised by the boot negative selftest.
/// Failed replies expose no prior input or address register values.
pub(crate) fn region_query(pd: &crate::capability::ProtectionDomain, number: u64,
    slot: u64, offset: u64, length: u64, rights: u64) -> [u64; 5]
{
    use crate::capability::CapabilityData;
    use sex_pdx::memory_region::{resolve_info, resolve_register_view};
    let fail = [u64::MAX, 0, 0, 0, 0];
    let Some(slot) = crate::capability_slot::decode_capability_slot(slot) else { return fail; };
    let grant = match pd.find_capability(slot).map(|cap| cap.data) {
        Some(CapabilityData::Region(grant)) => Some(grant),
        Some(CapabilityData::Pci(data)) => data.bar0_view,
        _ => None,
    };
    match number {
        54 => match resolve_register_view(grant.as_ref(), offset, length, rights) {
            Ok(view) => [0, view.virtual_address, view.length, view.dma_address.unwrap_or(0), view.region_id as u64],
            Err(_) => fail,
        },
        55 => match resolve_info(grant.as_ref()) {
            Ok(info) => [0, info.length, info.rights as u64, info.peer_pd.unwrap_or(0) as u64, info.region_id as u64],
            Err(_) => fail,
        },
        _ => fail,
    }
}

pub fn dispatch(regs: &mut SyscallRegs) -> u64 {
    let rax = regs.rax;
    let rdi = regs.rdi;
    let rsi = regs.rsi;
    let rdx = regs.rdx;
    let r10 = regs.r10;
    let r8  = regs.r8;

    // Reject full-width selector aliases before lookup, dequeue, allocation,
    // mapping, or reply insertion. Validated selector casts below are safe.
    if !crate::capability_slot::valid_syscall_selectors(rax, rdi, rdx) {
        let error = match rax {
            0 => {
                regs.rsi = 0;
                sex_pdx::ERR_CAP_INVALID
            }
            28 => {
                // Preserve the existing invalid-listen/empty result contract.
                regs.rsi = 0;
                regs.rdx = 0;
                regs.r10 = 0;
                regs.r8 = 0;
                0
            }
            29 => 1,
            68 | 69 | 71 => {
                regs.rsi=0;regs.rdx=0;regs.r10=0;regs.r8=0;
                sex_pdx::pci_command::DENIED
            },
            _ => u64::MAX,
        };
        regs.rax = error;
        return error;
    }

    let result = match rax {
        0 => { // SYSCALL_PDX_CALL
            let slot = rdi as u32;
            let num = rsi;
            let arg0 = rdx;
            let arg1 = r10;
            let arg2 = r8;

            let (status, value) = if slot == 0 {
                match num {
                    0xE6 => {
                        let caller=crate::core_local::CoreLocal::get().current_pd_ref().id;
                        match crate::boot_display::query(caller,arg0,arg1,arg2){Some(value)=>(0,value),None=>(sex_pdx::ERR_CAP_INVALID,0)}
                    }
                    0xE3 => { // PDX_GET_DISPLAY_INFO
                        let ptr = arg0 as *mut DisplayInfo;
                        let info_size = core::mem::size_of::<DisplayInfo>();
                        if ptr.is_null() || !validate_user_bytes(arg0, info_size, true) {
                            (sex_pdx::ERR_CAP_INVALID, 0)
                        } else if let Some(fb_res) = crate::FB_REQUEST.response() {
                            if let Some(fb) = fb_res.framebuffers().iter().next() {
                                unsafe {
                                    let fb_virt = fb.address() as u64;
                                    let hhdm = crate::HHDM_REQUEST.response().unwrap().offset;
                                    (*ptr).virt_addr = fb_virt - hhdm;
                                    (*ptr).width = fb.width as u32;
                                    (*ptr).height = fb.height as u32;
                                    (*ptr).pitch = (fb.pitch / 4) as u32;
                                }
                                (0u64, 0u64)
                            } else { (sex_pdx::ERR_CAP_INVALID, 0) }
                        } else { (sex_pdx::ERR_CAP_INVALID, 0) }
                    }
                    69 => { // raw_print
                        let ptr = arg0 as *const u8;
                        let len = arg1 as usize;
                        if len == 0 {
                            (0u64, 0u64)
                        } else if ptr.is_null()
                            || len > RAW_PRINT_MAX
                            || !validate_user_bytes(arg0, len, false)
                        {
                            (sex_pdx::ERR_CAP_INVALID, 0)
                        } else {
                            unsafe {
                                let slice = core::slice::from_raw_parts(ptr, len);
                                for &b in slice {
                                    use x86_64::instructions::port::Port;
                                    let mut port = Port::new(0x3f8);
                                    port.write(b);
                                }
                            }
                            (0u64, 0u64)
                        }
                    }
                    0x12 => { // pdx_get_init_arg
                        let core_local = crate::core_local::CoreLocal::get();
                        let sched = &crate::scheduler::SCHEDULERS[core_local.core_id as usize];
                        let current_task_ptr = sched.current_task.load(core::sync::atomic::Ordering::Acquire);
                        if !current_task_ptr.is_null() {
                            let task = unsafe { &mut *current_task_ptr };
                            // Phase 1.5: Pure structural ownership via move semantics.
                            // No identity/PD-ID checks here.
                            if let Some(init_arg) = task.ext_init.take() {
                                let lease = init_arg.display_lease;
                                // Pack lease: [8:bus][8:dev][8:func][16:vendor][16:device]
                                let packed = (lease.bus as u64) 
                                           | ((lease.dev as u64) << 8)
                                           | ((lease.func as u64) << 16)
                                           | ((lease.vendor_id as u64) << 24)
                                           | ((lease.device_id as u64) << 40);
                                (0u64, packed)
                            } else { (sex_pdx::ERR_CAP_INVALID, 0) }
                        } else { (sex_pdx::ERR_CAP_INVALID, 0) }
                    }
                    0xBB => { // Phase 2: FRAME_PRESENT debug hook
                        crate::serial_println!("PD1: FRAME_PRESENT event");
                        (0u64, 0u64)
                    }
                    _ => (sex_pdx::ERR_CAP_INVALID, 0),
                }
            } else {
                match crate::ipc::safe_pdx_call(slot, num, arg0, arg1, arg2) {
                    Ok(val) => (0u64, val),
                    Err(e)  => (e, 0u64),
                }
            };

            regs.rsi = value;
            status
        },

        28 => { // SYSCALL_PDX_LISTEN (Phase 25)
            let slot = rdi as u32;
            let core_local = crate::core_local::CoreLocal::get();
            let current_pd = core_local.current_pd_ref();

            let (type_id, s_caller_pd, s_arg0, s_arg1, s_arg2): (u64, u64, u64, u64, u64) = {
                // 1. Check priority reply buffer (Syscall 29 source)
                // Reply sender is captured by the kernel at send time.
                let mut replies = current_pd.incoming_replies.lock();
                if slot == 0 && !replies.is_empty() {
                    if let Some(reply) = replies.pop_front() {
                        drop(replies);
                        (0x1, reply.caller_pd as u64, reply.value, 0, 0)
                    } else {
                        drop(replies);
                        (0, 0, 0, 0, 0)
                    }
                } else if slot == 0 {
                    // Slot 0 with no pending reply → always dequeue from self message ring.
                    // (The old cap-table path was dead because find_capability(0) is rejected
                    //  by the 1-indexed id guard in CapabilityTable::find.)
                    drop(replies);
                    unsafe {
                        if let Some(msg) = (*current_pd.message_ring).dequeue() {
                            match msg {
                                MessageType::IpcCall { func_id, arg0, arg1, arg2, caller_pd } => {
                                    (func_id, caller_pd as u64, arg0, arg1, arg2)
                                }
                                MessageType::DisplayPrimaryFramebuffer { virt_addr, width, height, pitch } => {
                                    (0x11, 1, virt_addr, (width as u64) | ((height as u64) << 32), pitch as u64)
                                }
                                MessageType::RawInput(scancode) => {
                                    (0x201, 1, scancode as u64, 0, 0)
                                }
                                _ => {
                                    let tid: u64 = match msg {
                                        MessageType::WindowCreate       => 0xDE,
                                        MessageType::CompositorCommit   => 0xDD,
                                        MessageType::SetWindowRoundness => 0xDF,
                                        MessageType::SetWindowBlur      => 0xE0,
                                        MessageType::GetDisplayInfo     => 0xE3,
                                        _                               => 0xFF,
                                    };
                                    (tid, 1, 0, 0, 0)
                                }
                            }
                        } else {
                            (0, 0, 0, 0, 0) // EMPTY
                        }
                    }
                } else {
                    drop(replies);

                    // 2. Resolve Slot to Capability (non-zero slots only)
                    use crate::capability::CapabilityData;
                    let cap = current_pd.find_capability(slot);

                    match cap.map(|c| c.data) {
                        Some(CapabilityData::InputRing) => {
                            if let Some(scancode) = crate::interrupts::INPUT_RING.dequeue() {
                                (0x201, 1, scancode as u64, 0, 0)
                            } else {
                                (0, 0, 0, 0, 0)
                            }
                        }
                        Some(CapabilityData::MessageQueue) => {
                            unsafe {
                                if let Some(msg) = (*current_pd.message_ring).dequeue() {
                                    match msg {
                                        MessageType::IpcCall { func_id, arg0, arg1, arg2, caller_pd } => {
                                            (func_id, caller_pd as u64, arg0, arg1, arg2)
                                        }
                                        MessageType::DisplayPrimaryFramebuffer { virt_addr, width, height, pitch } => {
                                            (0x11, 1, virt_addr, (width as u64) | ((height as u64) << 32), pitch as u64)
                                        }
                                        MessageType::RawInput(scancode) => {
                                            (0x201, 1, scancode as u64, 0, 0)
                                        }
                                        _ => {
                                            let tid: u64 = match msg {
                                                MessageType::WindowCreate       => 0xDE,
                                                MessageType::CompositorCommit   => 0xDD,
                                                MessageType::SetWindowRoundness => 0xDF,
                                                MessageType::SetWindowBlur      => 0xE0,
                                                MessageType::GetDisplayInfo     => 0xE3,
                                                _                               => 0xFF,
                                            };
                                            (tid, 1, 0, 0, 0)
                                        }
                                    }
                                } else {
                                    (0, 0, 0, 0, 0)
                                }
                            }
                        }
                        _ => (0, 0, 0, 0, 0), // Unknown/None -> Empty
                    }
                }
            };

            regs.rsi = s_caller_pd;
            regs.rdx = s_arg0;
            regs.r10 = s_arg1;
            regs.r8  = s_arg2;

            type_id
        },

        29 => { // SYSCALL_PDX_REPLY
            let target_pd_id = rdi as u32;
            let val = rsi;
            if crate::ipc::router::send_reply(target_pd_id, val).is_ok() { 0 } else { 1 }
        },

        // Ambient physical allocation/mapping and runtime capability minting
        // are unavailable. Boot regions are queried through54/55 only.
        30 | 31 | 50 | 51 => u64::MAX,
        40 => 0, // historical shared-buffer failure convention

        32 => { // SYSCALL_YIELD — cooperative scheduling fallback
            // When the preemptive LAPIC timer does not fire (QEMU limitation),
            // perform a full context switch to the next runnable PD here.
            // yield_and_switch saves the current user context from the syscall
            // entry stub's saved registers, then calls tick()+switch_to().
            // This function never returns; the current PD resumes only when
            // the scheduler round-robins back to it.
            unsafe {
                crate::scheduler::yield_and_switch(regs as *const _);
            }
        },

        43 => { // Existing ABI, now an idempotent boot BAR0 view query.
            let pd = crate::core_local::CoreLocal::get().current_pd_ref();
            let grant = match pd.find_capability(rdi as u32).map(|cap| cap.data) {
                Some(crate::capability::CapabilityData::Pci(data)) => data.bar0_view,
                _ => None,
            };
            if rsi != 0 { u64::MAX } else {
                let size = if rdx == 0 { 4096 } else { rdx };
                sex_pdx::memory_region::resolve_register_view(grant.as_ref(), 0, size,
                    (sex_pdx::memory_region::QUERY_VIEW | sex_pdx::memory_region::READ | sex_pdx::memory_region::WRITE) as u64)
                    .map(|view| view.virtual_address).unwrap_or(u64::MAX)
            }
        },
        54 | 55 => {
            let pd = crate::core_local::CoreLocal::get().current_pd_ref();
            let reply = region_query(pd, rax, rdi, rsi, rdx, r10);
            regs.rsi = reply[1];
            regs.rdx = reply[2];
            regs.r10 = reply[3];
            regs.r8 = reply[4];
            reply[0]
        },
        56 => {
            let pd = crate::core_local::CoreLocal::get().current_pd_ref();
            crate::paging::commit(pd,rdi,rsi,rdx,r10,r8)
        },
        57 | 59 => {
            let pd = crate::core_local::CoreLocal::get().current_pd_ref();
            let reply = if rax == 57 { crate::paging::query(pd,rdi,rsi) }
                else { crate::paging::info(pd,rdi) };
            regs.rsi=reply[1]; regs.rdx=reply[2]; regs.r10=reply[3]; regs.r8=reply[4];
            reply[0]
        },
        58 => {
            let pd = crate::core_local::CoreLocal::get().current_pd_ref();
            crate::paging::reject(pd,rdi,rsi)
        },
        60 => {
            let pd = crate::core_local::CoreLocal::get().current_pd_ref();
            crate::paging::reclaim(pd,rdi,rsi,rdx,r10)
        },
        61 => {
            let pd = crate::core_local::CoreLocal::get().current_pd_ref();
            let reply = crate::paging::cleanup(pd,rdi,rsi);
            regs.rsi=reply[1]; regs.rdx=reply[2]; regs.r10=0; regs.r8=0;
            reply[0]
        },

        62 => { // Self-only task finish; never returns to the exited frame.
            unsafe { crate::scheduler::finish_current_task(regs as *const _) }
        },
        63 => {
            let pd=crate::core_local::CoreLocal::get().current_pd_ref();
            let reply=crate::app_resources::status(pd,rdi,rsi);
            regs.rsi=reply[1]; regs.rdx=reply[2]; regs.r10=reply[3]; regs.r8=reply[4]; reply[0]
        },
        64 => {
            let pd=crate::core_local::CoreLocal::get().current_pd_ref();
            crate::app_resources::ack(pd,rdi,rsi)
        },
        65 => {
            let pd=crate::core_local::CoreLocal::get().current_pd_ref();
            let reply=crate::app_resources::retire(pd,rdi);
            regs.rsi=reply[1]; regs.rdx=reply[2]; regs.r10=reply[3]; regs.r8=0; reply[0]
        },

        67 => {
            let pd=crate::core_local::CoreLocal::get().current_pd_ref();
            let reply=crate::history_recovery::query(pd,rdi,rsi);
            regs.rsi=reply[1];regs.rdx=reply[2];regs.r10=reply[3];regs.r8=reply[4];reply[0]
        },

        66 => {
            let pd=crate::core_local::CoreLocal::get().current_pd_ref();
            let reply=crate::app_resources::activation(pd,rdi,rsi);
            regs.rsi=reply[1];regs.rdx=reply[2];regs.r10=reply[3];regs.r8=reply[4];reply[0]
        },

        68 => {
            let pd=crate::core_local::CoreLocal::get().current_pd_ref();
            let reply=crate::pci_command::invoke(pd,rdi,rsi,rdx,r10,r8);
            regs.rsi=reply[1];regs.rdx=reply[2];regs.r10=reply[3];regs.r8=reply[4];reply[0]
        },
        69 => {
            let pd = crate::core_local::CoreLocal::get().current_pd_ref();
            let reply=crate::pci_command::identity(pd,rdi,rsi,rdx,r10,r8);
            regs.rsi=reply[1];regs.rdx=reply[2];regs.r10=reply[3];regs.r8=reply[4];
            reply[0]
        },
        71 => {
            let pd=crate::core_local::CoreLocal::get().current_pd_ref();
            let reply=crate::pci_command::network_config(pd,rdi,rsi,rdx,r10,r8);
            regs.rsi=reply[1];regs.rdx=reply[2];regs.r10=reply[3];regs.r8=reply[4];reply[0]
        },

        34 => { // SYSCALL_GET_TICKS — exposes kernel TICKS counter
            crate::interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed)
        },

        42 => { // SYS_SET_STATE
            let state = rdx as u8;
            let core_local = crate::core_local::CoreLocal::get();
            let pd_id = core_local.current_pd_ref().id;
            if state == crate::ipc::state::SVC_STATE_LISTENING {
                crate::ipc::state::set_service_listening(pd_id);
            }
            0
        },

        52 => { // SYS_NET_DIAG
            match rdi {
                0 => {
                    let (status, bytes, source) = crate::hal::pci::get_net_diag();
                    let packed = ((status as u64) << 32)
                        | ((source as u64) << 16)
                        | (core::cmp::min(bytes, 0xFFFF) as u64);
                    crate::serial_println!(
                        "[net.diag.syscall.reply] status={} bytes={} source={}",
                        status,
                        bytes,
                        source
                    );
                    packed
                }
                1 => {
                    let len = crate::hal::pci::get_net_diag_body_len();
                    crate::serial_println!("[net.diag.body.len.reply] len={}", len);
                    len as u64
                }
                2..=9 => {
                    let idx = (rdi - 2) as usize;
                    let chunk = crate::hal::pci::get_net_diag_body_chunk(idx);
                    crate::serial_println!("[net.diag.body.chunk.reply] idx={} ok=1", idx);
                    chunk
                }
                _ => u64::MAX,
            }
        },

        70 => { // PDX_WAIT_SELF_TICKS: bounded delay, current BOOT task only.
            if rsi!=0||rdx!=0||r10!=0||r8!=0 { sex_pdx::ERR_CAP_INVALID }
            else {
                regs.rax=0;
                if unsafe {crate::scheduler::wait_for_message_ticks(regs as *const _,rdi)} {0}
                else {sex_pdx::ERR_CAP_INVALID}
            }
        },

        53 => { // PDX_WAIT_SELF: only this domain's own inbox/reply events.
            if rdi != 0 {
                sex_pdx::ERR_CAP_INVALID
            } else {
                let pd = crate::core_local::CoreLocal::get().current_pd_ref();
                // The pending-wake handshake covers publication after these
                // observations. No event is consumed by this syscall.
                let has_message = unsafe { (*pd.message_ring).len() != 0 };
                let has_reply = !pd.incoming_replies.lock().is_empty();
                if has_message || has_reply {
                    0
                } else {
                    regs.rax = 0;
                    unsafe { crate::scheduler::wait_for_message(regs as *const _) };
                    0
                }
            }
        },

        _ => u64::MAX,
    };

    regs.rax = result;
    result
}
