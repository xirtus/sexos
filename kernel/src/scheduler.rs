use core::sync::atomic::{AtomicBool, AtomicU32, Ordering, AtomicPtr, AtomicU64};
use crate::capability::ProtectionDomain;
use crate::ipc_ring::RingBuffer;
use crate::serial_println;
use core::ptr;

/// The execution state of a vThread (Task).
#[repr(C)]
pub struct TaskContext {
    pub r15: u64, pub r14: u64, pub r13: u64, pub r12: u64,
    pub r11: u64, pub r10: u64, pub r9: u64, pub r8: u64,
    pub rdi: u64, pub rsi: u64, pub rbp: u64, pub rdx: u64,
    pub rcx: u64, pub rbx: u64, pub rax: u64,
    pub dummy_error_code: u64,
    pub pkru: u64,
    pub pd_id: u64,
    pub rip: u64, pub cs: u64, pub rflags: u64, pub rsp: u64, pub ss: u64,
    pub pd_ptr: *const ProtectionDomain,
    pub kstack_top: u64,
}

const _: () = assert!(core::mem::offset_of!(TaskContext, kstack_top) == 0xC0);

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready = 0,
    Running = 1,
    Blocked = 2,
    Exited = 3,
}

pub const STATE_READY: u32 = 0;
pub const STATE_RUNNING: u32 = 1;
pub const STATE_BLOCKED: u32 = 2;

#[path = "task_ownership.rs"]
pub mod task_ownership;
#[path = "fault_transaction.rs"]
pub mod fault_transaction;
#[path = "task_exit.rs"]
pub mod task_exit;
pub use task_exit::{register_paging_exit_watch, validate_dead_target, acknowledge_dead_cleanup, poll_paging_exit_notifications};
use task_ownership::{Admission, ParkResult, TaskOwnership, WakeResult};

/// Phase 1 scheduler ownership decision.
/// NoRunnable explicitly preserves current_task/current_pd; it must not be
/// treated as a switch.
pub enum ScheduleDecision {
    Switch {
        old: *mut TaskContext,
        next: *const TaskContext,
    },
    NoRunnable,
}

pub struct InitArg {
    pub display_lease: crate::capability::DisplayHardwareLease,
}

/// A vThread (Task) in the SASOS model.
pub struct Task {
    pub id: u32,
    pub context: TaskContext,
    pub state: TaskOwnership,
    pub fault: fault_transaction::FaultSlot,
    pub signal_ring: *mut RingBuffer<u8, 32>,
    pub kstack_top: u64,
    pub(crate) ext_init: Option<InitArg>,
    resume_observation_pending: AtomicBool,
}

impl Task {
    pub fn new(id: u32, entry_point: u64, stack_top: u64, pd: &ProtectionDomain, is_user: bool) -> Self {
        let pkru = pd.current_pkru_mask.load(Ordering::SeqCst);
        let selectors = crate::gdt::get_selectors();

        let (cs, ss, rflags) = if is_user {
            (0x2Bu64, 0x23u64, 0x202u64)  // CS=index5|RPL3, SS=index4|RPL3
        } else {
            (selectors.kernel_cs.0 as u64, selectors.kernel_ss.0 as u64, 0x202u64)
        };

        let kstack = alloc::vec![0u8; 65536];
        let kstack_alloc_top = kstack.as_ptr() as u64 + 65536;
        core::mem::forget(kstack);

        // Pre-seed kstack with IRETQ frame + Dummy Error + GPR zeros.
        // Layout low→high: [r15=0..rax=0][dummy=0][RIP][CS][RFLAGS][RSP][SS]
        let forged_ksp = unsafe {
            let mut ksp = kstack_alloc_top as *mut u64;
            ksp = ksp.sub(1); *ksp = ss;
            ksp = ksp.sub(1); *ksp = stack_top;
            ksp = ksp.sub(1); *ksp = rflags;
            ksp = ksp.sub(1); *ksp = cs;
            ksp = ksp.sub(1); *ksp = entry_point;
            ksp = ksp.sub(1); *ksp = 0; // dummy error code
            for _ in 0..15 { ksp = ksp.sub(1); *ksp = 0; } // rax..r15 zeros
            ksp as u64
        };

        let context = TaskContext {
            r15: 0, r14: 0, r13: 0, r12: 0, r11: 0, r10: 0, r9: 0, r8: 0,
            rdi: 0, rsi: 0, rbp: 0, rdx: 0, rcx: 0, rbx: 0, rax: 0,
            dummy_error_code: 0,
            pkru: pkru as u64,
            pd_id: pd.id as u64,
            rip: entry_point,
            cs,
            rflags,
            rsp: stack_top,
            ss,
            pd_ptr: pd as *const _,
            kstack_top: forged_ksp,  // active ksp; switch_to saves/loads here
        };

        Self {
            id,
            context,
            state: TaskOwnership::new(),
            fault: fault_transaction::FaultSlot::new(),
            signal_ring: pd.signal_ring,
            kstack_top: kstack_alloc_top,  // initial alloc top (for TSS RSP0)
            ext_init: None,
            resume_observation_pending: AtomicBool::new(false),
        }
    }
}

