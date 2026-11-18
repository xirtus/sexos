// SPDX-License-Identifier: GPL-3.0-only
//! PKU gate stubs (kernel redesign Phase 6, `docs/handoff/PKU_GATES_STUDY_2026_11_18.md`).
//!
//! A gate stub lets a trusted service C call a fixed entry of another trusted service E
//! without entering the kernel. The kernel builds one stub per granted (C, E, entry) and
//! maps the page user-executable and read-only; it holds the only WRPKRU bytes outside
//! the kernel. This crate builds and checks those stubs. It never assembles: the
//! template comes from `stub.S` through `scripts/gen_gate_stub.py`, and [`build`] only
//! patches fields.
//!
//! Every WRPKRU in a stub writes one of three immediates and is checked against it:
//! `G` (frame key writable, all else disabled), `E` (the callee's keys, then a direct
//! call into the fixed entry once the frame page shows this stub on top), or `C` (the
//! caller's keys, then a return that takes RSP only from C's own return slot). That is
//! what makes jumping into the middle of a stub useless (study section 4.2).
#![no_std]

pub mod template;

/// A patchable field of the template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Matrix,
    Stubid,
    PkruG,
    PkruE,
    PkruC,
    Cpage,
    Epage,
    Emask,
    Entry,
}

/// Size of the per-task frame page at the GS base; slot pages follow it.
pub const PAGE: u32 = 4096;
/// Deepest nesting of gate calls in one task.
pub const MAX_DEPTH: u64 = 32;
/// What a stub returns in RAX when the call matrix denies it.
pub const DENIED: u64 = u64::MAX;

/// Frame page layout (GS-relative). The frame key protects it: readable by every
/// domain, writable only while PKRU is the stub's `G` value.
pub mod frame {
    pub const CUR: usize = 0x00;
    pub const DEPTH: usize = 0x08;
    pub const ACTIVE: usize = 0x10;
    pub const SELF: usize = 0x18;
    pub const FRAMES: usize = 0x40;
    pub const SCRATCH_TOP: usize = 0xFF0;
}

/// Slot page layout for one domain (that domain's key).
pub mod slot {
    pub const RSP: usize = 0x00;
    pub const ID: usize = 0x08;
    pub const SAVED: usize = 0x10; // rbx, rbp, r12, r13, r14, r15
    pub const STACK: usize = 0x40;
}

/// GS offset of the slot page for domain index `d` (domain 0's page follows the frame page).
pub const fn slot_offset(domain: u32) -> u32 {
    PAGE * (1 + domain)
}

/// PKRU with every key disabled except `open` (read-write) and `read_only` keys.
pub const fn pkru(open: u16, read_only: u16) -> u32 {
    let mut v = 0u32;
    let mut k = 0;
    while k < 16 {
        let bit = 1u16 << k;
        if open & bit == 0 {
            v |= if read_only & bit != 0 { 0b10 } else { 0b11 } << (2 * k);
        }
        k += 1;
    }
    v
}

/// Everything one stub needs.
#[derive(Clone, Copy, Debug)]
pub struct Stub {
    /// Address the stub will run at (for the rel32 into the callee).
    pub at: u64,
    /// Address of this stub's matrix byte (bit 0 = allowed), readable by the caller.
    pub matrix: u64,
    /// Nonzero id, unique among stubs; at most `i32::MAX` (it is a sign-extended imm32).
    pub id: u32,
    /// The protection key of the frame pages.
    pub frame_key: u8,
    /// Caller and callee PKRU values. Both must give the frame key read-only.
    pub caller_pkru: u32,
    pub callee_pkru: u32,
    /// Domain indexes of caller and callee (slot pages and the active mask).
    pub caller: u32,
    pub callee: u32,
    /// The callee's entry point.
    pub entry: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    BadId,
    BadFrameKey,
    /// A domain PKRU must give the frame key read-only (AD=0, WD=1).
    FrameKeyNotReadOnly,
    SameDomain,
    BadDomain,
    /// The callee entry is not within ±2 GiB of the stub.
    EntryOutOfRange,
    /// Patched immediates formed key-register bytes (WRPKRU/XRSTOR/XRSTORS) somewhere
    /// other than the four WRPKRU instructions.
    KeyBytes(usize),
}

/// Domain indexes go up to this (63 slot pages, one bit each in the active mask).
pub const MAX_DOMAIN: u32 = 62;

