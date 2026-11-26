// SPDX-License-Identifier: GPL-3.0-only
//! Modern virtio-net (virtio 1.0+, PCI 0x1af4:0x1041) driver for SexNet.
//!
//! Operates strictly within the BOOT-leased BAR view (`SLOT_NIC`) and the
//! immutable 256 KiB network DMA pool (`SLOT_DMA_REGION`, region 2). Uses
//! split virtqueues (queue 0 = RX, queue 1 = TX) with zero heap allocations,
//! pre-posted RX buffers, out-of-order-safe TX slot tracking, and batched RX
//! refill notifications.

use core::sync::atomic::{fence, Ordering};
#[cfg(target_os = "none")]
use sex_pdx::{memory_region as m, pci_command as c, virtio as v};
use crate::ethernet::{self, Event, Failure, Frame, Phase, SendError};

const PCI_CMD_MEMORY: u16 = 1 << 1;
const PCI_CMD_BUS_MASTER: u16 = 1 << 2;
const DMA_POOL_BYTES: usize = 256 * 1024;

// virtio_pci_common_cfg field offsets (virtio 1.x, 4.1.4.3).
const DEVICE_FEATURE_SELECT: u32 = 0x00;
const DEVICE_FEATURE: u32 = 0x04;
const DRIVER_FEATURE_SELECT: u32 = 0x08;
const DRIVER_FEATURE: u32 = 0x0c;
const DEVICE_STATUS: u32 = 0x14;
const QUEUE_SELECT: u32 = 0x16;
const QUEUE_SIZE: u32 = 0x18;
const QUEUE_MSIX_VECTOR: u32 = 0x1a;
const QUEUE_ENABLE: u32 = 0x1c;
const QUEUE_NOTIFY_OFF: u32 = 0x1e;
const QUEUE_DESC: u32 = 0x20;
const QUEUE_DRIVER: u32 = 0x28;
const QUEUE_DEVICE: u32 = 0x30;

// Device status bits (virtio 1.x, 2.1).
const S_ACK: u8 = 1;
const S_DRIVER: u8 = 2;
const S_DRIVER_OK: u8 = 4;
const S_FEATURES_OK: u8 = 8;

// Feature bits (virtio 1.x, 5.1.3 & 6).
const F_MAC: u64 = 1 << 5;
const F_STATUS: u64 = 1 << 16;
const F_VERSION_1: u64 = 1 << 32;

// Virtqueue descriptor flags (virtio 1.x, 2.7.5).
const VIRTQ_DESC_F_WRITE: u16 = 2;
const VIRTIO_MSI_NO_VECTOR: u16 = 0xffff;

/// Modern virtio-net header (`virtio_net_hdr_v1`, 12 bytes).
pub const HDR_LEN: usize = 12;
/// Maximum RX/TX queue depths sized to fit inside the 256 KiB BOOT DMA pool.
pub const MAX_RX_COUNT: u16 = 64;
pub const MAX_TX_COUNT: u16 = 32;
pub const BUFFER: usize = 2048;
/// Batched RX refill doorbell threshold.
const RX_NOTIFY_BATCH: u16 = 8;
const RESET_SPINS: u32 = 1_000_000;

// Fixed offsets inside the 256 KiB DMA pool (total used = 0x34000 = 212,992 bytes).
pub const RX_DESC_OFF: usize = 0x0000;
pub const RX_AVAIL_OFF: usize = 0x0400;
pub const RX_USED_OFF: usize = 0x1000;
pub const TX_DESC_OFF: usize = 0x2000;
pub const TX_AVAIL_OFF: usize = 0x2400;
pub const TX_USED_OFF: usize = 0x3000;
pub const RX_DATA_OFF: usize = 0x4000;
pub const TX_DATA_OFF: usize = RX_DATA_OFF + (MAX_RX_COUNT as usize) * BUFFER;
pub const USED_BYTES: usize = TX_DATA_OFF + (MAX_TX_COUNT as usize) * BUFFER;

const _: () = assert!(USED_BYTES <= DMA_POOL_BYTES);

