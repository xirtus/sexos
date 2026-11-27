// SPDX-License-Identifier: GPL-3.0-only
//! Intel Wi-Fi in the network service, stages W2-W4b: from the chip BOOT leased us
//! to the firmware's ALIVE, then through the firmware's init commands to a chip
//! ready to scan, with one boot-log line per step so a first boot on real
//! hardware (the Alienware x17 R1's Killer AX1675x) says exactly how far it got.
//! Everything that can be checked without the chip lives in `sex_wifi`; this file
//! only maps the lease, the DMA pool and the firmware pack onto its traits.
//!
//! Recovery: a lost link (deauthentication, the AP's beacons gone, a firmware
//! crash) or a failed join restarts the chip from the beginning (`csr::stop`, then
//! a fresh bring-up and init), scans, and joins the same network again, with
//! growing pauses between tries. The network's name and passphrase are kept for
//! that in this service's memory only, for this boot (never in a file); a refused
//! handshake (usually a wrong passphrase) or `wifi leave` forgets them. Joining
//! another network, or a scan after a failure, restarts the chip the same way.
extern crate alloc;
use sex_pdx::wifi_status as ws;
use sex_pdx::{memory_region as m, pci_command as c};
use sex_wifi::{bringup::Bringup, context, csr, devices, ieee80211::Security, init::{self, Init}, join::{self, Join}, assoc::{self, Assoc}, rsna::{self, Rsna}, link::{self as wlink, Link}, tx::TxQueue, ieee80211::{self, Bss}, crypto, hcmd, pack::Pack, scan::{self, Scan}, ucode::Firmware, sae};

/// The chip's register window (BAR0 view).
struct Mmio { base: u64, length: u64 }
impl csr::Registers for Mmio {
    fn read(&mut self, offset: u32) -> u32 {
        if offset % 4 != 0 || offset as u64 + 4 > self.length { return u32::MAX; }
        unsafe { ((self.base + offset as u64) as *const u32).read_volatile() }
    }
    fn write(&mut self, offset: u32, value: u32) {
        if offset % 4 != 0 || offset as u64 + 4 > self.length { return; }
        unsafe { ((self.base + offset as u64) as *mut u32).write_volatile(value) }
    }
}