pub const QUEUE_SIZE: usize = 512;
// Host-tested independently of the privileged context-switch assembly.
#[path = "scheduler_queue.rs"]
pub mod scheduler_queue;
pub type WorkStealingQueue = scheduler_queue::BoundedQueue<Task, QUEUE_SIZE>;

pub struct Scheduler {
    pub runqueue: WorkStealingQueue,
    pub current_task: AtomicPtr<Task>,
    pending_outgoing: AtomicPtr<Task>,
    retry_cursor: AtomicU32,
    pub core_id: u32,
}

static SCHED_NO_RUNNABLE_LOG_BUDGET: AtomicU64 = AtomicU64::new(16);
static FIRST_SCHEDULE_LOGGED: AtomicU64 = AtomicU64::new(0);
static SCHED_TICK_ENTER_LOG_BUDGET: AtomicU64 = AtomicU64::new(32);
static SCHED_PICK_NEXT_LOG_BUDGET: AtomicU64 = AtomicU64::new(32);
static TASK_LIFECYCLE_LOG_BUDGET: AtomicU64 = AtomicU64::new(128);
// PERF_LOG_NOISE_ABLATION_V1: first 4 per-event lines, then power-of-two summaries.
static SCHED_YIELD_SAVED_COUNT: AtomicU64 = AtomicU64::new(0);

#[no_mangle]
pub static mut ACTUAL_IRET_RSP: u64 = 0;
#[no_mangle]
pub static mut ACTUAL_IRET_Q0_RIP: u64 = 0;
#[no_mangle]
pub static mut ACTUAL_IRET_Q1_CS: u64 = 0;
#[no_mangle]
pub static mut ACTUAL_IRET_Q2_RFLAGS: u64 = 0;
#[no_mangle]
pub static mut ACTUAL_IRET_Q3_RSP: u64 = 0;
#[no_mangle]
pub static mut ACTUAL_IRET_Q4_SS: u64 = 0;
#[no_mangle]
pub static mut SWITCH_NEXT_CTX_PTR: u64 = 0;

impl Scheduler {
    pub const fn new(core_id: u32) -> Self {
        Self {
            runqueue: WorkStealingQueue::new(),
            current_task: AtomicPtr::new(ptr::null_mut()),
            pending_outgoing: AtomicPtr::new(ptr::null_mut()),
            retry_cursor: AtomicU32::new(0),
            core_id,
        }
    }

