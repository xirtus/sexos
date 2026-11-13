// SPDX-License-Identifier: GPL-3.0-only
//! virtio-gpu 2D scanout (workstream L, slice L2).
//!
//! When BOOT leased Display a virtio-gpu (QEMU `virtio-vga` / `virtio-gpu-pci`),
//! Display stops relying on the firmware framebuffer: it reads the monitor's EDID
//! through the device, creates one host resource backed by a BOOT scanout buffer,
//! and after every publication transfers only the rows that changed and flushes
//! them. The host shows a resource only on flush, so a half-written frame never
//! reaches the screen: every publication is one tear-free flip, counted as a
//! vblank sequence for `sex-kms` flip events.
//!
//! The device protocol is written here from the virtio 1.2 specification
//! (5.7 GPU device): one split virtqueue per queue, a single request in flight,
//! requests and replies in the service's own virtio DMA pool. A driver never
//! names a physical register address; the pool and scanout addresses come from
//! regions BOOT granted with the DMA-address right.

use core::sync::atomic::{fence, Ordering};
use sex_pdx::{display_frames as df, memory_region as m, pci_command as c, virtio as v};

const QSIZE: u16 = 16;
// virtio_pci_common_cfg offsets (virtio 1.x 4.1.4.3).
const DEVICE_FEATURE_SELECT: u32 = 0x00;
const DEVICE_FEATURE: u32 = 0x04;
const DRIVER_FEATURE_SELECT: u32 = 0x08;
const DRIVER_FEATURE: u32 = 0x0c;
const DEVICE_STATUS: u32 = 0x14;
const QUEUE_SELECT: u32 = 0x16;
const QUEUE_SIZE: u32 = 0x18;
const QUEUE_ENABLE: u32 = 0x1c;
const QUEUE_NOTIFY_OFF: u32 = 0x1e;
const QUEUE_DESC: u32 = 0x20;
const QUEUE_DRIVER: u32 = 0x28;
const QUEUE_DEVICE: u32 = 0x30;
const S_ACK: u8 = 1;
const S_DRIVER: u8 = 2;
const S_DRIVER_OK: u8 = 4;
const S_FEATURES_OK: u8 = 8;
const F_EDID: u64 = 1 << 1;
const F_VERSION_1: u64 = 1 << 32;

// Commands and replies (5.7.6.7).
const GET_DISPLAY_INFO: u32 = 0x0100;
const RESOURCE_CREATE_2D: u32 = 0x0101;
const SET_SCANOUT: u32 = 0x0103;
const RESOURCE_FLUSH: u32 = 0x0104;
const TRANSFER_TO_HOST_2D: u32 = 0x0105;
const RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const GET_EDID: u32 = 0x010a;
const OK_NODATA: u32 = 0x1100;
const OK_DISPLAY_INFO: u32 = 0x1101;
const OK_EDID: u32 = 0x1104;
const FLAG_FENCE: u32 = 1;
/// B8G8R8X8: XRGB8888 as little-endian bytes, Display's packed pixel.
const FORMAT_XRGB: u32 = 2;
const RESOURCE: u32 = 1;
const HDR: usize = 24;
const MAX_SCANOUTS: usize = 16;

// Pool layout, in bytes from the start of the virtio DMA pool.
const CTRL_DESC: u64 = 0x0000;
const CTRL_AVAIL: u64 = 0x0400;
const CTRL_USED: u64 = 0x1000;
const REQUEST: u64 = 0x4000;
const REPLY: u64 = 0x5000;
const REPLY_BYTES: u32 = 0x1000;
const SPIN: u32 = 50_000_000;

/// One scanout as GET_DISPLAY_INFO reports it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Head { pub enabled: bool, pub width: u32, pub height: u32 }

pub struct Gpu {
    bar: u64,
    layout: v::Layout,
    pool_va: u64,
    pool_pa: u64,
    qsize: u16,
    avail_idx: u16,
    last_used: u16,
    notify: u64,
    scanout_va: u64,
    width: u32,
    height: u32,
    fence: u64,
    pub heads: [Head; MAX_SCANOUTS],
    pub scanouts: u32,
    pub edid_len: usize,
    pub edid: [u8; 256],
}

