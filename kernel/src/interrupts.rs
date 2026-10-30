use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame};
use crate::serial_println;
use crate::gdt;
use crate::ipc_ring::RingBuffer;
use lazy_static::lazy_static;

use core::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use crate::ipc::messages::MessageType;
static TIMER_TICK_LOG_BUDGET: AtomicU64 = AtomicU64::new(64);
static PS2_IRQ1_LOG_COUNT: AtomicU32 = AtomicU32::new(0);
static PS2_PORT60_LOG_COUNT: AtomicU32 = AtomicU32::new(0);
static PS2_ENQUEUE_LOG_COUNT: AtomicU32 = AtomicU32::new(0);
static PS2_DROP_LOG_COUNT: AtomicU32 = AtomicU32::new(0);
static PS2_EOI_LOG_COUNT: AtomicU32 = AtomicU32::new(0);

/// The event structure for a page fault.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct PageFaultEvent {
    pub addr: u64,
    pub error_code: u64,
    pub task_id: u32,
}

lazy_static! {
    /// Global Uptime Ticks (incremented by LAPIC timer).
    pub static ref TICKS: AtomicU64 = AtomicU64::new(0);

    /// The global asynchronous queue for Page Faults (sext pager).
    pub static ref SEXT_QUEUE: RingBuffer<PageFaultEvent, 256> = RingBuffer::new();

    /// Global lock-free input ring buffer for Ring-3 consumption
    pub static ref INPUT_RING: RingBuffer<u8, 512> = RingBuffer::new();
}

static VECTOR_OWNERS: [AtomicU32; 256] = [const { AtomicU32::new(0) }; 256];

pub fn register_irq_route(vector: u8, pd_id: u32) {
    VECTOR_OWNERS[vector as usize].store(pd_id, Ordering::Release);
    serial_println!("IRQ: Registered Vector {:#x} to PD {}", vector, pd_id);
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        idt.invalid_opcode.set_handler_fn(invalid_opcode_handler);
        unsafe {
            idt.double_fault.set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX as u16 + 1);
            
            // critical stubs (naked)
            idt.page_fault.set_handler_addr(x86_64::VirtAddr::new(page_fault_stub as *const () as u64));
            idt.general_protection_fault.set_handler_addr(x86_64::VirtAddr::new(general_protection_fault_stub as *const () as u64));
            idt[0x20].set_handler_addr(x86_64::VirtAddr::new(timer_interrupt_stub as *const () as u64));
        }

        idt[0x40].set_handler_fn(revoke_key_handler);
        idt[0x21].set_handler_fn(keyboard_interrupt_handler);
        idt[0x22].set_handler_fn(generic_irq_handler); 
        idt
    };
}

use x86_64::registers::model_specific::{LStar, SFMask, Efer, EferFlags};
use x86_64::VirtAddr;

pub fn init_idt() {
    serial_println!("   → Loading IDTR...");
    IDT.load();
    serial_println!("   → IDTR loaded successfully");

    unsafe {
        serial_println!("   → Setting up LSTAR (Syscall Entry)...");
        LStar::write(VirtAddr::new(syscall_entry as *const () as u64));
        serial_println!("   → Setting up STAR...");
        let selectors = crate::gdt::get_selectors();

        use x86_64::registers::model_specific::Star;

        // Intel/AMD Contract: STAR[47:32] = SYSCALL CS/SS, STAR[63:48] = SYSRET CS/SS
        Star::write(
            selectors.user_cs,
            selectors.user_ss,
            selectors.kernel_cs,
            selectors.kernel_ss,
        ).expect("Failed to write STAR MSR");

        serial_println!("→ STAR MSR locked");
        // Enter Rust only after the GS/RSP transition is protected. The original
        // user flags remain in R11 and are restored by SYSRET/the forged IRET.
        use x86_64::registers::rflags::RFlags;
        SFMask::write(RFlags::INTERRUPT_FLAG | RFlags::DIRECTION_FLAG
            | RFlags::TRAP_FLAG | RFlags::ALIGNMENT_CHECK);
        Efer::write(Efer::read() | EferFlags::SYSTEM_CALL_EXTENSIONS);
        serial_println!("   → Syscall setup COMPLETE (IF/DF/TF/AC masked at entry)");    }
}

