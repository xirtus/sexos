// SPDX-License-Identifier: GPL-3.0-only
//! A Drag cell: a VT-x guest with no guest operating system. Its memory is one block of
//! the host program's heap (`drag_vmm::memory::Ram`, lent to the VM at guest-physical 0);
//! this module carves that block into pages and builds everything the CPU needs to run
//! Windows user code in it:
//!
//! - 4-level page tables the host writes (the guest never edits them), user pages with
//!   the user bit, read-only and no-execute where the image or the caller asks;
//! - a GDT with Windows's x64 selectors (kernel code 0x10, data 0x18, user code 0x33,
//!   user data 0x2B, TSS 0x40), a TSS whose I/O bitmap opens only the hypercall port, and
//!   an IDT whose 32 exception gates lead to tiny ring-0 stubs;
//! - the stubs: each pushes the fault's CR2 and RAX and reports the vector to the host
//!   with `out HYPERCALL_PORT, eax`. The host reads the frame and decides (Windows
//!   exception dispatch happens in the host and in the guest's own user code, not in a
//!   guest kernel, because there is none).
//!
//! Hypercalls are `out 0xE0, eax` rather than `vmcall`: an I/O exit carries the same
//! registers on SexOS's SYS_VM and is also what Linux KVM hands to user space, so the
//! same cell runs in the CI harness (tests/kvm.rs). EAX says what the call is.
use alloc::vec::Vec;
use drag_vmm::memory::Ram;
use sex_pdx::vm::{Segment, State};

pub const PAGE: u64 = 4096;

/// The one I/O port a cell may use: every hypercall and every stub report goes here.
pub const HYPERCALL_PORT: u8 = 0xE0;
/// EAX of a call from an import thunk: `CALL | index`.
pub const CALL: u32 = 0x7C00_0000;
/// EAX of an exception stub: `EXCEPTION | vector`.
pub const EXCEPTION: u32 = 0x7E00_0000;
/// EAX when a vectored exception handler returns (its return address is this thunk).
pub const HANDLER_RETURN: u32 = 0x7F00_0000;
/// EAX when the program's entry point returns.
pub const ENTRY_RETURN: u32 = 0x7F00_0001;
pub const MAGIC_MASK: u32 = 0xFF00_0000;

/// Where the ring-0 pieces live (supervisor pages, top of the address space).
pub const SYS_VA: u64 = 0xFFFF_FF80_0000_0000;
const SYS_GDT: u64 = SYS_VA;
const SYS_IDT: u64 = SYS_VA + PAGE;
const SYS_TSS: u64 = SYS_VA + 2 * PAGE;
const SYS_STUBS: u64 = SYS_VA + 3 * PAGE;
const SYS_STACK: u64 = SYS_VA + 4 * PAGE;
const SYS_STACK_PAGES: u64 = 4;
/// Top of the ring-0 exception stack (IST1 and RSP0).
pub const SYS_STACK_TOP: u64 = SYS_STACK + SYS_STACK_PAGES * PAGE;
const STUB_SIZE: u64 = 32;

/// Windows x64 selectors.
pub const KERNEL_CS: u16 = 0x10;
pub const KERNEL_DS: u16 = 0x18;
pub const USER_CS32: u16 = 0x23;
pub const USER_DS: u16 = 0x2B;
pub const USER_CS: u16 = 0x33;
pub const TSS_SEL: u16 = 0x40;
const GDT_LIMIT: u32 = 10 * 8 - 1;
const IDT_LIMIT: u32 = 32 * 16 - 1;
/// TSS: 104 bytes, then an I/O bitmap for ports 0-255 and its terminating byte.
const TSS_IOMAP: u64 = 104;
const TSS_LIMIT: u32 = 104 + 32;

/// Highest user address (canonical lower half, below what Windows reserves).
pub const USER_TOP: u64 = 0x0000_7FFF_FFFE_FFFF;

/// Page-table entry bits.
const P: u64 = 1;
const RW: u64 = 2;
const US: u64 = 4;
const NX: u64 = 1 << 63;
const ADDR: u64 = 0x000F_FFFF_FFFF_F000;

