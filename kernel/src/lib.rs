#![no_std]
#![cfg_attr(target_arch = "x86_64", feature(abi_x86_interrupt))]

#[cfg(target_arch = "x86_64")]
extern crate alloc;

// ── x86_64-only modules (Phase 2 cfg-gating: AArch64 builds exclude these;
// x86 build is byte-for-byte the same module set as before) ─────────────────
#[cfg(target_arch = "x86_64")]
pub mod serial;
#[cfg(target_arch = "x86_64")]
pub mod vga;
#[cfg(target_arch = "x86_64")]
pub mod memory;
#[cfg(target_arch = "x86_64")]
pub mod hal;
#[cfg(target_arch = "x86_64")]
pub mod pku;
#[cfg(target_arch = "x86_64")]
pub mod slab;
#[cfg(target_arch = "x86_64")]
pub mod cheri;
#[cfg(target_arch = "x86_64")]
pub mod capability;
#[cfg(target_arch = "x86_64")]
pub mod capability_slot;
#[cfg(target_arch = "x86_64")]
pub mod boot_memory;
pub mod boot_display_state;
pub mod boot_display;
#[cfg(target_arch = "x86_64")]
pub mod paging;
#[cfg(target_arch = "x86_64")]
pub mod paging_state;
#[cfg(target_arch = "x86_64")]
pub mod app_resource_state;
#[cfg(target_arch = "x86_64")]
pub mod app_resources;
pub mod history_recovery;
#[cfg(target_arch = "x86_64")]
pub(crate) mod pci_bar_bounds;
#[cfg(target_arch = "x86_64")]
pub(crate) mod boot_frame_reservations;
#[cfg(target_arch = "x86_64")]
pub mod ucgm;
#[cfg(target_arch = "x86_64")]
pub mod ipc;
#[cfg(target_arch = "x86_64")]
pub mod ipc_ring;
#[cfg(target_arch = "x86_64")]
pub mod gdt;
#[cfg(target_arch = "x86_64")]
pub mod interrupts;
#[cfg(target_arch = "x86_64")]
pub mod amdahl;
#[cfg(target_arch = "x86_64")]
pub mod sunni;
// pub mod throughput_test;
#[cfg(target_arch = "x86_64")]
pub mod latency_guard;
#[cfg(target_arch = "x86_64")]
pub mod core_local;
#[cfg(target_arch = "x86_64")]
pub mod apic;
#[cfg(target_arch = "x86_64")]
pub mod smp;
#[cfg(target_arch = "x86_64")]
pub mod scheduler;
#[cfg(target_arch = "x86_64")]
pub mod elf;
#[cfg(target_arch = "x86_64")]
pub mod initrd;
#[cfg(target_arch = "x86_64")]
pub mod loader;
#[cfg(target_arch = "x86_64")]
pub mod drivers;
#[cfg(target_arch = "x86_64")]
pub mod pd;
#[cfg(target_arch = "x86_64")]
pub mod syscalls;
#[cfg(target_arch = "x86_64")]
pub mod capabilities;
#[cfg(target_arch = "x86_64")]
pub mod benchmark;
#[cfg(target_arch = "x86_64")]
pub mod init;
#[cfg(target_arch = "x86_64")]
pub mod graphics;
#[cfg(target_arch = "x86_64")]
pub mod devmgr;
#[cfg(target_arch = "x86_64")]
pub mod hw;
#[cfg(target_arch = "x86_64")]
pub mod keyboard;

// Phase 1 compile-time architecture seam + dead-code semantic protection
// types. Behavior-neutral; no call path changed.
// See Sex_ArmV9/docs/ARM_S1POE_PORT_PLAN_V1.md §D.
pub mod arch;
pub mod protection;

pub const MAP_MEMORY_SYSCALL_NUM: u64 = 30;
pub const ALLOCATE_MEMORY_SYSCALL_NUM: u64 = 31;

#[cfg(target_arch = "x86_64")]
use linked_list_allocator::LockedHeap;

#[cfg(target_arch = "x86_64")]
pub use crate::memory::allocator;

#[cfg(target_arch = "x86_64")]
#[global_allocator]
pub static ALLOCATOR: LockedHeap = LockedHeap::empty();

#[cfg(target_arch = "x86_64")]
pub const HEAP_START: usize = 0x_4444_4444_0000;
#[cfg(target_arch = "x86_64")]
pub const HEAP_SIZE: usize = 256 * 1024 * 1024; // 256 MiB

#[cfg(target_arch = "x86_64")]
use limine::{BaseRevision, RequestsStartMarker, RequestsEndMarker, request::{FramebufferRequest, HhdmRequest, MemmapRequest, ModulesRequest, RsdpRequest, StackSizeRequest}};