    pub fn tick(&self) -> ScheduleDecision {
        poll_paging_exit_notifications();
        if SCHED_TICK_ENTER_LOG_BUDGET
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1))
            .is_ok()
        {
            let b = self.runqueue.bottom.load(Ordering::Acquire);
            let t = self.runqueue.top.load(Ordering::Acquire);
            serial_println!(
                "scheduler.tick.enter core={} phase={} rq_depth={}",
                self.core_id,
                unsafe { crate::ipc::BOOT_CONTROLLER.phase() as u8 },
                b.wrapping_sub(t)
            );
        }
        // Enforce hard execution gate
        assert!(
            unsafe { crate::ipc::BOOT_CONTROLLER.phase() as u8 } >= crate::ipc::BootPhase::SchedulerRunning as u8,
            "SCHEDULER_RUNNING_VIOLATION"
        );

        // Boot Strap Gate
        assert!(
            crate::core_local::INITIALIZED.load(Ordering::Acquire) == true,
            "SCHEDULER_BOOTSTRAP_VIOLATION"
        );

        // Retained Detached custody survives Full/Contended admission. Scan a
        // bounded slice of the existing task inventory; Constructed never enters.
        self.retry_admission();
        let next_task = self.runqueue.steal();
        let next_task = if next_task.is_null() { self.attempt_steal() } else { next_task };
        // SCHEDULER_TICK_PD8_PF_FLAKE_V1: reject corrupt steal results before
        // they reach set_pd/switch_to. A clobbered Task (heap overwrite of a
        // live Task struct) or a stale/garbage queue slot shows up as an
        // out-of-heap pointer, a NULL context.pd_ptr, or an absurd pd_id.
        // Drain-and-retry rather than give up: a corrupt entry can sit AHEAD
        // of runnable tasks in the queue, and callers like yield_and_switch
        // park permanently on a None result (timer ticks skip kernel mode),
        // so returning None with runnable tasks still queued deadlocks
        // userland. Bounded by QUEUE_SIZE so a fully-corrupt queue cannot
        // spin forever. The marker payload identifies the injection path
        // (out_of_heap = queue slot corruption; corrupt_task = overwrite of
        // Task memory itself).
        let mut next_task = next_task;
        let mut reject_guard = 0usize;
        while !next_task.is_null() {
            let addr = next_task as usize;
            let corrupt = if addr < crate::HEAP_START || addr >= crate::HEAP_START + crate::HEAP_SIZE {
                serial_println!("[sched.steal.reject] ptr={:#x} reason=out_of_heap", addr as u64);
                true
            } else if !unsafe { (*next_task).state.begin_dispatch(self.core_id) } {
                // Stale/duplicate or exited token cannot grant execution custody.
                true
            } else {
                let pd_ptr = unsafe { (*next_task).context.pd_ptr };
                let pd_id = unsafe { (*next_task).context.pd_id };
                if pd_ptr.is_null() || pd_id as usize >= crate::ipc::MAX_DOMAINS {
                    serial_println!(
                        "[sched.steal.reject] ptr={:#x} pd_ptr={:#x} pd_id={} reason=corrupt_task",
                        addr as u64, pd_ptr as u64, pd_id
                    );
                    unsafe {
                        (*next_task).state.exit();
                        (*next_task).state.cancel_dispatch(self.core_id);
                    }
                    true
                } else {
                    // An exit before attachment consumes this token to Dead.
                    !unsafe { (*next_task).state.attach(self.core_id) }
                }
            };
            if !corrupt {
                break;
            }
            reject_guard += 1;
            if reject_guard >= QUEUE_SIZE {
                serial_println!("[sched.steal.reject.exhausted] rejects={}", reject_guard);
                next_task = ptr::null_mut();
                break;
            }
            next_task = self.runqueue.steal();
            if next_task.is_null() {
                next_task = self.attempt_steal();
            }
        }
        // Phase 1: explicit NoRunnable.
        // Do not touch current_task/current_pd until a real next task exists.
        if next_task.is_null() {
            if SCHED_NO_RUNNABLE_LOG_BUDGET
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1))
                .is_ok()
            {
                let current = self.current_task.load(Ordering::Acquire);
                if current.is_null() {
                    serial_println!(
                        "sched.no_runnable.idle core={} phase={} runqueue_empty=true",
                        self.core_id,
                        unsafe { crate::ipc::BOOT_CONTROLLER.phase() as u8 }
                    );
                } else {
                    serial_println!(
                        "sched.no_runnable.preserve core={} phase={} runqueue_empty=true task={:#x}",
                        self.core_id,
                        unsafe { crate::ipc::BOOT_CONTROLLER.phase() as u8 },
                        current as u64
                    );
                }
            }
            return ScheduleDecision::NoRunnable;
        }

        // A concrete next task has exclusive dispatch custody. Outgoing custody
        // remains SwitchingOut until the assembly's post-stack-release callback.
        let old_task = self.current_task.load(Ordering::Acquire);
        if !old_task.is_null() {
            assert!(unsafe { (*old_task).state.begin_switch_out(self.core_id) });
            assert!(self.pending_outgoing.swap(old_task, Ordering::AcqRel).is_null());
        }
        let replaced = self.current_task.swap(next_task, Ordering::AcqRel);
        assert_eq!(replaced, old_task);

        unsafe {
            if TASK_LIFECYCLE_LOG_BUDGET
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1))
                .is_ok()
            {
                serial_println!(
                    "task.running id={} pd_id={} rip={:#x} rsp={:#x}",
                    (*next_task).id,
                    (*next_task).context.pd_id,
                    (*next_task).context.rip,
                    (*next_task).context.rsp
                );
            }
        }

        unsafe {
            let core = crate::core_local::CoreLocal::get();
            let next_pd_ptr = (*next_task).context.pd_ptr as *mut crate::capability::ProtectionDomain;

            // Enforce strict atomic ordering: bind -> wrpkru -> switch_to
            core.set_pd(next_pd_ptr);
        }

        if SCHED_PICK_NEXT_LOG_BUDGET
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1))
            .is_ok()
        {
            let next_pd_id = unsafe { (*next_task).context.pd_id };
            serial_println!("scheduler.pick_next pd_id={}", next_pd_id);
        }

        // No outgoing pointer is published here: handlers still need to save
        // its context, and this core is still executing on its kernel stack.

        let old_ctx = if old_task.is_null() {
            core::ptr::null_mut()
        } else {
            unsafe { &mut (*old_task).context as *mut _ }
        };
        ScheduleDecision::Switch {
            old: old_ctx,
            next: unsafe { &(*next_task).context },
        }
    }


    pub fn admit_task(&self, task: *mut Task) -> Admission {
        if task.is_null() { return Admission::NotRunnable; }
        unsafe { (*task).state.try_admit(|| self.runqueue.try_push(task)) }
    }

    /// First admission is explicit; wake cannot activate unfinished trampolines.
    pub fn activate_task(&self, task: *mut Task) -> bool {
        if task.is_null() || unsafe { (*task).context.rip == 0 } { return false; }
        if !crate::app_resources::activation_permitted(unsafe{(*task).context.pd_id as u32}) {return false;}
        if !unsafe { (*task).state.activate() } { return false; }
        self.admit_task(task);
        true
    }

    fn retry_admission(&self) {
        const BATCH: u32 = 32;
        let start = self.retry_cursor.fetch_add(BATCH, Ordering::Relaxed);
        for offset in 0..BATCH {
            let id = start.wrapping_add(offset) % crate::ipc::MAX_DOMAINS as u32;
            if let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(id) {
                for task in [pd.main_task.load(Ordering::Acquire), pd.trampoline_task.load(Ordering::Acquire)] {
                    if !task.is_null() && unsafe { (*task).state.needs_admission() } {
                        self.admit_task(task);
                    }
                }
            }
        }
    }

    fn attempt_steal(&self) -> *mut Task {
        for i in 1..128 {
            let victim_id = (self.core_id + i) % 128;
            let task = SCHEDULERS[victim_id as usize].runqueue.steal();
            if !task.is_null() { return task; }
        }
        ptr::null_mut()
    }

    #[unsafe(naked)]
    pub unsafe extern "C" fn switch_to(_old_context: *mut TaskContext, next_context: *const TaskContext) -> ! {
        core::arch::naked_asm!(
            // rdi = old_ctx (*mut TaskContext) — kstack_top already set by timer_interrupt_handler
            // rsi = next_ctx (*const TaskContext)

            // old_ctx.kstack_top is set before this call in timer_interrupt_handler.
            // Do NOT save RSP here — that would overwrite the correct stub-frame base
            // with the kernel call-stack depth at time of switch_to entry.

            // Never publish the outgoing task until both context writes and
            // old-stack execution have ended. Scratch grows BELOW next's saved
            // frame, preserving every existing GPR/IRET offset. R12 is SysV
            // callee-saved and its user value remains in that untouched frame.
            "cli",
            "mov r12, rsi",
            "mov rsp, [r12 + 0xC0]",
            "and rsp, -16",
            "call {finish_switch}",
            "mov rsi, r12",
            "mov rsp, [rsi + 0xC0]",
            // Only now narrow PKRU; the Rust callback ran with kernel access.
            "cmp byte ptr [rip + {pku_enabled}], 0",
            "je 2f",
            "mov eax, [rsi + 0x80]", // TaskContext.pkru at offset 0x80
            "xor ecx, ecx",
            "xor edx, edx",
            "wrpkru",

            "2:",
            // 3. Debug-log IRET frame for GP fault logger (before pops, R11 scratch).
            //    IRET frame is at RSP+128 (16 qwords of GPRs+dummy above RIP).
            "lea r11, [rsp + 128]",
            "mov [rip + {actual_iret_rsp}], r11",
            "mov r11, [rsp + 128]", "mov [rip + {actual_q0}], r11",
            "mov r11, [rsp + 136]", "mov [rip + {actual_q1}], r11",
            "mov r11, [rsp + 144]", "mov [rip + {actual_q2}], r11",
            "mov r11, [rsp + 152]", "mov [rip + {actual_q3}], r11",
            "mov r11, [rsp + 160]", "mov [rip + {actual_q4}], r11",

            // 4. Check CS.RPL for userspace (swapgs needed before iretq).
            "mov r11, [rsp + 136]",
            "test r11, 3",
            "jz 3f",
            "swapgs",
            "3:",

            // 5. Pop GPRs in saved-stack order: [rax] at lowest address (kstack_top).
            //    Saved (preempted) tasks have correct register values here;
            //    forged (first-run) tasks have all zeros, so any pop order works.
            "pop rax", "pop rbx", "pop rcx", "pop rdx",
            "pop rbp", "pop rsi", "pop rdi",
            "pop r8", "pop r9", "pop r10", "pop r11",
            "pop r12", "pop r13", "pop r14", "pop r15",
            "add rsp, 8",  // skip dummy error code
            "iretq",

            finish_switch = sym scheduler_finish_switch,
            pku_enabled = sym crate::pku::PKU_ENABLED,
            actual_iret_rsp = sym ACTUAL_IRET_RSP,
            actual_q0 = sym ACTUAL_IRET_Q0_RIP,
            actual_q1 = sym ACTUAL_IRET_Q1_CS,
            actual_q2 = sym ACTUAL_IRET_Q2_RFLAGS,
            actual_q3 = sym ACTUAL_IRET_Q3_RSP,
            actual_q4 = sym ACTUAL_IRET_Q4_SS,
        );
    }
}