/// What a mapping allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access { pub write: bool, pub execute: bool, pub user: bool }
impl Access {
    pub const USER_RO: Access = Access { write: false, execute: false, user: true };
    pub const USER_RW: Access = Access { write: true, execute: false, user: true };
    pub const USER_RX: Access = Access { write: false, execute: true, user: true };
    pub const USER_RWX: Access = Access { write: true, execute: true, user: true };
    const SYS_RW: Access = Access { write: true, execute: false, user: false };
    const SYS_RX: Access = Access { write: false, execute: true, user: false };
    fn bits(self) -> u64 { P | if self.write { RW } else { 0 } | if self.user { US } else { 0 } | if self.execute { 0 } else { NX } }
}

/// A run of pages, contiguous in guest-physical memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Map { pub va: u64, pub pages: u64, pub gpa: u64, pub access: Access }
impl Map {
    fn end(&self) -> u64 { self.va + self.pages * PAGE }
    fn contains(&self, va: u64) -> bool { va >= self.va && va < self.end() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The cell's memory block is used up.
    OutOfMemory,
    /// The range overlaps a mapping or reservation, or is not page-aligned or canonical.
    BadRange,
}

pub struct Cell {
    pub ram: Ram,
    next_gpa: u64,
    guest_root: u64,
    maps: Vec<Map>,
    /// Address ranges reserved without memory (Windows MEM_RESERVE): (va, bytes).
    reserved: Vec<(u64, u64)>,
}

fn overlaps(a: u64, alen: u64, b: u64, blen: u64) -> bool { a < b + blen && b < a + alen }

impl Cell {
    /// A cell over `ram`, with the ring-0 pieces in place.
    pub fn new(ram: Ram) -> Result<Cell, Error> {
        let mut c = Cell { ram, next_gpa: PAGE, guest_root: 0, maps: Vec::new(), reserved: Vec::new() };
        c.guest_root = c.page()?;
        c.build_system()?;
        Ok(c)
    }

    pub fn cr3(&self) -> u64 { self.guest_root }
    pub fn maps(&self) -> &[Map] { &self.maps }
    pub fn free_bytes(&self) -> u64 { self.ram.len().saturating_sub(self.next_gpa) }

    /// One zeroed guest-physical page.
    fn page(&mut self) -> Result<u64, Error> { self.pages(1) }
    fn pages(&mut self, n: u64) -> Result<u64, Error> {
        let gpa = self.next_gpa;
        let end = gpa.checked_add(n * PAGE).ok_or(Error::OutOfMemory)?;
        if end > self.ram.len() { return Err(Error::OutOfMemory); }
        self.ram.fill(gpa, (n * PAGE) as usize, 0).map_err(|_| Error::OutOfMemory)?;
        self.next_gpa = end;
        Ok(gpa)
    }

    /// The page-table entry for `va`, making the tables above it.
    fn pte(&mut self, va: u64) -> Result<u64, Error> {
        let mut table = self.guest_root;
        for level in (1..4).rev() {
            let slot = table + 8 * ((va >> (12 + 9 * level)) & 511);
            let e = self.ram.read_u64(slot).map_err(|_| Error::OutOfMemory)?;
            table = if e & P != 0 { e & ADDR } else {
                let t = self.page()?;
                self.ram.write_u64(slot, t | P | RW | US).map_err(|_| Error::OutOfMemory)?;
                t
            };
        }
        Ok(table + 8 * ((va >> 12) & 511))
    }

    fn canonical(va: u64) -> bool { va <= 0x0000_7FFF_FFFF_FFFF || va >= 0xFFFF_8000_0000_0000 }

    /// Is any part of [va, va+bytes) mapped?
    pub fn is_mapped(&self, va: u64, bytes: u64) -> bool { self.maps.iter().any(|m| overlaps(va, bytes, m.va, m.pages * PAGE)) }
    /// Is any part of [va, va+bytes) mapped or reserved?
    pub fn is_taken(&self, va: u64, bytes: u64) -> bool {
        self.is_mapped(va, bytes) || self.reserved.iter().any(|&(r, n)| overlaps(va, bytes, r, n))
    }
    /// Is [va, va+bytes) inside one reservation?
    pub fn in_reservation(&self, va: u64, bytes: u64) -> bool { self.reserved.iter().any(|&(r, n)| va >= r && va + bytes <= r + n) }

