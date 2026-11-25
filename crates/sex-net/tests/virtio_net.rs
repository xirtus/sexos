// SPDX-License-Identifier: GPL-3.0-only
//! Host unit tests for the modern virtio-net (0x1af4:0x1041) split-virtqueue driver.

#[allow(dead_code)]
mod ethernet {
    pub const MAX_FRAME: usize = 1514;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Phase { Check, Zero, Descriptors, Registers, Enable, Queues, Link, Running, Quarantined }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Failure { Authority, Command, Registers, Deadline, Clock, Descriptor, Completion, Sequence }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SendError { NotReady, Busy, Length, Quarantined }
    pub struct Frame { pub length: usize, pub bytes: [u8; MAX_FRAME] }
    pub enum Event {
        Received { sequence: u64, index: u32, frame: Frame },
        Dropped { index: u32, length: usize, status: u8, errors: u8 },
        Transmitted { sequence: u64, index: u32, length: usize, hash: u64 },
    }
    pub fn fingerprint(bytes: &[u8]) -> u64 {
        let mut h = 0xcbf29ce484222325u64;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
}

#[allow(dead_code)]
#[path = "../../../servers/sexnet/src/virtio_net.rs"]
mod virtio_net;

use virtio_net::{
    Driver, Transport, HDR_LEN, MAX_RX_COUNT, MAX_TX_COUNT, RX_AVAIL_OFF, RX_DATA_OFF,
    RX_USED_OFF, TX_USED_OFF, USED_BYTES,
};

struct MockTransport {
    dma: Vec<u8>,
    dma_pa: u64,
    status: u8,
    device_features: u64,
    driver_features: u64,
    dev_feat_sel: u32,
    drv_feat_sel: u32,
    queue_select: u16,
    queue_size: [u16; 2],
    queue_msix: [u16; 2],
    queue_enable: [u16; 2],
    queue_desc: [u64; 2],
    queue_driver: [u64; 2],
    queue_device: [u64; 2],
    mac: [u8; 6],
    pci_cmd: u16,
    notifies: Vec<u16>,
    reject_features: bool,
    reject_msix: bool,
}

impl MockTransport {
    fn new() -> Self {
        Self {
            dma: vec![0u8; 256 * 1024],
            dma_pa: 0x2000_0000,
            status: 0,
            device_features: (1 << 32) | (1 << 5) | (1 << 16), // VERSION_1 | MAC | STATUS
            driver_features: 0,
            dev_feat_sel: 0,
            drv_feat_sel: 0,
            queue_select: 0,
            queue_size: [256, 256],
            queue_msix: [0xffff, 0xffff],
            queue_enable: [0, 0],
            queue_desc: [0, 0],
            queue_driver: [0, 0],
            queue_device: [0, 0],
            mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            pci_cmd: 0,
            notifies: Vec::new(),
            reject_features: false,
            reject_msix: false,
        }
    }
}

impl Transport for MockTransport {
    fn common_r8(&self, off: u32) -> u8 {
        if off == 0x14 { self.status } else { 0 }
    }
    fn common_r16(&self, off: u32) -> u16 {
        let q = (self.queue_select as usize).min(1);
        match off {
            0x18 => self.queue_size[q],
            0x1a => self.queue_msix[q],
            0x1c => self.queue_enable[q],
            0x1e => q as u16,
            _ => 0,
        }
    }
    fn common_r32(&self, off: u32) -> u32 {
        if off == 0x04 {
            if self.dev_feat_sel == 0 { self.device_features as u32 } else { (self.device_features >> 32) as u32 }
        } else {
            0
        }
    }
    fn common_w8(&mut self, off: u32, val: u8) {
        if off == 0x14 {
            self.status = if self.reject_features && (val & 8) != 0 { val & !8 } else { val };
        }
    }
    fn common_w16(&mut self, off: u32, val: u16) {
        let q = (self.queue_select as usize).min(1);
        match off {
            0x16 => self.queue_select = val,
            0x18 => self.queue_size[q] = val,
            0x1a => self.queue_msix[q] = if self.reject_msix { 0xffff } else { val },
            0x1c => self.queue_enable[q] = val,
            _ => {}
        }
    }
    fn common_w32(&mut self, off: u32, val: u32) {
        let q = (self.queue_select as usize).min(1);
        match off {
            0x00 => self.dev_feat_sel = val,
            0x08 => self.drv_feat_sel = val,
            0x0c => {
                if self.drv_feat_sel == 0 {
                    self.driver_features = (self.driver_features & !0xffff_ffff) | (val as u64);
                } else {
                    self.driver_features = (self.driver_features & 0xffff_ffff) | ((val as u64) << 32);
                }
            }
            0x20 => self.queue_desc[q] = (self.queue_desc[q] & !0xffff_ffff) | (val as u64),
            0x24 => self.queue_desc[q] = (self.queue_desc[q] & 0xffff_ffff) | ((val as u64) << 32),
            0x28 => self.queue_driver[q] = (self.queue_driver[q] & !0xffff_ffff) | (val as u64),
            0x2c => self.queue_driver[q] = (self.queue_driver[q] & 0xffff_ffff) | ((val as u64) << 32),
            0x30 => self.queue_device[q] = (self.queue_device[q] & !0xffff_ffff) | (val as u64),
            0x34 => self.queue_device[q] = (self.queue_device[q] & 0xffff_ffff) | ((val as u64) << 32),
            _ => {}
        }
    }
    fn device_r8(&self, off: u32) -> u8 {
        self.mac.get(off as usize).copied().unwrap_or(0)
    }
    fn isr_r8(&self) -> u8 { 1 }
    fn notify_queue(&mut self, queue: u16, _notify_off: u16) {
        self.notifies.push(queue);
    }
    fn pci_command(&mut self, set: u16, clear: u16) -> Result<(), u64> {
        self.pci_cmd = (self.pci_cmd & !clear) | set;
        Ok(())
    }
    fn dma_pa(&self) -> u64 { self.dma_pa }
    fn dma_va(&self) -> *mut u8 { self.dma.as_ptr() as *mut u8 }
    fn dma_len(&self) -> usize { self.dma.len() }
}

#[test]
fn init_rejects_legacy_and_invalid_mac() {
    let mut legacy = MockTransport::new();
    legacy.device_features &= !(1 << 32);
    assert_eq!(Driver::init(legacy).err(), Some("legacy_device"));

    let mut bad_mac = MockTransport::new();
    bad_mac.mac = [0x01, 0x00, 0x5e, 0x00, 0x00, 0x01]; // multicast
    assert_eq!(Driver::init(bad_mac).err(), Some("invalid_mac"));

    let mut rejected = MockTransport::new();
    rejected.reject_features = true;
    assert_eq!(Driver::init(rejected).err(), Some("features_rejected"));
}

#[test]
fn tx_padding_saturation_and_out_of_order_reclaim() {
    assert!(USED_BYTES <= 256 * 1024);
    let t = MockTransport::new();
    let mut drv = Driver::init(t).expect("init");
    assert_eq!(drv.mac, [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    assert_eq!(drv.rx_count(), MAX_RX_COUNT);
    assert_eq!(drv.tx_count(), MAX_TX_COUNT);
    assert!(drv.advance_with_receive(1, true).is_none());
    assert_eq!(drv.phase(), ethernet::Phase::Running);

    // Short frame is padded to 60 bytes with 12-byte zeroed virtio-net header.
    let short = [0xaa; 42];
    let (seq, slot, len, _) = drv.send(&short).expect("send short");
    assert_eq!((seq, slot, len), (1, 0, 60));

    // Fill remaining 31 TX slots; next send must return Busy.
    for _ in 1..MAX_TX_COUNT {
        assert!(drv.send(&[0xbb; 64]).is_ok());
    }
    assert!(drv.busy());
    assert_eq!(drv.send(&[0xcc; 64]), Err(ethernet::SendError::Busy));

    // Out-of-order TX reclaim.
    let mut t2 = MockTransport::new();
    let dma_ptr = t2.dma.as_mut_ptr();
    let mut drv2 = Driver::init(t2).expect("init2");
    drv2.advance_with_receive(1, true);
    for i in 0..4u8 {
        assert!(drv2.send(&[i; 64]).is_ok());
    }
    unsafe {
        *(dma_ptr.add(TX_USED_OFF + 4) as *mut u32) = 2;
        *(dma_ptr.add(TX_USED_OFF + 8) as *mut u32) = 0;
        *(dma_ptr.add(TX_USED_OFF + 2) as *mut u16) = 1;
    }
    match drv2.advance_with_receive(2, false) {
        Some(ethernet::Event::Transmitted { sequence, index, length, .. }) => {
            assert_eq!((sequence, index, length), (3, 2, 64));
        }
        _ => panic!("expected Transmitted event"),
    }
    let (_, reused, _, _) = drv2.send(&[0x55; 64]).expect("reuse slot 2");
    assert_eq!(reused, 2);
}

#[test]
fn rx_receive_strips_header_and_refills_ring() {
    let mut t = MockTransport::new();
    let dma_ptr = t.dma.as_mut_ptr();
    let mut drv = Driver::init(t).expect("init");
    drv.advance_with_receive(1, true);

    let initial_avail = unsafe { *(dma_ptr.add(RX_AVAIL_OFF + 2) as *const u16) };
    assert_eq!(initial_avail, MAX_RX_COUNT);

    let payload: Vec<u8> = (0..64u8).collect();
    unsafe {
        let buf = dma_ptr.add(RX_DATA_OFF);
        std::ptr::write_bytes(buf, 0, HDR_LEN);
        std::ptr::copy_nonoverlapping(payload.as_ptr(), buf.add(HDR_LEN), payload.len());
        *(dma_ptr.add(RX_USED_OFF + 4) as *mut u32) = 0;
        *(dma_ptr.add(RX_USED_OFF + 8) as *mut u32) = (HDR_LEN + payload.len()) as u32;
        *(dma_ptr.add(RX_USED_OFF + 2) as *mut u16) = 1;
    }

    match drv.advance_with_receive(2, true) {
        Some(ethernet::Event::Received { sequence, index, frame }) => {
            assert_eq!((sequence, index, frame.length), (1, 0, 64));
            assert_eq!(&frame.bytes[..64], payload.as_slice());
        }
        _ => panic!("expected Received event"),
    }
    let next_avail = unsafe { *(dma_ptr.add(RX_AVAIL_OFF + 2) as *const u16) };
    assert_eq!(next_avail, MAX_RX_COUNT + 1);
}

#[test]
fn msix_interrupt_arm_and_reject() {
    let t = MockTransport::new();
    let mut drv = Driver::init(t).expect("init");
    assert!(!drv.arm_interrupts(false));
    assert!(drv.arm_interrupts(true));
    drv.acknowledge_interrupts();
    drv.disarm_interrupts();

    let mut t_rej = MockTransport::new();
    t_rej.reject_msix = true;
    let mut drv_rej = Driver::init(t_rej).expect("init rej");
    assert!(!drv_rej.arm_interrupts(true));
}
