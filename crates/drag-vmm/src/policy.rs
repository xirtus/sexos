// SPDX-License-Identifier: GPL-3.0-only
//! What the guest learns about the CPU (CPUID) and which model-specific registers it
//! may use. Both are allow-lists: a feature or MSR is offered only when Drag either
//! emulates it or knows the hardware state behind it is the guest's own.
//!
//! Hidden on purpose: VMX/SMX (no nesting), XSAVE/AVX and everything needing XCR0
//! (the kernel does not switch XCR0; FXSAVE state is switched by the VMM), the local
//! APIC and x2APIC (D2 offers the 8259 PIC; APIC emulation is D4), SYSENTER (the
//! kernel zeroes the guest's SYSENTER fields), RDTSCP and TSC_AUX, PCID, PKU,
//! MTRR, MCE/MCA, performance counters, thermal and power leaves, and hybrid topology
//! (the guest sees one core).

/// Highest basic and extended leaves offered.
pub const BASIC_MAX: u32 = 0x16;
pub const EXTENDED_MAX: u32 = 0x8000_0008;
pub const HV_SIGNATURE: &[u8; 12] = b"SexOSDragVMM";

/// The guest's CPUID for (`leaf`, `subleaf`), from the host's answer `host(leaf, sub)`.
pub fn cpuid(leaf: u32, subleaf: u32, mut host: impl FnMut(u32, u32) -> [u32; 4]) -> [u32; 4] {
    let h = host(leaf, subleaf);
    match leaf {
        0 => [h[0].min(BASIC_MAX), h[1], h[2], h[3]],
        1 => {
            // ECX: SSE3, PCLMULQDQ, SSSE3, CX16, SSE4.1, SSE4.2, MOVBE, POPCNT, AES, RDRAND;
            // and the hypervisor bit.
            let ecx = h[2] & (1 | 1 << 1 | 1 << 9 | 1 << 13 | 1 << 19 | 1 << 20 | 1 << 22 | 1 << 23 | 1 << 25 | 1 << 30) | 1 << 31;
            // EDX: FPU VME DE PSE TSC MSR PAE CX8 PGE CMOV PAT PSE36 CLFSH MMX FXSR SSE SSE2.
            let edx = h[3] & (0x7F | 1 << 8 | 1 << 13 | 1 << 15 | 1 << 16 | 1 << 17 | 1 << 19 | 1 << 23 | 1 << 24 | 1 << 25 | 1 << 26);
            // EBX: keep brand index and CLFLUSH size; one logical CPU, APIC ID 0.
            [h[0], (h[1] & 0xFFFF) | 1 << 16, ecx, edx]
        }
        // Deterministic cache parameters: as the host, for one core.
        4 => [h[0] & 0x3FFF, h[1], h[2], h[3]],
        7 if subleaf == 0 => {
            // EBX: FSGSBASE, BMI1, SMEP, BMI2, ERMS, RDSEED, ADX, SMAP, CLFLUSHOPT, SHA.
            let ebx = h[1] & (1 | 1 << 3 | 1 << 7 | 1 << 8 | 1 << 9 | 1 << 18 | 1 << 19 | 1 << 20 | 1 << 23 | 1 << 29);
            [0, ebx, 0, 0]
        }
        // TSC and core crystal frequency, base/max frequency: lets Linux skip PIT calibration.
        0x15 | 0x16 => h,
        0x4000_0000 => {
            let s = HV_SIGNATURE;
            let w = |i: usize| u32::from_le_bytes([s[i], s[i + 1], s[i + 2], s[i + 3]]);
            [0x4000_0000, w(0), w(4), w(8)]
        }
        0x8000_0000 => [h[0].min(EXTENDED_MAX), h[1], h[2], h[3]],
        // ECX: LAHF, ABM/LZCNT, PREFETCHW. EDX: SYSCALL, NX, LM.
        0x8000_0001 => [h[0], 0, h[2] & (1 | 1 << 5 | 1 << 8), h[3] & (1 << 11 | 1 << 20 | 1 << 29)],
        0x8000_0002..=0x8000_0006 => h,
        // Invariant TSC only.
        0x8000_0007 => [0, 0, 0, h[3] & 1 << 8],
        // Physical address bits no more than the 512 GiB the kernel's EPT maps; one core.
        0x8000_0008 => [(h[0] & 0xFF).min(39) | (h[0] & 0xFF00), 0, 0, 0],
        _ => [0; 4],
    }
}

/// MSRs.
pub mod msr {
    pub const TSC: u32 = 0x10;
    pub const PLATFORM_ID: u32 = 0x17;
    pub const APIC_BASE: u32 = 0x1B;
    pub const FEATURE_CONTROL: u32 = 0x3A;
    pub const UCODE_WRITE: u32 = 0x79;
    pub const UCODE_REV: u32 = 0x8B;
    pub const MISC_ENABLE: u32 = 0x1A0;
    pub const DEBUGCTL: u32 = 0x1D9;
    pub const PAT: u32 = 0x277;
    pub const EFER: u32 = 0xC000_0080;
    pub const STAR: u32 = 0xC000_0081;
    pub const LSTAR: u32 = 0xC000_0082;
    pub const CSTAR: u32 = 0xC000_0083;
    pub const SFMASK: u32 = 0xC000_0084;
    pub const FS_BASE: u32 = 0xC000_0100;
    pub const GS_BASE: u32 = 0xC000_0101;
    pub const KERNEL_GS_BASE: u32 = 0xC000_0102;
}

