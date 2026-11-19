// SPDX-License-Identifier: GPL-3.0-only
//! The two cascaded 8259A interrupt controllers of a PC (master 0x20/0x21, slave
//! 0xA0/0xA1 on the master's IRQ 2): the guest's interrupt path in D2, with the local
//! APIC hidden (CPUID) so Linux uses them. Edge-triggered, fixed priority, normal or
//! automatic EOI, IRR/ISR reads through OCW3. Rotation and special mask mode are
//! accepted and treated as their plain forms.

#[derive(Clone, Copy, Debug, Default)]
struct Chip { irr: u8, isr: u8, imr: u8, base: u8, init: u8, icw4: bool, read_isr: bool, auto_eoi: bool, lines: u8 }

fn lowest(bits: u8) -> Option<u8> { if bits == 0 { None } else { Some(bits.trailing_zeros() as u8) } }

impl Chip {
    fn set(&mut self, irq: u8, level: bool) {
        let bit = 1 << (irq & 7);
        if level && self.lines & bit == 0 { self.irr |= bit; }
        if level { self.lines |= bit } else { self.lines &= !bit; }
    }
    /// The IRQ this chip would raise now: the highest-priority requested, unmasked line
    /// above everything in service.
    fn pending(&self) -> Option<u8> {
        let irq = lowest(self.irr & !self.imr)?;
        match lowest(self.isr) { Some(s) if s <= irq => None, _ => Some(irq) }
    }
    fn take(&mut self, irq: u8) {
        let bit = 1 << irq;
        self.irr &= !bit;
        if !self.auto_eoi { self.isr |= bit; }
    }
    fn command(&mut self, v: u8) {
        if v & 0x10 != 0 {
            // ICW1: start initialisation.
            *self = Chip { lines: self.lines, icw4: v & 1 != 0, init: 2, ..Chip::default() };
        } else if v & 0x08 != 0 {
            // OCW3
            if v & 2 != 0 { self.read_isr = v & 1 != 0; }
        } else {
            // OCW2: non-specific (1, 5) or specific (3, 7) EOI.
            match v >> 5 {
                1 | 5 => if let Some(s) = lowest(self.isr) { self.isr &= !(1 << s) },
                3 | 7 => self.isr &= !(1 << (v & 7)),
                _ => {}
            }
        }
    }
    fn data(&mut self, v: u8) {
        match self.init {
            2 => { self.base = v & 0xF8; self.init = 3; }
            3 => { self.init = if self.icw4 { 4 } else { 0 }; }
            4 => { self.auto_eoi = v & 2 != 0; self.init = 0; }
            _ => self.imr = v,
        }
    }
    fn read(&self, port_odd: bool) -> u8 {
        if port_odd { self.imr } else if self.read_isr { self.isr } else { self.irr }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Pic { master: Chip, slave: Chip, elcr: [u8; 2] }

impl Default for Pic { fn default() -> Self { Self::new() } }

impl Pic {
    pub fn new() -> Pic {
        // Before the guest programs it: everything masked, vectors 0x08/0x70 as a BIOS leaves it.
        Pic { master: Chip { imr: 0xFF, base: 0x08, ..Chip::default() }, slave: Chip { imr: 0xFF, base: 0x70, ..Chip::default() }, elcr: [0; 2] }
    }

    pub fn handles(port: u16) -> bool { matches!(port, 0x20 | 0x21 | 0xA0 | 0xA1 | 0x4D0 | 0x4D1) }

    /// Sets the level of `irq` (0..=15); a rising edge requests an interrupt.
    pub fn set_irq(&mut self, irq: u8, level: bool) {
        if irq < 8 { self.master.set(irq, level) } else if irq < 16 { self.slave.set(irq - 8, level) }
        self.cascade();
    }

    fn cascade(&mut self) {
        let raised = self.slave.pending().is_some();
        self.master.set(2, raised);
    }

    /// The vector the guest would get now, if any.
    pub fn pending(&self) -> Option<u8> {
        let irq = self.master.pending()?;
        if irq == 2 {
            let s = self.slave.pending()?;
            return Some(self.slave.base.wrapping_add(s));
        }
        Some(self.master.base.wrapping_add(irq))
    }

    /// The CPU takes the pending interrupt (INTA): its vector, now in service.
    pub fn acknowledge(&mut self) -> Option<u8> {
        let irq = self.master.pending()?;
        self.master.take(irq);
        let vector = if irq == 2 {
            let s = self.slave.pending()?;
            self.slave.take(s);
            self.slave.base.wrapping_add(s)
        } else {
            self.master.base.wrapping_add(irq)
        };
        self.cascade();
        Some(vector)
    }

    pub fn read(&mut self, port: u16) -> u8 {
        match port {
            0x20 | 0x21 => self.master.read(port & 1 != 0),
            0xA0 | 0xA1 => self.slave.read(port & 1 != 0),
            0x4D0 => self.elcr[0],
            0x4D1 => self.elcr[1],
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, port: u16, v: u8) {
        match port {
            0x20 => self.master.command(v),
            0x21 => self.master.data(v),
            0xA0 => self.slave.command(v),
            0xA1 => self.slave.data(v),
            0x4D0 => self.elcr[0] = v & 0xF8,
            0x4D1 => self.elcr[1] = v & 0xDE,
            _ => {}
        }
        self.cascade();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Linux's init_8259A writes.
    fn linux_init(p: &mut Pic) {
        for (port, v) in [(0x21, 0xFF), (0xA1, 0xFF), (0x20, 0x11), (0x21, 0x30), (0x21, 0x04), (0x21, 0x01),
            (0xA0, 0x11), (0xA1, 0x38), (0xA1, 0x02), (0xA1, 0x01)] { p.write(port, v); }
    }

    #[test]
    fn probe_reads_back_the_mask() {
        let mut p = Pic::new();
        p.write(0xA1, 0xFF);
        p.write(0x21, 0xFB);
        assert_eq!((p.read(0x21), p.read(0xA1)), (0xFB, 0xFF));
    }

    #[test]
    fn timer_interrupt_and_eoi() {
        let mut p = Pic::new();
        linux_init(&mut p);
        p.write(0x21, 0xFE); // only IRQ 0
        p.set_irq(0, true);
        p.set_irq(0, false);
        assert_eq!(p.pending(), Some(0x30));
        assert_eq!(p.acknowledge(), Some(0x30));
        assert_eq!(p.pending(), None);
        p.set_irq(0, true);
        p.set_irq(0, false);
        assert_eq!(p.pending(), None, "IRQ 0 is in service");
        p.write(0x20, 0x0B); // OCW3: read ISR
        assert_eq!(p.read(0x20), 1);
        p.write(0x20, 0x20); // EOI
        assert_eq!(p.acknowledge(), Some(0x30));
    }

    #[test]
    fn masked_and_priority() {
        let mut p = Pic::new();
        linux_init(&mut p);
        p.write(0x21, 0x00);
        p.set_irq(4, true);
        p.set_irq(1, true);
        assert_eq!(p.acknowledge(), Some(0x31), "IRQ 1 before IRQ 4");
        assert_eq!(p.pending(), None, "lower priority waits for EOI");
        p.write(0x20, 0x61); // specific EOI for IRQ 1
        assert_eq!(p.acknowledge(), Some(0x34));
        p.write(0x21, 0x10);
        p.set_irq(4, false);
        p.set_irq(4, true);
        p.write(0x20, 0x20);
        assert_eq!(p.pending(), None, "masked");
    }

    #[test]
    fn slave_cascades_through_irq2() {
        let mut p = Pic::new();
        linux_init(&mut p);
        p.write(0x21, 0xFB);
        p.write(0xA1, 0xFE);
        p.set_irq(8, true);
        assert_eq!(p.pending(), Some(0x38));
        assert_eq!(p.acknowledge(), Some(0x38));
        p.write(0xA0, 0x20);
        p.write(0x20, 0x20);
        assert_eq!(p.pending(), None);
        p.set_irq(200, true); // out of range: ignored
        assert_eq!(p.pending(), None);
    }

    #[test]
    fn auto_eoi() {
        let mut p = Pic::new();
        for (port, v) in [(0x20, 0x11), (0x21, 0x20), (0x21, 0x04), (0x21, 0x03), (0x21, 0x00)] { p.write(port, v); }
        p.set_irq(0, true);
        assert_eq!(p.acknowledge(), Some(0x20));
        p.set_irq(0, false);
        p.set_irq(0, true);
        assert_eq!(p.acknowledge(), Some(0x20), "nothing stays in service");
    }
}
