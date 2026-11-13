// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The Synopsys DesignWare APB I2C controller as an I2C master.
//!
//! Intel puts this block behind every Serial IO ("LPSS") I2C function, and AMD uses it
//! for its FCH I2C buses. Laptop touchpads (HID over I2C) sit behind one of them.
//!
//! The driver is a pure state machine over a [`Regs`] window: the service that holds
//! the controller's register lease supplies the reads and writes, and tests supply a
//! simulated controller. It polls; it never waits on an interrupt and never sleeps, so
//! every loop has a bound and every failure is a value.
#![cfg_attr(not(test), no_std)]

pub mod timing;
#[cfg(test)]
mod sim;

pub const IC_CON: u32 = 0x00;
pub const IC_TAR: u32 = 0x04;
pub const IC_DATA_CMD: u32 = 0x10;
pub const IC_SS_SCL_HCNT: u32 = 0x14;
pub const IC_SS_SCL_LCNT: u32 = 0x18;
pub const IC_FS_SCL_HCNT: u32 = 0x1C;
pub const IC_FS_SCL_LCNT: u32 = 0x20;
pub const IC_INTR_MASK: u32 = 0x30;
pub const IC_RAW_INTR_STAT: u32 = 0x34;
pub const IC_RX_TL: u32 = 0x38;
pub const IC_TX_TL: u32 = 0x3C;
pub const IC_CLR_INTR: u32 = 0x40;
pub const IC_CLR_TX_ABRT: u32 = 0x54;
pub const IC_CLR_STOP_DET: u32 = 0x60;
pub const IC_ENABLE: u32 = 0x6C;
pub const IC_STATUS: u32 = 0x70;
pub const IC_TXFLR: u32 = 0x74;
pub const IC_RXFLR: u32 = 0x78;
pub const IC_SDA_HOLD: u32 = 0x7C;
pub const IC_TX_ABRT_SOURCE: u32 = 0x80;
pub const IC_ENABLE_STATUS: u32 = 0x9C;
pub const IC_COMP_PARAM_1: u32 = 0xF4;
pub const IC_COMP_VERSION: u32 = 0xF8;
pub const IC_COMP_TYPE: u32 = 0xFC;
/// What `IC_COMP_TYPE` reads on every DesignWare I2C block ("DW" 0x0140).
pub const COMP_TYPE: u32 = 0x4457_0140;

/// Intel Serial IO private registers, after the DesignWare block in the same BAR.
pub const LPSS_PRIVATE: u32 = 0x200;
pub const LPSS_RESETS: u32 = LPSS_PRIVATE + 0x04;
/// Function and DMA out of reset.
pub const LPSS_RESETS_RELEASE: u32 = 0x7;
/// The BAR bytes a driver must be able to reach (DesignWare block plus Intel's private page).
pub const REGISTERS_END: u64 = 0x300;

pub const CON_MASTER: u32 = 1 << 0;
pub const CON_SPEED_STANDARD: u32 = 1 << 1;
pub const CON_SPEED_FAST: u32 = 2 << 1;
pub const CON_10BIT_MASTER: u32 = 1 << 4;
pub const CON_RESTART_EN: u32 = 1 << 5;
pub const CON_SLAVE_DISABLE: u32 = 1 << 6;

pub const CMD_READ: u32 = 1 << 8;
pub const CMD_STOP: u32 = 1 << 9;
pub const CMD_RESTART: u32 = 1 << 10;

pub const INTR_TX_ABRT: u32 = 1 << 6;
pub const INTR_STOP_DET: u32 = 1 << 9;

pub const STATUS_TFNF: u32 = 1 << 1;
pub const STATUS_RFNE: u32 = 1 << 3;
pub const STATUS_MST_ACTIVITY: u32 = 1 << 5;

pub const ABRT_7B_ADDR_NOACK: u32 = 1 << 0;
pub const ABRT_10ADDR1_NOACK: u32 = 1 << 1;
pub const ABRT_10ADDR2_NOACK: u32 = 1 << 2;
pub const ABRT_TXDATA_NOACK: u32 = 1 << 3;
pub const ABRT_ARB_LOST: u32 = 1 << 12;

/// Register reads and writes of one controller, 32 bits wide, at byte offsets in its BAR.
pub trait Regs {
    fn read(&mut self, offset: u32) -> u32;
    fn write(&mut self, offset: u32, value: u32);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `IC_COMP_TYPE` is not DesignWare's: wrong BAR, or the function is powered off.
    NotDesignWare(u32),
    /// The controller never reported enabled or disabled.
    EnableTimeout,
    /// Nobody answered at that address.
    AddressNack,
    /// The device refused a data byte.
    DataNack,
    /// Another master won the bus.
    ArbitrationLost,
    /// Any other abort, as `IC_TX_ABRT_SOURCE`.
    Abort(u32),
    /// The transfer did not finish within the poll budget.
    Timeout,
    /// More bytes than one transfer allows, or nothing to do.
    Length,
}

