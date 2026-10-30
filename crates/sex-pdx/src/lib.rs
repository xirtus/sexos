//! sex-pdx — Runtime Capability Authority Layer (ARCHITECTURE.md §1.1)
//!
//! Implements the authority layer of the SexOS memory model (ARCHITECTURE.md §0).
//! Single runtime enforcement point. No inter-domain interaction bypasses this crate.
//! Capability slots are the only sanctioned access paths.

#![no_std]

pub mod pagefault;
pub mod display_ready;
pub mod shell_ready;
pub mod memory_region;
pub mod dma_pool;
pub mod paging;
pub mod app_lifecycle;
pub mod spindle_history;
pub mod spindle_probe;
pub mod spindle_view;
pub mod spindle_status;
pub mod history_recovery;
pub use memory_region::MemoryRegionInfo;

/// Opaque protection domain identifier. All inter-domain references use this type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainId(pub u32);

/// SexOS authority layer tags. Canonical definition: ARCHITECTURE.md §0.
///
/// These layers are ORTHOGONAL authority domains, NOT a linear ordering.
/// No comparison or ordering between variants is implied or valid.
/// Use as categorical tags for enforcement routing decisions only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AuthLayer {
    /// Capability genesis domain. Static, boot-time only. Immutable after lock.
    BootDag = 0,
    /// Runtime authority domain. All runtime access decisions. Never bypassed.
    SexPdx  = 1,
    /// Enforcement acceleration domain. Hardware accel only. Non-authoritative.
    Pku     = 2,
}

/// Userland serial writer — calls kernel raw_print (syscall 0, opcode 69).
pub struct SerialWriter;

mod serial_record_buffer;

/// A complete bounded line uses one raw-print invocation. On the current BSP
/// syscall entry masks interrupts, so timer diagnostics cannot split its fields.
pub fn write_serial_record(arguments: core::fmt::Arguments<'_>) {
    serial_record_buffer::emit(arguments, |line| {
        let _ = core::fmt::Write::write_str(&mut SerialWriter, line);
    });
}

#[macro_export]
macro_rules! serial_record {
    ($($arg:tt)*) => { $crate::write_serial_record(core::format_args!($($arg)*)) };
}

impl core::fmt::Write for SerialWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let _ = pdx_call(0, 69, s.as_ptr() as u64, s.len() as u64, 0);
        Ok(())
    }
}

#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => ({
        use core::fmt::Write as _;
        let mut _w = $crate::SerialWriter;
        let _ = core::write!(_w, $($arg)*);
        let _ = core::write!(_w, "\n");
    });
}

/// PDX message returned by pdx_listen. type_id == 0 means EMPTY (never a valid payload).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PdxMessage {
    pub type_id:   u64,
    pub arg0:      u64,
    pub arg1:      u64,
    pub arg2:      u64,
    pub caller_pd: u32,
    _pad:          u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Window {
    pub id: u32,
    pub pid: u32,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub layer: u8,
    pub buffer_cap: u64, // Slot
}

#[repr(C, u64)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum WindowOp {
    Move(u32, i32, i32),
    Resize(u32, u32, u32),
    Focus(u32),
    Destroy(u32),
    Create(u32, u32, i32, i32, u32, u32, u64),
}

// Legacy window operation constants (used by sexdisplay)
pub const OP_WINDOW_CREATE: u64 = 0xE4;
pub const OP_WINDOW_SUBMIT: u64 = 0xE5;
pub const OP_WINDOW_VBLANK: u64 = 0xE6;
pub const OP_WINDOW_MAP:    u64 = 0xE7;
pub const OP_WINDOW_WRITE:  u64 = 0xE8;

// SilkBar protocol opcodes (reserved; server attaches in v7+)
pub const OP_SILKBAR_PING:     u64 = 0xF0;
pub const OP_SILKBAR_GET_ABI:  u64 = 0xF1;
pub const OP_SILKBAR_UPDATE:           u64 = 0xF2;
pub const OP_SILKBAR_WORKSPACE_ACTIVE: u64 = 0xF3;
pub const OP_SILKBAR_FOCUS_STATE:      u64 = 0xF4;
pub const SILKBAR_ABI_VERSION: u64 = 1;

// Quil proof ping — shell→Quil route verification (QUIL_PROTOCOL_ASSIGN_V1C).
// No display authority. Quil receives and logs, does not draw or create surfaces.
pub const OP_QUIL_PING: u64 = 0xD0;
/// LINEN_DISK_OPEN_V1: shell → quil PD nudge. Opening a disk-backed linen
/// object (the quil doc, /disk/quil-object-v1) tells the real quil PD to
/// restore the document from DiskFS. arg0 = linen object_id (diagnostic).
pub const OP_QUIL_OPEN_DISK_DOC: u64 = 0x4A;

// Bell event protocol opcodes (BELL_SLOT_OPCODE_ASSIGNMENT_V1, namespace audited).
// Range 0xC0-0xC7 assigned; 0xC8-0xCF reserved for Bell future expansion.
// No server spawn, no cap grants, no kernel edits in this phase.
pub const OP_BELL_NOTIFY:      u64 = 0xC0; // App → Bell: request to create a BellEvent
pub const OP_BELL_CLOSE:       u64 = 0xC1; // App/Shell → Bell: dismiss event by ID
pub const OP_BELL_ACTION:      u64 = 0xC2; // App/Shell → Bell: execute action callback
pub const OP_BELL_LIST:        u64 = 0xC3; // Shell → Bell: list current events (summary only)
pub const OP_BELL_CLEAR:       u64 = 0xC4; // Shell → Bell: clear events in a lane or all lanes
pub const OP_BELL_SUBSCRIBE:   u64 = 0xC5; // SilkBar → Bell: subscribe to lane-summary updates
pub const OP_BELL_SET_POLICY:  u64 = 0xC6; // Shell → Bell: set per-app user policy override
pub const OP_BELL_MUTE_SENDER: u64 = 0xC7; // Shell → Bell: mute a sender PD