    /// Maps `pages` fresh zeroed pages at `va`. Returns their first guest-physical address.
    pub fn map(&mut self, va: u64, pages: u64, access: Access) -> Result<u64, Error> {
        let bytes = pages.checked_mul(PAGE).ok_or(Error::BadRange)?;
        let end = va.checked_add(bytes).ok_or(Error::BadRange)?;
        if pages == 0 || va % PAGE != 0 || !Self::canonical(va) || !Self::canonical(end - 1) { return Err(Error::BadRange); }
        if access.user && end > USER_TOP + 1 { return Err(Error::BadRange); }
        if self.is_mapped(va, bytes) { return Err(Error::BadRange); }
        let gpa = self.pages(pages)?;
        for i in 0..pages {
            let slot = self.pte(va + i * PAGE)?;
            self.ram.write_u64(slot, (gpa + i * PAGE) | access.bits()).map_err(|_| Error::OutOfMemory)?;
        }
        self.maps.push(Map { va, pages, gpa, access });
        Ok(gpa)
    }

    /// Reserves [va, va+bytes) without memory: any access faults.
    pub fn reserve(&mut self, va: u64, bytes: u64) -> Result<(), Error> {
        if bytes == 0 || va % PAGE != 0 || bytes % PAGE != 0 || va.checked_add(bytes).map_or(true, |e| e > USER_TOP + 1) { return Err(Error::BadRange); }
        if self.is_taken(va, bytes) { return Err(Error::BadRange); }
        self.reserved.push((va, bytes));
        Ok(())
    }

    /// Guest-physical address of `va` if mapped (and user-accessible when `user`).
    fn gpa_of(&self, va: u64, user: bool, write: bool) -> Option<u64> {
        let m = self.maps.iter().find(|m| m.contains(va))?;
        if (user && !m.access.user) || (write && !m.access.write) { return None; }
        Some(m.gpa + (va - m.va))
    }

    /// Reads guest memory the way user code at `va` could (for hypercall arguments).
    pub fn read_user(&self, va: u64, out: &mut [u8]) -> Option<()> { self.copy_out(va, out, true) }
    /// Writes guest memory as user code could, honouring read-only pages.
    pub fn write_user(&mut self, va: u64, data: &[u8]) -> Option<()> { self.copy_in(va, data, true) }
    /// Reads any mapped guest memory (the host's own pages: stacks, TEB, stubs).
    pub fn read(&self, va: u64, out: &mut [u8]) -> Option<()> { self.copy_out(va, out, false) }
    /// Writes any mapped guest memory, read-only or not (loading the image).
    pub fn write(&mut self, va: u64, data: &[u8]) -> Option<()> { self.copy_in(va, data, false) }

    fn copy_out(&self, va: u64, out: &mut [u8], user: bool) -> Option<()> {
        let mut done = 0usize;
        while done < out.len() {
            let at = va.checked_add(done as u64)?;
            let n = ((PAGE - at % PAGE) as usize).min(out.len() - done);
            let gpa = self.gpa_of(at, user, false)?;
            self.ram.read(gpa, &mut out[done..done + n]).ok()?;
            done += n;
        }
        Some(())
    }
    fn copy_in(&mut self, va: u64, data: &[u8], user: bool) -> Option<()> {
        // Check every page first, so a refused write changes nothing.
        let mut at = va;
        while at < va.checked_add(data.len() as u64)? {
            self.gpa_of(at, user, user)?;
            at = (at & !(PAGE - 1)) + PAGE;
        }
        let mut done = 0usize;
        while done < data.len() {
            let at = va + done as u64;
            let n = ((PAGE - at % PAGE) as usize).min(data.len() - done);
            let gpa = self.gpa_of(at, user, user)?;
            self.ram.write(gpa, &data[done..done + n]).ok()?;
            done += n;
        }
        Some(())
    }
    pub fn read_u64(&self, va: u64) -> Option<u64> { let mut b = [0; 8]; self.read(va, &mut b)?; Some(u64::from_le_bytes(b)) }
    pub fn write_u64(&mut self, va: u64, v: u64) -> Option<()> { self.write(va, &v.to_le_bytes()) }
    pub fn write_u32(&mut self, va: u64, v: u32) -> Option<()> { self.write(va, &v.to_le_bytes()) }

