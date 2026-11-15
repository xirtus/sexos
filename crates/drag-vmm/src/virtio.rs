// SPDX-License-Identifier: GPL-3.0-only
//! virtio-blk on the virtio-mmio transport (virtio 1.2, sections 2, 4.2 and 5.2),
//! modern interface only (version 2, VIRTIO_F_VERSION_1), one split virtqueue.
//!
//! Linux finds it from its command line (`virtio_mmio.device=4K@0xd0000000:5`, which
//! [`Blk::cmdline`] writes). Everything the guest hands over (queue addresses,
//! descriptor chains, lengths, sectors) is checked: a chain that loops, leaves the
//! queue, points outside RAM or asks for more than [`MAX_REQUEST`] fails that request
//! (IOERR) or, when even the rings are unreadable, sets DEVICE_NEEDS_RESET and stops
//! the queue. Indirect descriptors and event index are not offered.
use crate::memory::Ram;
use alloc::vec::Vec;

pub const MAGIC: u32 = 0x7472_6976; // "virt"
pub const VENDOR: u32 = 0x4741_5244; // "DRAG"
pub const DEVICE_BLOCK: u32 = 2;
pub const MMIO_SIZE: u64 = 0x1000;
pub const QUEUE_MAX: u16 = 256;
/// Most data bytes one request may move.
pub const MAX_REQUEST: u64 = 1 << 20;
pub const SECTOR: u64 = 512;

pub const F_VERSION_1: u64 = 1 << 32;
pub const BLK_F_RO: u64 = 1 << 5;
pub const BLK_F_FLUSH: u64 = 1 << 9;

pub const STATUS_ACKNOWLEDGE: u32 = 1;
pub const STATUS_DRIVER: u32 = 2;
pub const STATUS_DRIVER_OK: u32 = 4;
pub const STATUS_FEATURES_OK: u32 = 8;
pub const STATUS_NEEDS_RESET: u32 = 0x40;
pub const STATUS_FAILED: u32 = 0x80;

pub const T_IN: u32 = 0;
pub const T_OUT: u32 = 1;
pub const T_FLUSH: u32 = 4;
pub const T_GET_ID: u32 = 8;
pub const S_OK: u8 = 0;
pub const S_IOERR: u8 = 1;
pub const S_UNSUPP: u8 = 2;

const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;
const DESC_INDIRECT: u16 = 4;

/// The disk behind the device: a file the program was granted, or memory.
pub trait Disk {
    /// Size in bytes.
    fn len(&self) -> u64;
    fn read_only(&self) -> bool;
    fn read(&mut self, offset: u64, out: &mut [u8]) -> Result<(), ()>;
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), ()>;
    fn flush(&mut self) -> Result<(), ()>;
}