/// The `G` value: frame key read-write, every other key disabled.
pub const fn gate_pkru(frame_key: u8) -> u32 {
    pkru(1 << frame_key, 0)
}

/// Builds one stub.
pub fn build(s: &Stub) -> Result<[u8; template::LEN], Error> {
    if s.id == 0 || s.id > i32::MAX as u32 {
        return Err(Error::BadId);
    }
    if s.frame_key == 0 || s.frame_key > 15 {
        return Err(Error::BadFrameKey);
    }
    let fk = 2 * s.frame_key as u32;
    for p in [s.caller_pkru, s.callee_pkru] {
        if (p >> fk) & 0b11 != 0b10 {
            return Err(Error::FrameKeyNotReadOnly);
        }
    }
    if s.caller == s.callee || s.caller_pkru == s.callee_pkru {
        return Err(Error::SameDomain);
    }
    if s.caller > MAX_DOMAIN || s.callee > MAX_DOMAIN {
        return Err(Error::BadDomain);
    }
    let rel = (s.entry as i128) - (s.at as i128 + template::RETURN as i128);
    if rel < i32::MIN as i128 || rel > i32::MAX as i128 {
        return Err(Error::EntryOutOfRange);
    }
    let mut out = template::TEMPLATE;
    for &(field, at, width, addend) in template::SITES {
        let value: u64 = match field {
            Field::Matrix => s.matrix,
            Field::Stubid => s.id as u64,
            Field::PkruG => gate_pkru(s.frame_key) as u64,
            Field::PkruE => s.callee_pkru as u64,
            Field::PkruC => s.caller_pkru as u64,
            Field::Cpage => (slot_offset(s.caller) + addend) as u64,
            Field::Epage => (slot_offset(s.callee) + addend) as u64,
            Field::Emask => 1u64 << s.callee,
            Field::Entry => rel as i32 as u32 as u64,
        };
        out[at..at + width as usize].copy_from_slice(&value.to_le_bytes()[..width as usize]);
    }
    let hits = key_bytes(&out);
    for h in hits.iter().flatten() {
        if !template::WRPKRU.contains(h) {
            return Err(Error::KeyBytes(*h));
        }
    }
    Ok(out)
}