// OP_BELL_NOTIFY category field reservation (BELL_ATTENTION_FIREWALL_V1).
// category is a u8 packed into arg0 bits [7:0] of OP_BELL_NOTIFY — this is a
// value convention, not a new opcode, so no ABI/opcode collision risk.
// 0=Info, 1..5=other existing categories (see servers/sexbell), 6=reserved
// below. No kernel/ABI edits — Bell enforces this range in its own
// valid_category() check.
pub const BELL_CATEGORY_SELF_CAP_DENIED: u8 = 6; // PD self-reports its own ERR_CAP_INVALID; forced into Bell's SYSTEM lane regardless of urgency_hint (not sender-spoofable into a higher lane)

// Surface tab metadata opcode (silk-shell → sexdisplay)
pub const OP_SURFACE_TAB_INFO: u64 = 0xFD;

// Appearance tokens opcode (silk-shell → sexdisplay, two-call state machine)
// Text draw opcode (app → sexdisplay): draw ASCII text on a surface.
// arg0 = surface_id (u64)
// arg1 = 8 ASCII bytes packed little-endian (u64)
// arg2 = byte_offset (lower 8 bits) | char_count (bits 8-11) | text_color ARGB (upper 32 bits)
pub const OP_TEXT_DRAW: u64 = 0xFB;

// Text clear opcode (app → sexdisplay): clear text on a surface.
// arg0 = surface_id (u64)
pub const OP_TEXT_CLEAR: u64 = 0xFA;

pub const OP_APPEARANCE_TOKENS: u64 = 0xFC;

// Typed input event class constants (IPC encoding for 0x202 OP_HID_EVENT)
pub const EV_KEY: u64 = 1;
pub const EV_REL: u64 = 2;
pub const EV_ABS: u64 = 3;
pub const EV_BTN: u64 = 4;

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ShellEvent {
    pub op: WindowOp,
}

/// Normalized input event. Single source of truth for all input in silk-shell FSM.
///
/// IPC encoding (type_id=0x202 from sexinput):
///   arg2=EV_KEY (1): arg0=scancode, arg1=1(dn)/0(up)
///   arg2=EV_REL (2): arg0=dx(i32),   arg1=dy(i32)   (relative delta)
///   arg2=EV_ABS (3): arg0=abs_x,   arg1=abs_y  (mouse position, absolute)
///   arg2=EV_BTN (4): arg0=button,  arg1=1(dn)/0(up)
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum InputEvent {
    KeyDown(u8),
    KeyUp(u8),
    MouseMove(i32, i32),
    MouseDown(u8),
    MouseUp(u8),
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Layer {
    pub win_id: u32,
    pub rect: [i32; 4], // [x, y, w, h]
    pub buf_ptr: u64,
    pub stride: usize,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct FrameContext {
    pub tick: u64,
    pub snapshot_version: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SceneSnapshot {
    pub layers_ptr: u64,
    pub layers_len: u32,
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub is_incremental: u32,
    pub damage_rects_ptr: u64,
    pub damage_rects_len: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct WindowDescriptor {
    pub window_id: u64,
    pub buffer_handle: u64, // Opaque capability handle
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub z_index: u32,
    pub focus_state: u32,
}

#[repr(C, u64)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MessageType {
    Ping = 0,
    Yield = 1,
    HIDEvent {
        code: u64,
        value: u64,
    } = 0x202,
}

pub struct PdxEvent; // Stub

/// Receive from a capability slot. Self-inbox waits retain wake events across
/// scheduler ownership transfers; other slot kinds remain cooperative polls.
pub fn pdx_listen_raw(slot: u64) -> PdxMessage {
    static mut PDX_LISTEN_WRAPPER_BUDGET: u32 = 8;
    loop {
        let type_id: u64;
        let caller_pd: u64;
        let arg0: u64;
        let arg1: u64;
        let arg2: u64;
        unsafe {
            core::arch::asm!(
                "syscall",
                in("rax") 28u64,
                in("rdi") slot,
                lateout("rax") type_id,    // 0 = EMPTY, non-zero = valid
                lateout("rsi") caller_pd,
                lateout("rdx") arg0,
                lateout("r10") arg1,
                lateout("r8")  arg2,
                out("rcx") _,
                out("r11") _,
            );
        }
        if type_id == 0 {
            if slot == 0 { sys_wait_self(); } else { sys_yield(); }
            continue;
        }
        unsafe {
            if PDX_LISTEN_WRAPPER_BUDGET > 0 {
                PDX_LISTEN_WRAPPER_BUDGET -= 1;
                serial_println!(
                    "[pdx.listen.raw.wrapper] type={:#x} caller={} a0={:#x} a1={:#x}",
                    type_id, caller_pd, arg0, arg1
                );
            }
        }
        return PdxMessage {
            type_id,
            arg0,
            arg1,
            arg2,
            caller_pd: caller_pd as u32,
            _pad: 0,
        };
    }
}

/// Non-blocking listen — calls syscall 28 once, returns None if empty.
pub fn pdx_try_listen_raw(slot: u64) -> Option<PdxMessage> {
    let type_id: u64;
    let caller_pd: u64;
    let arg0: u64;
    let arg1: u64;
    let arg2: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 28u64,
            in("rdi") slot,
            lateout("rax") type_id,
            lateout("rsi") caller_pd,
            lateout("rdx") arg0,
            lateout("r10") arg1,
            lateout("r8")  arg2,
            out("rcx") _,
            out("r11") _,
        );
    }
    if type_id == 0 {
        // Cooperative yield when idle — ensures other PDs get CPU time
        // even when the preemptive LAPIC timer is unavailable.
        sys_yield();
        None
    } else {
        Some(PdxMessage {
            type_id,
            arg0,
            arg1,
            arg2,
            caller_pd: caller_pd as u32,
            _pad: 0,
        })
    }
}

/// Spin-receive from default message ring (Slot 0).
pub fn pdx_listen() -> PdxMessage {
    pdx_listen_raw(0)
}

pub fn pdx_try_listen() -> Option<PdxMessage> {
    pdx_try_listen_raw(0)
}

#[inline(always)]
pub fn pdx_reply(target_pd: u32, value: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 29u64,
            in("rdi") target_pd as u64,
            in("rsi") value,
            out("rcx") _,
            out("r11") _,
            lateout("rax") ret,
            options(nostack),
        );
    }
    ret
}

