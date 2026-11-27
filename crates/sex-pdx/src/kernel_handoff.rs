// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Fixed-ABI structures and validation for `KernelHandoffV1` live microkernel hot-swap.
//!
//! Shared between the old kernel, the staged replacement kernel (`_handoff_entry`),
//! and host/userspace tooling. All ABI records are `#[repr(C)]`, explicitly padded,
//! and self-describing via `header_size` and `section_stride`.

/// `"SEXKHOV1"` (`"SEXKHO_V1"`) encoded little-endian as a 64-bit magic sentinel.
pub const KERNEL_HANDOFF_MAGIC: u64 = 0x3156_4F48_4B58_4553;

/// Current `KernelHandoffV1` wire/ABI version.
pub const KERNEL_HANDOFF_VERSION: u32 = 1;

/// Inter-processor interrupt vector used to quiesce secondary cores cleanly
/// during live kernel handoff without touching `console::STOPPING`.
pub const IPI_QUIESCE_VECTOR: u8 = 0xF3;

/// Primary higher-half kernel link/load virtual slot (cold boot default).
pub const KERNEL_VA_SLOT_A: u64 = 0xffff_ffff_8020_0000;

/// Alternate higher-half kernel link/load virtual slot for A/B live handoff.
pub const KERNEL_VA_SLOT_B: u64 = 0xffff_ffff_a000_0000;

/// Number of binary buddy orders (`0..=18`, matching `MAX_ORDER = 18` in kernel).
pub const HANDOFF_BUDDY_ORDERS: usize = 19;

// ── Header Flags (`KernelHandoffV1::flags`) ──────────────────────────────────

/// Kernel entered via live handoff rather than cold Limine boot.
pub const HO_FLAG_LIVE_HANDOFF: u64 = 1 << 0;
/// Framebuffer descriptor (`fb`) contains valid scanout geometry and address.
pub const HO_FLAG_HAS_FRAMEBUFFER: u64 = 1 << 1;
/// `rsdp_phys` holds a valid ACPI RSDP physical address.
pub const HO_FLAG_HAS_RSDP: u64 = 1 << 2;
/// `smbios_phys` holds a valid SMBIOS entry point physical address.
pub const HO_FLAG_HAS_SMBIOS: u64 = 1 << 3;
/// `boot_part_guid` holds the GPT partition UUID of the active boot volume.
pub const HO_FLAG_HAS_BOOT_PART_GUID: u64 = 1 << 4;
/// Secondary cores were parked via the `IPI_QUIESCE_VECTOR` barrier before handoff.
pub const HO_FLAG_APS_QUIESCED: u64 = 1 << 5;
/// COM1 serial console is already initialized and ready for immediate logging.
pub const HO_FLAG_SERIAL_READY: u64 = 1 << 6;
/// Intel MPK/PKU (`CR4.PKE`) is enabled across cores.
pub const HO_FLAG_PKU_ENABLED: u64 = 1 << 7;
/// Kernel heap state (`heap_state`) is populated for live adoption.
pub const HO_FLAG_HEAP_PRESERVED: u64 = 1 << 8;
/// Physical buddy allocator state (`buddy_state`) is populated for live adoption.
pub const HO_FLAG_BUDDY_PRESERVED: u64 = 1 << 9;

// ── Section Flags (`HandoffSection::flags`) ──────────────────────────────────

/// Receiver must recognize and adopt this section; abort handoff if unknown.
pub const SEC_FLAG_REQUIRED: u32 = 1 << 0;
/// Section payload is immutable / read-only across handoff.
pub const SEC_FLAG_READONLY: u32 = 1 << 1;
/// Section payload occupies a physically contiguous frame run at `phys_addr`.
pub const SEC_FLAG_PHYS_CONTIG: u32 = 1 << 2;
/// Scratch/handoff buffer may be zeroed and returned to the buddy after adoption.
pub const SEC_FLAG_ZERO_ON_ADOPT: u32 = 1 << 3;

/// Typed payload tags for entries in the `HandoffSection` descriptor table.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffSectionTag {
    None = 0,
    DomainRegistry = 1,
    Scheduler = 2,
    CoreLocal = 3,
    IpcV2 = 4,
    DeviceLeases = 5,
    MemoryRegions = 6,
    CrashLog = 7,
    BootModules = 8,
}

