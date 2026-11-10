// SPDX-License-Identifier: GPL-3.0-only
//! Intel RST (IMSM) RAID volumes, read only.
//!
//! When the first NVMe drive carries IMSM metadata, it is a RAID member: its
//! raw sectors belong to a volume (often Windows on RAID 0), so nothing is ever
//! written to it. If every member of a RAID 0 or RAID 1 volume is here (the
//! first drive, and the second at `SLOT_NVME_HOST_2`), the block service serves
//! that volume instead of the raw drive: BLOCK_CAPACITY is the volume's size and
//! BLOCK_READ reads volume sectors, strip by strip, from whichever drive holds
//! them. Writes stay refused in this first version.
//!
//! The second drive is driven by the small controller below: polled, one I/O
//! queue, reads through the same bounce lease as the first drive.

use sex_imsm::{Metadata, Volume, SECTOR, TAIL_SECTORS};
use sex_pdx::{serial_println, BLOCK_ERR_BAD_LEN, BLOCK_ERR_NO_DEVICE, BLOCK_MAX_RUN};

#[derive(Clone, Copy, PartialEq, Eq)]
enum State { Plain, Member, Serving }

static mut STATE: State = State::Plain;
static mut VOLUME: Volume = Volume { name: [0; 16], sectors: 0, level: 0, map_state: 0, migrating: false, start: 0,
    per_member: 0, strip: 0, members: 0, order: [0; 8], rebuilding: 0, status: 0 };
/// Which drive (0 = first, 1 = second) holds each member slot.
static mut SLOT_DRIVE: [u8; 8] = [0xFF; 8];
static mut PRESENT: u8 = 0;
static mut SECOND: Controller = Controller::empty();
static mut TAIL: [u8; TAIL_SECTORS * SECTOR] = [0; TAIL_SECTORS * SECTOR];
static mut REFUSALS: u32 = 0;

/// The first drive's raw sectors belong to a RAID volume: never write them.
pub fn writes_refused() -> bool {
    let refused = unsafe { STATE } != State::Plain;
    if refused {
        unsafe {
            if REFUSALS < 8 { serial_println!("[sexdrive.raid.write.refused] reason=raid_member_read_only"); }
            REFUSALS = REFUSALS.saturating_add(1);
        }
    }
    refused
}
pub fn serving() -> bool { unsafe { STATE == State::Serving } }

static mut RAW: bool = false;
/// True while this layer reads the first drive's own sectors.
pub fn raw() -> bool { unsafe { RAW } }
fn first_read(offset: u64, size: u64, dst_va: u64) -> u64 {
    // The member's own sectors, metadata at its end included: past the disk lens too.
    unsafe { RAW = true; super::RAW = true; }
    let status = super::nvme_read_into_mapped_va(offset, size, dst_va);
    unsafe { RAW = false; super::RAW = false; }
    status
}
pub fn sectors() -> u64 { unsafe { VOLUME.sectors } }

fn read_tail(sectors: u64, read: impl Fn(u64, u64, u64) -> u64) -> Option<Metadata> {
    if sectors < TAIL_SECTORS as u64 { return None; }
    let tail = unsafe { &mut *core::ptr::addr_of_mut!(TAIL) };
    let first = sectors - TAIL_SECTORS as u64;
    let status = read(first * SECTOR as u64, tail.len() as u64, tail.as_mut_ptr() as u64);
    if status != 0 {
        serial_println!("[sexdrive.raid.tail.err] status={} lba={}", status, first);
        return None;
    }
    match sex_imsm::from_tail(tail) {
        Ok(m) => Some(m),
        Err(sex_imsm::Error::NotMember) => None,
        Err(e) => {
            // Signature present but unreadable: still somebody's RAID member.
            serial_println!("[sexdrive.raid.metadata.err] error={:?}", e);
            unsafe { STATE = State::Member; }
            None
        }
    }
}

fn text(bytes: &[u8]) -> &str { core::str::from_utf8(bytes).unwrap_or("?") }

