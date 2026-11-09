// SPDX-License-Identifier: GPL-3.0-only
//! AHCI SATA disk behind the drive service's BOOT lease at `SLOT_AHCI_HOST`.
//!
//! The service sees only the leased ABAR view and its own DMA pool: no BDF, no
//! physical address it did not get from that pool. One port, one command slot,
//! one command at a time; every wait has a tick deadline. The caller owns the
//! data bounce buffer (and its DmaTransfer guard); this module owns only the
//! command list, received-FIS area and command table, all in one pool page.
//!
//! Out of scope (refused, not guessed at): port multipliers, ATAPI and SEMB
//! devices, logical sectors other than 512 bytes, NCQ, hot plug, interrupts.
use core::sync::atomic::{compiler_fence, Ordering};
use sex_pdx::pci_command::{NIC_BITS, SLOT_AHCI_HOST};
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};

// HBA (global) registers.
const CAP: u64 = 0x00;
const GHC: u64 = 0x04;
const PI: u64 = 0x0C;
const VS: u64 = 0x10;
const CAP2: u64 = 0x24;
const BOHC: u64 = 0x28;
const GHC_AE: u32 = 1 << 31;
const GHC_IE: u32 = 1 << 1;
const CAP_S64A: u32 = 1 << 31;
const CAP_SSS: u32 = 1 << 27;
const CAP2_BOH: u32 = 1 << 0;
const BOHC_BOS: u32 = 1 << 0;
const BOHC_OOS: u32 = 1 << 1;
const BOHC_BB: u32 = 1 << 4;

// Port registers, relative to 0x100 + 0x80 * port.
const PORT_BASE: u64 = 0x100;
const PORT_STRIDE: u64 = 0x80;
const P_CLB: u64 = 0x00;
const P_CLBU: u64 = 0x04;
const P_FB: u64 = 0x08;
const P_FBU: u64 = 0x0C;
const P_IS: u64 = 0x10;
const P_IE: u64 = 0x14;
const P_CMD: u64 = 0x18;
const P_TFD: u64 = 0x20;
const P_SIG: u64 = 0x24;
const P_SSTS: u64 = 0x28;
const P_SCTL: u64 = 0x2C;
const P_SERR: u64 = 0x30;
const P_CI: u64 = 0x38;
const CMD_ST: u32 = 1 << 0;
const CMD_SUD: u32 = 1 << 1;
const CMD_POD: u32 = 1 << 2;
const CMD_FRE: u32 = 1 << 4;
const CMD_FR: u32 = 1 << 14;
const CMD_CR: u32 = 1 << 15;
const IS_TFES: u32 = 1 << 30;
const TFD_BSY: u32 = 0x80;
const TFD_DRQ: u32 = 0x08;
const TFD_ERR: u32 = 0x01;

// Device signatures (PxSIG) this driver must not treat as a disk. An ATA disk
// reads 0x101; an unknown signature still has to answer IDENTIFY DEVICE.
const SIG_ATAPI: u32 = 0xEB14_0101;
const SIG_SEMB: u32 = 0xC33C_0101;
const SIG_PM: u32 = 0x9669_0101;

// ATA commands.
const ATA_IDENTIFY: u8 = 0xEC;
const ATA_READ_DMA_EXT: u8 = 0x25;
const ATA_WRITE_DMA_EXT: u8 = 0x35;
const ATA_FLUSH_CACHE_EXT: u8 = 0xEA;
const FIS_H2D: u8 = 0x27;

// Layout of the one control page: command list (1 KiB aligned, 32 headers),
// received FIS (256 B aligned), one command table (128 B aligned, one PRDT
// entry), and the IDENTIFY data.
const PAGE: u64 = 4096;
const CLB_OFF: u64 = 0x000;
const FB_OFF: u64 = 0x400;
const CT_OFF: u64 = 0x800;
const PRDT_OFF: u64 = 0x80;
const IDENT_OFF: u64 = 0xC00;
const SECTOR: u64 = 512;
/// One PRDT entry moves at most 4 MiB; READ/WRITE DMA EXT at most 65536 sectors.
const MAX_ENTRY_BYTES: u64 = 4 * 1024 * 1024;

/// Ticks are milliseconds. A data command gets as long as Linux gives one (30 s).
const COMMAND_TICKS: u64 = 30_000;
const ENGINE_TICKS: u64 = 500;
const BIOS_HANDOFF_TICKS: u64 = 2_000;
const LINK_TICKS: u64 = 1_000;
const QUICK_TURNS: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure { NotReady, BadArgs, Deadline, TaskFile(u32, u32) }