impl HandoffSectionTag {
    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    pub const fn from_u32(raw: u32) -> Option<Self> {
        match raw {
            0 => Some(Self::None),
            1 => Some(Self::DomainRegistry),
            2 => Some(Self::Scheduler),
            3 => Some(Self::CoreLocal),
            4 => Some(Self::IpcV2),
            5 => Some(Self::DeviceLeases),
            6 => Some(Self::MemoryRegions),
            7 => Some(Self::CrashLog),
            8 => Some(Self::BootModules),
            _ => None,
        }
    }
}

/// Descriptor for an auxiliary state blob passed across `KernelHandoffV1`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandoffSection {
    /// Raw [`HandoffSectionTag`] discriminant (stored as `u32` for ABI stability).
    pub tag: u32,
    /// `SEC_FLAG_*` bitmask.
    pub flags: u32,
    /// Per-section schema version.
    pub version: u32,
    pub _reserved: u32,
    pub phys_addr: u64,
    pub virt_addr: u64,
    pub byte_len: u64,
    pub checksum_fnv64: u64,
}

impl HandoffSection {
    pub const fn empty() -> Self {
        Self {
            tag: HandoffSectionTag::None.as_u32(),
            flags: 0,
            version: 1,
            _reserved: 0,
            phys_addr: 0,
            virt_addr: 0,
            byte_len: 0,
            checksum_fnv64: 0,
        }
    }

    pub const fn new(
        tag: HandoffSectionTag,
        flags: u32,
        phys_addr: u64,
        virt_addr: u64,
        byte_len: u64,
    ) -> Self {
        Self {
            tag: tag.as_u32(),
            flags,
            version: 1,
            _reserved: 0,
            phys_addr,
            virt_addr,
            byte_len,
            checksum_fnv64: 0,
        }
    }

    pub const fn tag_kind(&self) -> Option<HandoffSectionTag> {
        HandoffSectionTag::from_u32(self.tag)
    }
}

/// Fixed-ABI framebuffer metadata preserved across live kernel handoff.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandoffFramebuffer {
    pub phys_addr: u64,
    pub virt_addr: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bpp: u16,
    pub memory_model: u8,
    pub red_mask_size: u8,
    pub red_mask_shift: u8,
    pub green_mask_size: u8,
    pub green_mask_shift: u8,
    pub blue_mask_size: u8,
    pub blue_mask_shift: u8,
    pub _reserved: [u8; 3],
    pub edid_packed: u64,
}

impl HandoffFramebuffer {
    pub const fn empty() -> Self {
        Self {
            phys_addr: 0,
            virt_addr: 0,
            width: 0,
            height: 0,
            pitch: 0,
            bpp: 0,
            memory_model: 0,
            red_mask_size: 0,
            red_mask_shift: 0,
            green_mask_size: 0,
            green_mask_shift: 0,
            blue_mask_size: 0,
            blue_mask_shift: 0,
            _reserved: [0; 3],
            edid_packed: 0,
        }
    }
}

/// Fixed-ABI snapshot of the kernel `linked_list_allocator` heap (`kheap.rs`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandoffHeapState {
    pub heap_start: u64,
    pub mapped_bytes: u64,
    pub limit_bytes: u64,
    pub used_bytes: u64,
    pub peak_bytes: u64,
    pub live_allocs: u64,
    pub step_bytes: u64,
    pub flags: u64,
}

impl HandoffHeapState {
    pub const fn empty() -> Self {
        Self {
            heap_start: 0,
            mapped_bytes: 0,
            limit_bytes: 0,
            used_bytes: 0,
            peak_bytes: 0,
            live_allocs: 0,
            step_bytes: 0,
            flags: 0,
        }
    }
}

/// Fixed-ABI snapshot of the physical buddy allocator (`allocator.rs`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandoffBuddyState {
    pub metadata_base: u64,
    pub total_pages: u64,
    pub free_pages: u64,
    pub page_metadata_stride: u64,
    pub heads: [u64; HANDOFF_BUDDY_ORDERS],
    pub max_order: u32,
    pub flags: u32,
}