impl Gpu {
    fn common<T>(&self, off: u32) -> *mut T { (self.bar + self.layout.common.offset as u64 + off as u64) as *mut T }
    fn r8(&self, off: u32) -> u8 { unsafe { self.common::<u8>(off).read_volatile() } }
    fn r16(&self, off: u32) -> u16 { unsafe { self.common::<u16>(off).read_volatile() } }
    fn r32(&self, off: u32) -> u32 { unsafe { self.common::<u32>(off).read_volatile() } }
    fn w8(&self, off: u32, v: u8) { unsafe { self.common::<u8>(off).write_volatile(v) } }
    fn w16(&self, off: u32, v: u16) { unsafe { self.common::<u16>(off).write_volatile(v) } }
    fn w32(&self, off: u32, v: u32) { unsafe { self.common::<u32>(off).write_volatile(v) } }
    fn w64(&self, off: u32, v: u64) { self.w32(off, v as u32); self.w32(off + 4, (v >> 32) as u32); }
    fn cfg32(&self, off: u32) -> u32 {
        if off + 4 > self.layout.device.length { return 0; }
        unsafe { ((self.bar + self.layout.device.offset as u64 + off as u64) as *const u32).read_volatile() }
    }
    fn pool<T>(&self, off: u64) -> *mut T { (self.pool_va + off) as *mut T }
    fn put32(&self, at: u64, v: u32) { unsafe { self.pool::<u32>(REQUEST + at).write_volatile(v) } }
    fn put64(&self, at: u64, v: u64) { unsafe { self.pool::<u64>(REQUEST + at).write_volatile(v) } }
    fn reply32(&self, at: u64) -> u32 { unsafe { self.pool::<u32>(REPLY + at).read_volatile() } }

    fn header(&mut self, kind: u32, fenced: bool) {
        for i in 0..(HDR / 8) as u64 { self.put64(i * 8, 0); }
        self.put32(0, kind);
        if fenced { self.fence += 1; self.put32(4, FLAG_FENCE); self.put64(8, self.fence); }
    }
    fn rect(&self, at: u64, x: u32, y: u32, w: u32, h: u32) {
        self.put32(at, x); self.put32(at + 4, y); self.put32(at + 8, w); self.put32(at + 12, h);
    }

    /// Send the request built at REQUEST (`len` bytes) on the control queue and
    /// wait for the device's reply. Returns the reply type.
    fn submit(&mut self, len: u32) -> Result<u32, &'static str> {
        let desc = |i: u64, addr: u64, len: u32, flags: u16, next: u16| unsafe {
            let d = self.pool::<u8>(CTRL_DESC + i * 16);
            (d as *mut u64).write_volatile(addr);
            (d.add(8) as *mut u32).write_volatile(len);
            (d.add(12) as *mut u16).write_volatile(flags);
            (d.add(14) as *mut u16).write_volatile(next);
        };
        desc(0, self.pool_pa + REQUEST, len, 1, 1);
        desc(1, self.pool_pa + REPLY, REPLY_BYTES, 2, 0);
        unsafe {
            self.pool::<u32>(REPLY).write_volatile(0);
            self.pool::<u16>(CTRL_AVAIL + 4 + 2 * (self.avail_idx % self.qsize) as u64).write_volatile(0);
            fence(Ordering::SeqCst);
            self.avail_idx = self.avail_idx.wrapping_add(1);
            self.pool::<u16>(CTRL_AVAIL + 2).write_volatile(self.avail_idx);
            fence(Ordering::SeqCst);
            (self.notify as *mut u16).write_volatile(0);
        }
        for _ in 0..SPIN {
            let used = unsafe { self.pool::<u16>(CTRL_USED + 2).read_volatile() };
            if used != self.last_used {
                self.last_used = used;
                fence(Ordering::SeqCst);
                return Ok(self.reply32(0));
            }
            core::hint::spin_loop();
        }
        Err("timeout")
    }
    fn expect(&mut self, len: u32, want: u32) -> Result<(), &'static str> {
        if self.submit(len)? == want { Ok(()) } else { Err("refused") }
    }

    /// Copy the rows `first..end` of the scanout buffer to the host resource and
    /// show them. One call is one flip; returns its sequence number.
    pub fn present(&mut self, first: u32, end: u32) -> Result<u64, &'static str> {
        let end = end.min(self.height);
        if first >= end { return Ok(self.fence); }
        let rows = end - first;
        self.header(TRANSFER_TO_HOST_2D, false);
        self.rect(24, 0, first, self.width, rows);
        self.put64(40, first as u64 * self.width as u64 * 4);
        self.put32(48, RESOURCE); self.put32(52, 0);
        self.expect(56, OK_NODATA)?;
        self.header(RESOURCE_FLUSH, true);
        self.rect(24, 0, first, self.width, rows);
        self.put32(40, RESOURCE); self.put32(44, 0);
        self.expect(48, OK_NODATA)?;
        Ok(self.fence)
    }

    pub fn scanout_address(&self) -> u64 { self.scanout_va }
    pub fn sequence(&self) -> u64 { self.fence }
}

