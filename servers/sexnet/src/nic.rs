// SPDX-License-Identifier: GPL-3.0-only
//! Bounded normal-profile device preparation. No DMA or packet submission yet.
//! Only Live resolves the immutable BOOT lease; requests cannot supply addresses.
//! Intel e1000/e1000e/igb/igc share this register map; Realtek parts take the
//! same phases through crate::realtek's registers.
use sex_pdx::{memory_region as m, pci_command as c};
use crate::realtek as rtl;

pub const CTRL: u32 = 0;
pub const STATUS: u32 = 8;
pub const IMS: u32 = 0xd0;
pub const IMC: u32 = 0xd8;
pub const RCTL: u32 = 0x100;
pub const TCTL: u32 = 0x400;
pub const RAL: u32 = 0x5400;
pub const RAH: u32 = 0x5404;
/// igb/igc extended interrupt mask clear (MSI-X vectors); legacy parts lack it.
pub const EIMC: u32 = 0x1528;
const RESET: u32 = 1 << 26;
/// igc (I225/I226) resets the whole device with CTRL.DEV_RST instead of CTRL.RST.
const DEVICE_RESET: u32 = 1 << 29;
const ENABLE: u32 = 1 << 1;
const SETTLE_TICKS: u64 = 2;
const DEADLINE_TICKS: u64 = 2000;
const TURN_BUDGET: u16 = 8192;
const NIC_REGION: u32 = 0x10000 + 13 * 1024 + sex_pdx::SLOT_NIC as u32;

pub trait Hardware {
    fn command(&mut self, set: u16, clear: u16) -> Result<c::Reply, u64>;
    fn read(&mut self, offset: u32) -> u32;
    fn write(&mut self, offset: u32, value: u32);
    /// The BOOT identity's register layout; never a selector of hardware.
    fn model(&self) -> c::Model;
    /// Realtek byte and word registers. A 32-bit read-modify-write would also
    /// write the neighbouring registers (some of them write-one-to-clear).
    fn read8(&mut self, offset: u32) -> u8;
    fn write8(&mut self, offset: u32, value: u8);
    fn read16(&mut self, offset: u32) -> u16;
    fn write16(&mut self, offset: u32, value: u16);
}

/// Private view, obtained only by the current subject's capability invocation.
pub struct Live { view: m::RegionView, model: c::Model, dma: Option<m::RegionView> }
impl Live {
    pub fn open() -> Result<Self, u64> {
        // The identity names the register layout, so it decides the prefix the
        // lease must cover: Realtek keeps 256 bytes of registers, igb/igc 0xE030.
        let model=sex_pdx::device_identity(sex_pdx::SLOT_NIC)?.model().ok_or(c::DENIED)?;
        let info = sex_pdx::memory_region_info(sex_pdx::SLOT_NIC)?;
        if !valid_info(info, model) { return Err(c::DENIED); }
        let view = sex_pdx::memory_region_full_view(sex_pdx::SLOT_NIC, m::QUERY_VIEW | m::READ | m::WRITE)?;
        if !valid_view(info, view, model) { return Err(c::DENIED); }
        Ok(Self { view, model, dma: None })
    }
    fn at(&self, offset: u32, bytes: u32) -> u64 {
        assert!(offset % bytes == 0 && offset as u64 + bytes as u64 <= self.view.length);
        self.view.virtual_address + offset as u64
    }
    fn address(&self, offset: u32) -> *mut u32 { self.at(offset, 4) as *mut u32 }
}
fn valid_info(info: m::MemoryRegionInfo, model: c::Model) -> bool {
    info.region_id == NIC_REGION && info.rights == m::QUERY_VIEW | m::READ | m::WRITE
        && info.peer_pd.is_none() && info.length >= model.registers_end() && info.length <= 65536
}
fn valid_view(info: m::MemoryRegionInfo, view: m::RegionView, model: c::Model) -> bool {
    valid_info(info, model) && view.region_id == info.region_id && view.length == info.length
        && view.dma_address.is_none() && view.virtual_address != 0
        && view.virtual_address % m::PAGE_BYTES == 0
        && view.virtual_address.checked_add(view.length).is_some_and(|end| end <= m::USER_VA_END)
}
impl Hardware for Live {
    fn command(&mut self, set: u16, clear: u16) -> Result<c::Reply, u64> {
        sex_pdx::device_command(sex_pdx::SLOT_NIC, set as u64, clear as u64)
    }
    fn read(&mut self, offset: u32) -> u32 { unsafe { self.address(offset).read_volatile() } }
    fn write(&mut self, offset: u32, value: u32) { unsafe { self.address(offset).write_volatile(value); } }
    fn model(&self) -> c::Model { self.model }
    fn read8(&mut self, offset: u32) -> u8 { unsafe { (self.at(offset, 1) as *const u8).read_volatile() } }
    fn write8(&mut self, offset: u32, value: u8) { unsafe { (self.at(offset, 1) as *mut u8).write_volatile(value); } }
    fn read16(&mut self, offset: u32) -> u16 { unsafe { (self.at(offset, 2) as *const u16).read_volatile() } }
    fn write16(&mut self, offset: u32, value: u16) { unsafe { (self.at(offset, 2) as *mut u16).write_volatile(value); } }
}