pub fn sys_yield() {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 32u64,
            out("rcx") _,
            out("r11") _,
        );
    }
}

/// Wait for this domain's own inbox/reply activity without consuming a message.
/// There is no target-domain argument and no authority to wake another task.
/// Spurious/coalesced wakes are allowed: callers recheck their actual input.
pub fn sys_wait_self() {
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 53u64 => _,
            in("rdi") 0u64,
            out("rcx") _,
            out("r11") _,
        );
    }
}

pub const SYSCALL_GET_TICKS: u64 = 34;

#[inline]
pub fn get_ticks() -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") SYSCALL_GET_TICKS => ret,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    ret
}

pub fn sched_yield() {
    sys_yield();
}

pub const SYSCALL_PDX_CALL: u64   = 0;
pub const SYSCALL_PDX_LISTEN: u64 = 2;
pub const SYSCALL_YIELD: u64      = 32;
pub const SYS_SET_STATE: u64      = 42;

pub const SVC_STATE_LISTENING: u64 = 1;

pub unsafe fn sys_set_state(state: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") SYS_SET_STATE => ret,
        in("rdi") state,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

// Capability slots — single source of truth for all protection domains.
// These are capability graph edges, NOT service ports.
pub const SLOT_STORAGE: u64 = 1; // sexfiles VFS
pub const SLOT_SEXT:    u64 = 2; // sext demand pager (fault resolver)
pub const SLOT_INPUT:   u64 = 3; // HID input
pub const SLOT_AUDIO:   u64 = 4; // audio server
pub const SLOT_DISPLAY: u64 = 5; // SexDisplay compositor
pub const SLOT_SHELL:   u64 = 6; // silk-shell orchestration entry
pub const SLOT_SILKBAR: u64 = 7; // SilkBar model authority
pub const SLOT_USB_HOST:  u64 = 8;  // USB host controller lease (XHCI probe path)
pub const SLOT_SEXSTORE: u64 = 10; // sexstore K/V service (slot 9 = kernel-local SLOT_USB_SEXINPUT)
pub const SLOT_QUIL: u64 = 11;    // Quil app surface server (shell→Quil route, no display caps)
pub const SLOT_BELL: u64 = 12;   // Bell attention/event service (domain 10, namespace audited)
pub const SLOT_LINEN: u64 = 13;  // Linen app surface server
pub const SLOT_SPINDLE: u64 = 14; // Spindle command console (domain 12)
pub const SLOT_BLOCK:   u64 = 15; // sexdrive block/DMA service (sexfiles→sexdrive route)
pub const SLOT_BUF_LEND: u64 = 17; // kernel-allocated MemLend buffer cap (sexfiles→sexdrive, Phase A)
pub const SLOT_NET:     u64 = 18; // sexnet network manager route
pub const SLOT_NIC:     u64 = 19; // e1000e NIC hardware capability (sexnet driver)
pub const SLOT_DMA_REGION: u64 = 32;
pub const SLOT_FRAME_REGION: u64 = 33;
pub const SLOT_BLOCK_BUFFER: u64 = 17;

/// Change only explicitly BOOT-sanctioned command bits on the held device.
/// A hardware error may follow a partial mutation: inspect with set=clear=0;
/// never silently retry a failed mutation.
pub fn device_command(slot:u64,set:u64,clear:u64)->Result<pci_command::Reply,u64> {
    let(status,before,after,changed,reserved):(u64,u64,u64,u64,u64);
    unsafe { core::arch::asm!("syscall",
        inlateout("rax") pci_command::SYS_COMMAND=>status,in("rdi")slot,
        inlateout("rsi")set=>before,inlateout("rdx")clear=>after,
        inlateout("r10")0u64=>changed,inlateout("r8")0u64=>reserved,
        out("rcx")_,out("r11")_,options(nostack)); }
    if status!=0 {
        if before|after|changed|reserved!=0{return Err(pci_command::IO)}
        return Err(status)
    }
    if before>u16::MAX as u64||after>u16::MAX as u64||changed>1||reserved!=0{return Err(pci_command::IO)}
    Ok(pci_command::Reply{before:before as u16,after:after as u16,changed:changed!=0})
}

/// Inspect a boot paging capability without allocating or mapping anything.
pub fn paging_info(slot: u64) -> Result<paging::PagingInfo, u64> {
    let (status, base, length, target, rights): (u64, u64, u64, u64, u64);
    unsafe { core::arch::asm!("syscall",
        inlateout("rax") paging::SYS_PAGING_INFO => status, in("rdi") slot,
        lateout("rsi") base, lateout("rdx") length, lateout("r10") target,
        lateout("r8") rights, out("rcx") _, out("r11") _, options(nostack)); }
    if status != paging::PAGING_OK { return Err(status); }
    Ok(paging::PagingInfo { base, length,
        target_pd: u32::try_from(target).map_err(|_| paging::ERR_PAGING_INVALID)?,
        rights: u8::try_from(rights).map_err(|_| paging::ERR_PAGING_INVALID)? })
}

pub fn paging_query(mapping_slot: u64, token: u64) -> Result<paging::FaultQuery, u64> {
    let (status, page, error, target, rights): (u64, u64, u64, u64, u64);
    unsafe { core::arch::asm!("syscall",
        inlateout("rax") paging::SYS_PAGING_QUERY => status, in("rdi") mapping_slot,
        inlateout("rsi") token => page, lateout("rdx") error,
        lateout("r10") target, lateout("r8") rights,
        out("rcx") _, out("r11") _, options(nostack)); }
    if status != paging::PAGING_OK { return Err(status); }
    Ok(paging::FaultQuery { page_va: page, error_code: error,
        target_pd: u32::try_from(target).map_err(|_| paging::ERR_PAGING_INVALID)?,
        allowed_rights: u8::try_from(rights).map_err(|_| paging::ERR_PAGING_INVALID)? })
}

/// RETRY retains custody of the same frame; only exact arguments may be retried.
pub fn paging_commit(mapping_slot: u64, pool_slot: u64, frame_index: u64, token: u64, rights: u8) -> u64 {
    let status: u64;
    unsafe { core::arch::asm!("syscall",
        inlateout("rax") paging::SYS_PAGING_COMMIT => status,
        in("rdi") mapping_slot, inlateout("rsi") pool_slot => _, inlateout("rdx") frame_index => _,
        inlateout("r10") token => _, inlateout("r8") rights as u64 => _,
        out("rcx") _, out("r11") _, options(nostack)); }
    status
}

pub fn paging_reject(mapping_slot: u64, token: u64) -> u64 {
    let status: u64;
    unsafe { core::arch::asm!("syscall",
        inlateout("rax") paging::SYS_PAGING_REJECT => status,
        in("rdi") mapping_slot, inlateout("rsi") token => _,
        lateout("rdx") _, lateout("r10") _, lateout("r8") _,
        out("rcx") _, out("r11") _, options(nostack)); }
    status
}

/// Free backing only after OK: the exact old leaf and its BSP TLB entry are gone.
pub fn paging_reclaim(mapping_slot: u64, pool_slot: u64, frame_index: u64, token: u64) -> u64 {
    let status: u64;
    unsafe { core::arch::asm!("syscall",
        inlateout("rax") paging::SYS_PAGING_RECLAIM => status,
        in("rdi") mapping_slot, inlateout("rsi") pool_slot => _,
        inlateout("rdx") frame_index => _, inlateout("r10") token => _,
        lateout("r8") _, out("rcx") _, out("r11") _, options(nostack)); }
    status
}

/// Permanently close one boot paging range after its main task has exited and
/// released CPU/stack custody. OK covers every target-owned leaf, even an
/// uncertain earlier commit. Apply cleanup to local target ownership once.
pub fn paging_cleanup(mapping_slot: u64, pool_slot: u64) -> Result<paging::CleanupResult, u64> {
    let (status, frame_bitmap, task_id): (u64, u64, u64);
    unsafe { core::arch::asm!("syscall",
        inlateout("rax") paging::SYS_PAGING_CLEANUP => status,
        in("rdi") mapping_slot, inlateout("rsi") pool_slot => frame_bitmap,
        lateout("rdx") task_id, lateout("r10") _, lateout("r8") _,
        out("rcx") _, out("r11") _, options(nostack)); }
    if status != paging::PAGING_OK { return Err(status); }
    Ok(paging::CleanupResult { frame_bitmap, task_id })
}

/// Finish only the invoking task. The kernel releases its CPU custody before
/// authorizing cleanup; this call grants no authority over another task.
pub fn task_finish(reason: u64) -> ! {
    unsafe { core::arch::asm!("syscall",
        inlateout("rax") paging::SYS_TASK_FINISH => _, in("rdi") reason,
        out("rcx") _, out("r11") _, options(nostack)); }
    // Defensive fallback if a broken kernel unexpectedly returns.
    loop { sys_yield(); }
}

pub const SYS_MEMORY_REGION_VIEW: u64 = 54;
pub const SYS_MEMORY_REGION_INFO: u64 = 55;

/// Query immutable boot authority. No frames, mappings or capabilities are created.
pub fn memory_region_info(slot: u64) -> Result<MemoryRegionInfo, u64> {
    let (status, length, rights, peer, region_id): (u64, u64, u64, u64, u64);
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") SYS_MEMORY_REGION_INFO => status, in("rdi") slot,
            lateout("rsi") length, lateout("rdx") rights,
            lateout("r10") peer, lateout("r8") region_id,
            out("rcx") _, out("r11") _, options(nostack));
    }
    if status != 0 { return Err(status); }
    Ok(MemoryRegionInfo { region_id: region_id as u32, length, rights: rights as u8,
        peer_pd: if peer == 0 { None } else { Some(peer as u32) } })
}