macro_rules! generate_schedulers {
    ($($idx:expr),*) => { [$(Scheduler::new($idx)),*] };
}

pub static SCHEDULERS: [Scheduler; 128] = generate_schedulers!(
    0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,
    32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,
    64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,
    96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127
);

static PARK_COUNT: AtomicU64 = AtomicU64::new(0);
static DETACHED_WAKE_COUNT: AtomicU64 = AtomicU64::new(0);
static STACK_RELEASE_COUNT: AtomicU64 = AtomicU64::new(0);
static LOCAL_RESUME_COUNT: AtomicU64 = AtomicU64::new(0);
static USER_RESUME_COUNT: AtomicU64 = AtomicU64::new(0);

fn log_wake_event(event: &str, task: *mut Task) {
    if task.is_null() { return; }
    let counter = match event {
        "park" => &PARK_COUNT,
        "detached_wake" => &DETACHED_WAKE_COUNT,
        "post_stack_release" => &STACK_RELEASE_COUNT,
        "local_resume" => &LOCAL_RESUME_COUNT,
        "user_resumed" => &USER_RESUME_COUNT,
        _ => return,
    };
    let n = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= 4 || n.is_power_of_two() {
        serial_println!("[scheduler.ownership] event={} task={} count={}", event, unsafe { (*task).id }, n);
    }
}

/// A fresh syscall or user timer frame proves the parked task actually crossed
/// the restore/IRET boundary; a merely selected task is not reported resumed.
pub fn observe_user_entry() {
    let core = crate::core_local::CoreLocal::get().core_id as usize;
    let task = SCHEDULERS[core].current_task.load(Ordering::Acquire);
    if !task.is_null() && unsafe { (*task).resume_observation_pending.swap(false, Ordering::AcqRel) } {
        log_wake_event("user_resumed", task);
    }
}

