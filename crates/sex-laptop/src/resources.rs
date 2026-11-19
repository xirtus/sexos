// SPDX-License-Identifier: GPL-3.0-only
//! `_CRS` resource templates (ACPI 6.5 section 6.4): the parts laptop drivers need.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource<'a> {
    /// An I/O port range (small descriptors 0x47 and 0x4B).
    Io { base: u16, length: u8 },
    /// An I2C target (serial bus connection, type 1): 7- or 10-bit address, bus speed, and
    /// the ACPI path of the controller it hangs off.
    I2c { address: u16, ten_bit: bool, hz: u32, controller: &'a [u8] },
    /// An interrupt wired through a GPIO pin.
    GpioInt { pin: u16, edge: bool, active_low: bool, controller: &'a [u8] },
    /// An extended interrupt (large 0x89), first number only.
    Interrupt { number: u32, edge: bool, active_low: bool },
}

/// Walks a template, calling `f` for each resource it understands.
pub fn parse<'a>(bytes: &'a [u8], mut f: impl FnMut(Resource<'a>)) {
    let mut i = 0;
    while i < bytes.len() {
        let tag = bytes[i];
        if tag & 0x80 == 0 {
            let (kind, len) = (tag >> 3 & 0x0F, (tag & 7) as usize);
            let body = bytes.get(i + 1..i + 1 + len).unwrap_or(&[]);
            match kind {
                0x0F => return, // end tag
                0x08 if body.len() >= 7 => f(Resource::Io { base: u16::from_le_bytes([body[1], body[2]]), length: body[6] }),
                0x09 if body.len() >= 3 => f(Resource::Io { base: u16::from_le_bytes([body[0], body[1]]) & 0x3FF, length: body[2] }),
                _ => {}
            }
            i += 1 + len;
        } else {
            let Some(len) = bytes.get(i + 1..i + 3).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize) else { return };
            let Some(body) = bytes.get(i + 3..i + 3 + len) else { return };
            match tag {
                0x89 if body.len() >= 6 => {
                    let flags = body[0];
                    f(Resource::Interrupt { number: u32::from_le_bytes([body[2], body[3], body[4], body[5]]), edge: flags & 2 != 0, active_low: flags & 4 != 0 });
                }
                0x8C => gpio(body, &mut f),
                0x8E => serial_bus(body, &mut f),
                _ => {}
            }
            i += 3 + len;
        }
    }
}

/// The NUL-terminated resource-source path at `offset` (from the descriptor start, which is
/// three bytes before `body`).
fn source(body: &[u8], offset: u16) -> &[u8] {
    let at = (offset as usize).saturating_sub(3);
    let s = body.get(at..).unwrap_or(&[]);
    &s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]
}

fn gpio<'a>(body: &'a [u8], f: &mut impl FnMut(Resource<'a>)) {
    // revision(1) type(1) general flags(2) int flags(2) pin config(1) drive(2) debounce(2)
    // pin table offset(2) source index(1) source name offset(2) vendor offset(2) vendor len(2)
    if body.len() < 20 || body[1] != 0 { return; }
    let int_flags = u16::from_le_bytes([body[4], body[5]]);
    let pins_at = u16::from_le_bytes([body[11], body[12]]) as usize;
    let name_at = u16::from_le_bytes([body[14], body[15]]);
    let Some(pin) = body.get(pins_at.saturating_sub(3)..pins_at.saturating_sub(3) + 2) else { return };
    f(Resource::GpioInt {
        pin: u16::from_le_bytes([pin[0], pin[1]]),
        edge: int_flags & 1 != 0,
        active_low: int_flags >> 1 & 3 == 1,
        controller: source(body, name_at),
    });
}