#[repr(C)]
pub struct SyscallRegs {
    pub rax: u64, pub rdi: u64, pub rsi: u64, pub rdx: u64,
    pub r10: u64, pub r8:  u64, pub r9:  u64, pub rcx: u64, pub r11: u64,
}

static SYSCALL_STUB_ENTER_RAW: [u8; 23] = *b"syscall.stub.enter.raw\n";

#[no_mangle]
pub extern "C" fn syscall_stub_after_swapgs() {
    serial_println!("syscall.stub.enter.raw");
}

#[no_mangle]
pub extern "C" fn syscall_stub_after_kstack_switch() {
    serial_println!("syscall.stub.after.kstack.switch");
}

#[no_mangle]
pub extern "C" fn syscall_stub_before_dispatch() {
    serial_println!("syscall.stub.before.dispatch");
}

#[unsafe(naked)]
pub unsafe extern "C" fn syscall_entry() {
    core::arch::naked_asm!(
        "swapgs",
        "mov gs:[24], rsp", // Scratch: temporarily save user RSP
        "mov rsp, gs:[16]",  // Load Kernel RSP
        // Push user RSP onto the per-task kernel stack immediately.
        // gs:[24] is a shared per-core slot; if the timer preempts this task
        // mid-syscall and another task runs its own syscall, gs:[24] gets
        // overwritten. Saving user RSP on the kernel stack (per-task) prevents
        // that race. yield_and_switch reads user RSP from base+128, not gs:[24].
        "push qword ptr gs:[24]", // Save user RSP on per-task kernel stack

        // 1. SAVE ALL STATE BEFORE CLOBBERING
        "push r11",         // Save User RFLAGS
        "push rcx",         // Save User RIP
        "push rax",         // Save Syscall Number (rax)
        "push rdi",         // Save Arg 0 (rdi)
        "push rsi",         // Save Arg 1 (rsi)
        "push rdx",         // Save Arg 2 (rdx)
        "push r10",         // Save Arg 3 (r10)
        "push r8",          // Save Arg 4 (r8)
        "push r9",          // Save Arg 5 (r9)

        // 2. Read and Save User PKRU (or synthetic 0 when PKU unsupported)
        "cmp byte ptr [rip + {pku_enabled}], 0",
        "je 90f",
        "xor ecx, ecx",
        "rdpkru",
        "jmp 91f",
        "90:",
        "xor eax, eax",
        "91:",
        "push rax",         // Save PKRU mask to stack

        // 4. Enter God Mode
        "cmp byte ptr [rip + {pku_enabled}], 0",
        "je 92f",
        "xor eax, eax",
        "xor ecx, ecx",
        "xor edx, edx",
        "wrpkru",
        "92:",

        // 5. Save Callee-Saved Registers
        "push rbp", "push rbx", "push r12", "push r13", "push r14", "push r15",
        "mov rbp, rsp",

        // 6. Build SyscallRegs (Volatiles + Args)
        // Stack at this point (top down):
        // [rbp+0..47]: r15-rbp (48)
        // [rbp+48]: PKRU mask (8)
        // [rbp+56]: r9 (8)
        // [rbp+64]: r8 (8)
        // [rbp+72]: r10 (8)
        // [rbp+80]: rdx (8)
        // [rbp+88]: rsi (8)
        // [rbp+96]: rdi (8)
        // [rbp+104]: rax (8)
        // [rbp+112]: rcx (8) - RIP
        // [rbp+120]: r11 (8) - RFLAGS
        // [rbp+128]: user RSP (8) - pushed first, popped last via `pop rsp`

        "push qword ptr [rbp + 120]", // r11
        "push qword ptr [rbp + 112]", // rcx
        "push qword ptr [rbp + 56]",  // r9
        "push qword ptr [rbp + 64]",  // r8
        "push qword ptr [rbp + 72]",  // r10
        "push qword ptr [rbp + 80]",  // rdx
        "push qword ptr [rbp + 88]",  // rsi
        "push qword ptr [rbp + 96]",  // rdi
        "push qword ptr [rbp + 104]", // rax

        // Call Handler
        "mov rdi, rsp",     // Pointer to SyscallRegs
        "call syscall_handler",

        // 6.5 Propagate modified regs back to original saved slots.
        // dispatch may set regs.rsi/rdx/r10/r8 (e.g. PDX_CALL/PDX_LISTEN returns).
        // Restore path pops from [rbp+64..88]; copy SyscallRegs values there.
        "mov rcx, [rsp + 16]", // SyscallRegs.rsi (offset 16 in struct)
        "mov [rbp + 88], rcx", // original saved RSI slot
        "mov rcx, [rsp + 24]", // SyscallRegs.rdx (offset 24)
        "mov [rbp + 80], rcx", // original saved RDX slot
        "mov rcx, [rsp + 32]", // SyscallRegs.r10 (offset 32)
        "mov [rbp + 72], rcx", // original saved R10 slot
        "mov rcx, [rsp + 40]", // SyscallRegs.r8 (offset 40)
        "mov [rbp + 64], rcx", // original saved R8 slot

        // 7. RESTORE C-ABI VOLATILES
        "add rsp, 72",      // Discard SyscallRegs

        // 8. Restore Callee-Saved Registers
        "pop r15", "pop r14", "pop r13", "pop r12", "pop rbx", "pop rbp",

        // 9. Restore PKRU Mask
        "pop rdi",          // Pop original PKRU mask into rdi
        "push rax",         // Stash return value
        "cmp byte ptr [rip + {pku_enabled}], 0",
        "je 93f",
        "mov rax, rdi",
        "xor ecx, ecx",
        "xor edx, edx",
        "wrpkru",
        "93:",
        "pop rax",

        // 10. Restore Saved Volatiles
        "pop r9", "pop r8", "pop r10", "pop rdx", "pop rsi", "pop rdi",
        "add rsp, 8",       // Discard rax (return value already in rax)
        "pop rcx",          // RIP
        "pop r11",          // RFLAGS

        "pop rsp",          // Restore user RSP from per-task kernel stack (pushed at entry)
        "swapgs",
        "sysretq",
        pku_enabled = sym crate::pku::PKU_ENABLED,
    );
}

