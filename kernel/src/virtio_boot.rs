// SPDX-License-Identifier: GPL-3.0-only
//! BOOT policy for virtio PCI devices: which ones a service may drive, and where
//! their register blocks sit. Read once from config space before sealing; a
//! driver later sees only offsets inside the one BAR view it was leased.
use crate::hal::pci::PciDevice;
use sex_pdx::virtio::{self as v, Layout, Window};

const CAP_LIST: u32 = 1 << 20; // status register bit 4, read through the 0x04 dword
const CAP_VENDOR: u8 = 0x09;
const COMMON: u8 = 1;
const NOTIFY: u8 = 2;
const ISR: u8 = 3;
const DEVICE: u8 = 4;

/// Virtio devices some driver manifest claims by id. Modern (virtio 1.0+) ids only:
/// transitional ids need the legacy I/O BAR.
pub fn supported(vendor: u16, device: u16) -> bool {
    use sex_hwdb::manifest::{ALL, Lease};
    vendor == v::VENDOR && ALL.iter().flat_map(|m| m.matches.iter())
        .any(|e| e.lease == Lease::Virtio && e.vendor == vendor && e.device == device)
}

fn byte(dev: &PciDevice, at: u8) -> u8 { (dev.read_u32(at & !3) >> ((at & 3) * 8)) as u8 }

/// The BAR holding all four register blocks and their offsets inside it. A device
/// that spreads them over several BARs, or past the lease's prefix, is not leased.
pub fn layout(dev: &PciDevice) -> Option<(u8, Layout)> {
    if dev.read_u32(0x04) & CAP_LIST == 0 { return None; }
    let mut at = byte(dev, 0x34) & !3;
    let mut found: [Option<(u8, Window)>; 5] = [None; 5];
    let mut multiplier = 0;
    // A capability list is at most 48 entries in 192 bytes; a loop is malformed.
    for _ in 0..48 {
        if at < 0x40 { break; }
        let (id, next) = (byte(dev, at), byte(dev, at + 1) & !3);
        if id == CAP_VENDOR && byte(dev, at + 2) >= 16 {
            let kind = byte(dev, at + 3);
            let bar = byte(dev, at + 4);
            let window = Window { offset: dev.read_u32(at + 8), length: dev.read_u32(at + 12) };
            if matches!(kind, COMMON | NOTIFY | ISR | DEVICE) && found[kind as usize].is_none() && bar <= 5 {
                if kind == NOTIFY {
                    if byte(dev, at + 2) < 20 { at = next; continue; }
                    multiplier = dev.read_u32(at + 16);
                }
                found[kind as usize] = Some((bar, window));
            }
        }
        at = next;
    }
    let (bar, common) = found[COMMON as usize]?;
    let window = |kind: u8| found[kind as usize].filter(|(b, _)| *b == bar).map(|(_, w)| w);
    let layout = Layout { common, notify: window(NOTIFY)?, notify_multiplier: multiplier,
        isr: window(ISR)?, device: window(DEVICE)? };
    Some((bar, layout))
}