    fn build_system(&mut self) -> Result<(), Error> {
        self.map(SYS_GDT, 1, Access::SYS_RW)?;
        self.map(SYS_IDT, 1, Access::SYS_RW)?;
        self.map(SYS_TSS, 1, Access::SYS_RW)?;
        self.map(SYS_STUBS, 1, Access::SYS_RX)?;
        self.map(SYS_STACK, SYS_STACK_PAGES, Access::SYS_RW)?;
        let gdt: [u64; 10] = [
            0,
            0,
            0x00AF_9B00_0000_FFFF, // 0x10 kernel code, 64-bit
            0x00CF_9300_0000_FFFF, // 0x18 kernel data
            0x00CF_FB00_0000_FFFF, // 0x23 user code, 32-bit (WoW64 later)
            0x00CF_F300_0000_FFFF, // 0x2B user data
            0x00AF_FB00_0000_FFFF, // 0x33 user code, 64-bit
            0,
            tss_low(SYS_TSS, TSS_LIMIT),
            SYS_TSS >> 32,
        ];
        for (i, d) in gdt.iter().enumerate() { self.write_u64(SYS_GDT + 8 * i as u64, *d).ok_or(Error::OutOfMemory)?; }
        // TSS: RSP0 and IST1 both at the exception stack, the I/O bitmap opening only the
        // hypercall port to ring 3.
        self.write_u64(SYS_TSS + 4, SYS_STACK_TOP).ok_or(Error::OutOfMemory)?;
        self.write_u64(SYS_TSS + 0x24, SYS_STACK_TOP).ok_or(Error::OutOfMemory)?;
        self.write(SYS_TSS + 0x66, &(TSS_IOMAP as u16).to_le_bytes()).ok_or(Error::OutOfMemory)?;
        let mut bitmap = [0xFFu8; 33];
        bitmap[HYPERCALL_PORT as usize / 8] &= !(1 << (HYPERCALL_PORT % 8));
        self.write(SYS_TSS + TSS_IOMAP, &bitmap).ok_or(Error::OutOfMemory)?;
        for v in 0..32u8 {
            let stub = SYS_STUBS + v as u64 * STUB_SIZE;
            self.write(stub, &stub_code(v)).ok_or(Error::OutOfMemory)?;
            // #BP may be raised from ring 3 by int3 (DPL 3); the rest only by faults.
            let (lo, hi) = idt_gate(stub, if v == 3 { 0xEE } else { 0x8E });
            self.write_u64(SYS_IDT + 16 * v as u64, lo).ok_or(Error::OutOfMemory)?;
            self.write_u64(SYS_IDT + 16 * v as u64 + 8, hi).ok_or(Error::OutOfMemory)?;
        }
        Ok(())
    }