struct Port {
    ready: bool,
    regs: u64,
    port: u32,
    ctl_va: u64,
    ctl_phys: u64,
    sectors: u64,
    /// Without CAP.S64A every DMA address the HBA sees must be below 4 GiB.
    s64a: bool,
}

static mut PORT: Port = Port { ready: false, regs: 0, port: 0, ctl_va: 0, ctl_phys: 0, sectors: 0, s64a: false };

fn rd(addr: u64) -> u32 { unsafe { core::ptr::read_volatile(addr as *const u32) } }
fn wr(addr: u64, value: u32) { unsafe { core::ptr::write_volatile(addr as *mut u32, value) } }

/// Poll `done` until it holds or `ticks` pass. Yields first, then sleeps whole
/// ticks, as the NVMe completion wait does. Never unbounded.
fn wait_until(ticks: u64, mut done: impl FnMut() -> bool) -> bool {
    let start = sex_pdx::get_ticks();
    let Some(deadline) = start.checked_add(ticks) else { return false };
    let mut turns = 0u32;
    loop {
        for _ in 0..64 { if done() { return true; } }
        let now = sex_pdx::get_ticks();
        if now < start || now >= deadline { return done(); }
        turns += 1;
        if turns < QUICK_TURNS { sex_pdx::sys_yield(); }
        else if sex_pdx::sys_wait_self_ticks(1).is_err() { return done(); }
    }
}

pub fn ready() -> bool { unsafe { PORT.ready } }
pub fn sectors() -> u64 { unsafe { PORT.sectors } }
/// An unresolved or failed command stops all further I/O: there is no port
/// recovery (COMRESET + re-identify) yet, so the DMA pages stay quarantined.
pub fn stop_io() { unsafe { PORT.ready = false; } }

/// Find and start the first SATA disk. `reserve` hands out one page of the
/// service's own DMA pool as (virtual, physical); it is called at most once.
pub fn probe(reserve: impl FnOnce() -> Option<(u64, u64)>) -> bool {
    let view = match sex_pdx::memory_region_full_view(SLOT_AHCI_HOST, QUERY_VIEW | READ | WRITE) {
        Ok(view) if view.length >= PORT_BASE + PORT_STRIDE => view,
        _ => { sex_pdx::serial_record!("[sexdrive.ahci.absent] slot={}", SLOT_AHCI_HOST); return false; }
    };
    let abar = view.virtual_address;
    if abar == 0 || abar == u64::MAX { return false; }
    // Memory decode + bus master for this controller only; BOOT checked our pool.
    if let Err(error) = sex_pdx::device_command(SLOT_AHCI_HOST, NIC_BITS as u64, 0) {
        sex_pdx::serial_record!("[sexdrive.ahci.err] reason=command status={:#x}", error); return false;
    }
    let cap = rd(abar + CAP);
    let vs = rd(abar + VS);
    // Take the controller from firmware first (BIOS/OS handoff), bounded.
    if rd(abar + CAP2) & CAP2_BOH != 0 {
        wr(abar + BOHC, rd(abar + BOHC) | BOHC_OOS);
        let released = wait_until(BIOS_HANDOFF_TICKS, || rd(abar + BOHC) & (BOHC_BOS | BOHC_BB) == 0);
        sex_pdx::serial_record!("[sexdrive.ahci.handoff] released={} bohc={:#x}", released as u8, rd(abar + BOHC));
        if !released { return false; }
    }
    // AHCI mode, interrupts off: this driver polls.
    wr(abar + GHC, rd(abar + GHC) | GHC_AE);
    wr(abar + GHC, rd(abar + GHC) & !GHC_IE);
    let pi = rd(abar + PI);
    sex_pdx::serial_record!("[sexdrive.ahci.hba] view_bytes={} cap={:#x} vs={:#x} pi={:#x} s64a={} sss={} ncs={}",
        view.length, cap, vs, pi, (cap & CAP_S64A != 0) as u8, (cap & CAP_SSS != 0) as u8, ((cap >> 8) & 0x1F) + 1);

    // Quiesce every implemented port inside the view: firmware may have left an
    // engine running with its own command list and FIS area in memory it no
    // longer owns. A port we cannot stop is never used.
    let mut stopped = 0u32;
    for n in 0..32u32 {
        if pi & (1 << n) == 0 { continue; }
        let regs = abar + PORT_BASE + PORT_STRIDE * n as u64;
        if PORT_BASE + PORT_STRIDE * (n as u64 + 1) > view.length {
            sex_pdx::serial_record!("[sexdrive.ahci.port.skip] port={} reason=outside_view", n); continue;
        }
        if stop_engines(regs) { stopped |= 1 << n; }
        else { sex_pdx::serial_record!("[sexdrive.ahci.port.skip] port={} reason=engine_stuck", n); }
    }

    let Some((ctl_va, ctl_phys)) = reserve() else {
        sex_pdx::serial_record!("[sexdrive.ahci.err] reason=dma_page"); return false;
    };
    if ctl_va == 0 || ctl_phys == 0 || ctl_phys % PAGE != 0 { return false; }
    if cap & CAP_S64A == 0 && ctl_phys.checked_add(PAGE).map_or(true, |end| end > 1 << 32) {
        sex_pdx::serial_record!("[sexdrive.ahci.err] reason=dma_above_4g_without_s64a"); return false;
    }

    for n in 0..32u32 {
        if stopped & (1 << n) == 0 { continue; }
        let regs = abar + PORT_BASE + PORT_STRIDE * n as u64;
        let ssts = rd(regs + P_SSTS);
        if ssts & 0xF != 3 { continue; } // no device with established communication
        let sig = rd(regs + P_SIG);
        if matches!(sig, SIG_ATAPI | SIG_SEMB | SIG_PM) {
            sex_pdx::serial_record!("[sexdrive.ahci.port.skip] port={} sig={:#x} reason=not_a_disk", n, sig); continue;
        }
        unsafe {
            PORT = Port { ready: false, regs, port: n, ctl_va, ctl_phys, sectors: 0, s64a: cap & CAP_S64A != 0 };
        }
        if !start_port(regs, ctl_va, ctl_phys, cap) {
            // The control page is shared by every candidate: never leave a
            // port receiving FISes into it. One that will not stop ends the probe.
            if stop_engines(regs) { continue; } else { return false; }
        }
        unsafe { PORT.ready = true; }
        match identify() {
            Ok(sectors) => {
                unsafe { PORT.sectors = sectors; }
                sex_pdx::serial_record!("[sexdrive.ahci.ready] port={} sig={:#x} ssts={:#x} sectors={} lba_bytes={}", n, sig, ssts, sectors, SECTOR);
                return true;
            }
            Err(failure) => {
                unsafe { PORT.ready = false; }
                sex_pdx::serial_record!("[sexdrive.ahci.port.skip] port={} sig={:#x} reason=identify {:?}", n, sig, failure);
                // A command that never finished may still own the control page.
                if matches!(failure, Failure::Deadline) || !stop_engines(regs) { return false; }
            }
        }
    }
    sex_pdx::serial_record!("[sexdrive.ahci.no_disk] pi={:#x}", pi);
    false
}