/// After the first drive is ready: is it an Intel RAID member, and can the
/// whole volume be read? Returns true when it is a member (raw writes and the
/// write self-tests must not run).
pub fn probe(first_sectors: u64, first_serial: &[u8; 20]) -> bool {
    let Some(meta) = read_tail(first_sectors, |o, s, d| first_read(o, s, d)) else {
        if unsafe { STATE } == State::Member { return true; }
        serial_println!("[sexdrive.raid.none] imsm=0");
        return false;
    };
    unsafe { STATE = State::Member; }
    let version = text(&meta.version);
    serial_println!("[sexdrive.raid.member] imsm=1 version={} family={:#010x} generation={} disks={} volumes={} serial={}",
        version, meta.family, meta.generation, meta.disk_count, meta.volume_count, text(trim(first_serial)));
    for (i, d) in meta.disks().iter().enumerate() {
        serial_println!("[sexdrive.raid.disk] index={} serial={} sectors={} status={:#x}", i, text(d.serial()), d.sectors, d.status);
    }
    for v in meta.volumes() {
        serial_println!("[sexdrive.raid.volume] name={} level={} sectors={} strip_sectors={} members={} start={} map_state={} migrating={}",
            text(v.name()), v.level, v.sectors, v.strip, v.members, v.start, v.map_state, v.migrating as u8);
    }
    let Some(first) = meta.disk_for_serial(first_serial) else {
        serial_println!("[sexdrive.raid.serve] ok=0 reason=first_drive_not_in_disk_table");
        return true;
    };

    let mut drives = [None; 2];
    drives[0] = Some(first);
    let second = unsafe { &mut *core::ptr::addr_of_mut!(SECOND) };
    if second.init(sex_pdx::SLOT_NVME_HOST_2) {
        match read_tail(second.sectors, |o, s, d| second_read(o, s, d)) {
            Some(other) if other.same_set(&meta) => {
                drives[1] = other.disk_for_serial(&second.serial);
                serial_println!("[sexdrive.raid.second] same_set=1 disk={} serial={}",
                    drives[1].map(|d| d as i32).unwrap_or(-1), text(trim(&second.serial)));
            }
            Some(other) => serial_println!("[sexdrive.raid.second] same_set=0 family={:#010x}", other.family),
            None => serial_println!("[sexdrive.raid.second] imsm=0"),
        }
    }

    for v in meta.volumes() {
        let mut present = 0u8;
        let mut slot_drive = [0xFFu8; 8];
        for (drive, disk) in drives.iter().enumerate() {
            let Some(disk) = *disk else { continue };
            if meta.disks[disk].failed() { continue; }
            if let Some(slot) = v.slot_of(disk) { present |= 1 << slot; slot_drive[slot as usize] = drive as u8; }
        }
        match v.readable(present) {
            Ok(()) => {
                unsafe { VOLUME = *v; SLOT_DRIVE = slot_drive; PRESENT = present; STATE = State::Serving; }
                serial_println!("[sexdrive.raid.serve] ok=1 name={} level={} sectors={} present={:#b} writes=0",
                    text(v.name()), v.level, v.sectors, present);
                report_partition_table();
                return true;
            }
            Err(why) => serial_println!("[sexdrive.raid.serve] ok=0 name={} reason={:?} present={:#b}", text(v.name()), why, present),
        }
    }
    true
}

fn trim(serial: &[u8; 20]) -> &[u8] {
    let s = serial.iter().position(|&b| b > 0x20).unwrap_or(20);
    let e = serial.iter().rposition(|&b| b > 0x20).map(|e| e + 1).unwrap_or(s);
    &serial[s..e.max(s)]
}

/// Read volume sectors into `dst_va`, strip by strip.
pub fn read(offset: u64, size: u64, dst_va: u64) -> u64 {
    let sector = SECTOR as u64;
    if size == 0 || size > BLOCK_MAX_RUN || offset % sector != 0 || size % sector != 0 || dst_va == 0 || dst_va == u64::MAX {
        return BLOCK_ERR_BAD_LEN;
    }
    let (volume, present, slot_drive) = unsafe { (VOLUME, PRESENT, SLOT_DRIVE) };
    let (mut lba, mut left, mut at) = (offset / sector, size / sector, dst_va);
    while left > 0 {
        let Some(e) = volume.map(lba, left, present) else { return BLOCK_ERR_BAD_LEN; };
        let bytes = e.sectors * sector;
        let status = match slot_drive[e.member as usize] {
            0 => first_read(e.lba * sector, bytes, at),
            1 => second_read(e.lba * sector, bytes, at),
            _ => BLOCK_ERR_NO_DEVICE,
        };
        if status != 0 {
            serial_println!("[sexdrive.raid.read.err] lba={} member={} member_lba={} sectors={} status={}", lba, e.member, e.lba, e.sectors, status);
            return status;
        }
        lba += e.sectors; left -= e.sectors; at += bytes;
    }
    0
}

