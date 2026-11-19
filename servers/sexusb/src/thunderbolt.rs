// SPDX-License-Identifier: GPL-3.0-only
//! Thunderbolt 4 / USB4 in the USB service (stage T1b): BOOT leases Intel's
//! integrated NHI here with force power raised; this runs the firmware connection
//! manager from `sex_thunderbolt` on it and logs what plugs in.
//!
//! DisplayPort and USB tunnels need no approval: the firmware makes them as soon as
//! a dock or monitor connects. PCIe tunnels (eGPU, a dock's network chip) at
//! security level "user" wait for the desktop prompt (stage T2); until then they
//! are logged as waiting, never approved behind the person's back.

use sex_pdx::memory_region::{DMA_ADDRESS, QUERY_VIEW, READ, WRITE};
use sex_pdx::pci_command as c;
use sex_pdx::serial_record as log;
use sex_thunderbolt::icm::{Event, Manager, Phase};
use sex_thunderbolt::ring::Layout;
use sex_thunderbolt::{regs, Nhi};

struct Hardware { registers: u64, length: u64, pool: u64, pool_length: u64 }

impl Nhi for Hardware {
    fn read(&mut self, register: u32) -> u32 {
        if register as u64 + 4 > self.length { return u32::MAX; }
        // SAFETY: inside the BAR view BOOT granted this service.
        unsafe { core::ptr::read_volatile((self.registers + register as u64) as *const u32) }
    }
    fn write(&mut self, register: u32, value: u32) {
        if register as u64 + 4 > self.length { return; }
        // SAFETY: as above.
        unsafe { core::ptr::write_volatile((self.registers + register as u64) as *mut u32, value) }
    }
    fn load(&self, offset: u64, out: &mut [u8]) {
        if offset + out.len() as u64 > self.pool_length { return; }
        for (i, b) in out.iter_mut().enumerate() {
            // SAFETY: inside this service's Thunderbolt DMA pool.
            *b = unsafe { core::ptr::read_volatile((self.pool + offset + i as u64) as *const u8) };
        }
    }
    fn store(&mut self, offset: u64, bytes: &[u8]) {
        if offset + bytes.len() as u64 > self.pool_length { return; }
        for (i, b) in bytes.iter().enumerate() {
            // SAFETY: as above.
            unsafe { core::ptr::write_volatile((self.pool + offset + i as u64) as *mut u8, *b) }
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
}

pub struct Thunderbolt { hw: Hardware, manager: Manager, phase: Phase }

fn now_us() -> u64 { sex_pdx::get_ticks() * 1000 }

impl Thunderbolt {
    /// The leased NHI, ready to start; None when this machine has none (silent) or
    /// the lease is incomplete (logged).
    pub fn open() -> Option<Thunderbolt> {
        let identity = sex_pdx::device_identity(c::SLOT_THUNDERBOLT).ok()?;
        let name = sex_thunderbolt::known(identity.vendor, identity.product);
        log!("[tb.nhi] vendor={:#06x} device={:#06x} name=\"{}\"", identity.vendor, identity.product, name.unwrap_or("unknown"));
        if name.is_none() {
            log!("[tb.unsupported] device={:#06x} reason=no_firmware_connection_manager_known", identity.product);
            return None;
        }
        let bar = match sex_pdx::memory_region_full_view(c::SLOT_THUNDERBOLT, QUERY_VIEW | READ | WRITE) {
            Ok(v) if v.dma_address.is_none() && v.length >= regs::REGISTERS_END as u64 => v,
            other => { log!("[tb.failed] step=lease bar_bytes={}", other.map_or(0, |v| v.length)); return None; }
        };
        let pool = match sex_pdx::memory_region_info(c::SLOT_THUNDERBOLT_DMA) {
            Ok(i) if i.region_id == c::THUNDERBOLT_DMA_REGION_ID && i.length == c::THUNDERBOLT_DMA_BYTES && i.peer_pd.is_none() =>
                sex_pdx::memory_region_full_view(c::SLOT_THUNDERBOLT_DMA, QUERY_VIEW | READ | WRITE | DMA_ADDRESS).ok(),
            _ => None,
        };
        let Some((pool, bus)) = pool.and_then(|v| v.dma_address.map(|a| (v, a))) else {
            log!("[tb.failed] step=dma reason=no_pool"); return None;
        };
        let layout = Layout::control(bus);
        if layout.end > pool.length { log!("[tb.failed] step=dma reason=small_pool bytes={}", pool.length); return None; }
        match sex_pdx::device_command(c::SLOT_THUNDERBOLT, c::NIC_BITS as u64, 0) {
            Ok(r) if r.after & c::NIC_BITS == c::NIC_BITS => {}
            other => { log!("[tb.failed] step=command ok={}", other.is_ok() as u8); return None; }
        }
        let mut hw = Hardware { registers: bar.virtual_address, length: bar.length, pool: pool.virtual_address, pool_length: pool.length };
        let status = hw.read(regs::FW_STS);
        log!("[tb.prepared] bar_bytes={} pool={} bus={:#x} fw_sts={:#x} caps={:#x}", bar.length, pool.length, bus, status, hw.read(regs::CAPS));
        Some(Thunderbolt { hw, manager: Manager::new(layout, now_us()), phase: Phase::Firmware })
    }

    /// Runs the connection manager once; true if something happened.
    pub fn poll(&mut self) -> bool {
        let before = (self.manager.frames_in, self.manager.frames_out);
        let manager = &mut self.manager;
        manager.poll(&mut self.hw, now_us(), |e| match e {
            Event::Ready(r) => {
                let (major, minor) = r.nvm();
                log!("[tb.ready] nvm={:x}.{:02x} protocol={} boot_acl={} security={}", major, minor, r.proto_version, r.boot_acl, r.security);
            }
            Event::Connected(d) => {
                log!("[tb.device.connected] route={:#x} depth={} link={} gbps={}x{} security={:?} authorized={} boot={}",
                    d.route, d.depth, d.link, d.gbps(), if d.dual_lane { 2 } else { 1 }, d.security, d.authorized as u8, d.boot as u8);
                if !d.authorized && d.security.needs_approval() {
                    log!("[tb.device.waiting] route={:#x} reason=approval_prompt_not_built pcie=0 displayport=1 usb=1", d.route);
                }
            }
            Event::Rejected { route } => log!("[tb.device.rejected] route={:#x} reason=topology_limit", route),
            Event::Disconnected(d) => log!("[tb.device.disconnected] route={:#x}", d.route),
            Event::Approved { route, ok } => log!("[tb.device.approved] route={:#x} ok={}", route, ok as u8),
            Event::Veto(reason) => log!("[tb.rtd3.veto] reason={:#x}", reason),
            Event::XDomain { connected } => log!("[tb.xdomain] connected={}", connected as u8),
            Event::Failed(f) => log!("[tb.failed] step=manager reason={:?}", f),
        });
        if self.manager.phase != self.phase {
            self.phase = self.manager.phase;
            log!("[tb.phase] {:?} hops={} devices={}", self.phase, self.manager.hops, self.manager.count());
        }
        (self.manager.frames_in, self.manager.frames_out) != before
    }

    /// Still starting (the loop should not take long sleeps).
    pub fn starting(&self) -> bool { matches!(self.phase, Phase::Firmware | Phase::DriverReady | Phase::RootRouter) }
}