impl HandoffBuddyState {
    pub const fn empty() -> Self {
        Self {
            metadata_base: 0,
            total_pages: 0,
            free_pages: 0,
            page_metadata_stride: 0,
            heads: [0; HANDOFF_BUDDY_ORDERS],
            max_order: (HANDOFF_BUDDY_ORDERS - 1) as u32,
            flags: 0,
        }
    }
}

/// Validation and ELF staging errors for `KernelHandoffV1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum HandoffError {
    InvalidMagic = 1,
    UnsupportedVersion = 2,
    InvalidHeaderSize = 3,
    InvalidSectionStride = 4,
    MissingHhdm = 5,
    MissingCr3 = 6,
    MissingEntry = 7,
    InvalidKernelRange = 8,
    VirtualRangeOverlap = 9,
    PhysicalRangeOverlap = 10,
    InvalidSectionTable = 11,
    InvalidElfMagic = 12,
    UnsupportedElfClass = 13,
    UnsupportedElfArch = 14,
    InvalidElfHeader = 15,
    NoLoadSegments = 16,
    ElfBoundsOutOfRange = 17,
    EntryOutOfLoadSegments = 18,
    NullPointer = 19,
}

impl HandoffError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidMagic => "invalid_magic",
            Self::UnsupportedVersion => "unsupported_version",
            Self::InvalidHeaderSize => "invalid_header_size",
            Self::InvalidSectionStride => "invalid_section_stride",
            Self::MissingHhdm => "missing_hhdm",
            Self::MissingCr3 => "missing_cr3",
            Self::MissingEntry => "missing_entry",
            Self::InvalidKernelRange => "invalid_kernel_range",
            Self::VirtualRangeOverlap => "virtual_range_overlap",
            Self::PhysicalRangeOverlap => "physical_range_overlap",
            Self::InvalidSectionTable => "invalid_section_table",
            Self::InvalidElfMagic => "invalid_elf_magic",
            Self::UnsupportedElfClass => "unsupported_elf_class",
            Self::UnsupportedElfArch => "unsupported_elf_arch",
            Self::InvalidElfHeader => "invalid_elf_header",
            Self::NoLoadSegments => "no_load_segments",
            Self::ElfBoundsOutOfRange => "elf_bounds_out_of_range",
            Self::EntryOutOfLoadSegments => "entry_out_of_load_segments",
            Self::NullPointer => "null_pointer",
        }
    }
}

/// Fixed-ABI live kernel handoff header passed by physical address (`rdi`) to `_handoff_entry`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelHandoffV1 {
    pub magic: u64,
    pub version: u32,
    pub min_compat_version: u32,
    pub header_size: u32,
    pub section_stride: u32,
    pub flags: u64,
    pub handoff_phys: u64,
    pub hhdm_offset: u64,
    pub cr3_phys: u64,
    pub rsdp_phys: u64,
    pub smbios_phys: u64,
    pub boot_part_guid: [u8; 16],
    pub old_kernel_virt_base: u64,
    pub old_kernel_bytes: u64,
    pub old_kernel_phys_base: u64,
    pub new_kernel_virt_base: u64,
    pub new_kernel_bytes: u64,
    pub new_kernel_phys_base: u64,
    pub new_kernel_entry_va: u64,
    pub sections_phys: u64,
    pub section_count: u32,
    pub online_cores: u32,
    pub uptime_ticks: u64,
    pub fb: HandoffFramebuffer,
    pub heap_state: HandoffHeapState,
    pub buddy_state: HandoffBuddyState,
}

impl Default for KernelHandoffV1 {
    fn default() -> Self {
        Self::empty()
    }
}

#[inline]
const fn checked_ranges_overlap(
    a_start: u64,
    a_len: u64,
    b_start: u64,
    b_len: u64,
) -> Option<bool> {
    if a_len == 0 || b_len == 0 {
        return None;
    }
    let Some(a_end) = a_start.checked_add(a_len) else {
        return None;
    };
    let Some(b_end) = b_start.checked_add(b_len) else {
        return None;
    };
    Some(a_start < b_end && b_start < a_end)
}