/// Low-level register/DMA/command abstraction so the virtio-net state machine
/// and ring logic can be unit-tested on the host without hardware.
pub trait Transport {
    fn common_r8(&self, off: u32) -> u8;
    fn common_r16(&self, off: u32) -> u16;
    fn common_r32(&self, off: u32) -> u32;
    fn common_w8(&mut self, off: u32, val: u8);
    fn common_w16(&mut self, off: u32, val: u16);
    fn common_w32(&mut self, off: u32, val: u32);
    fn common_w64(&mut self, off: u32, val: u64) {
        self.common_w32(off, val as u32);
        self.common_w32(off + 4, (val >> 32) as u32);
    }
    fn device_r8(&self, off: u32) -> u8;
    fn isr_r8(&self) -> u8;
    fn notify_queue(&mut self, queue: u16, notify_off: u16);
    fn pci_command(&mut self, set: u16, clear: u16) -> Result<(), u64>;
    fn dma_pa(&self) -> u64;
    fn dma_va(&self) -> *mut u8;
    fn dma_len(&self) -> usize;
}

/// Live SexOS transport over the BOOT-leased BAR view and DMA region.
#[cfg(target_os = "none")]
pub struct LiveTransport {
    bar: usize,
    layout: v::Layout,
    dma_va: usize,
    dma_pa: u64,
    dma_len: usize,
}

#[cfg(target_os = "none")]
impl LiveTransport {
    /// Open the leased modern virtio-net device at `SLOT_NIC` if present.
    pub fn open() -> Result<Self, u64> {
        let identity = sex_pdx::device_identity(sex_pdx::SLOT_NIC)?;
        if identity.vendor != v::VENDOR || identity.product != v::NET {
            return Err(c::DENIED);
        }
        let view = sex_pdx::memory_region_full_view(
            sex_pdx::SLOT_NIC,
            m::QUERY_VIEW | m::READ | m::WRITE,
        )?;
        let layout = v::layout(sex_pdx::SLOT_NIC)?;
        if !layout.valid(view.length) || view.dma_address.is_some() || view.virtual_address == 0 {
            return Err(c::DENIED);
        }
        let dma_info = sex_pdx::memory_region_info(sex_pdx::SLOT_DMA_REGION)?;
        if dma_info.region_id != c::DMA_REGION_ID
            || dma_info.length != c::DMA_BYTES
            || dma_info.rights != m::QUERY_VIEW | m::READ | m::WRITE | m::DMA_ADDRESS
            || dma_info.peer_pd.is_some()
        {
            return Err(c::DENIED);
        }
        let dma = sex_pdx::memory_region_full_view(sex_pdx::SLOT_DMA_REGION, dma_info.rights)?;
        let dma_pa = dma.dma_address.ok_or(c::DENIED)?;
        if dma.region_id != dma_info.region_id || dma.length != dma_info.length || dma.virtual_address == 0 {
            return Err(c::DENIED);
        }
        Ok(Self {
            bar: view.virtual_address as usize,
            layout,
            dma_va: dma.virtual_address as usize,
            dma_pa,
            dma_len: dma.length as usize,
        })
    }

    fn at<T>(&self, window: v::Window, off: u32) -> *mut T {
        assert!(off as usize + core::mem::size_of::<T>() <= window.length as usize, "VIRTIO_NET_MMIO_BOUNDS");
        (self.bar + window.offset as usize + off as usize) as *mut T
    }
}

#[cfg(target_os = "none")]
impl Transport for LiveTransport {
    fn common_r8(&self, off: u32) -> u8 { unsafe { self.at::<u8>(self.layout.common, off).read_volatile() } }
    fn common_r16(&self, off: u32) -> u16 { unsafe { self.at::<u16>(self.layout.common, off).read_volatile() } }
    fn common_r32(&self, off: u32) -> u32 { unsafe { self.at::<u32>(self.layout.common, off).read_volatile() } }
    fn common_w8(&mut self, off: u32, val: u8) { unsafe { self.at::<u8>(self.layout.common, off).write_volatile(val) } }
    fn common_w16(&mut self, off: u32, val: u16) { unsafe { self.at::<u16>(self.layout.common, off).write_volatile(val) } }
    fn common_w32(&mut self, off: u32, val: u32) { unsafe { self.at::<u32>(self.layout.common, off).write_volatile(val) } }
    fn device_r8(&self, off: u32) -> u8 { unsafe { self.at::<u8>(self.layout.device, off).read_volatile() } }
    fn isr_r8(&self) -> u8 { unsafe { self.at::<u8>(self.layout.isr, 0).read_volatile() } }
    fn notify_queue(&mut self, queue: u16, notify_off: u16) {
        let at = notify_off as u32 * self.layout.notify_multiplier;
        unsafe { self.at::<u16>(self.layout.notify, at).write_volatile(queue) }
    }
    fn pci_command(&mut self, set: u16, clear: u16) -> Result<(), u64> {
        sex_pdx::device_command(sex_pdx::SLOT_NIC, set as u64, clear as u64).map(|_| ())
    }
    fn dma_pa(&self) -> u64 { self.dma_pa }
    fn dma_va(&self) -> *mut u8 { self.dma_va as *mut u8 }
    fn dma_len(&self) -> usize { self.dma_len }
}

