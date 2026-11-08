// SPDX-License-Identifier: GPL-3.0-only
//! Virtio PCI devices. BOOT reads a device's vendor capabilities once and leases
//! its driver one BAR view plus the offsets of the four register blocks inside it.
//! A driver never learns a physical register address, and a request can't name one.

/// `rdi` = the held device slot, `rsi` = page (0 or 1). Page 0 returns the common,
/// notify, ISR and device-specific windows; page 1 the notify offset multiplier.
pub const SYS_LAYOUT: u64 = 75;
pub const VENDOR: u16 = 0x1af4;
/// Modern (virtio 1.0+) PCI product ids: 0x1040 + device type.
pub const NET: u16 = 0x1041;
pub const BLOCK: u16 = 0x1042;
pub const INPUT: u16 = 0x1052;
pub const SOUND: u16 = 0x1059;

/// Sound device lease in the Bell service, the way `SLOT_NIC` is sexnet's NIC.
pub const SLOT_SOUND: u64 = 20;
/// The DMA pool BOOT gives a virtio driver at `SLOT_DMA_REGION`.
pub const DMA_REGION_ID: u32 = 0x4200;
pub const DMA_BYTES: u64 = 256 * 1024;
/// A lease never covers more than this prefix of the BAR.
pub const VIEW_MAX: u64 = 64 * 1024;
/// `virtio_pci_common_cfg` is 56 bytes in virtio 1.0+.
pub const COMMON_BYTES: u32 = 0x38;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Window { pub offset: u32, pub length: u32 }
impl Window {
    pub const fn end(self) -> u64 { self.offset as u64 + self.length as u64 }
    const fn pack(self) -> u64 { self.offset as u64 | (self.length as u64) << 32 }
    const fn unpack(word: u64) -> Self { Self { offset: word as u32, length: (word >> 32) as u32 } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Layout {
    pub common: Window,
    pub notify: Window,
    pub notify_multiplier: u32,
    pub isr: Window,
    pub device: Window,
}

impl Layout {
    pub fn end(&self) -> u64 {
        [self.common, self.notify, self.isr, self.device].iter().map(|w| w.end()).max().unwrap_or(0)
    }
    /// Every block fits the leased view, is aligned, and is big enough to use.
    pub fn valid(&self, view_length: u64) -> bool {
        let fits = |w: Window, min: u32| w.length >= min && w.offset % 4 == 0 && w.end() <= view_length;
        view_length <= VIEW_MAX && fits(self.common, COMMON_BYTES) && fits(self.notify, 2)
            && fits(self.isr, 1) && fits(self.device, 4) && self.notify_multiplier % 2 == 0
    }
    pub fn page(&self, page: u64) -> Option<[u64; 4]> {
        match page {
            0 => Some([self.common.pack(), self.notify.pack(), self.isr.pack(), self.device.pack()]),
            1 => Some([self.notify_multiplier as u64, 0, 0, 0]),
            _ => None,
        }
    }
    pub fn from_pages(first: [u64; 4], second: [u64; 4]) -> Self {
        Self { common: Window::unpack(first[0]), notify: Window::unpack(first[1]),
            isr: Window::unpack(first[2]), device: Window::unpack(first[3]),
            notify_multiplier: second[0] as u32 }
    }
}

/// The layout of the virtio device held at `slot`, checked against its view.
#[cfg(target_arch = "x86_64")]
pub fn layout(slot: u64) -> Result<Layout, u64> {
    let mut pages = [[0u64; 4]; 2];
    for (page, words) in pages.iter_mut().enumerate() {
        let (status, a, b, c, d): (u64, u64, u64, u64, u64);
        unsafe { core::arch::asm!("syscall",
            inlateout("rax") SYS_LAYOUT => status, in("rdi") slot,
            inlateout("rsi") page as u64 => a, inlateout("rdx") 0u64 => b,
            inlateout("r10") 0u64 => c, inlateout("r8") 0u64 => d,
            out("rcx") _, out("r11") _, options(nostack)); }
        if status != 0 { return Err(status); }
        *words = [a, b, c, d];
    }
    Ok(Layout::from_pages(pages[0], pages[1]))
}

/// Host builds (tests) have no kernel to ask.
#[cfg(not(target_arch = "x86_64"))]
pub fn layout(_slot: u64) -> Result<Layout, u64> { Err(u64::MAX) }

#[cfg(test)]
mod tests {
    use super::*;
    fn qemu() -> Layout {
        Layout { common: Window { offset: 0, length: 0x1000 }, isr: Window { offset: 0x1000, length: 0x1000 },
            device: Window { offset: 0x2000, length: 0x1000 }, notify: Window { offset: 0x3000, length: 0x1000 },
            notify_multiplier: 4 }
    }
    #[test]
    fn round_trip() {
        let l = qemu();
        assert_eq!(Layout::from_pages(l.page(0).unwrap(), l.page(1).unwrap()), l);
        assert!(l.page(2).is_none());
    }
    #[test]
    fn bounds() {
        let l = qemu();
        assert!(l.valid(0x4000));
        assert!(!l.valid(0x3fff));
        assert!(!l.valid(VIEW_MAX * 2));
        assert!(!Layout { notify_multiplier: 3, ..l }.valid(0x4000));
        assert!(!Layout { common: Window { offset: 0, length: 8 }, ..l }.valid(0x4000));
        assert!(!Layout { isr: Window { offset: 0x1001, length: 1 }, ..l }.valid(0x4000));
    }
}