#[no_mangle]
pub extern "C" fn syscall_handler(regs: &mut SyscallRegs) -> u64 {
    unsafe { crate::pku::wrpkru(0x00000000); }
    crate::scheduler::observe_user_entry();
    crate::syscalls::dispatch(regs)
}

// ── Assembly Stubs ──────────────────────────────────────────────────────────

#[unsafe(naked)]
pub unsafe extern "C" fn timer_interrupt_stub() {
    core::arch::naked_asm!(
        "push 0", // DUMMY ERROR CODE
        "push r15", "push r14", "push r13", "push r12", "push r11", "push r10", "push r9", "push r8",
        "push rdi", "push rsi", "push rbp", "push rdx", "push rcx", "push rbx", "push rax",
        
        // CPU Frame = 40 bytes. GPRs = 120 bytes. Error Code = 8 bytes.
        // RIP is at 128. CS is at 136.
        "mov rax, [rsp + 136]", 
        "test al, 3", 
        "jz 1f", 
        "swapgs", 
        "1:",

        "cmp byte ptr [rip + {pku_enabled}], 0",
        "je 3f",
        // Reuse the dummy error qword after all GPRs are saved. A returning
        // NoRunnable/kernel timer must restore entry PKRU, not leave it open.
        // The switch path still discards this slot; frame offsets do not move.
        "xor ecx, ecx",
        "rdpkru",
        "mov [rsp + 120], rax",
        "xor eax, eax",
        "xor ecx, ecx",
        "xor edx, edx",
        "wrpkru",
        "3:",

        "sub rsp, 8", // ALIGN STACK TO 16 BYTES
        "lea rdi, [rsp + 136]", // rdi points to InterruptStackFrame (RIP)
        "call timer_interrupt_handler",
        "add rsp, 8", // UNDO ALIGNMENT

        "cmp byte ptr [rip + {pku_enabled}], 0",
        "je 4f",
        "mov eax, [rsp + 120]",
        "xor ecx, ecx",
        "xor edx, edx",
        "wrpkru",
        "4:",

        "mov rax, [rsp + 136]", 
        "test al, 3", 
        "jz 2f", 
        "swapgs", 
        "2:",

        "pop rax", "pop rbx", "pop rcx", "pop rdx", "pop rbp", "pop rsi", "pop rdi",
        "pop r8", "pop r9", "pop r10", "pop r11", "pop r12", "pop r13", "pop r14", "pop r15",
        "add rsp, 8", // DISCARD DUMMY ERROR CODE
        "iretq",
        pku_enabled = sym crate::pku::PKU_ENABLED,
    );
}

