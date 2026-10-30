/// IPCtax-compliant Message structures for Driver/VFS interaction.
/// 100% Zero-Copy via Lent Capabilities.
extern crate alloc;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FsArgs {
    pub command: u32,
    pub offset: u64,
    pub size: u64,
    pub buffer: u64, // Virtual address of lent-memory buffer
}

pub const FS_READ: u32 = 1;
pub const FS_WRITE: u32 = 2;
pub const FS_LOOKUP: u32 = 3;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub ev_type: u16,
    pub code: u16,
    pub value: i32,
}

pub const EV_KEY: u16 = 1;
pub const EV_ABS: u16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    InvalidTarget,
    BufferFull,
    InvalidCapability,
}

#[derive(Debug, Clone, Copy)]
pub struct IpcReply {
    pub value: u64,
    /// Captured from the current protection domain, never supplied by userland.
    pub caller_pd: u32,
    /// Actual scheduled sender task, stamped by the kernel, not request data.
    pub caller_task: u32,
}
impl IpcReply{
    fn scrub(&mut self){unsafe{
        core::ptr::write_volatile(&mut self.value,0);
        core::ptr::write_volatile(&mut self.caller_pd,0);
        core::ptr::write_volatile(&mut self.caller_task,0);
    }}
    /// Return one authenticated value and erase its queue backing before the
    /// deque releases that slot. A logical pop alone retains old secret bits.
    pub fn take(queue:&mut self::alloc::collections::VecDeque<Self>)->Option<Self>{
        let item=*queue.front()?;queue.front_mut().unwrap().scrub();let _=queue.pop_front();Some(item)
    }
    pub fn erase_queue(queue:&mut self::alloc::collections::VecDeque<Self>){
        for item in queue.iter_mut(){item.scrub();}queue.clear();
    }
}

#[cfg(test)]mod reply_erasure_tests{
    use super::IpcReply;
    #[test]fn explicit_erasure_zeros_both_payload_and_sender(){let mut reply=IpcReply{value:u64::MAX,caller_pd:13,caller_task:901};reply.scrub();assert_eq!((reply.value,reply.caller_pd,reply.caller_task),(0,0,0));}
    #[test]fn take_preserves_exact_authenticated_value_and_erases_before_next_publication(){
        let mut queue=super::alloc::collections::VecDeque::new();queue.push_back(IpcReply{value:123,caller_pd:13,caller_task:901});
        let got=IpcReply::take(&mut queue).unwrap();assert_eq!((got.value,got.caller_pd,got.caller_task),(123,13,901));assert!(IpcReply::take(&mut queue).is_none());
        for sender in [14,15]{queue.push_back(IpcReply{value:sender as u64,caller_pd:sender,caller_task:sender+100});let got=IpcReply::take(&mut queue).unwrap();assert_eq!((got.value,got.caller_pd,got.caller_task),(sender as u64,sender,sender+100));}
        queue.push_back(got);IpcReply::erase_queue(&mut queue);assert!(queue.is_empty());
    }
}

/// Ring buffer message for Storage Completion.
#[repr(C)]
pub struct StorageCompletion {
    pub id: u64,
    pub status: i32,
}

/// Message types for XIPC Communication (PDX ring-buffer protocol).
#[derive(Debug, Clone, Copy)]
pub enum MessageType {
    Empty,
    Signal(u8),
    IpcCall { func_id: u64, arg0: u64, arg1: u64, arg2: u64, caller_pd: u32, caller_task: u32 },
    IpcReply(u64),
    PageFault { fault_addr: u64, error_code: u32, pd_id: u64, lent_cap: u64 },
    Spawn { path_ptr: u64 },
    VfsCall { command: u32, offset: u64, size: u64, buffer_cap: u32 },
    VfsReply { status: i64, size: u64 },
    NetCall { command: u32, socket_cap: u32, offset: u64, size: u64, buffer_cap: u32, remote_node: u32 },
    NetReply { status: i64, size: u64, socket_cap: u32 },
    DmaCall { command: u32, offset: u64, size: u64, buffer_cap: u32, device_cap: u32 },
    DmaReply { status: i64, size: u64 },
    GpuCall { command: u32, buffer_cap: u32, width: u32, height: u32 },
    GpuReply { status: i64 },
    HIDEvent { ev_type: u16, code: u16, value: u16 },
    PipeCall { command: u32, pipe_cap: u32, buffer_cap: u32, size: u64 },
    PipeReply { status: i64, size: u64, pipe_cap: u32 },
    ProcCall { command: u32, path_ptr: u64, arg_ptr: u64 },
    ProcReply { status: i64, pd_id: u32 },
    TranslatorCall { command: u32, path_ptr: u64, code_cap: u32 },
    TranslatorReply { status: i64, translated_entry: u64 },
    StoreCall { command: u32, package_name_ptr: u64, buffer_cap: u32 },
    StoreReply { status: i64, size: u64 },
    HardwareInterrupt { vector: u8, data: u64 },
    RawInput(u8),
    DriverLoadCall { command: u32, driver_name_ptr: u64 },
    DriverLoadReply { status: i64, driver_pd_id: u32 },

    // Display Server Protocol (Phase 16: PDX Display)
    DisplayBufferAlloc { width: u32, height: u32, format: u32 },
    DisplayBufferCommit { buffer_id: u32, damage_x: u32, damage_y: u32, damage_w: u32, damage_h: u32 },
    DisplayBufferReply { page_count: u32, pfn_list: [u64; 64], pku_key: u8 },
    DisplayModeset { width: u32, height: u32, refresh: u32 },
    DisplayCursor { x: i32, y: i32, visible: bool, buffer_id: u32 },
    DisplayGeminiRepairDisplay,
    DisplayPrimaryFramebuffer { virt_addr: u64, width: u32, height: u32, pitch: u32 },

    // Phase 25: Display Opcodes as variants
    WindowCreate,
    CompositorCommit,
    SetWindowRoundness,
    SetWindowBlur,
    SetWindowAnimation,
    SetWindowDecorations,
    GetDisplayInfo,
    FocusWindow,
    MoveWindow,
    ResizeWindow,
}

/// Compatibility alias — the rest of the kernel (ipc.rs / core_local.rs / scheduler.rs) still references `Message`
pub type Message = MessageType;

/// Shim for legacy call sites that expect .msg_type() on the old wrapper struct
/// (keeps scheduler.rs:252 and any other match sites working without further changes)
impl MessageType {
    #[inline(always)]
    pub fn msg_type(&self) -> MessageType {
        *self
    }
}