/// Offsets of bytes the loader refuses (same rule as `scripts/key_bytes.py`):
/// WRPKRU (0F 01 EF), XRSTOR (0F AE /5, memory) and XRSTORS (0F C7 /3, memory).
pub fn key_bytes(code: &[u8]) -> [Option<usize>; 8] {
    let mut out = [None; 8];
    let mut n = 0;
    for i in 0..code.len().saturating_sub(2) {
        let (b1, b2) = (code[i + 1], code[i + 2]);
        let reg = (b2 >> 3) & 7;
        let memory = (b2 >> 6) != 3;
        let refused = (b1 == 0x01 && b2 == 0xEF)
            || (b1 == 0xAE && reg == 5 && memory)
            || (b1 == 0xC7 && reg == 3 && memory);
        if code[i] == 0x0F && refused {
            if n == out.len() {
                out[n - 1] = Some(usize::MAX);
                break;
            }
            out[n] = Some(i);
            n += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Stub {
        Stub {
            at: 0x4000_0000,
            matrix: 0x5000_0000,
            id: 7,
            frame_key: 13,
            caller_pkru: pkru(1 << 0 | 1 << 3, 1 << 13),
            callee_pkru: pkru(1 << 2, 1 << 13),
            caller: 3,
            callee: 2,
            entry: 0x4100_0000,
        }
    }

    #[test]
    fn template_has_exactly_four_wrpkru_each_checked() {
        let hits = key_bytes(&template::TEMPLATE);
        let found: [usize; 4] = core::array::from_fn(|i| hits[i].unwrap());
        assert_eq!(found, template::WRPKRU);
        assert!(hits[4].is_none());
        for w in template::WRPKRU {
            // wrpkru; cmp eax, imm32; jne rel32
            assert_eq!(template::TEMPLATE[w + 3], 0x3d, "cmp eax after WRPKRU at {w:#x}");
            // jne to ud2 (short or near form).
            let t = &template::TEMPLATE;
            let target = match t[w + 8] {
                0x75 => w + 10 + (t[w + 9] as i8 as isize as usize),
                0x0f if t[w + 9] == 0x85 => w + 14 + (i32::from_le_bytes(t[w + 10..w + 14].try_into().unwrap()) as isize as usize),
                _ => panic!("no jne after the check at {w:#x}"),
            };
            assert_eq!(&t[target..target + 2], &[0x0f, 0x0b], "jne after WRPKRU at {w:#x} must go to ud2");
            // The immediate loaded into eax before the write is the one it is checked against.
            assert_eq!(template::TEMPLATE[w - 9], 0xb8, "mov eax, imm32 before WRPKRU at {w:#x}");
            assert_eq!(template::TEMPLATE[w - 8..w - 4], template::TEMPLATE[w + 4..w + 8]);
        }
    }

    #[test]
    fn writes_are_g_e_g_c_in_order() {
        let s = sample();
        let b = build(&s).unwrap();
        let imm = |w: usize| u32::from_le_bytes(b[w + 4..w + 8].try_into().unwrap());
        let [a, e, g, c] = template::WRPKRU;
        assert_eq!(imm(a), gate_pkru(13));
        assert_eq!(imm(e), s.callee_pkru);
        assert_eq!(imm(g), gate_pkru(13));
        assert_eq!(imm(c), s.caller_pkru);
    }

    #[test]
    fn every_site_is_patched() {
        let b = build(&sample()).unwrap();
        let other = Stub {
            at: 0x4000_1000, matrix: 0x5000_0040, id: 9, frame_key: 12,
            caller_pkru: pkru(1 << 5, 1 << 12), callee_pkru: pkru(1 << 6, 1 << 12),
            caller: 5, callee: 6, entry: 0x4200_0000,
        };
        let o = build(&other).unwrap();
        for &(field, at, width, _) in template::SITES {
            assert_ne!(b[at..at + width as usize], o[at..at + width as usize], "{field:?} at {at:#x} not patched");
        }
        let mut same = b;
        for &(_, at, width, _) in template::SITES {
            same[at..at + width as usize].copy_from_slice(&o[at..at + width as usize]);
        }
        assert_eq!(same, o, "bytes outside the sites must not depend on the parameters");
        // Entry rel32 points at the callee.
        let s = sample();
        let at = template::SITES.iter().find(|x| x.0 == Field::Entry).unwrap().1;
        let rel = i32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as i64;
        assert_eq!(s.at as i64 + template::RETURN as i64 + rel, s.entry as i64);
    }

    #[test]
    fn g_value_opens_only_the_frame_key() {
        let g = gate_pkru(13);
        for k in 0..16 {
            let bits = (g >> (2 * k)) & 3;
            assert_eq!(bits, if k == 13 { 0 } else { 3 }, "key {k}");
        }
    }

    #[test]
    fn rejects_bad_inputs() {
        let ok = sample();
        assert_eq!(build(&Stub { id: 0, ..ok }).unwrap_err(), Error::BadId);
        assert_eq!(build(&Stub { id: 0x8000_0000, ..ok }).unwrap_err(), Error::BadId);
        assert_eq!(build(&Stub { frame_key: 0, ..ok }).unwrap_err(), Error::BadFrameKey);
        assert_eq!(build(&Stub { callee_pkru: pkru(1 << 2 | 1 << 13, 0), ..ok }).unwrap_err(), Error::FrameKeyNotReadOnly);
        assert_eq!(build(&Stub { caller_pkru: pkru(1 << 3, 0), ..ok }).unwrap_err(), Error::FrameKeyNotReadOnly);
        assert_eq!(build(&Stub { callee: 3, ..ok }).unwrap_err(), Error::SameDomain);
        assert_eq!(build(&Stub { callee: 63, ..ok }).unwrap_err(), Error::BadDomain);
        assert_eq!(build(&Stub { entry: 0x4000_0000 + (3u64 << 31), ..ok }).unwrap_err(), Error::EntryOutOfRange);
    }

    #[test]
    fn rejects_immediates_that_make_key_bytes() {
        // A matrix address whose bytes hold 0F 01 EF would plant a fifth WRPKRU.
        let s = Stub { matrix: 0x0000_00ef_010f_0000, ..sample() };
        assert!(matches!(build(&s), Err(Error::KeyBytes(_))));
        // XRSTOR (0F AE /5 with a memory operand, e.g. 0F AE 28).
        let s = Stub { matrix: 0x0000_0028_ae0f_0000, ..sample() };
        assert!(matches!(build(&s), Err(Error::KeyBytes(_))));
    }
}