#[unsafe(naked)]
pub unsafe extern "C" fn page_fault_stub() {
    core::arch::naked_asm!(
        // CPU already pushed Error Code (8 bytes). CPU frame = 48 bytes.
        "push r15", "push r14", "push r13", "push r12", "push r11", "push r10", "push r9", "push r8",
        "push rdi", "push rsi", "push rbp", "push rdx", "push rcx", "push rbx", "push rax",

        // GPRs = 120 bytes. Error Code is at 120. RIP is at 128. CS is at 136.
        "mov rax, [rsp + 136]", 
        "test al, 3", 
        "jz 1f", 
        "swapgs", 
        "1:",

        "cmp byte ptr [rip + {pku_enabled}], 0",
        "je 3f",
        "xor eax, eax",
        "xor ecx, ecx",
        "xor edx, edx",
        "wrpkru",
        "3:",

        "sub rsp, 8", // CRITICAL: Re-align stack to 16 bytes before C call!
        "lea rdi, [rsp + 136]", // rdi points to InterruptStackFrame (RIP)
        "mov rsi, [rsp + 128]", // rsi contains the Error Code
        "call page_fault_handler",
        "add rsp, 8", // UNDO ALIGNMENT

        "mov rax, [rsp + 136]", 
        "test al, 3", 
        "jz 2f", 
        "swapgs", 
        "2:",

        "pop rax", "pop rbx", "pop rcx", "pop rdx", "pop rbp", "pop rsi", "pop rdi",
        "pop r8", "pop r9", "pop r10", "pop r11", "pop r12", "pop r13", "pop r14", "pop r15",
        "add rsp, 8", // DISCARD ERROR CODE
        "iretq",
        pku_enabled = sym crate::pku::PKU_ENABLED,
    );
}

#[unsafe(naked)]
pub unsafe extern "C" fn general_protection_fault_stub() {
    core::arch::naked_asm!(
        // CPU already pushed Error Code (8 bytes). CPU frame = 48 bytes.
        "push r15", "push r14", "push r13", "push r12", "push r11", "push r10", "push r9", "push r8",
        "push rdi", "push rsi", "push rbp", "push rdx", "push rcx", "push rbx", "push rax",

        // GPRs = 120 bytes. Error Code is at 120. RIP is at 128. CS is at 136.
        "mov rax, [rsp + 136]", 
        "test al, 3", 
        "jz 1f", 
        "swapgs", 
        "1:",

        "cmp byte ptr [rip + {pku_enabled}], 0",
        "je 3f",
        "xor eax, eax",
        "xor ecx, ecx",
        "xor edx, edx",
        "wrpkru",
        "3:",

        "sub rsp, 8", // CRITICAL: Re-align stack to 16 bytes before C call!
        "lea rdi, [rsp + 136]", // rdi points to InterruptStackFrame (RIP)
        "mov rsi, [rsp + 128]", // rsi contains the Error Code
        "call general_protection_fault_handler",
        "add rsp, 8", // UNDO ALIGNMENT

        "mov rax, [rsp + 136]", 
        "test al, 3", 
        "jz 2f", 
        "swapgs", 
        "2:",

        "pop rax", "pop rbx", "pop rcx", "pop rdx", "pop rbp", "pop rsi", "pop rdi",
        "pop r8", "pop r9", "pop r10", "pop r11", "pop r12", "pop r13", "pop r14", "pop r15",
        "add rsp, 8", // DISCARD ERROR CODE
        "iretq",
        pku_enabled = sym crate::pku::PKU_ENABLED,
    );
}

