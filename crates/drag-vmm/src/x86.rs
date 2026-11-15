// SPDX-License-Identifier: GPL-3.0-only
//! What the VMM needs to read a guest's x86: its general registers by encoding number,
//! guest page-table walks (linear to guest-physical), instruction fetch, and a decoder
//! for the instructions that touch MMIO: `mov` loads and stores (88, 89, 8A, 8B, C6,
//! C7) and `movzx`/`movsx` (0F B6, B7, BE, BF), with operand-size, address-size,
//! segment and lock prefixes and REX. That covers Linux's `readb..readq` and
//! `writeb..writeq`. Anything else is refused, and the VMM stops the guest with the
//! bytes in its log rather than guessing.
use crate::memory::Ram;
use sex_pdx::vm::{Regs, State};

/// A general register by its x86 encoding number (0 = RAX, 1 = RCX, ... 15 = R15).
pub fn gpr(r: &mut Regs, n: u8) -> &mut u64 {
    match n & 15 {
        0 => &mut r.rax, 1 => &mut r.rcx, 2 => &mut r.rdx, 3 => &mut r.rbx,
        4 => &mut r.rsp, 5 => &mut r.rbp, 6 => &mut r.rsi, 7 => &mut r.rdi,
        8 => &mut r.r8, 9 => &mut r.r9, 10 => &mut r.r10, 11 => &mut r.r11,
        12 => &mut r.r12, 13 => &mut r.r13, 14 => &mut r.r14, _ => &mut r.r15,
    }
}

const CR0_PG: u64 = 1 << 31;
const CR4_PSE: u64 = 1 << 4;
const CR4_PAE: u64 = 1 << 5;
const CR4_LA57: u64 = 1 << 12;
const EFER_LMA: u64 = 1 << 10;
const ADDR: u64 = 0x000F_FFFF_FFFF_F000;

/// The vCPU runs 64-bit code (IA-32e mode and CS.L).
pub fn long_mode(st: &State) -> bool { st.efer & EFER_LMA != 0 && st.segments[1].access & 0x2000 != 0 }
/// CS default operand size is 32 bits (CS.D), or 64-bit code.
pub fn default32(st: &State) -> bool { long_mode(st) || st.segments[1].access & 0x4000 != 0 }

/// Guest-physical address of `linear` under the guest's paging, or None (not present).
pub fn translate(ram: &Ram, st: &State, linear: u64) -> Option<u64> {
    if st.cr0 & CR0_PG == 0 { return Some(linear & 0xFFFF_FFFF); }
    if st.efer & EFER_LMA != 0 {
        let levels = if st.cr4 & CR4_LA57 != 0 { 5 } else { 4 };
        let mut table = st.guest_cr3 & ADDR;
        for level in (0..levels).rev() {
            let shift = 12 + 9 * level;
            let e = ram.read_u64(table + 8 * ((linear >> shift) & 511)).ok()?;
            if e & 1 == 0 { return None; }
            if level == 0 || (level <= 2 && e & 0x80 != 0) {
                let page = (1u64 << shift) - 1;
                return Some((e & ADDR & !page) | (linear & page));
            }
            table = e & ADDR;
        }
        None
    } else if st.cr4 & CR4_PAE != 0 {
        let pdpte = ram.read_u64((st.guest_cr3 & 0xFFFF_FFE0) + 8 * ((linear >> 30) & 3)).ok()?;
        if pdpte & 1 == 0 { return None; }
        let pde = ram.read_u64((pdpte & ADDR) + 8 * ((linear >> 21) & 511)).ok()?;
        if pde & 1 == 0 { return None; }
        if pde & 0x80 != 0 { return Some((pde & ADDR & !0x1F_FFFF) | (linear & 0x1F_FFFF)); }
        let pte = ram.read_u64((pde & ADDR) + 8 * ((linear >> 12) & 511)).ok()?;
        if pte & 1 == 0 { return None; }
        Some((pte & ADDR) | (linear & 0xFFF))
    } else {
        let linear = linear & 0xFFFF_FFFF;
        let pde = ram.read_u32((st.guest_cr3 & 0xFFFF_F000) + 4 * (linear >> 22)).ok()? as u64;
        if pde & 1 == 0 { return None; }
        if pde & 0x80 != 0 && st.cr4 & CR4_PSE != 0 { return Some((pde & 0xFFC0_0000) | (linear & 0x3F_FFFF)); }
        let pte = ram.read_u32((pde & 0xFFFF_F000) + 4 * ((linear >> 12) & 1023)).ok()? as u64;
        if pte & 1 == 0 { return None; }
        Some((pte & 0xFFFF_F000) | (linear & 0xFFF))
    }
}

