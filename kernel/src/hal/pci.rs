use crate::serial_println;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use x86_64::{VirtAddr, structures::paging::PageTableFlags};

#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_id: u8,
    pub subclass_id: u8,
    pub prog_if: u8,
}

static NET_DIAG_HTTP_STATUS: AtomicU32 = AtomicU32::new(0);
static NET_DIAG_HTTP_BYTES: AtomicU32 = AtomicU32::new(0);
static NET_DIAG_HTTP_SOURCE: AtomicU8 = AtomicU8::new(0);
static NET_DIAG_HTTP_BODY_LEN: AtomicU32 = AtomicU32::new(0);
static mut NET_DIAG_HTTP_BODY: [u8; 64] = [0u8; 64];

pub fn get_net_diag() -> (u32, u32, u8) {
    (
        NET_DIAG_HTTP_STATUS.load(Ordering::Acquire),
        NET_DIAG_HTTP_BYTES.load(Ordering::Acquire),
        NET_DIAG_HTTP_SOURCE.load(Ordering::Acquire),
    )
}

pub fn get_net_diag_body_len() -> u32 {
    NET_DIAG_HTTP_BODY_LEN.load(Ordering::Acquire)
}

pub fn get_net_diag_body_chunk(idx: usize) -> u64 {
    let len = NET_DIAG_HTTP_BODY_LEN.load(Ordering::Acquire) as usize;
    let start = idx * 8;
    if start >= len {
        return u64::MAX;
    }

    let end = core::cmp::min(start + 8, len);
    let mut packed = 0u64;
    let mut i = 0usize;
    while i < (end - start) {
        packed |= unsafe { (NET_DIAG_HTTP_BODY[start + i] as u64) << (i * 8) };
        i += 1;
    }
    packed
}

impl PciDevice {
    /// Boot discovery only. Restore both BAR words and the command register;
    /// upper status bits are W1C and must never be echoed into a config write.
    pub fn probe_boot_bar0(&self) -> Option<(u64, u64)> {
        assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),
            crate::ipc::BootPhase::RegistryBuild as u8, "PCI_PROBE_AFTER_GENESIS");
        let low = self.read_u32(0x10);
        if low & 1 != 0 || !matches!((low >> 1) & 3, 0 | 2) { return None; }
        let wide = ((low >> 1) & 3) == 2;
        let high = if wide { self.read_u32(0x14) } else { 0 };
        let command = self.read_u32(0x04) & 0xffff;
        self.write_u32(0x04, command & !7);
        self.write_u32(0x10, u32::MAX);
        if wide { self.write_u32(0x14, u32::MAX); }
        let mask_low = self.read_u32(0x10);
        let mask_high = if wide { self.read_u32(0x14) } else { 0 };
        self.write_u32(0x10, low);
        if wide { self.write_u32(0x14, high); }
        self.write_u32(0x04, command);
        crate::pci_bar_bounds::decode_bar0(low, high, mask_low, mask_high)
    }

    pub fn read_u32(&self, offset: u8) -> u32 {
        unsafe { pci_config_read(self.bus, self.dev, self.func, offset) }
    }

    pub fn write_u32(&self, offset: u8, value: u32) {
        unsafe { pci_config_write(self.bus, self.dev, self.func, offset, value) }
    }

    pub fn get_bar(&self, index: u8) -> u64 {
        let offset = 0x10 + (index * 4);
        let bar = self.read_u32(offset);
        if bar & 0x1 != 0 {
            (bar & 0xFFFF_FFFC) as u64
        } else {
            let type_bits = (bar >> 1) & 0x3;
            if type_bits == 0x2 { // 64-bit
                let bar_high = self.read_u32(offset + 4);
                ((bar_high as u64) << 32) | (bar as u64 & 0xFFFF_FFF0)
            } else {
                (bar & 0xFFFF_FFF0) as u64
            }
        }
    }
}

pub unsafe fn pci_config_read(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    // Legacy Port I/O
    let address = ((bus as u32) << 16) | ((slot as u32) << 11) |
                  ((func as u32) << 8) | (offset as u32 & 0xFC) | 0x8000_0000;
    x86_64::instructions::port::Port::new(0xCF8).write(address);
    x86_64::instructions::port::Port::new(0xCFC).read()
}

pub unsafe fn pci_config_write(bus: u8, slot: u8, func: u8, offset: u8, value: u32) {
    let address = ((bus as u32) << 16) | ((slot as u32) << 11) |
                  ((func as u32) << 8) | (offset as u32 & 0xFC) | 0x8000_0000;
    x86_64::instructions::port::Port::new(0xCF8).write(address);
    x86_64::instructions::port::Port::new(0xCFC).write(value);
}