// ── Rust Handlers ───────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn timer_interrupt_handler(stack_frame: &mut InterruptStackFrame) {
    // Runtime reachability proof v1: unconditional unbudgeted marker.
    // Fires on every timer interrupt to prove LAPIC → IDT → handler path.
    unsafe {
        static mut TIMER_FIRE_COUNT: u64 = 0;
        TIMER_FIRE_COUNT += 1;
        if TIMER_FIRE_COUNT <= 5 {
            serial_println!("timer.fire.count={}", TIMER_FIRE_COUNT);
        }
    }
    if TIMER_TICK_LOG_BUDGET
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1))
        .is_ok()
    {
        serial_println!("timer.tick.enter");
    }
    TICKS.fetch_add(1, Ordering::Relaxed);
    // Boot-phase guard: skip scheduler tick until SchedulerRunning.
    // Early LAPIC timer interrupts fire before userland PDs are spawned.
    // Accumulate TICKS harmlessly; defer sched.tick() until the boot
    // controller advances past SchedulerArmed.
    unsafe {
        if (crate::ipc::BOOT_CONTROLLER.phase() as u8)
            < (crate::ipc::BootPhase::SchedulerRunning as u8)
        {
            static mut TICK_DEFER_BUDGET: u32 = 8;
            if TICK_DEFER_BUDGET > 0 {
                TICK_DEFER_BUDGET -= 1;
                serial_println!(
                    "[timer.tick.defer] reason=scheduler_not_running phase={}",
                    crate::ipc::BOOT_CONTROLLER.phase() as u8
                );
            }
            send_eoi();
            return;
        }
    }
    let core_id = crate::core_local::CoreLocal::get().core_id;
    let sched = &crate::scheduler::SCHEDULERS[core_id as usize];
    crate::scheduler::poll_paging_exit_notifications();
    crate::scheduler::poll_self_wait_deadlines(TICKS.load(Ordering::Relaxed));

    // Timer fired in kernel mode (CS.RPL=0) means a task is mid-syscall on the shared
    // CoreLocal kernel stack. Saving context there is unsafe: other tasks' syscalls reuse
    // the same stack and will corrupt the saved frame before this task is rescheduled.
    // Skip the context switch; the task finishes its syscall and yields cooperatively.
    if stack_frame.code_segment.0 as u64 & 3 == 0 {
        crate::scheduler::proof_kernel_timer_wake();
        unsafe { send_eoi(); }
        return;
    }

    crate::scheduler::observe_user_entry();

    let (old_ctx_ptr, next_ctx_ptr) = match sched.tick() {
        crate::scheduler::ScheduleDecision::NoRunnable => {
            unsafe { send_eoi(); }
            return;
        }
        crate::scheduler::ScheduleDecision::Switch { old, next } => (old, next),
    };

    unsafe {
        // Save old context from interrupt stub's GPR push level on the stack.
        // stack_frame points to RIP.  Stub pushed [rax..r15][dummy=0] above RIP.
        // Layout from RIP downward: [dummy][r15][r14]..[rbx][rax]
        if !old_ctx_ptr.is_null() {
            let old_ctx = &mut *old_ctx_ptr;
            let base = stack_frame as *const _ as *const u64;
            old_ctx.rip = stack_frame.instruction_pointer.as_u64();
            old_ctx.cs = stack_frame.code_segment.0 as u64;
            old_ctx.rflags = stack_frame.cpu_flags.bits();
            old_ctx.rsp = stack_frame.stack_pointer.as_u64();
            old_ctx.ss = stack_frame.stack_segment.0 as u64;
            old_ctx.r15 = *base.offset(-2);
            old_ctx.r14 = *base.offset(-3);
            old_ctx.r13 = *base.offset(-4);
            old_ctx.r12 = *base.offset(-5);
            old_ctx.r11 = *base.offset(-6);
            old_ctx.r10 = *base.offset(-7);
            old_ctx.r9  = *base.offset(-8);
            old_ctx.r8  = *base.offset(-9);
            old_ctx.rdi = *base.offset(-10);
            old_ctx.rsi = *base.offset(-11);
            old_ctx.rbp = *base.offset(-12);
            old_ctx.rdx = *base.offset(-13);
            old_ctx.rcx = *base.offset(-14);
            old_ctx.rbx = *base.offset(-15);
            old_ctx.rax = *base.offset(-16);
            // kstack_top = top of stub's GPR push (where rax sits).
            // Both forged (Task::new) and saved stacks have 16 qwords
            // above IRET frame (15 GPRs + 1 dummy).  switch_to loads
            // this and does add rsp, 128 before iretq.
            old_ctx.kstack_top = (base as u64) - 128;
            // Read PKRU from the PD's stored mask (rdpkru would return 0
            // in God Mode inside this handler).
            // pp-2026-05-25: guard null pd_ptr to prevent cascading #PF on broken tasks
            old_ctx.pkru = if old_ctx.pd_ptr.is_null() {
                0xFFFF_FFFFu64  // deny-all PKRU (all keys disabled) for broken task
            } else {
                (*old_ctx.pd_ptr).current_pkru_mask.load(core::sync::atomic::Ordering::Relaxed) as u64
            };
        }

        let kstack_top = (*next_ctx_ptr).kstack_top;
        if TIMER_TICK_LOG_BUDGET.load(Ordering::Acquire) > 0 {
            serial_println!("scheduler.restore_context pd_id={} kstack_top={:#x} rip={:#x}",
                (*next_ctx_ptr).pd_id, kstack_top, (*next_ctx_ptr).rip);
        }

        // RSP0 must be empty kernel-stack top, not saved-context base.
        // kstack_top is where the context (GPRs+IRET) is saved. 
        // kstack_top + 168 (15 regs + dummy + 5 iret qwords) = kstack_alloc_top.
        crate::gdt::update_tss_rsp0(x86_64::VirtAddr::new(kstack_top + 168));

        send_eoi();
        crate::scheduler::Scheduler::switch_to(old_ctx_ptr, next_ctx_ptr);
    }
}


