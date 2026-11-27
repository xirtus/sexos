// SPDX-License-Identifier: GPL-3.0-only
//! Phase 1 kernel support for `KernelHandoffV1` live microkernel hot-swap.
//!
//! Provides:
//! - [`HandoffBootInfo`]: cached firmware/bootloader parameters initialized from
//!   Limine on cold boot or from [`KernelHandoffV1`] on live handoff.
//! - `IPI_QUIESCE` (`0xF3`) AP park/release atomic barrier (`park_ap` / `release_aps`),
//!   completely decoupled from `console::STOPPING`.
//! - [`validate_elf64_for_handoff`]: bounds-checked ELF64 x86_64 kernel image validator.
//! - [`_handoff_entry`]: Phase 1 live-handoff entry point that validates the
//!   incoming `KernelHandoffV1` header via HHDM, logs `[kernel.handoff.entry]`,
//!   and halts cleanly (ready for Phase 2 live state adoption).

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use sex_pdx::kernel_handoff::{
    HandoffError, HandoffFramebuffer, KernelHandoffV1, HO_FLAG_HAS_FRAMEBUFFER, HO_FLAG_HAS_RSDP,
    HO_FLAG_HAS_SMBIOS, IPI_QUIESCE_VECTOR,
};

/// Default Limine x86_64 4-level HHDM base used when entering via `_handoff_entry`
/// without a fresh Limine `HhdmResponse`.
pub const DEFAULT_HHDM_OFFSET: u64 = 0xffff_8000_0000_0000;

/// Cached boot/firmware descriptors preserved across live kernel handoff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandoffBootInfo {
    pub hhdm_offset: u64,
    pub rsdp_phys: u64,
    pub smbios_phys: u64,
    pub fb: Option<HandoffFramebuffer>,
    pub boot_part_guid: [u8; 16],
    pub is_live_handoff: bool,
}

impl HandoffBootInfo {
    /// Captures boot/firmware descriptors from Limine's static responses on cold boot.
    pub fn from_limine() -> Self {
        let hhdm_offset = crate::HHDM_REQUEST
            .response()
            .map(|r| r.offset)
            .unwrap_or(0);

        let phys_of = |addr: u64| -> u64 {
            if addr != 0 && hhdm_offset != 0 && addr >= hhdm_offset {
                addr - hhdm_offset
            } else {
                addr
            }
        };

        let rsdp_phys = crate::RSDP_REQUEST
            .response()
            .map(|r| phys_of(r.address as u64))
            .unwrap_or(0);

        let smbios_phys = crate::SMBIOS_REQUEST
            .response()
            .map(|r| {
                let addr = if r.entry_64 as u64 != 0 {
                    r.entry_64 as u64
                } else {
                    r.entry_32 as u64
                };
                phys_of(addr)
            })
            .unwrap_or(0);

        let fb = crate::FB_REQUEST
            .response()
            .and_then(|r| r.framebuffers().iter().next())
            .map(|f| {
                let virt_addr = f.address() as u64;
                let phys_addr = phys_of(virt_addr);
                let edid_packed = sex_pdx::display_boot::Physical::from_edid(
                    f.edid(),
                    f.width as u32,
                    f.height as u32,
                )
                .pack();
                HandoffFramebuffer {
                    phys_addr,
                    virt_addr,
                    width: f.width as u32,
                    height: f.height as u32,
                    pitch: f.pitch as u32,
                    bpp: f.bpp,
                    memory_model: f.memory_model,
                    red_mask_size: f.red_mask_size,
                    red_mask_shift: f.red_mask_shift,
                    green_mask_size: f.green_mask_size,
                    green_mask_shift: f.green_mask_shift,
                    blue_mask_size: f.blue_mask_size,
                    blue_mask_shift: f.blue_mask_shift,
                    _reserved: [0; 3],
                    edid_packed,
                }
            });

        let mut boot_part_guid = [0u8; 16];
        if let Some(file) = crate::KERNEL_FILE_REQUEST
            .response()
            .map(|r| r.executable_file())
        {
            let u = file.gpt_part_uuid;
            boot_part_guid[0..4].copy_from_slice(&u.a.to_le_bytes());
            boot_part_guid[4..6].copy_from_slice(&u.b.to_le_bytes());
            boot_part_guid[6..8].copy_from_slice(&u.c.to_le_bytes());
            boot_part_guid[8..16].copy_from_slice(&u.d);
        }

        Self {
            hhdm_offset,
            rsdp_phys,
            smbios_phys,
            fb,
            boot_part_guid,
            is_live_handoff: false,
        }
    }