/// Bus speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed { Standard, Fast }

/// Clock counts for one speed, as firmware (`SSCN`/`FMCN`) or [`timing`] gives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counts { pub high: u16, pub low: u16, pub sda_hold: u16 }

/// Register polls one transfer may take: at 400 kHz a byte is ~23 µs, and a register read
/// costs well under a microsecond, so this covers a full 4 KiB transfer many times over.
pub const POLL_BUDGET: u32 = 2_000_000;
/// The longest single transfer: a report descriptor of a large touchpad, with room.
pub const MAX_TRANSFER: usize = 4096;

pub struct Controller<R: Regs> {
    regs: R,
    tx_depth: u32,
    rx_depth: u32,
    target: Option<u16>,
    pub version: u32,
}

impl<R: Regs> Controller<R> {
    /// Takes the controller out of Intel's LPSS reset (harmless where that page is
    /// absent), checks it is DesignWare, and sets it up as a master at `speed`.
    pub fn new(mut regs: R, speed: Speed, counts: Counts, intel_lpss: bool) -> Result<Self, Error> {
        if intel_lpss && regs.read(LPSS_RESETS) & LPSS_RESETS_RELEASE != LPSS_RESETS_RELEASE {
            regs.write(LPSS_RESETS, 0);
            regs.write(LPSS_RESETS, LPSS_RESETS_RELEASE);
        }
        let kind = regs.read(IC_COMP_TYPE);
        if kind != COMP_TYPE { return Err(Error::NotDesignWare(kind)); }
        let version = regs.read(IC_COMP_VERSION);
        let param = regs.read(IC_COMP_PARAM_1);
        // A zero parameter word means the block does not report its FIFOs: assume the
        // smallest depth any Intel or AMD part has.
        let (tx_depth, rx_depth) = if param == 0 { (16, 16) } else { (((param >> 16) & 0xff) + 1, ((param >> 8) & 0xff) + 1) };
        let mut c = Self { regs, tx_depth, rx_depth, target: None, version };
        c.enable(false)?;
        let con = CON_MASTER | CON_RESTART_EN | CON_SLAVE_DISABLE
            | match speed { Speed::Standard => CON_SPEED_STANDARD, Speed::Fast => CON_SPEED_FAST };
        c.regs.write(IC_CON, con);
        let (h, l) = match speed { Speed::Standard => (IC_SS_SCL_HCNT, IC_SS_SCL_LCNT), Speed::Fast => (IC_FS_SCL_HCNT, IC_FS_SCL_LCNT) };
        c.regs.write(h, counts.high as u32);
        c.regs.write(l, counts.low as u32);
        if counts.sda_hold != 0 { c.regs.write(IC_SDA_HOLD, counts.sda_hold as u32); }
        c.regs.write(IC_RX_TL, 0);
        c.regs.write(IC_TX_TL, 0);
        // Polled: no interrupt ever leaves the block.
        c.regs.write(IC_INTR_MASK, 0);
        let _ = c.regs.read(IC_CLR_INTR);
        Ok(c)
    }

    pub fn fifo_depths(&self) -> (u32, u32) { (self.tx_depth, self.rx_depth) }

    fn enable(&mut self, on: bool) -> Result<(), Error> {
        for _ in 0..10_000 {
            self.regs.write(IC_ENABLE, on as u32);
            if self.regs.read(IC_ENABLE_STATUS) & 1 == on as u32 { return Ok(()); }
        }
        Err(Error::EnableTimeout)
    }

    fn select(&mut self, address: u16) -> Result<(), Error> {
        if self.target == Some(address) { return Ok(()); }
        self.enable(false)?;
        let mut con = self.regs.read(IC_CON) & !CON_10BIT_MASTER;
        let mut tar = (address & 0x3ff) as u32;
        if address > 0x7f { con |= CON_10BIT_MASTER; tar |= 1 << 12; }
        self.regs.write(IC_CON, con);
        self.regs.write(IC_TAR, tar);
        self.enable(true)?;
        self.target = Some(address);
        Ok(())
    }

