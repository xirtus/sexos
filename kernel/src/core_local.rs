use core::sync::atomic::{AtomicU32, AtomicU64, AtomicPtr, AtomicBool, Ordering};
use x86_64::registers::model_specific::GsBase;
use x86_64::VirtAddr;

pub static INITIALIZED_CORES: AtomicU32 = AtomicU32::new(0);

pub static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Per-core local data for SexOS.
#[repr(C)]
pub struct CoreLocal {
    pub current_pd_ptr: AtomicPtr<crate::capability::ProtectionDomain>,
    pub current_pd_id: AtomicU32, // Retained for debug/metadata where ID is strictly needed, but NOT for execution gating
    pub core_id: u32,
    pub kernel_stack: u64,
    pub user_stack: u64,
    /// Pointer to this CoreLocal struct (gs:[32]) so `CoreLocal::get()` is one
    /// load instead of an `RDMSR IA32_GS_BASE` on every syscall and switch.
    pub self_ptr: *const CoreLocal,
    /// Last IA32_FS_BASE written on this core (gs:[40]); `switch_to` skips WRMSR when unchanged.
    pub last_fs_base: AtomicU64,
    /// Last IA32_TSC_AUX written on this core (gs:[48]); `switch_to` skips WRMSR when unchanged.
    pub last_gate_aux: AtomicU64,
    /// Active protection view index on this core (`u32::MAX` before first `views::enter`).
    pub active_view: AtomicU32,
    pub _pad_iret: u32,
    /// Per-core last IRET frame debug snapshot (gs:[64..112]: rsp, rip, cs, rflags, user_rsp, ss),
    /// replacing global statics to eliminate SMP cache-line bouncing on every switch.
    pub last_iret: [u64; 6],
    pub pdx_ring: crate::ipc_ring::SpscRing<crate::ipc::messages::Message>,
    /// This core's own TSS, or null for the boot core's static one.
    pub tss: *mut x86_64::structures::tss::TaskStateSegment,
    /// This core's own stack, for when no task owns the core.
    pub idle_stack: u64,
}

const _: () = assert!(core::mem::offset_of!(CoreLocal, kernel_stack) == 16);
const _: () = assert!(core::mem::offset_of!(CoreLocal, user_stack) == 24);
const _: () = assert!(core::mem::offset_of!(CoreLocal, self_ptr) == 32);
const _: () = assert!(core::mem::offset_of!(CoreLocal, last_fs_base) == 40);
const _: () = assert!(core::mem::offset_of!(CoreLocal, last_gate_aux) == 48);
const _: () = assert!(core::mem::offset_of!(CoreLocal, active_view) == 56);
const _: () = assert!(core::mem::offset_of!(CoreLocal, last_iret) == 64);

impl CoreLocal {
    pub unsafe fn init(core_id: u32) { Self::init_with_tss(core_id, core::ptr::null_mut()) }

    pub unsafe fn init_with_tss(core_id: u32, tss: *mut x86_64::structures::tss::TaskStateSegment) {
        INITIALIZED_CORES.fetch_add(1, Ordering::AcqRel);

        let mut core_local = alloc::boxed::Box::new(CoreLocal {
            current_pd_ptr: AtomicPtr::new(core::ptr::null_mut()),
            current_pd_id: AtomicU32::new(0),
            core_id,
            kernel_stack: 0,
            user_stack: 0,
            self_ptr: core::ptr::null(),
            last_fs_base: AtomicU64::new(0),
            last_gate_aux: AtomicU64::new(0),
            active_view: AtomicU32::new(u32::MAX),
            _pad_iret: 0,
            last_iret: [0; 6],
            pdx_ring: crate::ipc_ring::SpscRing::new(),
            tss,
            idle_stack: 0,
        });
        
        // Allocate 64KB kernel stack
        let stack = alloc::vec![0u8; 65536];
        let stack_ptr = stack.as_ptr() as u64 + 65536;
        core_local.kernel_stack = stack_ptr;
        core_local.idle_stack = stack_ptr;
        core::mem::forget(stack); // Leak stack for simplicity
        
        let ptr = alloc::boxed::Box::into_raw(core_local);
        (*ptr).self_ptr = ptr;
        GsBase::write(VirtAddr::from_ptr(ptr));
        x86_64::registers::model_specific::KernelGsBase::write(VirtAddr::from_ptr(ptr));

        // Phase 32: Update TSS for Ring 3 -> Ring 0 transition safety. After GS, so
        // the write reaches this core's TSS and no other's.
        crate::gdt::update_tss_rsp0(VirtAddr::new(stack_ptr));
    }

    pub fn init_first_core(pd_ptr: *mut crate::capability::ProtectionDomain) {
        let core = Self::get();
        core.current_pd_ptr.store(pd_ptr, Ordering::Release);
        unsafe {
            core.current_pd_id.store((*pd_ptr).id, Ordering::Release);
        }
        INITIALIZED.store(true, Ordering::Release);
    }

    #[inline(always)]
    pub fn get() -> &'static CoreLocal {
        let ptr: *const CoreLocal;
        unsafe {
            core::arch::asm!("mov {}, gs:[32]", out(reg) ptr, options(nostack, pure, readonly, preserves_flags));
            &*ptr
        }
    }

    /// This core's state, or none before it is set up (a stop early in boot).
    pub fn try_get() -> Option<&'static CoreLocal> {
        let base = GsBase::read().as_u64();
        if base == 0 { None } else { Some(unsafe { &*(base as *const CoreLocal) }) }
    }

    #[inline(always)]
    pub fn get_mut() -> &'static mut CoreLocal {
        let ptr: *mut CoreLocal;
        unsafe {
            core::arch::asm!("mov {}, gs:[32]", out(reg) ptr, options(nostack, pure, readonly, preserves_flags));
            &mut *ptr
        }
    }

    pub fn current_pd(&self) -> u32 {
        self.current_pd_id.load(Ordering::Acquire)
    }

    pub fn set_pd(&self, pd_ptr: *mut crate::capability::ProtectionDomain) {
        // SCHEDULER_TICK_PD8_PF_FLAKE_V1: a stolen task carried a NULL
        // context.pd_ptr; the unconditional (*pd_ptr).id read below was the
        // kernel #PF at addr 0x58 inside Scheduler::tick. Refuse the bind
        // and keep the previous PD binding instead of faulting.
        if pd_ptr.is_null() {
            crate::serial_println!("[sched.set_pd.null] refused core={}", self.core_id);
            return;
        }
        self.current_pd_ptr.store(pd_ptr, Ordering::Release);
        unsafe {
            self.current_pd_id.store((*pd_ptr).id, Ordering::Release);
        }
    }

    pub fn current_pd_ref(&self) -> &'static crate::capability::ProtectionDomain {
        let ptr = self.current_pd_ptr.load(Ordering::Acquire);
        if ptr.is_null() {
            return crate::ipc::DOMAIN_REGISTRY.get(0).expect("KERNEL PD MISSING");
        }
        unsafe { &*ptr }
    }
}

pub fn get_pd() -> u32 {
    crate::core_local::CoreLocal::get().current_pd()
}

pub fn validate_core_state() {
    let core = crate::core_local::CoreLocal::get();
    let ptr = core.current_pd_ptr.load(Ordering::Acquire);
    assert!(!ptr.is_null(), "CORELOCAL_PTR_NULL");
    let expected_pkru = unsafe { (*ptr).current_pkru_mask.load(Ordering::Acquire) };
    assert_eq!(expected_pkru, unsafe { crate::pku::rdpkru() }, "CORELOCAL_PKU_DESYNC");
}