pub fn memory_region_view(slot: u64, offset: u64, length: u64, rights: u8) -> Result<memory_region::RegionView, u64> {
    let (status, virtual_address, view_length, physical, region_id): (u64, u64, u64, u64, u64);
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") SYS_MEMORY_REGION_VIEW => status, in("rdi") slot,
            inlateout("rsi") offset => virtual_address,
            inlateout("rdx") length => view_length,
            inlateout("r10") rights as u64 => physical, lateout("r8") region_id,
            out("rcx") _, out("r11") _, options(nostack));
    }
    if status != 0 { return Err(status); }
    Ok(memory_region::RegionView { region_id: region_id as u32, virtual_address,
        length: view_length, dma_address: if physical == 0 { None } else { Some(physical) } })
}

pub fn memory_region_full_view(slot: u64, rights: u8) -> Result<memory_region::RegionView, u64> {
    let info = memory_region_info(slot)?;
    memory_region_view(slot, 0, info.length, rights)
}

/// Resolve only the caller's existing BOOT document Region capability. Numeric
/// subject/slot parameters cannot create a view or upgrade its rights.
pub fn document_snapshot_view(subject: u32, writer: bool) -> Result<document_snapshot::Region, u32> {
    let index = match subject { 9 => 0, 16 => 1, 18 => 2, _ => return Err(document::DENIED) };
    let slot = if writer { document_buffer::PROVIDER_SLOTS[index] } else { document_buffer::CLIENT_SLOT };
    let rights = memory_region::QUERY_VIEW | memory_region::READ | if writer { memory_region::WRITE } else { 0 };
    let info = memory_region_info(slot).map_err(|_| document::DENIED)?;
    let view = memory_region_view(slot, 0, document_buffer::BYTES, rights).map_err(|_| document::DENIED)?;
    // Only immutable current-domain syscall results reach this constructor.
    unsafe { document_snapshot::Region::from_resolved(info, view, subject, writer) }
}

