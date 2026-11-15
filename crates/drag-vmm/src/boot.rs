// SPDX-License-Identifier: GPL-3.0-only
//! Linux direct boot: the x86 boot protocol (Documentation/arch/x86/boot.rst) for a
//! bzImage, the way a boot loader does it, without firmware.
//!
//! The image is a setup area (boot sector with the setup header at 0x1F1, then
//! `setup_sects` sectors of real-mode code we never run) followed by the protected-mode
//! kernel. Drag copies the kernel to `code32_start` (1 MiB), builds the zero page
//! (`boot_params`) with the header, the E820 map, the command line and the initrd,
//! and enters the kernel directly:
//!
//! - 64-bit (protocol 2.12+ with `XLF_KERNEL_64`): long mode at load address + 0x200,
//!   identity-mapped first 4 GiB (2 MiB pages), a GDT with flat `__BOOT_CS` (0x10) and
//!   `__BOOT_DS` (0x18), interrupts off, RSI = zero page.
//! - 32-bit (protocol 2.06+): flat protected mode, paging off, at the load address,
//!   ESI = zero page, EBP/EDI/EBX zero.
//!
//! The image is untrusted input: every field is range-checked and nothing is written
//! outside the guest's RAM.
use crate::memory::{Fault, Ram};
use sex_pdx::vm::{Regs, Segment, State};

/// Guest-physical layout (the low pages, below the EBDA).
pub const GDT: u64 = 0x500;
pub const ZERO_PAGE: u64 = 0x7000;
pub const STACK_TOP: u64 = 0x8FF0;
pub const GUEST_PML4: u64 = 0x9000;
pub const PDPT: u64 = 0xA000;
/// Four page directories, 0xB000..0xF000: 4 GiB of 2 MiB pages.
pub const PD: u64 = 0xB000;
pub const CMDLINE: u64 = 0x20000;
/// Most command-line bytes Drag places (the kernel's own `cmdline_size` may be less).
pub const CMDLINE_MAX: usize = 4096;
pub const EBDA: u64 = 0x9FC00;
pub const BIOS: u64 = 0xF0000;
pub const HIGH: u64 = 0x10_0000;
/// Guest RAM stays below this (the MMIO hole: virtio-mmio and friends live above).
pub const RAM_LIMIT: u64 = 0xC000_0000;
pub const SELECTOR_CODE: u16 = 0x10;
pub const SELECTOR_DATA: u16 = 0x18;

// Setup header offsets (in the image and in the zero page).
const SETUP_SECTS: usize = 0x1F1;
const VID_MODE: usize = 0x1FA;
const BOOT_FLAG: usize = 0x1FE;
const JUMP: usize = 0x200;
const HEADER: usize = 0x202;
const VERSION: usize = 0x206;
const TYPE_OF_LOADER: usize = 0x210;
const LOADFLAGS: usize = 0x211;
const CODE32_START: usize = 0x214;
const RAMDISK_IMAGE: usize = 0x218;
const RAMDISK_SIZE: usize = 0x21C;
const CMD_LINE_PTR: usize = 0x228;
const INITRD_ADDR_MAX: usize = 0x22C;
const RELOCATABLE: usize = 0x234;
const XLOADFLAGS: usize = 0x236;
const CMDLINE_SIZE: usize = 0x238;
const INIT_SIZE: usize = 0x260;
/// Zero-page fields outside the header.
const E820_ENTRIES: u64 = 0x1E8;
const E820_TABLE: u64 = 0x2D0;
const E820_MAX: usize = 128;
const EXT_CMD_LINE_PTR: u64 = 0x0C8;

const LOADED_HIGH: u8 = 1;
const XLF_KERNEL_64: u16 = 1;
pub const E820_RAM: u32 = 1;
pub const E820_RESERVED: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Shorter than its own header says, or than a setup header.
    Truncated,
    /// No 0xAA55 boot flag or no "HdrS" magic: not a bzImage.
    NotBzImage,
    /// Boot protocol older than 2.06, or not a big kernel (zImage).
    Protocol(u16),
    /// The command line is longer than the kernel or Drag accepts.
    CommandLine,
    /// The kernel, its init_size, or the initrd does not fit in the guest's RAM.
    TooBig,
    /// The load address is below 1 MiB or not page-aligned.
    LoadAddress(u64),
    Memory(Fault),
}
impl From<Fault> for Error { fn from(f: Fault) -> Error { Error::Memory(f) } }