/// The Wi-Fi DMA pool, by byte offset. Out-of-range accesses are dropped (writes)
/// or read as zero, never followed.
struct Pool { base: u64, length: u64 }
impl Pool {
    fn fits(&self, offset: u64, len: u64) -> bool { offset.checked_add(len).is_some_and(|end| end <= self.length) }
}
impl context::Dma for Pool {
    fn write(&mut self, offset: u64, bytes: &[u8]) {
        if !self.fits(offset, bytes.len() as u64) { return; }
        let p = (self.base + offset) as *mut u8;
        for (i, b) in bytes.iter().enumerate() { unsafe { p.add(i).write_volatile(*b) } }
    }
    fn zero(&mut self, offset: u64, len: u64) {
        if !self.fits(offset, len) { return; }
        unsafe { core::ptr::write_bytes((self.base + offset) as *mut u8, 0, len as usize) }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
}
impl context::DmaRead for Pool {
    fn read(&self, offset: u64, out: &mut [u8]) {
        if !self.fits(offset, out.len() as u64) { out.fill(0); return; }
        core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
        let p = (self.base + offset) as *const u8;
        for (i, b) in out.iter_mut().enumerate() { *b = unsafe { p.add(i).read_volatile() } }
    }
}

pub struct Wifi {
    mmio: Mmio,
    pool: Pool,
    bringup: Bringup,
    last: csr::Step,
    /// The firmware file and the chip's platform NVM file, from the read-only pack.
    file: &'static [u8],
    pnvm: Option<&'static [u8]>,
    hw_rev: u32,
    rf_id: u32,
    init: Option<Init>,
    last_init: Option<init::Step>,
    scan: Option<Scan>,
    /// Scans started, the first one included (the `wifi` command's count).
    scans: u16,
    /// Radio enabled (`wifi_status::RADIO`); false disables scanning and links.
    enabled: bool,
    /// Stage W5a: joining a network heard in the last scan.
    join: Option<Join>,
    joined_ssid: [u8; 32],
    joined_ssid_len: usize,
    joined_bss: Option<Bss>,
    joined_rssi: i8,
    joined_channel: u8,
    joined_security: u8,
    /// DHCPv4 address observed in `DHCPACK` on this Wi-Fi link, if any.
    dhcp_ip: Option<[u8; 4]>,
    /// Stage W5b: authentication and association once joined.
    assoc: Option<Assoc>,
    /// Stage W5c: the WPA2 handshake once associated.
    rsna: Option<Rsna>,
    /// The passphrase for the next join (`wifi_status::PASSWORD`), wiped when used.
    password: [u8; ws::PASSWORD_MAX],
    password_len: usize,
    /// Stage W5e: the connection as sexnet's Ethernet link.
    link: Option<Link>,
    /// Frames from the air waiting for the IP stack.
    rx: RxQueue,
    finished: bool,
    /// The network to stay on (name and passphrase), for this boot only.
    saved: Option<Saved>,
    /// A chip restart is due at this tick (after a link loss, a failure, a new join).
    restart_at: Option<u64>,
    /// Join `saved` when the scan after a restart finishes.
    want_join: bool,
    /// Tries since the last connection; pauses grow with it.
    attempts: u8,
    restarts: u32,
    /// No register access until this tick (the chip settling after a reset).
    settle_until: u64,
    /// Why the last join, association or handshake failed, as the `[wifi.failed]`
    /// line says it: Spindle's `wifi` shows it (`wifi_status::WHY`), since the boot
    /// log may have moved past that line by the time anyone asks.
    why: Why,
}

/// The newest failure's text, cut at `ws::WHY_BYTES`.
#[derive(Clone, Copy)]
struct Why { bytes: [u8; ws::WHY_BYTES], len: usize }
impl core::fmt::Write for Why {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() { if self.len < self.bytes.len() { self.bytes[self.len] = b; self.len += 1; } }
        Ok(())
    }
}
impl Why {
    const NONE: Why = Why { bytes: [0; ws::WHY_BYTES], len: 0 };
    fn set(&mut self, args: core::fmt::Arguments<'_>) { *self = Why::NONE; let _ = core::fmt::Write::write_fmt(self, args); }
}

#[derive(Clone, Copy)]
struct Saved { ssid: [u8; 32], ssid_len: usize, password: [u8; ws::PASSWORD_MAX], password_len: usize }
impl Saved { fn ssid(&self) -> &[u8] { &self.ssid[..self.ssid_len] } }

/// Tries to rejoin before giving up (until the next `wifi join`): about three minutes.
const MAX_ATTEMPTS: u8 = 10;
/// Ticks (ms) between tries: 1 s, doubling, at most 30 s.
fn pause(attempt: u8) -> u64 { (1000u64 << attempt.saturating_sub(1).min(5)).min(30_000) }
/// A software reset takes effect within milliseconds (`iwx_sw_reset` waits 5 ms).
const SETTLE_TICKS: u64 = 10;

/// Received Ethernet frames held for the IP stack, oldest first: the air delivers
/// a burst at a time, the stack takes one frame per turn. Full means dropped.
const RX_FRAMES: usize = 32;
/// The chip's .ucode, inflated from the firmware pack (the largest is 1.75 MB).
static mut FIRMWARE: [u8; 2 << 20] = [0; 2 << 20];
/// Stage X2: frames one block-ack session holds until the gap before them fills.
static mut BA_SLOTS: sex_wifi::ba::Slots = sex_wifi::ba::Slots::EMPTY;
static mut LINK_RX: [[u8; wlink::MAX_ETHERNET]; RX_FRAMES] = [[0; wlink::MAX_ETHERNET]; RX_FRAMES];
/// The receive queue, and what IPv4 went by (for `[wifi.link.ip]`: whether DHCP
/// requests leave and answers come back).
struct RxQueue { head: usize, count: usize, len: [u16; RX_FRAMES], overflow: u64, rx_v4: u32, rx_arp: u32, rx_dhcp: u32, tx_v4: u32, tx_arp: u32, tx_dhcp: u32, seen: (u32, u32, u32, u32, u32, u32), unicast_in: u32, tx_full: u32 }
impl RxQueue { const EMPTY: Self = Self { head: 0, count: 0, len: [0; RX_FRAMES], overflow: 0, rx_v4: 0, rx_arp: 0, rx_dhcp: 0, tx_v4: 0, tx_arp: 0, tx_dhcp: 0, seen: (0, 0, 0, 0, 0, 0), unicast_in: 0, tx_full: 0 }; }

/// An Ethernet frame's kind: 1 IPv4, 2 ARP, 3 DHCP (IPv4 UDP to or from port 67/68), 0 other.
fn ip_kind(f: &[u8]) -> u8 {
    if f.len() < 14 { return 0; }
    match u16::from_be_bytes([f[12], f[13]]) {
        0x0806 => 2,
        0x0800 => {
            let ihl = (f.get(14).copied().unwrap_or(0) & 0xf) as usize * 4;
            let udp = 14 + ihl;
            if f.get(23) == Some(&17) && f.len() >= udp + 4 {
                let (src, dst) = (u16::from_be_bytes([f[udp], f[udp + 1]]), u16::from_be_bytes([f[udp + 2], f[udp + 3]]));
                if matches!(src, 67 | 68) || matches!(dst, 67 | 68) { return 3; }
            }
            1
        }
        _ => 0,
    }
}

/// One line per DHCP frame on the Wi-Fi link (the first 24 each way): who it is
/// from and to, its message type (option 53: 1 discover, 2 offer, 3 request,
/// 5 ack, 6 nak), transaction id and client address. For the x17, where replies
/// arrive and no lease follows.
fn log_dhcp(way: &str, f: &[u8]) {
    let ihl = (f.get(14).copied().unwrap_or(0) & 0xf) as usize * 4;
    let (udp, ip) = (14 + ihl, 14);
    let d = udp + 8;
    if f.len() < d + 240 || ihl < 20 {
        sex_pdx::serial_record!("[wifi.link.dhcp] way={} short len={}", way, f.len());
        return;
    }
    let mut kind = 0u8;
    let mut at = d + 240;
    while at + 1 < f.len() {
        match f[at] { 0 => { at += 1; continue; } 255 => break, _ => {} }
        let len = f[at + 1] as usize;
        if f[at] == 53 && len >= 1 && at + 2 < f.len() { kind = f[at + 2]; break; }
        at += 2 + len;
    }
    let m = |a: usize| (f[a], f[a + 1], f[a + 2], f[a + 3], f[a + 4], f[a + 5]);
    let (e, c) = (m(0), m(d + 28));
    let xid = u32::from_be_bytes([f[d + 4], f[d + 5], f[d + 6], f[d + 7]]);
    // Two short lines: `hw` cuts lines at the screen's width.
    sex_pdx::serial_record!("[wifi.link.dhcp] {} type={} xid={:08x} client={:02x}{:02x}{:02x}{:02x}{:02x}{:02x} to={:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        way, kind, xid, c.0, c.1, c.2, c.3, c.4, c.5, e.0, e.1, e.2, e.3, e.4, e.5);
    sex_pdx::serial_record!("[wifi.link.dhcp2] {} {}.{}.{}.{}->{}.{}.{}.{} {}->{} you={}.{}.{}.{} fl={:04x} ip_len={} frame={} sum={:04x}",
        way, f[ip + 12], f[ip + 13], f[ip + 14], f[ip + 15], f[ip + 16], f[ip + 17], f[ip + 18], f[ip + 19],
        u16::from_be_bytes([f[udp], f[udp + 1]]), u16::from_be_bytes([f[udp + 2], f[udp + 3]]),
        f[d + 16], f[d + 17], f[d + 18], f[d + 19], u16::from_be_bytes([f[d + 10], f[d + 11]]),
        u16::from_be_bytes([f[16], f[17]]), f.len(), u16::from_be_bytes([f[udp + 6], f[udp + 7]]));
}

/// Extracts `yiaddr` from a DHCPv4 ACK (`op == 2`, option 53 == 5) Ethernet frame.
fn dhcp_ack_ip(f: &[u8]) -> Option<[u8; 4]> {
    if ip_kind(f) != 3 { return None; }
    let ihl = (*f.get(14)? & 0xf) as usize * 4;
    let d = 14 + ihl + 8;
    if ihl < 20 || f.len() < d + 240 || *f.get(d)? != 2 { return None; }
    let mut at = d + 240;
    while at + 1 < f.len() {
        match f[at] { 0 => { at += 1; continue; } 255 => break, _ => {} }
        let len = f[at + 1] as usize;
        if f[at] == 53 && len >= 1 && f.get(at + 2) == Some(&5) {
            let yi = [f[d + 16], f[d + 17], f[d + 18], f[d + 19]];
            return (yi != [0; 4]).then_some(yi);
        }
        at += 2 + len;
    }
    None
}

const fn security_to_ws(s: Security) -> u8 {
    match s {
        Security::Open => ws::OPEN,
        Security::Wpa2Personal => ws::WPA2,
        Security::Wpa3Personal => ws::WPA3,
        Security::Wpa3Transition => ws::WPA2_WPA3,
        Security::Unsupported => ws::UNSUPPORTED,
    }
}

/// A DHCP client message (UDP 68 -> 67) with the broadcast flag set, copied into
/// `out`, and the UDP checksum cleared (0 = none, allowed for UDP over IPv4).
fn dhcp_broadcast(f: &[u8], out: &mut [u8]) -> Option<usize> {
    if ip_kind(f) != 3 || f.len() > out.len() { return None; }
    let udp = 14 + (f[14] & 0xf) as usize * 4;
    let d = udp + 8;
    if f.len() < d + 12 || u16::from_be_bytes([f[udp], f[udp + 1]]) != 68 || u16::from_be_bytes([f[udp + 2], f[udp + 3]]) != 67 || f[d] != 1 { return None; }
    out[..f.len()].copy_from_slice(f);
    out[d + 10] |= 0x80;
    out[udp + 6] = 0;
    out[udp + 7] = 0;
    Some(f.len())
}

fn view(slot: u64, rights: u8) -> Result<m::RegionView, u64> {
    let info = sex_pdx::memory_region_info(slot)?;
    if info.rights & rights != rights { return Err(c::DENIED); }
    sex_pdx::memory_region_full_view(slot, rights)
}

impl Wifi {
    /// The leased chip, ready to power up; None (after a log line saying why) when
    /// there is none, it is not one we can start, or its firmware is missing.
    pub fn open(now_ticks: u64) -> Option<Wifi> {
        // No lease: no Intel Wi-Fi in this machine (always so under QEMU). Silent.
        let identity = sex_pdx::device_identity(c::SLOT_WIFI).ok()?;
        let (sub_vendor, sub_device) = sex_pdx::device_subsystem(c::SLOT_WIFI).unwrap_or((0, 0));
        let bar = match view(c::SLOT_WIFI, m::QUERY_VIEW | m::READ | m::WRITE) {
            Ok(v) if v.dma_address.is_none() && v.length >= 0x2000 => v,
            other => { sex_pdx::serial_record!("[wifi.lease.bad] product={:#06x} view={}", identity.product, other.is_ok() as u8); return None; }
        };
        // Memory decoding and bus mastering first: firmware may leave them off (the x17
        // R1 does), and then every register reads all ones: a wrong hw_rev and rf_id,
        // and a firmware start that fails at once.
        match sex_pdx::device_command(c::SLOT_WIFI, c::NIC_BITS as u64, 0) {
            Ok(r) if r.after & c::NIC_BITS == c::NIC_BITS => {}
            other => { sex_pdx::serial_record!("[wifi.failed] step=command ok={}", other.is_ok() as u8); return None; }
        }
        let mut mmio = Mmio { base: bar.virtual_address, length: bar.length };
        use csr::Registers;
        let raw_rev = mmio.read(csr::HW_REV);
        let hw_rev = if raw_rev == u32::MAX { u32::MAX } else { devices::normalize_hw_rev(raw_rev) };
        let rf_id = mmio.read(csr::HW_RF_ID);
        sex_pdx::serial_record!("[wifi.chip] vendor={:#06x} device={:#06x} subsystem={:04x}:{:04x} hw_rev={:#x} rf_id={:#x} bar_bytes={}",
            identity.vendor, identity.product, sub_vendor, sub_device, hw_rev, rf_id, bar.length);
        if hw_rev == u32::MAX { sex_pdx::serial_record!("[wifi.failed] step=identify reason=chip_reads_all_ones"); return None; }
        let Some(chip) = devices::identify(identity.product, sub_device, hw_rev, rf_id) else {
            sex_pdx::serial_record!("[wifi.unsupported] device={:#06x} subsystem={:#06x} hw_rev={:#x} rf_id={:#x}", identity.product, sub_device, hw_rev, rf_id);
            return None;
        };
        sex_pdx::serial_record!("[wifi.chip.known] name=\"{}\" family={:?} firmware={} integrated={}", chip.name, chip.family, chip.firmware, chip.integrated as u8);

        let (file, pnvm) = match view(c::SLOT_WIFI_FIRMWARE, m::QUERY_VIEW | m::READ) {
            Ok(v) => {
                let bytes: &'static [u8] = unsafe { core::slice::from_raw_parts(v.virtual_address as *const u8, v.length as usize) };
                // The .ucode is DEFLATE-compressed in the pack: inflate it once into
                // FIRMWARE (bss, never freed: the firmware's sections are read from it).
                let scratch: &'static mut [u8] = unsafe { &mut *core::ptr::addr_of_mut!(FIRMWARE) };
                let mut inflater = alloc::boxed::Box::new(sex_wifi::pack::Inflater::default());
                match Pack::parse(bytes).and_then(|p| p.open(chip.firmware, scratch, &mut inflater).map(|f| (f, chip.pnvm.and_then(|n| p.file(n).ok())))) {
                    Ok(f) => f,
                    Err(e) => { sex_pdx::serial_record!("[wifi.failed] step=firmware file={} reason={:?} hint=run_scripts/fetch_wifi_firmware.sh_before_building", chip.firmware, e); return None; }
                }
            }
            Err(e) => { sex_pdx::serial_record!("[wifi.failed] step=firmware reason=no_pack status={}", e); return None; }
        };
        let fw = match Firmware::parse(file) {
            Ok(fw) => fw,
            Err(e) => { sex_pdx::serial_record!("[wifi.failed] step=firmware file={} reason=parse_{:?}", chip.firmware, e); return None; }
        };
        let image = fw.image();
        sex_pdx::serial_record!("[wifi.firmware] file={} bytes={} version={}.{} api={} lmac={} umac={} paging={} iml={}",
            chip.firmware, file.len(), fw.major, fw.minor, fw.api, image.lmac.len(), image.umac.len(), image.paging.len(), fw.iml.map_or(0, |i| i.len()));

        let dma = match sex_pdx::memory_region_info(c::SLOT_WIFI_DMA) {
            Ok(i) if i.region_id == c::WIFI_DMA_REGION_ID && i.length == c::WIFI_DMA_BYTES && i.peer_pd.is_none() =>
                sex_pdx::memory_region_full_view(c::SLOT_WIFI_DMA, m::QUERY_VIEW | m::READ | m::WRITE | m::DMA_ADDRESS).ok(),
            _ => None,
        };
        let Some((dma, bus)) = dma.and_then(|v| v.dma_address.map(|a| (v, a))) else {
            sex_pdx::serial_record!("[wifi.failed] step=dma reason=no_pool"); return None;
        };
        let mut pool = Pool { base: dma.virtual_address, length: dma.length };
        let bringup = match Bringup::prepare(chip, &fw, &mut pool, bus, dma.length, hw_rev, now_ticks * 1000) {
            Ok(b) => b,
            Err(e) => { sex_pdx::serial_record!("[wifi.failed] step=layout reason={:?} pool={}", e, dma.length); return None; }
        };
        sex_pdx::serial_record!("[wifi.prepared] pool={} used={} rx_pages={} bus={:#x}",
            dma.length, bringup.layout.end, context::RX_BUFFERS, bus);
        Some(Wifi { mmio, pool, bringup, last: csr::Step::Ready, file, pnvm, hw_rev, rf_id, init: None, last_init: None, scan: None, scans: 0, enabled: true, join: None, joined_ssid: [0; 32], joined_ssid_len: 0, joined_bss: None, joined_rssi: 0, joined_channel: 0, joined_security: 0, dhcp_ip: None, assoc: None, rsna: None, password: [0; ws::PASSWORD_MAX], password_len: 0, link: None, rx: RxQueue::EMPTY, finished: false,
            saved: None, restart_at: None, want_join: false, attempts: 0, restarts: 0, settle_until: 0, why: Why::NONE })
    }

    /// True while the chip is busy (starting, scanning, joining) or recovering.
    pub fn pending(&self) -> bool {
        self.enabled && (!self.finished || self.restart_at.is_some() || self.link.as_ref().is_some_and(|l| !l.up()))
    }

    pub fn advance(&mut self, now_ticks: u64) {
        if !self.enabled { return; }
        if let Some(down) = self.link.as_ref().and_then(|l| l.down) { self.link_lost(down, now_ticks); }
        if let Some(at) = self.restart_at {
            if now_ticks >= at { self.restart(now_ticks); }
            return;
        }
        if now_ticks < self.settle_until { return; }
        if self.finished { return; }
        if self.rsna.is_some() { return self.advance_rsna(now_ticks); }
        if self.assoc.is_some() { return self.advance_assoc(now_ticks); }
        if self.join.is_some() { return self.advance_join(now_ticks); }
        if self.scan.is_some() { return self.advance_scan(now_ticks); }
        if self.init.is_some() { return self.advance_init(now_ticks); }
        let more = self.bringup.advance(&mut self.mmio, &mut self.pool, now_ticks * 1000);
        let step = self.bringup.boot.step;
        if step != self.last {
            sex_pdx::serial_record!("[wifi.step] from={:?} to={:?} ticks={}", self.last, step, now_ticks);
            self.last = step;
        }
        if more { return; }
        use csr::Registers;
        match (step, self.bringup.alive) {
            (csr::Step::Alive, Some(a)) => {
                sex_pdx::serial_record!(
                    "[wifi.alive] fw={}.{} umac={}.{} notifications={} error_table={:#x} umac_error_table={:#x} sku={:#x}.{:#x}.{:#x}",
                    a.lmac_major, a.lmac_minor, a.umac_major, a.umac_minor, self.bringup.notifications, a.error_table, a.umac_error_table,
                    a.sku[0], a.sku[1], a.sku[2]);
                // The file parsed in `open`; it is the same read-only bytes.
                let Ok(fw) = Firmware::parse(self.file) else { self.finished = true; return; };
                let i = Init::new(self.bringup.boot.chip, &fw, &a, self.pnvm, &self.bringup.layout, self.bringup.base,
                    &mut self.pool, self.hw_rev, self.rf_id, now_ticks * 1000);
                sex_pdx::serial_record!("[wifi.pnvm] source={:?} sections={} bytes={} version={:#x} file={}",
                    i.pnvm.source, i.pnvm.sections, i.pnvm.bytes, i.pnvm.version, self.bringup.boot.chip.pnvm.unwrap_or("none"));
                self.init = Some(i);
            }
            _ => {
                self.finished = true;
                sex_pdx::serial_record!(
                    // The registers first: a narrow screen still shows them.
                    "[wifi.failed] int={:#x} gp_cntrl={:#x} step={:?} alive_ok={:?} notifications={} bad_rx_buffer={:?} rx_closed={}",
                    self.mmio.read(csr::INT), self.mmio.read(csr::GP_CNTRL),
                    step, self.bringup.alive.map(|a| a.ok), self.bringup.notifications, self.bringup.bad_buffer,
                    self.bringup.rx.closed(&self.pool, &self.bringup.layout));
            }
        }
    }

    /// Stage W3: the firmware's init commands, ending in `[wifi.ready]`.
    fn advance_init(&mut self, now_ticks: u64) {
        let Some(i) = self.init.as_mut() else { return };
        let more = i.advance(&mut self.bringup, &mut self.mmio, &mut self.pool, now_ticks * 1000);
        if self.last_init != Some(i.step) {
            sex_pdx::serial_record!("[wifi.init] step={:?} commands={} answered={} other_notifications={} ticks={}",
                i.step, i.cmdq.sent, i.answered, i.other, now_ticks);
            self.last_init = Some(i.step);
        }
        if more { return; }
        if i.step != init::Step::Ready {
            self.finished = true;
            use csr::Registers;
            sex_pdx::serial_record!("[wifi.failed] step=init reason={:?} commands={} answered={} int={:#x} gp_cntrl={:#x}",
                i.step, i.cmdq.sent, i.answered, self.mmio.read(csr::INT), self.mmio.read(csr::GP_CNTRL));
            return;
        }
        let n = i.nvm.unwrap_or_default();
        let mac = i.mac;
        sex_pdx::serial_record!("[wifi.nvm] version={:#x} sku_flags={:#x} 2ghz={} 5ghz={} ax={} tx_chains={:#x} rx_chains={:#x} lar={} channels={}",
            n.version, n.sku_flags, (n.sku_flags & init::SKU_2GHZ != 0) as u8, (n.sku_flags & init::SKU_5GHZ != 0) as u8,
            (n.sku_flags & init::SKU_11AX != 0) as u8, n.tx_chains, n.rx_chains, n.lar as u8, n.channels);
        let (mcc, channels) = i.regulatory.map_or(([b'-'; 2], n.channels as u32), |r| (r.mcc, r.channels));
        sex_pdx::serial_record!("[wifi.ready] mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} country={}{} channels={} commands={} ticks={}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], mcc[0] as char, mcc[1] as char, channels, i.cmdq.sent, now_ticks);
        // Stage W4: look for networks straight away.
        let sc = i.scan(now_ticks * 1000);
        sex_pdx::serial_record!("[wifi.scan.start] channels={} ticks={}", sc.channels, now_ticks);
        self.scan = Some(sc);
        self.scans = 1;
    }

    /// Stage W4: one passive scan of every allowed channel, then the networks found.
    fn advance_scan(&mut self, now_ticks: u64) {
        let (Some(sc), Some(i)) = (self.scan.as_mut(), self.init.as_mut()) else { return };
        if sc.advance(&mut self.bringup, &mut i.cmdq, &mut self.mmio, &mut self.pool, now_ticks * 1000) { return; }
        self.finished = true;
        if sc.step != scan::Step::Done {
            sex_pdx::serial_record!("[wifi.failed] step=scan reason={:?} frames={} networks={}", sc.step, sc.frames, sc.count);
            if self.want_join {
                self.why.set(format_args!("scan {:?} networks={}", sc.step, sc.count));
                self.retry(now_ticks);
            }
            return;
        }
        for n in sc.networks() {
            let b = &n.bss;
            let ssid = core::str::from_utf8(b.ssid()).unwrap_or("?");
            let security = match b.security {
                Security::Open => "open", Security::Wpa2Personal => "wpa2", Security::Wpa3Personal => "wpa3",
                Security::Wpa3Transition => "wpa2/wpa3", Security::Unsupported => "unsupported",
            };
            sex_pdx::serial_record!("[wifi.network] ssid=\"{}\" bssid={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} channel={} rssi={} security={} beacons={}",
                ssid, b.bssid[0], b.bssid[1], b.bssid[2], b.bssid[3], b.bssid[4], b.bssid[5],
                if b.channel != 0 { b.channel } else { n.heard_on }, n.rssi, security, n.frames);
        }
        sex_pdx::serial_record!("[wifi.scan.done] networks={} frames={} dropped={} channels={} ticks={}",
            sc.count, sc.frames, sc.dropped, sc.channels, now_ticks);
        if self.joined_ssid_len > 0 {
            let cur = &self.joined_ssid[..self.joined_ssid_len];
            if let Some(best) = sc.networks().filter(|n| n.bss.ssid() == cur).max_by_key(|n| n.rssi) {
                self.joined_rssi = best.rssi.clamp(-128, 127) as i8;
            }
        }
        if self.want_join { self.join_saved(now_ticks); }
    }

    /// After a restart's scan: joins the saved network (its strongest access point).
    fn join_saved(&mut self, now_ticks: u64) {
        self.want_join = false;
        let Some(saved) = self.saved else { return };
        let best = self.scan.as_ref().and_then(|sc| sc.networks().enumerate()
            .filter(|(_, n)| n.bss.ssid() == saved.ssid()).max_by_key(|(_, n)| n.rssi).map(|(k, _)| k as u64));
        let Some(index) = best else {
            sex_pdx::serial_record!("[wifi.rejoin] ssid=\"{}\" heard=0 attempt={}", core::str::from_utf8(saved.ssid()).unwrap_or("?"), self.attempts);
            self.why.set(format_args!("not heard in the scan (try {})", self.attempts + 1));
            return self.retry(now_ticks);
        };
        self.password = saved.password;
        self.password_len = saved.password_len;
        if !self.start_join(index, now_ticks) {
            self.why.set(format_args!("could not start the join (try {})", self.attempts + 1));
            self.password = [0; ws::PASSWORD_MAX];
            self.password_len = 0;
            self.retry(now_ticks);
        }
    }

    /// Stops the chip and starts it again from the beginning; a scan follows its init.
    fn restart(&mut self, now_ticks: u64) {
        self.restart_at = None;
        let stopped = csr::stop(&mut self.mmio);
        self.init = None; self.last_init = None; self.scan = None; self.join = None; self.assoc = None; self.rsna = None;
        self.link = None; self.rx = RxQueue::EMPTY; self.joined_bss = None; self.dhcp_ip = None;
        self.password = [0; ws::PASSWORD_MAX];
        self.password_len = 0;
        self.restarts = self.restarts.wrapping_add(1);
        let chip = self.bringup.boot.chip;
        let (base, bytes) = (self.bringup.base, self.pool.length);
        let prepared = Firmware::parse(self.file).ok()
            .map(|fw| Bringup::prepare(chip, &fw, &mut self.pool, base, bytes, self.hw_rev, now_ticks * 1000));
        sex_pdx::serial_record!("[wifi.restart] count={} dma_stopped={} rejoin={} attempt={} ticks={}",
            self.restarts, stopped as u8, self.want_join as u8, self.attempts, now_ticks);
        match prepared {
            Some(Ok(b)) => { self.bringup = b; self.last = csr::Step::Ready; self.finished = false; self.settle_until = now_ticks + SETTLE_TICKS; }
            other => {
                self.finished = true;
                self.want_join = false;
                let reason = if other.is_none() { "firmware" } else { "layout" };
                sex_pdx::serial_record!("[wifi.failed] step=restart reason={}", reason);
                self.why.set(format_args!("chip restart failed: {}", reason));
            }
        }
    }

    /// Schedules another try at the saved network, or gives up after `MAX_ATTEMPTS`
    /// (the chip still restarts, so a scan or a join works again).
    fn retry(&mut self, now_ticks: u64) {
        if self.saved.is_none() { self.want_join = false; self.restart_at = Some(now_ticks); return; }
        self.attempts = self.attempts.saturating_add(1);
        if self.attempts > MAX_ATTEMPTS {
            sex_pdx::serial_record!("[wifi.rejoin.gave_up] attempts={}", self.attempts - 1);
            let mut last = Why::NONE;
            let _ = core::fmt::Write::write_fmt(&mut last, format_args!("gave up after {} tries; last: ", self.attempts - 1));
            for &b in &self.why.bytes[..self.why.len] { if last.len < last.bytes.len() { last.bytes[last.len] = b; last.len += 1; } }
            self.why = last;
            self.want_join = false;
            self.restart_at = Some(now_ticks);
            return;
        }
        let wait = pause(self.attempts);
        sex_pdx::serial_record!("[wifi.retry] attempt={} in_ticks={}", self.attempts, wait);
        self.want_join = true;
        self.restart_at = Some(now_ticks + wait);
    }

    /// The link went down: the IP stack has already let go of it (main loop).
    fn link_lost(&mut self, down: wlink::Down, now_ticks: u64) {
        sex_pdx::serial_record!("[wifi.lost] reason={:?} saved={}", down, self.saved.is_some() as u8);
        // Joined, then dropped: say how, or the retries that follow look unexplained.
        match down {
            wlink::Down::Disconnected(reason) => self.why.set(format_args!("connected, then the AP sent us away (reason {})", reason)),
            wlink::Down::BeaconLoss(missed) => self.why.set(format_args!("connected, then lost the AP ({} beacons missed)", missed)),
            wlink::Down::Failed(f) => self.why.set(format_args!("connected, then the link failed: {:?}", f)),
        }
        self.link = None;
        self.dhcp_ip = None;
        self.attempts = 0;
        self.retry(now_ticks);
    }

    /// A join, association or handshake failed. A refused handshake or SAE exchange
    /// usually means a wrong passphrase: it is forgotten and not tried again.
    fn join_failed(&mut self, wrong_password: bool, now_ticks: u64) {
        if wrong_password {
            sex_pdx::serial_record!("[wifi.passphrase.refused] saved=0");
            self.saved = None;
            self.attempts = 0;
        }
        self.retry(now_ticks);
    }

    /// Stops using Wi-Fi's network: forgets it and restarts the chip (it scans again).
    fn leave(&mut self, now_ticks: u64) {
        sex_pdx::serial_record!("[wifi.leave] connected={}", self.link_up() as u8);
        self.why = Why::NONE;
        self.saved = None;
        self.joined_ssid = [0; 32];
        self.joined_ssid_len = 0;
        self.joined_rssi = 0;
        self.joined_channel = 0;
        self.joined_security = 0;
        self.dhcp_ip = None;
        self.attempts = 0;
        self.want_join = false;
        if self.enabled { self.restart_at = Some(now_ticks); }
    }

    /// Turns the Intel Wi-Fi chip off or back on (`wifi_status::RADIO`).
    fn set_enabled(&mut self, on: bool, now_ticks: u64) {
        if !on {
            if !self.enabled { return; }
            self.enabled = false;
            let _ = csr::stop(&mut self.mmio);
            self.init = None; self.last_init = None; self.scan = None;
            self.join = None; self.assoc = None; self.rsna = None;
            self.link = None; self.rx = RxQueue::EMPTY; self.joined_bss = None;
            self.joined_ssid = [0; 32]; self.joined_ssid_len = 0;
            self.joined_rssi = 0; self.joined_channel = 0; self.joined_security = 0;
            self.dhcp_ip = None;
            self.password = [0; ws::PASSWORD_MAX]; self.password_len = 0;
            self.saved = None; self.restart_at = None; self.want_join = false;
            self.attempts = 0; self.why = Why::NONE; self.finished = true;
            sex_pdx::serial_record!("[wifi.radio] enabled=0");
        } else if !self.enabled {
            self.enabled = true;
            self.why = Why::NONE;
            self.want_join = false;
            self.attempts = 0;
            self.restart(now_ticks);
            sex_pdx::serial_record!("[wifi.radio] enabled=1");
        }
    }

    /// Stage W5a: onto the chosen network's channel (contexts, station, queue).
    fn advance_join(&mut self, now_ticks: u64) {
        let (Some(j), Some(i)) = (self.join.as_mut(), self.init.as_mut()) else { return };
        let before = j.step;
        let more = j.advance(&mut self.bringup, &mut i.cmdq, &mut self.mmio, &mut self.pool, now_ticks * 1000);
        if j.step != before {
            sex_pdx::serial_record!("[wifi.join] step={:?} status={:#x} ticks={}", j.step, j.status, now_ticks);
        }
        if more { return; }
        self.finished = true;
        let ssid = core::str::from_utf8(&self.joined_ssid).unwrap_or("?").trim_end_matches('\0');
        let t = j.target;
        if j.step == join::Step::Joined {
            sex_pdx::serial_record!("[wifi.joined] ssid=\"{}\" bssid={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} channel={} phy={} rlc={} queue={} ticks={}",
                ssid, t.bssid[0], t.bssid[1], t.bssid[2], t.bssid[3], t.bssid[4], t.bssid[5], t.channel,
                j.versions.phy, j.versions.rlc, j.versions.scd_queue, now_ticks);
            if let Some(bss) = self.joined_bss {
                let (mac, family) = (i.mac, self.bringup.boot.chip.family);
                let (versions, rx_antennas) = (j.versions, i.valid_rx_antennas());
                let a = if bss.security == Security::Wpa3Personal {
                    match self.sae_for(&bss, &mac) {
                        Some(sae) => Assoc::with_sae(bss, t, mac, family, now_ticks * 1000, sae, bss.h2e),
                        None => {
                            self.password = [0; ws::PASSWORD_MAX];
                            self.password_len = 0;
                            sex_pdx::serial_record!("[wifi.failed] step=sae reason=no_entropy_or_element");
                            self.why.set(format_args!("sae no_entropy_or_element"));
                            return self.join_failed(false, now_ticks);
                        }
                    }
                } else { Assoc::new(bss, t, mac, family, now_ticks * 1000) };
                let a = a.with_phy(versions, rx_antennas);
                sex_pdx::serial_record!("[wifi.assoc.start] security={:?} step={:?} ticks={}", bss.security, a.step, now_ticks);
                self.assoc = Some(a);
                self.finished = false;
            }
        } else {
            use csr::Registers;
            self.password = [0; ws::PASSWORD_MAX];
            self.password_len = 0;
            sex_pdx::serial_record!("[wifi.failed] step=join reason={:?} ssid=\"{}\" int={:#x}", j.step, ssid, self.mmio.read(csr::INT));
            self.why.set(format_args!("join {:?}", j.step));
            self.join_failed(false, now_ticks);
        }
    }

    fn advance_assoc(&mut self, now_ticks: u64) {
        let (Some(a), Some(i)) = (self.assoc.as_mut(), self.init.as_mut()) else { return };
        let before = a.step;
        let more = a.advance(&mut self.bringup, &mut i.cmdq, &mut self.mmio, &mut self.pool, now_ticks * 1000);
        if a.step != before {
            sex_pdx::serial_record!("[wifi.assoc] step={:?} tries={} frames={} ticks={}", a.step, a.attempts, a.tx.sent, now_ticks);
        }
        if more { return; }
        self.finished = true;
        let ssid = core::str::from_utf8(&self.joined_ssid).unwrap_or("?").trim_end_matches('\0');
        if a.step == assoc::Step::Associated {
            let m = a.mode();
            sex_pdx::serial_record!("[wifi.associated] ssid=\"{}\" aid={} frames={} security={:?} wifi={} width={} qos={} ticks={}",
                ssid, a.aid, a.tx.sent, a.bss.security, m.generation(), m.width.mhz(), m.qos as u8, now_ticks);
            if a.bss.security != ieee80211::Security::Open { self.start_rsna(now_ticks); }
            else if let Some(a) = self.assoc.as_mut() {
                let family = self.bringup.boot.chip.family;
                let tx = core::mem::replace(&mut a.tx, TxQueue::new(family));
                self.rx = RxQueue::EMPTY;
                self.link = Some(Link::new(tx, a.target, a.me(), family, false).with_rate_control(self.tlc_version()).with_block_ack(self.block_ack_command()));
                self.attempts = 0;
                sex_pdx::serial_record!("[wifi.link.up] security=open rate={:#x}", wlink::DATA_RATE);
            }
        } else {
            use csr::Registers;
            self.password = [0; ws::PASSWORD_MAX];
            self.password_len = 0;
            sex_pdx::serial_record!("[wifi.failed] step=assoc reason={:?} ssid=\"{}\" int={:#x}", a.step, ssid, self.mmio.read(csr::INT));
            self.why.set(format_args!("assoc {:?} frames={}", a.step, a.tx.sent));
            // SAE that the AP refuses or never confirms: the passphrase is wrong.
            let sae = a.bss.security == Security::Wpa3Personal && matches!(a.step, assoc::Step::Refused(_));
            self.join_failed(sae, now_ticks);
        }
    }

    /// 32 bytes from the entropy service.
    fn random32() -> Option<[u8; 32]> {
        let mut out = [0u8; 32];
        let plan = sex_pdx::browser_sources::PLANS[0];
        for chunk in out.chunks_exact_mut(8) {
            let mut word = None;
            for _ in 0..64 { if let Ok(w) = sex_pdx::entropy_word(plan.entropy_slot()) { word = Some(w.value); break; } }
            chunk.copy_from_slice(&word?.to_le_bytes());
        }
        Some(out)
    }

    /// WPA3: SAE's password element for this AP and us (hash-to-element when the AP
    /// does it, hunting and pecking otherwise) with fresh random values. The
    /// passphrase stays until association ends (it is wiped then either way).
    fn sae_for(&self, bss: &Bss, me: &[u8; 6]) -> Option<sae::Sae> {
        let password = &self.password[..self.password_len];
        let pwe = if bss.h2e { sae::pwe_from_pt(&sae::derive_pt(bss.ssid(), password, None), me, &bss.bssid) }
            else { sae::pwe_hunt_and_peck(me, &bss.bssid, password).ok()? };
        // Values outside 2..n-1 (vanishingly rare) are drawn again.
        for _ in 0..8 {
            if let Some(s) = sae::Sae::new(pwe, &Self::random32()?, &Self::random32()?) { return Some(s); }
        }
        None
    }

    /// The 4-way handshake after association: the PMK from SAE (WPA3) or from the
    /// passphrase (WPA2), which is then wiped, and a fresh SNonce from the entropy service.
    fn start_rsna(&mut self, now_ticks: u64) {
        let Some(a) = self.assoc.take() else { return };
        let Some(snonce) = Self::random32() else {
            sex_pdx::serial_record!("[wifi.failed] step=handshake reason=no_entropy");
            self.assoc = Some(a);
            return;
        };
        let (pmk, akm) = match a.sae_pmk() {
            Some(pmk) => (pmk, sex_wifi::eapol::Akm::Sae),
            None => (crypto::psk_from_passphrase(&self.password[..self.password_len], a.bss.ssid()), sex_wifi::eapol::Akm::Psk),
        };
        self.password = [0; ws::PASSWORD_MAX];
        self.password_len = 0;
        let mut own = [0u8; 32];
        let own_len = ieee80211::own_rsn(akm == sex_wifi::eapol::Akm::Sae, &mut own);
        let sec_key = Firmware::parse(self.file).ok()
            .and_then(|fw| fw.command_version(hcmd::DATA_PATH_GROUP, rsna::SEC_KEY_CMD)).is_some_and(|v| v.version != 0);
        let r = Rsna::new(pmk, snonce, &own[..own_len], a.bss.rsn_element(), &a.bss.rsn, a.target, a.me(), self.bringup.boot.chip.family, a.tx, sec_key, now_ticks * 1000, akm);
        let mut r = r;
        if let Some((frame, n)) = a.early { r.early(&self.bringup, &mut self.mmio, &mut self.pool, &frame[..n]); }
        sex_pdx::serial_record!("[wifi.handshake.start] akm={:?} sec_key_cmd={} early={} ticks={}", akm, sec_key as u8, a.early.is_some() as u8, now_ticks);
        self.rsna = Some(r);
        self.finished = false;
    }

    fn advance_rsna(&mut self, now_ticks: u64) {
        let (Some(r), Some(i)) = (self.rsna.as_mut(), self.init.as_mut()) else { return };
        let before = (r.step, r.messages, r.installed);
        let more = r.advance(&mut self.bringup, &mut i.cmdq, &mut self.mmio, &mut self.pool, now_ticks * 1000);
        if (r.step, r.messages, r.installed) != before {
            sex_pdx::serial_record!("[wifi.handshake] step={:?} messages={} keys={:?} ticks={}", r.step, r.messages, r.installed, now_ticks);
        }
        if more { return; }
        self.finished = true;
        let ssid = core::str::from_utf8(&self.joined_ssid).unwrap_or("?").trim_end_matches('\0');
        if r.step == rsna::Step::Connected {
            sex_pdx::serial_record!("[wifi.connected] ssid=\"{}\" security=WPA2 ticks={}", ssid, now_ticks);
            self.rx = RxQueue::EMPTY;
            let (version, command) = (self.tlc_version(), self.block_ack_command());
            self.link = self.rsna.take().map(|r| r.into_link().with_rate_control(version).with_block_ack(command));
            self.attempts = 0;
            sex_pdx::serial_record!("[wifi.link.up] security=WPA2 rate={:#x}", wlink::DATA_RATE);
        } else {
            use csr::Registers;
            sex_pdx::serial_record!("[wifi.failed] step=handshake reason={:?} ssid=\"{}\" int={:#x}", r.step, ssid, self.mmio.read(csr::INT));
            self.why.set(format_args!("handshake {:?}", r.step));
            let refused = matches!(r.step, rsna::Step::Refused(_));
            self.join_failed(refused, now_ticks);
        }
    }

    /// Part of the passphrase for the next join.
    fn password_part(&mut self, a: u64, b: u64) -> bool {
        let (k, len) = ((a & 0xff) as usize, ((a >> 8) & 0xff) as usize);
        if k >= ws::PASSWORD_PARTS as usize || !(ws::PASSWORD_MIN..=ws::PASSWORD_MAX).contains(&len) { return false; }
        let mut bytes = [0u8; 7];
        ws::word_bytes(b, &mut bytes);
        let from = k * 7;
        let n = (len.saturating_sub(from)).min(7);
        self.password[from..from + n].copy_from_slice(&bytes[..n]);
        self.password_len = len;
        true
    }

    /// The desktop's join of network `index` of the last scan: remembers it (with the
    /// passphrase just given) and joins at once when the chip is idle, or after a
    /// restart when it is on another network or a join failed.
    fn request_join(&mut self, index: u64, now_ticks: u64) -> bool {
        if !self.enabled { return false; }
        let Some(n) = self.scan.as_ref().and_then(|sc| sc.networks().nth(index as usize)) else { return false };
        if n.bss.security != ieee80211::Security::Open && self.password_len == 0 { return false; }
        if n.bss.security == ieee80211::Security::Unsupported { return false; }
        let mut saved = Saved { ssid: [0; 32], ssid_len: n.bss.ssid().len(), password: self.password, password_len: self.password_len };
        saved.ssid[..saved.ssid_len].copy_from_slice(n.bss.ssid());
        self.joined_ssid = saved.ssid;
        self.joined_ssid_len = saved.ssid_len;
        self.joined_rssi = n.rssi.clamp(-128, 127) as i8;
        self.joined_channel = if n.bss.channel != 0 { n.bss.channel } else { n.heard_on };
        self.joined_security = security_to_ws(n.bss.security);
        self.saved = Some(saved);
        self.attempts = 0;
        if self.start_join(index, now_ticks) { return true; }
        // Busy (connected, joining, failed, or mid-scan): start over and join by name.
        sex_pdx::serial_record!("[wifi.join.switch] ssid=\"{}\" connected={}", core::str::from_utf8(saved.ssid()).unwrap_or("?"), self.link_up() as u8);
        self.want_join = true;
        self.restart_at = Some(now_ticks);
        true
    }

    /// Starts joining network `index` of the last finished scan.
    fn start_join(&mut self, index: u64, now_ticks: u64) -> bool {
        if !self.enabled || !self.finished || self.join.is_some() { return false; }
        let (Some(sc), Some(i)) = (self.scan.as_ref(), self.init.as_ref()) else { return false };
        if sc.step != scan::Step::Done || i.step != init::Step::Ready { return false; }
        let Some(n) = sc.networks().nth(index as usize) else { return false };
        if n.bss.security != ieee80211::Security::Open && self.password_len == 0 { return false; }
        let Ok(fw) = Firmware::parse(self.file) else { return false };
        let versions = join::Versions::of(&fw, &i.features, self.hw_rev);
        let target = join::Target::of(&n.bss, n.heard_on).with_ours(i.ours(&fw));
        self.joined_ssid = [0; 32];
        self.joined_ssid_len = n.bss.ssid().len().min(32);
        self.joined_ssid[..self.joined_ssid_len].copy_from_slice(&n.bss.ssid()[..self.joined_ssid_len]);
        self.joined_bss = Some(n.bss);
        self.joined_rssi = n.rssi.clamp(-128, 127) as i8;
        self.joined_channel = if n.bss.channel != 0 { n.bss.channel } else { n.heard_on };
        self.joined_security = security_to_ws(n.bss.security);
        let j = Join::new(target, versions, i.mac, i.valid_rx_antennas(), now_ticks * 1000);
        let m = target.mode();
        sex_pdx::serial_record!("[wifi.join.start] index={} channel={} step={:?} wifi={} width={} ticks={}", index, target.channel, j.step, m.generation(), m.width.mhz(), now_ticks);
        self.join = Some(j);
        self.finished = false;
        true
    }

    /// TLC_MNG_CONFIG_CMD's version in this firmware (`iwx_rs_init`: 4, else 3).
    fn tlc_version(&self) -> u8 {
        Firmware::parse(self.file).ok().and_then(|fw| fw.command_version(hcmd::DATA_PATH_GROUP, sex_wifi::tlc::TLC_MNG_CONFIG_CMD))
            .map_or(3, |v| if v.version == 4 { 4 } else { 3 })
    }

    /// How block-ack sessions are set up (stage X2): RX_BAID_ALLOCATION_CONFIG_CMD's
    /// version when the firmware sets CAPA_BAID_ML_SUPPORT, else 0 (ADD_STA).
    fn block_ack_command(&self) -> u8 {
        let Ok(fw) = Firmware::parse(self.file) else { return 0 };
        if !fw.has_capa(sex_wifi::ba::CAPA_BAID_ML_SUPPORT) { return 0; }
        fw.command_version(hcmd::DATA_PATH_GROUP, sex_wifi::ba::RX_BAID_ALLOCATION_CONFIG_CMD).map_or(0, |v| v.version)
    }

    /// The link is up: sexnet's IP stack may use it.
    pub fn link_up(&self) -> bool { self.link.as_ref().is_some_and(|l| l.up()) }
    pub fn link_mac(&self) -> Option<[u8; 6]> { self.link.as_ref().map(|l| l.me()) }
    pub fn link_ready(&self) -> bool { self.link.as_ref().is_some_and(|l| l.ready()) }
    /// Logs the link's counters now (`[wifi.link.now]`, `[wifi.link.now2]`): Spindle's
    /// `wifi` shows them, since typing alone can push older lines out of the log's tail.
    pub fn log_link_now(&self) {
        let Some(l) = self.link.as_ref() else { sex_pdx::serial_record!("[wifi.link.now] link=none"); return };
        let q = &self.rx;
        let t = &l.tx;
        let (ssn, last) = t.last.map_or((0, 0), |s| (s.ssn, s.status));
        sex_pdx::serial_record!("[wifi.link.now] up={} ready={} sent={} ok={} failed={} queued={} cur={} hw={} tail_hw={} ssn={} last={:#x} skew={} full={}",
            l.up() as u8, l.ready() as u8, l.sent, l.tx_ok, l.tx_failed, t.queued, t.cur, t.hw, t.tail_hw(), ssn, last, t.ssn_skew, q.tx_full);
        if l.ba_bypass || !l.replay_check { sex_pdx::serial_record!("[wifi.link.now5] tests: ba={} replay={} ba_gave_up={}", if l.ba_bypass { "off" } else { "on" }, if l.replay_check { "on" } else { "off" }, l.ba_gave_up); }
        sex_pdx::serial_record!("[wifi.link.now2] rx={} dropped={} last_drop={:?} overflow={} queued_in={} v4_out={} v4_in={} arp={}/{} unicast_in={} missed_beacons={} ba={} flushes={}",
            l.received, l.dropped, l.last_drop, q.overflow, q.count, q.tx_v4, q.rx_v4, q.tx_arp, q.rx_arp, q.unicast_in, l.missed_beacons, l.ba.is_some() as u8, l.ba_flushes);
        let x = &l.rx_stats;
        sex_pdx::serial_record!("[wifi.link.now6] tid_in={:?} not_decrypted={:?} replays={:?} sw_ok={} sw_fail={}", x.tid_in, x.tid_not_decrypted, x.replays, x.sw_ok, x.sw_fail);
        if let Some(d) = x.last_other {
            let mut hex = [0u8; 2 * sex_wifi::link::MALFORMED_HEAD];
            let k = (d.len as usize).min(sex_wifi::link::MALFORMED_HEAD);
            for (j, b) in d.head[..k].iter().enumerate() { hex[2 * j] = b"0123456789abcdef"[(b >> 4) as usize]; hex[2 * j + 1] = b"0123456789abcdef"[(b & 15) as usize]; }
            sex_pdx::serial_record!("[wifi.link.now7] last_other={:?} tid={} len={} status={:#x} bytes={}", d.kind, d.tid, d.len, d.status, core::str::from_utf8(&hex[..2 * k]).unwrap_or("?"));
        }
        let (status, reorder) = l.last_unicast.unwrap_or((0, 0));
        match l.ba.as_ref() {
            Some(b) => sex_pdx::serial_record!("[wifi.link.now4] ba tid={} window={} head={} stored={} passed={} released={} dropped={} in_a_row={} dup_flagged={} last_status={:#x} reorder={:#x}",
                b.tid, b.window, b.head(), b.stored, b.passed, b.released, b.dropped, b.consecutive_drops, l.dup_flagged, status, reorder),
            None => sex_pdx::serial_record!("[wifi.link.now4] ba=none dup_flagged={} last_status={:#x} reorder={:#x}", l.dup_flagged, status, reorder),
        }
        if let Some(m) = l.malformed {
            // Header, CCMP header and the start of the body, to see which check refused it.
            let mut hex = [0u8; 2 * wlink::MALFORMED_HEAD];
            let k = (m.len as usize).min(wlink::MALFORMED_HEAD);
            for (i, b) in m.head[..k].iter().enumerate() {
                const D: &[u8; 16] = b"0123456789abcdef";
                hex[2 * i] = D[(b >> 4) as usize];
                hex[2 * i + 1] = D[(b & 15) as usize];
            }
            sex_pdx::serial_record!("[wifi.link.now3] malformed={} len={} status={:#x} subframe={:?} bytes={}",
                l.malformed_count, m.len, m.status, m.subframe, core::str::from_utf8(&hex[..2 * k]).unwrap_or("?"));
        }
    }
    /// A field test knob (`wifi_status::TUNE`), for this boot.
    pub fn tune(&mut self, knob: u64, on: bool) {
        let Some(l) = self.link.as_mut() else { return };
        match knob {
            ws::TUNE_BA => l.ba_bypass = !on,
            ws::TUNE_REPLAY => l.replay_check = on,
            _ => return,
        }
        sex_pdx::serial_record!("[wifi.tune] knob={} on={} ba_bypass={} replay_check={}", knob, on as u8, l.ba_bypass as u8, l.replay_check as u8);
    }
    /// Sends one Ethernet frame over the air.
    pub fn link_send(&mut self, ethernet: &[u8]) -> bool {
        let Some(l) = self.link.as_mut() else { return false };
        // DHCP requests ask for broadcast replies (BOOTP flag 0x8000, RFC 2131
        // 4.1). On the x17 the router's broadcast OFFERs to other clients came in
        // but no unicast reply to us ever did; broadcast replies come under the
        // group key, which is known to work here.
        let mut patched = [0u8; wlink::MAX_ETHERNET];
        let ethernet = match dhcp_broadcast(ethernet, &mut patched) { Some(n) => &patched[..n], None => ethernet };
        match l.send(&self.bringup, &mut self.mmio, &mut self.pool, ethernet) {
            Ok(()) => {
                let q = &mut self.rx;
                match ip_kind(ethernet) { 1 => q.tx_v4 += 1, 2 => q.tx_arp += 1, 3 => { q.tx_dhcp += 1; if q.tx_dhcp <= 24 { log_dhcp("out", ethernet); } } _ => {} }
                if l.sent <= 16 || l.sent.is_power_of_two() { sex_pdx::serial_record!("[wifi.link.tx] frames={} length={}", l.sent, ethernet.len()); } true }
            Err(e) => {
                let q = &mut self.rx;
                q.tx_full += 1;
                if q.tx_full <= 8 || q.tx_full.is_power_of_two() { sex_pdx::serial_record!("[wifi.link.tx.failed] error={:?} times={}", e, q.tx_full); }
                false
            }
        }
    }
    /// Reads the air into the receive queue, then offers the oldest queued frame to
    /// `submit` (the IP stack, which takes one frame per turn); it leaves the queue
    /// when `submit` takes it. Returns whether a frame was taken. Logs a link loss.
    pub fn link_receive(&mut self, submit: impl FnOnce(&[u8]) -> bool) -> bool {
        let (Some(l), Some(i)) = (self.link.as_mut(), self.init.as_mut()) else { return false };
        let before = (l.received, l.dropped);
        let keys = (l.rekey_messages, l.rekeyed);
        let rates = (l.rate_control, l.rate);
        let answered = (l.tx_ok, l.tx_failed);
        let block_ack = (l.ba_sessions, l.ba_declined, l.ba_flushes);
        let q = &mut self.rx;
        // Compared with what the last turn saw, not this turn's start: link_send
        // counts between turns, and requests nobody answers must still show.
        let ip = q.seen;
        let mut ack_ip = None;
        // SAFETY: sexnet runs one thread, and only this link touches BA_SLOTS.
        let slots = unsafe { &mut *core::ptr::addr_of_mut!(BA_SLOTS) };
        let up = l.poll_with(&mut self.bringup, &mut i.cmdq, &mut self.mmio, &mut self.pool, sex_pdx::get_ticks() * 1000, Some(slots), |frame| {
            // Frames to us alone (not broadcast or multicast): on the x17 none were seen.
            if frame[0] & 1 == 0 {
                q.unicast_in += 1;
                if q.unicast_in <= 8 || q.unicast_in.is_power_of_two() {
                    sex_pdx::serial_record!("[wifi.link.unicast] in={} type={:02x}{:02x} len={}", q.unicast_in, frame[12], frame[13], frame.len());
                }
            }
            match ip_kind(frame) {
                1 => q.rx_v4 += 1,
                2 => q.rx_arp += 1,
                3 => {
                    q.rx_dhcp += 1;
                    if let Some(yi) = dhcp_ack_ip(frame) { ack_ip = Some(yi); }
                    if q.rx_dhcp <= 24 { log_dhcp("in", frame); }
                }
                _ => {}
            }
            if q.count == RX_FRAMES || frame.len() > wlink::MAX_ETHERNET { q.overflow += 1; return; }
            let at = (q.head + q.count) % RX_FRAMES;
            // SAFETY: sexnet runs one thread, and only this queue touches LINK_RX.
            unsafe { (&mut *core::ptr::addr_of_mut!(LINK_RX))[at][..frame.len()].copy_from_slice(frame); }
            q.len[at] = frame.len() as u16;
            q.count += 1;
        });
        if let Some(yi) = ack_ip { self.dhcp_ip = Some(yi); }
        if !up {
            sex_pdx::serial_record!("[wifi.link.down] reason={:?} sent={} received={} dropped={} overflow={}", l.down, l.sent, l.received, l.dropped, q.overflow);
            *q = RxQueue::EMPTY;
            return false;
        }
        if (l.tx_ok, l.tx_failed) != answered {
            let n = l.tx_ok + l.tx_failed;
            if n <= 16 || n.is_power_of_two() || l.tx_failed != answered.1 && l.tx_failed <= 16 {
                sex_pdx::serial_record!("[wifi.link.txdone] ok={} failed={} last_failure={:#x}", l.tx_ok, l.tx_failed, l.tx_last_failure);
            }
        }
        let now_ip = (q.rx_v4, q.rx_arp, q.rx_dhcp, q.tx_v4, q.tx_arp, q.tx_dhcp);
        if now_ip != ip {
            q.seen = now_ip;
            let n = now_ip.0 + now_ip.1 + now_ip.2 + now_ip.3 + now_ip.4 + now_ip.5;
            if n <= 32 || n.is_power_of_two() || now_ip.2 != ip.2 && now_ip.2 <= 16 {
                sex_pdx::serial_record!("[wifi.link.ip] dhcp_sent={} dhcp_answers={} arp_sent={} arp_in={} v4_sent={} v4_in={}", q.tx_dhcp, q.rx_dhcp, q.tx_arp, q.rx_arp, q.tx_v4, q.rx_v4);
            }
        }
        if (l.rate_control, l.rate) != rates {
            sex_pdx::serial_record!("[wifi.rate] control={:?} rate={:#x}", l.rate_control, l.rate);
        }
        if (l.ba_sessions, l.ba_declined, l.ba_flushes) != block_ack {
            let (tid, window) = l.ba.as_ref().map_or((0xff, 0), |s| (s.tid, s.window));
            sex_pdx::serial_record!("[wifi.ba] sessions={} declined={} flushes={} tid={} window={}", l.ba_sessions, l.ba_declined, l.ba_flushes, tid, window);
        }
        if (l.rekey_messages, l.rekeyed) != keys {
            sex_pdx::serial_record!("[wifi.rekey] messages={} installed={} error={:?}", l.rekey_messages, l.rekeyed, l.rekey_error);
        }
        if l.received != before.0 && (l.received <= 16 || l.received.is_power_of_two()) {
            sex_pdx::serial_record!("[wifi.link.rx] frames={} dropped={} last_drop={:?} overflow={}", l.received, l.dropped, l.last_drop, q.overflow);
        }
        if q.count == 0 { return false; }
        let at = q.head;
        // SAFETY: as above.
        let frame = unsafe { &(&*core::ptr::addr_of!(LINK_RX))[at][..q.len[at] as usize] };
        if !submit(frame) { return false; }
        q.head = (q.head + 1) % RX_FRAMES;
        q.count -= 1;
        true
    }

    /// Whether network `index` of the last scan is secured and no passphrase was given.
    fn needs_password(&self, index: u64) -> bool {
        self.password_len == 0 && self.scan.as_ref().and_then(|sc| sc.networks().nth(index as usize))
            .is_some_and(|n| n.bss.security != ieee80211::Security::Open)
    }

    /// What the desktop sees (`sex_pdx::wifi_status`).
    fn state(&self) -> ws::State {
        if !self.enabled {
            return ws::State { phase: ws::OFF, networks: 0, scans: self.scans, channels: 0 };
        }
        if self.restart_at.is_some() || (self.want_join && !self.finished) {
            let phase = if self.want_join { ws::JOINING } else { ws::STARTING };
            return ws::State { phase, networks: 0, scans: self.scans, channels: 0 };
        }
        if let Some(l) = &self.link {
            let phase = if !l.up() { ws::FAILED } else if l.protected() { ws::CONNECTED } else { ws::ASSOCIATED };
            let (networks, channels) = self.scan.as_ref().map_or((0, 0), |sc| (sc.count.min(255) as u8, sc.channels));
            return ws::State { phase, networks, scans: self.scans, channels };
        }
        if let Some(r) = &self.rsna {
            let phase = match r.step { rsna::Step::Connected if self.link.as_ref().is_some_and(|l| !l.up()) => ws::FAILED, rsna::Step::Connected => ws::CONNECTED, rsna::Step::Handshake => ws::HANDSHAKE, _ => ws::FAILED };
            let (networks, channels) = self.scan.as_ref().map_or((0, 0), |sc| (sc.count.min(255) as u8, sc.channels));
            return ws::State { phase, networks, scans: self.scans, channels };
        }
        if let Some(a) = &self.assoc {
            let phase = match a.step { assoc::Step::Associated if self.link.as_ref().is_some_and(|l| !l.up()) => ws::FAILED, assoc::Step::Associated => ws::ASSOCIATED, assoc::Step::SaeCommit | assoc::Step::SaeConfirm | assoc::Step::Auth | assoc::Step::Assoc | assoc::Step::Phy | assoc::Step::Station | assoc::Step::He | assoc::Step::Mac => ws::ASSOCIATING, _ => ws::FAILED };
            let (networks, channels) = self.scan.as_ref().map_or((0, 0), |sc| (sc.count.min(255) as u8, sc.channels));
            return ws::State { phase, networks, scans: self.scans, channels };
        }
        if let Some(j) = &self.join {
            let phase = match j.step { join::Step::Joined => ws::JOINED, join::Step::Failed(_) => ws::FAILED, _ => ws::JOINING };
            let (networks, channels) = self.scan.as_ref().map_or((0, 0), |sc| (sc.count.min(255) as u8, sc.channels));
            return ws::State { phase, networks, scans: self.scans, channels };
        }
        let phase = match (&self.scan, self.finished) {
            (None, false) => ws::STARTING,
            (None, true) => ws::FAILED,
            (Some(_), false) => ws::SCANNING,
            (Some(sc), true) => if sc.step == scan::Step::Done { ws::READY } else { ws::FAILED },
        };
        let (networks, channels) = self.scan.as_ref().map_or((0, 0), |sc| (sc.count.min(255) as u8, sc.channels));
        ws::State { phase, networks, scans: self.scans, channels }
    }

    /// A fresh scan, when the chip is ready and the last scan has ended.
    fn rescan(&mut self, now_ticks: u64) -> bool {
        if !self.enabled { return false; }
        // After a failed join (or a chip that stopped) a scan needs a fresh chip.
        if self.finished && self.restart_at.is_none() && !self.link_up() && self.link.is_none()
            && (self.join.is_some() || self.init.as_ref().is_none_or(|i| i.step != init::Step::Ready)) {
            self.want_join = false;
            self.restart_at = Some(now_ticks);
            return true;
        }
        let Some(i) = self.init.as_ref() else { return false };
        // Once a join has begun the chip stays on that channel (leaving it is W5b's).
        if !self.finished || self.scan.is_none() || self.join.is_some() || i.step != init::Step::Ready { return false; }
        let mut sc = i.scan(now_ticks * 1000);
        if let Some(earlier) = self.scan.as_ref() { sc.keep_networks(earlier); }
        self.scans = self.scans.wrapping_add(1);
        sex_pdx::serial_record!("[wifi.scan.start] channels={} scan={} ticks={}", sc.channels, self.scans, now_ticks);
        self.scan = Some(sc);
        self.finished = false;
        true
    }

    /// Part `part` of the newest failure's text (`ws::WHY`).
    fn why_part(&self, part: u64) -> u64 {
        let from = (part as usize).saturating_mul(7);
        if part >= ws::WHY_PARTS || from >= self.why.len { return ws::MISSING; }
        ws::bytes_word(&self.why.bytes[from..self.why.len])
    }

    fn active_ssid_slice(&self) -> &[u8] {
        if self.joined_ssid_len > 0 {
            &self.joined_ssid[..self.joined_ssid_len]
        } else if let Some(s) = self.saved.as_ref() {
            s.ssid()
        } else {
            &[]
        }
    }

    fn active_info(&self) -> u64 {
        if !self.enabled { return ws::MISSING; }
        let phase = self.state().phase;
        if !matches!(phase, ws::JOINING | ws::JOINED | ws::ASSOCIATING | ws::ASSOCIATED | ws::HANDSHAKE | ws::CONNECTED) {
            return ws::MISSING;
        }
        let ssid = self.active_ssid_slice();
        if ssid.is_empty() { return ws::MISSING; }
        ws::Info {
            channel: self.joined_channel,
            rssi: self.joined_rssi,
            security: self.joined_security,
            ssid_len: ssid.len().min(32) as u8,
            beacons: 0,
        }.word()
    }

    fn active_ssid_part(&self, part: u64) -> u64 {
        let ssid = self.active_ssid_slice();
        if !self.enabled || ssid.is_empty() || part >= ws::SSID_PARTS { return ws::MISSING; }
        let from = (part * 7) as usize;
        if from >= ssid.len() { return ws::MISSING; }
        ws::bytes_word(&ssid[from..])
    }

    fn network(&self, index: u64, part: u64) -> u64 {
        let Some(n) = self.scan.as_ref().and_then(|sc| sc.networks().nth(index as usize)) else { return ws::MISSING };
        let b = &n.bss;
        let info = ws::Info {
            channel: if b.channel != 0 { b.channel } else { n.heard_on },
            rssi: n.rssi.clamp(-128, 127) as i8, security: security_to_ws(b.security), ssid_len: b.ssid().len() as u8, beacons: n.frames,
        };
        ws::network_part(info, &b.bssid, b.ssid(), part)
    }
}

fn local_ipv4_word(wifi: Option<&Wifi>) -> u64 {
    if let Some(ip) = wifi.and_then(|w| w.dhcp_ip) {
        return ws::ip4_word(ip, true);
    }
    let cfg = sex_pdx::device_network_config(sex_pdx::SLOT_NIC).ok()
        .or_else(|| sex_pdx::device_network_config(sex_pdx::usb_net::SLOT).ok());
    let link_up = wifi.is_some_and(|w| w.link_up()) || cfg.is_some();
    let ip = cfg.map_or([0; 4], |c| c.local);
    ws::ip4_word(ip, link_up)
}

/// Answers a desktop request (`sex_pdx::wifi_status::OP`). Shell, SilkBar and Spindle
/// rows may read; Shell and Spindle may start a scan; programs get `REFUSED`.
pub fn request(wifi: Option<&mut Wifi>, caller: u32, action: u64, a: u64, b: u64, now_ticks: u64) -> u64 {
    let spindle = sex_pdx::app_catalog::SPINDLE_TARGETS.contains(&caller);
    if !(caller == 3 || caller == 6 || spindle) {
        sex_pdx::serial_record!("[wifi.status.denied] caller_pd={}", caller);
        return ws::REFUSED;
    }
    let none = ws::State { phase: ws::NONE, networks: 0, scans: 0, channels: 0 }.word();
    match (action, wifi) {
        (ws::STATE, None) | (ws::SCAN, None) | (ws::RADIO, None) | (ws::JOIN, None) | (ws::LEAVE, None) => none,
        (ws::NETWORK, None) | (ws::WHY, None) => ws::MISSING,
        (ws::ACTIVE, None) => if a == ws::ACTIVE_IP4 { local_ipv4_word(None) } else { ws::MISSING },
        (ws::STATE, Some(w)) => {
            // Spindle's `wifi` (not SilkBar's polling): its counters, for `wifi` to show.
            if spindle { w.log_link_now(); }
            w.state().word()
        }
        (ws::SCAN, Some(w)) if caller != 6 => { w.rescan(now_ticks); w.state().word() }
        (ws::RADIO, Some(w)) if caller != 6 => { w.set_enabled(a != 0, now_ticks); w.state().word() }
        (ws::NETWORK, Some(w)) => w.network(a, b),
        (ws::ACTIVE, Some(w)) => match a {
            ws::ACTIVE_INFO => w.active_info(),
            ws::ACTIVE_IP4 => local_ipv4_word(Some(w)),
            p if p >= ws::ACTIVE_SSID && p < ws::ACTIVE_SSID + ws::SSID_PARTS => w.active_ssid_part(p - ws::ACTIVE_SSID),
            _ => ws::MISSING,
        },
        (ws::JOIN, Some(w)) if caller != 6 => {
            if w.needs_password(a) { return ws::NEEDS_PASSWORD; }
            w.request_join(a, now_ticks);
            w.state().word()
        }
        (ws::LEAVE, Some(w)) if caller != 6 => { w.leave(now_ticks); w.state().word() }
        (ws::TUNE, Some(w)) if spindle => { w.tune(a, b != 0); w.state().word() }
        (ws::WHY, Some(w)) => w.why_part(a),
        (ws::PASSWORD, Some(w)) if caller != 6 => if w.password_part(a, b) { w.state().word() } else { ws::REFUSED },
        _ => ws::REFUSED,
    }
}
