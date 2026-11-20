// SPDX-License-Identifier: GPL-3.0-only
//! NVIDIA GPUs (the x17's RTX 3070 Laptop): what the card hands the driver. The
//! VBIOS read through the PROM window (register words from the input), its image
//! chain, BIT tables and FWSEC ucode; the firmware files' headers and ELF
//! sections; and the GSP-RM message queue, which the GPU writes into shared
//! memory while the driver reads it.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
use sex_gpu_nv::{chip::Chip, elf, fw, msgq, vbios, Clock, Dma, Mmio};
use sexos_driver_fuzz::{FuzzRegs, Src};

struct Dev<'a>(FuzzRegs<'a>);
impl Mmio for Dev<'_> {
    fn read(&mut self, _offset: u32) -> u32 { self.0.read32() }
    fn write(&mut self, _offset: u32, _value: u32) { self.0.write32() }
}
struct NoClock;
impl Clock for NoClock { fn delay_us(&mut self, _us: u32) {} }

fn rom(r: &[u8]) {
    let mut base = 0usize;
    for _ in 0..8 { match vbios::image_at(r, base) { Some(i) if !i.last && i.size > 0 => base += i.size as usize, _ => break } }
    let Ok(v) = vbios::Vbios::parse(r) else { return };
    let _ = (v.pci_ids(), v.version());
    for id in [b'I', b'p', b'P', b'u', b'd'] { let _ = v.bit_entry(id); }
    for k in 0..4 { let _ = v.pmu_ucode(k); }
    if let Ok(f) = v.fwsec() {
        let mut staged = vec![0u8; f.ucode.staged_len().min(1 << 20)];
        if let Ok(idx) = f.ucode.signature_index(f.ucode.fuse_register().unwrap_or(0)) { let _ = f.ucode.stage(&mut staged, idx); }
        let _ = f.patch_command(&mut staged, vbios::FwsecCommand::Frts { addr: 0x1000_0000, size: 0x10_0000 });
        let _ = f.patch_command(&mut staged, vbios::FwsecCommand::Sb);
    }
}

fn run(d: &[u8]) {
    let mut s = Src::new(d);
    let _ = Chip::decode(s.u32());
    match s.u8() % 4 {
        // The VBIOS as the PROM window returns it, word by word.
        0 => {
            let mut m = Dev(FuzzRegs::new(s.rest()));
            let mut out = vec![0u8; 64 * 1024];
            if let Ok(n) = vbios::read_prom(&mut m, &mut out) { rom(&out[..n.min(out.len())]); }
            let _ = sex_gpu_nv::wait_for(&mut m, &mut NoClock, 0x100, 1, 1, 1000, 10);
        }
        // A VBIOS image as given.
        1 => rom(s.rest()),
        // Firmware files (booter, bootloader, GSP-RM ELF).
        2 => {
            let b = s.rest();
            if let Ok(h) = fw::bin_hdr(b) { let _ = (h.data_offset, h.data_size); }
            if let Ok(u) = fw::booter(b) {
                let mut staged = vec![0u8; u.staged_len().min(1 << 20)];
                if let Ok(i) = u.signature_index(s.u32()) { let _ = u.stage(&mut staged, i); }
            }
            let _ = fw::bootloader(b);
            let _ = fw::gsp_rm(b, ".fwsignature_ga10x");
            let _ = elf::section(b, ".fwimage");
        }
        // The GSP message queue: GSP owns its half of shared memory.
        _ => {
            let mut shm = vec![0u8; 512 * 1024];
            let mut dma = Dma::new(&mut shm, 0x8000_0000);
            let Ok(l) = msgq::init_shared(&mut dma) else { return };
            let total = l.total.min(shm.len());
            let gsp = s.chunk();
            let at = l.msgq_off.min(total);
            let n = gsp.len().min(total - at);
            shm[at..at + n].copy_from_slice(&gsp[..n]);
            // A few stray writes anywhere (pointers, headers).
            for _ in 0..s.below(8) { let i = s.below(total); shm[i] = s.u8(); }
            let mut q = msgq::Queues::new(l);
            let mut out = vec![0u8; 4096];
            for _ in 0..64 { match q.recv(&mut shm[..total], &mut out) { Ok(Some(_)) => {}, _ => break } }
            let mut scratch = vec![0u8; 64 * 1024];
            let _ = q.cmdq_free(&shm[..total]);
            let _ = q.push(&mut shm[..total], &mut scratch, 0x15, s.chunk());
        }
    }
}

fn seeds() -> Vec<Vec<u8>> {
    // PCI expansion ROM header + PCIR, then a BIT header.
    let mut r = vec![0u8; 4096];
    r[0] = 0x55; r[1] = 0xaa; r[0x18] = 0x40;
    r[0x40..0x44].copy_from_slice(b"PCIR");
    r[0x44..0x46].copy_from_slice(&0x10deu16.to_le_bytes());
    r[0x46..0x48].copy_from_slice(&0x24ddu16.to_le_bytes());
    r[0x50] = 8; r[0x55] = 0x80;
    r[0x100..0x106].copy_from_slice(&[0xff, 0xb8, b'B', b'I', b'T', 0]);
    let mut a = vec![0x00, 0, 0, 0, 1]; a.extend_from_slice(&r);
    let mut b = vec![0x00, 0, 0, 0, 3]; b.extend_from_slice(&[0, 0x40]); b.extend(std::iter::repeat(0x11).take(0x4000));
    vec![a, b]
}

sexos_driver_fuzz::driver_target!(run, seeds);
