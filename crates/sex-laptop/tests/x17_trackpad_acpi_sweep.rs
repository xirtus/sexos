// SPDX-License-Identifier: GPL-3.0-only
//! Sweep Test 1: Alienware x17 R1 ACPI `_CRS` (`I2cSerialBusV2`, `GpioInt`, `Interrupt`)
//! and Intel Tiger Lake-H (`INT34C6`) GPIO pad routing sweep.
use sex_laptop::intel_gpio::{self as gpio, Chip, Line, NotRouted, Pad};
use sex_laptop::resources::{self, Resource};

fn build_i2c_v2(addr: u16, ten_bit: bool, hz: u32, controller: &str) -> Vec<u8> {
    let mut b = vec![
        0x8E, 0, 0, // Large tag + placeholder length
        2,          // revision 2 (I2cSerialBusV2)
        0,          // resource source index
        1,          // bus type: I2C
        0x02,       // general flags (ControllerInitiated)
        u8::from(ten_bit), 0, // type-specific flags (bit 0 = 10-bit addressing)
        1,          // type revision
        6, 0,       // type data length = 6 (speed + address)
    ];
    b.extend_from_slice(&hz.to_le_bytes());
    b.extend_from_slice(&addr.to_le_bytes());
    b.extend_from_slice(controller.as_bytes());
    b.push(0);
    let len = (b.len() - 3) as u16;
    b[1..3].copy_from_slice(&len.to_le_bytes());
    b
}

fn build_gpio_int(pin: u16, edge: bool, active_low: bool, controller: &str) -> Vec<u8> {
    let int_flags: u16 = u16::from(edge) | (if active_low { 1 << 1 } else { 0 });
    let mut b = vec![
        0x8C, 0, 0, // Large tag + placeholder length
        1,          // revision
        0,          // type: GpioInt
        0, 0,       // general flags
        int_flags as u8, (int_flags >> 8) as u8,
        0,          // pin config
        0, 0,       // drive strength
        0, 0,       // debounce
        23, 0,      // pin table offset (from descriptor start = 3 + 20)
        0,          // resource source index
        25, 0,      // resource source name offset (3 + 22)
        0, 0,       // vendor offset
        0, 0,       // vendor length
    ];
    b.extend_from_slice(&pin.to_le_bytes());
    b.extend_from_slice(controller.as_bytes());
    b.push(0);
    let len = (b.len() - 3) as u16;
    b[1..3].copy_from_slice(&len.to_le_bytes());
    b
}

#[test]
fn x17_r1_crs_sweep_across_controllers_speeds_and_irq_types() {
    let cases = [
        ("\\_SB.PC00.I2C0", 0x2Cu16, false, 400_000u32, Some((295u16, false, true)), None),
        ("\\_SB.PC00.I2C1", 0x15u16, false, 400_000u32, Some((167u16, true, true)), None),
        ("\\_SB.PC00.I2C2", 0x20u16, false, 100_000u32, None, Some((0x5Du32, false, true))),
        ("\\_SB.PC00.I2C3", 0x38u16, false, 1_000_000u32, Some((416u16, false, false)), None),
        ("\\_SB.PC00.I2C4", 0x2A5u16, true, 100_000u32, None, Some((44u32, true, false))),
    ];
    for (ctrl, addr, ten_bit, hz, gpio_cfg, apic_cfg) in cases {
        let mut crs = build_i2c_v2(addr, ten_bit, hz, ctrl);
        if let Some((pin, edge, active_low)) = gpio_cfg {
            crs.extend_from_slice(&build_gpio_int(pin, edge, active_low, "\\_SB.GPI0"));
        }
        if let Some((num, edge, active_low)) = apic_cfg {
            let flags = 0x01 | if edge { 2 } else { 0 } | if active_low { 4 } else { 0 };
            crs.extend_from_slice(&[0x89, 6, 0, flags, 1]);
            crs.extend_from_slice(&num.to_le_bytes());
        }
        crs.extend_from_slice(&[0x79, 0]); // EndTag

        let mut got_i2c = None;
        let mut got_gpio = None;
        let mut got_irq = None;
        resources::parse(&crs, |r| match r {
            Resource::I2c { address, ten_bit, hz, controller } => got_i2c = Some((address, ten_bit, hz, controller.to_vec())),
            Resource::GpioInt { pin, edge, active_low, controller } => got_gpio = Some((pin, edge, active_low, controller.to_vec())),
            Resource::Interrupt { number, edge, active_low } => got_irq = Some((number, edge, active_low)),
            _ => {}
        });
        assert_eq!(got_i2c, Some((addr, ten_bit, hz, ctrl.as_bytes().to_vec())));
        assert_eq!(got_gpio, gpio_cfg.map(|(p, e, l)| (p, e, l, b"\\_SB.GPI0".to_vec())));
        assert_eq!(got_irq, apic_cfg);
    }
}

#[test]
fn x17_r1_tgl_h_gpio_routing_and_polling_fallback_sweep() {
    let chip = Chip::from_hid("INT34C6").expect("TGL-H GPIO chip INT34C6 must be known");
    assert_eq!(chip.name, "tgl-h");
    assert_eq!(chip.communities(), 5);

    // Typical Dell/Alienware Tiger Lake-H touchpad pins across communities.
    for (pin, expected_comm) in [(0u16, 0usize), (64, 0), (160, 1), (288 + 7, 2), (416 + 3, 3), (448 + 5, 4)] {
        let (comm, _pad) = chip.locate(pin).expect("pin must locate in TGL-H");
        assert_eq!(comm, expected_comm);
        let off = gpio::pad_offset(0x0094_0000, 0x700, 7);
        assert_eq!(off, 0x700 + 7 * 16);
    }

    // Pad routed to IOAPIC with RXINV: active-low pin becomes active-high at IOAPIC.
    let apic_pad = Pad { dw0: (1 << 20) | (1 << 23) | (1 << 8), dw1: 0x5D };
    assert_eq!(apic_pad.apic_line(true), Ok(Line { gsi: 0x5D, edge: false, active_low: false }));

    // Pad NOT routed to IOAPIC (PCH GPIO controller shared IRQ): must report NotApicRouted -> sexinput polls!
    let gpio_shared_pad = Pad { dw0: (1 << 23) | (1 << 8), dw1: 0x5D };
    assert_eq!(gpio_shared_pad.apic_line(true), Err(NotRouted::NotApicRouted));

    // Native-mode pad or RX-disabled pad: must report NotGpioInput -> sexinput polls!
    assert_eq!(Pad { dw0: (1 << 10) | (1 << 20), dw1: 40 }.apic_line(true), Err(NotRouted::NotGpioInput));
    assert_eq!(Pad { dw0: (1 << 9) | (1 << 20), dw1: 40 }.apic_line(true), Err(NotRouted::NotGpioInput));
}