// ── MemLend buffer cap ABI (SEXBLOCK_BUFFER_LEND_CAP_IMPLEMENT_PHASE_A_V1) ──
pub const SYS_MAP_PCI_BAR: u64 = 43;
pub const SYS_GRANT_MEM_LEND: u64 = 50; // rdi=domain_slot rsi=length(4096) rdx=lend_slot → producer_va
pub const SYS_MAP_MEM_LEND:   u64 = 51; // rdi=cap_slot → consumer_va
pub const MEM_LEND_PERM_RW:   u64 = 0x3;

pub fn sys_map_pci_bar(cap_slot: u64, bar_index: u64, map_size: u64) -> u64 {
    let va: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_MAP_PCI_BAR,
            in("rdi") cap_slot,
            in("rsi") bar_index,
            in("rdx") map_size,
            lateout("rax") va,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    va
}

pub fn sys_grant_mem_lend(domain_slot: u64, length: u64, lend_slot: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_GRANT_MEM_LEND,
            in("rdi") domain_slot,
            in("rsi") length,
            in("rdx") lend_slot,
            lateout("rax") ret,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    ret
}

pub fn sys_map_mem_lend(cap_slot: u64) -> u64 {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_MAP_MEM_LEND,
            in("rdi") cap_slot,
            lateout("rax") ret,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    ret
}

// ── Block DMA protocol (SLOT_BLOCK, namespace B0-BF assigned) ──
// Commands: sent as pdx_call opcode (rsi), decoded by sexdrive as msg.type_id.
pub const BLOCK_READ:  u64 = 1; // Read sectors from block device
pub const BLOCK_WRITE: u64 = 2; // Write sectors to block device
pub const BLOCK_SYNC:  u64 = 3; // Flush/barrier (no data transfer)

// Block protocol status codes — returned as pdx_reply value.
pub const BLOCK_OK:           u64 = 0; // Success
pub const BLOCK_ERR_BAD_CMD:  u64 = 1; // Unknown/unsupported command
pub const BLOCK_ERR_BAD_LEN:  u64 = 2; // Transfer size out of bounds
pub const BLOCK_ERR_BAD_CAP:  u64 = 3; // Invalid/missing buffer capability
pub const BLOCK_ERR_NO_DEVICE: u64 = 4; // No real block device backend
pub const BLOCK_ERR_TIMEOUT:  u64 = 5; // Operation timed out

// Block protocol bounds (must match servers/sexfiles/src/backends/diskfs.rs)
pub const BLOCK_SECTOR_SIZE:  u64 = 512;     // Minimum alignment unit
pub const BLOCK_MAX_XFER:     u64 = 4096;    // Max bytes per transfer (one page)

