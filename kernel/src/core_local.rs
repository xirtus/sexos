use core::sync::atomic::{AtomicU32, AtomicPtr, AtomicBool, Ordering};
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
    pub syscall_msg_ptr: u64,
    pub pdx_ring: crate::ipc_ring::SpscRing<crate::ipc::messages::Message>,
}

impl CoreLocal {
    pub unsafe fn init(core_id: u32) {
        INITIALIZED_CORES.fetch_add(1, Ordering::AcqRel);
        use x86_64::structures::paging::PageTableFlags;
        use crate::memory::manager::GLOBAL_VAS;

        // Allocate a page for syscall message returns (accessible by userland)
        let msg_va = crate::memory::va_allocator::allocate_va(4096).expect("CoreLocal: VA alloc fail");
        {
            let mut vas = GLOBAL_VAS.lock();
            let vas_ref = vas.as_mut().expect("VAS not initialized");
            // Map with PKEY 15 for shared syscall buffer.
            vas_ref.map_pku_range(
                VirtAddr::new(msg_va),
                4096,
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE,
                15 // PKEY 15
            ).expect("CoreLocal: Map fail");
        }

        let mut core_local = alloc::boxed::Box::new(CoreLocal {
            current_pd_ptr: AtomicPtr::new(core::ptr::null_mut()),
            current_pd_id: AtomicU32::new(0),
            core_id,
            kernel_stack: 0,
            user_stack: 0,
            syscall_msg_ptr: msg_va,
            pdx_ring: crate::ipc_ring::SpscRing::new(),
        });
        
        // Allocate 64KB kernel stack
        let stack = alloc::vec![0u8; 65536];
        let stack_ptr = stack.as_ptr() as u64 + 65536;
        core_local.kernel_stack = stack_ptr;
        core::mem::forget(stack); // Leak stack for simplicity
        
        // Phase 32: Update TSS for Ring 3 -> Ring 0 transition safety
        crate::gdt::update_tss_rsp0(VirtAddr::new(stack_ptr));

        let ptr = alloc::boxed::Box::into_raw(core_local);
        GsBase::write(VirtAddr::from_ptr(ptr));
        x86_64::registers::model_specific::KernelGsBase::write(VirtAddr::from_ptr(ptr));
    }

    pub fn init_first_core(pd_ptr: *mut crate::capability::ProtectionDomain) {
        let core = Self::get();
        core.current_pd_ptr.store(pd_ptr, Ordering::Release);
        unsafe {
            core.current_pd_id.store((*pd_ptr).id, Ordering::Release);
        }
        INITIALIZED.store(true, Ordering::Release);
    }

    pub fn get() -> &'static CoreLocal {
        unsafe {
            &*(GsBase::read().as_ptr() as *const CoreLocal)
        }
    }

    pub fn get_mut() -> &'static mut CoreLocal {
        unsafe {
            &mut *(GsBase::read().as_u64() as *mut CoreLocal)
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