/// Clear ST and wait for CR, then clear FRE and wait for FR (AHCI 1.3.1 §10.1.2).
fn stop_engines(regs: u64) -> bool {
    let cmd = rd(regs + P_CMD);
    if cmd & CMD_ST != 0 { wr(regs + P_CMD, cmd & !CMD_ST); }
    if !wait_until(ENGINE_TICKS, || rd(regs + P_CMD) & CMD_CR == 0) { return false; }
    let cmd = rd(regs + P_CMD);
    if cmd & CMD_FRE != 0 { wr(regs + P_CMD, cmd & !CMD_FRE); }
    wait_until(ENGINE_TICKS, || rd(regs + P_CMD) & CMD_FR == 0)
}

fn start_port(regs: u64, ctl_va: u64, ctl_phys: u64, cap: u32) -> bool {
    unsafe { core::ptr::write_bytes(ctl_va as *mut u8, 0, PAGE as usize); }
    let clb = ctl_phys + CLB_OFF;
    let fb = ctl_phys + FB_OFF;
    wr(regs + P_CLB, clb as u32);
    wr(regs + P_CLBU, if cap & CAP_S64A != 0 { (clb >> 32) as u32 } else { 0 });
    wr(regs + P_FB, fb as u32);
    wr(regs + P_FBU, if cap & CAP_S64A != 0 { (fb >> 32) as u32 } else { 0 });
    wr(regs + P_IE, 0);
    wr(regs + P_SERR, u32::MAX);
    wr(regs + P_IS, u32::MAX);
    compiler_fence(Ordering::SeqCst);
    // Spin up / power on (only meaningful with staggered spin-up), then FIS receive.
    let cmd = rd(regs + P_CMD) | CMD_SUD | CMD_POD;
    wr(regs + P_CMD, cmd);
    wr(regs + P_CMD, cmd | CMD_FRE);
    if !wait_until(ENGINE_TICKS, || rd(regs + P_CMD) & CMD_FR != 0) {
        sex_pdx::serial_record!("[sexdrive.ahci.port.skip] reason=fis_receive_off cmd={:#x}", rd(regs + P_CMD)); return false;
    }
    // The device must be idle before ST is set; if firmware left it busy, one COMRESET.
    if !wait_until(LINK_TICKS, || rd(regs + P_TFD) & (TFD_BSY | TFD_DRQ) == 0) {
        let sctl = rd(regs + P_SCTL) & !0xF;
        wr(regs + P_SCTL, sctl | 1);
        let start = sex_pdx::get_ticks();
        wait_until(2, || sex_pdx::get_ticks() >= start + 2); // DET=1 held for at least 1 ms
        wr(regs + P_SCTL, sctl);
        let linked = wait_until(LINK_TICKS, || rd(regs + P_SSTS) & 0xF == 3)
            && wait_until(LINK_TICKS, || rd(regs + P_TFD) & (TFD_BSY | TFD_DRQ) == 0);
        wr(regs + P_SERR, u32::MAX);
        sex_pdx::serial_record!("[sexdrive.ahci.comreset] ok={} tfd={:#x} ssts={:#x}", linked as u8, rd(regs + P_TFD), rd(regs + P_SSTS));
        if !linked { return false; }
    }
    wr(regs + P_IS, u32::MAX);
    wr(regs + P_CMD, rd(regs + P_CMD) | CMD_ST);
    true
}

