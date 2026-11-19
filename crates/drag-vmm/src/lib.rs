// SPDX-License-Identifier: GPL-3.0-only
//! Drag's virtual machine monitor, phase D2: everything the ring-3 `drag` program does
//! with a VM the kernel runs for it (`sex_pdx::vm`, syscall 110). No Linux or KVM code:
//! the devices, the loader and the exit handling are written for SexOS from the specs
//! (Linux boot protocol, Intel SDM, 8250/16550, 8259A, 8254, virtio 1.2), and the
//! crate is host-testable: the kernel is reached only through the [`vmm::Vcpu`] trait.
//!
//! - [`memory`]: guest RAM, bounds-checked.
//! - [`boot`]: bzImage loading, zero page, E820, identity tables and the entry state.
//! - [`vmm`]: the machine and its exit dispatch.
//! - [`x86`]: register numbering, guest page walks, the MMIO instruction decoder.
//! - [`policy`]: CPUID and MSR allow-lists.
//! - [`uart`], [`pic`], [`pit`], [`virtio`]: the devices.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod boot;
pub mod memory;
pub mod pic;
pub mod pit;
pub mod policy;
pub mod uart;
pub mod virtio;
pub mod vmm;
pub mod x86;

use alloc::vec::Vec;

/// The in-tree D2 gate guest (`guest/drag_test_guest.asm`): a bzImage whose 64-bit
/// entry prints over the UART, reads its command line, and reads and writes sector 0/1
/// of the virtio-blk disk named there.
pub static TEST_GUEST: &[u8] = include_bytes!("../guest/drag_test_guest.bin");

/// The line the test guest prints from sector 0 of [`test_disk`].
pub const TEST_DISK_LINE: &[u8] = b"Drag D2 disk: hello from a file the drag program was granted.";
/// What the test guest writes at the start of sector 1.
pub const TEST_GUEST_MARK: &[u8] = b"drag-d2 guest wrote sector one.";

/// A 64 KiB disk image for the test guest: [`TEST_DISK_LINE`] and a newline in sector 0.
pub fn test_disk() -> Vec<u8> {
    let mut d = alloc::vec![0u8; 64 * 1024];
    d[..TEST_DISK_LINE.len()].copy_from_slice(TEST_DISK_LINE);
    d[TEST_DISK_LINE.len()] = b'\n';
    d
}

/// Where Drag puts virtio-blk and its IRQ.
pub const BLK_BASE: u64 = 0xD000_0000;
pub const BLK_IRQ: u8 = 5;
/// The command line Drag gives a Linux guest before the user's words: serial console,
/// the PIC (no local APIC is offered), no PCI, and the PIT as the timer.
pub const DEFAULT_CMDLINE: &[u8] = b"console=ttyS0 earlyprintk=serial noapic nolapic pci=off tsc=reliable reboot=t panic=-1";

/// What `run drag ...` asked for. Paths are inside the program's own folder (or its
/// grants), as Files resolves them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub kernel: Option<Vec<u8>>,
    pub initrd: Option<Vec<u8>>,
    pub disk: Option<Vec<u8>>,
    pub read_only: bool,
    pub mem_mib: u32,
    /// Extra command-line words (after `--`).
    pub cmdline: Vec<u8>,
}

pub const MEM_MIB_DEFAULT: u32 = 256;
pub const MEM_MIB_TEST: u32 = 8;
pub const MEM_MIB_MAX: u32 = 2048;

impl Args {
    /// `kernel=PATH initrd=PATH disk=PATH ro mem=MIB -- words for the kernel`.
    pub fn parse(line: &[u8]) -> Result<Args, &'static str> {
        let mut a = Args { kernel: None, initrd: None, disk: None, read_only: false, mem_mib: 0, cmdline: Vec::new() };
        let mut words = line.split(|&c| c == b' ').filter(|w| !w.is_empty());
        while let Some(w) = words.next() {
            if w == b"--" {
                let rest: Vec<&[u8]> = words.by_ref().collect();
                a.cmdline = rest.join(&b' ');
                break;
            }
            let (key, value) = match w.iter().position(|&c| c == b'=') { Some(i) => (&w[..i], &w[i + 1..]), None => (w, &b""[..]) };
            match key {
                b"kernel" | b"initrd" | b"disk" if value.is_empty() => return Err("empty path"),
                b"kernel" => a.kernel = Some(value.to_vec()),
                b"initrd" => a.initrd = Some(value.to_vec()),
                b"disk" => a.disk = Some(value.to_vec()),
                b"ro" if value.is_empty() => a.read_only = true,
                b"mem" => {
                    let n = core::str::from_utf8(value).ok().and_then(|s| s.parse::<u32>().ok()).ok_or("mem is a number of MiB")?;
                    if !(4..=MEM_MIB_MAX).contains(&n) { return Err("mem is 4..=2048 MiB"); }
                    a.mem_mib = n;
                }
                _ => return Err("unknown word: use kernel= initrd= disk= ro mem= --"),
            }
        }
        if a.initrd.is_some() && a.kernel.is_none() { return Err("initrd= needs kernel="); }
        if a.mem_mib == 0 { a.mem_mib = if a.kernel.is_some() { MEM_MIB_DEFAULT } else { MEM_MIB_TEST }; }
        Ok(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtio::{Blk, MemDisk};

    #[test]
    fn args() {
        let a = Args::parse(b"").unwrap();
        assert_eq!((a.kernel, a.mem_mib), (None, MEM_MIB_TEST), "no words: the test guest");
        let a = Args::parse(b"kernel=bzImage initrd=init.cpio disk=root.img ro mem=512 -- root=/dev/vda  quiet").unwrap();
        assert_eq!(a.kernel.as_deref(), Some(&b"bzImage"[..]));
        assert_eq!((a.read_only, a.mem_mib), (true, 512));
        assert_eq!(a.cmdline, b"root=/dev/vda quiet");
        assert_eq!(Args::parse(b"kernel=k").unwrap().mem_mib, MEM_MIB_DEFAULT);
        assert!(Args::parse(b"mem=99999").is_err());
        assert!(Args::parse(b"mem=x").is_err());
        assert!(Args::parse(b"kernel=").is_err());
        assert!(Args::parse(b"initrd=i").is_err());
        assert!(Args::parse(b"rm -rf").is_err());
    }

    #[test]
    fn test_guest_loads_with_the_device_on_its_command_line() {
        let ram = memory::Ram::new((MEM_MIB_TEST as usize) << 20).unwrap();
        let blk = Blk::new(MemDisk { data: test_disk(), read_only: false }, BLK_BASE, BLK_IRQ);
        let mut m = vmm::Machine::new(ram, Some(blk));
        let b = m.load_linux(TEST_GUEST, None, b"console=ttyS0").unwrap();
        assert_eq!(b.entry.regs.rip, 0x10_0200);
        let line = m.ram.slice(boot::CMDLINE, 49).unwrap();
        assert_eq!(line, b"console=ttyS0 virtio_mmio.device=4K@0xd0000000:5\0");
        // The guest's 64-bit entry is where the protocol says, and its code is there.
        assert_eq!(m.ram.slice(0x10_0200, 1).unwrap(), &[0xFA], "cli");
        assert_eq!(&test_disk()[..TEST_DISK_LINE.len()], TEST_DISK_LINE);
    }
}