// Opt-in QEMU branch exercise. The sealed build spec supplies this cfg; normal
// images contain no queue-retention or synthetic wake behavior. Only BSP is an
// active scheduling lane today. Queued tokens retained here remain exclusively
// held by this proof until restored, with their ownership phase unchanged.
#[cfg(scheduler_wake_proof)]
static PROOF_STAGE: AtomicU32 = AtomicU32::new(0);
#[cfg(scheduler_wake_proof)]
static PROOF_TASK: AtomicPtr<Task> = AtomicPtr::new(ptr::null_mut());
#[cfg(scheduler_wake_proof)]
static PROOF_HELD_COUNT: AtomicU32 = AtomicU32::new(0);
#[cfg(scheduler_wake_proof)]
static PROOF_HELD: [AtomicPtr<Task>; QUEUE_SIZE] =
    [const { AtomicPtr::new(ptr::null_mut()) }; QUEUE_SIZE];

#[cfg(scheduler_wake_proof)]
fn begin_last_runnable_proof(sched: &Scheduler, current: *mut Task) {
    if sched.core_id != 0 || PROOF_STAGE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return;
    }
    PROOF_TASK.store(current, Ordering::Release);
    let mut count = 0;
    for source in &SCHEDULERS {
        loop {
            let token = source.runqueue.steal();
            if token.is_null() { break; }
            assert!(count < QUEUE_SIZE, "PROOF_QUEUE_CUSTODY_OVERFLOW");
            assert_eq!(unsafe { (*token).state.snapshot().phase }, task_ownership::Phase::Queued);
            PROOF_HELD[count].store(token, Ordering::Release);
            count += 1;
        }
    }
    PROOF_HELD_COUNT.store(count as u32, Ordering::Release);
    serial_println!("[scheduler.proof.last_runnable.begin] task={} held={} lane=bsp", unsafe { (*current).id }, count);
}

#[cfg(not(scheduler_wake_proof))]
fn begin_last_runnable_proof(_sched: &Scheduler, _current: *mut Task) {}

/// Called only for a kernel timer frame. This supplies the real asynchronous
/// wake while the last runnable task sleeps on its retained kernel stack.
#[cfg(scheduler_wake_proof)]
pub fn proof_kernel_timer_wake() {
    if PROOF_STAGE.load(Ordering::Acquire) != 1 { return; }
    let task = PROOF_TASK.load(Ordering::Acquire);
    let core = crate::core_local::CoreLocal::get().core_id;
    assert_eq!(core, 0, "PROOF_REQUIRES_BSP_LANE");
    assert_eq!(SCHEDULERS[0].current_task.load(Ordering::Acquire), task);
    let before = unsafe { (*task).state.snapshot() };
    assert_eq!(before.phase, task_ownership::Phase::Attached);
    assert!(before.blocked && !before.fault_blocked);
    unpark_thread(task);
    assert_eq!(unsafe { (*task).state.snapshot().phase }, task_ownership::Phase::Attached);
    PROOF_STAGE.store(2, Ordering::Release);
    serial_println!("[scheduler.proof.last_runnable.timer_wake] task={}", unsafe { (*task).id });
}

#[cfg(not(scheduler_wake_proof))]
pub fn proof_kernel_timer_wake() {}

#[cfg(scheduler_wake_proof)]
fn finish_last_runnable_proof(current: *mut Task) {
    if PROOF_STAGE.load(Ordering::Acquire) != 2 { return; }
    assert_eq!(PROOF_TASK.load(Ordering::Acquire), current);
    assert_eq!(SCHEDULERS[0].current_task.load(Ordering::Acquire), current);
    assert!(SCHEDULERS[0].pending_outgoing.load(Ordering::Acquire).is_null());
    let state = unsafe { (*current).state.snapshot() };
    assert_eq!(state.phase, task_ownership::Phase::Attached);
    assert!(!state.blocked && !state.pending);
    let count = PROOF_HELD_COUNT.load(Ordering::Acquire) as usize;
    for slot in &PROOF_HELD[..count] {
        let token = slot.swap(ptr::null_mut(), Ordering::AcqRel);
        assert!(!token.is_null());
        // Return the already-owned token; a second admission would be a bug.
        let restored = SCHEDULERS.iter().any(|sched| sched.runqueue.try_push(token).is_ok());
        assert!(restored, "PROOF_TOKEN_RESTORE_FAILED");
    }
    PROOF_STAGE.store(3, Ordering::Release);
    serial_println!("[scheduler.proof.last_runnable.pass] task={} restored={} ownership=preserved", unsafe { (*current).id }, count);
}

#[cfg(not(scheduler_wake_proof))]
fn finish_last_runnable_proof(_current: *mut Task) {}

