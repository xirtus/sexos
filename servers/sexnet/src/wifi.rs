// SPDX-License-Identifier: GPL-3.0-only
//! Intel Wi-Fi in the network service, stages W2-W3: from the chip BOOT leased us
//! to the firmware's ALIVE, then through the firmware's init commands to a chip
//! ready to scan, with one boot-log line per step so a first boot on real
//! hardware (the Alienware x17 R1's Killer AX1675x) says exactly how far it got.
//! Everything that can be checked without the chip lives in `sex_wifi`; this file
//! only maps the lease, the DMA pool and the firmware pack onto its traits.
use sex_pdx::{memory_region as m, pci_command as c};
use sex_wifi::{bringup::Bringup, context, csr, devices, init::{self, Init}, pack::Pack, ucode::Firmware};

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
    finished: bool,
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
        let mut mmio = Mmio { base: bar.virtual_address, length: bar.length };
        use csr::Registers;
        let hw_rev = devices::normalize_hw_rev(mmio.read(csr::HW_REV));
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
                match Pack::parse(bytes).and_then(|p| p.file(chip.firmware).map(|f| (f, chip.pnvm.and_then(|n| p.file(n).ok())))) {
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
        match sex_pdx::device_command(c::SLOT_WIFI, c::NIC_BITS as u64, 0) {
            Ok(r) if r.after & c::NIC_BITS == c::NIC_BITS => {}
            other => { sex_pdx::serial_record!("[wifi.failed] step=command ok={}", other.is_ok() as u8); return None; }
        }
        sex_pdx::serial_record!("[wifi.prepared] pool={} used={} rx_pages={} bus={:#x}",
            dma.length, bringup.layout.end, context::RX_BUFFERS, bus);
        Some(Wifi { mmio, pool, bringup, last: csr::Step::Ready, file, pnvm, hw_rev, rf_id, init: None, last_init: None, finished: false })
    }

    /// True until the chip is ready to scan or has failed.
    pub fn pending(&self) -> bool { !self.finished }

    pub fn advance(&mut self, now_ticks: u64) {
        if self.finished { return; }
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
                    "[wifi.failed] step={:?} alive_ok={:?} notifications={} bad_rx_buffer={:?} int={:#x} gp_cntrl={:#x} rx_closed={}",
                    step, self.bringup.alive.map(|a| a.ok), self.bringup.notifications, self.bringup.bad_buffer,
                    self.mmio.read(csr::INT), self.mmio.read(csr::GP_CNTRL),
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
        self.finished = true;
        if i.step != init::Step::Ready {
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
    }
}