/// A disk in memory (tests, and a guest with no file).
pub struct MemDisk { pub data: Vec<u8>, pub read_only: bool }
impl Disk for MemDisk {
    fn len(&self) -> u64 { self.data.len() as u64 }
    fn read_only(&self) -> bool { self.read_only }
    fn read(&mut self, offset: u64, out: &mut [u8]) -> Result<(), ()> {
        let s = self.data.get(offset as usize..).and_then(|d| d.get(..out.len())).ok_or(())?;
        out.copy_from_slice(s);
        Ok(())
    }
    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), ()> {
        let s = self.data.get_mut(offset as usize..).and_then(|d| d.get_mut(..data.len())).ok_or(())?;
        s.copy_from_slice(data);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), ()> { Ok(()) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Queue { num: u16, ready: bool, desc: u64, avail: u64, used: u64, last_avail: u16 }

/// Counters for the VMM's log.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats { pub requests: u64, pub read_bytes: u64, pub written_bytes: u64, pub errors: u64 }

pub struct Blk<D: Disk> {
    pub disk: D,
    pub base: u64,
    pub irq: u8,
    status: u32,
    device_features_sel: u32,
    driver_features_sel: u32,
    driver_features: u64,
    queue_sel: u32,
    queue: Queue,
    interrupt_status: u32,
    generation: u32,
    pub stats: Stats,
}

/// One segment of a chain: guest address, length, device-writable.
type Segment = (u64, u32, bool);

impl<D: Disk> Blk<D> {
    pub fn new(disk: D, base: u64, irq: u8) -> Blk<D> {
        Blk { disk, base, irq, status: 0, device_features_sel: 0, driver_features_sel: 0, driver_features: 0, queue_sel: 0,
            queue: Queue::default(), interrupt_status: 0, generation: 0, stats: Stats::default() }
    }

    /// The word Linux needs on its command line to find this device.
    pub fn cmdline(&self, out: &mut Vec<u8>) {
        use core::fmt::Write;
        struct W<'a>(&'a mut Vec<u8>);
        impl core::fmt::Write for W<'_> { fn write_str(&mut self, s: &str) -> core::fmt::Result { self.0.extend_from_slice(s.as_bytes()); Ok(()) } }
        let _ = write!(W(out), "virtio_mmio.device=4K@{:#x}:{}", self.base, self.irq);
    }

    pub fn contains(&self, gpa: u64) -> bool { gpa >= self.base && gpa - self.base < MMIO_SIZE }

    pub fn device_features(&self) -> u64 {
        F_VERSION_1 | BLK_F_FLUSH | if self.disk.read_only() { BLK_F_RO } else { 0 }
    }

    /// The interrupt line's level.
    pub fn irq_level(&self) -> bool { self.interrupt_status != 0 }

    fn reset(&mut self) {
        let (disk_base, irq, stats) = (self.base, self.irq, self.stats);
        self.status = 0; self.device_features_sel = 0; self.driver_features_sel = 0; self.driver_features = 0;
        self.queue_sel = 0; self.queue = Queue::default(); self.interrupt_status = 0;
        self.base = disk_base; self.irq = irq; self.stats = stats;
    }

    fn config(&self) -> [u8; 0x18] {
        let mut c = [0u8; 0x18];
        c[0..8].copy_from_slice(&(self.disk.len() / SECTOR).to_le_bytes());
        c[0x0C..0x10].copy_from_slice(&1u32.to_le_bytes()); // seg_max
        c[0x14..0x18].copy_from_slice(&(SECTOR as u32).to_le_bytes()); // blk_size
        c
    }

    /// A guest read of `size` (1, 2, 4 or 8) bytes at `offset` in the window.
    pub fn read(&mut self, offset: u64, size: u8) -> u64 {
        if offset >= 0x100 {
            let c = self.config();
            let at = (offset - 0x100) as usize;
            let mut v = [0u8; 8];
            for (i, b) in v.iter_mut().take(size.min(8) as usize).enumerate() { *b = *c.get(at + i).unwrap_or(&0); }
            return u64::from_le_bytes(v);
        }
        if size != 4 || offset % 4 != 0 { return 0; }
        (match offset {
            0x00 => MAGIC,
            0x04 => 2,
            0x08 => DEVICE_BLOCK,
            0x0C => VENDOR,
            0x10 => match self.device_features_sel { 0 => self.device_features() as u32, 1 => (self.device_features() >> 32) as u32, _ => 0 },
            0x34 => if self.queue_sel == 0 { QUEUE_MAX as u32 } else { 0 },
            0x44 => (self.queue_sel == 0 && self.queue.ready) as u32,
            0x60 => self.interrupt_status,
            0x70 => self.status,
            0xFC => self.generation,
            _ => 0,
        }) as u64
    }

    /// A guest write. Returns true when the interrupt line may have changed.
    pub fn write(&mut self, ram: &mut Ram, offset: u64, size: u8, value: u64) -> bool {
        if offset >= 0x100 || size != 4 || offset % 4 != 0 { return false; }
        let v = value as u32;
        let q0 = self.queue_sel == 0;
        let setting_up = !self.queue.ready;
        match offset {
            0x14 => self.device_features_sel = v,
            0x20 if self.status & STATUS_FEATURES_OK == 0 => match self.driver_features_sel {
                0 => self.driver_features = (self.driver_features & !0xFFFF_FFFF) | v as u64,
                1 => self.driver_features = (self.driver_features & 0xFFFF_FFFF) | (v as u64) << 32,
                _ => {}
            },
            0x24 => self.driver_features_sel = v,
            0x30 => self.queue_sel = v,
            0x38 if q0 && setting_up => self.queue.num = v.min(u16::MAX as u32) as u16,
            0x44 if q0 => {
                if v == 1 {
                    let n = self.queue.num;
                    if n == 0 || n > QUEUE_MAX || !n.is_power_of_two() { self.status |= STATUS_NEEDS_RESET; return false; }
                    self.queue.ready = true;
                } else {
                    self.queue.ready = false;
                }
            }
            0x50 => return v == 0 && self.notify(ram),
            0x64 => { self.interrupt_status &= !v; return true; }
            0x70 => {
                if v == 0 { self.reset(); return true; }
                let mut s = v & 0xFF;
                if s & STATUS_FEATURES_OK != 0 && self.status & STATUS_FEATURES_OK == 0 {
                    let ok = self.driver_features & !self.device_features() == 0 && self.driver_features & F_VERSION_1 != 0;
                    if !ok { s &= !STATUS_FEATURES_OK; }
                }
                self.status = s | (self.status & STATUS_NEEDS_RESET);
            }
            0x80 if q0 && setting_up => self.queue.desc = (self.queue.desc & !0xFFFF_FFFF) | v as u64,
            0x84 if q0 && setting_up => self.queue.desc = (self.queue.desc & 0xFFFF_FFFF) | (v as u64) << 32,
            0x90 if q0 && setting_up => self.queue.avail = (self.queue.avail & !0xFFFF_FFFF) | v as u64,
            0x94 if q0 && setting_up => self.queue.avail = (self.queue.avail & 0xFFFF_FFFF) | (v as u64) << 32,
            0xA0 if q0 && setting_up => self.queue.used = (self.queue.used & !0xFFFF_FFFF) | v as u64,
            0xA4 if q0 && setting_up => self.queue.used = (self.queue.used & 0xFFFF_FFFF) | (v as u64) << 32,
            _ => {}
        }
        false
    }

    /// Serves every request the driver made available. True when it raised the interrupt.
    pub fn notify(&mut self, ram: &mut Ram) -> bool {
        if !self.queue.ready || self.status & STATUS_DRIVER_OK == 0 || self.status & STATUS_NEEDS_RESET != 0 { return false; }
        let q = self.queue;
        let n = q.num as u64;
        let mut served = 0u32;
        loop {
            let Ok(avail_idx) = ram.read_u16(q.avail + 2) else { return self.broken() };
            if self.queue.last_avail == avail_idx || served >= q.num as u32 { break; }
            let slot = self.queue.last_avail as u64 % n;
            let Ok(head) = ram.read_u16(q.avail + 4 + 2 * slot) else { return self.broken() };
            let written = self.request(ram, head);
            let Ok(used_idx) = ram.read_u16(q.used + 2) else { return self.broken() };
            let at = q.used + 4 + 8 * (used_idx as u64 % n);
            if ram.write_u32(at, head as u32).is_err() || ram.write_u32(at + 4, written).is_err()
                || ram.write_u16(q.used + 2, used_idx.wrapping_add(1)).is_err() { return self.broken(); }
            self.queue.last_avail = self.queue.last_avail.wrapping_add(1);
            served += 1;
        }
        if served == 0 { return false; }
        let quiet = ram.read_u16(q.avail).map_or(false, |f| f & 1 != 0); // VIRTQ_AVAIL_F_NO_INTERRUPT
        if quiet { return false; }
        self.interrupt_status |= 1;
        true
    }

    fn broken(&mut self) -> bool {
        self.status |= STATUS_NEEDS_RESET;
        self.queue.ready = false;
        self.interrupt_status |= 2; // configuration change: the driver looks at the status
        true
    }

    /// Walks the chain from `head`: at most `num` descriptors, all in the table.
    fn chain(&self, ram: &Ram, head: u16) -> Option<Vec<Segment>> {
        let q = &self.queue;
        let mut out = Vec::new();
        let mut i = head;
        for _ in 0..q.num {
            if i >= q.num { return None; }
            let d = q.desc + 16 * i as u64;
            let (addr, len, flags, next) = (ram.read_u64(d).ok()?, ram.read_u32(d + 8).ok()?, ram.read_u16(d + 12).ok()?, ram.read_u16(d + 14).ok()?);
            if flags & DESC_INDIRECT != 0 { return None; }
            out.push((addr, len, flags & DESC_WRITE != 0));
            if flags & DESC_NEXT == 0 { return Some(out); }
            i = next;
        }
        None // longer than the queue: a loop
    }

    /// One request; returns the bytes written into device-writable buffers.
    fn request(&mut self, ram: &mut Ram, head: u16) -> u32 {
        self.stats.requests += 1;
        let Some(chain) = self.chain(ram, head) else { self.stats.errors += 1; return 0 };
        // Readable segments come first, then writable ones (virtio 2.7.4.2).
        let split = chain.iter().position(|s| s.2).unwrap_or(chain.len());
        if chain[split..].iter().any(|s| !s.2) { self.stats.errors += 1; return 0; }
        let (readable, writable) = chain.split_at(split);
        let total = |segs: &[Segment]| segs.iter().map(|s| s.1 as u64).sum::<u64>();
        let (in_len, out_len) = (total(writable), total(readable));
        let Some((status_addr, data_writable)) = status_byte(writable) else { self.stats.errors += 1; return 0 };
        let mut header = [0u8; 16];
        let status = if gather(ram, readable, 0, &mut header).is_err() {
            S_IOERR
        } else {
            let kind = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            let sector = u64::from_le_bytes(header[8..16].try_into().unwrap_or([0; 8]));
            let data_in = in_len - 1;
            match kind {
                T_IN => self.transfer(ram, &data_writable, sector, data_in, false),
                T_OUT => self.transfer(ram, readable, sector, out_len.saturating_sub(16), true),
                T_FLUSH => if self.disk.flush().is_ok() { S_OK } else { S_IOERR },
                T_GET_ID => {
                    let id = b"drag-virtio-blk\0\0\0\0\0";
                    let n = (data_in as usize).min(id.len());
                    if scatter(ram, &data_writable, &id[..n]).is_ok() { S_OK } else { S_IOERR }
                }
                _ => S_UNSUPP,
            }
        };
        if status != S_OK { self.stats.errors += 1; }
        if ram.write_u8(status_addr, status).is_err() { return 0; }
        in_len.min(u32::MAX as u64) as u32
    }

    /// Moves `len` bytes between the disk at `sector` and the segments (skipping the
    /// 16-byte header first when `out`).
    fn transfer(&mut self, ram: &mut Ram, segs: &[Segment], sector: u64, len: u64, out: bool) -> u8 {
        if len % SECTOR != 0 || len > MAX_REQUEST { return S_IOERR; }
        if out && self.disk.read_only() { return S_IOERR; }
        let Some(start) = sector.checked_mul(SECTOR) else { return S_IOERR };
        match start.checked_add(len) { Some(end) if end <= self.disk.len() => {}, _ => return S_IOERR }
        let mut skip = if out { 16u64 } else { 0 };
        let mut offset = start;
        for &(addr, seg_len, _) in segs {
            let mut seg_len = seg_len as u64;
            let mut addr = addr;
            if skip > 0 {
                let s = skip.min(seg_len);
                skip -= s; seg_len -= s; addr = addr.wrapping_add(s);
            }
            let left = start + len - offset;
            let n = seg_len.min(left);
            if n == 0 { continue; }
            let ok = if out {
                match ram.slice(addr, n as usize) { Ok(b) => self.disk.write(offset, b).is_ok(), Err(_) => false }
            } else {
                match ram.slice_mut(addr, n as usize) { Ok(b) => self.disk.read(offset, b).is_ok(), Err(_) => false }
            };
            if !ok { return S_IOERR; }
            offset += n;
        }
        if out { self.stats.written_bytes += len } else { self.stats.read_bytes += len }
        S_OK
    }
}