/// What the setup header of a bzImage says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u16,
    pub setup_bytes: usize,
    pub load_address: u64,
    pub entry64: bool,
    pub relocatable: bool,
    pub cmdline_size: u32,
    pub initrd_addr_max: u32,
    pub init_size: u32,
    /// Bytes of the header that go to the zero page (0x1F1 up to where the jump lands).
    pub header_end: usize,
}

fn u16_at(d: &[u8], o: usize) -> u16 { d.get(o..o + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]])) }
fn u32_at(d: &[u8], o: usize) -> u32 { d.get(o..o + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) }

impl Header {
    pub fn parse(image: &[u8]) -> Result<Header, Error> {
        if image.len() < 0x250 { return Err(Error::Truncated); }
        if u16_at(image, BOOT_FLAG) != 0xAA55 || &image[HEADER..HEADER + 4] != b"HdrS" { return Err(Error::NotBzImage); }
        let version = u16_at(image, VERSION);
        if version < 0x0206 || image[LOADFLAGS] & LOADED_HIGH == 0 { return Err(Error::Protocol(version)); }
        let sects = match image[SETUP_SECTS] { 0 => 4, n => n as usize };
        let setup_bytes = (sects + 1) * 512;
        if image.len() <= setup_bytes { return Err(Error::Truncated); }
        let code32 = u32_at(image, CODE32_START) as u64;
        let load_address = if code32 == 0 { HIGH } else { code32 };
        // The jump at 0x200 lands just past the header; never past what we know.
        let header_end = (JUMP + 2 + image[JUMP + 1] as usize).clamp(HEADER + 4, 0x290).min(image.len());
        Ok(Header {
            version,
            setup_bytes,
            load_address,
            entry64: version >= 0x020C && u16_at(image, XLOADFLAGS) & XLF_KERNEL_64 != 0,
            relocatable: image[RELOCATABLE] != 0,
            cmdline_size: u32_at(image, CMDLINE_SIZE),
            initrd_addr_max: if version >= 0x0203 { u32_at(image, INITRD_ADDR_MAX) } else { 0x37FF_FFFF },
            init_size: if version >= 0x020A { u32_at(image, INIT_SIZE) } else { 0 },
            header_end,
        })
    }
}

/// One E820 entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct E820 { pub addr: u64, pub size: u64, pub kind: u32 }

/// The memory map Drag reports for `ram` bytes of RAM at 0.
pub fn e820(ram: u64) -> [E820; 4] {
    [
        E820 { addr: 0, size: EBDA, kind: E820_RAM },
        E820 { addr: EBDA, size: 0xA0000 - EBDA, kind: E820_RESERVED },
        E820 { addr: BIOS, size: HIGH - BIOS, kind: E820_RESERVED },
        E820 { addr: HIGH, size: ram.saturating_sub(HIGH), kind: E820_RAM },
    ]
}

/// What `load` put where, and how to enter it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Boot {
    pub header: Header,
    pub entry: State,
    pub kernel: (u64, u64),
    pub initrd: Option<(u64, u64)>,
}