impl Live {
    fn dma_pointer(&self,offset:u32,bytes:usize)->*mut u8 {
        let view=self.dma.expect("NIC_DMA_NOT_RESOLVED");
        assert!((offset as u64).checked_add(bytes as u64).is_some_and(|end|end<=view.length));
        (view.virtual_address+offset as u64)as *mut u8
    }
}
impl crate::ethernet::Hardware for Live {
    fn mmio_length(&self)->u64{self.view.length}
    fn dma_region(&mut self)->Result<m::RegionView,u64>{
        // A single permanent pool per normal driver. No reset/reopen/reclaim.
        if self.dma.is_some(){return Err(c::DENIED);}
        let info=sex_pdx::memory_region_info(sex_pdx::SLOT_DMA_REGION)?;
        if info.region_id!=c::DMA_REGION_ID||info.length!=c::DMA_BYTES||info.rights!=m::QUERY_VIEW|m::READ|m::WRITE|m::DMA_ADDRESS||info.peer_pd.is_some(){return Err(c::DENIED);}
        let view=sex_pdx::memory_region_full_view(sex_pdx::SLOT_DMA_REGION,info.rights)?;
        if view.region_id!=info.region_id||view.length!=info.length||sex_pdx::dma_pool::DmaRegionPool::new(view).is_err(){return Err(c::DENIED);}
        self.dma=Some(view);Ok(view)
    }
    fn load64(&mut self,offset:u32)->u64{assert!(offset%8==0);unsafe{(self.dma_pointer(offset,8)as *const u64).read_volatile()}}
    fn store64(&mut self,offset:u32,value:u64){assert!(offset%8==0);unsafe{(self.dma_pointer(offset,8)as *mut u64).write_volatile(value);}}
    fn load_bytes(&mut self,offset:u32,out:&mut[u8]){let pointer=self.dma_pointer(offset,out.len());for(i,byte)in out.iter_mut().enumerate(){*byte=unsafe{pointer.add(i).read_volatile()};}}
    fn store_bytes(&mut self,offset:u32,bytes:&[u8]){let pointer=self.dma_pointer(offset,bytes.len());for(i,byte)in bytes.iter().enumerate(){unsafe{pointer.add(i).write_volatile(*byte);}}}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Cold, Drain, Reset, Verify, Quiescent, Quarantined }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure { Command, UnknownMaster, Deadline, Clock, Registers, Address }