// Limine 7.x protocol: start marker, base revision, all requests, end marker — in order
#[cfg(target_arch = "x86_64")]
#[used]
#[link_section = ".limine_requests_start"]
static _LIMINE_START: RequestsStartMarker = RequestsStartMarker::new();

#[cfg(target_arch = "x86_64")]
#[used]
#[link_section = ".limine_requests"]
static _LIMINE_BASE: BaseRevision = BaseRevision::new();

#[cfg(target_arch = "x86_64")]
#[used]
#[link_section = ".limine_requests"]
pub static FB_REQUEST: FramebufferRequest = FramebufferRequest::new();
#[cfg(target_arch = "x86_64")]
#[used]
#[link_section = ".limine_requests"]
pub static MEMMAP_REQUEST: MemmapRequest = MemmapRequest::new();
#[cfg(target_arch = "x86_64")]
#[used]
#[link_section = ".limine_requests"]
pub static HHDM_REQUEST: HhdmRequest = HhdmRequest::new();
#[cfg(target_arch = "x86_64")]
#[used]
#[link_section = ".limine_requests"]
pub static RSDP_REQUEST: RsdpRequest = RsdpRequest::new();
#[cfg(target_arch = "x86_64")]
#[used]
#[link_section = ".limine_requests"]
pub static MODULE_REQUEST: ModulesRequest = ModulesRequest::new();

#[cfg(target_arch = "x86_64")]
#[used]
#[link_section = ".limine_requests"]
static _STACK_SIZE: StackSizeRequest = StackSizeRequest::new(1024 * 1024); // 1 MiB

#[cfg(target_arch = "x86_64")]
#[used]
#[link_section = ".limine_requests_end"]
static _LIMINE_END: RequestsEndMarker = RequestsEndMarker::new();

#[cfg(target_arch = "x86_64")]
pub fn kernel_init() {
    // 1. HAL first: GDT + IDT + CR4.PKE — must precede any exception-prone code
    hal::init();

    // 2. Memory: heap + GLOBAL_VAS — must precede any Box/alloc
    let mmap = MEMMAP_REQUEST.response().unwrap_or_else(|| {
        serial_println!("FATAL: Limine memmap missing");
        loop { core::hint::spin_loop(); }
    });
    let hhdm = HHDM_REQUEST.response().unwrap_or_else(|| {
        serial_println!("FATAL: Limine HHDM missing");
        loop { core::hint::spin_loop(); }
    });
    memory::manager::init(mmap, hhdm.offset);

    // 2.5 Initialize CoreLocal for BSP (needs heap)
    unsafe { crate::core_local::CoreLocal::init(0); }
    serial_println!("kernel: CoreLocal initialized for BSP");

    // 3. Advanced Hardware (APIC + Timer)
    let rsdp_res = RSDP_REQUEST.response();
    serial_println!("kernel: RSDP Response present: {}", rsdp_res.is_some());
    let rsdp_virt = rsdp_res.map(|r| r.address as usize).unwrap_or(0);
    serial_println!("kernel: RSDP Virtual Address: {:#x}", rsdp_virt);
    
    // RSDP from Limine is virtual; convert to physical for ACPI handler
    let rsdp_phys = if rsdp_virt >= hhdm.offset as usize {
        rsdp_virt - hhdm.offset as usize
    } else {
        rsdp_virt // Assume already physical if below HHDM
    };
    serial_println!("kernel: RSDP Physical Address: {:#x}", rsdp_phys);

    hal::init_advanced(rsdp_phys as u64, hhdm.offset);

    // Initialize PS/2 Keyboard
    keyboard::init();

    // 4. PD bootstrap (PKU enabled, heap ready)
    init::init();

    // 5. Start Scheduler (Phase 21: Preemptive Multi-tasking)
    // APIC calibration leaves interrupts enabled; disable here so the LAPIC timer
    // cannot fire before init_first_core sets INITIALIZED=true.
    x86_64::instructions::interrupts::disable();
    serial_println!("scheduler.bind.before");
    let bind_id = unsafe { crate::init::SEXDISPLAY_PD_ID };
    serial_println!("scheduler.bind.target_pd_id={}", bind_id);
    unsafe {
        let first_pd = crate::ipc::DOMAIN_REGISTRY.get(crate::init::SEXDISPLAY_PD_ID).expect("BSP Bind: sexdisplay missing");
        crate::core_local::CoreLocal::init_first_core(first_pd as *const _ as *mut _);
    }
    serial_println!("scheduler.bind.after");
    serial_println!(
        "interrupts.enable.before if={}",
        x86_64::instructions::interrupts::are_enabled()
    );

    // Temporary debug fallback: prove scheduler/context-switch path even if timer IRQ never fires.
    // Remove after timer delivery is verified.
    {
        let core_id = crate::core_local::CoreLocal::get().core_id as usize;
        let sched = &crate::scheduler::SCHEDULERS[core_id];
        match sched.tick() {
            crate::scheduler::ScheduleDecision::Switch { old: _old_ctx_ptr, next: next_ctx_ptr } => {
                unsafe {
                    let kstack_top = (*next_ctx_ptr).kstack_top;
                    // TaskContext.kstack_top points at saved-frame base (rax slot),
                    // not empty kernel stack top. RSP0 must use empty top to avoid
                    // clobbering saved IRET/GPR frame on first user->kernel entry.
                    crate::gdt::update_tss_rsp0(x86_64::VirtAddr::new(kstack_top + 168));
                    crate::scheduler::log_first_scheduled_pd((*next_ctx_ptr).pd_id);
                    serial_println!(
                        "context_switch.before_switch_to rip={:#x} rsp={:#x} rflags={:#x} cs={:#x} ss={:#x} pd_id={}",
                        (*next_ctx_ptr).rip,
                        (*next_ctx_ptr).rsp,
                        (*next_ctx_ptr).rflags,
                        (*next_ctx_ptr).cs,
                        (*next_ctx_ptr).ss,
                        (*next_ctx_ptr).pd_id
                    );
                    serial_println!(
                        "switch.frame rip={:#x} cs={:#x} ss={:#x} rsp={:#x} rflags={:#x}",
                        (*next_ctx_ptr).rip,
                        (*next_ctx_ptr).cs,
                        (*next_ctx_ptr).ss,
                        (*next_ctx_ptr).rsp,
                        (*next_ctx_ptr).rflags
                    );
                    crate::scheduler::debug_dump_iret_frame(next_ctx_ptr);
                    crate::scheduler::debug_dump_user_entry_bytes(next_ctx_ptr);
                    crate::gdt::debug_dump_user_selectors();
                    serial_println!("tss.rsp0={:#x}", crate::gdt::debug_tss_rsp0());
                    serial_println!("context_switch.begin");
                    crate::scheduler::Scheduler::switch_to(core::ptr::null_mut(), next_ctx_ptr);
                }
            }
            crate::scheduler::ScheduleDecision::NoRunnable => {
                serial_println!("scheduler.debug_kick: no runnable task");
            }
        }
    }
    serial_println!("kernel: Enabling interrupts and entering scheduler loop...");
    x86_64::instructions::interrupts::enable();
    serial_println!(
        "interrupts.enable.after if={}",
        x86_64::instructions::interrupts::are_enabled()
    );

    // Infinite spin loop. The timer interrupt will trigger sched.tick()
    // which will perform the first switch_to() to a userland task.
    loop {
        core::hint::spin_loop();
    }
}