    /// The vCPU state for ring-3 64-bit code at `rip` with stack `rsp` and the TEB at `teb`.
    pub fn user_state(&self, rip: u64, rsp: u64, teb: u64) -> State {
        let seg = |selector: u16, access: u32, base: u64, limit: u32| Segment { base, limit, access, selector, _pad: [0; 3] };
        let data = seg(USER_DS, 0xC0F3, 0, 0xFFFF_FFFF);
        State {
            regs: sex_pdx::vm::Regs { rip, rsp, rflags: 0x202, ..Default::default() },
            cr0: 0x8001_0033, // PG | WP | NE | ET | MP | PE
            guest_cr3: self.guest_root,
            cr4: 0x620,       // OSXMMEXCPT | OSFXSR | PAE
            efer: 0xD00,      // NXE | LMA | LME
            segments: [
                data,                                     // ES
                seg(USER_CS, 0xA0FB, 0, 0xFFFF_FFFF),     // CS: 64-bit, DPL 3
                data,                                     // SS
                data,                                     // DS
                data,                                     // FS
                seg(USER_DS, 0xC0F3, teb, 0xFFFF_FFFF),   // GS: the TEB
                seg(0, 0x1_0000, 0, 0),                   // LDTR unusable
                seg(TSS_SEL, 0x8B, SYS_TSS, TSS_LIMIT),   // TR: busy 64-bit TSS
            ],
            gdtr_base: SYS_GDT, gdtr_limit: GDT_LIMIT,
            idtr_base: SYS_IDT, idtr_limit: IDT_LIMIT,
            interruptibility: 0, activity: 0,
        }
    }
}

fn tss_low(base: u64, limit: u32) -> u64 {
    (limit as u64 & 0xFFFF) | ((base & 0xFF_FFFF) << 16) | (0x89u64 << 40) | (((limit as u64 >> 16) & 0xF) << 48) | (((base >> 24) & 0xFF) << 56)
}

/// An interrupt gate to `target` in kernel code, on IST1.
fn idt_gate(target: u64, type_attr: u8) -> (u64, u64) {
    let lo = (target & 0xFFFF) | ((KERNEL_CS as u64) << 16) | (1u64 << 32) | ((type_attr as u64) << 40) | (((target >> 16) & 0xFFFF) << 48);
    (lo, target >> 32)
}

/// Vectors for which the CPU pushes an error code.
pub fn has_error_code(v: u8) -> bool { matches!(v, 8 | 10 | 11 | 12 | 13 | 14 | 17 | 21 | 29 | 30) }

/// The ring-0 stub for vector `v`: give every frame an error code slot, save RAX and
/// CR2, report to the host, and stop (the host always resumes elsewhere).
/// Frame at RSP when the host sees it: CR2, RAX, error, RIP, CS, RFLAGS, RSP, SS.
pub fn stub_code(v: u8) -> [u8; STUB_SIZE as usize] {
    let mut c = [0xCCu8; STUB_SIZE as usize];
    let mut n = 0;
    let mut put = |bytes: &[u8]| { c[n..n + bytes.len()].copy_from_slice(bytes); n += bytes.len(); };
    if !has_error_code(v) { put(&[0x6A, 0x00]); }          // push 0
    put(&[0x50]);                                           // push rax
    put(&[0x0F, 0x20, 0xD0]);                               // mov rax, cr2
    put(&[0x50]);                                           // push rax
    put(&[0xB8]); put(&(EXCEPTION | v as u32).to_le_bytes()); // mov eax, EXCEPTION | v
    put(&[0xE7, HYPERCALL_PORT]);                           // out 0xE0, eax
    put(&[0xF4, 0xEB, 0xFD]);                               // hlt; jmp to hlt
    c
}

/// The exception frame the stub leaves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame { pub cr2: u64, pub rax: u64, pub error: u64, pub rip: u64, pub cs: u64, pub rflags: u64, pub rsp: u64, pub ss: u64 }

impl Cell {
    /// Reads the frame at the stub's RSP. None if RSP is not on the exception stack.
    pub fn frame(&self, rsp: u64) -> Option<Frame> {
        if rsp < SYS_STACK || rsp + 64 > SYS_STACK_TOP { return None; }
        let q = |i: u64| self.read_u64(rsp + 8 * i);
        Some(Frame { cr2: q(0)?, rax: q(1)?, error: q(2)?, rip: q(3)?, cs: q(4)?, rflags: q(5)?, rsp: q(6)?, ss: q(7)? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drag_vmm::x86;

    fn cell() -> Cell { Cell::new(Ram::new(4 << 20).unwrap()).unwrap() }

    /// The leaf entry for `va`, by walking the tables the way the CPU does.
    fn leaf(c: &Cell, va: u64) -> Option<u64> {
        let mut table = c.cr3();
        for level in (0..4).rev() {
            let e = c.ram.read_u64(table + 8 * ((va >> (12 + 9 * level)) & 511)).ok()?;
            if e & P == 0 { return None; }
            if level == 0 { return Some(e); }
            table = e & ADDR;
        }
        None
    }

    #[test]
    fn page_tables_are_what_the_cpu_walks() {
        let mut c = cell();
        let gpa = c.map(0x1_4000_0000, 3, Access::USER_RX).unwrap();
        c.map(0x1000_0000, 1, Access::USER_RW).unwrap();
        let st = c.user_state(0x1_4000_0000, 0, 0);
        assert_eq!(x86::translate(&c.ram, &st, 0x1_4000_2123), Some(gpa + 0x2123), "drag's walker agrees");
        assert_eq!(x86::translate(&c.ram, &st, 0x1_4000_3000), None);
        let e = leaf(&c, 0x1_4000_0000).unwrap();
        assert_eq!((e & RW, e & US, e & NX), (0, US, 0), "code: user, read-only, executable");
        let e = leaf(&c, 0x1000_0000).unwrap();
        assert_eq!((e & RW, e & US, e & NX), (RW, US, NX), "data: user, writable, no-execute");
        let e = leaf(&c, SYS_STUBS).unwrap();
        assert_eq!((e & US, e & NX), (0, 0), "stubs: supervisor, executable");
        let e = leaf(&c, SYS_GDT).unwrap();
        assert_eq!((e & US, e & NX), (0, NX), "tables: supervisor, no-execute");
    }

    #[test]
    fn mappings_and_reservations_never_overlap() {
        let mut c = cell();
        c.map(0x20_0000, 4, Access::USER_RW).unwrap();
        assert_eq!(c.map(0x20_3000, 1, Access::USER_RW), Err(Error::BadRange));
        assert_eq!(c.reserve(0x20_0000, PAGE), Err(Error::BadRange));
        c.reserve(0x40_0000, 16 * PAGE).unwrap();
        assert!(c.in_reservation(0x40_1000, PAGE) && !c.in_reservation(0x3F_F000, 2 * PAGE));
        assert_eq!(c.map(0x123, 1, Access::USER_RW), Err(Error::BadRange), "unaligned");
        assert_eq!(c.map(0x0000_8000_0000_0000, 1, Access::USER_RW), Err(Error::BadRange), "not canonical");
        assert_eq!(c.map(USER_TOP & !0xFFF, 2, Access::USER_RW), Err(Error::BadRange), "user pages above the user range");
        assert_eq!(c.map(0x1000_0000, 100_000, Access::USER_RW), Err(Error::OutOfMemory));
    }

    #[test]
    fn user_copies_honour_page_rights() {
        let mut c = cell();
        c.map(0x50_0000, 1, Access::USER_RO).unwrap();
        c.map(0x50_1000, 1, Access::USER_RW).unwrap();
        assert!(c.write(0x50_0FFE, b"abcd").is_some(), "the host may write read-only pages");
        assert!(c.write_user(0x50_0FFE, b"xy").is_none(), "user writes to read-only pages are refused");
        assert!(c.write_user(0x50_0FFF, b"xy").is_none(), "even when only the first byte is read-only");
        let mut b = [0u8; 4];
        c.read_user(0x50_0FFE, &mut b).unwrap();
        assert_eq!(&b, b"abcd", "the refused write changed nothing");
        assert!(c.read_user(SYS_GDT, &mut b).is_none(), "supervisor pages are not user memory");
        assert!(c.read_user(0x50_2000, &mut b).is_none());
    }

    #[test]
    fn descriptors_and_gates() {
        let c = cell();
        let st = c.user_state(0x1000, 0x2000, 0x7000);
        assert_eq!(st.segments[1].selector & 3, 3, "CS is ring 3");
        assert_eq!((st.segments[2].access >> 5) & 3, 3, "SS.DPL (the CPL) is 3");
        assert_eq!(st.segments[5].base, 0x7000, "GS base is the TEB");
        assert_eq!(sex_pdx::vm::hw::descriptor_base(c.read_u64(SYS_GDT + 64).unwrap(), c.read_u64(SYS_GDT + 72).unwrap()), SYS_TSS);
        let lo = c.read_u64(SYS_IDT + 16 * 14).unwrap();
        let hi = c.read_u64(SYS_IDT + 16 * 14 + 8).unwrap();
        let target = (lo & 0xFFFF) | ((lo >> 48) << 16) | (hi << 32);
        assert_eq!(target, SYS_STUBS + 14 * STUB_SIZE);
        assert_eq!(((lo >> 16) & 0xFFFF, (lo >> 32) & 7, (lo >> 40) & 0xFF), (KERNEL_CS as u64, 1, 0x8E));
        assert_eq!((c.read_u64(SYS_IDT + 16 * 3).unwrap() >> 40) & 0xFF, 0xEE, "int3 allowed from ring 3");
        let mut port = [0u8; 1];
        c.read(SYS_TSS + TSS_IOMAP + 0xE0 / 8, &mut port).unwrap();
        assert_eq!(port[0], 0xFE, "only port 0xE0 is open");
        c.read(SYS_TSS + TSS_IOMAP + 32, &mut port).unwrap();
        assert_eq!(port[0], 0xFF, "bitmap terminator");
    }

    #[test]
    fn stubs_encode_as_intended() {
        let s = stub_code(13);
        assert_eq!(&s[..15], &[0x50, 0x0F, 0x20, 0xD0, 0x50, 0xB8, 13, 0, 0, 0x7E, 0xE7, 0xE0, 0xF4, 0xEB, 0xFD]);
        let s = stub_code(0);
        assert_eq!(&s[..3], &[0x6A, 0x00, 0x50], "no CPU error code: push a zero");
        // The stubs must not hold the key bytes the SexOS loader refuses (WRPKRU 0F 01 EF).
        for v in 0..32 { assert!(!stub_code(v).windows(3).any(|w| w == [0x0F, 0x01, 0xEF])); }
    }
}