/// Loads `image` (a bzImage), `initrd` and `cmdline` into `ram` and returns the vCPU
/// state that enters the kernel.
pub fn load(ram: &mut Ram, image: &[u8], initrd: Option<&[u8]>, cmdline: &[u8]) -> Result<Boot, Error> {
    let h = Header::parse(image)?;
    let mem = ram.len().min(RAM_LIMIT);
    if mem < 2 * HIGH { return Err(Error::TooBig); }
    if h.load_address < HIGH || h.load_address % crate::memory::PAGE != 0 { return Err(Error::LoadAddress(h.load_address)); }
    let kernel = &image[h.setup_bytes..];
    let span = (kernel.len() as u64).max(h.init_size as u64);
    let kernel_end = h.load_address.checked_add(span).ok_or(Error::TooBig)?;
    if kernel_end > mem { return Err(Error::TooBig); }

    // The command line ends in NUL, within the kernel's limit and ours.
    let limit = (h.cmdline_size as usize).min(CMDLINE_MAX - 1);
    if cmdline.len() > limit || cmdline.contains(&0) { return Err(Error::CommandLine); }

    // initrd: as high as the kernel allows, page-aligned, above the kernel.
    let initrd_at = match initrd {
        None | Some([]) => None,
        Some(data) => {
            let top = mem.min(h.initrd_addr_max as u64 + 1);
            let at = top.checked_sub(data.len() as u64).ok_or(Error::TooBig)? & !(crate::memory::PAGE - 1);
            if at < kernel_end { return Err(Error::TooBig); }
            Some((at, data.len() as u64))
        }
    };

    ram.fill(0, HIGH as usize, 0)?;
    ram.write(h.load_address, kernel)?;
    if let (Some((at, _)), Some(data)) = (initrd_at, initrd) { ram.write(at, data)?; }
    ram.write(CMDLINE, cmdline)?;
    ram.write_u8(CMDLINE + cmdline.len() as u64, 0)?;

    // The zero page: the header as the image has it, then what a loader fills in.
    ram.write(ZERO_PAGE + SETUP_SECTS as u64, &image[SETUP_SECTS..h.header_end])?;
    let zp = |o: usize| ZERO_PAGE + o as u64;
    ram.write_u16(zp(VID_MODE), 0xFFFF)?;
    ram.write_u8(zp(TYPE_OF_LOADER), 0xFF)?;
    ram.write_u8(zp(LOADFLAGS), image[LOADFLAGS] | LOADED_HIGH)?;
    ram.write_u32(zp(CODE32_START), h.load_address as u32)?;
    ram.write_u32(zp(CMD_LINE_PTR), CMDLINE as u32)?;
    ram.write_u32(ZERO_PAGE + EXT_CMD_LINE_PTR, 0)?;
    let (rd_at, rd_size) = initrd_at.unwrap_or((0, 0));
    ram.write_u32(zp(RAMDISK_IMAGE), rd_at as u32)?;
    ram.write_u32(zp(RAMDISK_SIZE), rd_size as u32)?;
    let map = e820(mem);
    debug_assert!(map.len() <= E820_MAX);
    for (i, e) in map.iter().enumerate() {
        let at = ZERO_PAGE + E820_TABLE + i as u64 * 20;
        ram.write_u64(at, e.addr)?;
        ram.write_u64(at + 8, e.size)?;
        ram.write_u32(at + 16, e.kind)?;
    }
    ram.write_u8(ZERO_PAGE + E820_ENTRIES, map.len() as u8)?;

    let entry = if h.entry64 {
        tables(ram)?;
        gdt(ram, 0x00AF_9B00_0000_FFFF)?;
        long_mode(h.load_address + 0x200)
    } else {
        gdt(ram, 0x00CF_9B00_0000_FFFF)?;
        protected_mode(h.load_address)
    };
    Ok(Boot { header: h, entry, kernel: (h.load_address, kernel.len() as u64), initrd: initrd_at })
}

/// Identity map of the first 4 GiB with 2 MiB pages.
fn tables(ram: &mut Ram) -> Result<(), Fault> {
    ram.fill(GUEST_PML4, 6 * 4096, 0)?;
    ram.write_u64(GUEST_PML4, PDPT | 3)?;
    for i in 0..4u64 {
        ram.write_u64(PDPT + i * 8, (PD + i * 4096) | 3)?;
    }
    for i in 0..2048u64 {
        ram.write_u64(PD + i * 8, (i << 21) | 0x83)?;
    }
    Ok(())
}

fn gdt(ram: &mut Ram, code: u64) -> Result<(), Fault> {
    ram.write_u64(GDT, 0)?;
    ram.write_u64(GDT + 8, 0)?;
    ram.write_u64(GDT + 16, code)?;
    ram.write_u64(GDT + 24, 0x00CF_9300_0000_FFFF)
}

fn flat(selector: u16, access: u32) -> Segment { Segment { base: 0, limit: 0xFFFF_FFFF, access, selector, _pad: [0; 3] } }

fn segments(code_access: u32) -> [Segment; 8] {
    let data = flat(SELECTOR_DATA, 0xC093);
    [data, flat(SELECTOR_CODE, code_access), data, data, data, data,
        Segment { base: 0, limit: 0, access: 0x1_0000, selector: 0, _pad: [0; 3] }, // LDTR unusable
        Segment { base: 0, limit: 0xFFFF, access: 0x8B, selector: 0, _pad: [0; 3] }] // TR: busy TSS
}

/// The vCPU at a 64-bit boot-protocol entry.
pub fn long_mode(rip: u64) -> State {
    State {
        regs: Regs { rip, rsi: ZERO_PAGE, rsp: STACK_TOP, rflags: 2, ..Regs::default() },
        cr0: 0x8000_0031, // PG | NE | ET | PE
        guest_cr3: GUEST_PML4,
        cr4: 0x20, // PAE
        efer: 0x500, // LME | LMA
        segments: segments(0xA09B),
        gdtr_base: GDT, gdtr_limit: 31,
        ..State::default()
    }
}