    /// Reconstructs boot/firmware descriptors from a validated [`KernelHandoffV1`].
    pub fn from_handoff(hdr: &KernelHandoffV1) -> Result<Self, HandoffError> {
        hdr.validate()?;
        let rsdp_phys = if (hdr.flags & HO_FLAG_HAS_RSDP) != 0 || hdr.rsdp_phys != 0 {
            hdr.rsdp_phys
        } else {
            0
        };
        let smbios_phys = if (hdr.flags & HO_FLAG_HAS_SMBIOS) != 0 || hdr.smbios_phys != 0 {
            hdr.smbios_phys
        } else {
            0
        };
        let fb = if (hdr.flags & HO_FLAG_HAS_FRAMEBUFFER) != 0
            && hdr.fb.width > 0
            && hdr.fb.height > 0
        {
            Some(hdr.fb)
        } else {
            None
        };
        Ok(Self {
            hhdm_offset: hdr.hhdm_offset,
            rsdp_phys,
            smbios_phys,
            fb,
            boot_part_guid: hdr.boot_part_guid,
            is_live_handoff: true,
        })
    }
}

static BOOT_INFO: spin::Once<HandoffBootInfo> = spin::Once::new();

/// Initializes and caches [`HandoffBootInfo`] from Limine on cold boot.
pub fn init_from_limine() -> &'static HandoffBootInfo {
    BOOT_INFO.call_once(HandoffBootInfo::from_limine)
}

/// Validates `hdr` and caches [`HandoffBootInfo`] for a live kernel handoff.
pub fn init_from_handoff(hdr: &KernelHandoffV1) -> Result<&'static HandoffBootInfo, HandoffError> {
    let info = HandoffBootInfo::from_handoff(hdr)?;
    Ok(BOOT_INFO.call_once(|| info))
}

/// Returns the cached [`HandoffBootInfo`] if already initialized.
pub fn boot_info() -> Option<&'static HandoffBootInfo> {
    BOOT_INFO.get()
}

/// Returns the active HHDM offset from cached boot info or Limine fallback.
pub fn cached_hhdm_offset() -> Option<u64> {
    BOOT_INFO
        .get()
        .map(|b| b.hhdm_offset)
        .filter(|&o| o != 0)
        .or_else(|| crate::HHDM_REQUEST.response().map(|r| r.offset))
}

// ── IPI_QUIESCE (0xF3) Park / Release Barrier ────────────────────────────────

/// True while BSP holds secondary cores parked in the `IPI_QUIESCE` barrier.
pub static QUIESCE_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Number of secondary cores currently parked inside [`park_ap`].
pub static APS_PARKED_COUNT: AtomicU32 = AtomicU32::new(0);
/// Monotonic generation incremented by [`release_aps`] to wake parked APs.
pub static APS_RELEASE_GEN: AtomicU64 = AtomicU64::new(0);
/// Number of secondary cores that have observed release and exited [`park_ap`].
pub static APS_RESUMED_COUNT: AtomicU32 = AtomicU32::new(0);

