// SPDX-License-Identifier: GPL-3.0-only
//! The ACPI embedded controller (ACPI 6.5 section 12): a byte-wide mailbox at two I/O
//! ports. Laptop firmware keeps the battery, AC adapter, lid and fan registers in its
//! 256-byte address space and reaches them through `EmbeddedControl` operation regions.

/// Status register bits.
pub const OBF: u8 = 1 << 0;
pub const IBF: u8 = 1 << 1;
pub const SCI_EVT: u8 = 1 << 5;
/// Commands.
pub const RD_EC: u8 = 0x80;
pub const WR_EC: u8 = 0x81;
pub const QR_EC: u8 = 0x84;
/// Default ports, used when firmware names none.
pub const DATA_PORT: u16 = 0x62;
pub const COMMAND_PORT: u16 = 0x66;

/// How many status polls to wait for one handshake step (each poll stalls ~10 us).
pub const SPIN_LIMIT: u32 = 10_000;

/// The two ports, as the server reaches them (through the kernel's firmware window).
pub trait Ports {
    fn status(&mut self) -> u8;
    fn command(&mut self, value: u8);
    fn read_data(&mut self) -> u8;
    fn write_data(&mut self, value: u8);
    /// A short pause between status polls.
    fn pause(&mut self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The controller never took the byte (IBF stayed set).
    InputFull,
    /// The controller never answered (OBF stayed clear).
    NoOutput,
}

pub struct Ec<P: Ports> { pub ports: P }

impl<P: Ports> Ec<P> {
    pub fn new(ports: P) -> Self { Self { ports } }
    fn wait_input_empty(&mut self) -> Result<(), Error> {
        for _ in 0..SPIN_LIMIT {
            if self.ports.status() & IBF == 0 { return Ok(()); }
            self.ports.pause();
        }
        Err(Error::InputFull)
    }
    fn wait_output_full(&mut self) -> Result<(), Error> {
        for _ in 0..SPIN_LIMIT {
            if self.ports.status() & OBF != 0 { return Ok(()); }
            self.ports.pause();
        }
        Err(Error::NoOutput)
    }
    /// Reads one byte of the controller's address space.
    pub fn read(&mut self, address: u8) -> Result<u8, Error> {
        self.wait_input_empty()?;
        self.ports.command(RD_EC);
        self.wait_input_empty()?;
        self.ports.write_data(address);
        self.wait_output_full()?;
        Ok(self.ports.read_data())
    }
    /// Writes one byte of the controller's address space.
    pub fn write(&mut self, address: u8, value: u8) -> Result<(), Error> {
        self.wait_input_empty()?;
        self.ports.command(WR_EC);
        self.wait_input_empty()?;
        self.ports.write_data(address);
        self.wait_input_empty()?;
        self.ports.write_data(value);
        self.wait_input_empty()
    }
    /// Little-endian multi-byte access, one byte at a time (wider fields in EC regions are
    /// split by the firmware's own access width).
    pub fn read_wide(&mut self, address: u8, bytes: usize) -> Result<u64, Error> {
        let mut v = 0u64;
        for i in 0..bytes.min(8) { v |= (self.read(address.wrapping_add(i as u8))? as u64) << (8 * i); }
        Ok(v)
    }
    pub fn write_wide(&mut self, address: u8, bytes: usize, value: u64) -> Result<(), Error> {
        for i in 0..bytes.min(8) { self.write(address.wrapping_add(i as u8), (value >> (8 * i)) as u8)?; }
        Ok(())
    }
    /// Whether the controller has an event waiting.
    pub fn event_pending(&mut self) -> bool { self.ports.status() & SCI_EVT != 0 }
    /// Takes the next event: the number of the `_Qxx` method to run (0 = none).
    pub fn query(&mut self) -> Result<u8, Error> {
        self.wait_input_empty()?;
        self.ports.command(QR_EC);
        self.wait_output_full()?;
        Ok(self.ports.read_data())
    }
}

/// The `_Qxx` method name for a query number, e.g. 0x1A -> "_Q1A".
pub fn query_method(number: u8) -> [u8; 4] {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    [b'_', b'Q', HEX[(number >> 4) as usize], HEX[(number & 15) as usize]]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A controller model: answers RD/WR/QR like the spec's handshake, with a queue of events.
    struct Model { space: [u8; 256], status: u8, state: State, out: u8, events: Vec<u8>, stuck: bool, writes: Vec<(u8, u8)> }
    #[derive(PartialEq)]
    enum State { Idle, ReadAddr, WriteAddr, WriteValue(u8) }
    impl Model {
        fn new() -> Self { Self { space: [0; 256], status: 0, state: State::Idle, out: 0, events: Vec::new(), stuck: false, writes: Vec::new() } }
    }
    impl Ports for Model {
        fn status(&mut self) -> u8 {
            let evt = if self.events.is_empty() { 0 } else { SCI_EVT };
            (self.status | evt) | if self.stuck { IBF } else { 0 }
        }
        fn command(&mut self, v: u8) {
            match v {
                RD_EC => self.state = State::ReadAddr,
                WR_EC => self.state = State::WriteAddr,
                QR_EC => { self.out = if self.events.is_empty() { 0 } else { self.events.remove(0) }; self.status |= OBF; }
                _ => {}
            }
        }
        fn read_data(&mut self) -> u8 { self.status &= !OBF; self.out }
        fn write_data(&mut self, v: u8) {
            match core::mem::replace(&mut self.state, State::Idle) {
                State::ReadAddr => { self.out = self.space[v as usize]; self.status |= OBF; }
                State::WriteAddr => self.state = State::WriteValue(v),
                State::WriteValue(a) => { self.space[a as usize] = v; self.writes.push((a, v)); }
                State::Idle => {}
            }
        }
        fn pause(&mut self) {}
    }

    #[test]
    fn reads_and_writes_bytes() {
        let mut m = Model::new();
        m.space[0x40] = 0x5A;
        let mut ec = Ec::new(m);
        assert_eq!(ec.read(0x40), Ok(0x5A));
        ec.write(0x41, 7).unwrap();
        assert_eq!(ec.ports.space[0x41], 7);
        assert_eq!(ec.ports.writes, vec![(0x41, 7)]);
    }

    #[test]
    fn wide_fields_are_little_endian() {
        let mut m = Model::new();
        m.space[0x10] = 0x34; m.space[0x11] = 0x12;
        let mut ec = Ec::new(m);
        assert_eq!(ec.read_wide(0x10, 2), Ok(0x1234));
        ec.write_wide(0x20, 2, 0xBEEF).unwrap();
        assert_eq!((ec.ports.space[0x20], ec.ports.space[0x21]), (0xEF, 0xBE));
    }

    #[test]
    fn queries_events_in_order() {
        let mut m = Model::new();
        m.events = vec![0x1A, 0x66];
        let mut ec = Ec::new(m);
        assert!(ec.event_pending());
        assert_eq!(ec.query(), Ok(0x1A));
        assert_eq!(ec.query(), Ok(0x66));
        assert!(!ec.event_pending());
        assert_eq!(&query_method(0x1A), b"_Q1A");
    }

    #[test]
    fn a_stuck_controller_times_out() {
        let mut m = Model::new();
        m.stuck = true;
        let mut ec = Ec::new(m);
        assert_eq!(ec.read(0), Err(Error::InputFull));
        // No controller at all reads 0xFF: IBF set forever.
        struct Absent;
        impl Ports for Absent {
            fn status(&mut self) -> u8 { 0xFF }
            fn command(&mut self, _: u8) {}
            fn read_data(&mut self) -> u8 { 0xFF }
            fn write_data(&mut self, _: u8) {}
            fn pause(&mut self) {}
        }
        assert_eq!(Ec::new(Absent).read(0), Err(Error::InputFull));
    }
}
