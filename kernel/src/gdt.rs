use lazy_static::lazy_static;
use crate::serial_println;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;
use x86_64::instructions::segmentation::Segment;

pub const DOUBLE_FAULT_IST_INDEX: usize = 0;
pub static mut TSS: TaskStateSegment = TaskStateSegment::new();

lazy_static! {
    pub static ref GDT: GlobalDescriptorTable = {
        let mut gdt = GlobalDescriptorTable::new(); // auto-null at index 0

        // === EXACT SYSCALL/SYSRET LAYOUT (manual §15) ===
        gdt.append(Descriptor::kernel_code_segment());   // index 1 → 0x08  SYSCALL CS
        gdt.append(Descriptor::kernel_data_segment());   // index 2 → 0x10  SYSCALL SS
        gdt.append(Descriptor::user_code_segment());     // index 3 → 0x18  compat32 pad (SYSRET base)
        gdt.append(Descriptor::user_data_segment());     // index 4 → 0x20  SYSRET SS (user data)
        gdt.append(Descriptor::user_code_segment());     // index 5 → 0x28  SYSRET CS (user code 64-bit)
        // TSS takes indices 6+7 (two slots) → 0x30 / 0x38
        gdt.append(unsafe { Descriptor::tss_segment(&TSS) });

        gdt
    };
}

/// This core's TSS: the static one until this core's CoreLocal says otherwise.
fn current_tss() -> *mut TaskStateSegment {
    let gs = x86_64::registers::model_specific::GsBase::read().as_u64();
    let local = if gs == 0 { core::ptr::null_mut() } else { unsafe { (*(gs as *const crate::core_local::CoreLocal)).tss } };
    if local.is_null() { core::ptr::addr_of_mut!(TSS) } else { local }
}

pub fn update_tss_rsp0(addr: VirtAddr) {
    unsafe {
        (*current_tss()).privilege_stack_table[0] = addr;
        if crate::core_local::INITIALIZED.load(core::sync::atomic::Ordering::Acquire) {
            crate::core_local::CoreLocal::get_mut().kernel_stack = addr.as_u64();
        }
    }
}
pub struct Selectors {
    pub kernel_cs: SegmentSelector,  // 0x08
    pub kernel_ss: SegmentSelector,  // 0x10
    pub user_cs:   SegmentSelector,  // 0x28
    pub user_ss:   SegmentSelector,  // 0x20
    pub tss:       SegmentSelector,  // 0x30
}

pub fn get_selectors() -> Selectors {
    Selectors {
        kernel_cs: SegmentSelector::new(1, x86_64::PrivilegeLevel::Ring0),
        kernel_ss: SegmentSelector::new(2, x86_64::PrivilegeLevel::Ring0),
        user_cs:   SegmentSelector::new(5, x86_64::PrivilegeLevel::Ring3),
        user_ss:   SegmentSelector::new(4, x86_64::PrivilegeLevel::Ring3),
        tss:       SegmentSelector::new(6, x86_64::PrivilegeLevel::Ring0),
    }
}

pub fn debug_dump_user_selectors() {
    let gdt = unsafe { &*GDT };
    let entries = gdt.entries();
    let user_data = entries.get(4).map(|e| e.raw()).unwrap_or(0);
    let user_code = entries.get(5).map(|e| e.raw()).unwrap_or(0);
    serial_println!("gdt.user.sel ss=0x23 raw={:#018x}", user_data);
    serial_println!("gdt.user.sel cs=0x2b raw={:#018x}", user_code);
}

pub fn debug_tss_rsp0() -> u64 {
    unsafe { (*current_tss()).privilege_stack_table[0].as_u64() }
}

/// A started core's own GDT and TSS. A TSS is marked busy when loaded, so no two
/// cores can share one, and each needs its own stack for double faults.
pub fn init_core() -> *mut TaskStateSegment {
    use x86_64::instructions::segmentation::{CS, DS, ES, FS, GS, SS};
    use x86_64::instructions::tables::load_tss;
    let stack = alloc::vec![0u8; 32 * 1024].leak();
    let mut tss = TaskStateSegment::new();
    // The IDT asks for IST index + 1.
    tss.interrupt_stack_table[DOUBLE_FAULT_IST_INDEX + 1] = VirtAddr::from_ptr(stack.as_ptr()) + stack.len() as u64;
    let tss: &'static mut TaskStateSegment = alloc::boxed::Box::leak(alloc::boxed::Box::new(tss));
    let pointer = tss as *mut TaskStateSegment;
    let mut gdt = GlobalDescriptorTable::new();
    gdt.append(Descriptor::kernel_code_segment());
    gdt.append(Descriptor::kernel_data_segment());
    gdt.append(Descriptor::user_code_segment());
    gdt.append(Descriptor::user_data_segment());
    gdt.append(Descriptor::user_code_segment());
    gdt.append(unsafe { Descriptor::tss_segment(&*pointer) });
    let gdt: &'static GlobalDescriptorTable = alloc::boxed::Box::leak(alloc::boxed::Box::new(gdt));
    gdt.load();
    let selectors = get_selectors();
    unsafe {
        CS::set_reg(selectors.kernel_cs);
        SS::set_reg(selectors.kernel_ss);
        DS::set_reg(selectors.kernel_ss);
        ES::set_reg(selectors.kernel_ss);
        FS::set_reg(selectors.kernel_ss);
        GS::set_reg(selectors.kernel_ss);
        load_tss(selectors.tss);
    }
    pointer
}

static mut DOUBLE_FAULT_STACK: [u8; 32 * 1024] = [0; 32 * 1024];

pub fn init() {
    use x86_64::instructions::segmentation::{CS, DS, ES, FS, GS, SS};
    use x86_64::instructions::tables::load_tss;
    use x86_64::structures::DescriptorTablePointer;

    // The IDT asks for IST index + 1. Before the GDT and TSS are loaded.
    unsafe {
        let top = core::ptr::addr_of!(DOUBLE_FAULT_STACK) as u64 + 32 * 1024;
        (*core::ptr::addr_of_mut!(TSS)).interrupt_stack_table[DOUBLE_FAULT_IST_INDEX + 1] = VirtAddr::new(top);
    }

    // Load GDT
    unsafe {
        let gdt_ptr = &*GDT;
        let gdtr = DescriptorTablePointer {
            limit: gdt_ptr.limit(),
            base: VirtAddr::new(gdt_ptr.entries().as_ptr() as u64),
        };
        x86_64::instructions::tables::lgdt(&gdtr);
    }

    // Reload segment registers
    let selectors = get_selectors();
    unsafe {
        CS::set_reg(selectors.kernel_cs);
        SS::set_reg(selectors.kernel_ss);
        DS::set_reg(selectors.kernel_ss);
        ES::set_reg(selectors.kernel_ss);
        FS::set_reg(selectors.kernel_ss);
        GS::set_reg(selectors.kernel_ss);

        // Load TSS
        load_tss(selectors.tss);
    }
}
