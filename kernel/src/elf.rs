use crate::memory::manager::GlobalVas;
use x86_64::{VirtAddr, structures::paging::PageTableFlags};
use crate::serial_println;

/// Minimal ELF64 Header structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ElfHeader {
    pub magic: [u8; 4],
    pub class: u8,
    pub data: u8,
    pub version: u8,
    pub osabi: u8,
    pub abiversion: u8,
    pub pad: [u8; 7],
    pub elf_type: u16,
    pub machine: u16,
    pub version2: u32,
    pub entry: u64,
    pub phoff: u64,
    pub shoff: u64,
    pub flags: u32,
    pub ehsize: u16,
    pub phentsize: u16,
    pub phnum: u16,
    pub shentsize: u16,
    pub shnum: u16,
    pub shstrndx: u16,
}

/// Minimal ELF64 Program Header structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ProgramHeader {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

/// ELF64 Dynamic Entry structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DynamicEntry {
    pub d_tag: u64,
    pub d_val: u64,
}

// Dynamic tags relevant to relocation
const DT_NULL: u64 = 0;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const DT_RELACOUNT: u64 = 0x6ffffff9;

/// ELF64 RELA relocation entry.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RelaEntry {
    pub r_offset: u64,   // Location to apply relocation (virtual address)
    pub r_info: u64,     // Type and symbol index
    pub r_addend: i64,   // Constant addend
}

const R_X86_64_RELATIVE: u64 = 8;

/// Get the type from a rela info field (lower 32 bits on x86_64)
fn rela_type(info: u64) -> u64 {
    info & 0xffffffff
}

fn load_bias(elf_data: &[u8]) -> Result<u64, &'static str> {
    let header = unsafe { &*(elf_data.as_ptr() as *const ElfHeader) };
    let ph_start = header.phoff as usize;
    let ph_count = header.phnum as usize;
    let ph_size = header.phentsize as usize;
    let mut min_vaddr = u64::MAX;

    for i in 0..ph_count {
        let ph_ptr = unsafe {
            let offset = ph_start + (i * ph_size);
            elf_data.as_ptr().add(offset) as *const ProgramHeader
        };
        let ph = unsafe { &*ph_ptr };
        if ph.p_type == PT_LOAD && ph.p_memsz > 0 {
            if ph.p_vaddr < min_vaddr {
                min_vaddr = ph.p_vaddr;
            }
        }
    }

    if min_vaddr == u64::MAX {
        return Err("ELF: no PT_LOAD segments");
    }
    Ok(min_vaddr)
}