// ═══════════════════════════════════════════════════════════════════════════
// DISK LAYOUT — canonical, single source of truth for every fixed on-disk
// LBA region any component reserves.
//
// Why this exists: DISKFS_V4's content pool was originally placed at LBA
// 128, chosen independently in servers/sexfiles/src/vfs.rs because it was
// the start of the SexFS v0 write-allowed range. apps/sexdrive's own
// boot-time self-test (nvme_multiblock_write_readback_proof) ALSO used LBA
// 128 as its base, chosen independently for the same reason, years
// earlier. Neither side knew about the other's reservation because there
// was no single place both were required to check. The self-test runs
// UNCONDITIONALLY on every boot and silently overwrote real DiskFS content
// there — see docs/handoff/SEXDRIVE_NVME_QUEUE_WRAP_V1.md for the full
// incident writeup (initially misdiagnosed as an NVMe queue-wrap bug
// before the actual LBA collision was found).
//
// Rule going forward: every fixed-LBA region, in any crate, gets declared
// here BEFORE it's used anywhere, and every new region gets a
// `ranges_overlap` assertion added below against every existing one. This
// makes a future collision a build failure, not a silent reboot-time data
// loss discovered by accident.
//
// Units are 512-byte sectors (LBAs) throughout.
pub const DISK_TOTAL_SECTORS: u64 = 2048;

// apps/sexdrive boot-time self-tests. AP3 and AP4 run UNCONDITIONALLY on
// every boot (no gate). AP5A and AP6 are gated behind option_env! flags
// (off in normal builds) but reserved anyway so an intentionally-enabled
// test build can't silently collide with real data either.
pub const SEXDRIVE_AP3_WRITE_PROOF_LBA: u64 = 2047;
pub const SEXDRIVE_AP3_WRITE_PROOF_SECTORS: u64 = 1;
pub const SEXDRIVE_AP4_MULTI_BASE_LBA: u64 = 128;
pub const SEXDRIVE_AP4_MULTI_SECTORS: u64 = 4;
pub const SEXDRIVE_AP5A_PERSIST_BASE_LBA: u64 = 256;
pub const SEXDRIVE_AP5A_PERSIST_SECTORS: u64 = 4;
pub const SEXDRIVE_AP6_NEG_MISMATCH_LBA: u64 = 384;
pub const SEXDRIVE_AP6_NEG_MISMATCH_SECTORS: u64 = 1;

// DiskFS fixed regions (servers/sexfiles/src/vfs.rs and
// servers/sexfiles/src/backends/diskfs.rs).
pub const DISKFS_MANIFEST_LBA: u64 = 2046;
pub const DISKFS_MANIFEST_SECTORS: u64 = 1;
/// Legacy V3 object slots (quil/linen/sexfiles-proof), migrated in place
/// by DISKFS_V4 — 15 slots x 8 sectors, ending just below the manifest.
pub const DISKFS_LEGACY_SLOTS_START_LBA: u64 = 1926;
pub const DISKFS_LEGACY_SLOTS_SECTORS: u64 = 2046 - 1926;
/// The 3 named system-object sub-ranges within the legacy slot region
/// (slots 0-2). apps/sexdrive's write_guard_allows checks these
/// individually rather than as one combined range, so they're declared
/// here too rather than only as the combined region above.
pub const DISKFS_SEXFILES_PROOF_START_LBA: u64 = 2038;
pub const DISKFS_SEXFILES_PROOF_SECTORS: u64 = 8;
pub const DISKFS_LINEN_OBJECT_START_LBA: u64 = 2030;
pub const DISKFS_LINEN_OBJECT_SECTORS: u64 = 8;
pub const DISKFS_QUIL_OBJECT_START_LBA: u64 = 2022;
pub const DISKFS_QUIL_OBJECT_SECTORS: u64 = 8;
/// Per-slot indirect extent descriptors: 15 sectors, one per V4 slot,
/// ending just below the legacy slot region.
pub const DISKFS_V4_INDIRECT_BASE_LBA: u64 = 1911;
pub const DISKFS_V4_INDIRECT_SECTORS: u64 = 15;
/// Variable-length content pool. Must stay inside SEXFS_V0's allowed write
/// envelope below AND clear of every sexdrive self-test region above —
/// including AP5A/AP6, which are gated off in normal builds but still
/// real reservations (a future enabled test build must not collide with
/// real data either). 400 clears AP3/AP4/AP5A/AP6 all with margin.
pub const DISKFS_V4_POOL_BASE_LBA: u64 = 400;
pub const DISKFS_V4_BLOCK_SECTORS: u64 = 8;
pub const DISKFS_V4_POOL_BLOCKS: u64 = 176;
pub const DISKFS_V4_POOL_SECTORS: u64 = DISKFS_V4_POOL_BLOCKS * DISKFS_V4_BLOCK_SECTORS;

// ── DISKFS_V2 canonical reply encoding (LANE3) ──────────────────────────────
// Shared by the server (servers/sexfiles/src/vfs.rs's handle_diskfs_read_v2)
// and every client (quil, spindle) so the bit layout is defined exactly
// once instead of hand-rolled per caller. See OP_DISKFS_READ_V2's doc
// comment in servers/sexfiles/src/messages.rs for the full rationale:
// status confined to the top byte, payload to the low 6 bytes, so a
// payload byte can never be misread as a status/error the way
// OP_DISKFS_READ's full-8-byte reply could.
pub const DISKFS_V2_STATUS_OK: u64 = 0x00;
pub const DISKFS_V2_STATUS_EOF: u64 = 0x01;
pub const DISKFS_V2_STATUS_ERR: u64 = 0xFF;
pub const DISKFS_V2_MAX_READ: usize = 6;