#[derive(Clone, Copy, Default)]
struct TxMeta {
    sequence: u64,
    length: usize,
    hash: u64,
}

/// Virtio-net driver state machine over split virtqueues.
pub struct Driver<T: Transport> {
    t: T,
    phase: Phase,
    failure: Option<Failure>,
    pub mac: [u8; 6],
    rx_qsize: u16,
    tx_qsize: u16,
    rx_notify_off: u16,
    tx_notify_off: u16,
    rx_avail_idx: u16,
    rx_last_used: u16,
    rx_refill_pending: u16,
    tx_avail_idx: u16,
    tx_last_used: u16,
    tx_in_use: u32,
    tx_meta: [TxMeta; MAX_TX_COUNT as usize],
    pub sent: u64,
    pub received: u64,
    pub dropped: u64,
}

#[cfg(target_os = "none")]
impl Driver<LiveTransport> {
    /// Open and initialize a live modern virtio-net device at `SLOT_NIC`.
    pub fn open() -> Result<Self, &'static str> {
        let transport = LiveTransport::open().map_err(|_| "no_lease")?;
        Self::init(transport)
    }
}

impl<T: Transport> Driver<T> {
    /// Initialize a virtio-net device over `t` and pre-post all RX buffers.
    pub fn init(mut t: T) -> Result<Self, &'static str> {
        if t.dma_len() < USED_BYTES {
            return Err("dma_small");
        }
        // Enable memory space with bus master off before touching device registers.
        t.pci_command(PCI_CMD_MEMORY, PCI_CMD_BUS_MASTER).map_err(|_| "command_mem")?;

