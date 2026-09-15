//! Phase-4 minimal scheduler — mechanism proof ONLY (DEVIATION noted in
//! handoff: the plan's "minimal scheduler.rs cfg hooks" refers to the x86
//! scheduler, which is cfg(target_arch="x86_64")-gated and cannot be
//! compiled for aarch64; policy/state-machine parity is a later phase).
//! This is an arch-local 2-task preemptive ping-pong: CNTP tick IRQ →
//! save frame → alternate A/B → restore frame. Plus the kill-task
//! containment path (a faulting task is dropped, the survivor resumes).

use super::context;
use super::serial;
use super::timer;
use core::sync::atomic::{AtomicBool, Ordering};

static mut FRAME_A: u64 = 0;
static mut FRAME_B: u64 = 0;
static mut CUR: u8 = 0; // 0 = A, 1 = B
static STARTED: AtomicBool = AtomicBool::new(false);
static B_DEAD: AtomicBool = AtomicBool::new(false);

pub fn b_dead() -> bool { B_DEAD.load(Ordering::Acquire) }

/// Build both task frames. Stacks/context are static; boot-only.
pub fn init() {
    unsafe {
        FRAME_A = context::build_frame(&raw mut context::STACK_A, task_a as u64);
        FRAME_B = context::build_frame(&raw mut context::STACK_B, task_b as u64);
        CUR = 0;
    }
}

/// Enter task A. Diverges (timer IRQ drives all further switching).
pub fn start() -> ! {
    serial::write_str("[sex.arm.sched] start=1\n");
    STARTED.store(true, Ordering::Release);
    unsafe { context::sex_arm_first(FRAME_A) }
}

/// Timer-tick switch. Called from the IRQ handler with the current frame;
/// returns the frame to resume. Single core, no locks needed.
#[unsafe(no_mangle)]
extern "C" fn sex_arm_tick(frame: u64, intid: u64) -> u64 {
    timer::reload();
    if intid != super::registers::INTID_TIMER as u64 {
        serial::write_str("[sex.arm.sched] FATAL stray_irq=");
        serial::write_dec(intid);
        serial::write_str("\n");
        loop { core::hint::spin_loop() }
    }
    unsafe {
        if CUR == 0 { FRAME_A = frame } else { FRAME_B = frame }
        let next = if b_dead() { 0 } else { 1 - CUR };
        CUR = next;
        if next == 0 { FRAME_A } else { FRAME_B }
    }
}

/// Kill-task containment (USER_FAULT_CONTAINMENT analog): a synchronous
/// fault in a SCHEDULED task is contained — print the marker, drop the
/// task, resume the other. A fault BEFORE the scheduler starts (boot
/// code) or in the survivor remains fatal. Returns the frame to resume,
/// or 0 = fatal (caller halts; details already printed).
#[unsafe(no_mangle)]
extern "C" fn sex_arm_kill_warden(esr: u64, far: u64, elr: u64, frame: u64) -> u64 {
    if !STARTED.load(Ordering::Acquire) {
        serial::write_str("[sex.arm.panic] el1_sync_boot esr=0x");
        serial::write_hex(esr);
        serial::write_str(" far=0x");
        serial::write_hex(far);
        serial::write_str(" elr=0x");
        serial::write_hex(elr);
        serial::write_str("\nSEX_ARM_PHASE4_FAIL\n");
        return 0;
    }
    unsafe {
        serial::write_str("[sex.arm.neg] kill_task=contained far=0x");
        serial::write_hex(far);
        serial::write_str("\n");
        // Only task B faults by design; if A ever faults, that is fatal.
        if CUR == 0 {
            serial::write_str("[sex.arm.panic] kill_survivor esr=0x");
            serial::write_hex(esr);
            serial::write_str("\nSEX_ARM_PHASE4_FAIL\n");
            return 0;
        }
        FRAME_B = frame; // recorded, never resumed
        B_DEAD.store(true, Ordering::Release);
        CUR = 0;
        FRAME_A
    }
}

/// Task A: bounded prints, then after B's death counts 10 further
/// activations and emits the phase marker. Silent WFE afterwards.
fn task_a() -> ! {
    let mut n: u64 = 0;
    let mut after: u64 = 0;
    loop {
        if b_dead() {
            after += 1;
            if after == 10 {
                serial::write_str("[sex.arm.sched] a_after_dead=10\n");
                serial::write_str("SEX_ARM_PHASE4_OK\n");
                super::phase5::run(); // diverges (timer off, EL0 SVC test)
            }
        } else {
            n += 1;
            if n <= 3 {
                serial::write_str("[sex.arm.sched] a_run=");
                serial::write_dec(n);
                serial::write_str("\n");
            }
        }
        super::registers::wfe();
    }
}

/// Task B: bounded prints, then a deliberate null write on run 3
/// (proves the kill-task containment path).
fn task_b() -> ! {
    let mut n: u64 = 0;
    loop {
        n += 1;
        if n <= 3 {
            serial::write_str("[sex.arm.sched] b_run=");
            serial::write_dec(n);
            serial::write_str("\n");
        }
        if n == 3 {
            // SAFETY: deliberately faulting write (null VA is unmapped) —
            // this is the Phase-4 kill-task negative test.
            unsafe { core::ptr::write_volatile(0 as *mut u64, 0) };
        }
        super::registers::wfe();
    }
}
