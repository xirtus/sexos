// SPDX-License-Identifier: GPL-3.0-only
//! Stage W2: from a leased chip and its firmware file to the firmware's ALIVE.
//! Puts the firmware and the context tables in the DMA region, posts the receive
//! pages, runs the power-up state machine, and reads the receive ring until ALIVE
//! (or an error) arrives. One bounded step per call; the service calls again.
use crate::context::{Dma, DmaRead, Layout, RX_BUFFER_BYTES};
use crate::csr::{Boot, Registers, Step};
use crate::devices::Chip;
use crate::hcmd::{self, Alive};
use crate::rx::{self, Rx};
use crate::ucode::Firmware;

pub struct Bringup {
    pub boot: Boot,
    pub layout: Layout,
    pub rx: Rx,
    pub base: u64,
    pub iml_len: u32,
    pub alive: Option<Alive>,
    /// Firmware notifications read from the receive ring so far.
    pub notifications: u32,
    /// A used-ring entry named a page the driver never posted.
    pub bad_buffer: Option<u16>,
}

impl Bringup {
    /// Lays the firmware and tables out in the region (`base` is its bus address)
    /// and posts the receive pages. Nothing is written to the chip yet.
    pub fn prepare(chip: Chip, fw: &Firmware<'_>, dma: &mut impl Dma, base: u64, region_bytes: u64, hw_rev: u32, now_us: u64)
        -> Result<Self, crate::context::Error>
    {
        let image = fw.image();
        let iml = if chip.family == crate::devices::Family::Ax210 { fw.iml.unwrap_or(&[]) } else { &[] };
        let layout = Layout::plan(chip.family, &image, iml.len(), region_bytes)?;
        layout.write(dma, base, &image, iml, hw_rev, false);
        let mut rx = Rx::new(chip.family);
        rx.fill(dma, &layout, base);
        Ok(Self { boot: Boot::new(chip, now_us), layout, rx, base, iml_len: iml.len() as u32,
            alive: None, notifications: 0, bad_buffer: None })
    }

    /// One step. Returns true while more steps are needed.
    pub fn advance<D: Dma + DmaRead>(&mut self, r: &mut impl Registers, dma: &mut D, now_us: u64) -> bool {
        // The free ring must be visible before the CPU starts (the NicLock step).
        if self.boot.step == Step::NicLock { self.rx.kick(r); }
        let interrupts = self.boot.alive_interrupts;
        self.boot.advance(r, now_us, &self.layout, self.base, self.iml_len);
        // ALIVE interrupt: the firmware has set the receive hardware up; publish again.
        if self.boot.alive_interrupts != interrupts { self.rx.republish(dma, &self.layout, self.base, r); }
        if self.boot.step == Step::WaitAlive { self.drain(r, dma); }
        !self.boot.done()
    }

    /// Reads every page the firmware filled, looks for ALIVE, and posts the page again.
    pub fn drain<D: Dma + DmaRead>(&mut self, r: &mut impl Registers, dma: &mut D) {
        let mut page = [0u8; RX_BUFFER_BYTES as usize];
        let mut took = false;
        while let Some(next) = self.rx.next(dma, &self.layout) {
            let id = match next {
                Ok(id) => id,
                Err(rx::Error::BadBuffer(id)) => { self.bad_buffer = Some(id); self.boot.alive_received(false); return; }
            };
            dma.read(Rx::buffer(&self.layout, id).offset, &mut page);
            for p in hcmd::packets(&page) {
                self.notifications = self.notifications.saturating_add(1);
                if let Some(a) = hcmd::parse_alive(&p) {
                    self.alive = Some(a);
                    self.boot.alive_received(a.ok);
                }
            }
            self.rx.post(dma, &self.layout, self.base, id);
            took = true;
        }
        if took { self.rx.kick(r); }
    }
}