pub struct Device<H: Hardware> {
    hardware: H,
    phase: Phase,
    failure: Option<Failure>,
    started: u64,
    last: u64,
    since: u64,
    turns: u16,
    pub mac: [u8; 6],
    pub status: u32,
}
impl<H: Hardware> Device<H> {
    /// Consumes the sole preparer's custody. The immutable BOOT backing is
    /// never freed, including when an unready/quarantined attempt is dropped.
    pub fn into_quiescent(self) -> Option<(H,[u8;6])> {
        if self.phase==Phase::Quiescent {Some((self.hardware,self.mac))}else{None}
    }
    pub fn new(hardware: H, now: u64) -> Self {
        Self { hardware, phase: Phase::Cold, failure: None, started: now, last: now,
            since: now, turns: 0, mac: [0; 6], status: 0 }
    }
    pub fn phase(&self) -> Phase { self.phase }
    pub fn failure(&self) -> Option<Failure> { self.failure }
    pub fn pending(&self) -> bool { !matches!(self.phase, Phase::Quiescent | Phase::Quarantined) }
    fn fail(&mut self, reason: Failure) { self.failure = Some(reason); self.phase = Phase::Quarantined; }
    fn realtek(&self) -> bool { self.hardware.model().family() == c::Family::Realtek }
    /// The CTRL bit that resets the MAC: igc uses DEV_RST, the others RST.
    fn reset_bit(&self) -> u32 { if self.hardware.model().family() == c::Family::Igc { DEVICE_RESET } else { RESET } }
    fn disable(&mut self) {
        if self.realtek() { self.status = rtl::disable(&mut self.hardware) as u32; return; }
        if self.hardware.model().advanced() { self.hardware.write(EIMC, u32::MAX); }
        self.hardware.write(IMC, u32::MAX);
        self.hardware.write(RCTL, 0);
        self.hardware.write(TCTL, 0);
        // Readback also flushes posted MMIO writes; this is not a DMA-drain proof.
        self.status = self.hardware.read(STATUS);
    }
    /// At most one bounded phase per turn. The caller services IPC and yields
    /// between turns; stalled/backward clocks terminate without retrying writes.
    pub fn advance(&mut self, now: u64) {
        if !self.pending() { return; }
        self.turns += 1;
        if now < self.last { self.fail(Failure::Clock); return; }
        self.last = now;
        if now - self.started >= DEADLINE_TICKS || self.turns >= TURN_BUDGET {
            self.fail(Failure::Deadline); return;
        }
        match self.phase {
            Phase::Cold => {
                let Ok(before) = self.hardware.command(0, 0) else { self.fail(Failure::Command); return; };
                if before.before != before.after || before.changed { self.fail(Failure::Command); return; }
                // An already-active controller has unproven DMA custody. Leave
                // it untouched; never overwrite buffers or infer a safe reset.
                if before.after & c::BUS_MASTER != 0 { self.fail(Failure::UnknownMaster); return; }
                let Ok(reply) = self.hardware.command(c::MEMORY, c::BUS_MASTER)
                    else { self.fail(Failure::Command); return; };
                if reply.before != before.after || reply.after != reply.before | c::MEMORY
                    || reply.changed != (reply.before != reply.after)
                { self.fail(Failure::Command); return; }
                self.disable(); self.since = now; self.phase = Phase::Drain;
            },
            Phase::Drain if now - self.since >= SETTLE_TICKS && self.realtek() => {
                let command = self.hardware.read8(rtl::CHIP_CMD);
                if !rtl::idle(&mut self.hardware) || command & rtl::CMD_RESET != 0 { self.fail(Failure::Registers); return; }
                self.hardware.write8(rtl::CHIP_CMD, rtl::CMD_RESET);
                self.since = now; self.phase = Phase::Reset;
            },
            Phase::Reset if now - self.since >= SETTLE_TICKS && self.realtek() => {
                let command = self.hardware.read8(rtl::CHIP_CMD);
                if command == u8::MAX { self.fail(Failure::Registers); return; }
                if command & rtl::CMD_RESET != 0 { return; }
                self.disable(); self.phase = Phase::Verify;
            },
            Phase::Drain if now - self.since >= SETTLE_TICKS => {
                let reset = self.reset_bit();
                if self.hardware.read(RCTL) & ENABLE != 0 || self.hardware.read(TCTL) & ENABLE != 0
                    || self.hardware.read(IMS) != 0
                { self.fail(Failure::Registers); return; }
                let ctrl = self.hardware.read(CTRL);
                if ctrl == u32::MAX || ctrl & reset != 0 { self.fail(Failure::Registers); return; }
                self.hardware.write(CTRL, ctrl | reset);
                self.since = now; self.phase = Phase::Reset;
            },
            Phase::Reset if now - self.since >= SETTLE_TICKS => {
                let ctrl = self.hardware.read(CTRL);
                if ctrl == u32::MAX { self.fail(Failure::Registers); return; }
                if ctrl & self.reset_bit() != 0 { return; }
                // igb/igc reload the station address from NVM after reset; RAH.AV
                // marks it loaded (Linux polls EECD.AUTO_RD for the same event).
                // A part that never sets it runs into the preparation deadline.
                if self.hardware.model().advanced() && self.hardware.read(RAH) & (1 << 31) == 0 { return; }
                self.disable(); self.phase = Phase::Verify;
            },
            Phase::Verify => {
                let Ok(command) = self.hardware.command(0, 0) else { self.fail(Failure::Command); return; };
                if command.before != command.after || command.changed || command.after & c::NIC_BITS != c::MEMORY {
                    self.fail(Failure::Command); return;
                }
                if self.realtek() {
                    if !rtl::idle(&mut self.hardware) { self.fail(Failure::Registers); return; }
                    // IDR0-5 hold the address the chip loaded from its EEPROM/eFuse.
                    self.mac = rtl::station(&mut self.hardware);
                    if self.mac == [0; 6] || self.mac == [0xff; 6] || self.mac[0] & 1 != 0 {
                        self.mac = [0; 6]; self.fail(Failure::Address); return;
                    }
                    self.status = rtl::phy_status(&mut self.hardware) as u32;
                    self.phase = Phase::Quiescent; return;
                }
                if self.hardware.read(RCTL) & ENABLE != 0 || self.hardware.read(TCTL) & ENABLE != 0
                    || self.hardware.read(IMS) != 0
                { self.fail(Failure::Registers); return; }
                let lo = self.hardware.read(RAL).to_le_bytes(); let hi = self.hardware.read(RAH);
                self.mac = [lo[0], lo[1], lo[2], lo[3], hi as u8, (hi >> 8) as u8];
                if hi & (1 << 31) == 0 || self.mac == [0; 6] || self.mac[0] & 1 != 0 {
                    self.mac = [0; 6]; self.fail(Failure::Address); return;
                }
                self.status = self.hardware.read(STATUS);
                if self.status == u32::MAX { self.fail(Failure::Registers); return; }
                self.phase = Phase::Quiescent;
            },
            _ => {},
        }
    }
}