/// The vCPU at the 32-bit boot-protocol entry.
pub fn protected_mode(eip: u64) -> State {
    State {
        regs: Regs { rip: eip, rsi: ZERO_PAGE, rflags: 2, ..Regs::default() },
        cr0: 0x31, // NE | ET | PE
        segments: segments(0xC09B),
        gdtr_base: GDT, gdtr_limit: 31,
        ..State::default()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// A synthetic bzImage: a setup header of `version`, `sects` setup sectors and
    /// `payload` as the kernel.
    pub fn bzimage(version: u16, sects: u8, xload: u16, payload: &[u8]) -> Vec<u8> {
        let setup = (if sects == 0 { 4 } else { sects as usize } + 1) * 512;
        let mut d = vec![0u8; setup];
        d[SETUP_SECTS] = sects;
        d[BOOT_FLAG..BOOT_FLAG + 2].copy_from_slice(&0xAA55u16.to_le_bytes());
        d[JUMP] = 0xEB;
        d[JUMP + 1] = 0x6A;
        d[HEADER..HEADER + 4].copy_from_slice(b"HdrS");
        d[VERSION..VERSION + 2].copy_from_slice(&version.to_le_bytes());
        d[LOADFLAGS] = LOADED_HIGH;
        d[CODE32_START..CODE32_START + 4].copy_from_slice(&0x10_0000u32.to_le_bytes());
        d[INITRD_ADDR_MAX..INITRD_ADDR_MAX + 4].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        d[RELOCATABLE] = 1;
        d[XLOADFLAGS..XLOADFLAGS + 2].copy_from_slice(&xload.to_le_bytes());
        d[CMDLINE_SIZE..CMDLINE_SIZE + 4].copy_from_slice(&2047u32.to_le_bytes());
        d[INIT_SIZE..INIT_SIZE + 4].copy_from_slice(&0x20_0000u32.to_le_bytes());
        d.extend_from_slice(payload);
        d
    }

    #[test]
    fn parses_a_64_bit_header() {
        let img = bzimage(0x020F, 2, 1, &[0x90; 0x400]);
        let h = Header::parse(&img).unwrap();
        assert_eq!((h.version, h.setup_bytes, h.load_address, h.entry64), (0x020F, 3 * 512, HIGH, true));
        assert_eq!(h.header_end, 0x26C);
        assert_eq!(h.init_size, 0x20_0000);
        let old = Header::parse(&bzimage(0x020B, 0, 1, &[0; 16])).unwrap();
        assert!(!old.entry64, "64-bit entry needs 2.12");
        assert_eq!(old.setup_bytes, 5 * 512, "setup_sects 0 means 4");
    }

    #[test]
    fn refuses_bad_images() {
        assert_eq!(Header::parse(&[0; 100]), Err(Error::Truncated));
        let mut img = bzimage(0x020F, 2, 1, &[0; 16]);
        img[HEADER] = b'X';
        assert_eq!(Header::parse(&img), Err(Error::NotBzImage));
        assert_eq!(Header::parse(&bzimage(0x0205, 2, 1, &[0; 16])), Err(Error::Protocol(0x0205)));
        let mut z = bzimage(0x020F, 2, 1, &[0; 16]);
        z[LOADFLAGS] = 0;
        assert_eq!(Header::parse(&z), Err(Error::Protocol(0x020F)), "zImage");
        let mut short = bzimage(0x020F, 2, 1, &[]);
        short.truncate(3 * 512);
        assert_eq!(Header::parse(&short), Err(Error::Truncated), "no kernel after setup");
        let mut huge_sects = bzimage(0x020F, 2, 1, &[0; 16]);
        huge_sects[SETUP_SECTS] = 255;
        assert_eq!(Header::parse(&huge_sects), Err(Error::Truncated));
    }

    #[test]
    fn loads_kernel_zero_page_e820_cmdline_and_initrd() {
        let mut ram = Ram::new(32 << 20).unwrap();
        let img = bzimage(0x020F, 2, 1, b"KERNEL-PAYLOAD");
        let initrd = [7u8; 5000];
        let b = load(&mut ram, &img, Some(&initrd), b"console=ttyS0").unwrap();
        assert_eq!(ram.slice(HIGH, 14).unwrap(), b"KERNEL-PAYLOAD");
        assert_eq!(ram.slice(ZERO_PAGE + 0x202, 4).unwrap(), b"HdrS");
        assert_eq!(ram.read_u8(ZERO_PAGE + 0x210), Ok(0xFF), "type_of_loader");
        assert_eq!(ram.read_u32(ZERO_PAGE + 0x228), Ok(CMDLINE as u32));
        assert_eq!(ram.slice(CMDLINE, 14).unwrap(), b"console=ttyS0\0");
        let (at, size) = b.initrd.unwrap();
        assert_eq!((at % 4096, size, at + 4096 > (32 << 20) - 5000), (0, 5000, true), "at the top, page-aligned");
        assert_eq!(ram.read_u32(ZERO_PAGE + 0x218), Ok(at as u32));
        assert_eq!(ram.read_u32(ZERO_PAGE + 0x21C), Ok(5000));
        assert_eq!(ram.slice(at, 5000).unwrap(), &initrd[..]);
        assert_eq!(ram.read_u8(ZERO_PAGE + 0x1E8), Ok(4));
        let e = |i: u64| (ram.read_u64(ZERO_PAGE + 0x2D0 + i * 20).unwrap(), ram.read_u64(ZERO_PAGE + 0x2D8 + i * 20).unwrap(), ram.read_u32(ZERO_PAGE + 0x2E0 + i * 20).unwrap());
        assert_eq!(e(0), (0, 0x9FC00, 1));
        assert_eq!(e(3), (0x10_0000, (32 << 20) - 0x10_0000, 1));
        // 64-bit entry: long mode, identity tables, boot GDT, RSI = zero page.
        let s = b.entry;
        assert_eq!((s.regs.rip, s.regs.rsi, s.guest_cr3, s.efer), (HIGH + 0x200, ZERO_PAGE, GUEST_PML4, 0x500));
        assert_eq!((s.segments[1].selector, s.segments[1].access & 0x2000), (0x10, 0x2000), "CS = __BOOT_CS, L=1");
        assert_eq!(s.segments[2].selector, 0x18);
        assert_eq!(ram.read_u64(GDT + 16), Ok(0x00AF_9B00_0000_FFFF));
        assert_eq!(ram.read_u64(GUEST_PML4), Ok(PDPT | 3));
        assert_eq!(ram.read_u64(PDPT + 24), Ok((PD + 3 * 4096) | 3));
        assert_eq!(ram.read_u64(PD + 2047 * 8), Ok((2047 << 21) | 0x83), "4 GiB mapped");
    }

    #[test]
    fn thirty_two_bit_entry_for_older_kernels() {
        let mut ram = Ram::new(8 << 20).unwrap();
        let b = load(&mut ram, &bzimage(0x020A, 2, 0, &[0xF4]), None, b"").unwrap();
        assert_eq!((b.entry.regs.rip, b.entry.cr0 & 0x8000_0001, b.entry.efer), (HIGH, 1, 0));
        assert_eq!(b.entry.segments[1].access, 0xC09B);
        assert_eq!(ram.read_u64(GDT + 16), Ok(0x00CF_9B00_0000_FFFF));
        assert_eq!(ram.read_u32(ZERO_PAGE + 0x218), Ok(0), "no initrd");
    }

    #[test]
    fn hostile_sizes_and_addresses_are_refused() {
        let mut ram = Ram::new(4 << 20).unwrap();
        let img = bzimage(0x020F, 2, 1, &[0; 16]);
        assert_eq!(load(&mut ram, &img, None, &[b'a'; 2048]), Err(Error::CommandLine));
        assert_eq!(load(&mut ram, &img, None, b"a\0b"), Err(Error::CommandLine));
        let mut big_init = img.clone();
        big_init[INIT_SIZE..INIT_SIZE + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(load(&mut ram, &big_init, None, b""), Err(Error::TooBig));
        assert_eq!(load(&mut ram, &img, Some(&vec![0; 3 << 20]), b""), Err(Error::TooBig), "initrd over the kernel");
        let mut low = img.clone();
        low[CODE32_START..CODE32_START + 4].copy_from_slice(&0x8000u32.to_le_bytes());
        assert_eq!(load(&mut ram, &low, None, b""), Err(Error::LoadAddress(0x8000)));
        let mut high = img.clone();
        high[CODE32_START..CODE32_START + 4].copy_from_slice(&0xFFFF_F000u32.to_le_bytes());
        assert_eq!(load(&mut ram, &high, None, b""), Err(Error::TooBig));
        let mut tiny = Ram::new(1 << 20).unwrap();
        assert_eq!(load(&mut tiny, &img, None, b""), Err(Error::TooBig));
    }

    #[test]
    fn the_in_tree_test_guest_is_a_bzimage() {
        let h = Header::parse(crate::TEST_GUEST).unwrap();
        assert_eq!((h.version, h.setup_bytes, h.load_address, h.entry64, h.relocatable), (0x020F, 0x600, HIGH, true, false));
    }
}
