pub const KERNEL: u8 = 0;
pub const SEXDISPLAY: u8 = 1;
pub const SEXDRIVE: u8 = 2;
pub const SEXFILES: u8 = 3;
pub const SILK: u8 = 4;
pub const SHARED: u8 = 14;
pub const UNTRUSTED: u8 = 15;

use core::sync::atomic::{AtomicU8, Ordering};

/// Runtime gate used by naked interrupt/syscall assembly paths.
/// 1 => rdpkru/wrpkru instructions are valid on this CPU.
#[unsafe(no_mangle)]
pub static PKU_ENABLED: AtomicU8 = AtomicU8::new(0);

/// Checks if the CPU supports Memory Protection Keys (PKU).
pub fn is_pku_supported() -> bool {
    use raw_cpuid::CpuId;
    let cpuid = CpuId::new();
    if let Some(features) = cpuid.get_extended_feature_info() {
        features.has_pku()
    } else {
        false
    }
}

/// Enables Memory Protection Keys (PKU) in the CR4 register.
pub unsafe fn enable_pku() {
    use x86_64::registers::control::{Cr4, Cr4Flags};
    let mut flags = Cr4::read();
    flags.insert(Cr4Flags::PROTECTION_KEY_USER);
    Cr4::write(flags);
    crate::serial_println!("PKU: Protection Keys enabled in CR4.");
}

#[inline(always)]
pub fn set_pku_enabled(enabled: bool) {
    PKU_ENABLED.store(enabled as u8, Ordering::Release);
}

#[inline(always)]
pub unsafe fn rdpkru() -> u32 {
    if PKU_ENABLED.load(Ordering::Acquire) == 0 {
        return 0;
    }
    let pkru: u32;
    core::arch::asm!(
        "xor ecx, ecx",
        "rdpkru",
        out("eax") pkru,
        out("edx") _,
        out("ecx") _,
        options(nomem, nostack, preserves_flags)
    );
    pkru
}

#[inline(always)]
pub unsafe fn wrpkru(mask: u32) {
    if PKU_ENABLED.load(Ordering::Acquire) == 0 {
        let _ = mask;
        return;
    }
    core::arch::asm!(
        "xor ecx, ecx",
        "xor edx, edx",
        "wrpkru",
        in("eax") mask,
        out("ecx") _,
        out("edx") _,
        options(nomem, nostack)
    );
}

#[inline(always)]
pub unsafe fn set_pkru(pkru: u32) {
    wrpkru(pkru);
}

#[inline(always)]
pub unsafe fn get_pkru() -> u32 {
    rdpkru()
}

/// A wrapper around the PKRU register.
pub struct Pkru(pub u32);

/// The PKU Warden: reports hardware security violations.
pub fn pku_warden(fault_addr: u64, rip: u64, error_code: u64) {
    let pkru = unsafe { rdpkru() };
    let current_pd = crate::core_local::CoreLocal::get().current_pd();
    
    crate::serial_println!("------------------------------------------------------------");
    crate::serial_println!("🔥 HARDWARE SECURITY VIOLATION: PKU LOCK ENGAGED 🔥");
    crate::serial_println!("FAULT ADDR: {:#x}", fault_addr);
    crate::serial_println!("FAULT RIP:  {:#x}", rip);
    crate::serial_println!("CURRENT PD: {}", current_pd);
    crate::serial_println!("PKRU STATE: {:#010x}", pkru);

    let is_instr_fetch = (error_code & 0x10) != 0;
    let is_write = (error_code & 0x02) != 0;

    if is_instr_fetch {
        crate::serial_println!("VIOLATION: Illegal Domain Execute (Bit 4 set)");
    } else if is_write {
        crate::serial_println!("VIOLATION: ReadOnly Capability Violation (Bit 1 set)");
    } else {
        crate::serial_println!("VIOLATION: Access Denied (Read/Data)");
    }
    crate::serial_println!("------------------------------------------------------------");
}

pub fn multicast_revoke_key(_key: u8) {
    unsafe { crate::apic::send_ipi(0, 0x40, 0b11 << 18); }
    crate::hal::tlb_flush_local();
}

pub unsafe fn tag_virtual_address(va: u64, pkey: u8) {
    use x86_64::registers::control::Cr3;
    let hhdm_offset = crate::HHDM_REQUEST.response().unwrap().offset;
    let (pml4_phys, _) = Cr3::read();
    let pml4_virt = (pml4_phys.start_address().as_u64() + hhdm_offset) as *const u64;

    let pml4_idx = (va >> 39) & 0x1FF;
    let pdpt_idx = (va >> 30) & 0x1FF;
    let pd_idx   = (va >> 21) & 0x1FF;
    let pt_idx   = (va >> 12) & 0x1FF;

    let pml4e = *pml4_virt.add(pml4_idx as usize);
    if (pml4e & 1) == 0 { return; }
    let pdpt_virt = ((pml4e & 0xFFFF_FFFF_FFFF_F000) + hhdm_offset) as *const u64;

    // Read PDPTE
    let pdpte = *pdpt_virt.add(pdpt_idx as usize);
    if (pdpte & 1) == 0 { return; }

    // Check for 1GiB huge page (PS bit in PDPTE)
    if (pdpte & (1 << 7)) != 0 {
        let entry = pdpt_virt.add(pdpt_idx as usize) as *mut u64;
        let mut val = *entry;
        val &= !(0xF << 59);
        val |= (pkey as u64 & 0xF) << 59;
        *entry = val;
        core::arch::asm!("invlpg [{}]", in(reg) va, options(nostack, preserves_flags));
        return;
    }

    // Read PDE
    let pde = *pdpt_virt.add(pdpt_idx as usize);
    if (pde & 1) == 0 { return; }
    let pd_virt = ((pde & 0xFFFF_FFFF_FFFF_F000) + hhdm_offset) as *const u64;

    // Check for 2MiB huge page (PS bit in PDE)
    if (pde & (1 << 7)) != 0 {
        let entry = pd_virt.add(pd_idx as usize) as *mut u64;
        let mut val = *entry;
        val &= !(0xF << 59);
        val |= (pkey as u64 & 0xF) << 59;
        *entry = val;
        core::arch::asm!("invlpg [{}]", in(reg) va, options(nostack, preserves_flags));
        return;
    }

    // Read PTE (4KiB page)
    let pte_ptr = *pd_virt.add(pd_idx as usize);
    if (pte_ptr & 1) == 0 { return; }
    let pt_virt = ((pte_ptr & 0xFFFF_FFFF_FFFF_F000) + hhdm_offset) as *mut u64;

    let mut pte = *pt_virt.add(pt_idx as usize);
    pte &= !(0xF << 59);
    pte |= (pkey as u64 & 0xF) << 59;
    *pt_virt.add(pt_idx as usize) = pte;

    core::arch::asm!("invlpg [{}]", in(reg) va, options(nostack, preserves_flags));
}