impl KernelHandoffV1 {
    /// Constructs a zeroed `KernelHandoffV1` pre-populated with canonical v1 magic,
    /// version, `header_size`, and `section_stride`.
    pub const fn empty() -> Self {
        Self {
            magic: KERNEL_HANDOFF_MAGIC,
            version: KERNEL_HANDOFF_VERSION,
            min_compat_version: KERNEL_HANDOFF_VERSION,
            header_size: core::mem::size_of::<Self>() as u32,
            section_stride: core::mem::size_of::<HandoffSection>() as u32,
            flags: 0,
            handoff_phys: 0,
            hhdm_offset: 0,
            cr3_phys: 0,
            rsdp_phys: 0,
            smbios_phys: 0,
            boot_part_guid: [0; 16],
            old_kernel_virt_base: 0,
            old_kernel_bytes: 0,
            old_kernel_phys_base: 0,
            new_kernel_virt_base: 0,
            new_kernel_bytes: 0,
            new_kernel_phys_base: 0,
            new_kernel_entry_va: 0,
            sections_phys: 0,
            section_count: 0,
            online_cores: 0,
            uptime_ticks: 0,
            fb: HandoffFramebuffer::empty(),
            heap_state: HandoffHeapState::empty(),
            buddy_state: HandoffBuddyState::empty(),
        }
    }

    /// Validates header integrity, version compatibility, structural strides,
    /// mandatory address fields, and strict non-overlap of old/new kernel ranges.
    pub fn validate(&self) -> Result<(), HandoffError> {
        if self.magic != KERNEL_HANDOFF_MAGIC {
            return Err(HandoffError::InvalidMagic);
        }
        if self.version == 0
            || self.min_compat_version == 0
            || self.min_compat_version > self.version
            || KERNEL_HANDOFF_VERSION < self.min_compat_version
        {
            return Err(HandoffError::UnsupportedVersion);
        }

        let expected_hdr = core::mem::size_of::<Self>() as u32;
        if self.version == KERNEL_HANDOFF_VERSION {
            if self.header_size != expected_hdr {
                return Err(HandoffError::InvalidHeaderSize);
            }
        } else if self.header_size < expected_hdr {
            return Err(HandoffError::InvalidHeaderSize);
        }

        let expected_stride = core::mem::size_of::<HandoffSection>() as u32;
        if self.version == KERNEL_HANDOFF_VERSION {
            if self.section_stride != expected_stride {
                return Err(HandoffError::InvalidSectionStride);
            }
        } else if self.section_stride < expected_stride {
            return Err(HandoffError::InvalidSectionStride);
        }

        if self.hhdm_offset == 0 {
            return Err(HandoffError::MissingHhdm);
        }
        if self.cr3_phys == 0 {
            return Err(HandoffError::MissingCr3);
        }
        if self.new_kernel_entry_va == 0 {
            return Err(HandoffError::MissingEntry);
        }

        if self.section_count > 0 && self.sections_phys == 0 {
            return Err(HandoffError::InvalidSectionTable);
        }

        if self.old_kernel_virt_base == 0 || self.new_kernel_virt_base == 0 {
            return Err(HandoffError::InvalidKernelRange);
        }
        match checked_ranges_overlap(
            self.old_kernel_virt_base,
            self.old_kernel_bytes,
            self.new_kernel_virt_base,
            self.new_kernel_bytes,
        ) {
            None => return Err(HandoffError::InvalidKernelRange),
            Some(true) => return Err(HandoffError::VirtualRangeOverlap),
            Some(false) => {}
        }

        if self.old_kernel_phys_base == 0 || self.new_kernel_phys_base == 0 {
            return Err(HandoffError::InvalidKernelRange);
        }
        match checked_ranges_overlap(
            self.old_kernel_phys_base,
            self.old_kernel_bytes,
            self.new_kernel_phys_base,
            self.new_kernel_bytes,
        ) {
            None => return Err(HandoffError::InvalidKernelRange),
            Some(true) => return Err(HandoffError::PhysicalRangeOverlap),
            Some(false) => {}
        }

        Ok(())
    }
}