/// Kernel-mode halt loop. Redirect faulted user tasks here so IRETQ
/// never returns to the faulting RIP (which causes immediate re-fault).
#[no_mangle]
pub extern "C" fn faulted_task_halt() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}

#[no_mangle]
pub extern "C" fn page_fault_handler(stack_frame: &mut InterruptStackFrame, error_code: u64) {
    use x86_64::registers::control::Cr2;
    let fault_addr = Cr2::read_raw();
    let fault_rip = stack_frame.instruction_pointer.as_u64();
    let fault_rsp = stack_frame.stack_pointer.as_u64();
    let fault_cs = stack_frame.code_segment.0 as u64;
    let fault_cs_rpl = fault_cs & 0x3;

    let cur_pd = crate::core_local::CoreLocal::get().current_pd();

    // Kernel faults never enter the user pager transaction. User protection,
    // PKU, reserved-bit, instruction-fetch and unknown faults fail closed too.
    if fault_cs_rpl != 3 {
        serial_println!("KERNEL PAGE FAULT HALT: addr={:#x} rip={:#x} rsp={:#x} err={:#x} pd={}",
            fault_addr, fault_rip, fault_rsp, error_code, cur_pd);
        loop { x86_64::instructions::hlt(); }
    }
    let recoverable = crate::scheduler::fault_transaction::eligible_user_fault(
        fault_addr, error_code, fault_cs);
    serial_println!("[paging.fault.trap] pd={} addr={:#x} rip={:#x} error={:#x} eligible={}",
        cur_pd, fault_addr, fault_rip, error_code, recoverable as u8);

    let core_id = crate::core_local::CoreLocal::get().core_id;
    let sched = &crate::scheduler::SCHEDULERS[core_id as usize];
    let current_ptr = sched.current_task.load(Ordering::Acquire);
    unsafe {
        let current_ctx_ptr = if current_ptr.is_null() { core::ptr::null_mut() }
            else { &mut (*current_ptr).context as *mut crate::scheduler::TaskContext };
        if !current_ctx_ptr.is_null() {
            let old_ctx = &mut *current_ctx_ptr;
            let base = stack_frame as *const _ as *const u64;
            old_ctx.rip = stack_frame.instruction_pointer.as_u64();
            old_ctx.cs = stack_frame.code_segment.0 as u64;
            old_ctx.rflags = stack_frame.cpu_flags.bits();
            old_ctx.rsp = stack_frame.stack_pointer.as_u64();
            old_ctx.ss = stack_frame.stack_segment.0 as u64;
            old_ctx.r15 = *base.offset(-2);
            old_ctx.r14 = *base.offset(-3);
            old_ctx.r13 = *base.offset(-4);
            old_ctx.r12 = *base.offset(-5);
            old_ctx.r11 = *base.offset(-6);
            old_ctx.r10 = *base.offset(-7);
            old_ctx.r9  = *base.offset(-8);
            old_ctx.r8  = *base.offset(-9);
            old_ctx.rdi = *base.offset(-10);
            old_ctx.rsi = *base.offset(-11);
            old_ctx.rbp = *base.offset(-12);
            old_ctx.rdx = *base.offset(-13);
            old_ctx.rcx = *base.offset(-14);
            old_ctx.rbx = *base.offset(-15);
            old_ctx.rax = *base.offset(-16);
            old_ctx.kstack_top = (base as u64) - 128;
            old_ctx.pkru = if old_ctx.pd_ptr.is_null() {
                0xFFFF_FFFFu64  // deny-all PKRU (all keys disabled) for broken task
            } else {
                (*old_ctx.pd_ptr).current_pkru_mask.load(core::sync::atomic::Ordering::Relaxed) as u64
            };
        }

        if !current_ptr.is_null() {
            // This complete frame belongs exclusively to the current owner.
            // Publish nothing until every saved GPR and IRET field is final.
            if recoverable {
                let parked = (*current_ptr).state.park_fault(core_id);
                if parked != crate::scheduler::task_ownership::ParkResult::Blocked {
                    (*current_ptr).state.exit();
                    serial_println!("fault.kill [paging.fault.terminal] pd={} reason=owner_not_parkable", cur_pd);
                } else if let Err(error) = crate::ipc::pagefault::forward_page_fault(fault_addr, error_code, cur_pd as u64) {
                    (*current_ptr).state.exit();
                    serial_println!("fault.kill [paging.fault.terminal] pd={} reason=dispatch_failed status={}", cur_pd, error);
                }
            } else {
                (*current_ptr).state.exit();
                serial_println!("fault.kill [paging.fault.terminal] pd={} reason=ineligible error={:#x}", cur_pd, error_code);
            }
        } else {
            panic!("USER_PAGE_FAULT_WITHOUT_TASK");
        }

        send_eoi();
        let next_ctx_ptr = match sched.tick() {
            crate::scheduler::ScheduleDecision::Switch { next, .. } => next,
            crate::scheduler::ScheduleDecision::NoRunnable => {
                crate::scheduler::wait_for_local_resume_or_switch(sched)
            }
        };
        let kstack_top = (*next_ctx_ptr).kstack_top;
        crate::gdt::update_tss_rsp0(x86_64::VirtAddr::new(kstack_top + 168));
        crate::scheduler::Scheduler::switch_to(current_ctx_ptr, next_ctx_ptr);
    }
}