/// Reads up to 15 instruction bytes at the vCPU's RIP; returns how many could be read.
pub fn fetch(ram: &Ram, st: &State, out: &mut [u8; 15]) -> usize {
    let base = if long_mode(st) { 0 } else { st.segments[1].base };
    let rip = base.wrapping_add(st.regs.rip);
    for (i, b) in out.iter_mut().enumerate() {
        match translate(ram, st, rip.wrapping_add(i as u64)).and_then(|pa| ram.read_u8(pa).ok()) {
            Some(v) => *b = v,
            None => return i,
        }
    }
    15
}

/// Reads `out.len()` bytes at a guest-linear address (string I/O), page by page.
pub fn read_linear(ram: &Ram, st: &State, linear: u64, out: &mut [u8]) -> Option<()> {
    for (i, b) in out.iter_mut().enumerate() {
        *b = ram.read_u8(translate(ram, st, linear.wrapping_add(i as u64))?).ok()?;
    }
    Some(())
}
pub fn write_linear(ram: &mut Ram, st: &State, linear: u64, data: &[u8]) -> Option<()> {
    for (i, &b) in data.iter().enumerate() {
        let pa = translate(ram, st, linear.wrapping_add(i as u64))?;
        ram.write_u8(pa, b).ok()?;
    }
    Some(())
}

/// An MMIO instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mmio {
    /// Instruction length.
    pub len: u8,
    /// Bytes the memory access moves.
    pub size: u8,
    pub op: Op,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// Memory into register `reg` (AH..BH when `high`), writing `dest` bytes of it,
    /// zero- or sign-extended from `size`.
    Load { reg: u8, high: bool, dest: u8, sign: bool },
    /// Register `reg` (or AH..BH) into memory.
    StoreReg { reg: u8, high: bool },
    StoreImm(u64),
}