    /// One combined transfer: `write` (if any), then a repeated start and `read` (if
    /// any), then a stop.
    pub fn transfer(&mut self, address: u16, write: &[u8], read: &mut [u8]) -> Result<(), Error> {
        let total = write.len() + read.len();
        if total == 0 || total > MAX_TRANSFER { return Err(Error::Length); }
        self.select(address)?;
        let mut budget = POLL_BUDGET;
        while self.regs.read(IC_STATUS) & STATUS_MST_ACTIVITY != 0 {
            budget = budget.checked_sub(1).ok_or(Error::Timeout)?;
        }
        let _ = self.regs.read(IC_CLR_INTR);
        let (mut sent, mut received) = (0usize, 0usize);
        loop {
            budget = budget.checked_sub(1).ok_or_else(|| { self.recover(); Error::Timeout })?;
            let raw = self.regs.read(IC_RAW_INTR_STAT);
            if raw & INTR_TX_ABRT != 0 { return Err(self.abort()); }
            // Queue commands while the TX FIFO has room and reads cannot overflow RX.
            while sent < total && self.regs.read(IC_STATUS) & STATUS_TFNF != 0 {
                let reading = sent >= write.len();
                if reading && (sent - write.len()) - received >= self.rx_depth as usize { break; }
                let mut cmd = if reading { CMD_READ } else { write[sent] as u32 };
                if reading && sent == write.len() && !write.is_empty() { cmd |= CMD_RESTART; }
                if sent + 1 == total { cmd |= CMD_STOP; }
                self.regs.write(IC_DATA_CMD, cmd);
                sent += 1;
            }
            while received < read.len() && self.regs.read(IC_STATUS) & STATUS_RFNE != 0 {
                read[received] = self.regs.read(IC_DATA_CMD) as u8;
                received += 1;
            }
            if sent == total && received == read.len() {
                if raw & INTR_STOP_DET != 0 || self.regs.read(IC_RAW_INTR_STAT) & INTR_STOP_DET != 0 {
                    let _ = self.regs.read(IC_CLR_STOP_DET);
                    // A NACK on the last byte can land with the stop.
                    if self.regs.read(IC_RAW_INTR_STAT) & INTR_TX_ABRT != 0 { return Err(self.abort()); }
                    return Ok(());
                }
            }
        }
    }

    pub fn write(&mut self, address: u16, bytes: &[u8]) -> Result<(), Error> { self.transfer(address, bytes, &mut []) }
    pub fn read(&mut self, address: u16, bytes: &mut [u8]) -> Result<(), Error> { self.transfer(address, &[], bytes) }

    fn abort(&mut self) -> Error {
        let source = self.regs.read(IC_TX_ABRT_SOURCE);
        let _ = self.regs.read(IC_CLR_TX_ABRT);
        let _ = self.regs.read(IC_CLR_INTR);
        if source & (ABRT_7B_ADDR_NOACK | ABRT_10ADDR1_NOACK | ABRT_10ADDR2_NOACK) != 0 { Error::AddressNack }
        else if source & ABRT_TXDATA_NOACK != 0 { Error::DataNack }
        else if source & ABRT_ARB_LOST != 0 { Error::ArbitrationLost }
        else { Error::Abort(source) }
    }

    /// After a timeout: abort whatever is in flight and start the next transfer clean.
    fn recover(&mut self) {
        let _ = self.enable(false);
        self.target = None;
        let _ = self.regs.read(IC_CLR_INTR);
    }

    pub fn into_regs(self) -> R { self.regs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Sim, Device};

    struct Eeprom { mem: [u8; 256], at: u8 }
    impl Device for Eeprom {
        fn write(&mut self, bytes: &[u8]) -> bool {
            if let Some((&a, rest)) = bytes.split_first() {
                self.at = a;
                for b in rest { self.mem[self.at as usize] = *b; self.at = self.at.wrapping_add(1); }
            }
            true
        }
        fn read(&mut self, out: &mut [u8]) { for b in out { *b = self.mem[self.at as usize]; self.at = self.at.wrapping_add(1); } }
    }

    fn counts() -> Counts { timing::counts(100_000_000, Speed::Fast, 300) }

    #[test]
    fn writes_then_reads_back_through_a_repeated_start() {
        let sim = Sim::new(0x50, Eeprom { mem: [0; 256], at: 0 });
        let mut c = Controller::new(sim, Speed::Fast, counts(), true).unwrap();
        c.write(0x50, &[0x10, 1, 2, 3, 4]).unwrap();
        let mut out = [0u8; 4];
        c.transfer(0x50, &[0x10], &mut out).unwrap();
        assert_eq!(out, [1, 2, 3, 4]);
        let sim = c.into_regs();
        assert_eq!(sim.restarts, 1);
        assert_eq!(sim.stops, 2);
        assert!(sim.lpss_released);
    }

