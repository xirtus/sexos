// SPDX-License-Identifier: GPL-3.0-only
//! The 8254 programmable interval timer (ports 0x40-0x43) and port 0x61 (channel 2's
//! gate and output), counting from the VMM's clock in nanoseconds.
//!
//! Channel 0 drives IRQ 0: `poll` reports whether a period (modes 2 and 3) or the
//! one-shot count (mode 0) ended since the last poll; missed periods coalesce into one
//! interrupt, as they would on a busy PIC. Channel 2 is what Linux calibrates the TSC
//! against (mode 0 with the gate on, polling the count's high byte and OUT2).
//! Modes 1, 4 and 5 count like mode 0. BCD counting is not offered (the bit is kept
//! and ignored).

pub const HZ: u64 = 1_193_182;

#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    mode: u8,
    access: u8,
    reload: u16,
    /// When the count was loaded (ns); None until the guest loads one.
    start: Option<u64>,
    write_hi: bool,
    pending_lo: u8,
    latch: Option<u16>,
    status: Option<u8>,
    read_hi: bool,
    gate: bool,
    /// The one-shot's end was reported, or the last period that was.
    fired: u64,
}

impl Channel {
    fn period(&self) -> u64 { if self.reload == 0 { 0x10000 } else { self.reload as u64 } }
    fn ticks(&self, now: u64) -> Option<u64> {
        let start = self.start?;
        if !self.gate { return Some(0); }
        Some((now.saturating_sub(start) as u128 * HZ as u128 / 1_000_000_000) as u64)
    }
    /// Modes 6 and 7 are 2 and 3.
    fn kind(&self) -> u8 { match self.mode { 6 => 2, 7 => 3, m => m } }
    fn periodic(&self) -> bool { matches!(self.kind(), 2 | 3) }
    fn count(&self, now: u64) -> u16 {
        let Some(t) = self.ticks(now) else { return 0 };
        let p = self.period();
        if self.periodic() { (p - t % p) as u16 } else { (p.wrapping_sub(t) % 0x10000) as u16 }
    }
    fn out(&self, now: u64) -> bool {
        let Some(t) = self.ticks(now) else { return false };
        let p = self.period();
        match self.kind() {
            3 => t % p < p.div_ceil(2),
            2 => t % p != p - 1,
            _ => t >= p,
        }
    }
    fn status_byte(&self, now: u64) -> u8 {
        (self.out(now) as u8) << 7 | (self.start.is_none() as u8) << 6 | self.access << 4 | self.mode << 1
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Pit { ch: [Channel; 3], speaker: u8, refresh: bool }

impl Default for Pit { fn default() -> Self { Self::new() } }

impl Pit {
    pub fn new() -> Pit {
        let mut ch = [Channel { access: 3, gate: true, ..Channel::default() }; 3];
        ch[2].gate = false;
        Pit { ch, speaker: 0, refresh: false }
    }

    pub fn handles(port: u16) -> bool { matches!(port, 0x40..=0x43 | 0x61) }

    pub fn read(&mut self, port: u16, now: u64) -> u8 {
        if port == 0x61 {
            self.refresh = !self.refresh;
            return self.speaker & 3 | (self.refresh as u8) << 4 | (self.ch[2].out(now) as u8) << 5;
        }
        let Some(c) = self.ch.get_mut((port.wrapping_sub(0x40)) as usize) else { return 0xFF };
        if let Some(s) = c.status.take() { return s; }
        let value = c.latch.unwrap_or_else(|| c.count(now));
        let byte = match c.access {
            1 => { c.latch = None; value as u8 }
            2 => { c.latch = None; (value >> 8) as u8 }
            _ => {
                let hi = c.read_hi;
                c.read_hi = !hi;
                if hi { c.latch = None; (value >> 8) as u8 } else { value as u8 }
            }
        };
        byte
    }

    pub fn write(&mut self, port: u16, v: u8, now: u64) {
        match port {
            0x61 => {
                let was = self.ch[2].gate;
                self.speaker = v & 3;
                self.ch[2].gate = v & 1 != 0;
                if !was && self.ch[2].gate && self.ch[2].start.is_some() { self.ch[2].start = Some(now); self.ch[2].fired = 0; }
            }
            0x43 => {
                let sel = (v >> 6) as usize;
                if sel == 3 {
                    // Read-back: bit 5 clear latches counts, bit 4 clear latches status.
                    for i in 0..3 {
                        if v & (2 << i) == 0 { continue; }
                        let c = &mut self.ch[i];
                        if v & 0x10 == 0 { c.status = Some(c.status_byte(now)); }
                        if v & 0x20 == 0 && c.latch.is_none() { c.latch = Some(c.count(now)); c.read_hi = false; }
                    }
                    return;
                }
                let c = &mut self.ch[sel];
                let access = (v >> 4) & 3;
                if access == 0 {
                    if c.latch.is_none() { c.latch = Some(c.count(now)); c.read_hi = false; }
                    return;
                }
                let gate = c.gate;
                *c = Channel { mode: (v >> 1) & 7, access, gate, ..Channel::default() };
            }
            0x40..=0x42 => {
                let c = &mut self.ch[(port - 0x40) as usize];
                let reload = match c.access {
                    1 => v as u16,
                    2 => (v as u16) << 8,
                    _ if !c.write_hi => { c.pending_lo = v; c.write_hi = true; return; }
                    _ => { c.write_hi = false; c.pending_lo as u16 | (v as u16) << 8 }
                };
                c.reload = reload;
                c.start = Some(now);
                c.fired = 0;
            }
            _ => {}
        }
    }

    /// Channel 0 ended a period or its one-shot since the last poll: raise IRQ 0.
    pub fn poll(&mut self, now: u64) -> bool {
        let c = &mut self.ch[0];
        let Some(t) = c.ticks(now) else { return false };
        let p = c.period();
        if c.periodic() {
            let periods = t / p;
            if periods > c.fired { c.fired = periods; return true; }
            false
        } else if t >= p && c.fired == 0 {
            c.fired = 1;
            true
        } else {
            false
        }
    }

    /// When channel 0 next raises IRQ 0 (ns), for a halted guest's wait.
    pub fn next_irq(&self, now: u64) -> Option<u64> {
        let c = &self.ch[0];
        let start = c.start?;
        let p = c.period();
        let ns = |ticks: u64| start.saturating_add((ticks as u128 * 1_000_000_000 / HZ as u128) as u64 + 1);
        if c.periodic() {
            Some(ns((c.fired + 1).saturating_mul(p)).max(now))
        } else if c.fired == 0 {
            Some(ns(p).max(now))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MS: u64 = 1_000_000;

    #[test]
    fn linux_periodic_tick_at_1000_hz() {
        let mut p = Pit::new();
        let latch = (HZ + 500) / 1000; // 1193
        p.write(0x43, 0x34, 0); // channel 0, lo/hi, mode 2
        p.write(0x40, latch as u8, 0);
        p.write(0x40, (latch >> 8) as u8, 0);
        assert!(!p.poll(MS / 2));
        assert!(p.poll(MS + 1000));
        assert!(!p.poll(MS + 2000), "one interrupt per period");
        assert!(p.poll(10 * MS), "missed periods coalesce");
        assert!(!p.poll(10 * MS));
        let next = p.next_irq(10 * MS).unwrap();
        assert!(next > 10 * MS && next <= 11 * MS + 1000);
    }

    #[test]
    fn one_shot_and_latch() {
        let mut p = Pit::new();
        p.write(0x43, 0x30, 0); // mode 0
        p.write(0x40, 0xFF, 0);
        p.write(0x40, 0xFF, 0);
        p.write(0x43, 0x00, MS); // latch channel 0
        let lo = p.read(0x40, 5 * MS);
        let hi = p.read(0x40, 5 * MS);
        let count = lo as u16 | (hi as u16) << 8;
        assert_eq!(count, 0xFFFF - 1193, "latched at 1 ms");
        assert!(!p.poll(54 * MS));
        assert!(p.poll(56 * MS));
        assert!(!p.poll(200 * MS), "once");
        assert_eq!(p.next_irq(200 * MS), None);
    }

    #[test]
    fn channel_2_calibration_path() {
        let mut p = Pit::new();
        // What pit_calibrate_tsc does: gate on, speaker off, mode 0, 0xFFFF.
        let port_b = p.read(0x61, 0);
        p.write(0x61, (port_b & !2) | 1, 0);
        p.write(0x43, 0xB0, 0);
        p.write(0x42, 0xFF, 0);
        p.write(0x42, 0xFF, 0);
        assert_eq!(p.read(0x61, 10 * MS) & 0x20, 0, "OUT2 low while counting");
        let _lo = p.read(0x42, 10 * MS);
        let msb = p.read(0x42, 10 * MS);
        assert_eq!(msb, ((0xFFFFu64 - 11931) >> 8) as u8);
        assert_eq!(p.read(0x61, 60 * MS) & 0x20, 0x20, "OUT2 high at terminal count");
    }

    #[test]
    fn read_back_status_and_untouched_channel() {
        let mut p = Pit::new();
        assert!(!p.poll(u64::MAX), "no count loaded");
        assert_eq!(p.next_irq(0), None);
        p.write(0x43, 0x36, 0); // ch0 lo/hi mode 3
        p.write(0x43, 0xE2, 0); // read-back status of ch0
        assert_eq!(p.read(0x40, 0), 0x40 | 0x30 | 0x06, "null count, lo/hi, mode 3");
        assert_eq!(p.read(0x44, 0), 0xFF);
    }
}
