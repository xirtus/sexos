// SPDX-License-Identifier: GPL-3.0-only
//! What sexacpi can drive of a FADT before handing it to the AML interpreter.
//! sexacpi reaches hardware through the kernel's firmware window (port, memory and
//! PCI calls) and maps nothing, but the `acpi` crate maps every PM1 register block
//! in system memory (hardware-reduced and some memory-mapped platforms put them
//! there), which would stop sexacpi at boot. Such a FADT is refused instead
//! (found by fuzz/drivers drv_acpi_aml). Shared with that fuzz target.
use acpi::address::AddressSpace;
use acpi::sdt::fadt::Fadt;

/// The first PM1 block that is not in I/O space (or that the FADT cannot even
/// describe), by name; None when every present block is an I/O port range.
pub fn pm1_not_io(fadt: &Fadt) -> Option<&'static str> {
    let io = |g: acpi::address::GenericAddress| g.address_space == AddressSpace::SystemIo;
    match fadt.pm1a_event_block() { Ok(g) if io(g) => {}, _ => return Some("pm1a_event") }
    match fadt.pm1b_event_block() { Ok(None) => {}, Ok(Some(g)) if io(g) => {}, _ => return Some("pm1b_event") }
    match fadt.pm1a_control_block() { Ok(g) if io(g) => {}, _ => return Some("pm1a_control") }
    match fadt.pm1b_control_block() { Ok(None) => {}, Ok(Some(g)) if io(g) => {}, _ => return Some("pm1b_control") }
    None
}