#[no_mangle]
pub extern "C" fn general_protection_fault_handler(
    stack_frame: &mut InterruptStackFrame,
    error_code: u64,
) {
    #[repr(C, packed)]
    struct Gdtr64 {
        limit: u16,
        base: u64,
    }
    let mut gdtr = Gdtr64 { limit: 0, base: 0 };
    let mut tr_sel: u16 = 0;
    unsafe {
        core::arch::asm!("sgdt [{}]", in(reg) &mut gdtr, options(nostack, preserves_flags));
        core::arch::asm!("str {0:x}", out(reg) tr_sel, options(nostack, preserves_flags));
    }
    let gdtr_base = unsafe { core::ptr::addr_of!(gdtr.base).read_unaligned() };
    let gdtr_limit = unsafe { core::ptr::addr_of!(gdtr.limit).read_unaligned() };

    let actual_rsp = unsafe { crate::scheduler::ACTUAL_IRET_RSP };
    let q0 = unsafe { crate::scheduler::ACTUAL_IRET_Q0_RIP };
    let q1 = unsafe { crate::scheduler::ACTUAL_IRET_Q1_CS };
    let q2 = unsafe { crate::scheduler::ACTUAL_IRET_Q2_RFLAGS };
    let q3 = unsafe { crate::scheduler::ACTUAL_IRET_Q3_RSP };
    let q4 = unsafe { crate::scheduler::ACTUAL_IRET_Q4_SS };

    serial_println!(
        "iret.actual rsp={:#x} q0.rip={:#x} q1.cs={:#x} q2.rflags={:#x} q3.rsp={:#x} q4.ss={:#x}",
        actual_rsp, q0, q1, q2, q3, q4
    );
    serial_println!(
        "iret.actual rsp.canonical={} rsp.align16={}",
        ((actual_rsp >> 48) == 0) || ((actual_rsp >> 48) == 0xffff),
        actual_rsp & 0xF
    );
    serial_println!("cpu.tables gdtr.base={:#x} gdtr.limit={:#x} tr={:#x}", gdtr_base, gdtr_limit, tr_sel);

    let gdt_base = gdtr_base as *const u64;
    let cs_idx = ((q1 & !0x7) >> 3) as isize;
    let ss_idx = ((q4 & !0x7) >> 3) as isize;
    let cs_raw = unsafe { *gdt_base.offset(cs_idx) };
    let ss_raw = unsafe { *gdt_base.offset(ss_idx) };
    serial_println!(
        "cpu.gdt.decode cs.sel={:#x} cs.raw={:#018x} ss.sel={:#x} ss.raw={:#018x}",
        q1, cs_raw, q4, ss_raw
    );

    let cur_pd = crate::core_local::CoreLocal::get().current_pd();
    serial_println!(
        "EXCEPTION: GP FAULT ENTER pd={} err={:#x} rip={:#x} rsp={:#x}",
        cur_pd,
        error_code,
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64()
    );
    panic!(
        "EXCEPTION: GP FAULT\nError: {:#x}\nRSP (Stack Pointer): {:#x}\nRIP (Instruction Pointer): {:#x}",
        error_code,
        stack_frame.stack_pointer.as_u64(),
        stack_frame.instruction_pointer.as_u64()
    );
}