fn serial_bus<'a>(body: &'a [u8], f: &mut impl FnMut(Resource<'a>)) {
    // revision(1) source index(1) bus type(1) general flags(1) type flags(2) type revision(1)
    // type data length(2) then for I2C: speed(4) address(2), vendor data, source name.
    if body.len() < 15 || body[2] != 1 { return; }
    let type_flags = u16::from_le_bytes([body[4], body[5]]);
    let data_len = u16::from_le_bytes([body[7], body[8]]) as usize;
    let hz = u32::from_le_bytes([body[9], body[10], body[11], body[12]]);
    let address = u16::from_le_bytes([body[13], body[14]]);
    let name = body.get(9 + data_len..).unwrap_or(&[]);
    let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
    f(Resource::I2c { address, ten_bit: type_flags & 1 != 0, hz, controller: name });
}

/// The embedded controller's ports: the first I/O range is data, the second command.
pub fn ec_ports(crs: &[u8]) -> Option<(u16, u16)> {
    let mut io = [0u16; 2];
    let mut n = 0;
    parse(crs, |r| if let Resource::Io { base, .. } = r { if n < 2 { io[n] = base; n += 1; } });
    (n == 2).then_some((io[0], io[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ec_crs_names_data_then_command() {
        // IO(Decode16, 0x62, 0x62, 0, 1), IO(Decode16, 0x66, 0x66, 0, 1), EndTag.
        let crs = [0x47, 1, 0x62, 0, 0x62, 0, 0, 1, 0x47, 1, 0x66, 0, 0x66, 0, 0, 1, 0x79, 0];
        assert_eq!(ec_ports(&crs), Some((0x62, 0x66)));
        let fixed = [0x4B, 0x62, 0, 1, 0x4B, 0x66, 0, 1, 0x79, 0];
        assert_eq!(ec_ports(&fixed), Some((0x62, 0x66)));
        assert_eq!(ec_ports(&[0x79, 0]), None);
    }

    #[test]
    fn touchpad_crs_gives_i2c_target_and_interrupt() {
        // I2cSerialBusV2(0x2C, ControllerInitiated, 400000, AddressingMode7Bit, "\\_SB.PC00.I2C1")
        let name = b"\\_SB.PC00.I2C1\0";
        let mut i2c = vec![0x8E, 0, 0, 2, 0, 1, 2, 0, 0, 1, 6, 0];
        i2c.extend_from_slice(&400_000u32.to_le_bytes());
        i2c.extend_from_slice(&0x2Cu16.to_le_bytes());
        i2c.extend_from_slice(name);
        let len = (i2c.len() - 3) as u16;
        i2c[1..3].copy_from_slice(&len.to_le_bytes());
        // Interrupt(ResourceConsumer, Level, ActiveLow, Exclusive) {0x5D}
        let int = [0x89, 6, 0, 0x05, 1, 0x5D, 0, 0, 0];
        let mut crs = i2c.clone();
        crs.extend_from_slice(&int);
        crs.extend_from_slice(&[0x79, 0]);
        let mut seen = Vec::new();
        parse(&crs, |r| seen.push(format!("{:?}", r)));
        assert_eq!(seen.len(), 2, "{:?}", seen);
        let mut got = None;
        parse(&crs, |r| if let Resource::I2c { address, hz, controller, ten_bit } = r { got = Some((address, hz, controller.to_vec(), ten_bit)); });
        assert_eq!(got, Some((0x2C, 400_000, b"\\_SB.PC00.I2C1".to_vec(), false)));
        let mut irq = None;
        parse(&crs, |r| if let Resource::Interrupt { number, edge, active_low } = r { irq = Some((number, edge, active_low)); });
        assert_eq!(irq, Some((0x5D, false, true)));
    }

    #[test]
    fn truncated_templates_do_not_panic() {
        for cut in 0..18 {
            let crs = [0x47, 1, 0x62, 0, 0x62, 0, 0, 1, 0x47, 1, 0x66, 0, 0x66, 0, 0, 1, 0x79, 0];
            parse(&crs[..cut], |_| {});
        }
        parse(&[0x8E, 0xFF, 0xFF, 1], |_| {});
        parse(&[0x8C, 4, 0, 1, 0, 0, 0], |_| {});
    }
}