/// Selects the alternate higher-half kernel virtual base slot for A/B hot-swap.
///
/// Returns [`KERNEL_VA_SLOT_B`] when `current_va` resides in slot A
/// (`[KERNEL_VA_SLOT_A, KERNEL_VA_SLOT_B)`), and [`KERNEL_VA_SLOT_A`] otherwise.
pub const fn select_alternate_kernel_va(current_va: u64) -> u64 {
    if current_va >= KERNEL_VA_SLOT_A && current_va < KERNEL_VA_SLOT_B {
        KERNEL_VA_SLOT_B
    } else {
        KERNEL_VA_SLOT_A
    }
}

// Compile-time ABI layout invariants.
const _: () = assert!(core::mem::size_of::<HandoffSection>() == 48);
const _: () = assert!(core::mem::size_of::<HandoffFramebuffer>() == 48);
const _: () = assert!(core::mem::size_of::<HandoffHeapState>() == 64);
const _: () = assert!(core::mem::size_of::<HandoffBuddyState>() == 192);
const _: () = assert!(core::mem::size_of::<KernelHandoffV1>() == 472);

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::{align_of, offset_of, size_of};

    fn sample_valid_handoff() -> KernelHandoffV1 {
        let mut h = KernelHandoffV1::empty();
        h.flags = HO_FLAG_LIVE_HANDOFF | HO_FLAG_HAS_RSDP | HO_FLAG_APS_QUIESCED;
        h.handoff_phys = 0x1000_0000;
        h.hhdm_offset = 0xffff_8000_0000_0000;
        h.cr3_phys = 0x0010_0000;
        h.rsdp_phys = 0x000f_0000;
        h.old_kernel_virt_base = KERNEL_VA_SLOT_A;
        h.old_kernel_bytes = 0x0040_0000;
        h.old_kernel_phys_base = 0x0020_0000;
        h.new_kernel_virt_base = KERNEL_VA_SLOT_B;
        h.new_kernel_bytes = 0x0040_0000;
        h.new_kernel_phys_base = 0x0400_0000;
        h.new_kernel_entry_va = KERNEL_VA_SLOT_B + 0x1200;
        h.online_cores = 4;
        h
    }

    #[test]
    fn constants_and_abi_sizes_are_pinned() {
        assert_eq!(KERNEL_HANDOFF_MAGIC, 0x3156_4F48_4B58_4553);
        assert_eq!(&KERNEL_HANDOFF_MAGIC.to_le_bytes(), b"SEXKHOV1");
        assert_eq!(KERNEL_HANDOFF_VERSION, 1);
        assert_eq!(IPI_QUIESCE_VECTOR, 0xF3);
        assert_eq!(KERNEL_VA_SLOT_A, 0xffff_ffff_8020_0000);
        assert_eq!(KERNEL_VA_SLOT_B, 0xffff_ffff_a000_0000);

        assert_eq!(size_of::<HandoffSection>(), 48);
        assert_eq!(align_of::<HandoffSection>(), 8);
        assert_eq!(size_of::<HandoffFramebuffer>(), 48);
        assert_eq!(align_of::<HandoffFramebuffer>(), 8);
        assert_eq!(size_of::<HandoffHeapState>(), 64);
        assert_eq!(align_of::<HandoffHeapState>(), 8);
        assert_eq!(size_of::<HandoffBuddyState>(), 192);
        assert_eq!(align_of::<HandoffBuddyState>(), 8);
        assert_eq!(size_of::<KernelHandoffV1>(), 472);
        assert_eq!(align_of::<KernelHandoffV1>(), 8);

        assert_eq!(offset_of!(KernelHandoffV1, magic), 0);
        assert_eq!(offset_of!(KernelHandoffV1, version), 8);
        assert_eq!(offset_of!(KernelHandoffV1, min_compat_version), 12);
        assert_eq!(offset_of!(KernelHandoffV1, header_size), 16);
        assert_eq!(offset_of!(KernelHandoffV1, section_stride), 20);
        assert_eq!(offset_of!(KernelHandoffV1, flags), 24);
        assert_eq!(offset_of!(KernelHandoffV1, hhdm_offset), 40);
        assert_eq!(offset_of!(KernelHandoffV1, cr3_phys), 48);
        assert_eq!(offset_of!(KernelHandoffV1, fb), 168);
        assert_eq!(offset_of!(KernelHandoffV1, heap_state), 216);
        assert_eq!(offset_of!(KernelHandoffV1, buddy_state), 280);
    }

    #[test]
    fn valid_handoff_passes_and_adjacent_ranges_allowed() {
        let mut h = sample_valid_handoff();
        assert_eq!(h.validate(), Ok(()));

        // Adjacent virtual and physical intervals [base, base+len) do not overlap.
        h.new_kernel_virt_base = h.old_kernel_virt_base + h.old_kernel_bytes;
        h.new_kernel_phys_base = h.old_kernel_phys_base + h.old_kernel_bytes;
        h.new_kernel_entry_va = h.new_kernel_virt_base + 0x100;
        assert_eq!(h.validate(), Ok(()));

        // Forward-compatible v2 header with min_compat_version=1 and larger header/stride.
        h.version = 2;
        h.min_compat_version = 1;
        h.header_size += 16;
        h.section_stride += 8;
        assert_eq!(h.validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_bad_magic_and_versions() {
        let mut h = sample_valid_handoff();
        h.magic ^= 1;
        assert_eq!(h.validate(), Err(HandoffError::InvalidMagic));

        let mut h = sample_valid_handoff();
        h.version = 0;
        assert_eq!(h.validate(), Err(HandoffError::UnsupportedVersion));

        let mut h = sample_valid_handoff();
        h.min_compat_version = 0;
        assert_eq!(h.validate(), Err(HandoffError::UnsupportedVersion));

        let mut h = sample_valid_handoff();
        h.version = 2;
        h.min_compat_version = 2;
        assert_eq!(h.validate(), Err(HandoffError::UnsupportedVersion));

        let mut h = sample_valid_handoff();
        h.version = 1;
        h.min_compat_version = 2;
        assert_eq!(h.validate(), Err(HandoffError::UnsupportedVersion));
    }

    #[test]
    fn validate_rejects_bad_header_size_and_section_stride() {
        let mut h = sample_valid_handoff();
        h.header_size -= 4;
        assert_eq!(h.validate(), Err(HandoffError::InvalidHeaderSize));

        let mut h = sample_valid_handoff();
        h.header_size += 4;
        assert_eq!(h.validate(), Err(HandoffError::InvalidHeaderSize));

        let mut h = sample_valid_handoff();
        h.section_stride -= 4;
        assert_eq!(h.validate(), Err(HandoffError::InvalidSectionStride));

        let mut h = sample_valid_handoff();
        h.section_stride += 4;
        assert_eq!(h.validate(), Err(HandoffError::InvalidSectionStride));

        let mut h = sample_valid_handoff();
        h.version = 2;
        h.min_compat_version = 1;
        h.header_size -= 1;
        assert_eq!(h.validate(), Err(HandoffError::InvalidHeaderSize));
    }

    #[test]
    fn validate_rejects_missing_hhdm_cr3_entry_and_section_table() {
        let mut h = sample_valid_handoff();
        h.hhdm_offset = 0;
        assert_eq!(h.validate(), Err(HandoffError::MissingHhdm));

        let mut h = sample_valid_handoff();
        h.cr3_phys = 0;
        assert_eq!(h.validate(), Err(HandoffError::MissingCr3));

        let mut h = sample_valid_handoff();
        h.new_kernel_entry_va = 0;
        assert_eq!(h.validate(), Err(HandoffError::MissingEntry));

        let mut h = sample_valid_handoff();
        h.section_count = 2;
        h.sections_phys = 0;
        assert_eq!(h.validate(), Err(HandoffError::InvalidSectionTable));
    }

    #[test]
    fn validate_rejects_overlapping_or_invalid_kernel_ranges() {
        // Virtual overlap (new inside old)
        let mut h = sample_valid_handoff();
        h.new_kernel_virt_base = h.old_kernel_virt_base + 0x1000;
        assert_eq!(h.validate(), Err(HandoffError::VirtualRangeOverlap));

        // Virtual overlap (new starts before old and extends into old)
        let mut h = sample_valid_handoff();
        h.new_kernel_virt_base = h.old_kernel_virt_base - 0x1000;
        assert_eq!(h.validate(), Err(HandoffError::VirtualRangeOverlap));

        // Physical overlap
        let mut h = sample_valid_handoff();
        h.new_kernel_phys_base = h.old_kernel_phys_base + h.old_kernel_bytes - 1;
        assert_eq!(h.validate(), Err(HandoffError::PhysicalRangeOverlap));

        // Zero size or integer overflow
        let mut h = sample_valid_handoff();
        h.old_kernel_bytes = 0;
        assert_eq!(h.validate(), Err(HandoffError::InvalidKernelRange));

        let mut h = sample_valid_handoff();
        h.new_kernel_bytes = u64::MAX;
        assert_eq!(h.validate(), Err(HandoffError::InvalidKernelRange));

        let mut h = sample_valid_handoff();
        h.old_kernel_phys_base = 0;
        assert_eq!(h.validate(), Err(HandoffError::InvalidKernelRange));
    }

    #[test]
    fn alternate_kernel_va_slot_selection() {
        assert_eq!(select_alternate_kernel_va(KERNEL_VA_SLOT_A), KERNEL_VA_SLOT_B);
        assert_eq!(
            select_alternate_kernel_va(KERNEL_VA_SLOT_A + 0x12_3456),
            KERNEL_VA_SLOT_B
        );
        assert_eq!(select_alternate_kernel_va(KERNEL_VA_SLOT_B), KERNEL_VA_SLOT_A);
        assert_eq!(
            select_alternate_kernel_va(KERNEL_VA_SLOT_B + 0x12_3456),
            KERNEL_VA_SLOT_A
        );
        assert_eq!(select_alternate_kernel_va(0), KERNEL_VA_SLOT_A);
        assert_eq!(select_alternate_kernel_va(KERNEL_VA_SLOT_A - 1), KERNEL_VA_SLOT_A);
    }

    #[test]
    fn section_tag_roundtrip() {
        let sec = HandoffSection::new(
            HandoffSectionTag::Scheduler,
            SEC_FLAG_REQUIRED | SEC_FLAG_PHYS_CONTIG,
            0x2000_0000,
            0xffff_8000_2000_0000,
            4096,
        );
        assert_eq!(sec.tag_kind(), Some(HandoffSectionTag::Scheduler));
        assert_eq!(HandoffSection { tag: 999, ..sec }.tag_kind(), None);
    }

    #[test]
    fn flags_and_errors_are_distinct() {
        let ho_flags = [
            HO_FLAG_LIVE_HANDOFF,
            HO_FLAG_HAS_FRAMEBUFFER,
            HO_FLAG_HAS_RSDP,
            HO_FLAG_HAS_SMBIOS,
            HO_FLAG_HAS_BOOT_PART_GUID,
            HO_FLAG_APS_QUIESCED,
            HO_FLAG_SERIAL_READY,
            HO_FLAG_PKU_ENABLED,
            HO_FLAG_HEAP_PRESERVED,
            HO_FLAG_BUDDY_PRESERVED,
        ];
        let mut combined_ho = 0u64;
        for f in ho_flags {
            assert!(f.is_power_of_two());
            assert_eq!(combined_ho & f, 0);
            combined_ho |= f;
        }

        let sec_flags = [
            SEC_FLAG_REQUIRED,
            SEC_FLAG_READONLY,
            SEC_FLAG_PHYS_CONTIG,
            SEC_FLAG_ZERO_ON_ADOPT,
        ];
        let mut combined_sec = 0u32;
        for f in sec_flags {
            assert!(f.is_power_of_two());
            assert_eq!(combined_sec & f, 0);
            combined_sec |= f;
        }

        assert_eq!(HandoffError::InvalidMagic.as_str(), "invalid_magic");
        assert_eq!(HandoffError::VirtualRangeOverlap.as_str(), "virtual_range_overlap");
        assert_eq!(HandoffError::PhysicalRangeOverlap.as_str(), "physical_range_overlap");
        assert_eq!(KernelHandoffV1::default(), KernelHandoffV1::empty());
    }
}