    #[test]
    fn long_reads_never_overflow_the_receive_fifo() {
        let mut mem = [0u8; 256];
        for (i, b) in mem.iter_mut().enumerate() { *b = i as u8; }
        let sim = Sim::new(0x2c, Eeprom { mem, at: 0 });
        let mut c = Controller::new(sim, Speed::Fast, counts(), true).unwrap();
        let mut out = [0u8; 200];
        c.transfer(0x2c, &[0], &mut out).unwrap();
        for (i, b) in out.iter().enumerate() { assert_eq!(*b, i as u8); }
        assert!(c.into_regs().max_rx_level <= 16);
    }

    #[test]
    fn a_missing_device_is_an_address_nack_and_the_bus_still_works() {
        let sim = Sim::new(0x50, Eeprom { mem: [7; 256], at: 0 });
        let mut c = Controller::new(sim, Speed::Fast, counts(), true).unwrap();
        let mut out = [0u8; 2];
        assert_eq!(c.transfer(0x51, &[0], &mut out), Err(Error::AddressNack));
        c.transfer(0x50, &[0], &mut out).unwrap();
        assert_eq!(out, [7, 7]);
    }

    #[test]
    fn a_refused_byte_is_a_data_nack() {
        struct Refuses;
        impl Device for Refuses {
            fn write(&mut self, _: &[u8]) -> bool { false }
            fn read(&mut self, _: &mut [u8]) {}
        }
        let mut c = Controller::new(Sim::new(0x15, Refuses), Speed::Fast, counts(), true).unwrap();
        assert_eq!(c.write(0x15, &[1, 2]), Err(Error::DataNack));
    }

    #[test]
    fn programs_master_mode_speed_and_counts() {
        let c = Controller::new(Sim::new(0x50, Eeprom { mem: [0; 256], at: 0 }), Speed::Fast, Counts { high: 104, low: 138, sda_hold: 30 }, true).unwrap();
        let sim = c.into_regs();
        let con = sim.reg(IC_CON);
        assert_eq!(con & (CON_MASTER | CON_SLAVE_DISABLE | CON_RESTART_EN), CON_MASTER | CON_SLAVE_DISABLE | CON_RESTART_EN);
        assert_eq!(con & (3 << 1), CON_SPEED_FAST);
        assert_eq!((sim.reg(IC_FS_SCL_HCNT), sim.reg(IC_FS_SCL_LCNT), sim.reg(IC_SDA_HOLD)), (104, 138, 30));
        assert_eq!(sim.reg(IC_INTR_MASK), 0);
    }

    #[test]
    fn refuses_a_block_that_is_not_designware() {
        let mut sim = Sim::new(0x50, Eeprom { mem: [0; 256], at: 0 });
        sim.comp_type = 0xffff_ffff;
        assert_eq!(Controller::new(sim, Speed::Fast, counts(), true).err(), Some(Error::NotDesignWare(0xffff_ffff)));
    }

    #[test]
    fn ten_bit_addresses_set_the_ten_bit_mode() {
        let sim = Sim::new(0x2a5, Eeprom { mem: [9; 256], at: 0 });
        let mut c = Controller::new(sim, Speed::Standard, timing::counts(100_000_000, Speed::Standard, 300), false).unwrap();
        let mut out = [0u8; 1];
        c.transfer(0x2a5, &[0], &mut out).unwrap();
        assert_eq!(out, [9]);
        let sim = c.into_regs();
        assert!(sim.reg(IC_CON) & CON_10BIT_MASTER != 0);
        assert!(!sim.lpss_released);
    }

    #[test]
    fn empty_and_oversized_transfers_are_refused() {
        let mut c = Controller::new(Sim::new(0x50, Eeprom { mem: [0; 256], at: 0 }), Speed::Fast, counts(), true).unwrap();
        assert_eq!(c.transfer(0x50, &[], &mut []), Err(Error::Length));
        let mut big = [0u8; MAX_TRANSFER + 1];
        assert_eq!(c.read(0x50, &mut big), Err(Error::Length));
    }

    #[test]
    fn a_hung_bus_times_out_instead_of_spinning_forever() {
        let mut sim = Sim::new(0x50, Eeprom { mem: [0; 256], at: 0 });
        sim.hang = true;
        let mut c = Controller::new(sim, Speed::Fast, counts(), true).unwrap();
        let mut out = [0u8; 1];
        assert_eq!(c.read(0x50, &mut out), Err(Error::Timeout));
    }
}