/// Called only by switch_to after its RSP pivot, before user PKRU restoration.
/// No context-to-Task cast is valid: Task's Rust layout is intentionally opaque.
#[no_mangle]
pub extern "C" fn scheduler_finish_switch() {
    let core = crate::core_local::CoreLocal::get().core_id;
    let sched = &SCHEDULERS[core as usize];
    let old = sched.pending_outgoing.swap(ptr::null_mut(), Ordering::AcqRel);
    if old.is_null() { return; }
    unsafe {
        let rsp: u64;
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nostack, preserves_flags));
        let top = (*old).kstack_top;
        assert!(rsp < top - 65536 || rsp >= top, "OUTGOING_STACK_NOT_RELEASED");
        let frame = (*old).context.kstack_top;
        assert!(frame >= top - 65536 && frame <= top - 168, "OUTGOING_FRAME_OUTSIDE_STACK");
        log_wake_event("post_stack_release", old);
        assert!((*old).state.release_after_stack_switch(core), "OUTGOING_CUSTODY_MISMATCH");
        // After release, access only immutable fields or the atomic protocol.
        // Another lane may immediately acquire and rewrite the saved context.
        sched.admit_task(old);
        poll_paging_exit_notifications();
    }
}

/// Preserves the attached owner until a real replacement exists or its pending
/// event can resume it locally. The caller has already saved the user frame.
/// IRQs are enabled only around HLT; every state/selection check runs with IF=0.
pub unsafe fn wait_for_local_resume_or_switch(sched: &Scheduler) -> *const TaskContext {
    loop {
        x86_64::instructions::interrupts::disable();
        let current = sched.current_task.load(Ordering::Acquire);
        if !current.is_null() && (*current).state.resume_attached(sched.core_id) {
            log_wake_event("local_resume", current);
            finish_last_runnable_proof(current);
            return &(*current).context;
        }
        match sched.tick() {
            ScheduleDecision::Switch { next, .. } => return next,
            ScheduleDecision::NoRunnable => {}
        }
        // Timer interrupts deliberately do not schedule kernel frames. They
        // return here, where pending wakes and retained admissions are retried.
        x86_64::instructions::interrupts::enable_and_hlt();
    }
}

/// Self-only inbox wait used by syscall 53. Publication performs a pending wake,
/// closing the empty-observation/park race without dequeuing the caller's data.
pub unsafe fn wait_for_message(regs_ptr: *const crate::interrupts::SyscallRegs) {
    x86_64::instructions::interrupts::disable();
    let core = crate::core_local::CoreLocal::get();
    let sched = &SCHEDULERS[core.core_id as usize];
    let current = sched.current_task.load(Ordering::Acquire);
    if current.is_null() { return; }
    let pd = core.current_pd_ref();
    if (*pd.message_ring).len() != 0 || !pd.incoming_replies.lock().is_empty() { return; }
    match (*current).state.park(core.core_id) {
        ParkResult::WakeConsumed => return,
        ParkResult::Blocked => {
            (*current).resume_observation_pending.store(true, Ordering::Release);
            log_wake_event("park", current);
            begin_last_runnable_proof(sched, current);
            yield_and_switch(regs_ptr);
        }
        ParkResult::Exited => yield_and_switch(regs_ptr),
        ParkResult::NotOwner => panic!("WAIT_WITHOUT_ATTACHED_OWNER"),
    }
}

pub fn park_current_thread() {
    let core_id = crate::core_local::CoreLocal::get().core_id;
    let sched = &SCHEDULERS[core_id as usize];
    let current = sched.current_task.load(Ordering::Acquire);
    if !current.is_null() {
        unsafe { (*current).state.park(core_id); }
    }
}

pub fn unpark_thread(task_ptr: *mut Task) {
    if task_ptr.is_null() { return; }
    if unsafe { (*task_ptr).state.wake() } == WakeResult::NeedsAdmission {
        // Only Detached can claim admission. Attached/SwitchingOut wakes stay
        // pending for their owner and cannot duplicate execution custody.
        let core = crate::core_local::CoreLocal::get().core_id as usize;
        let result = SCHEDULERS[core].admit_task(task_ptr);
        if result == Admission::Published {
            log_wake_event("detached_wake", task_ptr);
        }
    }
}

/// # Safety
/// A capability-authorized pager has committed the exact pending mapping and
/// retired its unique fault token. This operation never edits the saved frame.
pub unsafe fn resume_resolved_fault(task: *mut Task) -> bool {
    if task.is_null() { return false; }
    (*task).resume_observation_pending.store(true, Ordering::Release);
    if !(*task).state.complete_fault() { return false; }
    let core = crate::core_local::CoreLocal::get().core_id as usize;
    // Admission itself rejects Attached/SwitchingOut; final stack release or
    // the attached wait loop remains responsible for those exact owners.
    SCHEDULERS[core].admit_task(task);
    true
}

pub fn bootstrap_enqueue_all_domains() -> usize {
    serial_println!("scheduler.bootstrap_enqueue.begin");
    let sched = &SCHEDULERS[0];
    let mut count = 0usize;

    for pd_id in 0..crate::ipc::MAX_DOMAINS as u32 {
        if let Some(pd) = crate::ipc::DOMAIN_REGISTRY.get(pd_id) {
            let task_ptr = pd.main_task.load(Ordering::Acquire);
            if task_ptr.is_null() {
                continue;
            }
            if !sched.activate_task(task_ptr) { continue; }
            count += 1;
            unsafe {
                serial_println!("scheduler.enqueue pd_id={} task={:#x}", pd.id, task_ptr as u64);
                serial_println!(
                    "task.created id={} pd_id={} rip={:#x} rsp={:#x}",
                    (*task_ptr).id,
                    (*task_ptr).context.pd_id,
                    (*task_ptr).context.rip,
                    (*task_ptr).context.rsp
                );
            }
        }
    }

    serial_println!("scheduler.bootstrap_enqueue.done count={}", count);
    count
}