pub fn load_elf_for_pd(elf_data: &[u8], vas: &mut GlobalVas, pku_key: u8, load_base: VirtAddr) -> Result<VirtAddr, &'static str> {
    // 1. Validate ELF magic
    let header = unsafe { &*(elf_data.as_ptr() as *const ElfHeader) };
    if header.magic != [0x7f, b'E', b'L', b'F'] {
        return Err("ELF: Invalid magic number");
    }

    let min_vaddr = load_bias(elf_data)?;
    serial_println!(
        "ELF: Valid header. Entry point: {:#x} min_vaddr={:#x}",
        header.entry,
        min_vaddr
    );

    // 2. Iterate through program headers
    let ph_start = header.phoff as usize;
    let ph_count = header.phnum as usize;
    let ph_size = header.phentsize as usize;

    for i in 0..ph_count {
        let ph_ptr = unsafe {
            let offset = ph_start + (i * ph_size);
            elf_data.as_ptr().add(offset) as *const ProgramHeader
        };
        let ph = unsafe { &*ph_ptr };

        if ph.p_type == PT_LOAD {
            let relocated = ph.p_vaddr.saturating_sub(min_vaddr);
            let segment_vaddr = load_base + relocated;
            serial_println!("ELF: Loading segment: vaddr={:#x}, memsz={:#x} (Key: {})", 
                segment_vaddr.as_u64(), ph.p_memsz, pku_key);

            // Stage-1 mapping for kernel copy path: supervisor writable.
            // Keep USER at stage-1 so upper-level page-table entries are user-capable.
            // x86 requires U/S permission on every level for CPL3 fetch/access.
            let map_flags =
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
            // Final user-visible segment policy.
            let mut final_flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
            if (ph.p_flags & PF_W) != 0 {
                final_flags |= PageTableFlags::WRITABLE;
            }
            if (ph.p_flags & PF_X) == 0 {
                final_flags |= PageTableFlags::NO_EXECUTE;
            }
            let page_count = (ph.p_memsz + 4095) / 4096;
            serial_println!(
                "ELF: PT_LOAD idx={} vstart={:#x} vend={:#x} filesz={:#x} memsz={:#x} flags=R{}W{}X{} pages={}",
                i,
                segment_vaddr.as_u64(),
                segment_vaddr.as_u64().saturating_add(ph.p_memsz),
                ph.p_filesz,
                ph.p_memsz,
                if (ph.p_flags & PF_R) != 0 { "+" } else { "-" },
                if (ph.p_flags & PF_W) != 0 { "+" } else { "-" },
                if (ph.p_flags & PF_X) != 0 { "+" } else { "-" },
                page_count
            );
            serial_println!(
                "loader.map relocated old_va={:#x} -> new_va={:#x} flags={:?}",
                ph.p_vaddr,
                segment_vaddr.as_u64(),
                final_flags
            );
            
            serial_println!("   → Mapping range...");
            vas.map_pku_range(segment_vaddr, ph.p_memsz, map_flags, pku_key)?;
            serial_println!("   → Mapping complete. Copying data to {:#x}...", segment_vaddr.as_u64());

            let dest = segment_vaddr.as_mut_ptr::<u8>();
            let src_offset = ph.p_offset as usize;
            let src = &elf_data[src_offset..src_offset + ph.p_filesz as usize];
            
            unsafe {
                core::ptr::copy_nonoverlapping(src.as_ptr(), dest, ph.p_filesz as usize);
                serial_println!("   → Copy complete.");
                if ph.p_memsz > ph.p_filesz {
                    serial_println!("   → Zeroing BSS...");
                    core::ptr::write_bytes(dest.add(ph.p_filesz as usize), 0, (ph.p_memsz - ph.p_filesz) as usize);
                    serial_println!("   → BSS zeroed.");
                }
            }

            {
                use x86_64::structures::paging::{Mapper, Page, Size4KiB};
                let start_page = Page::<Size4KiB>::containing_address(segment_vaddr);
                let end_page = Page::<Size4KiB>::containing_address(
                    segment_vaddr + ph.p_memsz.saturating_sub(1u64),
                );
                for page in Page::range_inclusive(start_page, end_page) {
                    unsafe {
                        vas.mapper
                            .update_flags(page, PageTableFlags::from_bits_retain(crate::app_resource_state::final_elf_flags(final_flags.bits(), pku_key)))
                            .map_err(|_| "ELF: update_flags failed")?
                            .flush();
                    }
                }
                serial_println!("   → Finalized segment flags={:?}", final_flags);
            }

            if segment_vaddr.as_u64() <= 0x4000_0000
                && 0x4000_0000 < segment_vaddr.as_u64().saturating_add(ph.p_memsz)
            {
                match crate::memory::manager::read_pte_flags(VirtAddr::new(0x4000_0000)) {
                    Ok(pte) => {
                        serial_println!("ELF: final PTE @0x40000000 = {:#x}", pte);
                    }
                    Err(e) => {
                        serial_println!("ELF: final PTE @0x40000000 unavailable: {}", e);
                    }
                }
            }
        }
    }

    // 3. Apply R_X86_64_RELATIVE relocations from PT_DYNAMIC.
    //    PIE binaries store function pointers and data addresses in GOT/data sections
    //    that must be rebased to the runtime load_base. Without this, any call through
    //    an unrelocated GOT entry (vtable, trait method, etc.) jumps to a wrong address.
    //    The rela offset field is the virtual address (pre-relocation) of the 8-byte
    //    slot to patch; each slot receives load_base + addend.
    //    Offsets in BSS (beyond p_filesz) are zeroed during load and must be set here.
    for i in 0..ph_count {
        let ph_ptr = unsafe {
            let offset = ph_start + (i * ph_size);
            elf_data.as_ptr().add(offset) as *const ProgramHeader
        };
        let ph = unsafe { &*ph_ptr };
        if ph.p_type == PT_DYNAMIC {
            let dyn_vaddr = ph.p_vaddr;
            let dyn_size = ph.p_memsz;
            let mut rel_offset: u64 = 0;
            let mut rel_size: u64 = 0;
            let mut rel_ent: u64 = 24; // default RELA entry size
            let mut rel_count: u64 = 0;

            // Parse dynamic entries to find relocation metadata.
            let dyn_base = elf_data.as_ptr();
            let dyn_file_off = ph.p_offset as usize;
            let num_dyn = dyn_size / 16;
            for j in 0..num_dyn {
                let entry_ptr = unsafe {
                    dyn_base.add(dyn_file_off + (j * 16) as usize) as *const DynamicEntry
                };
                let entry = unsafe { &*entry_ptr };
                match entry.d_tag {
                    DT_RELA => {
                        rel_offset = entry.d_val; // virtual address of RELA table
                    }
                    DT_RELASZ => {
                        rel_size = entry.d_val;
                    }
                    DT_RELAENT => {
                        rel_ent = entry.d_val;
                    }
                    DT_RELACOUNT => {
                        rel_count = entry.d_val;
                    }
                    DT_NULL => {
                        break; // end of dynamic entries
                    }
                    _ => {}
                }
            }

            if rel_count > 0 && rel_size > 0 && rel_ent > 0 {
                // Convert RELA table virtual address to file offset within the ELF.
                // The RELA table is in a non-load segment (read-only), so we access
                // it directly from elf_data using the section offset embedded in the
                // virtual address.
                // For PIE binaries: rela_vaddr (file-relative) = DT_RELA value.
                // We need to find the program header that covers this vaddr to compute
                // the file offset, or compute it as: file_offset = DT_RELA value
                // because ELF files typically have the RELA table in the first LOAD
                // segment with a matching offset.
                //
                // More robust: iterate LOAD segments to find which one covers rel_offset.
                let mut rela_file_off: u64 = 0;
                for k in 0..ph_count {
                    let ph2_ptr = unsafe {
                        let off = ph_start + (k * ph_size);
                        elf_data.as_ptr().add(off) as *const ProgramHeader
                    };
                    let ph2 = unsafe { &*ph2_ptr };
                    if ph2.p_type == PT_LOAD {
                        let lo = ph2.p_vaddr;
                        let hi = ph2.p_vaddr + ph2.p_filesz;
                        if rel_offset >= lo && rel_offset < hi {
                            rela_file_off = ph2.p_offset + (rel_offset - lo);
                            break;
                        }
                    }
                }

                if rela_file_off > 0 && rela_file_off as usize + (rel_count * rel_ent) as usize <= elf_data.len() {
                    let mut applied: u64 = 0;
                    for k in 0..rel_count {
                        let rela_ptr = unsafe {
                            elf_data.as_ptr().add(rela_file_off as usize + (k * rel_ent) as usize) as *const RelaEntry
                        };
                        let rela = unsafe { &*rela_ptr };
                        if rela_type(rela.r_info) == R_X86_64_RELATIVE {
                            // Compute target address in loaded image.
                            let target_vaddr = load_base.as_u64() + rela.r_offset.saturating_sub(min_vaddr);
                            let write_val = load_base.as_u64() + rela.r_addend as u64;
                            unsafe {
                                core::ptr::write_volatile(
                                    target_vaddr as *mut u64,
                                    write_val,
                                );
                            }
                            applied += 1;
                        }
                    }
                    serial_println!("ELF: applied {} R_X86_64_RELATIVE relocations", applied);
                } else {
                    serial_println!("ELF: R_X86_64_RELATIVE table not in LOAD segment, skipping");
                }
            }
            break; // only one PT_DYNAMIC
        }
    }

    let relocated_entry = header.entry.saturating_sub(min_vaddr);
    Ok(load_base + relocated_entry)
}