extern "x86-interrupt" fn invalid_opcode_handler(stack_frame: InterruptStackFrame) {
    let cur_pd = crate::core_local::CoreLocal::get().current_pd();
    serial_println!(
        "EXCEPTION: UD ENTER pd={} rip={:#x} rsp={:#x}",
        cur_pd,
        stack_frame.instruction_pointer.as_u64(),
        stack_frame.stack_pointer.as_u64()
    );
    panic!("EXCEPTION: INVALID OPCODE");
}

pub unsafe fn send_eoi() {
    let lapic_vaddr = crate::apic::LAPIC_ADDR.load(Ordering::Acquire);
    if lapic_vaddr != 0 {
        (lapic_vaddr as *mut u32).offset(0x0B0 / 4).write_volatile(0);
    } else {
        core::arch::asm!("mov al, 0x20", "out 0x20, al", "out 0xa0, al");
    }
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    serial_println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(stack_frame: InterruptStackFrame, _error_code: u64) -> ! {
    serial_println!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
    panic!("DOUBLE FAULT");
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let irq_n = PS2_IRQ1_LOG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if irq_n <= 16 {
        serial_println!("[ps2.irq1.entry] n={}", irq_n);
    }

    let scancode = match crate::keyboard::read_scancode() {
        Some(sc) => sc,
        None => {
            // Keyboard not initialized (i8042 absent on this machine).
            // Drop the spurious interrupt, but MUST drain port 0x60 AND EOI:
            // an unread byte keeps the i8042 output line asserted (no future
            // IRQ1 edges), and a missing EOI leaves vector 0x21 in-service.
            // Either alone kills all future IRQ1 delivery (root cause of
            // "QEMU sendkey no PS/2 IRQ1 delivery", 2026-07-02).
            let _ = crate::keyboard::read_scancode_raw();
            unsafe { send_eoi(); }
            return;
        }
    };
    let read_n = PS2_PORT60_LOG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if read_n <= 16 {
        serial_println!("[ps2.port60.read] n={} sc={:#x}", read_n, scancode);
    }

    // Push into lock-free global queue; discard if full (to prevent blocking Ring-0)
    if INPUT_RING.enqueue(scancode).is_ok() {
        let enq_n = PS2_ENQUEUE_LOG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if enq_n <= 16 {
            serial_println!("[ps2.input_ring.enqueue] n={} type=0x201 sc={:#x}", enq_n, scancode);
        }
    } else {
        let drop_n = PS2_DROP_LOG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if drop_n <= 16 {
            serial_println!("[ps2.input_ring.drop] reason=ring_full sc={:#x}", scancode);
        }
    }
    
    let eoi_n = PS2_EOI_LOG_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
    if eoi_n <= 16 {
        serial_println!("[ps2.eoi] n={}", eoi_n);
    }
    unsafe { send_eoi(); }
}
extern "x86-interrupt" fn generic_irq_handler(_stack_frame: InterruptStackFrame) { route_interrupt(0x22); }
extern "x86-interrupt" fn revoke_key_handler(_stack_frame: InterruptStackFrame) {
    crate::hal::tlb_flush_local();
    unsafe { send_eoi(); }
}

fn route_interrupt(vector: u8) {
    let pd_id = VECTOR_OWNERS[vector as usize].load(Ordering::Acquire);
    if pd_id == 0 { unsafe { send_eoi(); } return; }
    if let Some(target_pd) = crate::ipc::DOMAIN_REGISTRY.get(pd_id) {
        let msg = MessageType::HardwareInterrupt { vector, data: 0 };
        let _ = unsafe { &*target_pd.message_ring }.enqueue(msg);
        let main_task = target_pd.main_task.load(Ordering::Acquire);
        if !main_task.is_null() { crate::scheduler::unpark_thread(main_task); }
    }
    unsafe { send_eoi(); }
}