        // 1. Reset device (virtio 1.x, 3.1.1).
        t.common_w8(DEVICE_STATUS, 0);
        let mut reset_ok = false;
        for _ in 0..RESET_SPINS {
            if t.common_r8(DEVICE_STATUS) == 0 {
                reset_ok = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !reset_ok {
            return Err("reset_timeout");
        }

        // 2-3. ACKNOWLEDGE | DRIVER.
        t.common_w8(DEVICE_STATUS, S_ACK | S_DRIVER);

        // 4. Feature negotiation.
        t.common_w32(DEVICE_FEATURE_SELECT, 0);
        let low = t.common_r32(DEVICE_FEATURE) as u64;
        t.common_w32(DEVICE_FEATURE_SELECT, 1);
        let offered = low | ((t.common_r32(DEVICE_FEATURE) as u64) << 32);
        if offered & F_VERSION_1 == 0 {
            return Err("legacy_device");
        }
        let negotiated = F_VERSION_1 | (offered & (F_MAC | F_STATUS));
        t.common_w32(DRIVER_FEATURE_SELECT, 0);
        t.common_w32(DRIVER_FEATURE, negotiated as u32);
        t.common_w32(DRIVER_FEATURE_SELECT, 1);
        t.common_w32(DRIVER_FEATURE, (negotiated >> 32) as u32);

        // 5-6. FEATURES_OK and verify.
        t.common_w8(DEVICE_STATUS, S_ACK | S_DRIVER | S_FEATURES_OK);
        if t.common_r8(DEVICE_STATUS) & S_FEATURES_OK == 0 {
            return Err("features_rejected");
        }

        // Read station MAC address from device config space.
        let mut mac = [0u8; 6];
        if negotiated & F_MAC != 0 {
            for (i, b) in mac.iter_mut().enumerate() {
                *b = t.device_r8(i as u32);
            }
        }
        if mac == [0; 6] || mac == [0xff; 6] || (mac[0] & 1) != 0 {
            return Err("invalid_mac");
        }

        // Zero ring metadata region before programming queue addresses.
        unsafe { core::ptr::write_bytes(t.dma_va(), 0, RX_DATA_OFF); }

        // 7a. Configure RX virtqueue (queue 0).
        t.common_w16(QUEUE_SELECT, 0);
        let max_rx = t.common_r16(QUEUE_SIZE);
        if max_rx < 2 || !max_rx.is_power_of_two() {
            return Err("rx_queue_size");
        }
        let rx_qsize = MAX_RX_COUNT.min(max_rx);
        let pa = t.dma_pa();
        t.common_w16(QUEUE_SIZE, rx_qsize);
        t.common_w64(QUEUE_DESC, pa + RX_DESC_OFF as u64);
        t.common_w64(QUEUE_DRIVER, pa + RX_AVAIL_OFF as u64);
        t.common_w64(QUEUE_DEVICE, pa + RX_USED_OFF as u64);
        let rx_notify_off = t.common_r16(QUEUE_NOTIFY_OFF);
        t.common_w16(QUEUE_ENABLE, 1);

        // 7b. Configure TX virtqueue (queue 1).
        t.common_w16(QUEUE_SELECT, 1);
        let max_tx = t.common_r16(QUEUE_SIZE);
        if max_tx < 2 || !max_tx.is_power_of_two() {
            return Err("tx_queue_size");
        }
        let tx_qsize = MAX_TX_COUNT.min(max_tx);
        t.common_w16(QUEUE_SIZE, tx_qsize);
        t.common_w64(QUEUE_DESC, pa + TX_DESC_OFF as u64);
        t.common_w64(QUEUE_DRIVER, pa + TX_AVAIL_OFF as u64);
        t.common_w64(QUEUE_DEVICE, pa + TX_USED_OFF as u64);
        let tx_notify_off = t.common_r16(QUEUE_NOTIFY_OFF);
        t.common_w16(QUEUE_ENABLE, 1);

        let mut driver = Self {
            t,
            phase: Phase::Check,
            failure: None,
            mac,
            rx_qsize,
            tx_qsize,
            rx_notify_off,
            tx_notify_off,
            rx_avail_idx: 0,
            rx_last_used: 0,
            rx_refill_pending: 0,
            tx_avail_idx: 0,
            tx_last_used: 0,
            tx_in_use: 0,
            tx_meta: [TxMeta::default(); MAX_TX_COUNT as usize],
            sent: 0,
            received: 0,
            dropped: 0,
        };

        // Pre-post all RX buffers before setting DRIVER_OK.
        for i in 0..rx_qsize {
            driver.post_rx_slot(i);
        }
        driver.flush_rx_avail();

        // Enable PCI bus mastering, then mark DRIVER_OK and kick RX queue.
        driver.t.pci_command(PCI_CMD_BUS_MASTER, 0).map_err(|_| "command_bm")?;
        driver.t.common_w8(DEVICE_STATUS, S_ACK | S_DRIVER | S_FEATURES_OK | S_DRIVER_OK);
        driver.t.notify_queue(0, driver.rx_notify_off);
        driver.rx_refill_pending = 0;

        Ok(driver)
    }

    pub fn phase(&self) -> Phase { self.phase }
    pub fn failure(&self) -> Option<Failure> { self.failure }
    pub fn rx_count(&self) -> u16 { self.rx_qsize }
    pub fn tx_count(&self) -> u16 { self.tx_qsize }
    pub fn busy(&self) -> bool {
        let mask = if self.tx_qsize >= 32 { u32::MAX } else { (1u32 << self.tx_qsize) - 1 };
        (self.tx_in_use & mask) == mask
    }

    /// Route RX and TX virtqueue interrupts to MSI-X vector 0.
    pub fn arm_interrupts(&mut self, msix: bool) -> bool {
        if !msix {
            return false;
        }
        self.t.common_w16(QUEUE_SELECT, 0);
        self.t.common_w16(QUEUE_MSIX_VECTOR, 0);
        if self.t.common_r16(QUEUE_MSIX_VECTOR) != 0 {
            return false;
        }
        self.t.common_w16(QUEUE_SELECT, 1);
        self.t.common_w16(QUEUE_MSIX_VECTOR, 0);
        if self.t.common_r16(QUEUE_MSIX_VECTOR) != 0 {
            self.disarm_interrupts();
            return false;
        }
        true
    }

    pub fn disarm_interrupts(&mut self) {
        self.t.common_w16(QUEUE_SELECT, 0);
        self.t.common_w16(QUEUE_MSIX_VECTOR, VIRTIO_MSI_NO_VECTOR);
        self.t.common_w16(QUEUE_SELECT, 1);
        self.t.common_w16(QUEUE_MSIX_VECTOR, VIRTIO_MSI_NO_VECTOR);
    }

    pub fn acknowledge_interrupts(&mut self) {
        let _ = self.t.isr_r8();
    }

    #[inline]
    fn dma_ptr<U>(&self, off: usize) -> *mut U {
        unsafe { self.t.dma_va().add(off) as *mut U }
    }

    fn write_desc(&mut self, desc_table_off: usize, slot: u16, addr: u64, len: u32, flags: u16) {
        let base = desc_table_off + (slot as usize) * 16;
        unsafe {
            self.dma_ptr::<u64>(base).write_volatile(addr);
            self.dma_ptr::<u32>(base + 8).write_volatile(len);
            self.dma_ptr::<u16>(base + 12).write_volatile(flags);
            self.dma_ptr::<u16>(base + 14).write_volatile(0);
        }
    }

    fn post_rx_slot(&mut self, slot: u16) {
        let buf_pa = self.t.dma_pa() + (RX_DATA_OFF + (slot as usize) * BUFFER) as u64;
        self.write_desc(RX_DESC_OFF, slot, buf_pa, BUFFER as u32, VIRTQ_DESC_F_WRITE);
        let ring_slot = (self.rx_avail_idx % self.rx_qsize) as usize;
        unsafe {
            self.dma_ptr::<u16>(RX_AVAIL_OFF + 4 + ring_slot * 2).write_volatile(slot);
        }
        self.rx_avail_idx = self.rx_avail_idx.wrapping_add(1);
        self.rx_refill_pending = self.rx_refill_pending.saturating_add(1);
    }

    fn flush_rx_avail(&mut self) {
        fence(Ordering::SeqCst);
        unsafe {
            self.dma_ptr::<u16>(RX_AVAIL_OFF + 2).write_volatile(self.rx_avail_idx);
        }
        fence(Ordering::SeqCst);
    }

    fn reclaim_tx(&mut self) -> Option<Event> {
        let mut event = None;
        let used_idx = unsafe { self.dma_ptr::<u16>(TX_USED_OFF + 2).read_volatile() };
        if used_idx == self.tx_last_used {
            return None;
        }
        fence(Ordering::Acquire);
        while self.tx_last_used != used_idx {
            let elem_off = TX_USED_OFF + 4 + ((self.tx_last_used % self.tx_qsize) as usize) * 8;
            let id = unsafe { self.dma_ptr::<u32>(elem_off).read_volatile() } as u16;
            self.tx_last_used = self.tx_last_used.wrapping_add(1);
            if id < self.tx_qsize {
                let bit = 1u32 << id;
                if self.tx_in_use & bit != 0 {
                    self.tx_in_use &= !bit;
                    if event.is_none() {
                        let meta = self.tx_meta[id as usize];
                        event = Some(Event::Transmitted {
                            sequence: meta.sequence,
                            index: id as u32,
                            length: meta.length,
                            hash: meta.hash,
                        });
                    }
                }
            }
        }
        event
    }

    /// Cooperative service-turn poll: reclaim completed TX descriptors and pop
    /// one received frame if `receive` is true.
    pub fn advance_with_receive(&mut self, _now: u64, receive: bool) -> Option<Event> {
        if self.phase == Phase::Quarantined {
            return None;
        }
        if self.phase == Phase::Check {
            self.phase = Phase::Running;
            return None;
        }

        let tx_event = self.reclaim_tx();

        if receive {
            let used_idx = unsafe { self.dma_ptr::<u16>(RX_USED_OFF + 2).read_volatile() };
            if used_idx != self.rx_last_used {
                fence(Ordering::Acquire);
                let elem_off = RX_USED_OFF + 4 + ((self.rx_last_used % self.rx_qsize) as usize) * 8;
                let id = unsafe { self.dma_ptr::<u32>(elem_off).read_volatile() } as u16;
                let total_len = unsafe { self.dma_ptr::<u32>(elem_off + 4).read_volatile() } as usize;
                self.rx_last_used = self.rx_last_used.wrapping_add(1);

                let slot = id % self.rx_qsize;
                let buf_off = RX_DATA_OFF + (slot as usize) * BUFFER;
                let event = if total_len > HDR_LEN && total_len - HDR_LEN <= ethernet::MAX_FRAME {
                    let frame_len = total_len - HDR_LEN;
                    let mut frame = Frame { length: frame_len, bytes: [0; ethernet::MAX_FRAME] };
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            self.t.dma_va().add(buf_off + HDR_LEN),
                            frame.bytes.as_mut_ptr(),
                            frame_len,
                        );
                    }
                    self.received = self.received.saturating_add(1);
                    Some(Event::Received { sequence: self.received, index: slot as u32, frame })
                } else {
                    self.dropped = self.dropped.saturating_add(1);
                    Some(Event::Dropped { index: slot as u32, length: total_len, status: 0, errors: 1 })
                };

                self.post_rx_slot(slot);
                self.flush_rx_avail();
                if self.rx_refill_pending >= RX_NOTIFY_BATCH || self.rx_last_used == used_idx {
                    self.t.notify_queue(0, self.rx_notify_off);
                    self.rx_refill_pending = 0;
                }
                return event;
            }
        }