/// Where an MSR lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MsrKind {
    /// In the vCPU state (EFER, FS/GS base): read and written through GET/SET_REGS.
    State,
    /// A value Drag keeps; writes go there, reads come back. `Stored(true)`: writes
    /// are accepted and dropped (read-only).
    Stored(bool),
    /// The host's TSC.
    Tsc,
    /// Not offered: #GP, as a CPU without it would raise.
    Absent,
}

pub fn msr_kind(index: u32) -> MsrKind {
    use msr::*;
    match index {
        EFER | FS_BASE | GS_BASE => MsrKind::State,
        TSC => MsrKind::Tsc,
        PLATFORM_ID | FEATURE_CONTROL | UCODE_REV | MISC_ENABLE | APIC_BASE => MsrKind::Stored(true),
        UCODE_WRITE | DEBUGCTL | PAT | STAR | LSTAR | CSTAR | SFMASK | KERNEL_GS_BASE => MsrKind::Stored(false),
        _ => MsrKind::Absent,
    }
}

/// Values of the stored MSRs at reset.
pub fn msr_reset(index: u32) -> u64 {
    match index {
        msr::FEATURE_CONTROL => 1, // locked, VMX off
        msr::MISC_ENABLE => 1,     // fast strings
        msr::PAT => 0x0007_0406_0007_0406,
        _ => 0,
    }
}

/// A small store for the stored MSRs.
#[derive(Clone, Debug, Default)]
pub struct MsrStore { values: [(u32, u64); 16], len: usize }

impl MsrStore {
    pub fn read(&self, index: u32) -> u64 {
        self.values[..self.len].iter().find(|v| v.0 == index).map_or(msr_reset(index), |v| v.1)
    }
    pub fn write(&mut self, index: u32, value: u64) {
        if let Some(v) = self.values[..self.len].iter_mut().find(|v| v.0 == index) { v.1 = value; return; }
        // Only `Stored` MSRs reach here, fewer than the slots.
        if self.len < self.values.len() { self.values[self.len] = (index, value); self.len += 1; }
    }
}

/// A guest write to EFER, given the current value: SCE, LME, NXE are the guest's;
/// LMA is the CPU's and stays. Err for reserved bits (#GP).
pub fn efer_write(current: u64, value: u64) -> Result<u64, ()> {
    const SCE: u64 = 1; const LME: u64 = 1 << 8; const LMA: u64 = 1 << 10; const NXE: u64 = 1 << 11;
    if value & !(SCE | LME | LMA | NXE) != 0 { return Err(()); }
    Ok((value & (SCE | LME | NXE)) | (current & LMA))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_ones(_: u32, _: u32) -> [u32; 4] { [u32::MAX; 4] }

    #[test]
    fn cpuid_hides_what_drag_cannot_give() {
        let l1 = cpuid(1, 0, all_ones);
        assert_eq!(l1[2] & (1 << 5), 0, "no VMX");
        assert_eq!(l1[2] & (1 << 21 | 1 << 26 | 1 << 27 | 1 << 28), 0, "no x2APIC, XSAVE, OSXSAVE, AVX");
        assert_ne!(l1[2] & (1 << 31), 0, "hypervisor bit");
        assert_eq!(l1[3] & (1 << 9 | 1 << 11 | 1 << 12 | 1 << 14 | 1 << 28), 0, "no APIC, SEP, MTRR, MCA, HTT");
        assert_ne!(l1[3] & (1 << 24 | 1 << 26), 0, "FXSR, SSE2");
        assert_eq!(l1[1] >> 16, 1, "one logical CPU, APIC ID 0");
        assert_eq!(cpuid(7, 0, all_ones)[2] & (1 << 3), 0, "no PKU");
        assert_eq!(cpuid(7, 1, all_ones), [0; 4]);
        assert_eq!(cpuid(0xD, 0, all_ones), [0; 4], "no XSAVE leaf");
        assert_eq!(cpuid(0xB, 0, all_ones), [0; 4]);
        assert_eq!(cpuid(0, 0, all_ones)[0], BASIC_MAX);
        assert_eq!(cpuid(0x8000_0001, 0, all_ones)[3], 1 << 11 | 1 << 20 | 1 << 29, "no RDTSCP, no 1G pages");
        assert_eq!(cpuid(0x8000_0008, 0, all_ones)[0] & 0xFF, 39);
        let s = cpuid(0x4000_0000, 0, all_ones);
        let mut sig = [0u8; 12];
        for i in 0..3 { sig[i * 4..i * 4 + 4].copy_from_slice(&s[i + 1].to_le_bytes()); }
        assert_eq!(&sig, HV_SIGNATURE);
    }

    #[test]
    fn msr_policy() {
        assert_eq!(msr_kind(msr::EFER), MsrKind::State);
        assert_eq!(msr_kind(0x48), MsrKind::Absent, "SPEC_CTRL is not offered");
        assert_eq!(msr_kind(0x4B56_4D00), MsrKind::Absent, "not KVM");
        let mut s = MsrStore::default();
        assert_eq!(s.read(msr::PAT), 0x0007_0406_0007_0406);
        s.write(msr::LSTAR, 0xFFFF_FFFF_8100_0000);
        assert_eq!(s.read(msr::LSTAR), 0xFFFF_FFFF_8100_0000);
        for i in 0..100 { s.write(0x1000 + i, i as u64); }
        assert_eq!(s.read(msr::LSTAR), 0xFFFF_FFFF_8100_0000, "a full store keeps what it has");
        assert_eq!(efer_write(0x500, 0xD01), Ok(0xD01));
        assert_eq!(efer_write(0x500, 0x1), Ok(0x401), "LMA stays");
        assert_eq!(efer_write(0, 1 << 12), Err(()), "SVME is reserved here");
    }
}