fn second_read(offset: u64, size: u64, dst_va: u64) -> u64 {
    unsafe { (*core::ptr::addr_of_mut!(SECOND)).read(offset, size, dst_va) }
}

/// Evidence for a bare-metal log: what is on the volume. Reads the first
/// 16 KiB (the MBR, the GPT header and the first 120 entries) and the backup
/// GPT header in the volume's last sector, so both ends of the mapping are used.
fn report_partition_table() {
    const BYTES: usize = 16 * 1024;
    static mut BUF: [u8; BYTES] = [0; BYTES];
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(BUF) };
    if read(0, BYTES as u64, buf.as_mut_ptr() as u64) != 0 { return; }
    let mbr = buf[510] == 0x55 && buf[511] == 0xAA;
    let gpt = &buf[512..520] == b"EFI PART";
    let mut entries = 0u32;
    if gpt {
        let start = u64::from_le_bytes(buf[512 + 72..512 + 80].try_into().unwrap());
        let count = u32::from_le_bytes(buf[512 + 80..512 + 84].try_into().unwrap());
        let size = u32::from_le_bytes(buf[512 + 84..512 + 88].try_into().unwrap()) as usize;
        if start == 2 && size >= 128 && size <= 512 {
            let room = (BYTES - 1024) / size;
            for i in 0..(count as usize).min(room) {
                let e = &buf[1024 + i * size..1024 + (i + 1) * size];
                if e[..16].iter().all(|&b| b == 0) { continue; }
                entries += 1;
                let mut name = [0u8; 36];
                for (k, c) in e[56..128].chunks_exact(2).enumerate() {
                    name[k] = match (c[0], c[1]) { (0, 0) => 0, (b, 0) if b.is_ascii_graphic() || b == b' ' => b, _ => b'?' };
                }
                let n = name.iter().position(|&b| b == 0).unwrap_or(36);
                let first = u64::from_le_bytes(e[32..40].try_into().unwrap());
                let last = u64::from_le_bytes(e[40..48].try_into().unwrap());
                serial_println!("[sexdrive.raid.partition] index={} first_lba={} sectors={} name={}", i, first, (last + 1).saturating_sub(first), text(&name[..n]));
            }
        }
    }
    let mut backup = false;
    let last = sectors().saturating_sub(1);
    if gpt && read(last * SECTOR as u64, SECTOR as u64, buf.as_mut_ptr() as u64) == 0 {
        backup = &buf[..8] == b"EFI PART" && u64::from_le_bytes(buf[24..32].try_into().unwrap()) == last;
    }
    serial_println!("[sexdrive.raid.volume.read] ok=1 mbr_signature={} gpt={} partitions={} backup_gpt_at_end={}", mbr as u8, gpt as u8, entries, backup as u8);
}

// ── The second drive ───────────────────────────────────────────────────────

const Q: u32 = 16;
const CID_BASE: u16 = 0x8000;

pub struct Controller {
    regs: u64, stride: u64, ready: bool,
    asq: (u64, u64), acq: (u64, u64), sq: (u64, u64), cq: (u64, u64),
    a_tail: u32, a_head: u32, a_phase: u32,
    tail: u32, head: u32, phase: u32, cid: u16,
    pub sectors: u64,
    pub serial: [u8; 20],
}

fn page() -> Option<(u64, u64)> {
    let phys = super::reserve_dma_phys(super::PAGE_SIZE);
    let va = super::own_dma_virtual(phys, super::PAGE_SIZE);
    if phys == 0 || va == 0 || va == u64::MAX { return None; }
    unsafe { core::ptr::write_bytes(va as *mut u8, 0, super::PAGE_SIZE as usize); }
    Some((va, phys))
}

impl Controller {
    const fn empty() -> Self {
        Controller { regs: 0, stride: 4, ready: false, asq: (0, 0), acq: (0, 0), sq: (0, 0), cq: (0, 0),
            a_tail: 0, a_head: 0, a_phase: 1, tail: 0, head: 0, phase: 1, cid: CID_BASE, sectors: 0, serial: [0; 20] }
    }
    fn r32(&self, at: u64) -> u32 { unsafe { core::ptr::read_volatile((self.regs + at) as *const u32) } }
    fn w32(&self, at: u64, v: u32) { unsafe { core::ptr::write_volatile((self.regs + at) as *mut u32, v) } }
    fn w64(&self, at: u64, v: u64) { self.w32(at, v as u32); self.w32(at + 4, (v >> 32) as u32); }