/// The status byte (the last writable byte) and the writable segments before it.
fn status_byte(writable: &[Segment]) -> Option<(u64, Vec<Segment>)> {
    let mut segs: Vec<Segment> = writable.iter().copied().filter(|s| s.1 != 0).collect();
    let last = segs.last_mut()?;
    let at = last.0.checked_add(last.1 as u64 - 1)?;
    last.1 -= 1;
    Some((at, segs))
}

fn gather(ram: &Ram, segs: &[Segment], mut skip: u64, out: &mut [u8]) -> Result<(), ()> {
    let mut done = 0;
    for &(addr, len, _) in segs {
        if done == out.len() { break; }
        let len = len as u64;
        if skip >= len { skip -= len; continue; }
        let n = ((len - skip) as usize).min(out.len() - done);
        out[done..done + n].copy_from_slice(ram.slice(addr.wrapping_add(skip), n).map_err(|_| ())?);
        done += n;
        skip = 0;
    }
    if done == out.len() { Ok(()) } else { Err(()) }
}

fn scatter(ram: &mut Ram, segs: &[Segment], data: &[u8]) -> Result<(), ()> {
    let mut done = 0;
    for &(addr, len, _) in segs {
        if done == data.len() { break; }
        let n = (len as usize).min(data.len() - done);
        ram.write(addr, &data[done..done + n]).map_err(|_| ())?;
        done += n;
    }
    if done == data.len() { Ok(()) } else { Err(()) }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloc::vec;

    pub const Q: u64 = 0x10000;
    const AVAIL: u64 = Q + 0x100;
    const USED: u64 = Q + 0x1000;

    pub fn disk(sectors: usize) -> MemDisk {
        let mut data = vec![0u8; sectors * 512];
        for (i, s) in data.chunks_mut(512).enumerate() { s[0] = i as u8; s[1] = b'#'; }
        MemDisk { data, read_only: false }
    }

    /// What a driver does: reset, features, queue 0 of 8, DRIVER_OK.
    pub fn bring_up(b: &mut Blk<MemDisk>, ram: &mut Ram) {
        let w = |b: &mut Blk<MemDisk>, ram: &mut Ram, o: u64, v: u32| { b.write(ram, o, 4, v as u64); };
        w(b, ram, 0x70, 0); w(b, ram, 0x70, 1); w(b, ram, 0x70, 3);
        w(b, ram, 0x24, 1); w(b, ram, 0x20, 1);
        w(b, ram, 0x70, 11);
        assert_eq!(b.read(0x70, 4) as u32 & 8, 8);
        w(b, ram, 0x30, 0); w(b, ram, 0x38, 8);
        w(b, ram, 0x80, Q as u32); w(b, ram, 0x90, AVAIL as u32); w(b, ram, 0xA0, USED as u32);
        w(b, ram, 0x44, 1); w(b, ram, 0x70, 15);
    }

    pub fn desc(ram: &mut Ram, i: u64, addr: u64, len: u32, flags: u16, next: u16) {
        let d = Q + 16 * i;
        ram.write_u64(d, addr).unwrap(); ram.write_u32(d + 8, len).unwrap(); ram.write_u16(d + 12, flags).unwrap(); ram.write_u16(d + 14, next).unwrap();
    }

    pub fn offer(ram: &mut Ram, head: u16) {
        let idx = ram.read_u16(AVAIL + 2).unwrap();
        ram.write_u16(AVAIL + 4 + 2 * (idx as u64 % 8), head).unwrap();
        ram.write_u16(AVAIL + 2, idx.wrapping_add(1)).unwrap();
    }

    fn header(ram: &mut Ram, at: u64, kind: u32, sector: u64) {
        ram.write_u32(at, kind).unwrap(); ram.write_u32(at + 4, 0).unwrap(); ram.write_u64(at + 8, sector).unwrap();
    }

    fn setup() -> (Blk<MemDisk>, Ram) {
        let mut ram = Ram::new(1 << 20).unwrap();
        let mut b = Blk::new(disk(16), 0xD000_0000, 5);
        bring_up(&mut b, &mut ram);
        (b, ram)
    }

    #[test]
    fn identity_registers_and_config() {
        let (mut b, _) = setup();
        assert_eq!((b.read(0, 4), b.read(4, 4), b.read(8, 4)), (MAGIC as u64, 2, 2));
        b.write(&mut Ram::new(4096).unwrap(), 0x14, 4, 1);
        assert_eq!(b.read(0x10, 4), 1, "VERSION_1 in the high word");
        assert_eq!(b.read(0x100, 8), 16, "capacity in sectors");
        assert_eq!(b.read(0x100, 1), 16);
        assert_eq!(b.read(0x114, 4), 512);
        assert_eq!(b.read(0x34, 4), QUEUE_MAX as u64);
        assert_eq!(b.read(0x2, 2), 0, "unaligned register reads are 0");
        let mut out = Vec::new();
        b.cmdline(&mut out);
        assert_eq!(out, b"virtio_mmio.device=4K@0xd0000000:5");
    }

    #[test]
    fn read_write_flush_and_get_id() {
        let (mut b, mut ram) = setup();
        // Read sector 3 the Linux way: header, data, status in three descriptors.
        header(&mut ram, 0x20000, T_IN, 3);
        desc(&mut ram, 0, 0x20000, 16, DESC_NEXT, 1);
        desc(&mut ram, 1, 0x21000, 1024, DESC_NEXT | DESC_WRITE, 2);
        desc(&mut ram, 2, 0x22000, 1, DESC_WRITE, 0);
        offer(&mut ram, 0);
        assert!(b.write(&mut ram, 0x50, 4, 0));
        assert!(b.irq_level());
        assert_eq!(ram.read_u8(0x22000), Ok(S_OK));
        assert_eq!(ram.slice(0x21000, 2).unwrap(), &[3, b'#']);
        assert_eq!(ram.slice(0x21200, 2).unwrap(), &[4, b'#']);
        assert_eq!((ram.read_u16(USED + 2), ram.read_u32(USED + 4), ram.read_u32(USED + 8)), (Ok(1), Ok(0), Ok(1025)));
        assert_eq!(b.read(0x60, 4), 1);
        b.write(&mut ram, 0x64, 4, 1);
        assert!(!b.irq_level());

        // Write sector 15 with header and data in one descriptor.
        header(&mut ram, 0x30000, T_OUT, 15);
        ram.fill(0x30010, 512, b'W').unwrap();
        desc(&mut ram, 3, 0x30000, 16 + 512, DESC_NEXT, 4);
        desc(&mut ram, 4, 0x31000, 1, DESC_WRITE, 0);
        offer(&mut ram, 3);
        b.notify(&mut ram);
        assert_eq!(ram.read_u8(0x31000), Ok(S_OK));
        assert!(b.disk.data[15 * 512..].iter().all(|&c| c == b'W'));

        header(&mut ram, 0x32000, T_FLUSH, 0);
        desc(&mut ram, 5, 0x32000, 16, DESC_NEXT, 6);
        desc(&mut ram, 6, 0x33000, 1, DESC_WRITE, 0);
        offer(&mut ram, 5);
        header(&mut ram, 0x34000, T_GET_ID, 0);
        desc(&mut ram, 7, 0x34000, 16, DESC_NEXT, 1);
        desc(&mut ram, 1, 0x35000, 21, DESC_WRITE, 0);
        offer(&mut ram, 7);
        b.notify(&mut ram);
        assert_eq!(ram.read_u8(0x33000), Ok(S_OK));
        assert_eq!(ram.slice(0x35000, 15).unwrap(), b"drag-virtio-blk");
        assert_eq!(ram.read_u8(0x35014), Ok(S_OK));
        assert_eq!(b.stats.requests, 4);
    }

    #[test]
    fn hostile_requests_fail_cleanly() {
        let (mut b, mut ram) = setup();
        let status = |b: &mut Blk<MemDisk>, ram: &mut Ram, kind: u32, sector: u64, data: u32, at: u64| {
            header(ram, 0x20000, kind, sector);
            ram.write_u8(0x22000, 0xEE).unwrap();
            desc(ram, 0, 0x20000, 16, DESC_NEXT, 1);
            desc(ram, 1, at, data, DESC_NEXT | DESC_WRITE, 2);
            desc(ram, 2, 0x22000, 1, DESC_WRITE, 0);
            offer(ram, 0);
            b.notify(ram);
            ram.read_u8(0x22000).unwrap()
        };
        assert_eq!(status(&mut b, &mut ram, T_IN, 16, 512, 0x21000), S_IOERR, "past the end");
        assert_eq!(status(&mut b, &mut ram, T_IN, u64::MAX, 512, 0x21000), S_IOERR, "sector overflow");
        assert_eq!(status(&mut b, &mut ram, T_IN, 0, 100, 0x21000), S_IOERR, "not whole sectors");
        assert_eq!(status(&mut b, &mut ram, T_IN, 0, 512, 0xFFFF_FFFF_0000), S_IOERR, "buffer outside RAM");
        assert_eq!(status(&mut b, &mut ram, T_IN, 0, 2 << 20, 0x21000), S_IOERR, "too big");
        assert_eq!(status(&mut b, &mut ram, 99, 0, 512, 0x21000), S_UNSUPP);
        b.disk.read_only = true;
        assert_eq!(status(&mut b, &mut ram, T_OUT, 0, 512, 0x21000), S_IOERR, "read-only disk");
        assert_eq!(b.stats.errors, 7);

        // A chain that loops, and one that leaves the table: no status, no hang.
        desc(&mut ram, 0, 0x20000, 16, DESC_NEXT, 1);
        desc(&mut ram, 1, 0x21000, 16, DESC_NEXT, 0);
        offer(&mut ram, 0);
        desc(&mut ram, 2, 0x20000, 16, DESC_NEXT, 200);
        offer(&mut ram, 2);
        offer(&mut ram, 9); // head outside the queue
        desc(&mut ram, 3, 0x20000, 16, DESC_INDIRECT, 0);
        offer(&mut ram, 3);
        b.notify(&mut ram);
        assert_eq!(ram.read_u16(USED + 2), Ok(11), "every request answered");
        assert_eq!(b.stats.errors, 11);

        // A huge avail index: at most one queue's worth per notify.
        ram.write_u16(AVAIL + 2, 0x7FFF).unwrap();
        b.notify(&mut ram);
        assert_eq!(ram.read_u16(USED + 2), Ok(19));
    }

    #[test]
    fn rings_outside_ram_need_reset() {
        let mut ram = Ram::new(1 << 20).unwrap();
        let mut b = Blk::new(disk(4), 0xD000_0000, 5);
        bring_up(&mut b, &mut ram);
        b.write(&mut ram, 0x44, 4, 0);
        b.write(&mut ram, 0x94, 4, 0x10);
        b.write(&mut ram, 0x44, 4, 1);
        assert!(b.notify(&mut ram));
        assert_eq!(b.read(0x70, 4) as u32 & STATUS_NEEDS_RESET, STATUS_NEEDS_RESET);
        assert!(!b.notify(&mut ram), "stopped");
        b.write(&mut ram, 0x70, 4, 0);
        assert_eq!(b.read(0x70, 4), 0, "reset clears it");
    }

    #[test]
    fn feature_negotiation_is_checked() {
        let mut ram = Ram::new(4096).unwrap();
        let mut b = Blk::new(disk(1), 0xD000_0000, 5);
        b.write(&mut ram, 0x70, 4, 3);
        b.write(&mut ram, 0x70, 4, 11); // without VERSION_1
        assert_eq!(b.read(0x70, 4) & 8, 0);
        b.write(&mut ram, 0x24, 4, 0);
        b.write(&mut ram, 0x20, 4, 1 << 20); // a feature we do not offer
        b.write(&mut ram, 0x24, 4, 1);
        b.write(&mut ram, 0x20, 4, 1);
        b.write(&mut ram, 0x70, 4, 11);
        assert_eq!(b.read(0x70, 4) & 8, 0);
        b.write(&mut ram, 0x30, 4, 0);
        b.write(&mut ram, 0x38, 4, 6);
        b.write(&mut ram, 0x44, 4, 1);
        assert_eq!(b.read(0x44, 4), 0, "queue size not a power of two");
    }
}