/// Take the screen over from firmware if BOOT leased a virtio-gpu. `boot_fb`
/// (pitch in pixels) is copied first so the switch shows the same picture.
pub fn open(width: u32, height: u32, boot_fb: u64, boot_pitch: u32) -> Result<Gpu, &'static str> {
    let identity = sex_pdx::device_identity(v::SLOT_GPU).map_err(|_| "no_lease")?;
    if identity.vendor != v::VENDOR || identity.product != v::GPU { return Err("not_gpu"); }
    let view = sex_pdx::memory_region_full_view(v::SLOT_GPU, m::QUERY_VIEW | m::READ | m::WRITE).map_err(|_| "no_view")?;
    let layout = v::layout(v::SLOT_GPU).map_err(|_| "no_layout")?;
    if !layout.valid(view.length) || view.dma_address.is_some() { return Err("layout"); }
    let dma_info = sex_pdx::memory_region_info(sex_pdx::SLOT_DMA_REGION).map_err(|_| "no_dma")?;
    if dma_info.region_id != v::DMA_REGION_ID || dma_info.length != v::DMA_BYTES { return Err("dma_region"); }
    let dma = sex_pdx::memory_region_full_view(sex_pdx::SLOT_DMA_REGION, dma_info.rights).map_err(|_| "no_dma_view")?;
    let pool_pa = dma.dma_address.ok_or("no_dma_address")?;
    let so_info = sex_pdx::memory_region_info(df::SCANOUT_SLOT).map_err(|_| "no_scanout")?;
    if so_info.region_id != df::SCANOUT_REGION_ID { return Err("scanout_region"); }
    let so = sex_pdx::memory_region_full_view(df::SCANOUT_SLOT, so_info.rights).map_err(|_| "no_scanout_view")?;
    let so_pa = so.dma_address.ok_or("no_scanout_address")?;
    let bytes = width as u64 * height as u64 * 4;
    if so.length < bytes { return Err("scanout_small"); }
    sex_pdx::device_command(v::SLOT_GPU, c::NIC_BITS as u64, 0).map_err(|_| "command")?;

    let mut g = Gpu { bar: view.virtual_address, layout, pool_va: dma.virtual_address, pool_pa, qsize: QSIZE, avail_idx: 0, last_used: 0,
        notify: 0, scanout_va: so.virtual_address, width, height, fence: 0,
        heads: [Head::default(); MAX_SCANOUTS], scanouts: 0, edid_len: 0, edid: [0; 256] };
    unsafe { core::ptr::write_bytes(g.pool_va as *mut u8, 0, 0x6000); }

    // Device initialisation (3.1.1).
    g.w8(DEVICE_STATUS, 0);
    for _ in 0..SPIN { if g.r8(DEVICE_STATUS) == 0 { break; } }
    g.w8(DEVICE_STATUS, S_ACK | S_DRIVER);
    g.w32(DEVICE_FEATURE_SELECT, 0);
    let low = g.r32(DEVICE_FEATURE) as u64;
    g.w32(DEVICE_FEATURE_SELECT, 1);
    let offered = low | (g.r32(DEVICE_FEATURE) as u64) << 32;
    if offered & F_VERSION_1 == 0 { return Err("legacy"); }
    let want = F_VERSION_1 | (offered & F_EDID);
    g.w32(DRIVER_FEATURE_SELECT, 0); g.w32(DRIVER_FEATURE, want as u32);
    g.w32(DRIVER_FEATURE_SELECT, 1); g.w32(DRIVER_FEATURE, (want >> 32) as u32);
    g.w8(DEVICE_STATUS, S_ACK | S_DRIVER | S_FEATURES_OK);
    if g.r8(DEVICE_STATUS) & S_FEATURES_OK == 0 { return Err("features"); }
    // Control queue only; the cursor queue (1) comes with the cursor plane (L6).
    g.w16(QUEUE_SELECT, 0);
    let max = g.r16(QUEUE_SIZE);
    if max < 2 { return Err("queue"); }
    let qsize = QSIZE.min(max);
    g.w16(QUEUE_SIZE, qsize);
    g.w64(QUEUE_DESC, pool_pa + CTRL_DESC);
    g.w64(QUEUE_DRIVER, pool_pa + CTRL_AVAIL);
    g.w64(QUEUE_DEVICE, pool_pa + CTRL_USED);
    g.w16(QUEUE_ENABLE, 1);
    let off = g.r16(QUEUE_NOTIFY_OFF) as u32 * layout.notify_multiplier;
    if off + 2 > layout.notify.length { return Err("notify"); }
    g.notify = g.bar + layout.notify.offset as u64 + off as u64;
    g.w8(DEVICE_STATUS, S_ACK | S_DRIVER | S_FEATURES_OK | S_DRIVER_OK);
    g.qsize = qsize;
    g.scanouts = g.cfg32(8).min(MAX_SCANOUTS as u32);

    g.header(GET_DISPLAY_INFO, false);
    if g.submit(HDR as u32)? == OK_DISPLAY_INFO {
        for i in 0..MAX_SCANOUTS {
            let at = HDR as u64 + i as u64 * 24;
            g.heads[i] = Head { width: g.reply32(at + 8), height: g.reply32(at + 12), enabled: g.reply32(at + 16) != 0 };
        }
    }
    if want & F_EDID != 0 {
        g.header(GET_EDID, false);
        g.put32(24, 0); g.put32(28, 0);
        if g.submit(32)? == OK_EDID {
            let n = (g.reply32(24) as usize).min(256);
            for i in 0..n { g.edid[i] = unsafe { g.pool::<u8>(REPLY + 32 + i as u64).read_volatile() }; }
            g.edid_len = n;
        }
    }

    g.header(RESOURCE_CREATE_2D, false);
    g.put32(24, RESOURCE); g.put32(28, FORMAT_XRGB); g.put32(32, width); g.put32(36, height);
    g.expect(40, OK_NODATA)?;
    g.header(RESOURCE_ATTACH_BACKING, false);
    g.put32(24, RESOURCE); g.put32(28, 1);
    g.put64(32, so_pa); g.put32(40, bytes as u32); g.put32(44, 0);
    g.expect(48, OK_NODATA)?;

    // Same picture before the switch: no flash of black.
    if boot_fb != 0 {
        for y in 0..height as u64 {
            unsafe {
                core::ptr::copy_nonoverlapping((boot_fb + y * boot_pitch as u64 * 4) as *const u32,
                    (g.scanout_va + y * width as u64 * 4) as *mut u32, width as usize);
            }
        }
    }
    g.header(TRANSFER_TO_HOST_2D, false);
    g.rect(24, 0, 0, width, height); g.put64(40, 0); g.put32(48, RESOURCE); g.put32(52, 0);
    g.expect(56, OK_NODATA)?;
    g.header(SET_SCANOUT, false);
    g.rect(24, 0, 0, width, height); g.put32(40, 0); g.put32(44, RESOURCE);
    g.expect(48, OK_NODATA)?;
    g.present(0, height)?;
    Ok(g)
}