#[no_mangle]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    let mut len = 0;
    while *s.add(len) != 0 {
        len += 1;
    }
    len
}

#[no_mangle]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    if dest < src as *mut u8 {
        memcpy(dest, src, n)
    } else if dest > src as *mut u8 {
        let mut i = n;
        while i > 0 {
            i -= 1;
            *dest.add(i) = *src.add(i);
        }
        dest
    } else {
        dest
    }
}

#[no_mangle]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *dest.add(i) = *src.add(i);
        i += 1;
    }
    dest
}

#[no_mangle]
pub unsafe extern "C" fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8 {
    let mut i = 0;
    while i < n {
        *s.add(i) = c as u8;
        i += 1;
    }
    s
}

#[no_mangle]
pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    let mut i = 0;
    while i < n {
        let a = *s1.add(i);
        let b = *s2.add(i);
        if a != b {
            return a as i32 - b as i32;
        }
        i += 1;
    }
    0
}

#[cfg(target_arch = "x86_64")]
pub fn bootstrap_advanced_services() {
    serial_println!("SexOS: Advanced interaction suite initialized via standalone PDs.");
}

#[cfg(test)]
use core::panic::PanicInfo;

#[cfg(test)]
pub fn test_runner(tests: &[&dyn Fn()]) {
    serial_println!("Running {} tests", tests.len());
    for test in tests {
        test();
    }
    exit_qemu(QemuExitCode::Success);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum QemuExitCode {
    Success = 0x10,
    Failed = 0x11,
}

#[cfg(test)]
pub fn exit_qemu(exit_code: QemuExitCode) {
    use x86_64::instructions::port::Port;

    unsafe {
        let mut port = Port::new(0xf4);
        port.write(exit_code as u32);
    }
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("TEST FAILED: {}", info);
    exit_qemu(QemuExitCode::Failed);
    loop {}
}