/// Decodes `bytes` (at most 15) as an MMIO access. `long`: 64-bit code; `default32`:
/// 32-bit default operand and address size (CS.D, or long mode).
pub fn decode(bytes: &[u8], long: bool, default32: bool) -> Option<Mmio> {
    let bytes = &bytes[..bytes.len().min(15)];
    let mut i = 0;
    let (mut opsize, mut addrsize) = (false, false);
    loop {
        match *bytes.get(i)? {
            0x66 => opsize = true,
            0x67 => addrsize = true,
            0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 | 0xF0 => {}
            _ => break,
        }
        i += 1;
    }
    let mut rex = 0u8;
    if long && (0x40..=0x4F).contains(bytes.get(i)?) { rex = bytes[i]; i += 1; }
    let mut op = *bytes.get(i)? as u16;
    i += 1;
    if op == 0x0F { op = 0x0F00 | *bytes.get(i)? as u16; i += 1; }
    let modrm = *bytes.get(i)?;
    i += 1;
    let (md, rm) = (modrm >> 6, modrm & 7);
    if md == 3 { return None; }
    let reg = (modrm >> 3) & 7 | (rex & 4) << 1;
    let addr16 = !long && default32 == addrsize;
    let disp = if addr16 {
        match (md, rm) { (0, 6) => 2, (1, _) => 1, (2, _) => 2, _ => 0 }
    } else {
        if rm == 4 {
            let sib = *bytes.get(i)?;
            i += 1;
            if md == 0 && sib & 7 == 5 { i += 4; }
        }
        match (md, rm) { (0, 5) => 4, (1, _) => 1, (2, _) => 4, _ => 0 }
    };
    i += disp;
    let wide: u8 = if rex & 8 != 0 { 8 } else if default32 != opsize { 4 } else { 2 };
    let byte_reg = |r: u8| if rex == 0 && (4..8).contains(&r) { (r - 4, true) } else { (r, false) };
    let imm = |at: usize, n: usize| -> Option<u64> {
        let b = bytes.get(at..at + n)?;
        let mut v = [0u8; 8];
        v[..n].copy_from_slice(b);
        Some(u64::from_le_bytes(v))
    };
    let (size, op, extra) = match op {
        0x88 => { let (r, h) = byte_reg(reg); (1, Op::StoreReg { reg: r, high: h }, 0) }
        0x89 => (wide, Op::StoreReg { reg, high: false }, 0),
        0x8A => { let (r, h) = byte_reg(reg); (1, Op::Load { reg: r, high: h, dest: 1, sign: false }, 0) }
        0x8B => (wide, Op::Load { reg, high: false, dest: wide, sign: false }, 0),
        0xC6 if reg & 7 == 0 => (1, Op::StoreImm(imm(i, 1)?), 1),
        0xC7 if reg & 7 == 0 => {
            let n = if wide == 2 { 2 } else { 4 };
            let v = imm(i, n)?;
            let v = if wide == 8 { v as u32 as i32 as i64 as u64 } else { v };
            (wide, Op::StoreImm(v), n)
        }
        0x0FB6 | 0x0FB7 | 0x0FBE | 0x0FBF => {
            let size = if op & 1 == 0 { 1 } else { 2 };
            (size, Op::Load { reg, high: false, dest: wide, sign: op & 8 != 0 }, 0)
        }
        _ => return None,
    };
    let len = i + extra;
    if len > bytes.len() { return None; }
    Some(Mmio { len: len as u8, size, op })
}

fn mask(size: u8) -> u64 { if size >= 8 { u64::MAX } else { (1u64 << (size as u32 * 8)) - 1 } }

/// The value a store writes.
pub fn store_value(regs: &mut Regs, m: &Mmio) -> u64 {
    match m.op {
        Op::StoreReg { reg, high } => (if high { *gpr(regs, reg) >> 8 } else { *gpr(regs, reg) }) & mask(m.size),
        Op::StoreImm(v) => v & mask(m.size),
        Op::Load { .. } => 0,
    }
}