pub fn log_first_scheduled_pd(pd_id: u64) {
    if FIRST_SCHEDULE_LOGGED
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        serial_println!("first scheduled pd_id={}", pd_id);
    }
}

#[inline]
fn is_canonical(addr: u64) -> bool {
    let sign = (addr >> 47) & 1;
    let high = addr >> 48;
    (sign == 0 && high == 0) || (sign == 1 && high == 0xFFFF)
}

pub unsafe fn debug_dump_iret_frame(next_ctx: *const TaskContext) {
    // switch_to pops 15 GPR qwords + 1 dummy error, then iretq reads RIP/CS/RFLAGS/RSP/SS.
    let iret_rsp = ((*next_ctx).kstack_top + (16 * 8)) as *const u64;
    let rip = *iret_rsp.add(0);
    let cs = *iret_rsp.add(1);
    let rflags = *iret_rsp.add(2);
    let rsp = *iret_rsp.add(3);
    let ss = *iret_rsp.add(4);

    serial_println!(
        "iret.frame.qwords rsp={:#x} [0]={:#x} [1]={:#x} [2]={:#x} [3]={:#x} [4]={:#x}",
        iret_rsp as u64, rip, cs, rflags, rsp, ss
    );
    serial_println!(
        "iret.frame.check rip_canon={} rsp_canon={} rflags.bit1={} rflags.if={} cs={:#x} ss={:#x}",
        is_canonical(rip),
        is_canonical(rsp),
        (rflags & 0x2) != 0,
        (rflags & 0x200) != 0,
        cs,
        ss
    );
}

pub unsafe fn debug_dump_user_entry_bytes(next_ctx: *const TaskContext) {
    let rip = (*next_ctx).rip as *const u8;
    let mut bytes = [0u8; 32];
    core::ptr::copy_nonoverlapping(rip, bytes.as_mut_ptr(), bytes.len());

    serial_println!(
        "user.entry.bytes rip={:#x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}",
        rip as u64,
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        bytes[16], bytes[17], bytes[18], bytes[19], bytes[20], bytes[21], bytes[22], bytes[23],
        bytes[24], bytes[25], bytes[26], bytes[27], bytes[28], bytes[29], bytes[30], bytes[31]
    );
}

/// Legacy hint only. Actual yield requires a saved user frame and the unified
/// switch boundary; it must never publish a still-attached current task.
pub fn yield_now() { core::hint::spin_loop(); }

/// Cooperative yield + context switch for runtime reachability proof.
/// Saves current PD context from syscall stack, picks next runnable PD,
/// and switches to it. Never returns to caller.
/// Terminate only the currently attached task; no target ID or foreign authority.
/// The existing yield path saves its complete frame and releases stack custody.
pub unsafe fn finish_current_task(regs_ptr: *const crate::interrupts::SyscallRegs) -> ! {
    x86_64::instructions::interrupts::disable();
    let core=crate::core_local::CoreLocal::get();
    let task=SCHEDULERS[core.core_id as usize].current_task.load(Ordering::Acquire);
    assert!(!task.is_null(), "TASK_FINISH_WITHOUT_TASK");
    (*task).state.exit();
    serial_println!("[task.finish] pd={} task={} reason={} terminal=1",core.current_pd(),(*task).id,(*regs_ptr).rdi);
    poll_paging_exit_notifications();
    yield_and_switch(regs_ptr)
}