    fn wait_ready(&self, want: bool, ticks: u64) -> bool {
        let start = sex_pdx::get_ticks();
        loop {
            let csts = self.r32(0x1C);
            if csts & 2 != 0 { return false; } // controller fatal status
            if (csts & 1 != 0) == want { return true; }
            if sex_pdx::get_ticks().saturating_sub(start) > ticks { return false; }
            sex_pdx::sys_yield();
        }
    }

    /// Bring up the controller leased at `slot`: reset, admin queue, identify,
    /// one I/O queue pair. False (and logged) on any failure.
    pub fn init(&mut self, slot: u64) -> bool {
        let bytes = match sex_pdx::memory_region_info(slot) {
            Ok(info) if info.length >= 0x2000 => info.length,
            _ => { serial_println!("[sexdrive.nvme2.absent] slot={}", slot); return false; }
        };
        let va: u64;
        unsafe {
            core::arch::asm!("syscall", in("rax") 43u64, in("rdi") slot, in("rsi") 0u64, in("rdx") bytes,
                lateout("rax") va, out("rcx") _, out("r11") _);
        }
        if va == 0 || va == u64::MAX { serial_println!("[sexdrive.nvme2.err] reason=bar_map"); return false; }
        self.regs = va;
        let cap = self.r32(0) as u64 | (self.r32(4) as u64) << 32;
        self.stride = 4 << ((cap >> 32) & 0xF);
        if 0x1000 + 4 * self.stride > bytes { serial_println!("[sexdrive.nvme2.err] reason=doorbells_outside_bar"); return false; }
        // CAP.TO is in 500 ms units; clock ticks are milliseconds.
        let timeout = (((cap >> 24) & 0xFF).max(1)) * 500 + 500;
        if (cap & 0xFFFF) + 1 < Q as u64 { serial_println!("[sexdrive.nvme2.err] reason=queue_too_small mqes={}", cap & 0xFFFF); return false; }
        self.w32(0x0C, u32::MAX); // mask pin interrupts: this driver polls
        if self.r32(0x14) & 1 != 0 { self.w32(0x14, self.r32(0x14) & !1); }
        if !self.wait_ready(false, timeout) { serial_println!("[sexdrive.nvme2.err] reason=disable_timeout"); return false; }
        let (Some(asq), Some(acq), Some(sq), Some(cq), Some(id)) = (page(), page(), page(), page(), page()) else {
            serial_println!("[sexdrive.nvme2.err] reason=dma_pages"); return false;
        };
        (self.asq, self.acq, self.sq, self.cq) = (asq, acq, sq, cq);
        self.w32(0x24, (Q - 1) << 16 | (Q - 1));
        self.w64(0x28, asq.1);
        self.w64(0x30, acq.1);
        self.w32(0x14, 0x0046_0001); // IOCQES 16 B, IOSQES 64 B, 4 KiB pages, NVM command set, enable
        if !self.wait_ready(true, timeout) { serial_println!("[sexdrive.nvme2.err] reason=enable_timeout csts={:#x}", self.r32(0x1C)); return false; }

        if self.admin([0x06, 0, 0, 0, 0, 0, id.1 as u32, (id.1 >> 32) as u32, 0, 0, 1, 0, 0, 0, 0, 0]) != Some(0) {
            serial_println!("[sexdrive.nvme2.err] reason=identify_controller"); return false;
        }
        for i in 0..20 { self.serial[i] = unsafe { core::ptr::read_volatile((id.0 + 4 + i as u64) as *const u8) }; }
        if self.admin([0x06, 1, 0, 0, 0, 0, id.1 as u32, (id.1 >> 32) as u32, 0, 0, 0, 0, 0, 0, 0, 0]) != Some(0) {
            serial_println!("[sexdrive.nvme2.err] reason=identify_namespace"); return false;
        }
        let (size, format) = unsafe { (core::ptr::read_volatile(id.0 as *const u64), core::ptr::read_volatile((id.0 + 26) as *const u8) & 0xF) };
        let shift = unsafe { core::ptr::read_volatile((id.0 + 128 + 4 * format as u64 + 2) as *const u8) };
        if shift != 9 { serial_println!("[sexdrive.nvme2.err] reason=sector_size lba_shift={}", shift); return false; }
        self.sectors = size;
        if self.admin([0x05, 0, 0, 0, 0, 0, cq.1 as u32, (cq.1 >> 32) as u32, 0, 0, (Q - 1) << 16 | 1, 1, 0, 0, 0, 0]) != Some(0)
            || self.admin([0x01, 0, 0, 0, 0, 0, sq.1 as u32, (sq.1 >> 32) as u32, 0, 0, (Q - 1) << 16 | 1, 1 << 16 | 1, 0, 0, 0, 0]) != Some(0) {
            serial_println!("[sexdrive.nvme2.err] reason=io_queue"); return false;
        }
        self.ready = true;
        serial_println!("[sexdrive.nvme2.ready] slot={} sectors={} stride={} serial={}", slot, size, self.stride, text(trim(&self.serial)));
        true
    }