        if self.rx_refill_pending > 0 {
            self.t.notify_queue(0, self.rx_notify_off);
            self.rx_refill_pending = 0;
        }
        tx_event
    }

    /// Submit one Ethernet frame onto the TX virtqueue.
    pub fn send(&mut self, packet: &[u8]) -> Result<(u64, u32, usize, u64), SendError> {
        if self.phase != Phase::Running {
            return Err(SendError::NotReady);
        }
        if packet.is_empty() || packet.len() > ethernet::MAX_FRAME {
            return Err(SendError::Length);
        }
        let _ = self.reclaim_tx();
        let slot = (0..self.tx_qsize).find(|&i| (self.tx_in_use & (1u32 << i)) == 0)
            .ok_or(SendError::Busy)?;

        let frame_len = packet.len().max(60);
        let buf_off = TX_DATA_OFF + (slot as usize) * BUFFER;
        unsafe {
            // Zero 12-byte virtio_net_hdr_v1.
            core::ptr::write_bytes(self.t.dma_va().add(buf_off), 0, HDR_LEN);
            core::ptr::copy_nonoverlapping(
                packet.as_ptr(),
                self.t.dma_va().add(buf_off + HDR_LEN),
                packet.len(),
            );
            if frame_len > packet.len() {
                core::ptr::write_bytes(
                    self.t.dma_va().add(buf_off + HDR_LEN + packet.len()),
                    0,
                    frame_len - packet.len(),
                );
            }
        }
        let hash = ethernet::fingerprint(unsafe {
            core::slice::from_raw_parts(self.t.dma_va().add(buf_off + HDR_LEN), frame_len)
        });

        let buf_pa = self.t.dma_pa() + buf_off as u64;
        self.write_desc(TX_DESC_OFF, slot, buf_pa, (HDR_LEN + frame_len) as u32, 0);
        let ring_slot = (self.tx_avail_idx % self.tx_qsize) as usize;
        unsafe {
            self.dma_ptr::<u16>(TX_AVAIL_OFF + 4 + ring_slot * 2).write_volatile(slot);
        }
        fence(Ordering::SeqCst);
        self.tx_avail_idx = self.tx_avail_idx.wrapping_add(1);
        unsafe {
            self.dma_ptr::<u16>(TX_AVAIL_OFF + 2).write_volatile(self.tx_avail_idx);
        }
        fence(Ordering::SeqCst);
        self.t.notify_queue(1, self.tx_notify_off);

        self.sent = self.sent.saturating_add(1);
        self.tx_in_use |= 1u32 << slot;
        self.tx_meta[slot as usize] = TxMeta { sequence: self.sent, length: frame_len, hash };
        Ok((self.sent, slot as u32, frame_len, hash))
    }
}