/// Replaces the huge-page entry at `slot` (`size` bytes, 1 GiB or 2 MiB) with a table
/// of 512 entries mapping the same frames with the same flags: 2 MiB pages, or 4 KiB
/// ones, whose PAT bit is bit 7 instead of 12. Returns the new entry.
unsafe fn split_huge(slot: *mut u64, size: u64, va: u64) -> u64 {
    let hhdm = crate::HHDM_REQUEST.response().unwrap().offset;
    let entry = *slot;
    let (child, base) = (size >> 9, entry & 0x000F_FFFF_FFFF_F000 & !(size - 1));
    let mut flags = entry & !0x000F_FFFF_FFFF_F000;
    if child == 4096 { flags &= !(1 << 7); if entry & (1 << 12) != 0 { flags |= 1 << 7; } } else { flags |= entry & (1 << 12); }
    let table = crate::memory::allocator::GLOBAL_ALLOCATOR.alloc(0).expect("PKU_SPLIT_TABLE");
    let leaves = (table + hhdm) as *mut u64;
    for i in 0..512 { *leaves.add(i) = base + i as u64 * child | flags; }
    let new = table | 0x3;
    *slot = new;
    core::arch::asm!("invlpg [{0}]", in(reg) va, options(nostack, preserves_flags));
    new
}

/// Set USER_ACCESSIBLE, WRITABLE, and PKEY on the terminal page table entry
/// for the given virtual address. A 2MiB or 1GiB page holding it is split first.
/// Must be called from ring 0 (kernel context).
pub unsafe fn set_page_user_accessible(va: u64, pkey: u8) {
    use x86_64::registers::control::Cr3;
    let hhdm_offset = crate::HHDM_REQUEST.response().unwrap().offset;
    let (pml4_phys, _) = Cr3::read();
    let pml4_virt = (pml4_phys.start_address().as_u64() + hhdm_offset) as *mut u64;

    let pml4_idx = ((va >> 39) & 0x1FF) as usize;
    let pdpt_idx = ((va >> 30) & 0x1FF) as usize;
    let pd_idx   = ((va >> 21) & 0x1FF) as usize;
    let pt_idx   = ((va >> 12) & 0x1FF) as usize;

    // PML4 — skip if not present
    let mut pml4e = *pml4_virt.add(pml4_idx);
    if (pml4e & 1) == 0 { return; }
    pml4e |= 0x6;
    *pml4_virt.add(pml4_idx) = pml4e;

    // PDPT
    let pdpt_virt = ((pml4e & 0xFFFF_FFFF_FFFF_F000) + hhdm_offset) as *mut u64;
    let mut pdpte = *pdpt_virt.add(pdpt_idx);
    if (pdpte & 1) == 0 { return; }
    // A huge page is split first, before any bit changes: the user bit and key go on
    // this 4 KiB page only, never on whatever else shares the huge page (audit S17).
    if (pdpte & (1 << 7)) != 0 { pdpte = split_huge(pdpt_virt.add(pdpt_idx), 1 << 30, va); }
    pdpte |= 0x6;
    *pdpt_virt.add(pdpt_idx) = pdpte;

    // PD
    let pd_virt = ((pdpte & 0xFFFF_FFFF_FFFF_F000) + hhdm_offset) as *mut u64;
    let mut pde = *pd_virt.add(pd_idx);
    if (pde & 1) == 0 { return; }
    if (pde & (1 << 7)) != 0 { pde = split_huge(pd_virt.add(pd_idx), 1 << 21, va); }
    pde |= 0x6;
    *pd_virt.add(pd_idx) = pde;

    // PT (4KiB page)
    let pt_virt = ((pde & 0xFFFF_FFFF_FFFF_F000) + hhdm_offset) as *mut u64;
    let pte = *pt_virt.add(pt_idx);
    if (pte & 1) == 0 { return; }

    let mut entry = pte;
    entry |= 0x6 | (1 << 63);    // USER_ACCESSIBLE | WRITABLE | NO_EXECUTE
    entry &= !(0xF << 59);
    entry |= (pkey as u64 & 0xF) << 59;
    *pt_virt.add(pt_idx) = entry;
    core::arch::asm!("invlpg [{0}]", in(reg) va, options(nostack, preserves_flags));
}