/// Build slot 0 for one command and issue it. `on_submit` runs immediately
/// before the device-visible CI write (the DMA guard's submit); if it refuses,
/// nothing is issued. Returns the PRD byte count the HBA reported.
fn issue(command: u8, lba: u64, count: u32, data_phys: u64, bytes: u64, write: bool,
         on_submit: impl FnOnce() -> bool) -> Result<u32, Failure> {
    let (regs, ctl_va, ctl_phys, s64a) = unsafe {
        if !PORT.ready { return Err(Failure::NotReady); }
        (PORT.regs, PORT.ctl_va, PORT.ctl_phys, PORT.s64a)
    };
    if !s64a && data_phys.checked_add(bytes).map_or(true, |end| end > 1 << 32) { return Err(Failure::BadArgs); }
    if bytes > MAX_ENTRY_BYTES || bytes % 2 != 0 || (bytes != 0 && (data_phys == 0 || data_phys % 2 != 0))
        || count > 0x1_0000 || lba >> 48 != 0 {
        return Err(Failure::BadArgs);
    }
    if rd(regs + P_CI) & 1 != 0 || rd(regs + P_TFD) & (TFD_BSY | TFD_DRQ) != 0 {
        stop_io();
        sex_pdx::serial_record!("[sexdrive.ahci.err] reason=port_busy ci={:#x} tfd={:#x}", rd(regs + P_CI), rd(regs + P_TFD));
        return Err(Failure::NotReady);
    }
    let ct = ctl_va + CT_OFF;
    unsafe {
        core::ptr::write_bytes(ct as *mut u8, 0, (PRDT_OFF + 16) as usize);
        let fis = ct as *mut u8;
        let fields: [u8; 16] = [
            FIS_H2D, 0x80, command, 0,
            lba as u8, (lba >> 8) as u8, (lba >> 16) as u8, 0x40,
            (lba >> 24) as u8, (lba >> 32) as u8, (lba >> 40) as u8, 0,
            count as u8, (count >> 8) as u8, 0, 0,
        ];
        for (i, b) in fields.iter().enumerate() { core::ptr::write_volatile(fis.add(i), *b); }
        let prdtl = if bytes == 0 { 0u32 } else {
            let prd = (ct + PRDT_OFF) as *mut u32;
            core::ptr::write_volatile(prd.add(0), data_phys as u32);
            core::ptr::write_volatile(prd.add(1), (data_phys >> 32) as u32);
            core::ptr::write_volatile(prd.add(3), (bytes as u32 - 1) | (1 << 31));
            1
        };
        let header = (ctl_va + CLB_OFF) as *mut u32;
        let ctba = ctl_phys + CT_OFF;
        // CFL = 5 dwords, W for host-to-device data, PRDTL entries.
        core::ptr::write_volatile(header.add(0), 5 | if write { 1 << 6 } else { 0 } | (prdtl << 16));
        core::ptr::write_volatile(header.add(1), 0);
        core::ptr::write_volatile(header.add(2), ctba as u32);
        core::ptr::write_volatile(header.add(3), (ctba >> 32) as u32);
    }
    wr(regs + P_IS, u32::MAX);
    compiler_fence(Ordering::SeqCst);
    if !on_submit() { return Err(Failure::NotReady); }
    wr(regs + P_CI, 1);
    let mut failed = false;
    let done = wait_until(COMMAND_TICKS, || {
        if rd(regs + P_IS) & IS_TFES != 0 { failed = true; return true; }
        rd(regs + P_CI) & 1 == 0
    });
    // Observe completion before the data the device wrote.
    compiler_fence(Ordering::Acquire);
    let tfd = rd(regs + P_TFD);
    if !done {
        stop_io();
        sex_pdx::serial_record!("[sexdrive.ahci.err] reason=deadline cmd={:#x} lba={} ci={:#x} is={:#x} tfd={:#x}", command, lba, rd(regs + P_CI), rd(regs + P_IS), tfd);
        return Err(Failure::Deadline);
    }
    if failed || tfd & (TFD_ERR | TFD_DRQ | TFD_BSY) != 0 {
        // A task-file error ends the command (its DMA is over), but the port
        // needs recovery this driver does not do yet: stop serving.
        // With TFES the HBA leaves CI set; only a stopped command engine (CR
        // clear) proves no more DMA can reach the buffer.
        let serr = rd(regs + P_SERR);
        stop_io();
        let quiet = !failed || stop_engines(regs);
        sex_pdx::serial_record!("[sexdrive.ahci.err] reason=task_file cmd={:#x} lba={} tfd={:#x} serr={:#x} engine_stopped={}", command, lba, tfd, serr, quiet as u8);
        if !quiet { return Err(Failure::Deadline); }
        return Err(Failure::TaskFile(tfd, serr));
    }
    Ok(unsafe { core::ptr::read_volatile(((ctl_va + CLB_OFF) as *const u32).add(1)) })
}