/// Parks the calling AP inside the `IPI_QUIESCE` (`0xF3`) barrier until BSP
/// calls [`release_aps`].
///
/// Must NEVER touch `console::STOPPING`, `CoreLocal` (`GS`), heap, or serial locks,
/// as this may run from an interrupt arriving while the core was in Ring 3.
pub fn park_ap() {
    if !QUIESCE_ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let was_enabled = x86_64::instructions::interrupts::are_enabled();
    x86_64::instructions::interrupts::disable();

    let release_gen = APS_RELEASE_GEN.load(Ordering::Acquire);
    core::sync::atomic::fence(Ordering::SeqCst);
    if !QUIESCE_ACTIVE.load(Ordering::Acquire) {
        if was_enabled {
            x86_64::instructions::interrupts::enable();
        }
        return;
    }

    APS_PARKED_COUNT.fetch_add(1, Ordering::AcqRel);
    while QUIESCE_ACTIVE.load(Ordering::Acquire)
        && APS_RELEASE_GEN.load(Ordering::Acquire) == release_gen
    {
        core::hint::spin_loop();
    }
    APS_RESUMED_COUNT.fetch_add(1, Ordering::AcqRel);

    if was_enabled {
        x86_64::instructions::interrupts::enable();
    }
}

/// Releases all APs parked by [`park_ap`] and waits (bounded) until they resume.
///
/// Never touches `console::STOPPING`.
pub fn release_aps() {
    let parked = APS_PARKED_COUNT.load(Ordering::Acquire);
    QUIESCE_ACTIVE.store(false, Ordering::Release);
    APS_RELEASE_GEN.fetch_add(1, Ordering::AcqRel);

    let mut spins = 0u64;
    while APS_RESUMED_COUNT.load(Ordering::Acquire) < parked {
        core::hint::spin_loop();
        spins += 1;
        if spins > 100_000_000 {
            crate::serial_println!(
                "[kernel.handoff.quiesce] release_aps timeout parked={} resumed={}",
                parked,
                APS_RESUMED_COUNT.load(Ordering::Acquire)
            );
            break;
        }
    }
}

/// Initiates an `IPI_QUIESCE` (`0xF3`) barrier across all secondary cores.
///
/// Returns `true` if `expected_aps` parked within the bounded spin window, or
/// automatically releases any partially parked APs and returns `false` on timeout
/// or if a quiesce round is already active.
pub fn quiesce_aps(expected_aps: u32) -> bool {
    if QUIESCE_ACTIVE.load(Ordering::Acquire) {
        return false;
    }
    APS_PARKED_COUNT.store(0, Ordering::SeqCst);
    APS_RESUMED_COUNT.store(0, Ordering::SeqCst);
    QUIESCE_ACTIVE.store(true, Ordering::SeqCst);

    if expected_aps == 0 {
        return true;
    }

    crate::apic::send_to_others(IPI_QUIESCE_VECTOR);

    let mut spins = 0u64;
    while APS_PARKED_COUNT.load(Ordering::Acquire) < expected_aps {
        core::hint::spin_loop();
        spins += 1;
        if spins > 200_000_000 {
            release_aps();
            return false;
        }
    }
    true
}

// ── ELF64 Validation for Kernel Staging ──────────────────────────────────────

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const EV_CURRENT: u8 = 1;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
const EM_X86_64: u16 = 0x3E;
const PT_LOAD: u32 = 1;
const ELF64_EHDR_SIZE: usize = 64;
const ELF64_PHDR_SIZE: usize = 56;

/// Validated ELF64 metadata needed to stage a replacement kernel image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElfStageInfo {
    pub entry_va: u64,
    pub entry_offset: u64,
    pub min_vaddr: u64,
    pub max_vaddr: u64,
    pub mem_bytes: u64,
    pub load_segment_count: u16,
    pub elf_type: u16,
}