    /// One admin command; Some(status) once it completes.
    fn admin(&mut self, mut cmd: [u32; 16]) -> Option<u32> {
        let cid = self.cid; self.cid = self.cid.wrapping_add(1) | CID_BASE;
        cmd[0] |= (cid as u32) << 16;
        let sqe = (self.asq.0 + self.a_tail as u64 * 64) as *mut u32;
        for (i, w) in cmd.iter().enumerate() { unsafe { core::ptr::write_volatile(sqe.add(i), *w); } }
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
        self.a_tail = (self.a_tail + 1) % Q;
        self.w32(0x1000, self.a_tail);
        let wait = super::nvme_wait_completion(self.acq.0, self.a_head, cid, self.a_phase);
        let done = wait.result.ok()?;
        self.a_head = (self.a_head + 1) % Q;
        if self.a_head == 0 { self.a_phase ^= 1; }
        self.w32(0x1000 + self.stride, self.a_head);
        Some((done.dw3 >> 17) & 0x7FFF)
    }

    pub fn read(&mut self, offset: u64, size: u64, dst_va: u64) -> u64 {
        let sector = SECTOR as u64;
        if !self.ready { return BLOCK_ERR_NO_DEVICE; }
        if size == 0 || size > BLOCK_MAX_RUN || offset % sector != 0 || size % sector != 0 { return BLOCK_ERR_BAD_LEN; }
        let cid = self.cid; self.cid = self.cid.wrapping_add(1) | CID_BASE;
        let Some(mut transfer) = super::prepare_bounce(false, cid) else { return BLOCK_ERR_NO_DEVICE };
        if transfer.buffer.length < size { return BLOCK_ERR_NO_DEVICE; }
        let Some(prp2) = super::prp2_for(&transfer.buffer, size) else { return BLOCK_ERR_BAD_LEN };
        let (phys, va) = (transfer.buffer.physical_address, transfer.buffer.virtual_address);
        let lba = offset / sector;
        let cmd = [0x02 | (cid as u32) << 16, 1, 0, 0, 0, 0, phys as u32, (phys >> 32) as u32, prp2 as u32, (prp2 >> 32) as u32,
            lba as u32, (lba >> 32) as u32, (size / sector - 1) as u32, 0, 0, 0];
        let sqe = (self.sq.0 + self.tail as u64 * 64) as *mut u32;
        for (i, w) in cmd.iter().enumerate() { unsafe { core::ptr::write_volatile(sqe.add(i), *w); } }
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
        self.tail = (self.tail + 1) % Q;
        if transfer.submit().is_err() { return BLOCK_ERR_NO_DEVICE; }
        self.w32(0x1000 + 2 * self.stride, self.tail);
        let wait = super::nvme_wait_completion(self.cq.0, self.head, cid, self.phase);
        let Ok(done) = wait.result else {
            self.ready = false;
            serial_println!("[sexdrive.nvme2.read.err] reason=timeout cid={} lba={}", cid, lba);
            return BLOCK_ERR_NO_DEVICE;
        };
        if transfer.complete((done.dw3 & 0xFFFF) as u16, (done.dw3 >> 16) & 1, self.phase).is_err() { return BLOCK_ERR_NO_DEVICE; }
        self.head = (self.head + 1) % Q;
        if self.head == 0 { self.phase ^= 1; }
        self.w32(0x1000 + 3 * self.stride, self.head);
        let status = (done.dw3 >> 17) & 0x7FFF;
        if status != 0 {
            serial_println!("[sexdrive.nvme2.read.err] reason=status status={:#x} lba={}", status, lba);
            return BLOCK_ERR_NO_DEVICE;
        }
        unsafe { core::ptr::copy_nonoverlapping(va as *const u8, dst_va as *mut u8, size as usize); }
        0
    }
}