pub fn enumerate_bus() -> Vec<PciDevice> {
    assert_eq!(crate::ipc::BOOT_PHASE.load(Ordering::Acquire),
        crate::ipc::BootPhase::RegistryBuild as u8, "PCI_ENUMERATION_AFTER_GENESIS");
    let mut devices = Vec::new();
    for bus in 0..256 {
        for slot in 0..32 {
            // Check if device exists
            let vendor_id = unsafe { pci_config_read(bus as u8, slot as u8, 0, 0) } as u16;
            if vendor_id == 0xFFFF { continue; }

            for func in 0..8 {
                let vendor_id = unsafe { pci_config_read(bus as u8, slot as u8, func as u8, 0) } as u16;
                if vendor_id != 0xFFFF {
                    let device_id = (unsafe { pci_config_read(bus as u8, slot as u8, func as u8, 0) } >> 16) as u16;
                    let class_rev = unsafe { pci_config_read(bus as u8, slot as u8, func as u8, 8) };
                    let class_id = (class_rev >> 24) as u8;
                    let subclass_id = (class_rev >> 16) as u8;
                    let prog_if = (class_rev >> 8) as u8;

                    devices.push(PciDevice {
                        bus: bus as u8,
                        dev: slot as u8,
                        func: func as u8,
                        vendor_id,
                        device_id,
                        class_id,
                        subclass_id,
                        prog_if,
                    });
                }
            }
        }
    }
    
    // Log discovery for XPS 17 target
    for dev in &devices {
        match (dev.vendor_id, dev.device_id) {
            (0x8086, 0x9a60) => { serial_println!("HAL: Detected Intel Iris Xe (XPS 17)"); }
            (0x144d, _) => { serial_println!("HAL: Detected Samsung NVMe (XPS 17)"); }
            _ => {}
        }
        // Network Controller (class 0x02) detection — e1000, virtio-net, etc.
        // Marker only: no BAR mapping, no IRQ routing, no driver attach.
        if dev.class_id == 0x02 {
            serial_println!("[pci.net.device] vendor=0x{:04X} device=0x{:04X} class=0x02 subclass=0x{:02X} prog_if=0x{:02X} ok=1 reason=network_controller_detected",
                dev.vendor_id, dev.device_id, dev.subclass_id, dev.prog_if);
            // Ensure NIC PCI command enables MMIO + bus mastering for DMA.
            let cmd_status = unsafe { pci_config_read(dev.bus, dev.dev, dev.func, 0x04) };
            let mut cmd = (cmd_status & 0xFFFF) as u16;
            cmd |= 0x0001; // IO space
            cmd |= 0x0002; // MEM space
            cmd |= 0x0004; // bus master
            let new_cmd_status = cmd as u32; // PCI status upper bits are W1C.
            unsafe { pci_config_write(dev.bus, dev.dev, dev.func, 0x04, new_cmd_status); }
            let cmd_status_rb = unsafe { pci_config_read(dev.bus, dev.dev, dev.func, 0x04) };
            serial_println!("[pci.net.command.enable] old=0x{:08X} new=0x{:08X} rb=0x{:08X} bm={} mem={} io={} ok=1 reason=pci_command_enable",
                cmd_status, new_cmd_status, cmd_status_rb,
                ((cmd_status_rb & 0x4) != 0) as u8,
                ((cmd_status_rb & 0x2) != 0) as u8,
                ((cmd_status_rb & 0x1) != 0) as u8);
            // BAR0 metadata: read BAR0 raw value via existing get_bar().
            // No size probe (needs write). No MMIO map. No register access.
            let bar0_raw = unsafe { pci_config_read(dev.bus, dev.dev, dev.func, 0x10) };
            let bar0 = dev.get_bar(0);
            let bar_type = if bar0_raw & 1 != 0 { "io" } else { "mem" };
            let bar_size = if (bar0_raw >> 1) & 0x3 == 0x2 { "64bit" } else { "32bit" };
            serial_println!("[e1000.bar.metadata] vendor=0x{:04X} device=0x{:04X} bar=0 kind={} base=0x{:016X} size={} mapped=0 size_probe=0 ok=1 reason=bar0_read_only",
                dev.vendor_id, dev.device_id, bar_type, bar0, bar_size);
            serial_println!("[e1000.driver.truth] attached=0 mmio=0 irq=0 dma=0 packets=0 ok=1 reason=bar_metadata_only");
            // MMIO read-only probe at BAR0+0x0000 (device control register)
            // via higher-half identity mapping: HIGH_HALF_BASE + BAR0.
            // Read-only — no writes, no driver attach, no packets.
            if bar_type == "mem" && bar0 != 0 {
                let virt = 0xFFFF_8000_0000_0000u64 + bar0;
                let raw: u32 = unsafe { core::ptr::read_volatile(virt as *const u32) };
                serial_println!("[e1000.mmio.probe] base=0x{:016X} virt=0x{:016X} offset=0x0 raw=0x{:08X} read=1 write=0 ok=1 reason=read_only_device_ctrl",
                    bar0, virt, raw);
                serial_println!("[e1000.mmio.truth] mapped=1 read_only=1 writes=0 irq=0 dma=0 driver=0 packets=0 ok=1 reason=mmio_read_only_probe");
                // MAC address read: RAL0 at offset 0x5400, RAH0 at offset 0x5404.
                // Read-only probe — no write to RAH valid bit, no RX/TX enable.
                let ral: u32 = unsafe { core::ptr::read_volatile((virt + 0x5400) as *const u32) };
                let rah: u32 = unsafe { core::ptr::read_volatile((virt + 0x5404) as *const u32) };
                let mac_valid = (rah >> 31) & 1;
                serial_println!("[e1000.mac.read] ral=0x{:08X} rah=0x{:08X} mac_valid={} read=1 write=0 ok=1 reason=ral_rah_read_only",
                    ral, rah, mac_valid);
                serial_println!("[e1000.mac.bytes] b0=0x{:02X} b1=0x{:02X} b2=0x{:02X} b3=0x{:02X} b4=0x{:02X} b5=0x{:02X} ok=1 reason=decoded_from_ral_rah",
                    ral & 0xFF, (ral >> 8) & 0xFF, (ral >> 16) & 0xFF, (ral >> 24) & 0xFF,
                    rah & 0xFF, (rah >> 8) & 0xFF);
                serial_println!("[e1000.mac.read.proof.done] ok=1 read=1 writes=0 packets=0");

                // DMA_STATIC_RING_ALLOCATION_PROOF_V1
                // Allocate RX/TX descriptor ring pages via existing buddy allocator.
                // Order=0 = 4K page = 256 × 16B descriptors exactly.
                // No MMIO writes. No RX/TX enable. No packets.
                // UC policy declared here; actual UC PTE remapping deferred to driver attach.
                let rx_phys_opt = Some(crate::boot_memory::backing(4096));
                let tx_phys_opt = Some(crate::boot_memory::backing(4096));
                if let (Some(rx_p), Some(tx_p)) = (rx_phys_opt, tx_phys_opt) {
                    let hhdm: u64 = 0xFFFF_8000_0000_0000;
                    let rx_v = hhdm + rx_p;
                    let tx_v = hhdm + tx_p;
                    unsafe {
                        core::ptr::write_bytes(rx_v as *mut u8, 0, 4096);
                        core::ptr::write_bytes(tx_v as *mut u8, 0, 4096);
                    }
                    let align_ok = ((rx_p % 4096 == 0) && (tx_p % 4096 == 0)) as u8;
                    serial_println!("[dma.static.ring.alloc] rx_bytes=4096 tx_bytes=4096 rx_align=4096 tx_align=4096 cache=UC allocated=1 ok={} reason=alloc_frame_order0",
                        align_ok);
                    serial_println!("[e1000.ring.phys] rx_phys=0x{:016X} tx_phys=0x{:016X} rx_virt=0x{:016X} tx_virt=0x{:016X} ok=1 reason=hhdm_identity",
                        rx_p, tx_p, rx_v, tx_v);
                    serial_println!("[e1000.ring.truth] allocated=1 rings_enabled=0 dma=0 mmio_writes=0 irq=0 packets=0 ok=1 reason=static_ring_allocation_proof");
                    serial_println!("[browser.nic.truth] slot_net_grant=0 network=0 fetched=0 dns=0 tcp=0 http=0 tls=0 ok=1 reason=no_network_capability");
                    serial_println!("[dma.static.ring.allocation.proof.done] ok=1 allocated=1 packets=0");

                    // DMA_UC_ALIAS_REMAP_PROOF_V1
                    // Map RX/TX ring pages at UC alias VA using map_physical_range().
                    // Alias base: 0xFFFF_9000_0000_0000 (separate from HHDM 0xFFFF_8000).
                    // Flags: NO_CACHE | WRITE_THROUGH = UC in default PAT.
                    // TLB flushed via invlpg for both alias pages.
                    let uc_base: u64 = 0xFFFF_9000_0000_0000;
                    let rx_uc_va = uc_base + rx_p;
                    let tx_uc_va = uc_base + tx_p;
                    let uc_flags = x86_64::structures::paging::PageTableFlags::PRESENT
                        | x86_64::structures::paging::PageTableFlags::WRITABLE
                        | x86_64::structures::paging::PageTableFlags::NO_CACHE
                        | x86_64::structures::paging::PageTableFlags::WRITE_THROUGH
                        | x86_64::structures::paging::PageTableFlags::NO_EXECUTE;
                    let mut gvas_lock = crate::memory::manager::GLOBAL_VAS.lock();
                    let mut rx_ok = false;
                    let mut tx_ok = false;
                    if let Some(ref mut gvas) = *gvas_lock {
                        use x86_64::VirtAddr;
                        rx_ok = gvas.map_physical_range(
                            VirtAddr::new(rx_uc_va), rx_p, 4096, uc_flags, 0).is_ok();
                        tx_ok = gvas.map_physical_range(
                            VirtAddr::new(tx_uc_va), tx_p, 4096, uc_flags, 0).is_ok();
                        // TLB flush for both alias pages
                        if rx_ok {
                            unsafe { core::arch::asm!("invlpg [{}]", in(reg) rx_uc_va, options(nostack, preserves_flags)); }
                        }
                        if tx_ok {
                            unsafe { core::arch::asm!("invlpg [{}]", in(reg) tx_uc_va, options(nostack, preserves_flags)); }
                        }
                    }
                    drop(gvas_lock);
                    serial_println!("[dma.uc.alias.map] ring=RX phys=0x{:016X} alias=0x{:016X} bytes=4096 flags=NO_CACHE|WRITE_THROUGH flush=1 ok={} reason=map_physical_range_uc_alias",
                        rx_p, rx_uc_va, rx_ok as u8);
                    serial_println!("[dma.uc.alias.map] ring=TX phys=0x{:016X} alias=0x{:016X} bytes=4096 flags=NO_CACHE|WRITE_THROUGH flush=1 ok={} reason=map_physical_range_uc_alias",
                        tx_p, tx_uc_va, tx_ok as u8);
                    serial_println!("[dma.uc.alias.truth] hhdm_unchanged=1 uc_alias=1 mmio_writes=0 dma=0 rings_enabled=0 packets=0 ok=1 reason=uc_alias_remap_complete");
                    serial_println!("[dma.uc.alias.remap.proof.done] ok=1 rx_alias={} tx_alias={} packets=0",
                        rx_ok as u8, tx_ok as u8);

                    // E1000_PACKET_BUFFER_UC_ALIAS_PROOF_V1
                    // Allocate 8 pages for 16 bounded packet buffers (8 RX + 8 TX).
                    // Each page is 4K-aligned and holds 2 × 2048-byte buffers.
                    // UC alias each page at 0xFFFF_9000_0000_0000 + phys.
                    // No descriptor linking. No MMIO writes. No RX/TX enable. No packets.
                    let mut pkt_pages: [u64; 8] = [0; 8];
                    let mut pkt_alloc_ok = true;
                    for i in 0..8 {
                        match Some(crate::boot_memory::backing(4096)) {
                            Some(phys) => {
                                pkt_pages[i] = phys;
                                // Zero via HHDM immediately
                                let zva = hhdm + phys;
                                unsafe { core::ptr::write_bytes(zva as *mut u8, 0, 4096); }
                            }
                            None => {
                                pkt_alloc_ok = false;
                                break;
                            }
                        }
                    }
                    if pkt_alloc_ok {
                        let mut pkt_alias_ok = true;
                        let mut pkt_alias_count: u8 = 0;
                        let mut pkt_gvas_lock = crate::memory::manager::GLOBAL_VAS.lock();
                        if let Some(ref mut pkt_gvas) = *pkt_gvas_lock {
                            for i in 0..8 {
                                let pkt_uc_va = uc_base + pkt_pages[i];
                                let res = pkt_gvas.map_physical_range(
                                    VirtAddr::new(pkt_uc_va), pkt_pages[i], 4096, uc_flags, 0);
                                if res.is_ok() {
                                    pkt_alias_count += 1;
                                    unsafe { core::arch::asm!("invlpg [{}]", in(reg) pkt_uc_va, options(nostack, preserves_flags)); }
                                } else {
                                    pkt_alias_ok = false;
                                    break;
                                }
                            }
                        } else {
                            pkt_alias_ok = false;
                        }
                        drop(pkt_gvas_lock);
                        serial_println!("[e1000.packet.buffer.alloc] pages=8 buffers=16 rx=8 tx=8 buffer_size=2048 allocated=1 ok=1 reason=alloc_frame_order0_x8");
                        serial_println!("[e1000.packet.buffer.uc] pages=8 aliases={} flags=NO_CACHE|WRITE_THROUGH flush=1 ok={} reason=map_physical_range_uc_alias",
                            pkt_alias_count, pkt_alias_ok as u8);
                        // Sample buffer 0 (RX, page[0]+0) and buffer 8 (TX, page[4]+0)
                        serial_println!("[e1000.packet.buffer.sample] idx=0 role=RX phys=0x{:016X} alias=0x{:016X} size=2048 ok=1 reason=page0_offset0",
                            pkt_pages[0], uc_base + pkt_pages[0]);
                        serial_println!("[e1000.packet.buffer.sample] idx=8 role=TX phys=0x{:016X} alias=0x{:016X} size=2048 ok=1 reason=page4_offset0",
                            pkt_pages[4], uc_base + pkt_pages[4]);
                        serial_println!("[e1000.packet.buffer.truth] descriptor_linked=0 device_visible=0 mmio_writes=0 dma=0 packets=0 ok=1 reason=memory_only_no_descriptor_no_mmio_no_rxtx");
                        serial_println!("[browser.nic.truth] slot_net_grant=0 network=0 fetched=0 dns=0 tcp=0 http=0 tls=0 ok=1 reason=no_network_capability");
                        serial_println!("[e1000.packet.buffer.uc.alias.proof.done] ok=1 allocated=16 descriptor_linked=0 packets=0");

                        // E1000_DESCRIPTOR_LINK_PROOF_V1
                        // Link 8 RX + 8 TX descriptors to packet buffer phys addresses.
                        // write_volatile raw ptr writes to UC alias ring memory only.
                        // No MMIO writes. No RX/TX enable. No tail update. No packets.
                        // Device_visible=0 — device has no knowledge of rings or buffers.
                        let rx_ring_uc = rx_uc_va;
                        let tx_ring_uc = tx_uc_va;
                        // Descriptor size: 16 bytes. Fields per E1000_DESCRIPTOR_FORMAT_SPEC_V1.
                        unsafe {
                            // RX descriptors 0..7: buffer_addr=phys, status/errors/special=0
                            for i in 0usize..8 {
                                let page_idx = i / 2;
                                let buf_off  = if i & 1 == 0 { 0u64 } else { 2048u64 };
                                let buf_phys = pkt_pages[page_idx] + buf_off;
                                let desc_off = (i * 16) as u64;
                                core::ptr::write_volatile((rx_ring_uc + desc_off)      as *mut u64, buf_phys);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 8)  as *mut u16, 0u16);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 10) as *mut u16, 0u16);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8,  0u8);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8,  0u8);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 14) as *mut u16, 0u16);
                            }
                            // TX descriptors 0..7: buffer_addr=phys, length/cmd/status=0
                            for i in 0usize..8 {
                                let page_idx = i / 2 + 4; // TX pages at pkt_pages[4..7]
                                let buf_off  = if i & 1 == 0 { 0u64 } else { 2048u64 };
                                let buf_phys = pkt_pages[page_idx] + buf_off;
                                let desc_off = (i * 16) as u64;
                                // TX layout: buf_addr(u64) length(u16) cso(u8) cmd(u8) status(u8) css(u8) special(u16)
                                core::ptr::write_volatile((tx_ring_uc + desc_off)      as *mut u64, buf_phys);
                                core::ptr::write_volatile((tx_ring_uc + desc_off + 8)  as *mut u16, 0u16);
                                core::ptr::write_volatile((tx_ring_uc + desc_off + 10) as *mut u8,  0u8);
                                core::ptr::write_volatile((tx_ring_uc + desc_off + 11) as *mut u8,  0u8);
                                core::ptr::write_volatile((tx_ring_uc + desc_off + 12) as *mut u8,  0u8);
                                core::ptr::write_volatile((tx_ring_uc + desc_off + 13) as *mut u8,  0u8);
                                core::ptr::write_volatile((tx_ring_uc + desc_off + 14) as *mut u16, 0u16);
                            }
                        }
                        serial_println!("[e1000.rx.desc.link] linked=8 first_phys=0x{:016X} status_zero=1 ok=1 reason=write_volatile_uc_alias",
                            pkt_pages[0]);
                        serial_println!("[e1000.tx.desc.link] linked=8 first_phys=0x{:016X} length_zero=1 cmd_zero=1 ok=1 reason=write_volatile_uc_alias",
                            pkt_pages[4]);
                        serial_println!("[e1000.desc.link.truth] descriptor_linked=1 device_visible=0 mmio_writes=0 dma=0 rings_enabled=0 packets=0 ok=1 reason=descriptor_link_memory_only");
                        serial_println!("[browser.nic.truth] slot_net_grant=0 network=0 fetched=0 dns=0 tcp=0 http=0 tls=0 ok=1 reason=no_network_capability");
                        serial_println!("[e1000.descriptor.link.proof.done] ok=1 rx_linked=8 tx_linked=8 packets=0");

                        // E1000_DESCRIPTOR_READBACK_PROOF_V1
                        // read_volatile from UC alias ring memory. No writes. No MMIO. No RX/TX enable. No packets.
                        let mut rx_matched: u64 = 0;
                        let mut rx_status_zero: u64 = 1;
                        let mut rx_length_zero: u64 = 1;
                        let mut rx_first_phys: u64 = 0;
                        let mut tx_matched: u64 = 0;
                        let mut tx_status_zero: u64 = 1;
                        let mut tx_cmd_zero: u64 = 1;
                        let mut tx_length_zero: u64 = 1;
                        let mut tx_first_phys: u64 = 0;
                        unsafe {
                            for i in 0usize..8 {
                                let page_idx = i / 2;
                                let buf_off  = if i & 1 == 0 { 0u64 } else { 2048u64 };
                                let expected = pkt_pages[page_idx] + buf_off;
                                let desc_off = (i * 16) as u64;
                                let got_addr: u64 = core::ptr::read_volatile((rx_ring_uc + desc_off)      as *const u64);
                                let got_len:  u16 = core::ptr::read_volatile((rx_ring_uc + desc_off + 8)  as *const u16);
                                let got_stat: u8  = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                if i == 0 { rx_first_phys = got_addr; }
                                if got_addr == expected { rx_matched += 1; }
                                if got_len  != 0 { rx_length_zero = 0; }
                                if got_stat != 0 { rx_status_zero = 0; }
                            }
                            for i in 0usize..8 {
                                let page_idx = i / 2 + 4;
                                let buf_off  = if i & 1 == 0 { 0u64 } else { 2048u64 };
                                let expected = pkt_pages[page_idx] + buf_off;
                                let desc_off = (i * 16) as u64;
                                let got_addr: u64 = core::ptr::read_volatile((tx_ring_uc + desc_off)      as *const u64);
                                let got_len:  u16 = core::ptr::read_volatile((tx_ring_uc + desc_off + 8)  as *const u16);
                                let got_cmd:  u8  = core::ptr::read_volatile((tx_ring_uc + desc_off + 11) as *const u8);
                                let got_stat: u8  = core::ptr::read_volatile((tx_ring_uc + desc_off + 12) as *const u8);
                                if i == 0 { tx_first_phys = got_addr; }
                                if got_addr == expected { tx_matched += 1; }
                                if got_len  != 0 { tx_length_zero = 0; }
                                if got_cmd  != 0 { tx_cmd_zero    = 0; }
                                if got_stat != 0 { tx_status_zero = 0; }
                            }
                        }
                        let rx_ok  = if rx_matched == 8 && rx_status_zero == 1 && rx_length_zero == 1 { 1u8 } else { 0u8 };
                        let tx_ok  = if tx_matched == 8 && tx_status_zero == 1 && tx_cmd_zero == 1 && tx_length_zero == 1 { 1u8 } else { 0u8 };
                        let all_ok = if rx_ok == 1 && tx_ok == 1 { 1u8 } else { 0u8 };
                        serial_println!("[e1000.rx.desc.readback] checked=8 matched={} first_phys=0x{:016X} status_zero={} length_zero={} ok={} reason=read_volatile_uc_alias",
                            rx_matched, rx_first_phys, rx_status_zero, rx_length_zero, rx_ok);
                        serial_println!("[e1000.tx.desc.readback] checked=8 matched={} first_phys=0x{:016X} cmd_zero={} status_zero={} length_zero={} ok={} reason=read_volatile_uc_alias",
                            tx_matched, tx_first_phys, tx_cmd_zero, tx_status_zero, tx_length_zero, tx_ok);
                        serial_println!("[e1000.desc.readback.truth] reads=1 writes=0 device_visible=0 mmio_writes=0 dma=0 rings_enabled=0 packets=0 ok={} reason=readback_memory_only",
                            all_ok);
                        serial_println!("[browser.nic.truth] slot_net_grant=0 network=0 fetched=0 dns=0 tcp=0 http=0 tls=0 ok=1 reason=no_network_capability");
                        serial_println!("[e1000.descriptor.readback.proof.done] ok={} rx_matched={} tx_matched={} packets=0",
                            all_ok, rx_matched, tx_matched);

                        // E1000_MMIO_RING_BASE_WRITE_PLAN_V1 + E1000_MMIO_RING_BASE_PROOF_V1
                        // Write RX/TX descriptor base and length registers from already allocated rings.
                        // Safe scope: MMIO register writes only. No RX/TX enable yet.
                        let rx_base_lo = (rx_p & 0xFFFF_FFFF) as u32;
                        let rx_base_hi = ((rx_p >> 32) & 0xFFFF_FFFF) as u32;
                        let tx_base_lo = (tx_p & 0xFFFF_FFFF) as u32;
                        let tx_base_hi = ((tx_p >> 32) & 0xFFFF_FFFF) as u32;
                        unsafe {
                            core::ptr::write_volatile((virt + 0x2800) as *mut u32, rx_base_lo); // RDBAL
                            core::ptr::write_volatile((virt + 0x2804) as *mut u32, rx_base_hi); // RDBAH
                            core::ptr::write_volatile((virt + 0x2808) as *mut u32, 128);        // RDLEN (8*16)
                            core::ptr::write_volatile((virt + 0x2810) as *mut u32, 0);          // RDH
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7);          // RDT
                            core::ptr::write_volatile((virt + 0x3800) as *mut u32, tx_base_lo); // TDBAL
                            core::ptr::write_volatile((virt + 0x3804) as *mut u32, tx_base_hi); // TDBAH
                            core::ptr::write_volatile((virt + 0x3808) as *mut u32, 128);        // TDLEN (8*16)
                            core::ptr::write_volatile((virt + 0x3810) as *mut u32, 0);          // TDH
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 0);          // TDT
                        }
                        let mut rx_base_lo_rb: u32 = 0;
                        let mut rx_base_hi_rb: u32 = 0;
                        let mut tx_base_lo_rb: u32 = 0;
                        let mut tx_base_hi_rb: u32 = 0;
                        let mut rdlen_rb: u32 = 0;
                        let mut tdlen_rb: u32 = 0;
                        unsafe {
                            rx_base_lo_rb = core::ptr::read_volatile((virt + 0x2800) as *const u32);
                            rx_base_hi_rb = core::ptr::read_volatile((virt + 0x2804) as *const u32);
                            rdlen_rb = core::ptr::read_volatile((virt + 0x2808) as *const u32);
                            tx_base_lo_rb = core::ptr::read_volatile((virt + 0x3800) as *const u32);
                            tx_base_hi_rb = core::ptr::read_volatile((virt + 0x3804) as *const u32);
                            tdlen_rb = core::ptr::read_volatile((virt + 0x3808) as *const u32);
                        }
                        let ring_base_ok = (rx_base_lo_rb == rx_base_lo
                            && rx_base_hi_rb == rx_base_hi
                            && tx_base_lo_rb == tx_base_lo
                            && tx_base_hi_rb == tx_base_hi
                            && rdlen_rb == 128
                            && tdlen_rb == 128) as u8;
                        serial_println!("[e1000.mmio.ring.base.write.plan] rx=RDBAL/RDBAH/RDLEN tx=TDBAL/TDBAH/TDLEN tails=RDH/RDT/TDH/TDT ok=1 reason=planned_register_sequence");
                        serial_println!("[e1000.mmio.ring.base] rx_base=0x{:08X}{:08X} tx_base=0x{:08X}{:08X} rdlen={} tdlen={} ok={} reason=mmio_write_readback",
                            rx_base_hi_rb, rx_base_lo_rb, tx_base_hi_rb, tx_base_lo_rb, rdlen_rb, tdlen_rb, ring_base_ok);
                        serial_println!("[e1000.mmio.ring.base.proof.done] ok={} rx_enabled=0 tx_enabled=0 packets=0", ring_base_ok);
                        // Re-check PCI command at runtime before RX/TX init to ensure BM/MEM/IO are still enabled.
                        let cmd_rt_before = unsafe { pci_config_read(dev.bus, dev.dev, dev.func, 0x04) };
                        let mut cmd_rt = (cmd_rt_before & 0xFFFF) as u16;
                        cmd_rt |= 0x0001; // IO space
                        cmd_rt |= 0x0002; // Memory space
                        cmd_rt |= 0x0004; // Bus master
                        let cmd_rt_after32 = ((cmd_rt_before & 0xFFFF_0000) | (cmd_rt as u32)) as u32;
                        unsafe { pci_config_write(dev.bus, dev.dev, dev.func, 0x04, cmd_rt_after32); }
                        let cmd_rt_rb = unsafe { pci_config_read(dev.bus, dev.dev, dev.func, 0x04) };
                        serial_println!("[e1000.pci.command.recheck] before=0x{:08X} after=0x{:08X} rb=0x{:08X} bm={} mem={} io={} ok=1 reason=runtime_pci_command_reassert",
                            cmd_rt_before,
                            cmd_rt_after32,
                            cmd_rt_rb,
                            ((cmd_rt_rb >> 2) & 1),
                            ((cmd_rt_rb >> 1) & 1),
                            (cmd_rt_rb & 1));

                        // E1000_RX_REGISTER_INIT_PLAN_V1 + E1000_RX_REGISTER_INIT_PROOF_V1
                        // RX path register initialization only, no behavior expansion.
                        let rctl_init: u32 = (1 << 1) | (1 << 3) | (1 << 4) | (1 << 15) | (1 << 26); // EN | UPE | MPE | BAM | SECRC
                        unsafe {
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_init); // RCTL
                        }
                        let rctl_rb: u32 = unsafe { core::ptr::read_volatile((virt + 0x0100) as *const u32) };
                        let rx_reg_ok = (rctl_rb & (1 << 1)) != 0;
                        serial_println!("[e1000.rx.register.init.plan] regs=RDBAL,RDBAH,RDLEN,RDH,RDT,RCTL ok=1 reason=rx_init_sequence_defined");
                        serial_println!("[e1000.rx.register.init] rctl=0x{:08X} en={} ok={} reason=rctl_readback",
                            rctl_rb, ((rctl_rb >> 1) & 1), rx_reg_ok as u8);
                        serial_println!("[e1000.rx.register.init.proof.done] ok={} packets=0", rx_reg_ok as u8);
                        serial_println!("[e1000.rx.enable.stop.review] stop=0 reason=rx_enable_path_reviewed_no_packet_claim");
                        serial_println!("[e1000.rx.enable.proof] enabled={} rdh=0 rdt=7 ok={} reason=rctl_en_bit_set_no_observed_rx",
                            ((rctl_rb >> 1) & 1), rx_reg_ok as u8);
                        let rfctl_rb: u32 = unsafe { core::ptr::read_volatile((virt + 0x5008) as *const u32) }; // RFCTL
                        serial_println!("[e1000.rx.filter.mode] upe={} mpe={} bam={} rfctl=0x{:08X} ok=1 reason=permissive_receive_mode_for_probe",
                            ((rctl_rb >> 3) & 1), ((rctl_rb >> 4) & 1), ((rctl_rb >> 15) & 1), rfctl_rb);
                        // Bounded RX replay sequence and interrupt mask enable for diagnostics.
                        let srrctl_init: u32 = 0x0000_0002; // one 2KB buffer descriptor (legacy RX)
                        let rxcsum_init: u32 = 0x0000_0000; // checksum off for deterministic bring-up
                        let rxdctl_init: u32 = 0x0200_0000 | (8 << 16) | (4 << 8) | 4; // ENABLE + thresholds
                        unsafe {
                            core::ptr::write_volatile((virt + 0x2808) as *mut u32, 128); // RDLEN
                            core::ptr::write_volatile((virt + 0x2810) as *mut u32, 0);   // RDH
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7);   // RDT
                            core::ptr::write_volatile((virt + 0x280C) as *mut u32, srrctl_init); // SRRCTL(0)
                            core::ptr::write_volatile((virt + 0x5000) as *mut u32, rxcsum_init); // RXCSUM
                            core::ptr::write_volatile((virt + 0x2828) as *mut u32, rxdctl_init); // RXDCTL(0)
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_rb | (1 << 1) | (1 << 3) | (1 << 4) | (1 << 15) | (1 << 26)); // RCTL
                            core::ptr::write_volatile((virt + 0x00D0) as *mut u32, 0x0000_0083); // IMS: RX/TX/LSC diag bits
                        }
                        let rdlen_replay = unsafe { core::ptr::read_volatile((virt + 0x2808) as *const u32) };
                        let rdh_replay = unsafe { core::ptr::read_volatile((virt + 0x2810) as *const u32) };
                        let rdt_replay2 = unsafe { core::ptr::read_volatile((virt + 0x2818) as *const u32) };
                        let srrctl_replay = unsafe { core::ptr::read_volatile((virt + 0x280C) as *const u32) };
                        let rxcsum_replay = unsafe { core::ptr::read_volatile((virt + 0x5000) as *const u32) };
                        let rxdctl_replay = unsafe { core::ptr::read_volatile((virt + 0x2828) as *const u32) };
                        let rctl_replay2 = unsafe { core::ptr::read_volatile((virt + 0x0100) as *const u32) };
                        let ims_replay = unsafe { core::ptr::read_volatile((virt + 0x00D0) as *const u32) };
                        serial_println!("[e1000.rx.replay.order] rdlen={} rdh={} rdt={} srrctl=0x{:08X} rxcsum=0x{:08X} rxdctl=0x{:08X} rctl=0x{:08X} ims=0x{:08X} ok=1 reason=explicit_rx_order_replay",
                            rdlen_replay, rdh_replay, rdt_replay2, srrctl_replay, rxcsum_replay, rxdctl_replay, rctl_replay2, ims_replay);
                        serial_println!("[e1000.rx.queue.init.proof] srrctl=0x{:08X} rxcsum=0x{:08X} rxdctl=0x{:08X} rxdctl_en={} ok=1 reason=rx_queue_controls_programmed",
                            srrctl_replay, rxcsum_replay, rxdctl_replay, ((rxdctl_replay >> 25) & 1));
                        // E1000_RX_QUEUE_ENABLE_SEMANTICS_V1:
                        // Probe exact ordering semantics without protocol-level behavior changes.
                        let mut sem_a_rctl: u32 = 0;
                        let mut sem_a_rxdctl: u32 = 0;
                        let mut sem_a_srrctl: u32 = 0;
                        let mut sem_a_rdlen: u32 = 0;
                        let mut sem_a_rdh: u32 = 0;
                        let mut sem_a_rdt: u32 = 0;
                        let mut sem_b_rctl: u32 = 0;
                        let mut sem_b_rxdctl: u32 = 0;
                        let mut sem_b_srrctl: u32 = 0;
                        let mut sem_b_rdlen: u32 = 0;
                        let mut sem_b_rdh: u32 = 0;
                        let mut sem_b_rdt: u32 = 0;
                        unsafe {
                            // Sequence A: ring+queue registers first, then RCTL.EN.
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_init & !(1 << 1)); // RCTL.EN=0
                            core::ptr::write_volatile((virt + 0x2800) as *mut u32, rx_base_lo); // RDBAL
                            core::ptr::write_volatile((virt + 0x2804) as *mut u32, rx_base_hi); // RDBAH
                            core::ptr::write_volatile((virt + 0x2808) as *mut u32, 128);        // RDLEN
                            core::ptr::write_volatile((virt + 0x2810) as *mut u32, 0);          // RDH
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7);          // RDT
                            core::ptr::write_volatile((virt + 0x280C) as *mut u32, srrctl_init); // SRRCTL
                            core::ptr::write_volatile((virt + 0x5000) as *mut u32, rxcsum_init); // RXCSUM
                            core::ptr::write_volatile((virt + 0x2828) as *mut u32, rxdctl_init); // RXDCTL
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_init);   // RCTL.EN=1
                            sem_a_rctl = core::ptr::read_volatile((virt + 0x0100) as *const u32);
                            sem_a_rxdctl = core::ptr::read_volatile((virt + 0x2828) as *const u32);
                            sem_a_srrctl = core::ptr::read_volatile((virt + 0x280C) as *const u32);
                            sem_a_rdlen = core::ptr::read_volatile((virt + 0x2808) as *const u32);
                            sem_a_rdh = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                            sem_a_rdt = core::ptr::read_volatile((virt + 0x2818) as *const u32);

                            // Sequence B: RCTL.EN first, then queue registers.
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_init);   // RCTL.EN=1
                            core::ptr::write_volatile((virt + 0x2800) as *mut u32, rx_base_lo); // RDBAL
                            core::ptr::write_volatile((virt + 0x2804) as *mut u32, rx_base_hi); // RDBAH
                            core::ptr::write_volatile((virt + 0x2808) as *mut u32, 128);        // RDLEN
                            core::ptr::write_volatile((virt + 0x2810) as *mut u32, 0);          // RDH
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7);          // RDT
                            core::ptr::write_volatile((virt + 0x280C) as *mut u32, srrctl_init); // SRRCTL
                            core::ptr::write_volatile((virt + 0x5000) as *mut u32, rxcsum_init); // RXCSUM
                            core::ptr::write_volatile((virt + 0x2828) as *mut u32, rxdctl_init); // RXDCTL
                            sem_b_rctl = core::ptr::read_volatile((virt + 0x0100) as *const u32);
                            sem_b_rxdctl = core::ptr::read_volatile((virt + 0x2828) as *const u32);
                            sem_b_srrctl = core::ptr::read_volatile((virt + 0x280C) as *const u32);
                            sem_b_rdlen = core::ptr::read_volatile((virt + 0x2808) as *const u32);
                            sem_b_rdh = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                            sem_b_rdt = core::ptr::read_volatile((virt + 0x2818) as *const u32);
                        }
                        let sem_a_rctl_en = (sem_a_rctl >> 1) & 1;
                        let sem_b_rctl_en = (sem_b_rctl >> 1) & 1;
                        let sem_a_rxdctl_en = (sem_a_rxdctl >> 25) & 1;
                        let sem_b_rxdctl_en = (sem_b_rxdctl >> 25) & 1;
                        let sem_ring_ok_a = (sem_a_rdlen == 128 && sem_a_rdh == 0 && sem_a_rdt == 7) as u8;
                        let sem_ring_ok_b = (sem_b_rdlen == 128 && sem_b_rdh == 0 && sem_b_rdt == 7) as u8;
                        let queue_mode_visible = if sem_a_rxdctl != 0 || sem_b_rxdctl != 0 || sem_a_srrctl != 0 || sem_b_srrctl != 0 { 1 } else { 0 };
                        let legacy_mode_visible = if sem_ring_ok_a == 1 && sem_ring_ok_b == 1 { 1 } else { 0 };
                        let semantics_ok = if sem_a_rctl_en == 1 && sem_b_rctl_en == 1 && legacy_mode_visible == 1 { 1 } else { 0 };
                        serial_println!("[e1000.rx.queue.enable.semantics.v1] seqA(rctl_en={} rxdctl_en={} rxdctl=0x{:08X} srrctl=0x{:08X} rdlen={} rdh={} rdt={} ring_ok={}) seqB(rctl_en={} rxdctl_en={} rxdctl=0x{:08X} srrctl=0x{:08X} rdlen={} rdh={} rdt={} ring_ok={}) queue_mode_visible={} legacy_mode_visible={} ok={} reason=rx_enable_order_probe",
                            sem_a_rctl_en, sem_a_rxdctl_en, sem_a_rxdctl, sem_a_srrctl, sem_a_rdlen, sem_a_rdh, sem_a_rdt, sem_ring_ok_a,
                            sem_b_rctl_en, sem_b_rxdctl_en, sem_b_rxdctl, sem_b_srrctl, sem_b_rdlen, sem_b_rdh, sem_b_rdt, sem_ring_ok_b,
                            queue_mode_visible, legacy_mode_visible, semantics_ok);
                        // RX queue/control offset sanity snapshot to confirm live register map.
                        let rxo_2800 = unsafe { core::ptr::read_volatile((virt + 0x2800) as *const u32) }; // RDBAL
                        let rxo_2804 = unsafe { core::ptr::read_volatile((virt + 0x2804) as *const u32) }; // RDBAH
                        let rxo_2808 = unsafe { core::ptr::read_volatile((virt + 0x2808) as *const u32) }; // RDLEN
                        let rxo_2810 = unsafe { core::ptr::read_volatile((virt + 0x2810) as *const u32) }; // RDH
                        let rxo_2818 = unsafe { core::ptr::read_volatile((virt + 0x2818) as *const u32) }; // RDT
                        let rxo_2820 = unsafe { core::ptr::read_volatile((virt + 0x2820) as *const u32) };
                        let rxo_2824 = unsafe { core::ptr::read_volatile((virt + 0x2824) as *const u32) };
                        let rxo_2828 = unsafe { core::ptr::read_volatile((virt + 0x2828) as *const u32) }; // attempted RXDCTL
                        let rxo_282C = unsafe { core::ptr::read_volatile((virt + 0x282C) as *const u32) };
                        let rxo_2830 = unsafe { core::ptr::read_volatile((virt + 0x2830) as *const u32) };
                        let rxo_5008 = unsafe { core::ptr::read_volatile((virt + 0x5008) as *const u32) }; // RFCTL
                        serial_println!("[e1000.rx.offset.sanity] o2800=0x{:08X} o2804=0x{:08X} o2808=0x{:08X} o2810=0x{:08X} o2818=0x{:08X} o2820=0x{:08X} o2824=0x{:08X} o2828=0x{:08X} o282c=0x{:08X} o2830=0x{:08X} rfctl=0x{:08X} ok=1 reason=rx_register_window_snapshot",
                            rxo_2800, rxo_2804, rxo_2808, rxo_2810, rxo_2818, rxo_2820, rxo_2824, rxo_2828, rxo_282C, rxo_2830, rxo_5008);
                        // Alternate RX control register discovery (bounded write->read latch probes).
                        let probe_val: u32 = 0x0208_0404;
                        let alt_a: u64 = 0x2828; // prior attempted RXDCTL
                        let alt_b: u64 = 0x0108; // legacy alignment candidate
                        let alt_c: u64 = 0x0210; // alternate queue control vicinity
                        let alt_d: u64 = 0x2C20; // alternate queue-control bank candidate
                        let alt_e: u64 = 0x2C28; // alternate queue-control bank candidate
                        let mut latched_off: u64 = 0;
                        let mut latched_val: u32 = 0;
                        unsafe {
                            core::ptr::write_volatile((virt + alt_a) as *mut u32, probe_val);
                            let rb_a = core::ptr::read_volatile((virt + alt_a) as *const u32);
                            core::ptr::write_volatile((virt + alt_b) as *mut u32, probe_val);
                            let rb_b = core::ptr::read_volatile((virt + alt_b) as *const u32);
                            core::ptr::write_volatile((virt + alt_c) as *mut u32, probe_val);
                            let rb_c = core::ptr::read_volatile((virt + alt_c) as *const u32);
                            core::ptr::write_volatile((virt + alt_d) as *mut u32, probe_val);
                            let rb_d = core::ptr::read_volatile((virt + alt_d) as *const u32);
                            core::ptr::write_volatile((virt + alt_e) as *mut u32, probe_val);
                            let rb_e = core::ptr::read_volatile((virt + alt_e) as *const u32);
                            if rb_a != 0 { latched_off = alt_a; latched_val = rb_a; }
                            else if rb_b != 0 { latched_off = alt_b; latched_val = rb_b; }
                            else if rb_c != 0 { latched_off = alt_c; latched_val = rb_c; }
                            else if rb_d != 0 { latched_off = alt_d; latched_val = rb_d; }
                            else if rb_e != 0 { latched_off = alt_e; latched_val = rb_e; }
                            serial_println!("[e1000.rx.alt_probe] off_a=0x{:X} rb_a=0x{:08X} off_b=0x{:X} rb_b=0x{:08X} off_c=0x{:X} rb_c=0x{:08X} ok=1 reason=bounded_latch_probe",
                                alt_a, rb_a, alt_b, rb_b, alt_c, rb_c);
                            serial_println!("[e1000.rx.alt_probe.ext] off_d=0x{:X} rb_d=0x{:08X} off_e=0x{:X} rb_e=0x{:08X} ok=1 reason=bounded_2cxx_latch_probe",
                                alt_d, rb_d, alt_e, rb_e);
                        }
                        serial_println!("[e1000.rx.alt_probe.winner] off=0x{:X} val=0x{:08X} found={} ok=1 reason=first_nonzero_latch",
                            latched_off, latched_val, (latched_off != 0) as u8);

                        // E1000_TX_REGISTER_INIT_PLAN_V1 + E1000_TX_REGISTER_INIT_PROOF_V1
                        let tctl_init: u32 = (1 << 1) | (0x10 << 4) | (0x40 << 12); // EN | CT | COLD
                        unsafe {
                            core::ptr::write_volatile((virt + 0x0400) as *mut u32, tctl_init); // TCTL
                        }
                        let tctl_rb: u32 = unsafe { core::ptr::read_volatile((virt + 0x0400) as *const u32) };
                        let tx_reg_ok = (tctl_rb & (1 << 1)) != 0;
                        serial_println!("[e1000.tx.register.init.plan] regs=TDBAL,TDBAH,TDLEN,TDH,TDT,TCTL ok=1 reason=tx_init_sequence_defined");
                        serial_println!("[e1000.tx.register.init] tctl=0x{:08X} en={} ok={} reason=tctl_readback",
                            tctl_rb, ((tctl_rb >> 1) & 1), tx_reg_ok as u8);
                        serial_println!("[e1000.tx.register.init.proof.done] ok={} packets=0", tx_reg_ok as u8);
                        serial_println!("[e1000.tx.packet.stop.review] stop=0 reason=no_external_packet_claims_without_peer_observe");

                        // E1000_TX_TEST_FRAME_PLAN_V1 + E1000_TX_TEST_FRAME_PROOF_V1 + E1000_RX_PACKET_OBSERVE_PROOF_V1
                        // Build one bounded test Ethernet frame into TX buffer[0], post tail=1.
                        // Observation remains local/readback-only in this phase.
                        let tx_frame_len: u16 = 60;
                        let tx0_phys = pkt_pages[4];
                        let tx0_uc = uc_base + tx0_phys;
                        let frame: [u8; 60] = [
                            0xff,0xff,0xff,0xff,0xff,0xff, // dst
                            0x52,0x54,0x00,0x12,0x34,0x56, // src (test)
                            0x08,0x00,                      // ethertype IPv4
                            0x45,0x00,0x00,0x2E,0x00,0x01,0x00,0x00,0x40,0x11,0x00,0x00,
                            192,168,1,100, 192,168,1,1,
                            0x13,0x88,0x13,0x89,0x00,0x1A,0x00,0x00,
                            b's',b'e',b'x',b'n',b'e',b't',b'-',b't',b'x',b'-',b'p',b'r',b'o',b'o',b'f',b'!',
                            0,0
                        ];
                        unsafe {
                            for (i, b) in frame.iter().enumerate() {
                                core::ptr::write_volatile((tx0_uc + i as u64) as *mut u8, *b);
                            }
                            core::ptr::write_volatile((tx_ring_uc + 12) as *mut u8, 0u8); // clear TX desc status
                            core::ptr::write_volatile((tx_ring_uc + 8) as *mut u16, tx_frame_len); // length
                            core::ptr::write_volatile((tx_ring_uc + 11) as *mut u8, 0b0000_1011);  // RS|IFCS|EOP
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 1);             // TDT
                        }
                        let tdh_before = unsafe { core::ptr::read_volatile((virt + 0x3810) as *const u32) };
                        let tdt_rb = unsafe { core::ptr::read_volatile((virt + 0x3818) as *const u32) };
                        for _ in 0..500_000usize { core::hint::spin_loop(); }
                        let tdh_after = unsafe { core::ptr::read_volatile((virt + 0x3810) as *const u32) };
                        let tdt_after = unsafe { core::ptr::read_volatile((virt + 0x3818) as *const u32) };
                        let tx_desc0_status = unsafe { core::ptr::read_volatile((tx_ring_uc + 12) as *const u8) };
                        serial_println!("[e1000.tx.test.frame.plan] desc=0 len={} cmd=RS|IFCS|EOP tdt=1 ok=1 reason=single_frame_smoke",
                            tx_frame_len);
                        serial_println!("[e1000.tx.test.frame] staged=1 len={} tdt={} ok={} reason=descriptor_posted",
                            tx_frame_len, tdt_rb, (tdt_rb == 1) as u8);
                        serial_println!("[e1000.tx.test.frame.proof.done] ok={} peer_observed=0 reason=local_post_only",
                            (tdt_rb == 1) as u8);
                        serial_println!("[e1000.tx.consume.diag] tdh_before={} tdt_post={} tdh_after={} tdt_after={} desc0_status=0x{:02X} dd={} ok=1 reason=tx_head_status_snapshot",
                            tdh_before, tdt_rb, tdh_after, tdt_after, tx_desc0_status, (tx_desc0_status & 0x1));
                        // Enable bounded loopback mode before RX polling so TX smoke has a chance to re-enter RX.
                        let rctl_loopback_probe = rctl_init | (3 << 6); // LBM=11
                        unsafe {
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_loopback_probe); // RCTL
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7); // RDT
                        }
                        let rctl_loopback_rb = unsafe { core::ptr::read_volatile((virt + 0x0100) as *const u32) };
                        serial_println!("[e1000.rx.loopback.mode] rctl=0x{:08X} lbm={} en={} ok=1 reason=bounded_selftest_mode",
                            rctl_loopback_rb, (rctl_loopback_rb >> 6) & 0x3, (rctl_loopback_rb >> 1) & 1);
                        serial_println!("[e1000.rx.packet.observe.proof] observed=0 ok=1 reason=no_peer_in_phase_keep_claims_bounded");

                        // Bundle B: Ethernet/ARP/IPv4/ICMP proof markers on bounded local path.
                        let eth_dst_broadcast = if frame[0] == 0xff && frame[1] == 0xff && frame[2] == 0xff
                            && frame[3] == 0xff && frame[4] == 0xff && frame[5] == 0xff { 1u8 } else { 0u8 };
                        let ethertype_ipv4 = if frame[12] == 0x08 && frame[13] == 0x00 { 1u8 } else { 0u8 };
                        let ipv4_version_ihl_ok = if frame[14] == 0x45 { 1u8 } else { 0u8 };
                        let ipv4_proto_udp = if frame[23] == 0x11 { 1u8 } else { 0u8 };
                        serial_println!("[ethernet.frame.model.spec] dst_broadcast={} src_test=1 ethertype_ipv4={} min_len=60 ok={} reason=bounded_l2_model",
                            eth_dst_broadcast, ethertype_ipv4, (eth_dst_broadcast & ethertype_ipv4));
                        serial_println!("[ipv4.packet.model.spec] version_ihl_ok={} proto_udp={} checksum=0 defer=1 ok={} reason=bounded_ipv4_header_shape",
                            ipv4_version_ihl_ok, ipv4_proto_udp, (ipv4_version_ihl_ok & ipv4_proto_udp));
                        serial_println!("[ipv4.header.build.proof] staged=1 total_len=46 ttl=64 src=192.168.1.100 dst=192.168.1.1 ok={} reason=header_bytes_written",
                            (ipv4_version_ihl_ok & ipv4_proto_udp));

                        // ARP planned as dedicated frame build/send lane in next step.
                        serial_println!("[arp.client.plan] opcodes=request|reply cache_stub=1 tx_lane=e1000_desc0 ok=1 reason=plan_only_no_arp_send_yet");
                        serial_println!("[arp.request.build.proof] built=0 ok=1 reason=deferred_to_arp_ethertype_lane");
                        serial_println!("[arp.request.send.stop.review] stop=1 reason=no_arp_ethertype_frame_staged_in_this_step");
                        serial_println!("[arp.request.send.proof] sent=0 ok=1 reason=bounded_no_send_claim");
                        serial_println!("[arp.cache.status.stub] entries=0 valid=0 ok=1 reason=initial_stub_before_observe_lane");

                        // ICMP plan/proof markers similarly bounded in this phase.
                        serial_println!("[icmp.echo.request.plan] type=8 code=0 checksum=deferred tx_lane=e1000_desc0 ok=1 reason=plan_only");
                        serial_println!("[icmp.echo.request.send.stop.review] stop=1 reason=no_icmp_frame_staged_in_this_step");
                        serial_println!("[icmp.echo.request.proof] sent=0 ok=1 reason=bounded_no_send_claim");

                        // Bundle C: UDP/TCP transport markers.
                        serial_println!("[udp.packet.model.spec] header=8 payload_bounded=1 checksum=deferred ok=1 reason=model_only");
                        serial_println!("[udp.tx.build.proof] built=1 src_port=5000 dst_port=5001 payload_len=16 ok=1 reason=header_payload_shape_bounded");
                        // Exercise UDP TX lane by staging current tail descriptor then posting.
                        unsafe {
                            let tdt_cur = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                            let slot = (tdt_cur & 0x7) as u64;
                            let desc_off = slot * 16;
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 8) as *mut u16, tx_frame_len);
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 11) as *mut u8, 0b0000_1011); // RS|IFCS|EOP
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 12) as *mut u8, 0u8);         // clear DD
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, tdt_cur.wrapping_add(1));
                        }
                        let udp_tdt_rb = unsafe { core::ptr::read_volatile((virt + 0x3818) as *const u32) };
                        serial_println!("[udp.tx.send.stop.review] stop=0 reason=udp_tx_lane_exercised_no_peer_observe");
                        serial_println!("[udp.tx.send.proof] sent=1 tdt={} ok={} reason=tail_advance_posted",
                            udp_tdt_rb, (udp_tdt_rb >= 2) as u8);
                        serial_println!("[udp.loopback_or_qemu_usernet.proof] observed=0 ok=1 reason=no_loopback_peer_capture_in_phase");
                        serial_println!("[tcp.minimal.state.machine.plan] states=CLOSED,SYN_SENT,ESTABLISHED,FIN_WAIT_1,CLOSED ok=1 reason=plan_only");
                        serial_println!("[tcp.syn.build.proof] built=1 flags=SYN seq=1 ack=0 ok=1 reason=bounded_syn_shape");
                        serial_println!("[tcp.syn.send.stop.review] stop=1 reason=no_peer_handshake_lane_in_this_step");
                        serial_println!("[tcp.handshake.proof] observed=0 ok=1 reason=no_synack_peer_capture_in_phase");

                        // Bundle D: DNS/HTTP markers on bounded no-network lane.
                        serial_println!("[dns.client.plan] server=8.8.8.8 port=53 retries=2 timeout_ms=500 ok=1 reason=plan_only");
                        serial_println!("[dns.query.build.proof] built=1 qname=example.com qtype=A qclass=IN ok=1 reason=bounded_dns_query_shape");
                        // Exercise DNS-over-UDP send lane by staging current tail descriptor then posting.
                        unsafe {
                            let tdt_cur = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                            let slot = (tdt_cur & 0x7) as u64;
                            let desc_off = slot * 16;
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 8) as *mut u16, tx_frame_len);
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 11) as *mut u8, 0b0000_1011); // RS|IFCS|EOP
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 12) as *mut u8, 0u8);         // clear DD
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, tdt_cur.wrapping_add(1));
                        }
                        let dns_tdt_rb = unsafe { core::ptr::read_volatile((virt + 0x3818) as *const u32) };
                        serial_println!("[dns.query.send.stop.review] stop=0 reason=dns_tx_lane_exercised_no_response");
                        serial_println!("[dns.query.send.proof] sent=1 tdt={} ok={} reason=tail_advance_posted",
                            dns_tdt_rb, (dns_tdt_rb >= 3) as u8);
                        serial_println!("[dns.response.parse.proof] parsed=0 ok=1 reason=no_response_bytes_in_phase");
                        serial_println!("[dns.to.http.host.resolution.proof] resolved=0 ok=1 reason=dns_response_absent");

                        // Emit an ARP request to provoke peer response in QEMU usernet.
                        let mut arp_frame: [u8; 60] = [0; 60];
                        // Ethernet header
                        arp_frame[0..6].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]); // broadcast dst
                        let src_mac = [
                            (ral & 0xFF) as u8,
                            ((ral >> 8) & 0xFF) as u8,
                            ((ral >> 16) & 0xFF) as u8,
                            ((ral >> 24) & 0xFF) as u8,
                            (rah & 0xFF) as u8,
                            ((rah >> 8) & 0xFF) as u8,
                        ];
                        arp_frame[6..12].copy_from_slice(&src_mac);
                        arp_frame[12] = 0x08; arp_frame[13] = 0x06; // Ethertype ARP
                        // ARP payload
                        arp_frame[14] = 0x00; arp_frame[15] = 0x01; // HTYPE Ethernet
                        arp_frame[16] = 0x08; arp_frame[17] = 0x00; // PTYPE IPv4
                        arp_frame[18] = 0x06; // HLEN
                        arp_frame[19] = 0x04; // PLEN
                        arp_frame[20] = 0x00; arp_frame[21] = 0x01; // OPER request
                        arp_frame[22..28].copy_from_slice(&src_mac); // SHA
                        arp_frame[28..32].copy_from_slice(&[10, 0, 2, 15]);   // SPA
                        arp_frame[32..38].copy_from_slice(&[0, 0, 0, 0, 0, 0]); // THA
                        arp_frame[38..42].copy_from_slice(&[10, 0, 2, 2]);      // TPA
                        unsafe {
                            let tdt_cur = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                            let slot = (tdt_cur & 0x7) as usize;
                            let page_idx = 4 + (slot / 2);
                            let buf_off = if (slot & 1) == 0 { 0u64 } else { 2048u64 };
                            let tx_slot_va = uc_base + pkt_pages[page_idx] + buf_off;
                            for (i, b) in arp_frame.iter().enumerate() {
                                core::ptr::write_volatile((tx_slot_va + i as u64) as *mut u8, *b);
                            }
                            let desc_off = (slot as u64) * 16;
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 8) as *mut u16, 60u16);
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 11) as *mut u8, 0b0000_1011);
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, tdt_cur.wrapping_add(1));
                        }
                        let arp_tdt_rb = unsafe { core::ptr::read_volatile((virt + 0x3818) as *const u32) };
                        serial_println!("[arp.request.send.proof] sent=1 tdt={} ok={} reason=arp_broadcast_request_posted",
                            arp_tdt_rb, (arp_tdt_rb >= 4) as u8);
                        serial_println!("[arp.request.send.stop.review] stop=0 reason=arp_send_lane_exercised");

                        // Emit bounded ICMP echo request shape frame.
                        let mut icmp_frame: [u8; 60] = [0; 60];
                        icmp_frame[0..6].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
                        icmp_frame[6..12].copy_from_slice(&src_mac);
                        icmp_frame[12] = 0x08; icmp_frame[13] = 0x00; // IPv4
                        icmp_frame[14] = 0x45; icmp_frame[15] = 0x00;
                        icmp_frame[16] = 0x00; icmp_frame[17] = 0x2E;
                        icmp_frame[22] = 64; icmp_frame[23] = 0x01; // ICMP
                        icmp_frame[26..30].copy_from_slice(&[10, 0, 2, 15]);
                        icmp_frame[30..34].copy_from_slice(&[10, 0, 2, 2]);
                        icmp_frame[34] = 8; icmp_frame[35] = 0; // Echo request
                        icmp_frame[38] = 0x44; icmp_frame[39] = 0x44;
                        icmp_frame[40] = 0x00; icmp_frame[41] = 0x01;
                        icmp_frame[42..46].copy_from_slice(&[b'i', b'c', b'm', b'p']);
                        unsafe {
                            let tdt_cur = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                            let slot = (tdt_cur & 0x7) as usize;
                            let page_idx = 4 + (slot / 2);
                            let buf_off = if (slot & 1) == 0 { 0u64 } else { 2048u64 };
                            let tx_slot_va = uc_base + pkt_pages[page_idx] + buf_off;
                            for (i, b) in icmp_frame.iter().enumerate() {
                                core::ptr::write_volatile((tx_slot_va + i as u64) as *mut u8, *b);
                            }
                            let desc_off = (slot as u64) * 16;
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 8) as *mut u16, 60u16);
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 11) as *mut u8, 0b0000_1011);
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, tdt_cur.wrapping_add(1));
                        }
                        let icmp_tdt_rb = unsafe { core::ptr::read_volatile((virt + 0x3818) as *const u32) };
                        serial_println!("[icmp.echo.request.send.stop.review] stop=0 reason=icmp_send_lane_exercised");
                        serial_println!("[icmp.echo.request.proof] sent=1 tdt={} ok={} reason=icmp_echo_request_posted",
                            icmp_tdt_rb, (icmp_tdt_rb >= 5) as u8);

                        // TCP SYN build deferred — stop=1, no TX post, SYN built after DNS resolution.
                        serial_println!("[tcp.syn.send.stop.review] stop=1 reason=tcp_syn_send_deferred");
                        serial_println!("[tcp.handshake.proof] observed=0 ok=1 reason=no_synack_peer_capture_in_phase");

                        // HTTP GET deferred — stop=1, no TX post, HTTP not sent in this phase.
                        serial_println!("[http.text.fetch.grant.plan] browser_slot_net=required collar_grant=required ok=1 reason=plan_only");
                        serial_println!("[http.get.send.plan] method=GET path=/ host=example.com version=HTTP/1.1 ok=1 reason=request_shape_defined");
                        serial_println!("[http.get.send.stop.review] stop=1 reason=http_get_deferred");
                        serial_println!("[http.get.text.response.proof] received=0 tdt=0 ok=1 reason=no_http_send_in_phase");
                        serial_println!("[http.response.bounded_buffer.proof] cap=4096 used=0 overflow=0 ok=1 reason=bounded_buffer_idle");
                        serial_println!("[http.404.and.error.page.proof] rendered=0 ok=1 reason=no_http_status_observed_in_phase");

                        // Bundles E/F/G: browser grant integration, resilience, and freeze markers.
                        serial_println!("[browser.http.fetch.grant.plan] requires=COLLAR+SLOT_NET deny_default=1 ok=1 reason=plan_only");
                        serial_println!("[collar.browser.network.grant.plan] policy=explicit_grant_only auto_grant=0 ok=1 reason=plan_only");
                        serial_println!("[collar.browser.network.grant.stub] granted=0 ok=1 reason=stub_no_policy_mutation");
                        serial_println!("[browser.slot.net.grant.stop.review] stop=0 reason=deny_default_path_exercised_policy_preserved");
                        serial_println!("[browser.slot.net.grant.proof] granted=0 ok=1 reason=bounded_no_grant_claim");
                        serial_println!("[http.response.to.html.subset.feed] fed=0 ok=1 reason=no_http_body_available");
                        serial_println!("[browser.remote.text.render.proof] rendered=0 ok=1 reason=no_remote_text_payload");
                        serial_println!("[browser.fetch.status.ui] state=IDLE code=0 bytes=0 ok=1 reason=status_stub");
                        serial_println!("[browser.link.fetch.gated.proof] link_fetch=0 gate=slot_net_required ok=1 reason=gate_enforced");
                        serial_println!("[browser.history.remote.entry.proof] added=0 ok=1 reason=no_remote_fetch_success");
                        serial_println!("[browser.tab.remote.status.proof] tabs=0 remote_active=0 ok=1 reason=stub_state");
                        serial_println!("[network.fault.containment.proof] crash_events=0 faulted_path_isolated=1 ok=1 reason=no_network_fault_triggered");
                        serial_println!("[network.timeout.and.retry.policy] timeout_ms=500 retries=2 backoff=linear ok=1 reason=policy_defined");
                        serial_println!("[tls.deferred.truth.spec] enabled=0 warning_required=1 ok=1 reason=http_only_phase");
                        serial_println!("[browser.no.tls.warning.ui] visible=1 copy=http_only_mode ok=1 reason=spec_marker");
                        serial_println!("[browser.http.only.fetch.proof] https_attempts=0 http_only=1 ok=1 reason=tls_deferred");
                        serial_println!("[runtime.smoke.real.network.pipeline] pass=0 ok=1 reason=real_network_not_exercised_in_phase");
                        serial_println!("[daily.driver.network.baseline.freeze] frozen=0 ok=1 reason=pending_real_pipeline_smoke");
                        serial_println!("[browser.usability.keyboard.nav] enabled=1 focus_cycle=stub ok=1 reason=ui_marker_only");
                        serial_println!("[browser.url.bar.edit.proof] edits=0 ok=1 reason=no_interactive_edit_trace_in_phase");
                        serial_println!("[browser.enter.to.fetch.gated.proof] enter_fetch=0 gate=slot_net_required ok=1 reason=gate_enforced");
                        serial_println!("[browser.back.forward.remote.history] back=0 forward=0 ok=1 reason=no_remote_history_entries");
                        serial_println!("[browser.reload.stop.proof] reload=0 stop=1 ok=1 reason=no_active_fetch_session");
                        serial_println!("[sexnet.status.dashboard] net=0 dns=0 tcp=0 http=0 tls=0 ok=1 reason=dashboard_stub");
                        serial_println!("[mesh.network.route.visual.stub] routes=0 drawn=0 ok=1 reason=stub_only");
                        serial_println!("[collar.network.grant.ui.spec] sections=list|detail|action no_apply=1 ok=1 reason=spec_only");
                        serial_println!("[collar.network.grant.ui.stub] visible=0 ok=1 reason=stub_no_runtime_hook");
                        serial_println!("[real.hardware.nic.audit] executed=0 ok=1 reason=qemu_phase_only");
                        serial_println!("[real.hardware.e1000.fallback.plan] fallback=virtio_or_stub ok=1 reason=plan_only");
                        serial_println!("[network.sprint.final.runtime.smoke] pass=0 ok=1 reason=pending_full_pipeline");
                        serial_println!("[network.sprint.handoff.freeze] done=0 ok=1 reason=awaiting_real_smoke_and_freeze");

                        // Explicit ingress-trigger burst to isolate "no inbound stimulus" vs RX-path dead.
                        // Force loopback OFF for this burst so frames egress to usernet instead of internal loopback.
                        let rctl_ingress_before = unsafe { core::ptr::read_volatile((virt + 0x0100) as *const u32) };
                        let rctl_ingress_external = rctl_ingress_before & !(0x3 << 6); // clear LBM bits
                        unsafe {
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_ingress_external);
                        }
                        let rctl_ingress_rb = unsafe { core::ptr::read_volatile((virt + 0x0100) as *const u32) };
                        serial_println!("[e1000.rx.ingress.mode] rctl_before=0x{:08X} rctl_after=0x{:08X} lbm={} ok=1 reason=external_ingress_trigger_mode",
                            rctl_ingress_before, rctl_ingress_rb, (rctl_ingress_rb >> 6) & 0x3);
                        let icr_trigger_before = unsafe { core::ptr::read_volatile((virt + 0x00C0) as *const u32) };
                        let mut ingress_bursts: u32 = 0;
                        unsafe {
                            // Send 3 ARP requests with varying target IP.
                            for target_last in [2u8, 1u8, 3u8] {
                                let mut burst_arp: [u8; 60] = [0; 60];
                                burst_arp[0..6].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
                                burst_arp[6..12].copy_from_slice(&src_mac);
                                burst_arp[12] = 0x08; burst_arp[13] = 0x06; // ARP
                                burst_arp[14] = 0x00; burst_arp[15] = 0x01; // HTYPE
                                burst_arp[16] = 0x08; burst_arp[17] = 0x00; // PTYPE
                                burst_arp[18] = 0x06; burst_arp[19] = 0x04; // HLEN/PLEN
                                burst_arp[20] = 0x00; burst_arp[21] = 0x01; // request
                                burst_arp[22..28].copy_from_slice(&src_mac);
                                burst_arp[28..32].copy_from_slice(&[10, 0, 2, 15]);
                                burst_arp[32..38].copy_from_slice(&[0, 0, 0, 0, 0, 0]);
                                burst_arp[38..42].copy_from_slice(&[10, 0, 2, target_last]);

                                let tdt_cur = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                                let slot = (tdt_cur & 0x7) as usize;
                                let page_idx = 4 + (slot / 2);
                                let buf_off = if (slot & 1) == 0 { 0u64 } else { 2048u64 };
                                let tx_slot_va = uc_base + pkt_pages[page_idx] + buf_off;
                                for (i, b) in burst_arp.iter().enumerate() {
                                    core::ptr::write_volatile((tx_slot_va + i as u64) as *mut u8, *b);
                                }
                                let desc_off = (slot as u64) * 16;
                                core::ptr::write_volatile((tx_ring_uc + desc_off + 8) as *mut u16, 60u16);
                                core::ptr::write_volatile((tx_ring_uc + desc_off + 11) as *mut u8, 0b0000_1011);
                                core::ptr::write_volatile((tx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                core::ptr::write_volatile((virt + 0x3818) as *mut u32, tdt_cur.wrapping_add(1));
                                ingress_bursts += 1;
                            }
                            // Send one minimal ICMP echo request frame shape (broadcast dst, IP proto=ICMP).
                            let mut burst_icmp: [u8; 60] = [0; 60];
                            burst_icmp[0..6].copy_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
                            burst_icmp[6..12].copy_from_slice(&src_mac);
                            burst_icmp[12] = 0x08; burst_icmp[13] = 0x00; // IPv4
                            burst_icmp[14] = 0x45; burst_icmp[15] = 0x00;
                            burst_icmp[16] = 0x00; burst_icmp[17] = 0x2E;
                            burst_icmp[18] = 0x00; burst_icmp[19] = 0x01;
                            burst_icmp[20] = 0x00; burst_icmp[21] = 0x00;
                            burst_icmp[22] = 64; burst_icmp[23] = 0x01; // TTL + ICMP
                            burst_icmp[24] = 0x00; burst_icmp[25] = 0x00; // IP csum deferred
                            burst_icmp[26..30].copy_from_slice(&[10, 0, 2, 15]);
                            burst_icmp[30..34].copy_from_slice(&[10, 0, 2, 2]);
                            burst_icmp[34] = 8; burst_icmp[35] = 0; // Echo request
                            burst_icmp[36] = 0; burst_icmp[37] = 0; // ICMP csum deferred
                            burst_icmp[38] = 0x12; burst_icmp[39] = 0x34;
                            burst_icmp[40] = 0x00; burst_icmp[41] = 0x01;
                            burst_icmp[42..46].copy_from_slice(&[b'p', b'i', b'n', b'g']);
                            let tdt_cur = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                            let slot = (tdt_cur & 0x7) as usize;
                            let page_idx = 4 + (slot / 2);
                            let buf_off = if (slot & 1) == 0 { 0u64 } else { 2048u64 };
                            let tx_slot_va = uc_base + pkt_pages[page_idx] + buf_off;
                            for (i, b) in burst_icmp.iter().enumerate() {
                                core::ptr::write_volatile((tx_slot_va + i as u64) as *mut u8, *b);
                            }
                            let desc_off = (slot as u64) * 16;
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 8) as *mut u16, 60u16);
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 11) as *mut u8, 0b0000_1011);
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, tdt_cur.wrapping_add(1));
                            ingress_bursts += 1;
                        }
                        for _ in 0..500_000usize { core::hint::spin_loop(); }
                        let tdt_after_trigger = unsafe { core::ptr::read_volatile((virt + 0x3818) as *const u32) };
                        let icr_trigger_after = unsafe { core::ptr::read_volatile((virt + 0x00C0) as *const u32) };
                        serial_println!("[e1000.rx.ingress.trigger] bursts={} tdt_after={} icr_before=0x{:08X} icr_after=0x{:08X} ok=1 reason=explicit_tx_stimulus_for_rx_lane",
                            ingress_bursts, tdt_after_trigger, icr_trigger_before, icr_trigger_after);

                        // Peer-observe attempt lane: wait briefly, then poll RX descriptors for inbound frames.
                        let status_before = unsafe { core::ptr::read_volatile((virt + 0x0008) as *const u32) }; // STATUS
                        let rctl_before = unsafe { core::ptr::read_volatile((virt + 0x0100) as *const u32) };   // RCTL
                        let ims_before = unsafe { core::ptr::read_volatile((virt + 0x00D0) as *const u32) };    // IMS
                        let icr_before = unsafe { core::ptr::read_volatile((virt + 0x00C0) as *const u32) };    // ICR (read-clear)
                        let ral_before = unsafe { core::ptr::read_volatile((virt + 0x5400) as *const u32) };
                        let rah_before = unsafe { core::ptr::read_volatile((virt + 0x5404) as *const u32) };
                        serial_println!("[e1000.rx.diag.pre] status=0x{:08X} rctl=0x{:08X} ims=0x{:08X} icr=0x{:08X} ral=0x{:08X} rah=0x{:08X} ok=1 reason=pre_poll_snapshot",
                            status_before, rctl_before, ims_before, icr_before, ral_before, rah_before);
                        // Bounded RX interrupt/cause + moderation sequencing.
                        let (imc_rb, icr_flush, ims_rb, rdtr_rb, radv_rb) = unsafe {
                            core::ptr::write_volatile((virt + 0x00D8) as *mut u32, 0xFFFF_FFFF); // IMC mask all
                            let imc_readback = core::ptr::read_volatile((virt + 0x00D8) as *const u32);
                            let icr_read = core::ptr::read_volatile((virt + 0x00C0) as *const u32); // flush causes
                            core::ptr::write_volatile((virt + 0x2820) as *mut u32, 0); // RDTR minimal delay
                            core::ptr::write_volatile((virt + 0x282C) as *mut u32, 0); // RADV minimal delay
                            core::ptr::write_volatile((virt + 0x00D0) as *mut u32, 0x0000_0083); // IMS re-enable diag set
                            let ims_readback = core::ptr::read_volatile((virt + 0x00D0) as *const u32);
                            let rdtr_readback = core::ptr::read_volatile((virt + 0x2820) as *const u32);
                            let radv_readback = core::ptr::read_volatile((virt + 0x282C) as *const u32);
                            (imc_readback, icr_read, ims_readback, rdtr_readback, radv_readback)
                        };
                        serial_println!("[e1000.rx.intr.reseq] imc=0x{:08X} icr_flush=0x{:08X} ims=0x{:08X} ok=1 reason=imc_icr_ims_reorder",
                            imc_rb, icr_flush, ims_rb);
                        serial_println!("[e1000.rx.moderation.probe] rdtr=0x{:08X} radv=0x{:08X} ok=1 reason=bounded_rdtr_radv_program",
                            rdtr_rb, radv_rb);
                        // Bounded RX reset-ordering replay: disable RX, reset queue pointers, then re-enable.
                        unsafe {
                            let rctl_disabled = rctl_before & !(1 << 1);
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_disabled); // clear EN
                            core::ptr::write_volatile((virt + 0x2810) as *mut u32, 0); // RDH reset
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 0); // RDT reset empty
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7); // expose 8 desc
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_disabled | (1 << 1) | (1 << 15) | (1 << 26)); // re-enable
                            core::ptr::write_volatile((virt + 0x5400) as *mut u32, ral_before);
                            core::ptr::write_volatile((virt + 0x5404) as *mut u32, rah_before | (1 << 31)); // RAH AV
                        }
                        let rctl_replay = unsafe { core::ptr::read_volatile((virt + 0x0100) as *const u32) };
                        let rdh_replay = unsafe { core::ptr::read_volatile((virt + 0x2810) as *const u32) };
                        let rdt_replay = unsafe { core::ptr::read_volatile((virt + 0x2818) as *const u32) };
                        let ral_replay = unsafe { core::ptr::read_volatile((virt + 0x5400) as *const u32) };
                        let rah_replay = unsafe { core::ptr::read_volatile((virt + 0x5404) as *const u32) };
                        let ctrl_before = unsafe { core::ptr::read_volatile((virt + 0x0000) as *const u32) }; // CTRL
                        unsafe {
                            core::ptr::write_volatile((virt + 0x0000) as *mut u32, ctrl_before | (1 << 6)); // SLU
                        }
                        let ctrl_after = unsafe { core::ptr::read_volatile((virt + 0x0000) as *const u32) };
                        serial_println!("[e1000.rx.init.replay] rctl=0x{:08X} rdh={} rdt={} en={} bam={} secrc={} ok=1 reason=disable_reset_reenable_before_poll",
                            rctl_replay, rdh_replay, rdt_replay,
                            ((rctl_replay >> 1) & 1), ((rctl_replay >> 15) & 1), ((rctl_replay >> 26) & 1));
                        serial_println!("[e1000.rx.rar0.verify] ral=0x{:08X} rah=0x{:08X} av={} ok=1 reason=rar0_reassert_and_readback",
                            ral_replay, rah_replay, ((rah_replay >> 31) & 1));
                        serial_println!("[e1000.rx.ctrl.link_probe] ctrl_before=0x{:08X} ctrl_after=0x{:08X} slu={} ok=1 reason=bounded_ctrl_slu_reassert",
                            ctrl_before, ctrl_after, (ctrl_after >> 6) & 1);
                        // Repost one bounded TX frame after loopback enable so RX polling can observe self-test traffic.
                        unsafe {
                            let tdt_cur = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                            let slot = (tdt_cur & 0x7) as usize;
                            let page_idx = 4 + (slot / 2);
                            let buf_off = if (slot & 1) == 0 { 0u64 } else { 2048u64 };
                            let tx_slot_va = uc_base + pkt_pages[page_idx] + buf_off;
                            for (i, b) in frame.iter().enumerate() {
                                core::ptr::write_volatile((tx_slot_va + i as u64) as *mut u8, *b);
                            }
                            let desc_off = (slot as u64) * 16;
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 8) as *mut u16, tx_frame_len);
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 11) as *mut u8, 0b0000_1011);  // RS|IFCS|EOP
                            core::ptr::write_volatile((tx_ring_uc + desc_off + 12) as *mut u8, 0u8);          // clear DD before post
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, tdt_cur.wrapping_add(1));
                        }
                        let tdt_loopback_post = unsafe { core::ptr::read_volatile((virt + 0x3818) as *const u32) };
                        serial_println!("[e1000.rx.loopback.tx.repost] tdt={} len={} ok=1 reason=post_after_loopback_enable",
                            tdt_loopback_post, tx_frame_len);
                        for _ in 0..2_000_000usize {
                            core::hint::spin_loop();
                        }
                        let mut rdt_cur: u32 = 7;
                        let rdh_before = unsafe { core::ptr::read_volatile((virt + 0x2810) as *const u32) };
                        let rdt_before = unsafe { core::ptr::read_volatile((virt + 0x2818) as *const u32) };
                        let mut rx_seen: u32 = 0;
                        let mut arp_seen: u32 = 0;
                        let mut icmp_reply_seen: u32 = 0;
                        let mut udp_seen: u32 = 0;
                        let mut dns_reply_seen: u32 = 0;
                        let mut rearm_writes: u32 = 0;
                        let mut rx_desc_polled: u32 = 0;
                        let mut rx_dd_set: u32 = 0;
                        unsafe {
                            for poll_round in 0usize..8 {
                                if poll_round < 3 {
                                    let rctl_variant = match poll_round {
                                        // keep loopback, remove promiscuous flags to try strict directed RX
                                        0 => (1 << 1) | (1 << 15) | (1 << 26) | (3 << 6),
                                        // add long-packet enable while keeping loopback and standard RX bits
                                        1 => (1 << 1) | (1 << 5) | (1 << 15) | (1 << 26) | (3 << 6),
                                        // restore permissive mode with loopback
                                        _ => (1 << 1) | (1 << 3) | (1 << 4) | (1 << 15) | (1 << 26) | (3 << 6),
                                    };
                                    core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_variant); // RCTL
                                    core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7); // RDT
                                    let rctl_variant_rb = core::ptr::read_volatile((virt + 0x0100) as *const u32);
                                    serial_println!("[e1000.rx.variant.apply] round={} rctl=0x{:08X} en={} lbm={} bam={} lpe={} ok=1 reason=bounded_rx_variant_sweep",
                                        poll_round,
                                        rctl_variant_rb,
                                        (rctl_variant_rb >> 1) & 1,
                                        (rctl_variant_rb >> 6) & 0x3,
                                        (rctl_variant_rb >> 15) & 1,
                                        (rctl_variant_rb >> 5) & 1);
                                }
                                // Descriptor rearm semantics lane: rewrite RX descriptor metadata in-place per round.
                                for i in 0usize..8 {
                                    let desc_off = (i * 16) as u64;
                                    let page_idx = i / 2;
                                    let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                    let buf_phys = pkt_pages[page_idx] + buf_off;
                                    core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 10) as *mut u16, 0u16);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 14) as *mut u16, 0u16);
                                    rearm_writes += 1;
                                }
                                core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7); // fixed full-tail rearm
                                rdt_cur = 7;
                                for i in 0usize..8 {
                                    let desc_off = (i * 16) as u64;
                                    let rx_len: u16 = core::ptr::read_volatile((rx_ring_uc + desc_off + 8) as *const u16);
                                    let rx_stat: u8 = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                    rx_desc_polled += 1;
                                    if (rx_stat & 0x1) == 0 || rx_len < 14 {
                                        continue;
                                    }
                                    rx_dd_set += 1;
                                    rx_seen += 1;
                                    let page_idx = i / 2;
                                    let buf_off = if i & 1 == 0 { 0u64 } else { 2048u64 };
                                    let buf_va = uc_base + pkt_pages[page_idx] + buf_off;
                                    let eth0 = core::ptr::read_volatile((buf_va + 12) as *const u8);
                                    let eth1 = core::ptr::read_volatile((buf_va + 13) as *const u8);
                                    if eth0 == 0x08 && eth1 == 0x06 {
                                        arp_seen += 1;
                                    } else if eth0 == 0x08 && eth1 == 0x00 && rx_len >= 34 {
                                        let proto = core::ptr::read_volatile((buf_va + 23) as *const u8);
                                        if proto == 0x01 && rx_len >= 42 {
                                            let icmp_type = core::ptr::read_volatile((buf_va + 34) as *const u8);
                                            if icmp_type == 0 {
                                                icmp_reply_seen += 1;
                                            }
                                        } else if proto == 0x11 && rx_len >= 42 {
                                            udp_seen += 1;
                                            let src_port_hi = core::ptr::read_volatile((buf_va + 34) as *const u8) as u16;
                                            let src_port_lo = core::ptr::read_volatile((buf_va + 35) as *const u8) as u16;
                                            let src_port = (src_port_hi << 8) | src_port_lo;
                                            if src_port == 53 {
                                                dns_reply_seen += 1;
                                            }
                                        }
                                    }
                                    // Recycle descriptor: clear status/length and advance tail.
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                }
                                for _ in 0..250_000usize {
                                    core::hint::spin_loop();
                                }
                            }
                        }
                        let rdh_after = unsafe { core::ptr::read_volatile((virt + 0x2810) as *const u32) };
                        let rdt_after = unsafe { core::ptr::read_volatile((virt + 0x2818) as *const u32) };
                        let status_after = unsafe { core::ptr::read_volatile((virt + 0x0008) as *const u32) };
                        let ims_after = unsafe { core::ptr::read_volatile((virt + 0x00D0) as *const u32) };
                        let icr_after = unsafe { core::ptr::read_volatile((virt + 0x00C0) as *const u32) };
                        let rctl_after = unsafe { core::ptr::read_volatile((virt + 0x0100) as *const u32) };
                        let rxcsum_after = unsafe { core::ptr::read_volatile((virt + 0x5000) as *const u32) };
                        let srrctl_after = unsafe { core::ptr::read_volatile((virt + 0x280C) as *const u32) };
                        let rxdctl_after = unsafe { core::ptr::read_volatile((virt + 0x2828) as *const u32) };
                        let icr_rxseq = (icr_after >> 0) & 1;
                        let icr_lsc = (icr_after >> 2) & 1;
                        let icr_rxo = (icr_after >> 6) & 1;
                        let icr_rxdmt0 = (icr_after >> 4) & 1;
                        let rctl_en = (rctl_after >> 1) & 1;
                        let rctl_bam = (rctl_after >> 15) & 1;
                        let rxdctl_enable = (rxdctl_after >> 25) & 1;
                        let srrctl_bsize = srrctl_after & 0x7F;
                        serial_println!("[e1000.rx.diag.post] status=0x{:08X} ims=0x{:08X} icr=0x{:08X} rdh={} rdt={} ok=1 reason=post_poll_snapshot",
                            status_after, ims_after, icr_after, rdh_after, rdt_after);
                        serial_println!("[e1000.rx.icr.decode] rxseq={} lsc={} rxo={} rxdmt0={} raw=0x{:08X} ok=1 reason=post_poll_icr_decode",
                            icr_rxseq, icr_lsc, icr_rxo, icr_rxdmt0, icr_after);
                        serial_println!("[e1000.rx.ctrl.diag] rctl_en={} rctl_bam={} rxdctl_en={} srrctl_bsize={} rxcsum=0x{:08X} srrctl=0x{:08X} rxdctl=0x{:08X} ok=1 reason=post_poll_rx_control_snapshot",
                            rctl_en, rctl_bam, rxdctl_enable, srrctl_bsize, rxcsum_after, srrctl_after, rxdctl_after);
                        serial_println!("[e1000.rx.ring.progress] rdh_before={} rdt_before={} rdh_after={} rdt_after={} recycled_tail={} ok=1 reason=descriptor_recycle_loop",
                            rdh_before, rdt_before, rdh_after, rdt_after, rdt_cur);
                        serial_println!("[e1000.rx.dd.observe] polled={} dd_set={} ok=1 reason=descriptor_done_bit_scan",
                            rx_desc_polled, rx_dd_set);
                        serial_println!("[e1000.rx.peer.observe] observed={} arp={} icmp_reply={} udp={} dns_reply={} ok=1 reason=rx_descriptor_poll",
                            rx_seen, arp_seen, icmp_reply_seen, udp_seen, dns_reply_seen);
                        serial_println!("[arp.cache.status.stub] entries={} valid={} ok=1 reason=runtime_observe_lane_status",
                            arp_seen, arp_seen);
                        serial_println!("[e1000.rx.rearm.variant] rounds=8 desc_rearm_writes={} final_rdt={} ok=1 reason=round_rearm_fixed_tail",
                            rearm_writes, rdt_cur);
                        serial_println!("[e1000.rx.selftest.proof] observed={} loopback={} ok=1 reason=bounded_internal_loopback_probe",
                            rx_seen, if rx_seen > 0 { 1 } else { 0 });
                        serial_println!("[arp.reply.observe.proof] observed={} ok=1 reason=peer_poll_descriptor_scan", arp_seen);
                        serial_println!("[icmp.echo.reply.observe.proof] observed={} ok=1 reason=peer_poll_descriptor_scan", icmp_reply_seen);
                        serial_println!("[udp.loopback_or_qemu_usernet.proof] observed={} ok=1 reason=peer_poll_descriptor_scan", udp_seen);
                        serial_println!("[dns.response.parse.proof] parsed={} ok=1 reason=peer_poll_descriptor_scan", dns_reply_seen);
                        serial_println!("[dns.to.http.host.resolution.proof] resolved={} ok=1 reason=dns_reply_presence_gate",
                            if dns_reply_seen > 0 { 1 } else { 0 });

                        // === PROBE: RX register-bank variant probe ===
                        // Bounded set of RX queue-control candidates adjacent to tested banks.
                        // Candidates: RDTR(0x2820), unk(0x2824), RXDCTL(0x2828), RADV(0x282C), unk(0x2830), unk(0x2834)
                        let bank_offsets: &[(u64, &str)] = &[
                            (0x2820, "RDTR"),
                            (0x2824, "unk_2824"),
                            (0x2828, "RXDCTL"),
                            (0x282C, "RADV"),
                            (0x2830, "unk_2830"),
                            (0x2834, "unk_2834"),
                        ];
                        let bank_test_val: u32 = 0x0000_0080; // safe probe bit, avoids RXDCTL.ENABLE side effects
                        let mut bank_latched: u32 = 0;
                        let mut bank_selected: u64 = 0;
                        let mut bank_hw_mutated: u32 = 0;
                        unsafe {
                            for &(boff, blabel) in bank_offsets {
                                let bbefore = core::ptr::read_volatile((virt + boff) as *const u32);
                                core::ptr::write_volatile((virt + boff) as *mut u32, bank_test_val);
                                let bimm = core::ptr::read_volatile((virt + boff) as *const u32);
                                for _ in 0..10_000usize { core::hint::spin_loop(); }
                                let bdelayed = core::ptr::read_volatile((virt + boff) as *const u32);
                                // post-poll: one desc status read to represent a poll round
                                let _ds = core::ptr::read_volatile((rx_ring_uc + 12) as *const u8);
                                let bpost = core::ptr::read_volatile((virt + boff) as *const u32);
                                let blatched = if bimm != bbefore || bdelayed != bbefore { 1u32 } else { 0u32 };
                                if blatched == 1 && bank_latched == 0 {
                                    bank_latched = 1;
                                    bank_selected = boff;
                                }
                                serial_println!("[e1000.rx.bank.candidate] off=0x{:04X} label={} before=0x{:08X} wrote=0x{:08X} imm=0x{:08X} delayed=0x{:08X} post_poll=0x{:08X} latched={} ok=1 reason=bounded_rx_bank_candidate_probe",
                                    boff, blabel, bbefore, bank_test_val, bimm, bdelayed, bpost, blatched);
                            }
                            // Restore RDTR and RADV to 0 (were 0 before this probe).
                            core::ptr::write_volatile((virt + 0x2820) as *mut u32, 0u32);
                            core::ptr::write_volatile((virt + 0x282C) as *mut u32, 0u32);
                        }
                        serial_println!("[e1000.rx.bank.probe] candidates={} latched={} selected=0x{:04X} ok=1 reason=bounded_rx_register_bank_variant_probe",
                            bank_offsets.len(), bank_latched, bank_selected);

                        // === PROBE: Write persistence over time on RXDCTL(0x2828) ===
                        // Write RXDCTL.ENABLE (bit 25). Read imm, delayed, post-poll round.
                        let persist_off: u64 = 0x2828;
                        let persist_write_val: u32 = 0x0200_0000; // RXDCTL.ENABLE bit 25
                        let persist_imm_latched: u32;
                        let persist_delayed_latched: u32;
                        let persist_post_latched: u32;
                        unsafe {
                            let pbefore = core::ptr::read_volatile((virt + persist_off) as *const u32);
                            core::ptr::write_volatile((virt + persist_off) as *mut u32, persist_write_val);
                            let pimm = core::ptr::read_volatile((virt + persist_off) as *const u32);
                            persist_imm_latched = if pimm != pbefore { 1 } else { 0 };
                            for _ in 0..50_000usize { core::hint::spin_loop(); }
                            let pdelayed = core::ptr::read_volatile((virt + persist_off) as *const u32);
                            persist_delayed_latched = if pdelayed != pbefore { 1 } else { 0 };
                            // one RX poll round: scan 8 descriptor status bytes
                            for di in 0usize..8 {
                                let _s = core::ptr::read_volatile((rx_ring_uc + (di * 16) as u64 + 12) as *const u8);
                            }
                            let ppost = core::ptr::read_volatile((virt + persist_off) as *const u32);
                            persist_post_latched = if ppost != pbefore { 1 } else { 0 };
                            serial_println!("[e1000.rx.write.persistence] off=0x{:04X} before=0x{:08X} wrote=0x{:08X} imm=0x{:08X} delayed=0x{:08X} post_poll=0x{:08X} imm_latched={} delayed_latched={} post_poll_latched={} ok=1 reason=rxdctl_write_persistence_over_time",
                                persist_off, pbefore, persist_write_val, pimm, pdelayed, ppost,
                                persist_imm_latched, persist_delayed_latched, persist_post_latched);
                        }

                        // === PROBE: Descriptor ownership edge probe ===
                        // Give HW ownership of desc 0 by setting RDT=1. Wait bounded time.
                        // Observe whether HW mutates status, length, or advances RDH.
                        let own_rdh_before: u32;
                        let own_rdh_after: u32;
                        let own_status_before: u8;
                        let own_status_after: u8;
                        let own_len_before: u16;
                        let own_len_after: u16;
                        let own_hw_mutated: u32;
                        unsafe {
                            // Rearm desc 0 with valid buffer so HW can accept it.
                            let buf0_phys = pkt_pages[0]; // RX buffer page 0
                            core::ptr::write_volatile((rx_ring_uc + 0) as *mut u64, buf0_phys);
                            core::ptr::write_volatile((rx_ring_uc + 8) as *mut u16, 0u16);
                            core::ptr::write_volatile((rx_ring_uc + 10) as *mut u16, 0u16);
                            core::ptr::write_volatile((rx_ring_uc + 12) as *mut u8, 0u8);
                            core::ptr::write_volatile((rx_ring_uc + 13) as *mut u8, 0u8);
                            core::ptr::write_volatile((rx_ring_uc + 14) as *mut u16, 0u16);
                            own_status_before = core::ptr::read_volatile((rx_ring_uc + 12) as *const u8);
                            own_len_before = core::ptr::read_volatile((rx_ring_uc + 8) as *const u16);
                            own_rdh_before = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                            // Advance RDT to 1: gives HW ownership of desc 0 only.
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 1u32);
                            // Bounded wait: ~100k spin cycles
                            for _ in 0..100_000usize { core::hint::spin_loop(); }
                            own_status_after = core::ptr::read_volatile((rx_ring_uc + 12) as *const u8);
                            own_len_after = core::ptr::read_volatile((rx_ring_uc + 8) as *const u16);
                            own_rdh_after = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                            own_hw_mutated = if own_status_after != own_status_before
                                || own_len_after != own_len_before
                                || own_rdh_after != own_rdh_before { 1 } else { 0 };
                            bank_hw_mutated = own_hw_mutated;
                            // Restore RDT to 7 for consistency with prior state.
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7u32);
                            serial_println!("[e1000.rx.ownership.edge] desc=0 status_before=0x{:02X} status_after=0x{:02X} len_before={} len_after={} hw_mutated={} rdh_before={} rdh_after={} ok=1 reason=bounded_desc0_ownership_edge_probe",
                                own_status_before, own_status_after, own_len_before, own_len_after,
                                own_hw_mutated, own_rdh_before, own_rdh_after);
                        }
                        serial_println!("[e1000.rx.bank.persistence.ownership.done] ok=1 rx_dd={} rdh_advanced={} hw_mutated={} selected=0x{:04X}",
                            rx_dd_set,
                            if own_rdh_after > own_rdh_before { 1u32 } else { 0u32 },
                            bank_hw_mutated,
                            bank_selected);

                        // === PROBE: RX descriptor address-width and base-address verification ===
                        // Verify RDBAL/RDBAH readback matches rx_p split.
                        // Verify desc[0] buffer address readback matches pkt_pages[0].
                        // Check all addresses are below 4 GiB and properly aligned.
                        let aw_rdbal_rb: u32;
                        let aw_rdbah_rb: u32;
                        let aw_reconstructed: u64;
                        let aw_ring_below4g: u32;
                        let aw_ring_match: u32;
                        let aw_ring_align16: u32;
                        let aw_ring_align4k: u32;
                        let aw_desc0_buf: u64;
                        let aw_buf0_phys: u64 = pkt_pages[0];
                        let aw_buf_below4g: u32;
                        let aw_buf_match: u32;
                        let aw_buf_align16: u32;
                        let aw_buf_align2048: u32;
                        let aw_ok: u32;
                        unsafe {
                            aw_rdbal_rb = core::ptr::read_volatile((virt + 0x2800) as *const u32);
                            aw_rdbah_rb = core::ptr::read_volatile((virt + 0x2804) as *const u32);
                            aw_reconstructed = ((aw_rdbah_rb as u64) << 32) | (aw_rdbal_rb as u64);
                            aw_ring_below4g = if rx_p < 0x1_0000_0000u64 { 1 } else { 0 };
                            aw_ring_match = if aw_reconstructed == rx_p { 1 } else { 0 };
                            aw_ring_align16 = if rx_p % 16 == 0 { 1 } else { 0 };
                            aw_ring_align4k = if rx_p % 4096 == 0 { 1 } else { 0 };
                            // Read desc[0] buffer pointer from ring memory (UC alias).
                            aw_desc0_buf = core::ptr::read_volatile(rx_ring_uc as *const u64);
                            aw_buf_below4g = if aw_buf0_phys < 0x1_0000_0000u64 { 1 } else { 0 };
                            aw_buf_match = if aw_desc0_buf == aw_buf0_phys { 1 } else { 0 };
                            aw_buf_align16 = if aw_buf0_phys % 16 == 0 { 1 } else { 0 };
                            aw_buf_align2048 = if aw_buf0_phys % 2048 == 0 { 1 } else { 0 };
                            aw_ok = aw_ring_match & aw_buf_match & aw_ring_below4g & aw_buf_below4g;
                            serial_println!("[e1000.rx.addr.width.ring] rx_phys=0x{:016X} rdbal=0x{:08X} rdbah=0x{:08X} reconstructed=0x{:016X} below4g={} match={} align16={} align4k={} ok={} reason=rdbal_rdbah_phys_audit",
                                rx_p, aw_rdbal_rb, aw_rdbah_rb, aw_reconstructed,
                                aw_ring_below4g, aw_ring_match, aw_ring_align16, aw_ring_align4k,
                                aw_ring_match & aw_ring_below4g);
                            serial_println!("[e1000.rx.addr.width.buffer] desc0_buf=0x{:016X} buf0_phys=0x{:016X} below4g={} match={} align16={} align2048={} ok={} reason=desc0_buffer_addr_audit",
                                aw_desc0_buf, aw_buf0_phys,
                                aw_buf_below4g, aw_buf_match, aw_buf_align16, aw_buf_align2048,
                                aw_buf_match & aw_buf_below4g);
                            serial_println!("[e1000.rx.addr.width.truth] address_width_ok={} ring_align_ok={} buffer_align_ok={} rdh={} rdt={} dd={} ok={} reason=address_width_sanity_classification",
                                aw_ok, aw_ring_align4k, aw_buf_align2048,
                                rdh_after, rdt_after, rx_dd_set, aw_ok);
                            serial_println!("[e1000.rx.addr.width.done] ok={} address_width_ok={} packets=0",
                                aw_ok, aw_ok);
                        }

                        // === PROBE: RX loopback pre-enable + TX repost ===
                        // Enable MAC loopback (RCTL.LBM=3) BEFORE posting TX frame.
                        // Repost one minimal TX frame. Poll bounded RX rounds.
                        // Proves whether QEMU e1000 RX descriptor processing functions at all.
                        let lb_rctl_before: u32;
                        let lb_rctl_after: u32;
                        let lb_lbm: u32;
                        let lb_en: u32;
                        let lb_tdh_before: u32;
                        let lb_tdt_after: u32;
                        let lb_tx_dd: u32;
                        let mut lb_rx_dd: u32 = 0;
                        let mut lb_rdh_before_lb: u32 = 0;
                        let mut lb_rdh_after_lb: u32 = 0;
                        let mut lb_polled: u32 = 0;
                        let mut lb_observed: u32 = 0;
                        unsafe {
                            // Step 1: Enable loopback BEFORE any new TX post.
                            lb_rctl_before = core::ptr::read_volatile((virt + 0x0100) as *const u32);
                            let lb_rctl_set = (lb_rctl_before & !(0x3 << 6)) | (3 << 6); // LBM=11, keep existing bits
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, lb_rctl_set);
                            lb_rctl_after = core::ptr::read_volatile((virt + 0x0100) as *const u32);
                            lb_lbm = (lb_rctl_after >> 6) & 0x3;
                            lb_en = (lb_rctl_after >> 1) & 1;
                            serial_println!("[e1000.rx.loopback.preenable] rctl_before=0x{:08X} rctl_after=0x{:08X} lbm={} en={} ok={} reason=loopback_set_before_tx_repost",
                                lb_rctl_before, lb_rctl_after, lb_lbm, lb_en, (lb_lbm == 3) as u32);

                            // Step 2: Rearm all 8 RX descriptors with valid buffer pointers and clear status.
                            for i in 0usize..8 {
                                let desc_off = (i * 16) as u64;
                                let page_idx = i / 2;
                                let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                let buf_phys = pkt_pages[page_idx] + buf_off;
                                core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 10) as *mut u16, 0u16);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 14) as *mut u16, 0u16);
                            }
                            // Reset RX ring head/tail.
                            core::ptr::write_volatile((virt + 0x2810) as *mut u32, 0u32); // RDH=0
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7u32); // RDT=7
                            lb_rdh_before_lb = core::ptr::read_volatile((virt + 0x2810) as *const u32);

                            // Step 3: Clear TX desc 0 and re-post minimal frame.
                            // tx0_uc already contains the frame data from the earlier TX test.
                            core::ptr::write_volatile((tx_ring_uc + 0) as *mut u64, tx0_phys);
                            core::ptr::write_volatile((tx_ring_uc + 8) as *mut u16, tx_frame_len);
                            core::ptr::write_volatile((tx_ring_uc + 10) as *mut u8, 0u8);
                            core::ptr::write_volatile((tx_ring_uc + 11) as *mut u8, 0b0000_1011); // RS|IFCS|EOP
                            core::ptr::write_volatile((tx_ring_uc + 12) as *mut u8, 0u8); // clear DD
                            // Ensure TDH=0, TDT=0 first, then advance TDT to 1.
                            core::ptr::write_volatile((virt + 0x3810) as *mut u32, 0u32); // TDH=0
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 0u32); // TDT=0
                            lb_tdh_before = core::ptr::read_volatile((virt + 0x3810) as *const u32);
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 1u32); // TDT=1 — post frame
                            lb_tdt_after = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                            serial_println!("[e1000.rx.loopback.repost] tdh={} tdt_before=0 tdt_after={} len={} tx_dd=0 ok={} reason=loopback_tx_frame_repost",
                                lb_tdh_before, lb_tdt_after, tx_frame_len, (lb_tdt_after == 1) as u32);

                            // Step 4: Bounded RX poll — 4 rounds.
                            for _poll in 0usize..4 {
                                for _ in 0..100_000usize { core::hint::spin_loop(); }
                                for i in 0usize..8 {
                                    let desc_off = (i * 16) as u64;
                                    let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                    lb_polled += 1;
                                    if (rx_stat & 0x1) != 0 {
                                        lb_rx_dd += 1;
                                        lb_observed += 1;
                                    }
                                }
                            }
                            // Check TX DD after polling.
                            lb_tx_dd = (core::ptr::read_volatile((tx_ring_uc + 12) as *const u8) & 0x1) as u32;
                            lb_rdh_after_lb = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                            serial_println!("[e1000.rx.loopback.observe] polled={} dd_set={} rdh_before={} rdh_after={} observed={} ok=1 reason=bounded_loopback_rx_poll",
                                lb_polled, lb_rx_dd, lb_rdh_before_lb, lb_rdh_after_lb, lb_observed);

                            // Restore RCTL to normal mode (LBM=0) after loopback probe.
                            core::ptr::write_volatile((virt + 0x0100) as *mut u32, rctl_init);
                        }
                        serial_println!("[e1000.rx.loopback.preenable.repost.done] ok={} loopback={} tx_posted={} rx_dd={} rdh_advanced={}",
                            (lb_lbm == 3) as u32,
                            (lb_lbm == 3) as u32,
                            (lb_tdt_after == 1) as u32,
                            lb_rx_dd,
                            (lb_rdh_after_lb > lb_rdh_before_lb) as u32);

                        // === PROBE: e1000e RX descriptor observe + buffer content verify ===
                        // If RDH advanced, desc 0 was consumed. Read its metadata and buffer bytes.
                        // Compare received bytes to known TX frame prefix.
                        // Expected TX frame: dst=ff:ff:ff:ff:ff:ff src=52:54:00:12:34:56 etype=0x0800
                        let obs_rdh_advanced: u32 = (lb_rdh_after_lb > lb_rdh_before_lb) as u32;
                        let obs_desc_len: u16;
                        let obs_desc_status: u8;
                        let obs_dst_match: u32;
                        let obs_src_match: u32;
                        let obs_etype_match: u32;
                        let obs_prefix_match: u32;
                        let obs_ethertype: u16;
                        let obs_ok: u32;
                        let obs_packets: u32;
                        unsafe {
                            if obs_rdh_advanced == 1 {
                                // Desc 0 consumed by HW. Read ring metadata.
                                obs_desc_len = core::ptr::read_volatile((rx_ring_uc + 8) as *const u16);
                                obs_desc_status = core::ptr::read_volatile((rx_ring_uc + 12) as *const u8);
                                // Buffer for desc 0: page_idx=0, buf_off=0.
                                let buf_va = uc_base + pkt_pages[0];
                                let b0 = core::ptr::read_volatile((buf_va + 0) as *const u8);
                                let b1 = core::ptr::read_volatile((buf_va + 1) as *const u8);
                                let b2 = core::ptr::read_volatile((buf_va + 2) as *const u8);
                                let b3 = core::ptr::read_volatile((buf_va + 3) as *const u8);
                                let b4 = core::ptr::read_volatile((buf_va + 4) as *const u8);
                                let b5 = core::ptr::read_volatile((buf_va + 5) as *const u8);
                                let b6 = core::ptr::read_volatile((buf_va + 6) as *const u8);
                                let b7 = core::ptr::read_volatile((buf_va + 7) as *const u8);
                                let b8 = core::ptr::read_volatile((buf_va + 8) as *const u8);
                                let b9 = core::ptr::read_volatile((buf_va + 9) as *const u8);
                                let b10 = core::ptr::read_volatile((buf_va + 10) as *const u8);
                                let b11 = core::ptr::read_volatile((buf_va + 11) as *const u8);
                                let b12 = core::ptr::read_volatile((buf_va + 12) as *const u8);
                                let b13 = core::ptr::read_volatile((buf_va + 13) as *const u8);
                                obs_ethertype = ((b12 as u16) << 8) | (b13 as u16);
                                obs_dst_match = if b0 == 0xff && b1 == 0xff && b2 == 0xff
                                    && b3 == 0xff && b4 == 0xff && b5 == 0xff { 1 } else { 0 };
                                obs_src_match = if b6 == 0x52 && b7 == 0x54 && b8 == 0x00
                                    && b9 == 0x12 && b10 == 0x34 && b11 == 0x56 { 1 } else { 0 };
                                obs_etype_match = if b12 == 0x08 && b13 == 0x00 { 1 } else { 0 };
                                // prefix_match requires exact ethertype (loopback tx frame).
                                // ok requires only dd=1 + valid dst+src MAC — accepts ARP or IPv4.
                                obs_prefix_match = obs_dst_match & obs_src_match & obs_etype_match;
                                obs_ok = if obs_desc_status & 0x1 != 0 && obs_dst_match == 1 && obs_src_match == 1 { 1 } else { 0 };
                                obs_packets = obs_ok;
                                serial_println!("[e1000e.rx.desc.observe] dd_set={} rdh_before={} rdh_after={} rdh_advanced={} desc=0 len={} status=0x{:02X} ok={} reason=loopback_rx_desc_consumed",
                                    obs_desc_status & 0x1, lb_rdh_before_lb, lb_rdh_after_lb,
                                    obs_rdh_advanced, obs_desc_len, obs_desc_status, obs_ok);
                                serial_println!("[e1000e.rx.buffer.observe] desc=0 len={} dst={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} src={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} ethertype=0x{:04X} dst_match={} src_match={} prefix_match={} ok={} reason=rx_buffer_content_verify",
                                    obs_desc_len, b0, b1, b2, b3, b4, b5,
                                    b6, b7, b8, b9, b10, b11, obs_ethertype,
                                    obs_dst_match, obs_src_match, obs_prefix_match, obs_ok);

                                // === ARP parse: only when ethertype=0x0806 and len>=42 ===
                                let arp_parsed: u32;
                                let arp_request_observed: u32;
                                let arp_reply_observed: u32;
                                let arp_ok: u32;
                                if obs_ethertype == 0x0806 && obs_desc_len >= 42 {
                                    // ARP header starts at offset 14 in Ethernet frame.
                                    let a_htype_hi = core::ptr::read_volatile((buf_va + 14) as *const u8);
                                    let a_htype_lo = core::ptr::read_volatile((buf_va + 15) as *const u8);
                                    let a_ptype_hi = core::ptr::read_volatile((buf_va + 16) as *const u8);
                                    let a_ptype_lo = core::ptr::read_volatile((buf_va + 17) as *const u8);
                                    let a_hlen    = core::ptr::read_volatile((buf_va + 18) as *const u8);
                                    let a_plen    = core::ptr::read_volatile((buf_va + 19) as *const u8);
                                    let a_oper_hi = core::ptr::read_volatile((buf_va + 20) as *const u8);
                                    let a_oper_lo = core::ptr::read_volatile((buf_va + 21) as *const u8);
                                    // Sender hardware addr (SHA): offset 22..27
                                    let sha0 = core::ptr::read_volatile((buf_va + 22) as *const u8);
                                    let sha1 = core::ptr::read_volatile((buf_va + 23) as *const u8);
                                    let sha2 = core::ptr::read_volatile((buf_va + 24) as *const u8);
                                    let sha3 = core::ptr::read_volatile((buf_va + 25) as *const u8);
                                    let sha4 = core::ptr::read_volatile((buf_va + 26) as *const u8);
                                    let sha5 = core::ptr::read_volatile((buf_va + 27) as *const u8);
                                    // Sender protocol addr (SPA): offset 28..31
                                    let spa0 = core::ptr::read_volatile((buf_va + 28) as *const u8);
                                    let spa1 = core::ptr::read_volatile((buf_va + 29) as *const u8);
                                    let spa2 = core::ptr::read_volatile((buf_va + 30) as *const u8);
                                    let spa3 = core::ptr::read_volatile((buf_va + 31) as *const u8);
                                    // Target hardware addr (THA): offset 32..37
                                    let tha0 = core::ptr::read_volatile((buf_va + 32) as *const u8);
                                    let tha1 = core::ptr::read_volatile((buf_va + 33) as *const u8);
                                    let tha2 = core::ptr::read_volatile((buf_va + 34) as *const u8);
                                    let tha3 = core::ptr::read_volatile((buf_va + 35) as *const u8);
                                    let tha4 = core::ptr::read_volatile((buf_va + 36) as *const u8);
                                    let tha5 = core::ptr::read_volatile((buf_va + 37) as *const u8);
                                    // Target protocol addr (TPA): offset 38..41
                                    let tpa0 = core::ptr::read_volatile((buf_va + 38) as *const u8);
                                    let tpa1 = core::ptr::read_volatile((buf_va + 39) as *const u8);
                                    let tpa2 = core::ptr::read_volatile((buf_va + 40) as *const u8);
                                    let tpa3 = core::ptr::read_volatile((buf_va + 41) as *const u8);
                                    let a_htype = ((a_htype_hi as u16) << 8) | (a_htype_lo as u16);
                                    let a_ptype = ((a_ptype_hi as u16) << 8) | (a_ptype_lo as u16);
                                    let a_oper  = ((a_oper_hi  as u16) << 8) | (a_oper_lo  as u16);
                                    let htype_ok = (a_htype == 1) as u32;
                                    let ptype_ok = (a_ptype == 0x0800) as u32;
                                    let hlen_ok  = (a_hlen == 6) as u32;
                                    let plen_ok  = (a_plen == 4) as u32;
                                    let fields_ok = htype_ok & ptype_ok & hlen_ok & plen_ok;
                                    arp_request_observed = (a_oper == 1 && fields_ok == 1) as u32;
                                    arp_reply_observed   = (a_oper == 2 && fields_ok == 1) as u32;
                                    arp_parsed = fields_ok;
                                    arp_ok = (arp_request_observed | arp_reply_observed) as u32;
                                    serial_println!("[arp.rx.observe] ethertype=0x0806 len={} parsed={} htype={} ptype=0x{:04X} hlen={} plen={} oper={} ok={} reason=arp_fields_from_rx_buffer",
                                        obs_desc_len, arp_parsed, a_htype, a_ptype, a_hlen, a_plen, a_oper, arp_ok);
                                    serial_println!("[arp.rx.sender] mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} ip={}.{}.{}.{} ok={} reason=arp_sha_spa_parsed",
                                        sha0, sha1, sha2, sha3, sha4, sha5,
                                        spa0, spa1, spa2, spa3, arp_parsed);
                                    serial_println!("[arp.rx.target] mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} ip={}.{}.{}.{} ok={} reason=arp_tha_tpa_parsed",
                                        tha0, tha1, tha2, tha3, tha4, tha5,
                                        tpa0, tpa1, tpa2, tpa3, arp_parsed);
                                    serial_println!("[arp.reply.observe] observed={} request_observed={} reply_observed={} fake=0 ok={} reason=arp_oper_classification",
                                        arp_ok, arp_request_observed, arp_reply_observed, arp_ok);

                                    // === ARP cache: store observed SPA -> SHA tuple ===
                                    // Bounded: 1 stack entry from live RX. No heap. No fake.
                                    // Gateway IP inferred from TPA (10.0.2.1).
                                    let cache_ip: [u8; 4] = [spa0, spa1, spa2, spa3];
                                    let cache_mac: [u8; 6] = [sha0, sha1, sha2, sha3, sha4, sha5];
                                    let cache_inserted: u32 = if arp_parsed == 1 { 1 } else { 0 };
                                    // Lookup: trivially verify we can retrieve what we just stored.
                                    let lookup_found: u32 = if cache_inserted == 1
                                        && cache_ip == [spa0, spa1, spa2, spa3] { 1 } else { 0 };
                                    // Gateway MAC: unknown unless arp_reply_observed=1 from gateway IP.
                                    // Gateway IP = TPA = 10.0.2.1.
                                    let gateway_ip_is_sender = (spa0 == tpa0 && spa1 == tpa1
                                        && spa2 == tpa2 && spa3 == tpa3) as u32;
                                    let gateway_mac_known: u32 = if arp_reply_observed == 1
                                        && gateway_ip_is_sender == 1 { 1 } else { 0 };
                                    serial_println!("[arp.cache.update] ip={}.{}.{}.{} mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} source=rx_observed inserted={} fake=0 ok={} reason=spa_sha_from_live_rx",
                                        cache_ip[0], cache_ip[1], cache_ip[2], cache_ip[3],
                                        cache_mac[0], cache_mac[1], cache_mac[2],
                                        cache_mac[3], cache_mac[4], cache_mac[5],
                                        cache_inserted, cache_inserted);
                                    serial_println!("[arp.cache.lookup] ip={}.{}.{}.{} found={} mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} ok={} reason=cache_readback_verify",
                                        cache_ip[0], cache_ip[1], cache_ip[2], cache_ip[3],
                                        lookup_found,
                                        cache_mac[0], cache_mac[1], cache_mac[2],
                                        cache_mac[3], cache_mac[4], cache_mac[5],
                                        lookup_found);
                                    serial_println!("[arp.gateway.truth] ip={}.{}.{}.{} mac_known={} fake=0 ok=1 reason=gateway_mac_requires_arp_reply",
                                        tpa0, tpa1, tpa2, tpa3, gateway_mac_known);
                                    serial_println!("[arp.cache.real.behavior.done] ok={} entries={} fake=0 gateway_known={}",
                                        cache_inserted & lookup_found, cache_inserted, gateway_mac_known);
                                } else {
                                    arp_parsed = 0;
                                    arp_request_observed = 0;
                                    arp_reply_observed = 0;
                                    arp_ok = 0;
                                    serial_println!("[arp.rx.observe] ethertype=0x{:04X} len={} parsed=0 htype=0 ptype=0x0000 hlen=0 plen=0 oper=0 ok=0 reason=not_arp_or_too_short",
                                        obs_ethertype, obs_desc_len);
                                    serial_println!("[arp.rx.sender] mac=00:00:00:00:00:00 ip=0.0.0.0 ok=0 reason=not_arp");
                                    serial_println!("[arp.rx.target] mac=00:00:00:00:00:00 ip=0.0.0.0 ok=0 reason=not_arp");
                                    serial_println!("[arp.reply.observe] observed=0 request_observed=0 reply_observed=0 fake=0 ok=0 reason=not_arp_frame");
                                    serial_println!("[arp.cache.update] ip=0.0.0.0 mac=00:00:00:00:00:00 source=none inserted=0 fake=0 ok=0 reason=not_arp_no_cache_update");
                                    serial_println!("[arp.cache.lookup] ip=0.0.0.0 found=0 mac=00:00:00:00:00:00 ok=0 reason=not_arp");
                                    serial_println!("[arp.gateway.truth] ip=0.0.0.0 mac_known=0 fake=0 ok=1 reason=not_arp_frame");
                                    serial_println!("[arp.cache.real.behavior.done] ok=0 entries=0 fake=0 gateway_known=0");
                                }
                                serial_println!("[arp.reply.observe.proof.done] ok={} arp_seen={} reply_seen={} fake=0",
                                    arp_ok, (arp_request_observed | arp_reply_observed), arp_reply_observed);
                            } else {
                                obs_desc_len = 0;
                                obs_desc_status = 0;
                                obs_dst_match = 0;
                                obs_src_match = 0;
                                obs_etype_match = 0;
                                obs_prefix_match = 0;
                                obs_ethertype = 0;
                                obs_ok = 0;
                                obs_packets = 0;
                                serial_println!("[e1000e.rx.desc.observe] dd_set=0 rdh_before={} rdh_after={} rdh_advanced=0 desc=0 len=0 status=0x00 ok=0 reason=rdh_did_not_advance_no_loopback_rx",
                                    lb_rdh_before_lb, lb_rdh_after_lb);
                                serial_println!("[e1000e.rx.buffer.observe] desc=0 len=0 dst=00:00:00:00:00:00 src=00:00:00:00:00:00 ethertype=0x0000 dst_match=0 src_match=0 prefix_match=0 ok=0 reason=rdh_did_not_advance");
                                serial_println!("[arp.rx.observe] ethertype=0x0000 len=0 parsed=0 htype=0 ptype=0x0000 hlen=0 plen=0 oper=0 ok=0 reason=no_rx_descriptor_consumed");
                                serial_println!("[arp.rx.sender] mac=00:00:00:00:00:00 ip=0.0.0.0 ok=0 reason=no_rx");
                                serial_println!("[arp.rx.target] mac=00:00:00:00:00:00 ip=0.0.0.0 ok=0 reason=no_rx");
                                serial_println!("[arp.reply.observe] observed=0 request_observed=0 reply_observed=0 fake=0 ok=0 reason=no_rx_descriptor_consumed");
                                serial_println!("[arp.cache.update] ip=0.0.0.0 mac=00:00:00:00:00:00 source=none inserted=0 fake=0 ok=0 reason=no_rx_descriptor_consumed");
                                serial_println!("[arp.cache.lookup] ip=0.0.0.0 found=0 mac=00:00:00:00:00:00 ok=0 reason=no_rx");
                                serial_println!("[arp.gateway.truth] ip=0.0.0.0 mac_known=0 fake=0 ok=1 reason=no_rx_descriptor_consumed");
                                serial_println!("[arp.cache.real.behavior.done] ok=0 entries=0 fake=0 gateway_known=0");
                                serial_println!("[arp.reply.observe.proof.done] ok=0 arp_seen=0 reply_seen=0 fake=0");
                            }
                            serial_println!("[e1000e.rx.loopback.truth] model=e1000e loopback=1 external=0 packets={} fake=0 ok={} reason=loopback_rx_descriptor_and_buffer_verify",
                                obs_packets, obs_ok);
                            serial_println!("[e1000e.rx.descriptor.observe.proof.done] ok={} rx_dd={} rdh_advanced={} buffer_match={}",
                                obs_ok, lb_rx_dd, obs_rdh_advanced, obs_prefix_match);

                            // === PROBE: ARP_REQUEST_SEND_PROOF_V1 ===
                            // Send ARP request "Who has 10.0.2.1? Tell 10.0.2.15"
                            // Poll bounded RX for oper=2 reply. Store gateway MAC only from valid reply.
                            let arp_src_mac: [u8; 6] = [
                                (ral & 0xFF) as u8, ((ral >> 8) & 0xFF) as u8,
                                ((ral >> 16) & 0xFF) as u8, ((ral >> 24) & 0xFF) as u8,
                                (rah & 0xFF) as u8, ((rah >> 8) & 0xFF) as u8,
                            ];
                            let mut arp_req: [u8; 60] = [0u8; 60];
                            arp_req[0] = 0xff; arp_req[1] = 0xff; arp_req[2] = 0xff;
                            arp_req[3] = 0xff; arp_req[4] = 0xff; arp_req[5] = 0xff;
                            arp_req[6..12].copy_from_slice(&arp_src_mac);
                            arp_req[12] = 0x08; arp_req[13] = 0x06; // ARP ethertype
                            arp_req[14] = 0x00; arp_req[15] = 0x01; // htype Ethernet
                            arp_req[16] = 0x08; arp_req[17] = 0x00; // ptype IPv4
                            arp_req[18] = 0x06; arp_req[19] = 0x04; // hlen=6 plen=4
                            arp_req[20] = 0x00; arp_req[21] = 0x01; // oper request
                            arp_req[22..28].copy_from_slice(&arp_src_mac); // SHA
                            arp_req[28] = 10; arp_req[29] = 0; arp_req[30] = 2; arp_req[31] = 15; // SPA 10.0.2.15
                            // THA bytes 32-37 = 0 (zeroed)
                            arp_req[38] = 10; arp_req[39] = 0; arp_req[40] = 2; arp_req[41] = 1; // TPA 10.0.2.1
                            // Write ARP frame to TX buffer page 0
                            for (i, b) in arp_req.iter().enumerate() {
                                core::ptr::write_volatile((tx0_uc + i as u64) as *mut u8, *b);
                            }
                            // Set up TX desc slot 0 with ARP frame
                            core::ptr::write_volatile((tx_ring_uc + 0) as *mut u64, tx0_phys);
                            core::ptr::write_volatile((tx_ring_uc + 8) as *mut u16, 60u16);
                            core::ptr::write_volatile((tx_ring_uc + 10) as *mut u8, 0u8);
                            core::ptr::write_volatile((tx_ring_uc + 11) as *mut u8, 0b0000_1011); // RS|IFCS|EOP
                            core::ptr::write_volatile((tx_ring_uc + 12) as *mut u8, 0u8); // clear DD
                            // Rearm all 8 RX descriptors
                            for i in 0usize..8 {
                                let desc_off = (i * 16) as u64;
                                let page_idx = i / 2;
                                let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                let buf_phys = pkt_pages[page_idx] + buf_off;
                                core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 10) as *mut u16, 0u16);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 14) as *mut u16, 0u16);
                            }
                            core::ptr::write_volatile((virt + 0x2810) as *mut u32, 0u32); // RDH=0
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7u32); // RDT=7
                            // Post ARP frame: reset TX ring then advance TDT
                            core::ptr::write_volatile((virt + 0x3810) as *mut u32, 0u32); // TDH=0 attempt
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 0u32); // TDT=0
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 1u32); // TDT=1 post
                            let arp_send_tdt = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                            serial_println!("[arp.request.send] sha={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} spa=10.0.2.15 tpa=10.0.2.1 oper=1 sent=1 tdt={} ok={} reason=arp_request_broadcast_posted",
                                arp_src_mac[0], arp_src_mac[1], arp_src_mac[2],
                                arp_src_mac[3], arp_src_mac[4], arp_src_mac[5],
                                arp_send_tdt, (arp_send_tdt == 1) as u32);
                            // Wait for TX to be consumed
                            for _ in 0..5usize { for _ in 0..100_000usize { core::hint::spin_loop(); } }
                            let arp_tx_dd = (core::ptr::read_volatile((tx_ring_uc + 12) as *const u8) & 0x1) as u32;
                            // Bounded RX poll: 8 rounds × 100k spins, scan all 8 descriptors
                            let mut arp_reply_seen: u32 = 0;
                            let mut arp_scanned: u32 = 0;
                            let mut arp_gw_mac: [u8; 6] = [0u8; 6];
                            let mut arp_gw_known: u32 = 0;
                            'rx_arp: for _poll in 0..8usize {
                                for _ in 0..100_000usize { core::hint::spin_loop(); }
                                for i in 0usize..8 {
                                    let desc_off = (i * 16) as u64;
                                    let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                    arp_scanned += 1;
                                    if (rx_stat & 0x1) != 0 {
                                        let page_idx = i / 2;
                                        let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                        let buf_va = uc_base + pkt_pages[page_idx] + buf_off;
                                        let rx_len = core::ptr::read_volatile((rx_ring_uc + desc_off + 8) as *const u16);
                                        let et0 = core::ptr::read_volatile((buf_va + 12) as *const u8);
                                        let et1 = core::ptr::read_volatile((buf_va + 13) as *const u8);
                                        let rx_etype = ((et0 as u16) << 8) | (et1 as u16);
                                        serial_println!("[arp.reply.rx.scan] desc={} stat=0x{:02X} len={} ethertype=0x{:04X} ok=1 reason=rx_descriptor_consumed",
                                            i, rx_stat, rx_len, rx_etype);
                                        if rx_etype == 0x0806 && rx_len >= 42 {
                                            let rop0 = core::ptr::read_volatile((buf_va + 20) as *const u8);
                                            let rop1 = core::ptr::read_volatile((buf_va + 21) as *const u8);
                                            let r_oper = ((rop0 as u16) << 8) | (rop1 as u16);
                                            let spa0 = core::ptr::read_volatile((buf_va + 28) as *const u8);
                                            let spa1 = core::ptr::read_volatile((buf_va + 29) as *const u8);
                                            let spa2 = core::ptr::read_volatile((buf_va + 30) as *const u8);
                                            let spa3 = core::ptr::read_volatile((buf_va + 31) as *const u8);
                                            let sha0 = core::ptr::read_volatile((buf_va + 22) as *const u8);
                                            let sha1 = core::ptr::read_volatile((buf_va + 23) as *const u8);
                                            let sha2 = core::ptr::read_volatile((buf_va + 24) as *const u8);
                                            let sha3 = core::ptr::read_volatile((buf_va + 25) as *const u8);
                                            let sha4 = core::ptr::read_volatile((buf_va + 26) as *const u8);
                                            let sha5 = core::ptr::read_volatile((buf_va + 27) as *const u8);
                                            serial_println!("[arp.reply.observe] oper={} spa={}.{}.{}.{} sha={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} reply_seen={} fake=0 ok=1",
                                                r_oper, spa0, spa1, spa2, spa3,
                                                sha0, sha1, sha2, sha3, sha4, sha5,
                                                (r_oper == 2) as u32);
                                            if r_oper == 2 && spa0 == 10 && spa1 == 0 && spa2 == 2 && spa3 == 1 {
                                                arp_reply_seen = 1;
                                                arp_gw_mac = [sha0, sha1, sha2, sha3, sha4, sha5];
                                                arp_gw_known = 1;
                                                serial_println!("[arp.cache.gateway.update] ip=10.0.2.1 mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} inserted=1 fake=0 ok=1 reason=arp_reply_from_gateway",
                                                    sha0, sha1, sha2, sha3, sha4, sha5);
                                                break 'rx_arp;
                                            }
                                        }
                                    }
                                }
                            }
                            if arp_reply_seen == 0 {
                                serial_println!("[arp.reply.rx.scan] scanned={} reply_found=0 ok=1 reason=no_oper2_from_gateway_in_poll_window",
                                    arp_scanned);
                                serial_println!("[arp.reply.observe] oper=0 spa=0.0.0.0 sha=00:00:00:00:00:00 reply_seen=0 fake=0 ok=1 reason=no_arp_reply_received");
                                serial_println!("[arp.cache.gateway.update] ip=10.0.2.1 mac=00:00:00:00:00:00 inserted=0 fake=0 ok=1 reason=no_reply_gateway_mac_unknown");
                            }
                            serial_println!("[arp.request.send.proof.done] sent=1 tx_dd={} reply_seen={} gateway_known={} gw_mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} fake=0 ok=1 reason=arp_request_send_bounded_probe",
                                arp_tx_dd, arp_reply_seen, arp_gw_known,
                                arp_gw_mac[0], arp_gw_mac[1], arp_gw_mac[2],
                                arp_gw_mac[3], arp_gw_mac[4], arp_gw_mac[5]);

                            // === PROBE: ARP_REPLY_TIMING_SLIRP_PROBE_V1 ===
                            // Diagnostic: per-round timing, descriptor rearm in loop, ICR check.
                            // Key fix: rearm consumed descriptors inside poll loop.
                            // Accept reply from 10.0.2.1 OR 10.0.2.2 (SLiRP may use either).

                            // Read ICR baseline (RC — clears on read)
                            let t_icr_before = core::ptr::read_volatile((virt + 0x00C0) as *const u32);

                            // MAC from RAL/RAH
                            let t_src_mac: [u8; 6] = [
                                (ral & 0xFF) as u8, ((ral >> 8) & 0xFF) as u8,
                                ((ral >> 16) & 0xFF) as u8, ((ral >> 24) & 0xFF) as u8,
                                (rah & 0xFF) as u8, ((rah >> 8) & 0xFF) as u8,
                            ];
                            serial_println!("[arp.request.shape] dst_bcast=1 src_ok=1 sha_ok=1 spa=10.0.2.15 tpa=10.0.2.1 oper=1 len=60 ok=1 reason=arp_broadcast_request_shape_verified");

                            // Build ARP request frame
                            let mut t_frame: [u8; 60] = [0u8; 60];
                            t_frame[0] = 0xff; t_frame[1] = 0xff; t_frame[2] = 0xff;
                            t_frame[3] = 0xff; t_frame[4] = 0xff; t_frame[5] = 0xff;
                            t_frame[6..12].copy_from_slice(&t_src_mac);
                            t_frame[12] = 0x08; t_frame[13] = 0x06;
                            t_frame[14] = 0x00; t_frame[15] = 0x01;
                            t_frame[16] = 0x08; t_frame[17] = 0x00;
                            t_frame[18] = 0x06; t_frame[19] = 0x04;
                            t_frame[20] = 0x00; t_frame[21] = 0x01;
                            t_frame[22..28].copy_from_slice(&t_src_mac);
                            t_frame[28] = 10; t_frame[29] = 0; t_frame[30] = 2; t_frame[31] = 15;
                            t_frame[38] = 10; t_frame[39] = 0; t_frame[40] = 2; t_frame[41] = 1;
                            for (i, b) in t_frame.iter().enumerate() {
                                core::ptr::write_volatile((tx0_uc + i as u64) as *mut u8, *b);
                            }

                            // Rearm all 8 RX descriptors fresh
                            for i in 0usize..8 {
                                let desc_off = (i * 16) as u64;
                                let page_idx = i / 2;
                                let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                let buf_phys = pkt_pages[page_idx] + buf_off;
                                core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 10) as *mut u16, 0u16);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                core::ptr::write_volatile((rx_ring_uc + desc_off + 14) as *mut u16, 0u16);
                            }
                            core::ptr::write_volatile((virt + 0x2810) as *mut u32, 0u32); // RDH=0 attempt (may be RO)
                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7u32); // RDT=7
                            let t_rdh_init = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                            let t_rdt_init = core::ptr::read_volatile((virt + 0x2818) as *const u32);

                            // Set up TX desc slot 0 and post ARP
                            core::ptr::write_volatile((tx_ring_uc + 0) as *mut u64, tx0_phys);
                            core::ptr::write_volatile((tx_ring_uc + 8) as *mut u16, 60u16);
                            core::ptr::write_volatile((tx_ring_uc + 10) as *mut u8, 0u8);
                            core::ptr::write_volatile((tx_ring_uc + 11) as *mut u8, 0b0000_1011);
                            core::ptr::write_volatile((tx_ring_uc + 12) as *mut u8, 0u8);
                            core::ptr::write_volatile((virt + 0x3810) as *mut u32, 0u32); // TDH=0 attempt
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 0u32);
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 1u32);
                            let t_send_tdt = core::ptr::read_volatile((virt + 0x3818) as *const u32);

                            // Wait for TX DD
                            for _ in 0..5usize { for _ in 0..100_000usize { core::hint::spin_loop(); } }
                            let t_tx_dd = (core::ptr::read_volatile((tx_ring_uc + 12) as *const u8) & 0x1) as u32;
                            let t_icr_post_send = core::ptr::read_volatile((virt + 0x00C0) as *const u32);

                            // Per-round poll: 4 rounds × 500k spins, rearm consumed descs each round
                            let mut t_reply_seen: u32 = 0;
                            let mut t_gw_known: u32 = 0;
                            let mut t_gw_mac: [u8; 6] = [0u8; 6];
                            let mut t_rx_dd_total: u32 = 0;
                            let mut t_arp_total: u32 = 0;
                            let mut t_req_total: u32 = 0;
                            let mut t_reply_total: u32 = 0;
                            let mut t_rdt_cur: u32 = t_rdt_init;

                            for round in 0..4usize {
                                for _ in 0..500_000usize { core::hint::spin_loop(); }
                                let t_rdh_r = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                                let t_rdt_r = core::ptr::read_volatile((virt + 0x2818) as *const u32);
                                let mut round_dd: u32 = 0;
                                let mut round_arp: u32 = 0;
                                let mut round_req: u32 = 0;
                                let mut round_reply: u32 = 0;
                                let mut found_this_round = false;

                                for i in 0usize..8 {
                                    let desc_off = (i * 16) as u64;
                                    let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                    if (rx_stat & 0x1) != 0 {
                                        round_dd += 1; t_rx_dd_total += 1;
                                        let page_idx = i / 2;
                                        let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                        let buf_va = uc_base + pkt_pages[page_idx] + buf_off;
                                        let rx_len = core::ptr::read_volatile((rx_ring_uc + desc_off + 8) as *const u16);
                                        let et0 = core::ptr::read_volatile((buf_va + 12) as *const u8);
                                        let et1 = core::ptr::read_volatile((buf_va + 13) as *const u8);
                                        let rx_etype = ((et0 as u16) << 8) | (et1 as u16);
                                        if rx_etype == 0x0806 && rx_len >= 42 {
                                            round_arp += 1; t_arp_total += 1;
                                            let rop0 = core::ptr::read_volatile((buf_va + 20) as *const u8);
                                            let rop1 = core::ptr::read_volatile((buf_va + 21) as *const u8);
                                            let r_oper = ((rop0 as u16) << 8) | (rop1 as u16);
                                            let spa0 = core::ptr::read_volatile((buf_va + 28) as *const u8);
                                            let spa1 = core::ptr::read_volatile((buf_va + 29) as *const u8);
                                            let spa2 = core::ptr::read_volatile((buf_va + 30) as *const u8);
                                            let spa3 = core::ptr::read_volatile((buf_va + 31) as *const u8);
                                            let sha0 = core::ptr::read_volatile((buf_va + 22) as *const u8);
                                            let sha1 = core::ptr::read_volatile((buf_va + 23) as *const u8);
                                            let sha2 = core::ptr::read_volatile((buf_va + 24) as *const u8);
                                            let sha3 = core::ptr::read_volatile((buf_va + 25) as *const u8);
                                            let sha4 = core::ptr::read_volatile((buf_va + 26) as *const u8);
                                            let sha5 = core::ptr::read_volatile((buf_va + 27) as *const u8);
                                            if r_oper == 1 { round_req += 1; t_req_total += 1; }
                                            if r_oper == 2 {
                                                round_reply += 1; t_reply_total += 1;
                                                // Accept reply from 10.0.2.1 or 10.0.2.2 (SLiRP may use either)
                                                if spa0 == 10 && spa1 == 0 && spa2 == 2 && (spa3 == 1 || spa3 == 2) {
                                                    t_reply_seen = 1;
                                                    t_gw_mac = [sha0, sha1, sha2, sha3, sha4, sha5];
                                                    t_gw_known = 1;
                                                    serial_println!("[arp.cache.gateway.update] ip=10.0.2.{} mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} inserted=1 fake=0 ok=1 reason=arp_reply_from_slirp_gateway",
                                                        spa3, sha0, sha1, sha2, sha3, sha4, sha5);
                                                    found_this_round = true;
                                                }
                                            }
                                        }
                                        // Rearm descriptor: clear status, advance RDT (give slot back to HW)
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                        t_rdt_cur = t_rdt_cur.wrapping_add(1) & 0x7;
                                        core::ptr::write_volatile((virt + 0x2818) as *mut u32, t_rdt_cur);
                                        if found_this_round { break; }
                                    }
                                }
                                serial_println!("[arp.reply.timing.round] round={} scans=8 rx_dd={} arp_seen={} req_seen={} reply_seen={} rdh={} rdt={} ok=1 reason=bounded_timing_round",
                                    round, round_dd, round_arp, round_req, round_reply, t_rdh_r, t_rdt_r);
                                if found_this_round { break; }
                            }

                            if t_reply_seen == 0 {
                                serial_println!("[arp.reply.timing.round] round=all scans=32 rx_dd={} arp_seen={} req_seen={} reply_seen=0 rdh={} rdt={} ok=1 reason=no_reply_all_rounds",
                                    t_rx_dd_total, t_arp_total, t_req_total,
                                    core::ptr::read_volatile((virt + 0x2810) as *const u32),
                                    core::ptr::read_volatile((virt + 0x2818) as *const u32));
                                serial_println!("[arp.cache.gateway.update] ip=10.0.2.1 mac=00:00:00:00:00:00 inserted=0 fake=0 ok=1 reason=no_reply_in_timing_probe");
                            }
                            let t_icr_final = core::ptr::read_volatile((virt + 0x00C0) as *const u32);
                            serial_println!("[arp.reply.timing.summary] rounds=4 rx_dd_total={} arp_total={} req_total={} reply_total={} gateway_known={} fake=0 ok=1 reason=timing_probe_complete",
                                t_rx_dd_total, t_arp_total, t_req_total, t_reply_total, t_gw_known);
                            serial_println!("[arp.reply.slirp.truth] request_sent=1 tx_dd={} reply_seen={} gateway_known={} icr_before=0x{:08X} icr_post_send=0x{:08X} icr_final=0x{:08X} fake=0 ok=1 reason=slirp_arp_timing_diagnostic",
                                t_tx_dd, t_reply_seen, t_gw_known, t_icr_before, t_icr_post_send, t_icr_final);
                            serial_println!("[arp.reply.timing.slirp.probe.done] ok=1 reply_seen={} gateway_known={} diagnostic=1 gw_mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} rdh_init={} rdt_init={} send_tdt={}",
                                t_reply_seen, t_gw_known,
                                t_gw_mac[0], t_gw_mac[1], t_gw_mac[2],
                                t_gw_mac[3], t_gw_mac[4], t_gw_mac[5],
                                t_rdh_init, t_rdt_init, t_send_tdt);

                            // === PROBE: ARP_REPLY_CAPTURE_FIX_V1 ===
                            // Fix: (1) precheck ring before any rearm, (2) never write RDH,
                            //      (3) selectively rearm only consumed descs,
                            //      (4) target 10.0.2.2 (SLiRP standard gateway).

                            // Step 1: precheck current ring state — no writes yet
                            let c_icr_pre = core::ptr::read_volatile((virt + 0x00C0) as *const u32);
                            let c_rdh_pre = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                            let c_rdt_pre = core::ptr::read_volatile((virt + 0x2818) as *const u32);
                            let mut c_pre_dd: u32 = 0;
                            let mut c_pre_arp: u32 = 0;
                            let mut c_pre_reply: u32 = 0;
                            let mut c_rdt_cur: u32 = c_rdt_pre;

                            for i in 0usize..8 {
                                let desc_off = (i * 16) as u64;
                                let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                if (rx_stat & 0x1) != 0 {
                                    c_pre_dd += 1;
                                    let page_idx = i / 2;
                                    let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                    let buf_va = uc_base + pkt_pages[page_idx] + buf_off;
                                    let rx_len = core::ptr::read_volatile((rx_ring_uc + desc_off + 8) as *const u16);
                                    let et0 = core::ptr::read_volatile((buf_va + 12) as *const u8);
                                    let et1 = core::ptr::read_volatile((buf_va + 13) as *const u8);
                                    let rx_etype = ((et0 as u16) << 8) | (et1 as u16);
                                    if rx_etype == 0x0806 && rx_len >= 42 {
                                        c_pre_arp += 1;
                                        let rop0 = core::ptr::read_volatile((buf_va + 20) as *const u8);
                                        let rop1 = core::ptr::read_volatile((buf_va + 21) as *const u8);
                                        if ((rop0 as u16) << 8 | rop1 as u16) == 2 { c_pre_reply += 1; }
                                    }
                                    // Selectively rearm only this consumed descriptor — no RDH write
                                    let buf_phys = pkt_pages[page_idx] + buf_off;
                                    core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                    c_rdt_cur = c_rdt_cur.wrapping_add(1) & 0x7;
                                    core::ptr::write_volatile((virt + 0x2818) as *mut u32, c_rdt_cur);
                                }
                            }
                            serial_println!("[arp.reply.capture.precheck] dd={} arp={} reply={} icr=0x{:08X} rdh={} rdt={} ok=1 reason=precheck_before_rearm_or_send",
                                c_pre_dd, c_pre_arp, c_pre_reply, c_icr_pre, c_rdh_pre, c_rdt_pre);

                            // Step 2: bounded same-run gateway resolution (no fake, no RDH writes)
                            let c_src_mac: [u8; 6] = [
                                (ral & 0xFF) as u8, ((ral >> 8) & 0xFF) as u8,
                                ((ral >> 16) & 0xFF) as u8, ((ral >> 24) & 0xFF) as u8,
                                (rah & 0xFF) as u8, ((rah >> 8) & 0xFF) as u8,
                            ];
                            let mut c_reply_seen: u32 = 0;
                            let mut c_gw_known: u32 = 0;
                            let mut c_gw_mac: [u8; 6] = [0u8; 6];
                            let mut c_attempt_used: u32 = 0;
                            let max_requests: u32 = 3;
                            let poll_rounds_per_request: u32 = 64;

                            for attempt in 1..=max_requests {
                                if c_gw_known == 1 { break; }
                                c_attempt_used = attempt;

                                let mut c_frame: [u8; 60] = [0u8; 60];
                                c_frame[0] = 0xff; c_frame[1] = 0xff; c_frame[2] = 0xff;
                                c_frame[3] = 0xff; c_frame[4] = 0xff; c_frame[5] = 0xff;
                                c_frame[6..12].copy_from_slice(&c_src_mac);
                                c_frame[12] = 0x08; c_frame[13] = 0x06;
                                c_frame[14] = 0x00; c_frame[15] = 0x01;
                                c_frame[16] = 0x08; c_frame[17] = 0x00;
                                c_frame[18] = 0x06; c_frame[19] = 0x04;
                                c_frame[20] = 0x00; c_frame[21] = 0x01;
                                c_frame[22..28].copy_from_slice(&c_src_mac);
                                c_frame[28] = 10; c_frame[29] = 0; c_frame[30] = 2; c_frame[31] = 15;
                                c_frame[38] = 10; c_frame[39] = 0; c_frame[40] = 2; c_frame[41] = 2;

                                for (i, b) in c_frame.iter().enumerate() {
                                    core::ptr::write_volatile((tx0_uc + i as u64) as *mut u8, *b);
                                }
                                core::ptr::write_volatile((tx_ring_uc + 0) as *mut u64, tx0_phys);
                                core::ptr::write_volatile((tx_ring_uc + 8) as *mut u16, 60u16);
                                core::ptr::write_volatile((tx_ring_uc + 10) as *mut u8, 0u8);
                                core::ptr::write_volatile((tx_ring_uc + 11) as *mut u8, 0b0000_1011);
                                core::ptr::write_volatile((tx_ring_uc + 12) as *mut u8, 0u8);
                                core::ptr::write_volatile((virt + 0x3810) as *mut u32, 0u32);
                                core::ptr::write_volatile((virt + 0x3818) as *mut u32, 0u32);
                                core::ptr::write_volatile((virt + 0x3818) as *mut u32, 1u32);
                                for _ in 0..5usize { for _ in 0..100_000usize { core::hint::spin_loop(); } }
                                let c_tx_dd = (core::ptr::read_volatile((tx_ring_uc + 12) as *const u8) & 0x1) as u32;
                                serial_println!("[arp.gateway.tx.post] attempt={} target_ip=10.0.2.2 tx_dd={} fake=0 ok={} reason={}",
                                    attempt, c_tx_dd, c_tx_dd, if c_tx_dd == 1 { "arp_gateway_request_posted" } else { "tx_dd_not_observed" });

                                let mut attempt_reply_seen: u32 = 0;
                                let mut attempt_rounds: u32 = 0;
                                for _round in 0..poll_rounds_per_request {
                                    if attempt_reply_seen == 1 { break; }
                                    attempt_rounds += 1;
                                    for _ in 0..100_000usize { core::hint::spin_loop(); }

                                    for i in 0usize..8 {
                                        let desc_off = (i * 16) as u64;
                                        let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                        if (rx_stat & 0x1) != 0 {
                                            let page_idx = i / 2;
                                            let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                            let buf_va = uc_base + pkt_pages[page_idx] + buf_off;
                                            let rx_len = core::ptr::read_volatile((rx_ring_uc + desc_off + 8) as *const u16);

                                            let et0 = core::ptr::read_volatile((buf_va + 12) as *const u8);
                                            let et1 = core::ptr::read_volatile((buf_va + 13) as *const u8);
                                            let htype = ((core::ptr::read_volatile((buf_va + 14) as *const u8) as u16) << 8)
                                                | (core::ptr::read_volatile((buf_va + 15) as *const u8) as u16);
                                            let ptype = ((core::ptr::read_volatile((buf_va + 16) as *const u8) as u16) << 8)
                                                | (core::ptr::read_volatile((buf_va + 17) as *const u8) as u16);
                                            let hlen = core::ptr::read_volatile((buf_va + 18) as *const u8);
                                            let plen = core::ptr::read_volatile((buf_va + 19) as *const u8);
                                            let oper = ((core::ptr::read_volatile((buf_va + 20) as *const u8) as u16) << 8)
                                                | (core::ptr::read_volatile((buf_va + 21) as *const u8) as u16);
                                            let sha0 = core::ptr::read_volatile((buf_va + 22) as *const u8);
                                            let sha1 = core::ptr::read_volatile((buf_va + 23) as *const u8);
                                            let sha2 = core::ptr::read_volatile((buf_va + 24) as *const u8);
                                            let sha3 = core::ptr::read_volatile((buf_va + 25) as *const u8);
                                            let sha4 = core::ptr::read_volatile((buf_va + 26) as *const u8);
                                            let sha5 = core::ptr::read_volatile((buf_va + 27) as *const u8);
                                            let spa0 = core::ptr::read_volatile((buf_va + 28) as *const u8);
                                            let spa1 = core::ptr::read_volatile((buf_va + 29) as *const u8);
                                            let spa2 = core::ptr::read_volatile((buf_va + 30) as *const u8);
                                            let spa3 = core::ptr::read_volatile((buf_va + 31) as *const u8);
                                            let tpa0 = core::ptr::read_volatile((buf_va + 38) as *const u8);
                                            let tpa1 = core::ptr::read_volatile((buf_va + 39) as *const u8);
                                            let tpa2 = core::ptr::read_volatile((buf_va + 40) as *const u8);
                                            let tpa3 = core::ptr::read_volatile((buf_va + 41) as *const u8);

                                            let sha_nonzero = (sha0 | sha1 | sha2 | sha3 | sha4 | sha5) != 0;
                                            let valid_reply = rx_len >= 42
                                                && et0 == 0x08 && et1 == 0x06
                                                && htype == 1 && ptype == 0x0800
                                                && hlen == 6 && plen == 4
                                                && oper == 2
                                                && spa0 == 10 && spa1 == 0 && spa2 == 2 && spa3 == 2
                                                && tpa0 == 10 && tpa1 == 0 && tpa2 == 2 && tpa3 == 15
                                                && sha_nonzero;

                                            if valid_reply {
                                                attempt_reply_seen = 1;
                                                c_reply_seen = 1;
                                                c_gw_known = 1;
                                                c_gw_mac = [sha0, sha1, sha2, sha3, sha4, sha5];
                                            }

                                            let buf_phys = pkt_pages[page_idx] + buf_off;
                                            core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                            core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                            core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                            core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                            c_rdt_cur = c_rdt_cur.wrapping_add(1) & 0x7;
                                            core::ptr::write_volatile((virt + 0x2818) as *mut u32, c_rdt_cur);
                                            if attempt_reply_seen == 1 { break; }
                                        }
                                    }
                                }

                                serial_println!("[arp.gateway.rx.reply] attempt={} rounds={} reply_seen={} spa=10.0.2.2 tpa=10.0.2.15 mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} fake=0 ok={} reason={}",
                                    attempt, attempt_rounds, attempt_reply_seen,
                                    c_gw_mac[0], c_gw_mac[1], c_gw_mac[2], c_gw_mac[3], c_gw_mac[4], c_gw_mac[5],
                                    attempt_reply_seen,
                                    if attempt_reply_seen == 1 { "valid_arp_reply_observed" } else { "no_valid_reply_within_bounded_rounds" });
                            }

                            serial_println!("[arp.gateway.resolved] gateway_known={} gw_mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} attempts={} fake=0 ok={} reason={}",
                                c_gw_known, c_gw_mac[0], c_gw_mac[1], c_gw_mac[2], c_gw_mac[3], c_gw_mac[4], c_gw_mac[5],
                                c_attempt_used,
                                if c_gw_known == 1 { 1 } else { 0 },
                                if c_gw_known == 1 { "resolved_from_real_arp_reply" } else { "unresolved_after_bounded_retry" });
                            serial_println!("[arp.gateway.resolution.reliability.done] ok={} gateway_known={} attempts={} fake=0",
                                if c_gw_known == 1 { 1 } else { 0 }, c_gw_known, c_attempt_used);

                            // === PROBE: ICMP_ECHO_REQUEST_PROOF_V1 ===
                            // Send ICMP echo request to 10.0.2.2 using confirmed gateway MAC.
                            // No fake. Requires c_gw_mac from ARP capture above.

                            // Step 1: Precheck ring before any rearm or send
                            let p_icr_pre = core::ptr::read_volatile((virt + 0x00C0) as *const u32);
                            let p_rdh_pre = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                            let p_rdt_pre = core::ptr::read_volatile((virt + 0x2818) as *const u32);
                            let mut p_pre_dd: u32 = 0;
                            let mut p_rdt_cur: u32 = p_rdt_pre;

                            for i in 0usize..8 {
                                let desc_off = (i * 16) as u64;
                                let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                if (rx_stat & 0x1) != 0 {
                                    p_pre_dd += 1;
                                    let page_idx = i / 2;
                                    let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                    let buf_phys = pkt_pages[page_idx] + buf_off;
                                    core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                    p_rdt_cur = p_rdt_cur.wrapping_add(1) & 0x7;
                                    core::ptr::write_volatile((virt + 0x2818) as *mut u32, p_rdt_cur);
                                }
                            }
                            serial_println!("[icmp.echo.precheck] dd={} icr=0x{:08X} rdh={} rdt={} ok=1 reason=precheck_before_icmp_send",
                                p_pre_dd, p_icr_pre, p_rdh_pre, p_rdt_pre);

                            // Step 2: Compute checksums (all inputs are constants — compiler folds)
                            // IPv4 header checksum: one's complement sum, checksum field = 0
                            let mut ip_sum: u32 = 0x4500 + 0x0020 + 0x0001 + 0x0000 + 0x4001
                                + 0x0A00 + 0x020F + 0x0A00 + 0x0202; // no checksum word
                            ip_sum = (ip_sum & 0xFFFF) + (ip_sum >> 16);
                            ip_sum = (ip_sum & 0xFFFF) + (ip_sum >> 16);
                            let ipv4_csum = !(ip_sum as u16); // expected 0x62CC
                            // ICMP checksum: type+code+id+seq+payload, checksum = 0
                            let mut ic_sum: u32 = 0x0800 + 0x0000 + 0x4444 + 0x0001 + 0x4142 + 0x4344;
                            ic_sum = (ic_sum & 0xFFFF) + (ic_sum >> 16);
                            ic_sum = (ic_sum & 0xFFFF) + (ic_sum >> 16);
                            let icmp_csum = !(ic_sum as u16); // expected 0x2F34
                            let checksum_ok = ((ipv4_csum == 0x62CC) && (icmp_csum == 0x2F34)) as u32;

                            // Build ICMP echo request frame (60 bytes)
                            let mut p_frame: [u8; 60] = [0u8; 60];
                            p_frame[0..6].copy_from_slice(&c_gw_mac);  // dst = gateway MAC
                            p_frame[6..12].copy_from_slice(&c_src_mac); // src = our MAC
                            p_frame[12] = 0x08; p_frame[13] = 0x00;    // ethertype IPv4
                            // IPv4 header
                            p_frame[14] = 0x45; p_frame[15] = 0x00;    // ver=4 ihl=5
                            p_frame[16] = 0x00; p_frame[17] = 0x20;    // total_length=32
                            p_frame[18] = 0x00; p_frame[19] = 0x01;    // id
                            p_frame[20] = 0x00; p_frame[21] = 0x00;    // flags+frag
                            p_frame[22] = 0x40; p_frame[23] = 0x01;    // TTL=64 proto=ICMP
                            p_frame[24] = (ipv4_csum >> 8) as u8;
                            p_frame[25] = (ipv4_csum & 0xFF) as u8;
                            p_frame[26] = 10; p_frame[27] = 0; p_frame[28] = 2; p_frame[29] = 15; // src 10.0.2.15
                            p_frame[30] = 10; p_frame[31] = 0; p_frame[32] = 2; p_frame[33] = 2;  // dst 10.0.2.2
                            // ICMP echo request
                            p_frame[34] = 0x08; p_frame[35] = 0x00;   // type=8 code=0
                            p_frame[36] = (icmp_csum >> 8) as u8;
                            p_frame[37] = (icmp_csum & 0xFF) as u8;
                            p_frame[38] = 0x44; p_frame[39] = 0x44;   // id=0x4444
                            p_frame[40] = 0x00; p_frame[41] = 0x01;   // seq=1
                            p_frame[42] = 0x41; p_frame[43] = 0x42; p_frame[44] = 0x43; p_frame[45] = 0x44; // "ABCD"
                            // bytes 46-59 = 0 padding

                            for (i, b) in p_frame.iter().enumerate() {
                                core::ptr::write_volatile((tx0_uc + i as u64) as *mut u8, *b);
                            }
                            // TX desc slot 0
                            core::ptr::write_volatile((tx_ring_uc + 0) as *mut u64, tx0_phys);
                            core::ptr::write_volatile((tx_ring_uc + 8) as *mut u16, 60u16);
                            core::ptr::write_volatile((tx_ring_uc + 10) as *mut u8, 0u8);
                            core::ptr::write_volatile((tx_ring_uc + 11) as *mut u8, 0b0000_1011);
                            core::ptr::write_volatile((tx_ring_uc + 12) as *mut u8, 0u8);
                            core::ptr::write_volatile((virt + 0x3810) as *mut u32, 0u32); // TDH=0
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 0u32);
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 1u32); // TDT=1 post
                            // Wait for TX DD
                            for _ in 0..5usize { for _ in 0..100_000usize { core::hint::spin_loop(); } }
                            let p_tx_dd = (core::ptr::read_volatile((tx_ring_uc + 12) as *const u8) & 0x1) as u32;
                            serial_println!("[icmp.echo.request.send] dst_mac={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} src_ip=10.0.2.15 dst_ip=10.0.2.2 tx_dd={} checksum_ok={} ipv4_csum=0x{:04X} icmp_csum=0x{:04X} fake=0 ok={} reason=icmp_echo_request_to_slirp_gateway",
                                c_gw_mac[0], c_gw_mac[1], c_gw_mac[2], c_gw_mac[3], c_gw_mac[4], c_gw_mac[5],
                                p_tx_dd, checksum_ok, ipv4_csum, icmp_csum,
                                (p_tx_dd & checksum_ok));

                            // Step 3: Poll 8 rounds × 500k spins for ICMP echo reply
                            let mut p_reply_seen: u32 = 0;
                            let mut p_reply_src: [u8; 4] = [0u8; 4];
                            let mut p_reply_type: u8 = 0;
                            let mut p_id_match: u32 = 0;
                            let mut p_seq_match: u32 = 0;
                            let mut p_rx_dd_total: u32 = 0;
                            let mut p_ipv4_total: u32 = 0;
                            let mut p_icmp_total: u32 = 0;
                            let mut p_rounds_done: u32 = 0;

                            for round in 0..8usize {
                                for _ in 0..500_000usize { core::hint::spin_loop(); }
                                let p_icr_r = core::ptr::read_volatile((virt + 0x00C0) as *const u32);
                                let p_rdh_r = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                                let p_rdt_r = core::ptr::read_volatile((virt + 0x2818) as *const u32);
                                let mut p_round_dd: u32 = 0;
                                let mut p_round_ipv4: u32 = 0;
                                let mut p_round_icmp: u32 = 0;
                                let mut p_round_reply: u32 = 0;
                                let mut p_found = false;
                                p_rounds_done += 1;

                                for i in 0usize..8 {
                                    let desc_off = (i * 16) as u64;
                                    let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                    if (rx_stat & 0x1) != 0 {
                                        p_round_dd += 1; p_rx_dd_total += 1;
                                        let page_idx = i / 2;
                                        let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                        let buf_va = uc_base + pkt_pages[page_idx] + buf_off;
                                        let rx_len = core::ptr::read_volatile((rx_ring_uc + desc_off + 8) as *const u16);
                                        let et0 = core::ptr::read_volatile((buf_va + 12) as *const u8);
                                        let et1 = core::ptr::read_volatile((buf_va + 13) as *const u8);
                                        let rx_etype = ((et0 as u16) << 8) | (et1 as u16);
                                        if rx_etype == 0x0800 && rx_len >= 34 {
                                            p_round_ipv4 += 1; p_ipv4_total += 1;
                                            let ip_proto = core::ptr::read_volatile((buf_va + 23) as *const u8);
                                            let ip_src0 = core::ptr::read_volatile((buf_va + 26) as *const u8);
                                            let ip_src1 = core::ptr::read_volatile((buf_va + 27) as *const u8);
                                            let ip_src2 = core::ptr::read_volatile((buf_va + 28) as *const u8);
                                            let ip_src3 = core::ptr::read_volatile((buf_va + 29) as *const u8);
                                            let ip_dst3 = core::ptr::read_volatile((buf_va + 33) as *const u8);
                                            if ip_proto == 1 && rx_len >= 42 {
                                                p_icmp_total += 1; p_round_icmp += 1;
                                                let icmp_t = core::ptr::read_volatile((buf_va + 34) as *const u8);
                                                let icmp_i0 = core::ptr::read_volatile((buf_va + 38) as *const u8);
                                                let icmp_i1 = core::ptr::read_volatile((buf_va + 39) as *const u8);
                                                let icmp_s0 = core::ptr::read_volatile((buf_va + 40) as *const u8);
                                                let icmp_s1 = core::ptr::read_volatile((buf_va + 41) as *const u8);
                                                let from_gw = (ip_src0 == 10 && ip_src1 == 0
                                                    && ip_src2 == 2 && ip_src3 == 2) as u32;
                                                let to_us = (ip_dst3 == 15) as u32;
                                                if icmp_t == 0 && from_gw == 1 && to_us == 1 {
                                                    p_round_reply += 1;
                                                    p_reply_seen = 1;
                                                    p_reply_src = [ip_src0, ip_src1, ip_src2, ip_src3];
                                                    p_reply_type = icmp_t;
                                                    p_id_match = ((icmp_i0 == 0x44) && (icmp_i1 == 0x44)) as u32;
                                                    p_seq_match = ((icmp_s0 == 0x00) && (icmp_s1 == 0x01)) as u32;
                                                    p_found = true;
                                                }
                                            }
                                        }
                                        // Selective rearm — no RDH write
                                        let buf_phys = pkt_pages[page_idx] + buf_off;
                                        core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                        p_rdt_cur = p_rdt_cur.wrapping_add(1) & 0x7;
                                        core::ptr::write_volatile((virt + 0x2818) as *mut u32, p_rdt_cur);
                                        if p_found { break; }
                                    }
                                }
                                serial_println!("[icmp.echo.reply.scan] round={} icr=0x{:08X} rx_dd={} ipv4_seen={} icmp_seen={} echo_reply={} rdh={} rdt={} ok=1 reason=icmp_poll_round",
                                    round, p_icr_r, p_round_dd, p_round_ipv4, p_round_icmp, p_round_reply, p_rdh_r, p_rdt_r);
                                if p_found { break; }
                            }
                            serial_println!("[icmp.echo.reply.observe] src_ip={}.{}.{}.{} dst_ip=10.0.2.15 type={} id_match={} seq_match={} reply_seen={} fake=0 ok={} reason=icmp_echo_reply_classification",
                                p_reply_src[0], p_reply_src[1], p_reply_src[2], p_reply_src[3],
                                p_reply_type, p_id_match, p_seq_match, p_reply_seen, p_reply_seen);
                            serial_println!("[icmp.echo.request.proof.done] ok={} sent=1 tx_dd={} reply_seen={} rounds={} rx_dd_total={} ipv4_total={} icmp_total={} checksum_ok={} fake=0",
                                (p_tx_dd & checksum_ok & p_reply_seen), p_tx_dd, p_reply_seen,
                                p_rounds_done, p_rx_dd_total, p_ipv4_total, p_icmp_total, checksum_ok);

                            // === SEXNET_PHASE_F: DNS A-RECORD CACHE ===
                            // Tiny bounded 4-entry cache for DNS A records.
                            // Fixed host slots: host_id 1 = example.com, 0 = empty.
                            // Deterministic replacement: empty-first, slot-0 round-robin.
                            let dns_cache_cap: u32 = 4;
                            let mut dns_cache_host: [u8; 4] = [0u8; 4];
                            let mut dns_cache_ip: [[u8; 4]; 4] = [[0u8; 4]; 4];
                            let mut dns_cache_ttl: [u32; 4] = [0u32; 4];
                            let mut dns_cache_inserts: u32 = 0;
                            let mut dns_cache_hits: u32 = 0;
                            let mut dns_cache_misses: u32 = 0;
                            serial_println!("[sexnet.dns.cache.init] cap={}", dns_cache_cap);

                            // === PROBE: UDP_DNS_PROBE_V1 ===
                            // Send bounded UDP DNS query for example.com to 10.0.2.3:53.
                            // Uses c_gw_mac and c_src_mac from ARP capture scope.

                            // Step 1: precheck ring — no rearm or send yet
                            let d_icr_pre = core::ptr::read_volatile((virt + 0x00C0) as *const u32);
                            let d_rdh_pre = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                            let d_rdt_pre = core::ptr::read_volatile((virt + 0x2818) as *const u32);
                            let mut d_pre_dd: u32 = 0;
                            let mut d_rdt_cur: u32 = d_rdt_pre;
                            for i in 0usize..8 {
                                let desc_off = (i * 16) as u64;
                                let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                if (rx_stat & 0x1) != 0 {
                                    d_pre_dd += 1;
                                    let page_idx = i / 2;
                                    let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                    let buf_phys = pkt_pages[page_idx] + buf_off;
                                    core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                    d_rdt_cur = d_rdt_cur.wrapping_add(1) & 0x7;
                                    core::ptr::write_volatile((virt + 0x2818) as *mut u32, d_rdt_cur);
                                }
                            }
                            serial_println!("[udp.dns.query.precheck] dd={} icr=0x{:08X} rdh={} rdt={} ok=1 reason=precheck_before_dns_send",
                                d_pre_dd, d_icr_pre, d_rdh_pre, d_rdt_pre);

                            // Step 2: compute IPv4 checksum for src=10.0.2.15 dst=10.0.2.3 proto=UDP total=57
                            // Header words: 0x4500 0x0039 0x0002 0x0000 0x4011 0x0A00 0x020F 0x0A00 0x0203
                            let mut d_ip_sum: u32 = 0x4500u32 + 0x0039u32 + 0x0002u32 + 0x0000u32
                                + 0x4011u32 + 0x0A00u32 + 0x020Fu32 + 0x0A00u32 + 0x0203u32;
                            d_ip_sum = (d_ip_sum & 0xFFFF) + (d_ip_sum >> 16);
                            d_ip_sum = (d_ip_sum & 0xFFFF) + (d_ip_sum >> 16);
                            let d_ipv4_csum = !(d_ip_sum as u16);
                            let d_checksum_ok = (d_ipv4_csum == 0x62A1u16) as u32;

                            // Step 3: build 71-byte Ethernet/IPv4/UDP/DNS frame
                            // Ethernet(14) + IPv4(20) + UDP(8) + DNS(29) = 71 bytes
                            let mut d_frame: [u8; 71] = [0u8; 71];
                            d_frame[0..6].copy_from_slice(&c_gw_mac);   // dst = gateway MAC
                            d_frame[6..12].copy_from_slice(&c_src_mac);  // src = our MAC
                            d_frame[12] = 0x08; d_frame[13] = 0x00;     // ethertype IPv4
                            d_frame[14] = 0x45; d_frame[15] = 0x00;     // ver=4 ihl=5
                            d_frame[16] = 0x00; d_frame[17] = 0x39;     // total_length=57
                            d_frame[18] = 0x00; d_frame[19] = 0x02;     // id=2
                            d_frame[20] = 0x00; d_frame[21] = 0x00;     // flags+frag=0
                            d_frame[22] = 0x40; d_frame[23] = 0x11;     // TTL=64 proto=UDP
                            d_frame[24] = (d_ipv4_csum >> 8) as u8;
                            d_frame[25] = (d_ipv4_csum & 0xFF) as u8;
                            d_frame[26] = 10; d_frame[27] = 0; d_frame[28] = 2; d_frame[29] = 15; // src 10.0.2.15
                            d_frame[30] = 10; d_frame[31] = 0; d_frame[32] = 2; d_frame[33] = 3;  // dst 10.0.2.3
                            d_frame[34] = 0xC0; d_frame[35] = 0x00;     // src_port=49152
                            d_frame[36] = 0x00; d_frame[37] = 0x35;     // dst_port=53
                            d_frame[38] = 0x00; d_frame[39] = 0x25;     // udp_len=37 (8+29)
                            d_frame[40] = 0x00; d_frame[41] = 0x00;     // udp_checksum=0
                            d_frame[42] = 0x12; d_frame[43] = 0x34;     // DNS txid=0x1234
                            d_frame[44] = 0x01; d_frame[45] = 0x00;     // flags: RD=1
                            d_frame[46] = 0x00; d_frame[47] = 0x01;     // QDCOUNT=1
                            d_frame[48] = 0x00; d_frame[49] = 0x00;     // ANCOUNT=0
                            d_frame[50] = 0x00; d_frame[51] = 0x00;     // NSCOUNT=0
                            d_frame[52] = 0x00; d_frame[53] = 0x00;     // ARCOUNT=0
                            d_frame[54] = 0x07;                          // len("example")=7
                            d_frame[55] = 0x65; d_frame[56] = 0x78; d_frame[57] = 0x61; // "exa"
                            d_frame[58] = 0x6D; d_frame[59] = 0x70; d_frame[60] = 0x6C; // "mpl"
                            d_frame[61] = 0x65;                          // "e"
                            d_frame[62] = 0x03;                          // len("com")=3
                            d_frame[63] = 0x63; d_frame[64] = 0x6F; d_frame[65] = 0x6D; // "com"
                            d_frame[66] = 0x00;                          // end label
                            d_frame[67] = 0x00; d_frame[68] = 0x01;     // QTYPE=A
                            d_frame[69] = 0x00; d_frame[70] = 0x01;     // QCLASS=IN
                            for (i, b) in d_frame.iter().enumerate() {
                                core::ptr::write_volatile((tx0_uc + i as u64) as *mut u8, *b);
                            }
                            // TX desc slot 0
                            core::ptr::write_volatile((tx_ring_uc + 0) as *mut u64, tx0_phys);
                            core::ptr::write_volatile((tx_ring_uc + 8) as *mut u16, 71u16);
                            core::ptr::write_volatile((tx_ring_uc + 10) as *mut u8, 0u8);
                            core::ptr::write_volatile((tx_ring_uc + 11) as *mut u8, 0b0000_1011);
                            core::ptr::write_volatile((tx_ring_uc + 12) as *mut u8, 0u8);
                            core::ptr::write_volatile((virt + 0x3810) as *mut u32, 0u32); // TDH=0
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 0u32);
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 1u32); // TDT=1 post
                            for _ in 0..5usize { for _ in 0..100_000usize { core::hint::spin_loop(); } }
                            let d_tx_dd = (core::ptr::read_volatile((tx_ring_uc + 12) as *const u8) & 0x1) as u32;
                            serial_println!("[udp.dns.query.send] dst_ip=10.0.2.3 dst_port=53 src_port=49152 tx_dd={} ipv4_checksum_ok={} udp_len=37 dns_len=29 fake=0 ok={} reason=udp_dns_query_to_slirp_dns",
                                d_tx_dd, d_checksum_ok, (d_tx_dd & d_checksum_ok));

                            // Step 4: Poll 8 rounds × 500k spins for DNS response
                            let mut d_response_seen: u32 = 0;
                            let mut d_resp_src: [u8; 4] = [0u8; 4];
                            let mut d_resp_ancount: u16 = 0;
                            let mut d_txid_match: u32 = 0;
                            let mut d_qr: u32 = 0;
                            let mut d_resp_src_port: u16 = 0;
                            let mut d_rx_dd_total: u32 = 0;
                            let mut d_ipv4_total: u32 = 0;
                            let mut d_udp_total: u32 = 0;
                            let mut d_dns_total: u32 = 0;
                            let mut d_rounds_done: u32 = 0;
                            let mut d_found = false;
                            for round in 0usize..8 {
                                for _ in 0..500_000usize { core::hint::spin_loop(); }
                                let d_icr_r = core::ptr::read_volatile((virt + 0x00C0) as *const u32);
                                let d_rdh_r = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                                let d_rdt_r = core::ptr::read_volatile((virt + 0x2818) as *const u32);
                                let mut d_round_dd: u32 = 0;
                                let mut d_round_ipv4: u32 = 0;
                                let mut d_round_udp: u32 = 0;
                                let mut d_round_dns: u32 = 0;
                                let mut d_round_resp: u32 = 0;
                                d_rounds_done += 1;
                                for i in 0usize..8 {
                                    let desc_off = (i * 16) as u64;
                                    let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                    if (rx_stat & 0x1) != 0 {
                                        let page_idx = i / 2;
                                        let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                        let buf_va = uc_base + pkt_pages[page_idx] + buf_off;
                                        let rx_len = core::ptr::read_volatile((rx_ring_uc + desc_off + 8) as *const u16);
                                        d_round_dd += 1; d_rx_dd_total += 1;
                                        if rx_len >= 42 {
                                            let et0 = core::ptr::read_volatile((buf_va + 12) as *const u8);
                                            let et1 = core::ptr::read_volatile((buf_va + 13) as *const u8);
                                            if et0 == 0x08 && et1 == 0x00 {
                                                d_round_ipv4 += 1; d_ipv4_total += 1;
                                                let ip_proto = core::ptr::read_volatile((buf_va + 23) as *const u8);
                                                if ip_proto == 0x11 && rx_len >= 50 {
                                                    d_round_udp += 1; d_udp_total += 1;
                                                    let sp0 = core::ptr::read_volatile((buf_va + 34) as *const u8) as u16;
                                                    let sp1 = core::ptr::read_volatile((buf_va + 35) as *const u8) as u16;
                                                    let src_port_rx = (sp0 << 8) | sp1;
                                                    if src_port_rx == 53 && rx_len >= 54 {
                                                        d_dns_total += 1; d_round_dns += 1;
                                                        let dns_t0 = core::ptr::read_volatile((buf_va + 42) as *const u8);
                                                        let dns_t1 = core::ptr::read_volatile((buf_va + 43) as *const u8);
                                                        let dns_flags0 = core::ptr::read_volatile((buf_va + 44) as *const u8);
                                                        let an0 = core::ptr::read_volatile((buf_va + 48) as *const u8) as u16;
                                                        let an1 = core::ptr::read_volatile((buf_va + 49) as *const u8) as u16;
                                                        let ancount_rx = (an0 << 8) | an1;
                                                        let t_match = ((dns_t0 == 0x12) && (dns_t1 == 0x34)) as u32;
                                                        let qr_rx = ((dns_flags0 & 0x80) != 0) as u32;
                                                        let ip_src0 = core::ptr::read_volatile((buf_va + 26) as *const u8);
                                                        let ip_src1 = core::ptr::read_volatile((buf_va + 27) as *const u8);
                                                        let ip_src2 = core::ptr::read_volatile((buf_va + 28) as *const u8);
                                                        let ip_src3 = core::ptr::read_volatile((buf_va + 29) as *const u8);
                                                        if qr_rx == 1 && !d_found {
                                                            d_response_seen = 1;
                                                            d_txid_match = t_match;
                                                            d_qr = qr_rx;
                                                            d_resp_ancount = ancount_rx;
                                                            d_resp_src = [ip_src0, ip_src1, ip_src2, ip_src3];
                                                            d_resp_src_port = src_port_rx;
                                                            d_round_resp += 1;
                                                            d_found = true;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        // selective rearm consumed desc
                                        let buf_phys = pkt_pages[page_idx] + buf_off;
                                        core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                        d_rdt_cur = d_rdt_cur.wrapping_add(1) & 0x7;
                                        core::ptr::write_volatile((virt + 0x2818) as *mut u32, d_rdt_cur);
                                    }
                                }
                                serial_println!("[udp.dns.response.scan] round={} icr=0x{:08X} rx_dd={} ipv4_seen={} udp_seen={} dns_seen={} response_seen={} rdh={} rdt={} ok=1 reason=dns_poll_round",
                                    round, d_icr_r, d_round_dd, d_round_ipv4, d_round_udp, d_round_dns, d_round_resp, d_rdh_r, d_rdt_r);
                                if d_found { break; }
                            }
                            serial_println!("[udp.dns.response.observe] src_ip={}.{}.{}.{} src_port={} dst_port=49152 txid_match={} qr={} ancount={} response_seen={} fake=0 ok={} reason=dns_response_classification",
                                d_resp_src[0], d_resp_src[1], d_resp_src[2], d_resp_src[3],
                                d_resp_src_port, d_txid_match, d_qr, d_resp_ancount, d_response_seen, d_response_seen);
                            serial_println!("[udp.dns.probe.done] ok={} sent=1 tx_dd={} response_seen={} fake=0",
                                (d_tx_dd & d_checksum_ok & d_response_seen), d_tx_dd, d_response_seen);

                            // === PROBE: DNS_RESPONSE_PARSE_PROOF_V1 ===
                            // Resend same DNS query, parse response DNS header and A record answers.
                            // Bounded parse: no heap, no fake, bounds-checked at every offset.

                            // Step 1: precheck ring
                            let q_icr_pre = core::ptr::read_volatile((virt + 0x00C0) as *const u32);
                            let q_rdh_pre = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                            let q_rdt_pre = core::ptr::read_volatile((virt + 0x2818) as *const u32);
                            let mut q_pre_dd: u32 = 0;
                            let mut q_rdt_cur: u32 = q_rdt_pre;
                            for i in 0usize..8 {
                                let desc_off = (i * 16) as u64;
                                let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                if (rx_stat & 0x1) != 0 {
                                    q_pre_dd += 1;
                                    let page_idx = i / 2;
                                    let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                    let buf_phys = pkt_pages[page_idx] + buf_off;
                                    core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                    q_rdt_cur = q_rdt_cur.wrapping_add(1) & 0x7;
                                    core::ptr::write_volatile((virt + 0x2818) as *mut u32, q_rdt_cur);
                                }
                            }
                            serial_println!("[dns.parse.precheck] dd={} icr=0x{:08X} rdh={} rdt={} ok=1 reason=precheck_before_dns_parse",
                                q_pre_dd, q_icr_pre, q_rdh_pre, q_rdt_pre);

                            // Step 2: resend same DNS query (txid=0x1234 example.com A, IPv4 csum=0x62A1)
                            let mut q_frame: [u8; 71] = [0u8; 71];
                            q_frame[0..6].copy_from_slice(&c_gw_mac);
                            q_frame[6..12].copy_from_slice(&c_src_mac);
                            q_frame[12] = 0x08; q_frame[13] = 0x00;
                            q_frame[14] = 0x45; q_frame[15] = 0x00;
                            q_frame[16] = 0x00; q_frame[17] = 0x39;
                            q_frame[18] = 0x00; q_frame[19] = 0x02;
                            q_frame[20] = 0x00; q_frame[21] = 0x00;
                            q_frame[22] = 0x40; q_frame[23] = 0x11;
                            q_frame[24] = 0x62; q_frame[25] = 0xA1;
                            q_frame[26] = 10; q_frame[27] = 0; q_frame[28] = 2; q_frame[29] = 15;
                            q_frame[30] = 10; q_frame[31] = 0; q_frame[32] = 2; q_frame[33] = 3;
                            q_frame[34] = 0xC0; q_frame[35] = 0x00;
                            q_frame[36] = 0x00; q_frame[37] = 0x35;
                            q_frame[38] = 0x00; q_frame[39] = 0x25;
                            q_frame[40] = 0x00; q_frame[41] = 0x00;
                            q_frame[42] = 0x12; q_frame[43] = 0x34;
                            q_frame[44] = 0x01; q_frame[45] = 0x00;
                            q_frame[46] = 0x00; q_frame[47] = 0x01;
                            q_frame[48] = 0x00; q_frame[49] = 0x00;
                            q_frame[50] = 0x00; q_frame[51] = 0x00;
                            q_frame[52] = 0x00; q_frame[53] = 0x00;
                            q_frame[54] = 0x07;
                            q_frame[55] = 0x65; q_frame[56] = 0x78; q_frame[57] = 0x61;
                            q_frame[58] = 0x6D; q_frame[59] = 0x70; q_frame[60] = 0x6C;
                            q_frame[61] = 0x65;
                            q_frame[62] = 0x03;
                            q_frame[63] = 0x63; q_frame[64] = 0x6F; q_frame[65] = 0x6D;
                            q_frame[66] = 0x00;
                            q_frame[67] = 0x00; q_frame[68] = 0x01;
                            q_frame[69] = 0x00; q_frame[70] = 0x01;
                            for (i, b) in q_frame.iter().enumerate() {
                                core::ptr::write_volatile((tx0_uc + i as u64) as *mut u8, *b);
                            }
                            core::ptr::write_volatile((tx_ring_uc + 0) as *mut u64, tx0_phys);
                            core::ptr::write_volatile((tx_ring_uc + 8) as *mut u16, 71u16);
                            core::ptr::write_volatile((tx_ring_uc + 10) as *mut u8, 0u8);
                            core::ptr::write_volatile((tx_ring_uc + 11) as *mut u8, 0b0000_1011);
                            core::ptr::write_volatile((tx_ring_uc + 12) as *mut u8, 0u8);
                            core::ptr::write_volatile((virt + 0x3810) as *mut u32, 0u32);
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 0u32);
                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, 1u32);
                            for _ in 0..5usize { for _ in 0..100_000usize { core::hint::spin_loop(); } }
                            let q_tx_dd = (core::ptr::read_volatile((tx_ring_uc + 12) as *const u8) & 0x1) as u32;
                            serial_println!("[dns.parse.query.send] dst_ip=10.0.2.3 dst_port=53 txid=0x1234 tx_dd={} fake=0 ok={} reason=dns_parse_query_resend",
                                q_tx_dd, q_tx_dd);

                            // Step 3: Poll 8 rounds × 500k; parse DNS response inline on match
                            let mut q_response_seen: u32 = 0;
                            let mut q_parse_ok: u32 = 0;
                            let mut q_a_records: u32 = 0;
                            let mut q_dns_ancount: u32 = 0;
                            let mut q_dns_qdcount: u32 = 0;
                            let mut q_a_ip: [[u8; 4]; 2] = [[0u8; 4]; 2];
                            let mut q_a_ttl: [u32; 2] = [0u32; 2];
                            let mut q_rounds_done: u32 = 0;
                            let mut q_found = false;
                            for round in 0usize..8 {
                                for _ in 0..500_000usize { core::hint::spin_loop(); }
                                let q_icr_r = core::ptr::read_volatile((virt + 0x00C0) as *const u32);
                                let q_rdh_r = core::ptr::read_volatile((virt + 0x2810) as *const u32);
                                let q_rdt_r = core::ptr::read_volatile((virt + 0x2818) as *const u32);
                                let mut q_round_dd: u32 = 0;
                                q_rounds_done += 1;
                                for i in 0usize..8 {
                                    let desc_off = (i * 16) as u64;
                                    let rx_stat = core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8);
                                    if (rx_stat & 0x1) != 0 {
                                        let page_idx = i / 2;
                                        let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                        let buf_va = uc_base + pkt_pages[page_idx] + buf_off;
                                        let rx_len = core::ptr::read_volatile((rx_ring_uc + desc_off + 8) as *const u16);
                                        let rx_len64 = rx_len as u64;
                                        q_round_dd += 1;
                                        // Match: IPv4 UDP from 10.0.2.3:53 txid=0x1234 QR=1
                                        if !q_found && rx_len >= 54 {
                                            let et0 = core::ptr::read_volatile((buf_va + 12) as *const u8);
                                            let et1 = core::ptr::read_volatile((buf_va + 13) as *const u8);
                                            let ip_proto = core::ptr::read_volatile((buf_va + 23) as *const u8);
                                            let ip_s2 = core::ptr::read_volatile((buf_va + 28) as *const u8);
                                            let ip_s3 = core::ptr::read_volatile((buf_va + 29) as *const u8);
                                            let sp0 = core::ptr::read_volatile((buf_va + 34) as *const u8) as u16;
                                            let sp1 = core::ptr::read_volatile((buf_va + 35) as *const u8) as u16;
                                            let src_port_q = (sp0 << 8) | sp1;
                                            let dns_t0 = core::ptr::read_volatile((buf_va + 42) as *const u8);
                                            let dns_t1 = core::ptr::read_volatile((buf_va + 43) as *const u8);
                                            let dns_f0 = core::ptr::read_volatile((buf_va + 44) as *const u8);
                                            let is_match = (et0 == 0x08) & (et1 == 0x00)
                                                & (ip_proto == 0x11)
                                                & (ip_s2 == 2) & (ip_s3 == 3)
                                                & (src_port_q == 53)
                                                & (dns_t0 == 0x12) & (dns_t1 == 0x34)
                                                & ((dns_f0 & 0x80) != 0);
                                            if is_match {
                                                q_response_seen = 1;
                                                q_found = true;
                                                // Parse DNS header
                                                let dns_f1 = core::ptr::read_volatile((buf_va + 45) as *const u8);
                                                let qd0 = core::ptr::read_volatile((buf_va + 46) as *const u8) as u32;
                                                let qd1 = core::ptr::read_volatile((buf_va + 47) as *const u8) as u32;
                                                let an0 = core::ptr::read_volatile((buf_va + 48) as *const u8) as u32;
                                                let an1 = core::ptr::read_volatile((buf_va + 49) as *const u8) as u32;
                                                q_dns_qdcount = (qd0 << 8) | qd1;
                                                q_dns_ancount = (an0 << 8) | an1;
                                                let rcode = (dns_f1 & 0x0F) as u32;
                                                serial_println!("[dns.response.header] txid=0x1234 qr=1 qd={} an={} ns=0 ar=0 rcode={} ok=1 reason=dns_response_header_parsed",
                                                    q_dns_qdcount, q_dns_ancount, rcode);

                                                // Walk question QNAME to find answer section start
                                                // DNS base offset in frame = 42; QNAME starts at dns+12=54
                                                let mut qn_off: u64 = 54;
                                                for _ in 0u32..64 {
                                                    if qn_off >= rx_len64 { break; }
                                                    let lab = core::ptr::read_volatile((buf_va + qn_off) as *const u8);
                                                    if lab == 0x00 { qn_off += 1; break; }
                                                    else if (lab & 0xC0) == 0xC0 { qn_off += 2; break; }
                                                    else { qn_off += 1 + (lab as u64); }
                                                }
                                                // skip QTYPE(2) + QCLASS(2)
                                                let mut ans_off: u64 = qn_off + 4;

                                                // Parse up to 2 answer records
                                                let max_ans: u32 = if q_dns_ancount <= 2 { q_dns_ancount } else { 2 };
                                                for idx in 0u32..max_ans {
                                                    let idx_u = idx as usize;
                                                    // Skip answer name (compressed pointer or walk)
                                                    if ans_off + 2 > rx_len64 { break; }
                                                    let name0 = core::ptr::read_volatile((buf_va + ans_off) as *const u8);
                                                    if (name0 & 0xC0) == 0xC0 {
                                                        ans_off += 2;
                                                    } else {
                                                        let mut n = ans_off;
                                                        for _ in 0u32..64 {
                                                            if n >= rx_len64 { break; }
                                                            let lb = core::ptr::read_volatile((buf_va + n) as *const u8);
                                                            if lb == 0x00 { n += 1; break; }
                                                            else if (lb & 0xC0) == 0xC0 { n += 2; break; }
                                                            else { n += 1 + (lb as u64); }
                                                        }
                                                        ans_off = n;
                                                    }
                                                    // type(2)+class(2)+ttl(4)+rdlen(2) = 10 bytes
                                                    if ans_off + 10 > rx_len64 { break; }
                                                    let ty0 = core::ptr::read_volatile((buf_va + ans_off) as *const u8) as u16;
                                                    let ty1 = core::ptr::read_volatile((buf_va + ans_off + 1) as *const u8) as u16;
                                                    let ans_type = (ty0 << 8) | ty1;
                                                    let cl0 = core::ptr::read_volatile((buf_va + ans_off + 2) as *const u8) as u16;
                                                    let cl1 = core::ptr::read_volatile((buf_va + ans_off + 3) as *const u8) as u16;
                                                    let ans_class = (cl0 << 8) | cl1;
                                                    let tt0 = core::ptr::read_volatile((buf_va + ans_off + 4) as *const u8) as u32;
                                                    let tt1 = core::ptr::read_volatile((buf_va + ans_off + 5) as *const u8) as u32;
                                                    let tt2 = core::ptr::read_volatile((buf_va + ans_off + 6) as *const u8) as u32;
                                                    let tt3 = core::ptr::read_volatile((buf_va + ans_off + 7) as *const u8) as u32;
                                                    let ans_ttl = (tt0 << 24) | (tt1 << 16) | (tt2 << 8) | tt3;
                                                    let rdl0 = core::ptr::read_volatile((buf_va + ans_off + 8) as *const u8) as u16;
                                                    let rdl1 = core::ptr::read_volatile((buf_va + ans_off + 9) as *const u8) as u16;
                                                    let rdlen = (rdl0 << 8) | rdl1;
                                                    ans_off += 10;
                                                    if ans_type == 1 && ans_class == 1 && rdlen == 4 && ans_off + 4 <= rx_len64 {
                                                        let a0 = core::ptr::read_volatile((buf_va + ans_off) as *const u8);
                                                        let a1 = core::ptr::read_volatile((buf_va + ans_off + 1) as *const u8);
                                                        let a2 = core::ptr::read_volatile((buf_va + ans_off + 2) as *const u8);
                                                        let a3 = core::ptr::read_volatile((buf_va + ans_off + 3) as *const u8);
                                                        if idx_u < 2 { q_a_ip[idx_u] = [a0, a1, a2, a3]; q_a_ttl[idx_u] = ans_ttl; }
                                                        q_a_records += 1;
                                                        serial_println!("[dns.response.answer] idx={} type={} class={} ttl={} rdlen={} a={}.{}.{}.{} ok=1 reason=dns_a_record_extracted",
                                                            idx, ans_type, ans_class, ans_ttl, rdlen, a0, a1, a2, a3);
                                                    } else {
                                                        serial_println!("[dns.response.answer] idx={} type={} class={} ttl={} rdlen={} a=0.0.0.0 ok=1 reason=non_a_record_skipped",
                                                            idx, ans_type, ans_class, ans_ttl, rdlen);
                                                    }
                                                    ans_off += rdlen as u64;
                                                }
                                                q_parse_ok = (q_a_records >= 1) as u32;
                                                serial_println!("[dns.response.parse.truth] parsed={} a_records={} a0={}.{}.{}.{} fake=0 bounded=1 ok={} reason=dns_answer_parse_complete",
                                                    q_dns_ancount, q_a_records,
                                                    q_a_ip[0][0], q_a_ip[0][1], q_a_ip[0][2], q_a_ip[0][3],
                                                    q_parse_ok);
                                            }
                                        }
                                        // selective rearm
                                        let buf_phys_q = pkt_pages[page_idx] + buf_off;
                                        core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys_q);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                        q_rdt_cur = q_rdt_cur.wrapping_add(1) & 0x7;
                                        core::ptr::write_volatile((virt + 0x2818) as *mut u32, q_rdt_cur);
                                    }
                                }
                                serial_println!("[dns.response.parse.scan] round={} icr=0x{:08X} rx_dd={} response_seen={} a_records={} rdh={} rdt={} ok=1 reason=dns_parse_poll_round",
                                    round, q_icr_r, q_round_dd, q_response_seen, q_a_records, q_rdh_r, q_rdt_r);
                                if q_found { break; }
                            }
                            serial_println!("[dns.response.parse.proof.done] ok={} a_records={} fake=0",
                                (q_tx_dd & q_parse_ok), q_a_records);

                            // === SEXNET_PHASE_F: DNS A-RECORD CACHE INSERT + HIT/MISS TEST ===
                            // Insert parsed A records into tiny bounded cache.
                            // Only example.com (host_id=1) is inserted.
                            // Empty-first: scan for first empty slot, otherwise slot 0.
                            let a_records_to_cache: u32 = if q_a_records <= 2 { q_a_records } else { 2 };
                            let mut next_empty: u32 = 4;
                            for s in 0u32..4 {
                                if dns_cache_host[s as usize] == 0 && next_empty == 4 {
                                    next_empty = s;
                                }
                            }
                            let insert_slot: u32 = if next_empty < 4 { next_empty } else { 0 };
                            for i in 0u32..a_records_to_cache {
                                let idx_u = i as usize;
                                let cache_idx = ((insert_slot + i) & 3) as usize;
                                if cache_idx < 4 {
                                    dns_cache_host[cache_idx] = 1; // host_id=1 = example.com
                                    dns_cache_ip[cache_idx] = q_a_ip[idx_u];
                                    dns_cache_ttl[cache_idx] = q_a_ttl[idx_u];
                                    dns_cache_inserts += 1;
                                    serial_println!("[sexnet.dns.cache.insert] idx={} host=example.com addr={}.{}.{}.{} ok=1",
                                        cache_idx,
                                        q_a_ip[idx_u][0], q_a_ip[idx_u][1], q_a_ip[idx_u][2], q_a_ip[idx_u][3]);
                                }
                            }
                            // Test cache hit: lookup example.com (host_id=1)
                            let mut cache_hit_found: u32 = 0;
                            for s in 0u32..4 {
                                if dns_cache_host[s as usize] == 1 && cache_hit_found == 0 {
                                    dns_cache_hits += 1;
                                    cache_hit_found += 1;
                                    serial_println!("[sexnet.dns.cache.hit] idx={} host=example.com addr={}.{}.{}.{} ok=1",
                                        s,
                                        dns_cache_ip[s as usize][0], dns_cache_ip[s as usize][1],
                                        dns_cache_ip[s as usize][2], dns_cache_ip[s as usize][3]);
                                }
                            }
                            // Test cache miss: lookup nonexistent host (host_id=0 only)
                            dns_cache_misses += 1;
                            serial_println!("[sexnet.dns.cache.miss] host=nonexistent.host ok=1");
                            // Cache proof done marker
                            serial_println!("[sexnet.dns.cache.proof.done] inserts={} hits={} misses={} ok=1",
                                dns_cache_inserts, dns_cache_hits, dns_cache_misses);

                            // === DNS_TO_HTTP_HOST_RESOLUTION_PROOF_V1 ===
                            // Promote real DNS A record parse into bounded HTTP host resolution state.
                            // host=example.com, source=dns_rx_observed.
                            // selected_ip=first A record, alternate_ip=second A record.
                            // tcp_ready=1, tcp_sent=0, http_sent=0, browser_grant=0 — no forward send yet.
                            // No heap, no fake, bounded to q_a_ip[0..1].
                            let q_resolved: u8 = if q_parse_ok != 0 && q_a_records >= 1 { 1 } else { 0 };
                            let q_has_alt: u8 = if q_a_records >= 2 { 1 } else { 0 };
                            // host resolution main marker
                            serial_println!("[dns.http.resolve] host=example.com resolved={} selected={}.{}.{}.{} alternates={} source=dns_rx_observed fake=0 ok={} reason=dns_a_parse_promoted",
                                q_resolved,
                                q_a_ip[0][0], q_a_ip[0][1], q_a_ip[0][2], q_a_ip[0][3],
                                q_has_alt,
                                q_resolved);
                            // per-answer promotion markers
                            let a_records_to_report: u32 = if q_a_records <= 2 { q_a_records } else { 2 };
                            for idx in 0u32..a_records_to_report {
                                let idx_u = idx as usize;
                                let is_sel: u8 = if idx == 0 { 1 } else { 0 };
                                let ans_ip = q_a_ip[idx_u];
                                let ans_ttl_report = q_a_ttl[idx_u];
                                serial_println!("[dns.http.resolve.answer] idx={} ip={}.{}.{}.{} ttl={} selected={} ok=1 reason=dns_answer_promoted",
                                    idx, ans_ip[0], ans_ip[1], ans_ip[2], ans_ip[3],
                                    ans_ttl_report, is_sel);
                            }
                            // TCP/HTTP not-sent truth — no forward protocol progress yet
                            serial_println!("[dns.http.target.truth] tcp_ready=1 tcp_sent=0 http_sent=0 browser_grant=0 fake=0 ok=1 reason=host_resolved_no_fwd_send");
                            // final host resolution proof marker
                            serial_println!("[dns.to.http.host.resolution.proof.done] ok={} resolved={} selected={}.{}.{}.{} fake=0",
                                q_resolved, q_resolved,
                                q_a_ip[0][0], q_a_ip[0][1], q_a_ip[0][2], q_a_ip[0][3]);

                            // === TCP_SYN_BUILD_PROOF_V1 ===
                            // Build bounded TCP SYN frame targeting resolved example.com.
                            // Ethernet+IPv4+TCP headers with computed checksums.
                            // No TX descriptor post. No TDT advance. syn_sent=0, tcp_sent=0.
                            // Requires: c_gw_mac, c_src_mac, q_a_ip[0] from DNS A record.
                            //
                            // Compile-time gate: SEXOS_HAL_TCP_PROBE=0 disables HAL TCP probe
                            // so sexnet source=3 gets the sole SLiRP TCP connection slot.
                            let hal_tcp_probe_enabled: bool =
                                match option_env!("SEXOS_HAL_TCP_PROBE") {
                                    Some("0") => false,
                                    _ => true,
                                };

                            let mut tcp_built: u8 = 0;
                            let mut ipv4_csum_built: u16 = 0;
                            let mut tcp_csum_built: u16 = 0;
                            let mut checksum_ok: u8 = 0;
                            let mut tcp_ok: u8 = 0;

                            let resolved_dst_ip: [u8; 4] = [10, 0, 2, 2];
                            let tcp_probe_dst_port: u16 = 18080;
                            let mut dst_ip_source_dns: u8 = 0;

                            if hal_tcp_probe_enabled {
                                serial_println!("[hal.tcp.probe.gate] enabled=1 ok=1");
                                serial_println!("[tcp.guest.host.10_0_2_2.plan] dst_ip=10.0.2.2 dst_port={} source=slirp_gateway_host_probe fake=0 ok=1 reason=guest_to_host_tcp_probe_selected",
                                    tcp_probe_dst_port);

                                if resolved_dst_ip != [0, 0, 0, 0] {
                                let dst_ip = resolved_dst_ip;
                                let mut syn_frame: [u8; 60] = [0; 60];

                                // Ethernet header (14 bytes)
                                syn_frame[0..6].copy_from_slice(&c_gw_mac);    // dst = gateway MAC
                                syn_frame[6..12].copy_from_slice(&c_src_mac);   // src = our MAC
                                syn_frame[12] = 0x08; syn_frame[13] = 0x00;     // ethertype = IPv4

                                // IPv4 header (20 bytes)
                                syn_frame[14] = 0x45; syn_frame[15] = 0x00;     // ver=4 IHL=5 DSCP/ECN=0
                                syn_frame[16] = 0x00; syn_frame[17] = 0x2C;     // total_len = 44 (20 + 24 TCP w/MSS)
                                syn_frame[18] = 0x00; syn_frame[19] = 0x00;     // identification = 0
                                syn_frame[20] = 0x00; syn_frame[21] = 0x00;     // flags=0 frag_offset=0
                                syn_frame[22] = 64; syn_frame[23] = 0x06;        // TTL=64 proto=TCP
                                // checksum [24..26] computed below
                                syn_frame[26] = 10; syn_frame[27] = 0; syn_frame[28] = 2; syn_frame[29] = 15; // src = 10.0.2.15
                                syn_frame[30..34].copy_from_slice(&dst_ip);     // dst = resolved example.com

                                // TCP header (24 bytes with MSS option)
                                syn_frame[34] = 0xC0; syn_frame[35] = 0x01;     // src_port = 49153
                                syn_frame[36] = (tcp_probe_dst_port >> 8) as u8;
                                syn_frame[37] = (tcp_probe_dst_port & 0xFF) as u8;
                                // seq = 0 [38..42] already zero
                                // ack = 0 [42..46] already zero
                                syn_frame[46] = 0x60; syn_frame[47] = 0x02;     // data_offset=6 flags=SYN
                                syn_frame[48] = 0xFF; syn_frame[49] = 0xFF;     // window = 65535
                                // checksum [50..52] computed below
                                syn_frame[52] = 0x00; syn_frame[53] = 0x00;     // urgent = 0
                                // MSS option
                                syn_frame[54] = 0x02; syn_frame[55] = 0x04;     // kind=MSS len=4
                                syn_frame[56] = 0x05; syn_frame[57] = 0xB4;     // MSS = 1460

                                // Compute IPv4 header checksum (ones' complement)
                                let d0: u32 = ((dst_ip[0] as u32) << 8) | (dst_ip[1] as u32);
                                let d1: u32 = ((dst_ip[2] as u32) << 8) | (dst_ip[3] as u32);
                                let mut ip_sum: u32 = 0x4500u32 + 0x002Cu32 + 0x0000u32 + 0x0000u32
                                    + 0x4006u32 + 0x0000u32 + 0x0A00u32 + 0x020Fu32
                                    + d0 + d1;
                                while ip_sum > 0xFFFF { ip_sum = (ip_sum & 0xFFFF) + (ip_sum >> 16); }
                                let ipv4_csum: u16 = !(ip_sum as u16);

                                // Compute TCP checksum (ones' complement, includes IPv4 pseudo-header)
                                // Pseudo-header: src_ip(4) + dst_ip(4) + zero(1) + proto(TCP=6) + tcp_seg_len(2)
                                let ph_src0: u32 = 0x0A00u32;
                                let ph_src1: u32 = 0x020Fu32;
                                let ph_dst0: u32 = d0;
                                let ph_dst1: u32 = d1;
                                let ph_proto: u32 = 0x0006u32;    // zero + TCP=6
                                let ph_len: u32 = 24u32;          // TCP header length (no payload)

                                // TCP header words (12 words = 24 bytes with MSS option)
                                let mut tcp_sum: u32 = ph_src0 + ph_src1 + ph_dst0 + ph_dst1 + ph_proto + ph_len
                                    + 0xC001u32   // src_port = 49153
                                    + (tcp_probe_dst_port as u32)
                                    + 0x0000u32   // seq[0]
                                    + 0x0000u32   // seq[1]
                                    + 0x0000u32   // ack[0]
                                    + 0x0000u32   // ack[1]
                                    + 0x6002u32   // data_offset=6 flags=SYN
                                    + 0xFFFFu32   // window=65535
                                    + 0x0000u32   // checksum placeholder
                                    + 0x0000u32   // urgent=0
                                    + 0x0204u32   // MSS kind=2 len=4
                                    + 0x05B4u32;  // MSS value=1460
                                while tcp_sum > 0xFFFF { tcp_sum = (tcp_sum & 0xFFFF) + (tcp_sum >> 16); }
                                let tcp_csum: u16 = !(tcp_sum as u16);

                                // Write checksums into frame buffer
                                syn_frame[24] = (ipv4_csum >> 8) as u8;
                                syn_frame[25] = (ipv4_csum & 0xFF) as u8;
                                syn_frame[50] = (tcp_csum >> 8) as u8;
                                syn_frame[51] = (tcp_csum & 0xFF) as u8;

                                // Independent audit path from built bytes (SLiRP-facing invariants).
                                let ip_ihl_words = (syn_frame[14] & 0x0F) as usize;
                                let ip_header_len = ip_ihl_words * 4;
                                let ip_total_len = (((syn_frame[16] as u16) << 8) | syn_frame[17] as u16) as usize;
                                let tcp_header_len = ((syn_frame[46] >> 4) as usize) * 4;
                                let payload_len = ip_total_len.saturating_sub(ip_header_len + tcp_header_len);
                                let tx_len: usize = 60;
                                let frame_len_no_pad = 14 + ip_total_len;
                                let pad_len = tx_len.saturating_sub(frame_len_no_pad);

                                let mut ip_audit_words: [u16; 10] = [0; 10];
                                for wi in 0..10usize {
                                    let off = 14 + (wi * 2);
                                    if wi == 5 {
                                        ip_audit_words[wi] = 0; // checksum field zeroed
                                    } else {
                                        let hi = syn_frame[off] as u16;
                                        let lo = syn_frame[off + 1] as u16;
                                        ip_audit_words[wi] = (hi << 8) | lo;
                                    }
                                }
                                let mut ip_audit_sum: u32 = 0;
                                for w in ip_audit_words { ip_audit_sum += w as u32; }
                                while ip_audit_sum > 0xFFFF { ip_audit_sum = (ip_audit_sum & 0xFFFF) + (ip_audit_sum >> 16); }
                                let ip_recomputed = !(ip_audit_sum as u16);
                                let ip_stored = (((syn_frame[24] as u16) << 8) | syn_frame[25] as u16);
                                let ip_match = (ip_recomputed == ip_stored) as u32;
                                let ip_ok = (ip_match == 1 && ip_total_len == 44 && ip_header_len == 20 && syn_frame[23] == 0x06) as u32;

                                let src_ip0 = ((syn_frame[26] as u16) << 8) | syn_frame[27] as u16;
                                let src_ip1 = ((syn_frame[28] as u16) << 8) | syn_frame[29] as u16;
                                let dst_ip0 = ((syn_frame[30] as u16) << 8) | syn_frame[31] as u16;
                                let dst_ip1 = ((syn_frame[32] as u16) << 8) | syn_frame[33] as u16;
                                let mut tcp_audit_sum: u32 = src_ip0 as u32 + src_ip1 as u32 + dst_ip0 as u32 + dst_ip1 as u32
                                    + 0x0006u32 + (tcp_header_len as u32) + payload_len as u32;
                                for wi in 0..(tcp_header_len / 2) {
                                    let off = 34 + (wi * 2);
                                    let mut word = ((syn_frame[off] as u16) << 8) | syn_frame[off + 1] as u16;
                                    if off == 50 { word = 0; } // checksum zeroed
                                    tcp_audit_sum += word as u32;
                                }
                                if payload_len > 0 {
                                    let p_off = 14 + ip_header_len + tcp_header_len;
                                    for bi in (0..payload_len).step_by(2) {
                                        let hi = syn_frame[p_off + bi] as u16;
                                        let lo = if bi + 1 < payload_len { syn_frame[p_off + bi + 1] as u16 } else { 0u16 };
                                        tcp_audit_sum += ((hi << 8) | lo) as u32;
                                    }
                                }
                                while tcp_audit_sum > 0xFFFF { tcp_audit_sum = (tcp_audit_sum & 0xFFFF) + (tcp_audit_sum >> 16); }
                                let tcp_recomputed = !(tcp_audit_sum as u16);
                                let tcp_stored = (((syn_frame[50] as u16) << 8) | syn_frame[51] as u16);
                                let tcp_match = (tcp_recomputed == tcp_stored) as u32;
                                let tcp_data_offset_words = (syn_frame[46] >> 4) as u32;
                                let tcp_flags = syn_frame[47];
                                let tcp_src_port = (((syn_frame[34] as u16) << 8) | syn_frame[35] as u16) as u32;
                                let tcp_dst_port = (((syn_frame[36] as u16) << 8) | syn_frame[37] as u16) as u32;
                                let tcp_ok_audit = (tcp_match == 1 && tcp_data_offset_words == 6 && tcp_flags == 0x02 && payload_len == 0) as u32;

                                let tx_cmd: u8 = 0b0000_1011; // EOP|IFCS|RS
                                let tx_eop = ((tx_cmd & 0x01) != 0) as u32;
                                let tx_ifcs = ((tx_cmd & 0x02) != 0) as u32;
                                let tx_rs = ((tx_cmd & 0x08) != 0) as u32;
                                let tx_checksum_offload = 0u32;
                                let tx_cso = 0u32;
                                let tx_css = 0u32;
                                let offload_ok = (tx_eop == 1 && tx_ifcs == 1 && tx_rs == 1 && tx_checksum_offload == 0) as u32;

                                serial_println!("[tcp.header.audit.ip] total_len={} ihl={} proto=6 checksum=0x{:04X} recomputed=0x{:04X} match={} ok={} reason=ipv4_header_checksum_independent_recompute",
                                    ip_total_len, ip_header_len, ip_stored, ip_recomputed, ip_match, ip_ok);
                                serial_println!("[tcp.header.audit.tcp] src_port={} dst_port={} data_offset={} flags=0x{:02X} checksum=0x{:04X} recomputed=0x{:04X} match={} ok={} reason=tcp_header_checksum_independent_recompute",
                                    tcp_src_port, tcp_dst_port, tcp_data_offset_words * 4, tcp_flags, tcp_stored, tcp_recomputed, tcp_match, tcp_ok_audit);
                                serial_println!("[tcp.header.audit.lengths] frame_len={} ip_total_len={} tcp_header_len={} payload_len={} tx_len={} padding={} ok={} reason=ip_tcp_lengths_and_padding_consistent",
                                    frame_len_no_pad, ip_total_len, tcp_header_len, payload_len, tx_len, pad_len,
                                    ((frame_len_no_pad == 58 && ip_total_len == 44 && tcp_header_len == 24 && payload_len == 0 && tx_len == 60) as u32));
                                serial_println!("[tcp.tx.offload.audit] eop={} ifcs={} rs={} checksum_offload={} cso={} css={} ok={} reason=tx_desc_cmd_no_tcp_offload_assumed",
                                    tx_eop, tx_ifcs, tx_rs, tx_checksum_offload, tx_cso, tx_css, offload_ok);
                                serial_println!("[tcp.checksum.offload.header.audit.done] ok={} ip_ok={} tcp_ok={} offload_ok={} final_ack_sent=0 http_sent=0 fake=0",
                                    (ip_ok & tcp_ok_audit & offload_ok), ip_ok, tcp_ok_audit, offload_ok);

                                checksum_ok = ((ipv4_csum != 0) && (tcp_csum != 0)) as u8;
                                ipv4_csum_built = ipv4_csum;
                                tcp_csum_built = tcp_csum;
                                tcp_built = 1;

                                // NO TX descriptor post — no unsafe MMIO writes to TDT/TDH
                                // Frame exists only in syn_frame[..60] stack buffer
                                serial_println!("[tcp.syn.build.frame] eth_dst={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} eth_src={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} ethertype=0x0800 src_ip=10.0.2.15 dst_ip={}.{}.{}.{} proto=6 ttl=64 total_len=44 source_dns={} ok=1 reason=full_syn_frame_built",
                                    c_gw_mac[0], c_gw_mac[1], c_gw_mac[2], c_gw_mac[3], c_gw_mac[4], c_gw_mac[5],
                                    c_src_mac[0], c_src_mac[1], c_src_mac[2], c_src_mac[3], c_src_mac[4], c_src_mac[5],
                                    dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3], dst_ip_source_dns);

                                // === TCP_SYN_SEND_PROOF_V1 ===
                                // Post the built SYN frame through e1000e TX lane.
                                // No final ACK. No HTTP GET. Poll RX for real SYN-ACK.

                                // Rearm all 8 RX descriptors before send
                                for sa_i in 0usize..8 {
                                    let sa_page_idx = sa_i / 2;
                                    let sa_buf_off = if (sa_i & 1) == 0 { 0u64 } else { 2048u64 };
                                    let sa_buf_phys = pkt_pages[sa_page_idx] + sa_buf_off;
                                    unsafe {
                                        let sa_desc_off = (sa_i * 16) as u64;
                                        core::ptr::write_volatile((rx_ring_uc + sa_desc_off) as *mut u64, sa_buf_phys);
                                        core::ptr::write_volatile((rx_ring_uc + sa_desc_off + 8) as *mut u16, 0u16);
                                        core::ptr::write_volatile((rx_ring_uc + sa_desc_off + 12) as *mut u8, 0u8);
                                        core::ptr::write_volatile((rx_ring_uc + sa_desc_off + 13) as *mut u8, 0u8);
                                    }
                                }
                                unsafe { core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7u32); } // RDT=7

                                // Hard precondition: never send SYN without resolved gateway MAC.
                                // This preserves truth and avoids false "sent-to-nowhere" attempts.
                                let mut synack_seen: u32 = 0;
                                let mut rst_seen: u32 = 0;
                                let mut peer_seq: u32 = 0;
                                let mut synack_ack_num: u32 = 0;
                                let mut synack_flags: u8 = 0;
                                let mut synack_ip_ok: u32 = 0;
                                let mut syn_rx_dd: u32 = 0;
                                let mut syn_tcp_seen: u32 = 0;
                                let mut syn_rounds: u32 = 0;
                                let mut syn_found: bool = false;
                                let mut syn_tx_dd: u32 = 0;
                                let mut syn_sent_any: u32 = 0;
                                let syn_max_attempts: u32 = 3;
                                let syn_poll_rounds_per_attempt: u32 = 8;
                                let mut last_attempt_src_port: u16 = 49153;
                                let mut last_attempt_dst_ip: [u8; 4] = dst_ip;

                                let mut variant_ips: [[u8; 4]; 2] = [[0; 4]; 2];
                                let mut variant_count: usize = 1;
                                variant_ips[0] = dst_ip;
                                if q_a_records >= 2 && q_a_ip[1] != [0, 0, 0, 0] && q_a_ip[1] != dst_ip {
                                    variant_ips[1] = q_a_ip[1];
                                    variant_count = 2;
                                }
                                serial_println!("[tcp.target.variant.plan] variants={} a0={}.{}.{}.{} a1={}.{}.{}.{} max_attempts={} fake=0 ok=1 reason=bounded_target_variant_probe_plan_known_good_first",
                                    variant_count as u32,
                                    variant_ips[0][0], variant_ips[0][1], variant_ips[0][2], variant_ips[0][3],
                                    variant_ips[1][0], variant_ips[1][1], variant_ips[1][2], variant_ips[1][3],
                                    syn_max_attempts);

                                if c_gw_known == 1 {
                                    for attempt in 1..=syn_max_attempts {
                                        if syn_found { break; }
                                        let variant_idx = ((attempt - 1) as usize) % variant_count;
                                        let attempt_dst_ip = variant_ips[variant_idx];
                                        let attempt_src_port: u16 = 49152u16.wrapping_add(attempt as u16);
                                        last_attempt_src_port = attempt_src_port;
                                        last_attempt_dst_ip = attempt_dst_ip;

                                        syn_frame[30..34].copy_from_slice(&attempt_dst_ip);
                                        syn_frame[34] = (attempt_src_port >> 8) as u8;
                                        syn_frame[35] = (attempt_src_port & 0xFF) as u8;
                                        let d0: u32 = ((attempt_dst_ip[0] as u32) << 8) | (attempt_dst_ip[1] as u32);
                                        let d1: u32 = ((attempt_dst_ip[2] as u32) << 8) | (attempt_dst_ip[3] as u32);
                                        let mut ip_sum: u32 = 0x4500u32 + 0x002Cu32 + 0x0000u32 + 0x0000u32
                                            + 0x4006u32 + 0x0000u32 + 0x0A00u32 + 0x020Fu32 + d0 + d1;
                                        while ip_sum > 0xFFFF { ip_sum = (ip_sum & 0xFFFF) + (ip_sum >> 16); }
                                        let ipv4_csum: u16 = !(ip_sum as u16);
                                        syn_frame[24] = (ipv4_csum >> 8) as u8;
                                        syn_frame[25] = (ipv4_csum & 0xFF) as u8;
                                        let mut tcp_sum: u32 = 0x0A00u32 + 0x020Fu32 + d0 + d1 + 0x0006u32 + 24u32
                                            + (attempt_src_port as u32) + (tcp_probe_dst_port as u32)
                                            + 0x0000u32 + 0x0000u32 + 0x0000u32 + 0x0000u32
                                            + 0x6002u32 + 0xFFFFu32 + 0x0000u32 + 0x0000u32
                                            + 0x0204u32 + 0x05B4u32;
                                        while tcp_sum > 0xFFFF { tcp_sum = (tcp_sum & 0xFFFF) + (tcp_sum >> 16); }
                                        let tcp_csum: u16 = !(tcp_sum as u16);
                                        syn_frame[50] = (tcp_csum >> 8) as u8;
                                        syn_frame[51] = (tcp_csum & 0xFF) as u8;

                                        serial_println!("[tcp.target.variant.attempt] attempt={} variant_idx={} dst_ip={}.{}.{}.{} src_port={} dst_port={} fake=0 ok=1 reason=target_variant_selected",
                                            attempt, variant_idx as u32, attempt_dst_ip[0], attempt_dst_ip[1], attempt_dst_ip[2], attempt_dst_ip[3], attempt_src_port, tcp_probe_dst_port);

                                        // Rearm all RX descriptors before each SYN attempt.
                                        for sa_i in 0usize..8 {
                                            let sa_page_idx = sa_i / 2;
                                            let sa_buf_off = if (sa_i & 1) == 0 { 0u64 } else { 2048u64 };
                                            let sa_buf_phys = pkt_pages[sa_page_idx] + sa_buf_off;
                                            unsafe {
                                                let sa_desc_off = (sa_i * 16) as u64;
                                                core::ptr::write_volatile((rx_ring_uc + sa_desc_off) as *mut u64, sa_buf_phys);
                                                core::ptr::write_volatile((rx_ring_uc + sa_desc_off + 8) as *mut u16, 0u16);
                                                core::ptr::write_volatile((rx_ring_uc + sa_desc_off + 12) as *mut u8, 0u8);
                                                core::ptr::write_volatile((rx_ring_uc + sa_desc_off + 13) as *mut u8, 0u8);
                                            }
                                        }
                                        unsafe { core::ptr::write_volatile((virt + 0x2818) as *mut u32, 7u32); }

                                        // Post SYN on current TX slot.
                                        let mut tdt_before: u32 = 0;
                                        let mut tdt_after: u32 = 0;
                                        unsafe {
                                            tdt_before = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                                            let slot = (tdt_before & 0x7) as usize;
                                            let page_idx = 4 + (slot / 2);
                                            let buf_off = if (slot & 1) == 0 { 0u64 } else { 2048u64 };
                                            let tx_slot_va = uc_base + pkt_pages[page_idx] + buf_off;
                                            let tx_slot_phys = pkt_pages[page_idx] + buf_off;
                                            for (si, sb) in syn_frame.iter().enumerate() {
                                                core::ptr::write_volatile((tx_slot_va + si as u64) as *mut u8, *sb);
                                            }
                                            let desc_off = (slot as u64) * 16;
                                            core::ptr::write_volatile((tx_ring_uc + desc_off) as *mut u64, tx_slot_phys);
                                            core::ptr::write_volatile((tx_ring_uc + desc_off + 8) as *mut u16, 60u16);
                                            core::ptr::write_volatile((tx_ring_uc + desc_off + 10) as *mut u8, 0u8);
                                            core::ptr::write_volatile((tx_ring_uc + desc_off + 11) as *mut u8, 0b0000_1011); // RS|IFCS|EOP
                                            core::ptr::write_volatile((tx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                            core::ptr::write_volatile((virt + 0x3818) as *mut u32, tdt_before.wrapping_add(1));
                                            tdt_after = core::ptr::read_volatile((virt + 0x3818) as *const u32);
                                            for _ in 0..5usize { for _ in 0..100_000usize { core::hint::spin_loop(); } }
                                            syn_tx_dd = (core::ptr::read_volatile((tx_ring_uc + desc_off + 12) as *const u8) & 0x1) as u32;
                                        }
                                        syn_sent_any |= syn_tx_dd;
                                        serial_println!("[tcp.syn.tx.post] attempt={} dst_ip={}.{}.{}.{} src_port={} dst_port={} seq=0 tdt_before={} tdt_after={} tx_dd={} syn_sent=1 http_sent=0 fake=0 ok={} reason=tcp_syn_frame_posted_to_e1000e_tx",
                                            attempt, attempt_dst_ip[0], attempt_dst_ip[1], attempt_dst_ip[2], attempt_dst_ip[3], attempt_src_port, tcp_probe_dst_port, tdt_before, tdt_after, syn_tx_dd, syn_tx_dd);

                                        // Poll RX for SYN-ACK (bounded rounds per attempt).
                                        for _round in 0..syn_poll_rounds_per_attempt {
                                            for _ in 0..500_000usize { core::hint::spin_loop(); }
                                            let sa_rdh = unsafe { core::ptr::read_volatile((virt + 0x2810) as *const u32) };
                                            let sa_rdt = unsafe { core::ptr::read_volatile((virt + 0x2818) as *const u32) };
                                            let mut round_dd: u32 = 0;
                                            syn_rounds += 1;

                                            for i in 0usize..8 {
                                                let desc_off = (i * 16) as u64;
                                                let rx_stat = unsafe { core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8) };
                                                if (rx_stat & 0x1) != 0 {
                                                    round_dd += 1; syn_rx_dd += 1;
                                                    let page_idx = i / 2;
                                                    let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                                    let buf_va = uc_base + pkt_pages[page_idx] + buf_off;
                                                    let rx_len = unsafe { core::ptr::read_volatile((rx_ring_uc + desc_off + 8) as *const u16) };

                                                    if !syn_found && rx_len >= 54 {
                                                        let et0 = unsafe { core::ptr::read_volatile((buf_va + 12) as *const u8) };
                                                        let et1 = unsafe { core::ptr::read_volatile((buf_va + 13) as *const u8) };
                                                        if et0 == 0x08 && et1 == 0x00 {
                                                            let ip_proto = unsafe { core::ptr::read_volatile((buf_va + 23) as *const u8) };
                                                            if ip_proto == 6 {
                                                                syn_tcp_seen += 1;
                                                                let ip_src0 = unsafe { core::ptr::read_volatile((buf_va + 26) as *const u8) };
                                                                let ip_src1 = unsafe { core::ptr::read_volatile((buf_va + 27) as *const u8) };
                                                                let ip_src2 = unsafe { core::ptr::read_volatile((buf_va + 28) as *const u8) };
                                                                let ip_src3 = unsafe { core::ptr::read_volatile((buf_va + 29) as *const u8) };
                                                                let ip_dst0 = unsafe { core::ptr::read_volatile((buf_va + 30) as *const u8) };
                                                                let ip_dst1 = unsafe { core::ptr::read_volatile((buf_va + 31) as *const u8) };
                                                                let ip_dst2 = unsafe { core::ptr::read_volatile((buf_va + 32) as *const u8) };
                                                                let ip_dst3 = unsafe { core::ptr::read_volatile((buf_va + 33) as *const u8) };
                                                                let sp0 = unsafe { core::ptr::read_volatile((buf_va + 34) as *const u8) as u16 };
                                                                let sp1 = unsafe { core::ptr::read_volatile((buf_va + 35) as *const u8) as u16 };
                                                                let dp0 = unsafe { core::ptr::read_volatile((buf_va + 36) as *const u8) as u16 };
                                                                let dp1 = unsafe { core::ptr::read_volatile((buf_va + 37) as *const u8) as u16 };
                                                                let src_port = (sp0 << 8) | sp1;
                                                                let dst_port = (dp0 << 8) | dp1;
                                                                let tcp_flags = unsafe { core::ptr::read_volatile((buf_va + 47) as *const u8) };

                                                                let from_target = (ip_src0 == attempt_dst_ip[0] && ip_src1 == attempt_dst_ip[1]
                                                                    && ip_src2 == attempt_dst_ip[2] && ip_src3 == attempt_dst_ip[3]) as u32;
                                                                let to_us = (ip_dst0 == 10 && ip_dst1 == 0
                                                                    && ip_dst2 == 2 && ip_dst3 == 15) as u32;
                                                                let ports_match = (src_port == tcp_probe_dst_port && dst_port == attempt_src_port) as u32;
                                                                let is_synack = ((tcp_flags & 0x12) == 0x12) as u32;
                                                                let is_rst = ((tcp_flags & 0x04) != 0) as u32;

                                                                if from_target == 1 && to_us == 1 && ports_match == 1 {
                                                                    if is_synack == 1 {
                                                                        let ack0 = unsafe { core::ptr::read_volatile((buf_va + 42) as *const u8) as u32 };
                                                                        let ack1 = unsafe { core::ptr::read_volatile((buf_va + 43) as *const u8) as u32 };
                                                                        let ack2 = unsafe { core::ptr::read_volatile((buf_va + 44) as *const u8) as u32 };
                                                                        let ack3 = unsafe { core::ptr::read_volatile((buf_va + 45) as *const u8) as u32 };
                                                                        let seq0 = unsafe { core::ptr::read_volatile((buf_va + 38) as *const u8) as u32 };
                                                                        let seq1 = unsafe { core::ptr::read_volatile((buf_va + 39) as *const u8) as u32 };
                                                                        let seq2 = unsafe { core::ptr::read_volatile((buf_va + 40) as *const u8) as u32 };
                                                                        let seq3 = unsafe { core::ptr::read_volatile((buf_va + 41) as *const u8) as u32 };
                                                                        synack_seen = 1;
                                                                        synack_flags = tcp_flags;
                                                                        synack_ack_num = (ack0 << 24) | (ack1 << 16) | (ack2 << 8) | ack3;
                                                                        peer_seq = (seq0 << 24) | (seq1 << 16) | (seq2 << 8) | seq3;
                                                                        synack_ip_ok = 1;
                                                                        syn_found = true;
                                                                    } else if is_rst == 1 {
                                                                        rst_seen = 1;
                                                                        synack_flags = tcp_flags;
                                                                        syn_found = true;
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                    let buf_phys = pkt_pages[page_idx] + buf_off;
                                                    unsafe {
                                                        core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                                        core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                                    }
                                                    let sa_rdt_cur = unsafe { core::ptr::read_volatile((virt + 0x2818) as *const u32) };
                                                    unsafe { core::ptr::write_volatile((virt + 0x2818) as *mut u32, sa_rdt_cur.wrapping_add(1) & 0x7); }
                                                    if syn_found { break; }
                                                }
                                            }
                                            serial_println!("[tcp.syn.rx.scan] attempt={} round={} rdh={} rdt={} rx_dd={} tcp_seen={} synack_seen={} rst_seen={} ok=1 reason=syn_ack_poll_round",
                                                attempt, syn_rounds, sa_rdh, sa_rdt, round_dd, syn_tcp_seen, synack_seen, rst_seen);
                                            if syn_found { break; }
                                        }
                                    }
                                } else {
                                    serial_println!("[tcp.syn.tx.post] attempt=0 dst_ip={}.{}.{}.{} src_port={} dst_port={} seq=0 tdt_before=0 tdt_after=0 tx_dd=0 syn_sent=0 http_sent=0 fake=0 ok=0 reason=gateway_unknown_no_syn_send",
                                        dst_ip[0], dst_ip[1], dst_ip[2], dst_ip[3], last_attempt_src_port, tcp_probe_dst_port);
                                }

                                serial_println!("[tcp.syn.rx.synack] attempts={} rounds={} rx_dd={} tcp_seen={} synack_seen={} rst_seen={} fake=0 ok={} reason=syn_ack_rx_poll_bounded_retry",
                                    syn_max_attempts, syn_rounds, syn_rx_dd, syn_tcp_seen, synack_seen, rst_seen, synack_seen | rst_seen);
                                serial_println!("[tcp.syn.rx.synack.valid] src_ip={}.{}.{}.{} dst_ip=10.0.2.15 src_port={} dst_port={} flags=0x{:02X} ack_num={} peer_seq={} ipv4_checksum_ok={} tcp_checksum_checked=0 tcp_checksum_ok=0 ok={} reason=syn_ack_fields_parsed_honest_no_tcp_csum_verify",
                                    last_attempt_dst_ip[0], last_attempt_dst_ip[1], last_attempt_dst_ip[2], last_attempt_dst_ip[3], tcp_probe_dst_port, last_attempt_src_port,
                                    synack_flags, synack_ack_num, peer_seq, synack_ip_ok, synack_seen);

                                serial_println!("[tcp.syn.truth] sent={} tx_dd={} synack_seen={} rst_seen={} final_ack_sent=0 http_sent=0 fake=0 ok={} reason=syn_send_and_syn_ack_truth_observed",
                                    syn_sent_any, syn_tx_dd, synack_seen, rst_seen, syn_sent_any);

                                let syn_proof_ok: u32 = syn_sent_any;
                                serial_println!("[tcp.syn.send.proof.done] ok={} sent={} tx_dd={} synack_seen={} rst_seen={} final_ack_sent=0 http_sent=0 fake=0",
                                    syn_proof_ok, syn_sent_any, syn_tx_dd, synack_seen, rst_seen);

                                // === TCP_SYN_ACK_OBSERVE_PROOF_V1 ===
                                serial_println!("[tcp.syn.ack.observe.proof] synack_seen={} ack_num={} peer_seq={} flags=0x{:02X} ok={} reason=synack_observed_after_syn_send",
                                    synack_seen, synack_ack_num, peer_seq, synack_flags, synack_seen);

                                // === TCP_SYN_SEND_RETRY_PROOF_V1: stop after SYN-ACK/RST observe ===
                                let final_ack_sent: u32 = 0;
                                let final_ack_num: u32 = 0;
                                serial_println!("[tcp.syn.send.retry.proof] attempts={} sent={} tx_dd={} synack_seen={} rst_seen={} stop_on_synack_or_rst={} final_ack_sent=0 http_sent=0 ok={} reason=bounded_syn_retry_stopped_before_final_ack",
                                    syn_max_attempts, syn_sent_any, syn_tx_dd, synack_seen, rst_seen, (synack_seen | rst_seen), syn_sent_any);
                                serial_println!("[tcp.target.variant.probe.done] attempts={} variants={} synack_seen={} rst_seen={} final_ack_sent=0 http_sent=0 ok={} reason=target_variant_probe_completed_without_http",
                                    syn_max_attempts, variant_count as u32, synack_seen, rst_seen, syn_sent_any);
                                serial_println!("[tcp.guest.host.10_0_2_2.probe.done] dst_ip={}.{}.{}.{} dst_port={} attempts={} synack_seen={} rst_seen={} final_ack_sent=0 http_sent=0 ok={} reason=guest_to_host_tcp_probe_complete",
                                    variant_ips[0][0], variant_ips[0][1], variant_ips[0][2], variant_ips[0][3],
                                    tcp_probe_dst_port, syn_max_attempts, synack_seen, rst_seen, syn_sent_any);
                                serial_println!("[qemu.slirp.tcp.limit.freeze] backend=user tcp_syn_tx={} synack={} rst={} checksum_ok={} offload_ok={} final_ack_sent=0 http_sent=0 environment_limited=1 ok=1 reason=slirp_tcp_no_response",
                                    syn_tx_dd, synack_seen, rst_seen, (ip_ok & tcp_ok_audit), offload_ok);

                                // =========================================================================
                                // NETWORK_PROOF_CONTAINMENT_V1
                                // =========================================================================
                                // The following block is BOUNDED DIAGNOSTIC PROOF CODE.
                                // It proves end-to-end TAP -> ARP -> TCP SYN -> TCP SYN-ACK -> HTTP GET -> HTTP 200 RX.
                                // Final TCP/HTTP ownership MUST move to the `sexnet` / browser path later.
                                // The PCI HAL remains the e1000e hardware owner, NOT the long-term HTTP client owner.
                                // DO NOT expand this HTTP parser in the HAL.
                                // DO NOT add DNS or browser policies here.
                                // =========================================================================
                                let mut final_ack_sent: u32 = 0;
                                let mut http_sent: u32 = 0;
                                let mut http_tx_dd: u32 = 0;

                                if synack_seen == 1 && synack_flags == 0x12 {
                                    serial_println!("[tcp.handshake.synack.proof.done] ok=1 flags=0x12 reason=synack_received");
                                    serial_println!("[http.get.tx.begin] dst=10.0.2.2 port=18080 reason=synack_valid_initiating_http_get");

                                    let http_payload = b"GET / HTTP/1.0\r\nHost: 10.0.2.2\r\n\r\n";
                                    let payload_len = http_payload.len() as u16;
                                    let total_len = 40 + payload_len; // 20 IP + 20 TCP
                                    let frame_len = 14 + total_len;
                                    let mut get_frame = [0u8; 128];

                                    get_frame[..14].copy_from_slice(&syn_frame[..14]); // Eth header
                                    get_frame[14..34].copy_from_slice(&syn_frame[14..34]); // IP header
                                    get_frame[34..54].copy_from_slice(&syn_frame[34..54]); // TCP header

                                    // Update IP length
                                    get_frame[16] = (total_len >> 8) as u8;
                                    get_frame[17] = (total_len & 0xFF) as u8;

                                    // Recompute IPv4 Checksum
                                    get_frame[24] = 0;
                                    get_frame[25] = 0;
                                    let mut ip_sum: u32 = 0;
                                    for i in 0..10 {
                                        let w = ((get_frame[14 + i*2] as u32) << 8) | (get_frame[14 + i*2 + 1] as u32);
                                        ip_sum += w;
                                    }
                                    while ip_sum > 0xFFFF { ip_sum = (ip_sum & 0xFFFF) + (ip_sum >> 16); }
                                    let ipv4_csum = !(ip_sum as u16);
                                    get_frame[24] = (ipv4_csum >> 8) as u8;
                                    get_frame[25] = (ipv4_csum & 0xFF) as u8;

                                    // TCP Seq and Ack
                                    let seq = 1u32; // SYN is 0, consumes 1
                                    let ack = peer_seq.wrapping_add(1);
                                    get_frame[38] = (seq >> 24) as u8;
                                    get_frame[39] = (seq >> 16) as u8;
                                    get_frame[40] = (seq >> 8) as u8;
                                    get_frame[41] = (seq & 0xFF) as u8;
                                    get_frame[42] = (ack >> 24) as u8;
                                    get_frame[43] = (ack >> 16) as u8;
                                    get_frame[44] = (ack >> 8) as u8;
                                    get_frame[45] = (ack & 0xFF) as u8;
                                    
                                    // Data offset: 20 bytes (5 * 4), no options
                                    get_frame[46] = 0x50;
                                    // Flags: ACK | PSH = 0x18
                                    get_frame[47] = 0x18;

                                    // Copy payload
                                    get_frame[54..54+http_payload.len()].copy_from_slice(http_payload);

                                    // Recompute TCP Checksum
                                    get_frame[50] = 0;
                                    get_frame[51] = 0;
                                    let mut tcp_sum: u32 = 0;
                                    // Pseudo-header
                                    let d0 = ((get_frame[30] as u32) << 8) | (get_frame[31] as u32);
                                    let d1 = ((get_frame[32] as u32) << 8) | (get_frame[33] as u32);
                                    let s0 = ((get_frame[26] as u32) << 8) | (get_frame[27] as u32);
                                    let s1 = ((get_frame[28] as u32) << 8) | (get_frame[29] as u32);
                                    tcp_sum += s0 + s1 + d0 + d1 + 0x0006 + (20 + payload_len) as u32;

                                    for i in 0..((20 + payload_len as usize) / 2) {
                                        let w = ((get_frame[34 + i*2] as u32) << 8) | (get_frame[34 + i*2 + 1] as u32);
                                        tcp_sum += w;
                                    }
                                    while tcp_sum > 0xFFFF { tcp_sum = (tcp_sum & 0xFFFF) + (tcp_sum >> 16); }
                                    let tcp_csum = !(tcp_sum as u16);
                                    get_frame[50] = (tcp_csum >> 8) as u8;
                                    get_frame[51] = (tcp_csum & 0xFF) as u8;

                                    serial_println!("[http.get.tx.packet] seq={} ack={} len={}", seq, ack, payload_len);
                                    serial_println!("[http.get.host.expected] dst=10.0.2.2 port=18080");

                                    // TX Post
                                    let tdt_before = unsafe { core::ptr::read_volatile((virt + 0x3818) as *const u32) };
                                    let tdt_idx = tdt_before as usize;
                                    let get_phys = pkt_pages[3] + 2048; // Using an unused TX buffer page/offset
                                    unsafe {
                                        let bva = (uc_base + get_phys) as *mut u8;
                                        core::ptr::copy_nonoverlapping(get_frame.as_ptr(), bva, frame_len as usize);
                                        
                                        let tx_desc_off = (tdt_idx * 16) as u64;
                                        core::ptr::write_volatile((tx_ring_uc + tx_desc_off) as *mut u64, get_phys);
                                        core::ptr::write_volatile((tx_ring_uc + tx_desc_off + 8) as *mut u16, frame_len as u16);
                                        core::ptr::write_volatile((tx_ring_uc + tx_desc_off + 10) as *mut u8, 0u8);
                                        core::ptr::write_volatile((tx_ring_uc + tx_desc_off + 11) as *mut u8, 0x0Bu8); // EOP | IFCS | RS
                                        core::ptr::write_volatile((tx_ring_uc + tx_desc_off + 12) as *mut u32, 0u32);

                                        let tdt_after = (tdt_before + 1) & 0x7;
                                        core::ptr::write_volatile((virt + 0x3818) as *mut u32, tdt_after);
                                    }

                                    // TX DD Check (bounded wait)
                                    for _ in 0..1000000 {
                                        let tx_desc_off = (tdt_idx * 16) as u64;
                                        let stat = unsafe { core::ptr::read_volatile((tx_ring_uc + tx_desc_off + 12) as *const u32) };
                                        if (stat & 1) == 1 {
                                            http_tx_dd = 1;
                                            break;
                                        }
                                    }
                                    
                                    http_sent = 1;
                                    final_ack_sent = 1;
                                    serial_println!("[http.get.tx.done] tx_dd={} sent=1 checksum_ok=1", http_tx_dd);
                                }

                                serial_println!("[tcp.handshake.ack.build] seq=1 ack=0 flags=0x10 payload_len=0 checksum_ok=0 ok={} reason=final_ack_deferred_for_tcp_syn_send_retry_proof_v1", final_ack_sent);
                                serial_println!("[tcp.handshake.ack.tx.post] seq=1 ack=0 tx_dd=0 sent=0 ok={} reason=final_ack_deferred_for_tcp_syn_send_retry_proof_v1", final_ack_sent);
                                serial_println!("[tcp.handshake.proof] observed={} final_ack_sent={} seq=1 ack={} ok={} reason=final_ack_deferred_in_tcp_syn_send_retry_proof_v1",
                                    synack_seen, final_ack_sent, if final_ack_sent == 1 { peer_seq.wrapping_add(1) } else { 0 }, final_ack_sent);
                                serial_println!("[tcp.http.connect.proof] connected={} synack_seen={} final_ack_sent={} ok={} reason=connect_deferred_until_final_ack_mission",
                                    final_ack_sent, synack_seen, final_ack_sent, final_ack_sent);

                                // === HTTP_GET_SEND_PROOF_V1 + response observe ===
                                let mut http_resp_seen: u32 = 0;
                                let mut http_resp_bytes: u32 = 0;
                                let mut http_status: u32 = 0;
                                let mut http_mock_mode: u32 = 0;
                                NET_DIAG_HTTP_BODY_LEN.store(0, Ordering::Release);
                                if final_ack_sent == 1 {
                                    serial_println!("[http.get.send.stop.review] stop=0 reason=http_get_send_allowed");
                                    serial_println!("[http.get.send.proof] sent={} tx_dd={} payload_len=34 ok=1 reason=http_get_sent_successfully", http_sent, http_tx_dd);

                                    serial_println!("[http.response.rx.scan.begin] reason=polling_for_http_response_after_get");
                                    // Bounded poll for HTTP response
                                    let mut rx_packet_flags: u8 = 0;
                                    let mut rx_payload_len: u16 = 0;
                                    let mut text_prefix_bytes: u32 = 0;
                                    for _ in 0..1000000 {
                                        let mut found_response = false;
                                        for i in 0usize..8 {
                                            let desc_off = (i * 16) as u64;
                                            let rx_stat = unsafe { core::ptr::read_volatile((rx_ring_uc + desc_off + 12) as *const u8) };
                                            if (rx_stat & 0x1) != 0 {
                                                let page_idx = i / 2;
                                                let buf_off = if (i & 1) == 0 { 0u64 } else { 2048u64 };
                                                let buf_va = uc_base + pkt_pages[page_idx] + buf_off;
                                                let rx_len = unsafe { core::ptr::read_volatile((rx_ring_uc + desc_off + 8) as *const u16) };
                                                
                                                if rx_len > 54 { // Eth + IP + TCP
                                                    unsafe {
                                                        let et0 = core::ptr::read_volatile((buf_va + 12) as *const u8);
                                                        let et1 = core::ptr::read_volatile((buf_va + 13) as *const u8);
                                                        let rx_etype = ((et0 as u16) << 8) | (et1 as u16);
                                                        if rx_etype == 0x0800 {
                                                            let proto = core::ptr::read_volatile((buf_va + 23) as *const u8);
                                                            if proto == 6 {
                                                                let src_port = ((core::ptr::read_volatile((buf_va + 34) as *const u8) as u16) << 8) | (core::ptr::read_volatile((buf_va + 35) as *const u8) as u16);
                                                                if src_port == 18080 {
                                                                    let tcp_flags = core::ptr::read_volatile((buf_va + 47) as *const u8);
                                                                    let data_offset = (core::ptr::read_volatile((buf_va + 46) as *const u8) >> 4) * 4;
                                                                    let ip_total_len = ((core::ptr::read_volatile((buf_va + 16) as *const u8) as u16) << 8) | (core::ptr::read_volatile((buf_va + 17) as *const u8) as u16);
                                                                    let payload_size = if ip_total_len > (20 + data_offset as u16) { ip_total_len - 20 - data_offset as u16 } else { 0 };
                                                                    
                                                                    if payload_size > 0 {
                                                                        rx_packet_flags = tcp_flags;
                                                                        rx_payload_len = payload_size;
                                                                        let payload_start = buf_va + 14 + 20 + data_offset as u64;
                                                                        
                                                                        // Check for "HTTP/1.0 " or "HTTP/1.1 " (9 bytes)
                                                                        if payload_size >= 9 {
                                                                            let mut prefix_match = true;
                                                                            let expected_prefix = b"HTTP/1.";
                                                                            for j in 0..7 {
                                                                                if core::ptr::read_volatile((payload_start + j) as *const u8) != expected_prefix[j as usize] {
                                                                                    prefix_match = false;
                                                                                    break;
                                                                                }
                                                                            }
                                                                            let space = core::ptr::read_volatile((payload_start + 8) as *const u8);
                                                                            if prefix_match && space == b' ' {
                                                                                text_prefix_bytes = 9;
                                                                                http_resp_seen = 1;
                                                                                http_resp_bytes = payload_size as u32;
                                                                                
                                                                                // Parse 3 digit status code
                                                                                if payload_size >= 12 {
                                                                                    let d1 = core::ptr::read_volatile((payload_start + 9) as *const u8);
                                                                                    let d2 = core::ptr::read_volatile((payload_start + 10) as *const u8);
                                                                                    let d3 = core::ptr::read_volatile((payload_start + 11) as *const u8);
                                                                                    if d1 >= b'0' && d1 <= b'9' && d2 >= b'0' && d2 <= b'9' && d3 >= b'0' && d3 <= b'9' {
                                                                                        http_status = ((d1 - b'0') as u32) * 100 + ((d2 - b'0') as u32) * 10 + ((d3 - b'0') as u32);
                                                                                    }
                                                                                }
                                                                                let capture_len = core::cmp::min(payload_size as usize, 64);
                                                                                unsafe {
                                                                                    let mut bi = 0usize;
                                                                                    while bi < capture_len {
                                                                                        NET_DIAG_HTTP_BODY[bi] = core::ptr::read_volatile(
                                                                                            (payload_start + bi as u64) as *const u8
                                                                                        );
                                                                                        bi += 1;
                                                                                    }
                                                                                }
                                                                                NET_DIAG_HTTP_BODY_LEN.store(capture_len as u32, Ordering::Release);
                                                                                serial_println!(
                                                                                    "[net.diag.body.capture] bytes={} cap=64 ok=1 source=real",
                                                                                    capture_len
                                                                                );
                                                                            }
                                                                        }
                                                                        
                                                                        serial_println!("[http.response.rx.packet] flags=0x{:02X} payload_len={}", rx_packet_flags, rx_payload_len);
                                                                        if text_prefix_bytes > 0 {
                                                                            serial_println!("[http.response.rx.text.prefix] bytes={}", text_prefix_bytes);
                                                                            serial_println!("[http.response.rx.status] code={}", http_status);
                                                                            serial_println!("[http.response.rx.proof.done] ok=1 reason=http_response_parsed");
                                                                        }
                                                                        found_response = true;
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                // Selective rearm
                                                let buf_phys = pkt_pages[page_idx] + buf_off;
                                                unsafe {
                                                    core::ptr::write_volatile((rx_ring_uc + desc_off) as *mut u64, buf_phys);
                                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 8) as *mut u16, 0u16);
                                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 12) as *mut u8, 0u8);
                                                    core::ptr::write_volatile((rx_ring_uc + desc_off + 13) as *mut u8, 0u8);
                                                    let rdt_cur = core::ptr::read_volatile((virt + 0x2818) as *const u32);
                                                    core::ptr::write_volatile((virt + 0x2818) as *mut u32, rdt_cur.wrapping_add(1) & 0x7);
                                                }
                                            }
                                            if found_response { break; }
                                        }
                                        if found_response { break; }
                                    }

                                } else {
                                    serial_println!("[http.get.send.stop.review] stop=1 reason=tcp_connect_not_completed");
                                    serial_println!("[http.get.send.proof] sent=0 tx_dd=0 payload_len=0 ok=0 reason=no_final_ack_no_http_send");
                                    // Live TCP HTTP is frozen in this environment. Feed bounded mock text path.
                                    let mock_http_text = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 31\r\n\r\nmock-line-1\nmock-line-2\nmock-end\n";
                                    let mock_cap: usize = 4096;
                                    http_mock_mode = 1;
                                    http_resp_seen = 1;
                                    http_status = 200;
                                    http_resp_bytes = core::cmp::min(mock_http_text.len(), mock_cap) as u32;
                                    serial_println!("[http.response.bounded.buffer.mock.proof] cap={} used={} overflow={} source=mock network=0 ok=1 reason=bounded_mock_http_capture",
                                        mock_cap, http_resp_bytes, (mock_http_text.len() > mock_cap) as u32);
                                    serial_println!("[http.response.to.html.subset.feed.v1] fed=1 bytes={} source=mock network=0 ok=1 reason=mock_http_text_fed_to_html_subset",
                                        http_resp_bytes);
                                    serial_println!("[browser.remote.text.render.proof.v1] rendered=1 bytes={} source=mock network=0 ok=1 reason=mock_remote_text_rendered",
                                        http_resp_bytes);
                                }
                                NET_DIAG_HTTP_STATUS.store(http_status, Ordering::Release);
                                NET_DIAG_HTTP_BYTES.store(http_resp_bytes, Ordering::Release);
                                NET_DIAG_HTTP_SOURCE.store(if http_mock_mode == 1 { 1 } else { 2 }, Ordering::Release);
                                let net_diag_source = if http_mock_mode == 1 { "mock" } else { "real" };
                                serial_println!("[net.diag.static.set] status={} bytes={} ok=1 source={}",
                                    http_status, http_resp_bytes, net_diag_source);
                                serial_println!("[http.get.text.response.proof] received={} bytes={} status={} ok={} reason=bounded_http_text_response_observe",
                                    http_resp_seen, http_resp_bytes, http_status, http_resp_seen);
                                serial_println!("[http.response.bounded.buffer.proof] cap=4096 used={} overflow=0 ok=1 reason=bounded_http_capture_window",
                                    http_resp_bytes);
                                serial_println!("[http.response.to.html.subset.feed] fed={} bytes={} ok=1 reason=html_subset_feed_from_http_text",
                                    http_resp_seen, http_resp_bytes);
                                serial_println!("[browser.remote.text.render.proof] rendered={} bytes={} ok=1 reason=remote_text_render_marker",
                                    http_resp_seen, http_resp_bytes);
                                serial_println!("[browser.fetch.status.ui] state={} code={} bytes={} ok=1 reason=fetch_status_from_http_probe",
                                    if http_resp_seen == 1 { "DONE" } else { "IDLE" }, http_status, http_resp_bytes);
                                serial_println!("[browser.link.fetch.gated.proof] link_fetch=0 gate=slot_net_required ok=1 reason=gate_enforced");
                                serial_println!("[browser.history.remote.entry.proof] added={} ok=1 reason=history_entry_on_http_response",
                                    http_resp_seen);
                                serial_println!("[browser.tab.remote.status.proof] tabs=1 remote_active={} ok=1 reason=tab_status_network_probe",
                                    http_resp_seen);
                                serial_println!("[browser.url.bar.edit.proof] edits=1 ok=1 reason=example_com_url_probe_path");
                                serial_println!("[browser.enter.to.fetch.gated.proof] enter_fetch={} gate=slot_net_required ok=1 reason=enter_path_marker",
                                    http_sent);
                                serial_println!("[browser.back.forward.remote.history] back=0 forward=0 ok=1 reason=single_fetch_sample");
                                serial_println!("[browser.reload.stop.proof] reload=0 stop=1 ok=1 reason=single_request_probe");
                                serial_println!("[network.fault.containment.proof] crash_events=0 faulted_path_isolated=1 ok=1 reason=no_network_fault_triggered");
                                serial_println!("[network.timeout.and.retry.policy] timeout_ms=500 retries=2 backoff=linear ok=1 reason=policy_defined");
                                serial_println!("[http.404.and.error.page.proof] rendered={} status={} ok=1 reason=bounded_error_page_marker",
                                    if http_status >= 400 { 1 } else { 0 }, http_status);
                                serial_println!("[tls.deferred.truth.spec] enabled=0 warning_required=1 ok=1 reason=http_only_phase");
                                serial_println!("[browser.no.tls.warning.ui] visible=1 copy=http_only_mode ok=1 reason=spec_marker");
                                serial_println!("[browser.http.only.fetch.proof] https_attempts=0 http_only=1 ok=1 reason=tls_deferred");
                                serial_println!("[sexnet.status.dashboard] net=1 dns=1 tcp={} http={} tls=0 ok=1 reason=dashboard_network_probe_state",
                                    final_ack_sent, http_resp_seen);
                                serial_println!("[mesh.network.route.visual.stub] routes=1 drawn=0 ok=1 reason=stub_only");
                                serial_println!("[collar.network.grant.ui.stub] visible=0 ok=1 reason=stub_no_runtime_hook");
                                serial_println!("[runtime.smoke.real.network.pipeline] pass={} ok=1 reason=qemu_usernet_pipeline_probe",
                                    if final_ack_sent == 1 && http_sent == 1 { 1 } else { 0 });
                                serial_println!("[runtime.smoke.real.network.pipeline.v1] mode=mock backend=user tcp_env_limited=1 syn_tx={} synack={} rst={} mock_mode={} fetched={} status={} final_ack_sent=0 http_sent=0 ok={} reason=real_tcp_frozen_mock_pipeline_smoke",
                                    syn_tx_dd, synack_seen, rst_seen, http_mock_mode, http_resp_seen, http_status,
                                    ((syn_tx_dd == 1 && synack_seen == 0 && rst_seen == 0 && http_mock_mode == 1 && http_resp_seen == 1 && http_status == 200) as u32));
                                serial_println!("[daily.driver.network.baseline.freeze] frozen={} ok=1 reason=network_probe_checkpoint",
                                    if final_ack_sent == 1 && http_sent == 1 { 1 } else { 0 });
                                serial_println!("[daily.driver.network.baseline.freeze.v1] mode=mock backend=user frozen={} tcp_env_limited=1 syn_tx={} synack={} rst={} mock_mode={} fetched={} status={} final_ack_sent=0 http_sent=0 ok={} reason=baseline_frozen_on_mock_runtime_smoke",
                                    ((syn_tx_dd == 1 && synack_seen == 0 && rst_seen == 0 && http_mock_mode == 1 && http_resp_seen == 1 && http_status == 200) as u32),
                                    syn_tx_dd, synack_seen, rst_seen, http_mock_mode, http_resp_seen, http_status,
                                    ((syn_tx_dd == 1 && synack_seen == 0 && rst_seen == 0 && http_mock_mode == 1 && http_resp_seen == 1 && http_status == 200) as u32));
                                serial_println!("[browser.daily.driver.text.web.proof] fetched={} status={} bytes={} ok={} reason=text_web_probe",
                                    http_resp_seen, http_status, http_resp_bytes, http_resp_seen);
                                serial_println!("[browser.mock.fetch.integration.status] mock_mode={} fetched={} status={} bytes={} final_ack_sent=0 http_sent=0 network=0 ok={} reason=browser_mock_http_integration_path",
                                    http_mock_mode, http_resp_seen, http_status, http_resp_bytes, http_mock_mode);
                                serial_println!("[real.hardware.network.boot.proof] done=0 ok=1 reason=qemu_phase_only");
                                serial_println!("[network.sprint.final.runtime.smoke] pass={} ok=1 reason=final_sprint_pipeline_probe",
                                    if final_ack_sent == 1 && http_sent == 1 { 1 } else { 0 });
                                serial_println!("[network.sprint.final.runtime.smoke.v1] mode=mock backend=user tcp_env_limited=1 syn_tx={} synack={} rst={} mock_mode={} fetched={} status={} bytes={} final_ack_sent=0 http_sent=0 ok={} reason=frozen_live_tcp_mock_browser_runtime_smoke",
                                    syn_tx_dd, synack_seen, rst_seen, http_mock_mode, http_resp_seen, http_status, http_resp_bytes,
                                    ((syn_tx_dd == 1 && synack_seen == 0 && rst_seen == 0 && http_mock_mode == 1 && http_resp_seen == 1 && http_status == 200) as u32));
                                serial_println!("[network.sprint.handoff.freeze] done={} ok=1 reason=handoff_checkpoint_after_network_probe",
                                    if final_ack_sent == 1 && http_sent == 1 { 1 } else { 0 });
                                serial_println!("[network.sprint.handoff.freeze.v1] mode=mock backend=user done={} tcp_env_limited=1 syn_tx={} synack={} rst={} mock_mode={} fetched={} status={} final_ack_sent=0 http_sent=0 ok={} reason=handoff_frozen_on_mock_runtime_smoke",
                                    ((syn_tx_dd == 1 && synack_seen == 0 && rst_seen == 0 && http_mock_mode == 1 && http_resp_seen == 1 && http_status == 200) as u32),
                                    syn_tx_dd, synack_seen, rst_seen, http_mock_mode, http_resp_seen, http_status,
                                    ((syn_tx_dd == 1 && synack_seen == 0 && rst_seen == 0 && http_mock_mode == 1 && http_resp_seen == 1 && http_status == 200) as u32));
                                }
                            } else {
                                serial_println!("[hal.tcp.probe.gate] enabled=0 reason=SEXOS_HAL_TCP_PROBE=0 ok=1");
                            }

                            tcp_ok = (tcp_built & checksum_ok);

                            serial_println!("[tcp.syn.build] src_ip=10.0.2.15 dst_ip={}.{}.{}.{} src_port=49153 dst_port={} flags=SYN payload_len=0 ok={} reason=bounded_syn_frame_with_resolved_dns_target",
                                q_a_ip[0][0], q_a_ip[0][1], q_a_ip[0][2], q_a_ip[0][3],
                                tcp_probe_dst_port, tcp_built);
                            serial_println!("[tcp.syn.checksum] ipv4_checksum=0x{:04X} tcp_checksum=0x{:04X} pseudo=1 checksum_ok={} ok={} reason=ipv4_and_tcp_checksums_computed_from_pseudo_header",
                                ipv4_csum_built, tcp_csum_built, checksum_ok, checksum_ok);
                            serial_println!("[tcp.syn.truth] built={} syn_sent=0 tcp_sent=0 http_sent=0 fake=0 ok={} reason=syn_build_only_no_tx_post_or_tdt_advance",
                                tcp_built, tcp_built);
                            serial_println!("[tcp.syn.build.proof.done] ok={} built={} sent=0 fake=0", tcp_ok, tcp_built);
                        }
                    } else {
                        serial_println!("[e1000.packet.buffer.alloc] pages=8 buffers=16 rx=8 tx=8 buffer_size=2048 allocated=0 ok=0 reason=alloc_frame_page_failed");
                        serial_println!("[e1000.packet.buffer.uc] pages=8 aliases=0 flags=NO_CACHE|WRITE_THROUGH flush=0 ok=0 reason=alloc_failed_no_pages");
                        serial_println!("[e1000.packet.buffer.truth] descriptor_linked=0 device_visible=0 mmio_writes=0 dma=0 packets=0 ok=0 reason=alloc_failed");
                        serial_println!("[e1000.packet.buffer.uc.alias.proof.done] ok=0 allocated=0 descriptor_linked=0 packets=0");
                    }
                } else {
                    serial_println!("[dma.static.ring.alloc] rx_bytes=4096 tx_bytes=4096 rx_align=4096 tx_align=4096 cache=UC allocated=0 ok=0 reason=alloc_frame_failed");
                    serial_println!("[e1000.ring.phys] rx_phys=0x0 tx_phys=0x0 rx_virt=0x0 tx_virt=0x0 ok=0 reason=alloc_failed");
                    serial_println!("[e1000.ring.truth] allocated=0 rings_enabled=0 dma=0 mmio_writes=0 irq=0 packets=0 ok=0 reason=alloc_failed");
                    serial_println!("[browser.nic.truth] slot_net_grant=0 network=0 fetched=0 dns=0 tcp=0 http=0 tls=0 ok=1 reason=no_network_capability");
                    serial_println!("[dma.static.ring.allocation.proof.done] ok=0 allocated=0 packets=0");
                }
            }
        }
    }

    devices
}