#[inline]
fn read_u16_le(buf: &[u8], off: usize) -> Option<u16> {
    let bytes: [u8; 2] = buf.get(off..off + 2)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

#[inline]
fn read_u32_le(buf: &[u8], off: usize) -> Option<u32> {
    let bytes: [u8; 4] = buf.get(off..off + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

#[inline]
fn read_u64_le(buf: &[u8], off: usize) -> Option<u64> {
    let bytes: [u8; 8] = buf.get(off..off + 8)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

/// Validates an ELF64 x86_64 image prior to mapping and staging for live handoff.
///
/// Checks ELF64 magic, class/endianness, `EM_X86_64` (`0x3E`), program header table
/// bounds, all `PT_LOAD` file/virtual bounds (`p_filesz <= p_memsz`), and verifies
/// `e_entry` resides inside a `PT_LOAD` segment.
pub fn validate_elf64_for_handoff(elf: &[u8]) -> Result<ElfStageInfo, HandoffError> {
    if elf.len() < ELF64_EHDR_SIZE {
        return Err(HandoffError::InvalidElfHeader);
    }
    if elf[0..4] != ELF_MAGIC {
        return Err(HandoffError::InvalidElfMagic);
    }
    if elf[4] != ELFCLASS64 || elf[5] != ELFDATA2LSB {
        return Err(HandoffError::UnsupportedElfClass);
    }
    if elf[6] != EV_CURRENT {
        return Err(HandoffError::InvalidElfHeader);
    }

    let elf_type = read_u16_le(elf, 16).ok_or(HandoffError::InvalidElfHeader)?;
    if elf_type != ET_EXEC && elf_type != ET_DYN {
        return Err(HandoffError::InvalidElfHeader);
    }

    let machine = read_u16_le(elf, 18).ok_or(HandoffError::InvalidElfHeader)?;
    if machine != EM_X86_64 {
        return Err(HandoffError::UnsupportedElfArch);
    }

    let version = read_u32_le(elf, 20).ok_or(HandoffError::InvalidElfHeader)?;
    if version != 1 {
        return Err(HandoffError::InvalidElfHeader);
    }

    let entry_va = read_u64_le(elf, 24).ok_or(HandoffError::InvalidElfHeader)?;
    let phoff = read_u64_le(elf, 32).ok_or(HandoffError::InvalidElfHeader)?;
    let ehsize = read_u16_le(elf, 52).ok_or(HandoffError::InvalidElfHeader)?;
    let phentsize = read_u16_le(elf, 54).ok_or(HandoffError::InvalidElfHeader)?;
    let phnum = read_u16_le(elf, 56).ok_or(HandoffError::InvalidElfHeader)?;

    if (ehsize as usize) < ELF64_EHDR_SIZE || (phentsize as usize) < ELF64_PHDR_SIZE {
        return Err(HandoffError::InvalidElfHeader);
    }
    if phnum == 0 {
        return Err(HandoffError::NoLoadSegments);
    }
    if phoff < ehsize as u64 {
        return Err(HandoffError::InvalidElfHeader);
    }

    let ph_table_bytes = (phnum as u64)
        .checked_mul(phentsize as u64)
        .ok_or(HandoffError::ElfBoundsOutOfRange)?;
    let ph_end = phoff
        .checked_add(ph_table_bytes)
        .ok_or(HandoffError::ElfBoundsOutOfRange)?;
    if ph_end > elf.len() as u64 {
        return Err(HandoffError::ElfBoundsOutOfRange);
    }

    let mut min_vaddr = u64::MAX;
    let mut max_vaddr = 0u64;
    let mut load_segment_count: u16 = 0;
    let mut entry_in_load = false;

    for i in 0..phnum as usize {
        let off = phoff as usize + i * phentsize as usize;
        let p_type = read_u32_le(elf, off).ok_or(HandoffError::ElfBoundsOutOfRange)?;
        if p_type != PT_LOAD {
            continue;
        }
        let p_offset = read_u64_le(elf, off + 8).ok_or(HandoffError::ElfBoundsOutOfRange)?;
        let p_vaddr = read_u64_le(elf, off + 16).ok_or(HandoffError::ElfBoundsOutOfRange)?;
        let p_filesz = read_u64_le(elf, off + 32).ok_or(HandoffError::ElfBoundsOutOfRange)?;
        let p_memsz = read_u64_le(elf, off + 40).ok_or(HandoffError::ElfBoundsOutOfRange)?;

        if p_filesz > p_memsz {
            return Err(HandoffError::ElfBoundsOutOfRange);
        }
        let file_end = p_offset
            .checked_add(p_filesz)
            .ok_or(HandoffError::ElfBoundsOutOfRange)?;
        if file_end > elf.len() as u64 {
            return Err(HandoffError::ElfBoundsOutOfRange);
        }
        if p_memsz == 0 {
            continue;
        }
        let va_end = p_vaddr
            .checked_add(p_memsz)
            .ok_or(HandoffError::ElfBoundsOutOfRange)?;

        if p_vaddr < min_vaddr {
            min_vaddr = p_vaddr;
        }
        if va_end > max_vaddr {
            max_vaddr = va_end;
        }
        if entry_va >= p_vaddr && entry_va < va_end {
            entry_in_load = true;
        }
        load_segment_count = load_segment_count
            .checked_add(1)
            .ok_or(HandoffError::ElfBoundsOutOfRange)?;
    }

    if load_segment_count == 0 {
        return Err(HandoffError::NoLoadSegments);
    }
    if !entry_in_load {
        return Err(HandoffError::EntryOutOfLoadSegments);
    }

    let mem_bytes = max_vaddr
        .checked_sub(min_vaddr)
        .ok_or(HandoffError::ElfBoundsOutOfRange)?;
    let entry_offset = entry_va - min_vaddr;

    Ok(ElfStageInfo {
        entry_va,
        entry_offset,
        min_vaddr,
        max_vaddr,
        mem_bytes,
        load_segment_count,
        elf_type,
    })
}

// ── Phase 1 Live Handoff Entry Point ─────────────────────────────────────────

/// Retained reference so `--gc-sections` never strips `_handoff_entry` from the kernel ELF.
#[used]
static _KEEP_HANDOFF_ENTRY: extern "C" fn(u64) -> ! = _handoff_entry;

/// Phase 1 live-handoff entry point invoked by the outgoing kernel with `handoff_phys`
/// in `rdi`. Validates the [`KernelHandoffV1`] header via `hhdm_offset`, logs
/// `[kernel.handoff.entry]` on serial, and halts cleanly (ready for Phase 2 live
/// state adoption).
#[no_mangle]
pub extern "C" fn _handoff_entry(handoff_phys: u64) -> ! {
    x86_64::instructions::interrupts::disable();

    let align = core::mem::align_of::<KernelHandoffV1>() as u64;
    if handoff_phys == 0 || handoff_phys % align != 0 {
        crate::serial_println!(
            "[kernel.handoff.entry] ok=0 err=invalid_phys phys={:#x}",
            handoff_phys
        );
        halt_forever();
    }

    let hhdm = crate::HHDM_REQUEST
        .response()
        .map(|r| r.offset)
        .filter(|&o| o != 0)
        .unwrap_or(DEFAULT_HHDM_OFFSET);

    let Some(hdr_va) = hhdm.checked_add(handoff_phys) else {
        crate::serial_println!(
            "[kernel.handoff.entry] ok=0 err=hhdm_overflow phys={:#x} hhdm={:#x}",
            handoff_phys,
            hhdm
        );
        halt_forever();
    };

    let hdr = unsafe { &*(hdr_va as *const KernelHandoffV1) };
    match init_from_handoff(hdr) {
        Ok(boot_info) => {
            crate::serial_println!(
                "[kernel.handoff.entry] ok=1 version={} magic={:#x} hhdm={:#x} cr3={:#x} entry={:#x} old_va={:#x} new_va={:#x} bytes={} sections={} cores={} live={}",
                hdr.version,
                hdr.magic,
                boot_info.hhdm_offset,
                hdr.cr3_phys,
                hdr.new_kernel_entry_va,
                hdr.old_kernel_virt_base,
                hdr.new_kernel_virt_base,
                hdr.new_kernel_bytes,
                hdr.section_count,
                hdr.online_cores,
                boot_info.is_live_handoff as u8
            );
        }
        Err(err) => {
            crate::serial_println!(
                "[kernel.handoff.entry] ok=0 err={} phys={:#x}",
                err.as_str(),
                handoff_phys
            );
        }
    }

    halt_forever();
}

#[inline(always)]
fn halt_forever() -> ! {
    loop {
        x86_64::instructions::interrupts::disable();
        x86_64::instructions::hlt();
    }
}