/// Monitor identity from the EDID base block, without allocation: vendor id,
/// preferred (first detailed) timing and refresh. The full parse lives in
/// `sex-kms` for driver services that have a heap.
fn edid_summary(e: &[u8]) -> Option<([u8; 3], u32, u32, u32)> {
    if e.len() < 128 || e[..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] { return None; }
    if e[..128].iter().fold(0u8, |a, b| a.wrapping_add(*b)) != 0 { return None; }
    let id = u16::from_be_bytes([e[8], e[9]]);
    let letter = |s: u16| b'A' - 1 + (s & 0x1f) as u8;
    let d = &e[54..72];
    let clock = u16::from_le_bytes([d[0], d[1]]) as u64 * 10_000;
    let w = d[2] as u32 | ((d[4] >> 4) as u32) << 8;
    let hb = d[3] as u32 | ((d[4] & 0xf) as u32) << 8;
    let h = d[5] as u32 | ((d[7] >> 4) as u32) << 8;
    let vb = d[6] as u32 | ((d[7] & 0xf) as u32) << 8;
    let total = (w + hb) as u64 * (h + vb) as u64;
    let hz = if total == 0 { 0 } else { ((clock + total / 2) / total) as u32 };
    Some(([letter(id >> 10), letter(id >> 5), letter(id)], w, h, hz))
}

/// Boot receipt: heads, the monitor's EDID, the takeover.
pub fn report(g: &Gpu) {
    for (i, h) in g.heads.iter().enumerate().take(g.scanouts as usize) {
        sex_pdx::serial_record!("[display.kms.head] scanout={} enabled={} width={} height={}", i, h.enabled as u8, h.width, h.height);
    }
    match edid_summary(&g.edid[..g.edid_len]) {
        Some((v, w, h, hz)) => {
            let (wmm, hmm) = sex_pdx::display_boot::edid_size_mm(&g.edid[..g.edid_len]).unwrap_or((0, 0));
            sex_pdx::serial_record!("[display.kms.edid] vendor={} preferred={}x{}@{} size_mm={}x{} bytes={}",
                core::str::from_utf8(&v).unwrap_or("???"), w, h, hz, wmm, hmm, g.edid_len);
        }
        None => sex_pdx::serial_record!("[display.kms.edid] bytes={} valid=0", g.edid_len),
    }
    sex_pdx::serial_record!("[display.kms.virtio] takeover=1 scanouts={} width={} height={} resource={} sequence={} source=boot_copy",
        g.scanouts, g.width, g.height, RESOURCE, g.sequence());
}

/// Flip receipts: the first few, then every 64th, so a gate can count them.
pub fn flip_receipt(g: &Gpu, first: usize, end: usize) {
    let seq = g.sequence();
    if seq <= 4 || seq % 64 == 0 {
        sex_pdx::serial_record!("[display.kms.flip] sequence={} rows={}..{} fenced=1", seq, first, end);
    }
}