/// Server side: encode a successful read of `payload[..n]` (n <= DISKFS_V2_MAX_READ).
pub fn diskfs_v2_encode_ok(n: usize, payload: &[u8]) -> u64 {
    let n = n.min(DISKFS_V2_MAX_READ);
    let mut word: u64 = 0;
    for i in 0..n { word |= (payload[i] as u64) << (i * 8); }
    (DISKFS_V2_STATUS_OK << 56) | ((n as u64 & 0xFF) << 48) | (word & 0xFFFF_FFFF_FFFF)
}

/// Server side: encode explicit EOF (offset == object length; zero bytes
/// available, not an error).
pub fn diskfs_v2_encode_eof() -> u64 {
    DISKFS_V2_STATUS_EOF << 56
}

/// Server side: encode a real error. `magnitude` is the ERR_* constant's
/// absolute value (e.g. 4 for ERR_OVERFLOW = -4), recovered by the caller
/// from whatever u64-encoded-negative-i64 value the rest of the codebase's
/// error plumbing already produced.
pub fn diskfs_v2_encode_err(magnitude: u64) -> u64 {
    (DISKFS_V2_STATUS_ERR << 56) | ((magnitude & 0xFF) << 48)
}

/// Client side: extract the status byte (compare against the
/// DISKFS_V2_STATUS_* constants above).
pub fn diskfs_v2_status(reply: u64) -> u64 { (reply >> 56) & 0xFF }

/// Client side: extract the length/magnitude field (bytes actually read
/// when status==OK; the ERR_* magnitude when status==ERR; unused/0 on EOF).
pub fn diskfs_v2_len_field(reply: u64) -> u64 { (reply >> 48) & 0xFF }

/// Client side: extract up to DISKFS_V2_MAX_READ payload bytes, LE. Only
/// meaningful when status==OK; call diskfs_v2_len_field() first for the
/// actual valid count.
pub fn diskfs_v2_payload(reply: u64) -> [u8; DISKFS_V2_MAX_READ] {
    let bytes = (reply & 0xFFFF_FFFF_FFFF).to_le_bytes();
    let mut out = [0u8; DISKFS_V2_MAX_READ];
    out.copy_from_slice(&bytes[..DISKFS_V2_MAX_READ]);
    out
}

/// Two crash-consistent private-history banks, each header + 8 payload sectors.
pub const SPINDLE_HISTORY_BASE_LBA:u64=1808;
pub const SPINDLE_HISTORY_SECTORS:u64=18;
const _:()={
 let regions=[(DISKFS_V4_POOL_BASE_LBA,DISKFS_V4_POOL_SECTORS),(DISKFS_V4_INDIRECT_BASE_LBA,DISKFS_V4_INDIRECT_SECTORS),(DISKFS_LEGACY_SLOTS_START_LBA,DISKFS_LEGACY_SLOTS_SECTORS),(DISKFS_MANIFEST_LBA,DISKFS_MANIFEST_SECTORS),(SEXDRIVE_AP3_WRITE_PROOF_LBA,SEXDRIVE_AP3_WRITE_PROOF_SECTORS),(SEXDRIVE_AP4_MULTI_BASE_LBA,SEXDRIVE_AP4_MULTI_SECTORS),(SEXDRIVE_AP5A_PERSIST_BASE_LBA,SEXDRIVE_AP5A_PERSIST_SECTORS),(SEXDRIVE_AP6_NEG_MISMATCH_LBA,SEXDRIVE_AP6_NEG_MISMATCH_SECTORS)];
 let mut i=0;while i<regions.len(){assert!(!ranges_overlap(SPINDLE_HISTORY_BASE_LBA,SPINDLE_HISTORY_SECTORS,regions[i].0,regions[i].1));i+=1;}
 assert!(SPINDLE_HISTORY_BASE_LBA>=128&&SPINDLE_HISTORY_BASE_LBA+SPINDLE_HISTORY_SECTORS<=2020);
};

// SexFS v0 allowed write envelope (apps/sexdrive write_guard_allows) — not
// itself a reservation, the outer bound DiskFS's pool must stay inside.
pub const SEXFS_V0_META_START_LBA: u64 = 0;
pub const SEXFS_V0_META_END_LBA: u64 = 47;
pub const SEXFS_V0_OBJECT_START_LBA: u64 = 128;
pub const SEXFS_V0_OBJECT_END_LBA: u64 = 2019;

/// Also used at runtime (not just in this file's compile-time asserts) by
/// servers/sexfiles for a boot-time confirmation log line — see
/// v4_ensure's disk-layout self-check.
pub const fn ranges_overlap(a_start: u64, a_len: u64, b_start: u64, b_len: u64) -> bool {
    a_start < b_start + b_len && b_start < a_start + a_len
}