/// Puts a loaded `value` in its register, as the CPU would.
pub fn complete_load(regs: &mut Regs, m: &Mmio, value: u64) {
    let Op::Load { reg, high, dest, sign } = m.op else { return };
    let mut v = value & mask(m.size);
    if sign && m.size < 8 {
        let bits = m.size as u32 * 8;
        v = (((v << (64 - bits)) as i64) >> (64 - bits)) as u64;
    }
    let r = gpr(regs, reg);
    *r = match (dest, high) {
        (1, true) => (*r & !0xFF00) | (v & 0xFF) << 8,
        (1, false) => (*r & !0xFF) | (v & 0xFF),
        (2, _) => (*r & !0xFFFF) | (v & 0xFFFF),
        (4, _) => v & 0xFFFF_FFFF, // a 32-bit write clears the upper half
        _ => v,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(bytes: &[u8]) -> Mmio { decode(bytes, true, true).unwrap() }

    #[test]
    fn linux_mmio_accessors() {
        // writel: mov %eax,(%rdx)  readl: mov (%rdx),%eax
        assert_eq!(d(&[0x89, 0x02]), Mmio { len: 2, size: 4, op: Op::StoreReg { reg: 0, high: false } });
        assert_eq!(d(&[0x8B, 0x02]), Mmio { len: 2, size: 4, op: Op::Load { reg: 0, high: false, dest: 4, sign: false } });
        // writew: mov %ax,(%rdx); readb into %al via movzbl
        assert_eq!(d(&[0x66, 0x89, 0x02]).size, 2);
        assert_eq!(d(&[0x0F, 0xB6, 0x02]), Mmio { len: 3, size: 1, op: Op::Load { reg: 0, high: false, dest: 4, sign: false } });
        // writeq: mov %rax,0x10(%rdi); mov %r9d,0x100(%rbx)
        assert_eq!(d(&[0x48, 0x89, 0x47, 0x10]), Mmio { len: 4, size: 8, op: Op::StoreReg { reg: 0, high: false } });
        assert_eq!(d(&[0x44, 0x89, 0x8B, 0, 1, 0, 0]), Mmio { len: 7, size: 4, op: Op::StoreReg { reg: 9, high: false } });
        // The test guest: movl $1,0x70(%rbx); mov 0x100(%rbx),%eax
        assert_eq!(d(&[0xC7, 0x43, 0x70, 1, 0, 0, 0]), Mmio { len: 7, size: 4, op: Op::StoreImm(1) });
        assert_eq!(d(&[0x8B, 0x83, 0, 1, 0, 0]).len, 6);
        // SIB with no base: mov %eax,0xd0000050(,%rcx,1)
        assert_eq!(d(&[0x89, 0x04, 0x0D, 0x50, 0, 0, 0xD0]).len, 7);
        // RIP-relative and a segment prefix.
        assert_eq!(d(&[0x65, 0x8B, 0x05, 1, 2, 3, 4]).len, 7);
        // movb $0x5,(%rax); mov %ah,(%rcx); with REX, mov %sil,(%rcx)
        assert_eq!(d(&[0xC6, 0x00, 5]), Mmio { len: 3, size: 1, op: Op::StoreImm(5) });
        assert_eq!(d(&[0x88, 0x21]).op, Op::StoreReg { reg: 0, high: true });
        assert_eq!(d(&[0x40, 0x88, 0x31]).op, Op::StoreReg { reg: 6, high: false });
        // movq $-1,(%rax): imm32 sign-extended.
        assert_eq!(d(&[0x48, 0xC7, 0x00, 0xFF, 0xFF, 0xFF, 0xFF]).op, Op::StoreImm(u64::MAX));
    }

    /// Every device access the in-tree test guest makes (a `mov` load or `movl` store
    /// based on RBX) decodes, with the length the assembler gave it.
    #[test]
    fn the_test_guests_mmio_instructions_decode() {
        let code = &crate::TEST_GUEST[0x800..]; // the 64-bit code
        let mut seen = 0;
        for i in 0..code.len() {
            let len = match code[i..] {
                [0xC7, 0x43, ..] => 7,
                [0xC7, 0x83, ..] => 10,
                [0x8B, 0x03, ..] => 2,
                [0x8B, 0x43, ..] => 3,
                [0x8B, 0x83, ..] => 6,
                _ => continue,
            };
            let m = decode(&code[i..(i + 15).min(code.len())], true, true).unwrap();
            assert_eq!((m.len, m.size), (len, 4), "at payload offset {:#x}", i + 0x200);
            seen += 1;
        }
        assert_eq!(seen, 27, "the accesses objdump lists in the guest");
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        assert_eq!(decode(&[0x01, 0x02], true, true), None, "add");
        assert_eq!(decode(&[0x89, 0xC0], true, true), None, "register operand");
        assert_eq!(decode(&[0xC7, 0x48, 0x10], true, true), None, "/1 is not mov, and truncated");
        assert_eq!(decode(&[0xC7, 0x43, 0x70, 1], true, true), None, "immediate cut short");
        assert_eq!(decode(&[0x66; 20], true, true), None, "prefixes only");
        assert_eq!(decode(&[], true, true), None);
        assert_eq!(decode(&[0xF3, 0xA5], true, true), None, "rep movs");
    }

    #[test]
    fn sixteen_and_thirty_two_bit_code() {
        // 32-bit: 0x40 is INC, not REX.
        assert_eq!(decode(&[0x40, 0x89, 0x02], false, true), None);
        assert_eq!(decode(&[0x89, 0x02], false, true).unwrap().size, 4);
        // 16-bit: mov [bx+si+0x10],ax uses a 16-bit displacement form.
        let m = decode(&[0x89, 0x40, 0x10], false, false).unwrap();
        assert_eq!((m.len, m.size), (3, 2));
        assert_eq!(decode(&[0x89, 0x06, 0x00, 0x10], false, false).unwrap().len, 4, "[disp16]");
        assert_eq!(decode(&[0x66, 0x89, 0x06, 0x00, 0x10], false, false).unwrap().size, 4);
    }

    #[test]
    fn loads_write_registers_like_the_cpu() {
        let mut r = Regs { rax: 0xFFFF_FFFF_FFFF_FFFF, ..Regs::default() };
        complete_load(&mut r, &d(&[0x8B, 0x02]), 0x1234_5678_9ABC);
        assert_eq!(r.rax, 0x5678_9ABC, "32-bit load zero-extends");
        r.rax = u64::MAX;
        complete_load(&mut r, &d(&[0x8A, 0x02]), 0x42);
        assert_eq!(r.rax, 0xFFFF_FFFF_FFFF_FF42, "8-bit load merges");
        complete_load(&mut r, &d(&[0x8A, 0x22]), 0x17);
        assert_eq!(r.rax & 0xFF00, 0x1700, "AH");
        complete_load(&mut r, &d(&[0x48, 0x0F, 0xBF, 0x02]), 0x8000);
        assert_eq!(r.rax, 0xFFFF_FFFF_FFFF_8000, "movsx sign-extends");
        complete_load(&mut r, &d(&[0x4C, 0x8B, 0x3A]), 7);
        assert_eq!(r.r15, 7);
        r.rbx = 0xAABB;
        assert_eq!(store_value(&mut r, &d(&[0x88, 0x3A])), 0xAA, "BH");
        assert_eq!(store_value(&mut r, &d(&[0x66, 0x89, 0x1A])), 0xAABB);
    }

    #[test]
    fn page_walks() {
        let mut ram = Ram::new(1 << 22).unwrap();
        let mut st = crate::boot::long_mode(0);
        crate::boot::load(&mut ram, &crate::boot::tests::bzimage(0x020F, 2, 1, &[0; 16]), None, b"").unwrap();
        assert_eq!(translate(&ram, &st, 0xD000_0050), Some(0xD000_0050), "identity, 2 MiB pages");
        // A 4 KiB mapping: linear 0x40_0000 -> 0x3000 through a page table at 0x2000.
        ram.write_u64(crate::boot::PD + 2 * 8, 0x2000 | 3).unwrap();
        ram.write_u64(0x2000, 0x3000 | 3).unwrap();
        assert_eq!(translate(&ram, &st, 0x40_0123), Some(0x3123));
        assert_eq!(translate(&ram, &st, 0x40_1000), None, "not present");
        st.guest_cr3 = 0xFFFF_0000_0000; // outside RAM
        assert_eq!(translate(&ram, &st, 0), None);
        // Paging off: physical = linear.
        let real = sex_pdx::vm::State::real_mode(0, 0);
        assert_eq!(translate(&ram, &real, 0x1234), Some(0x1234));
        // Fetch stops at an unmapped page.
        ram.write(0x3FF8, &[0x90; 8]).unwrap();
        let mut st = crate::boot::long_mode(0x40_0FF8);
        st.guest_cr3 = crate::boot::PML4;
        ram.write_u64(0x2008, 0).unwrap();
        let mut buf = [0; 15];
        assert_eq!(fetch(&ram, &st, &mut buf), 8);
    }
}
