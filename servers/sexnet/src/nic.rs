// SPDX-License-Identifier: GPL-3.0-only
//! Bounded normal-profile device preparation. No DMA or packet submission yet.
//! Only Live resolves the immutable BOOT lease; requests cannot supply addresses.
use sex_pdx::{memory_region as m, pci_command as c};

pub const CTRL: u32 = 0;
pub const STATUS: u32 = 8;
pub const IMS: u32 = 0xd0;
pub const IMC: u32 = 0xd8;
pub const RCTL: u32 = 0x100;
pub const TCTL: u32 = 0x400;
pub const RAL: u32 = 0x5400;
pub const RAH: u32 = 0x5404;
const RESET: u32 = 1 << 26;
const ENABLE: u32 = 1 << 1;
const SETTLE_TICKS: u64 = 2;
const DEADLINE_TICKS: u64 = 2000;
const TURN_BUDGET: u16 = 8192;
const NIC_REGION: u32 = 0x10000 + 13 * 1024 + sex_pdx::SLOT_NIC as u32;

pub trait Hardware {
    fn command(&mut self, set: u16, clear: u16) -> Result<c::Reply, u64>;
    fn read(&mut self, offset: u32) -> u32;
    fn write(&mut self, offset: u32, value: u32);
}

/// Private view, obtained only by the current subject's capability invocation.
pub struct Live { view: m::RegionView, model: c::Model, dma: Option<m::RegionView> }
impl Live {
    pub fn open() -> Result<Self, u64> {
        let info = sex_pdx::memory_region_info(sex_pdx::SLOT_NIC)?;
        if !valid_info(info) { return Err(c::DENIED); }
        let view = sex_pdx::memory_region_full_view(sex_pdx::SLOT_NIC, m::QUERY_VIEW | m::READ | m::WRITE)?;
        if !valid_view(info, view) { return Err(c::DENIED); }
        let model=sex_pdx::device_identity(sex_pdx::SLOT_NIC)?.model().ok_or(c::DENIED)?;
        Ok(Self { view, model, dma: None })
    }
    fn address(&self, offset: u32) -> *mut u32 {
        assert!(offset % 4 == 0 && offset as u64 + 4 <= self.view.length);
        (self.view.virtual_address + offset as u64) as *mut u32
    }
}
fn valid_info(info: m::MemoryRegionInfo) -> bool {
    info.region_id == NIC_REGION && info.rights == m::QUERY_VIEW | m::READ | m::WRITE
        && info.peer_pd.is_none() && info.length >= RAH as u64 + 4 && info.length <= 65536
}
fn valid_view(info: m::MemoryRegionInfo, view: m::RegionView) -> bool {
    valid_info(info) && view.region_id == info.region_id && view.length == info.length
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
}

impl Live {
    fn dma_pointer(&self,offset:u32,bytes:usize)->*mut u8 {
        let view=self.dma.expect("NIC_DMA_NOT_RESOLVED");
        assert!((offset as u64).checked_add(bytes as u64).is_some_and(|end|end<=view.length));
        (view.virtual_address+offset as u64)as *mut u8
    }
}
impl crate::ethernet::Hardware for Live {
    fn model(&self)->c::Model{self.model}
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
    fn disable(&mut self) {
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
            Phase::Drain if now - self.since >= SETTLE_TICKS => {
                if self.hardware.read(RCTL) & ENABLE != 0 || self.hardware.read(TCTL) & ENABLE != 0
                    || self.hardware.read(IMS) != 0
                { self.fail(Failure::Registers); return; }
                let ctrl = self.hardware.read(CTRL);
                if ctrl == u32::MAX || ctrl & RESET != 0 { self.fail(Failure::Registers); return; }
                self.hardware.write(CTRL, ctrl | RESET);
                self.since = now; self.phase = Phase::Reset;
            },
            Phase::Reset if now - self.since >= SETTLE_TICKS => {
                let ctrl = self.hardware.read(CTRL);
                if ctrl == u32::MAX { self.fail(Failure::Registers); return; }
                if ctrl & RESET != 0 { return; }
                self.disable(); self.phase = Phase::Verify;
            },
            Phase::Verify => {
                let Ok(command) = self.hardware.command(0, 0) else { self.fail(Failure::Command); return; };
                if command.before != command.after || command.changed || command.after & c::NIC_BITS != c::MEMORY {
                    self.fail(Failure::Command); return;
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