pub unsafe fn yield_and_switch(regs_ptr: *const crate::interrupts::SyscallRegs) -> ! {
    x86_64::instructions::interrupts::disable();
    let core_id = crate::core_local::CoreLocal::get().core_id as usize;
    let sched = &SCHEDULERS[core_id];
    let current = sched.current_task.load(Ordering::Acquire);
    if !current.is_null() {
        let ctx = &mut (*current).context;
        let base = regs_ptr as u64 + 72u64;

        let saved_r15   = *(base.wrapping_add(0)   as *const u64);
        let saved_r14   = *(base.wrapping_add(8)   as *const u64);
        let saved_r13   = *(base.wrapping_add(16)  as *const u64);
        let saved_r12   = *(base.wrapping_add(24)  as *const u64);
        let saved_rbx   = *(base.wrapping_add(32)  as *const u64);
        let saved_rbp   = *(base.wrapping_add(40)  as *const u64);
        let saved_pkru  = *(base.wrapping_add(48)  as *const u64);
        let saved_r9    = *(base.wrapping_add(56)  as *const u64);
        let saved_r8    = *(base.wrapping_add(64)  as *const u64);
        let saved_r10   = *(base.wrapping_add(72)  as *const u64);
        let saved_rdx   = *(base.wrapping_add(80)  as *const u64);
        let saved_rsi   = *(base.wrapping_add(88)  as *const u64);
        let saved_rdi   = *(base.wrapping_add(96)  as *const u64);
        let saved_rax   = (*regs_ptr).rax; // wait syscall resumes with its explicit result
        let user_rip    = *(base.wrapping_add(112) as *const u64);
        let user_rflags = *(base.wrapping_add(120) as *const u64);

        // User RSP is at base+128: pushed onto the per-task kernel stack at syscall_entry
        // before any other registers, so it survives timer-triggered context switches
        // that would clobber the shared gs:[24] scratch slot.
        let user_rsp = *(base.wrapping_add(128) as *const u64);
        // User SS is always 0x23 (GDT index 4, RPL 3: user data segment).
        let user_ss: u64 = 0x23u64;

        // (*current).kstack_top = kstack_alloc_top (Task struct field, fixed at alloc).
        // ctx.kstack_top = last saved ksp (changes each yield — NOT the stable top).
        // Using ctx.kstack_top here would drift ksp_base by -168 per yield → overflow ~97s.
        // LAYOUT DEPENDENCY: base = regs_ptr + 72 = rbp in syscall_entry's frame.
        // If syscall_entry push order changes, all base+N offsets below must be reaudited.
        let ksp_base = (*current).kstack_top.wrapping_sub(168);
        let ksp = ksp_base as *mut u64;
        let iret = ksp.add(16);
        *iret.add(0) = user_rip;
        *iret.add(1) = 0x2Bu64;
        *iret.add(2) = user_rflags;
        *iret.add(3) = user_rsp;
        *iret.add(4) = user_ss;
        *ksp.add(0)  = saved_rax;
        *ksp.add(1)  = saved_rbx;
        *ksp.add(2)  = user_rip;
        *ksp.add(3)  = saved_rdx;
        *ksp.add(4)  = saved_rbp;
        *ksp.add(5)  = saved_rsi;
        *ksp.add(6)  = saved_rdi;
        *ksp.add(7)  = saved_r8;
        *ksp.add(8)  = saved_r9;
        *ksp.add(9)  = saved_r10;
        *ksp.add(10) = user_rflags;
        *ksp.add(11) = saved_r12;
        *ksp.add(12) = saved_r13;
        *ksp.add(13) = saved_r14;
        *ksp.add(14) = saved_r15;
        *ksp.add(15) = 0u64;
        ctx.r15 = saved_r15; ctx.r14 = saved_r14; ctx.r13 = saved_r13; ctx.r12 = saved_r12;
        ctx.r11 = user_rflags; ctx.r10 = saved_r10; ctx.r9 = saved_r9; ctx.r8 = saved_r8;
        ctx.rdi = saved_rdi; ctx.rsi = saved_rsi; ctx.rbp = saved_rbp; ctx.rdx = saved_rdx;
        ctx.rcx = user_rip; ctx.rbx = saved_rbx; ctx.rax = saved_rax;
        ctx.rip = user_rip; ctx.cs = 0x2Bu64; ctx.rflags = user_rflags;
        ctx.rsp = user_rsp; ctx.ss = user_ss; ctx.pkru = saved_pkru;
        ctx.kstack_top = ksp_base;
        let yield_n = SCHED_YIELD_SAVED_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if yield_n <= 4 {
            serial_println!("scheduler.yield_and_switch.saved pd_id={}", ctx.pd_id);
        } else if yield_n & (yield_n - 1) == 0 {
            serial_println!("[perf.noise.summary] name=scheduler.yield count={} suppressed={}",
                yield_n, yield_n - 4);
        }

    }
    let next = match sched.tick() {
        ScheduleDecision::Switch { next, .. } => next,
        ScheduleDecision::NoRunnable => wait_for_local_resume_or_switch(sched),
    };
    {
        let kstack_top = (*next).kstack_top;
        // SCHEDULER_TICK_PD8_PF_FLAKE_V1 root cause: `next` is *const TaskContext,
        // so kstack_top here is ctx.kstack_top = saved-frame BASE (rax slot), the
        // same field the timer/page-fault paths take +168 on. (The previous
        // comment believed this was Task.kstack_top/alloc-top — type confusion;
        // TaskContext sits at Task offset 0, so the pointers alias numerically.)
        // Without +168, RSP0 lands at the frame base: the next ring3 interrupt
        // pushes from there and re-saves the frame 168 bytes lower, permanently.
        // Each yield-path dispatch of a preempted task ratcheted its kstack down
        // ~168 bytes until pushes ran off the 64 KiB allocation into the heap
        // neighbor below (adjacent Task structs — the pd_ptr=0/pointer-spray
        // corruption captured by hardware watchpoint on quil's Task).
        // RSP0 must be the frame TOP, identical to the timer path.
        crate::gdt::update_tss_rsp0(x86_64::VirtAddr::new(kstack_top + 168));
        static FIX_MARKER: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
        if !FIX_MARKER.swap(true, Ordering::Relaxed) {
            serial_println!("[scheduler.pd8.flake.fix.ok] reason=yield_rsp0_frame_base_ratchet");
        }
        unsafe {
            Scheduler::switch_to(core::ptr::null_mut(), next);
        }
    }
}