fn identify() -> Result<u64, Failure> {
    let (ctl_va, ctl_phys) = unsafe { (PORT.ctl_va, PORT.ctl_phys) };
    issue(ATA_IDENTIFY, 0, 0, ctl_phys + IDENT_OFF, SECTOR, false, || true)?;
    let word = |i: u64| -> u16 { unsafe { core::ptr::read_volatile((ctl_va + IDENT_OFF + 2 * i) as *const u16) } };
    let lba48 = word(83) & (1 << 10) != 0;
    let sectors = if lba48 {
        (0..4).fold(0u64, |acc, i| acc | (word(100 + i) as u64) << (16 * i))
    } else {
        word(60) as u64 | (word(61) as u64) << 16
    };
    // Logical sector size (word 106 valid, bit 12: longer than 256 words).
    let w106 = word(106);
    let logical = if w106 & 0xC000 == 0x4000 && w106 & (1 << 12) != 0 {
        2 * (word(117) as u64 | (word(118) as u64) << 16)
    } else { SECTOR };
    let flush = word(83) & (1 << 13) != 0;
    sex_pdx::serial_record!("[sexdrive.ahci.identify] lba48={} sectors={} logical_bytes={} flush_ext={}", lba48 as u8, sectors, logical, flush as u8);
    // The block contract and LBA layout are 512-byte sectors; 4Kn disks are refused.
    if !lba48 || logical != SECTOR || sectors == 0 { return Err(Failure::BadArgs); }
    Ok(sectors)
}

/// Read or write `bytes` (whole sectors) at `lba` through the caller's DMA buffer.
pub fn transfer(write: bool, lba: u64, bytes: u64, data_phys: u64, on_submit: impl FnOnce() -> bool) -> Result<(), Failure> {
    if bytes == 0 || bytes % SECTOR != 0 || bytes / SECTOR > 0x1_0000 { return Err(Failure::BadArgs); }
    let count = (bytes / SECTOR) as u32;
    if !lba.checked_add(count as u64).is_some_and(|end| end <= sectors()) { return Err(Failure::BadArgs); }
    let command = if write { ATA_WRITE_DMA_EXT } else { ATA_READ_DMA_EXT };
    issue(command, lba, count & 0xFFFF, data_phys, bytes, write, on_submit).map(|_| ())
}

/// FLUSH CACHE EXT: completes after the drive's volatile write cache is on media.
pub fn flush() -> Result<(), Failure> {
    issue(ATA_FLUSH_CACHE_EXT, 0, 0, 0, 0, false, || true).map(|_| ())
}

pub fn port() -> u32 { unsafe { PORT.port } }