// Compile-time non-overlap guarantees. Add a new assert here for every
// new fixed region introduced anywhere in the system, checked against
// every region that already exists — this is the one place a collision
// gets caught before boot instead of after a reboot silently eats data.
const _: () = assert!(
    !ranges_overlap(DISKFS_V4_POOL_BASE_LBA, DISKFS_V4_POOL_SECTORS, SEXDRIVE_AP3_WRITE_PROOF_LBA, SEXDRIVE_AP3_WRITE_PROOF_SECTORS),
    "DISKFS_V4 content pool overlaps sexdrive AP3 self-test LBA"
);
const _: () = assert!(
    !ranges_overlap(DISKFS_V4_POOL_BASE_LBA, DISKFS_V4_POOL_SECTORS, SEXDRIVE_AP4_MULTI_BASE_LBA, SEXDRIVE_AP4_MULTI_SECTORS),
    "DISKFS_V4 content pool overlaps sexdrive AP4 self-test LBA range"
);
const _: () = assert!(
    !ranges_overlap(DISKFS_V4_POOL_BASE_LBA, DISKFS_V4_POOL_SECTORS, SEXDRIVE_AP5A_PERSIST_BASE_LBA, SEXDRIVE_AP5A_PERSIST_SECTORS),
    "DISKFS_V4 content pool overlaps sexdrive AP5A self-test LBA range"
);
const _: () = assert!(
    !ranges_overlap(DISKFS_V4_POOL_BASE_LBA, DISKFS_V4_POOL_SECTORS, SEXDRIVE_AP6_NEG_MISMATCH_LBA, SEXDRIVE_AP6_NEG_MISMATCH_SECTORS),
    "DISKFS_V4 content pool overlaps sexdrive AP6 self-test LBA"
);
const _: () = assert!(
    !ranges_overlap(DISKFS_V4_POOL_BASE_LBA, DISKFS_V4_POOL_SECTORS, DISKFS_V4_INDIRECT_BASE_LBA, DISKFS_V4_INDIRECT_SECTORS),
    "DISKFS_V4 content pool overlaps its own indirect-descriptor region"
);
const _: () = assert!(
    !ranges_overlap(DISKFS_V4_POOL_BASE_LBA, DISKFS_V4_POOL_SECTORS, DISKFS_LEGACY_SLOTS_START_LBA, DISKFS_LEGACY_SLOTS_SECTORS),
    "DISKFS_V4 content pool overlaps the legacy V3 slot region"
);
const _: () = assert!(
    !ranges_overlap(DISKFS_V4_POOL_BASE_LBA, DISKFS_V4_POOL_SECTORS, DISKFS_MANIFEST_LBA, DISKFS_MANIFEST_SECTORS),
    "DISKFS_V4 content pool overlaps the manifest sector"
);
const _: () = assert!(
    !ranges_overlap(DISKFS_V4_INDIRECT_BASE_LBA, DISKFS_V4_INDIRECT_SECTORS, DISKFS_LEGACY_SLOTS_START_LBA, DISKFS_LEGACY_SLOTS_SECTORS),
    "DiskFS indirect-descriptor region overlaps the legacy V3 slot region"
);
const _: () = assert!(
    DISKFS_V4_POOL_BASE_LBA >= SEXFS_V0_OBJECT_START_LBA
        && DISKFS_V4_POOL_BASE_LBA + DISKFS_V4_POOL_SECTORS <= SEXFS_V0_OBJECT_END_LBA + 1,
    "DISKFS_V4 content pool must stay entirely inside the sexdrive-allowed SexFS v0 write range"
);
const _: () = assert!(
    DISKFS_V4_POOL_BASE_LBA + DISKFS_V4_POOL_SECTORS <= DISK_TOTAL_SECTORS,
    "DISKFS_V4 content pool runs past the end of the disk image"
);

// Capability invocation trap numbers (ring-3 → ring-0 transition only).
// These are sex-pdx implementation details, NOT POSIX-style syscall numbers.
pub const SYSCALL_PDX_REPLY: u64 = 1;

// Capability error sentinels — must match kernel/src/ipc.rs exactly.
// Returned when sex-pdx rejects an invocation.
pub const ERR_SERVICE_NOT_READY: u64 = 0xFFFF_FFFF_FFFF_FFFE;
pub const ERR_CAP_INVALID:       u64 = 0xFFFF_FFFF_FFFF_FFFC;

pub fn pdx_call(slot: u64, opcode: u64, arg0: u64, arg1: u64, arg2: u64) -> (u64, u64) {
    let status: u64;
    let value: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 0u64,
            in("rdi") slot,
            in("rsi") opcode,
            in("rdx") arg0,
            in("r10") arg1,
            in("r8")  arg2,
            lateout("rax") status,
            lateout("rsi") value,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    (status, value)
}

#[inline]
pub fn pdx_call_checked(slot: u64, opcode: u64, arg0: u64, arg1: u64, arg2: u64) -> Result<u64, u64> {
    let (status, value) = pdx_call(slot, opcode, arg0, arg1, arg2);
    if status == 0 {
        Ok(value)
    } else {
        Err(status)
    }
}

/// Spawn a new protection domain from an ELF image.
///
/// BOOT_DAG-gated: only domains with an explicit spawn capability granted by
/// BOOT_DAG at boot time may call this. Without the capability → ERR_CAP_INVALID.
/// Calling before BOOT_DAG lock is undefined behavior in the capability model.
pub fn pdx_spawn_pd(elf_addr: u64, elf_len: u64) -> Result<DomainId, u64> {
    let (status, value) = pdx_call(0, 0x10, elf_addr, elf_len, 0);
    if status == 0 {
        Ok(DomainId(value as u32))
    } else {
        Err(status)
    }
}

pub mod document;
pub mod document_buffer;
pub mod document_snapshot;
pub mod pci_command;

pub mod desktop_style;
pub mod document_view;

pub mod display_boot;

pub mod text_batch;
