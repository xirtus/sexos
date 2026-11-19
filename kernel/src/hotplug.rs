// SPDX-License-Identifier: GPL-3.0-only
//! PCI Express native hot-plug: the mechanism half (stage H1).
//!
//! BOOT records every hot-plug port (Thunderbolt/USB4 root ports, a dock's
//! downstream ports, server slots) and turns its hot-plug interrupts off. Core 0
//! looks at each slot every `LOOK_MS` from the timer tick (three config reads a
//! port) and feeds what it sees to `sex_hotplug::slot`, which owns the rules:
//! settle, power, link, attention button and surprise removal.
//!
//! When devices arrive, this file numbers the buses below the port, sizes every
//! BAR, asks `sex_hotplug::plan` for addresses inside the port's windows and
//! writes them, so the tree decodes. Bridges get memory and bus-master on;
//! endpoints stay off until a driver lease turns them on (stage H2: runtime
//! leases into BOOT-prepared slots). When they leave, everything below the port
//! has decode and bus mastering turned off. Nothing here allocates.
use core::sync::atomic::{AtomicUsize, Ordering};
use sex_hotplug::{pcie, plan, slot};
use crate::hal::pci::PciDevice;

const MAX_PORTS: usize = 16;
const LOOK_MS: u64 = 20;

#[derive(Clone, Copy)]
struct Port { at: PciDevice, cap: u8, link_reporting: bool, slot: slot::Slot }

static PORTS: spin::Mutex<[Option<Port>; MAX_PORTS]> = spin::Mutex::new([None; MAX_PORTS]);
static COUNT: AtomicUsize = AtomicUsize::new(0);

/// The tree being planned: kept off the interrupted stack, only touched with `PORTS` held.
struct Work { nodes: [plan::Node; plan::MAX_NODES], at: [PciDevice; plan::MAX_NODES], placed: [plan::Placed; plan::MAX_NODES] }
static WORK: spin::Mutex<Work> = spin::Mutex::new(Work {
    nodes: [plan::Node { parent: None, bridge: false, bars: [plan::Bar { size: 0, prefetch: false, wide: false }; 6] }; plan::MAX_NODES],
    at: [PciDevice { bus: 0, dev: 0, func: 0, vendor_id: 0, device_id: 0, class_id: 0, subclass_id: 0, prog_if: 0, ecam: 0 }; plan::MAX_NODES],
    placed: [plan::Placed { bars: [0; 6], mem: plan::Window { base: 0, size: 0 }, pref: plan::Window { base: 0, size: 0 } }; plan::MAX_NODES],
});

fn at(bus: u8, dev: u8, func: u8) -> PciDevice {
    PciDevice { bus, dev, func, vendor_id: 0, device_id: 0, class_id: 0, subclass_id: 0, prog_if: 0, ecam: 0 }
}
fn byte(d: &PciDevice, off: u8) -> u8 { (d.read_u32(off & !3) >> ((off & 3) * 8)) as u8 }
fn half(d: &PciDevice, off: u8) -> u16 { (d.read_u32(off & !3) >> ((off & 2) * 8)) as u16 }

/// Offset of the PCI Express capability, if any.
fn express(d: &PciDevice) -> Option<u8> {
    if d.read_u32(0x04) & 1 << 20 == 0 { return None; }
    let mut cap = byte(d, 0x34) & !3;
    for _ in 0..48 {
        if cap < 0x40 { return None; }
        if byte(d, cap) == pcie::CAP_ID { return Some(cap); }
        cap = byte(d, cap + 1) & !3;
    }
    None
}

fn look(p: &Port) -> (slot::Look, u16, u16) {
    let d = &p.at;
    let word = d.read_u32(p.cap + pcie::SLOT_CONTROL);
    let (control, status) = (word as u16, (word >> 16) as u16);
    let present = status & pcie::status::PRESENT != 0;
    let link = if p.link_reporting { half(d, p.cap + pcie::LINK_STATUS) & pcie::LINK_ACTIVE != 0 } else { present };
    let powered = !p.slot.power_controller || control & pcie::control::POWER_OFF == 0;
    (slot::Look { present, link, button: status & pcie::status::BUTTON != 0, powered }, control, status)
}

/// Write slot control and clear the event bits seen, in one 32-bit write.
fn write_control(p: &Port, control: u16, clear: u16) {
    p.at.write_u32(p.cap + pcie::SLOT_CONTROL, control as u32 | ((clear & pcie::status::EVENTS) as u32) << 16);
}

fn name(d: &PciDevice) -> (u8, u8, u8) { (d.bus, d.dev, d.func) }

/// BOOT: remember the hot-plug ports among the devices found.
pub fn init(devices: &[PciDevice]) {
    let mut ports = PORTS.lock();
    let mut n = 0;
    for d in devices {
        if d.class_id != 0x06 || d.subclass_id != 0x04 || d.ecam != 0 { continue; }
        let Some(cap) = express(d) else { continue };
        let caps = half(d, cap + pcie::CAPS);
        let slot_caps = d.read_u32(cap + pcie::SLOT_CAPS);
        if !pcie::is_hotplug_port(caps, slot_caps) { continue; }
        if n == MAX_PORTS { crate::serial_println!("[hotplug.port.skip] reason=table_full"); break; }
        let sc = pcie::SlotCaps::decode(slot_caps);
        let link_reporting = d.read_u32(cap + pcie::LINK_CAPS) & pcie::LINK_ACTIVE_REPORTING != 0;
        let mut port = Port { at: *d, cap, link_reporting, slot: slot::Slot { state: slot::State::Empty, power_controller: sc.power_controller } };
        let (seen, control, _) = look(&port);
        port.slot = slot::Slot::at_boot(seen, sc.power_controller);
        write_control(&port, pcie::quiet_control(control), pcie::status::EVENTS);
        let buses = d.read_u32(0x18);
        let (mem, pref) = plan::windows(d.read_u32(0x20), d.read_u32(0x24), d.read_u32(0x28), d.read_u32(0x2c));
        crate::serial_println!("[hotplug.port] {:02x}:{:02x}.{} {:04x}:{:04x} kind={} slot={} button={} power={} surprise={} present={} link={} buses={:02x}-{:02x} mem={:#x}+{:#x} pref={:#x}+{:#x}",
            d.bus, d.dev, d.func, d.vendor_id, d.device_id, if (caps >> 4) & 0xf == pcie::TYPE_ROOT_PORT as u16 { "root" } else { "downstream" },
            sc.number, sc.button as u8, sc.power_controller as u8, sc.surprise as u8, seen.present as u8, seen.link as u8,
            (buses >> 8) as u8, (buses >> 16) as u8, mem.base, mem.size, pref.base, pref.size);
        ports[n] = Some(port);
        n += 1;
    }
    COUNT.store(n, Ordering::Release);
}

/// Timer tick on core 0, `now` in milliseconds.
pub fn tick(now: u64) {
    if COUNT.load(Ordering::Acquire) == 0 || now % LOOK_MS != 0 { return; }
    // Never wait in an interrupt: a look already running (another core) wins.
    let Some(mut ports) = PORTS.try_lock() else { return };
    for port in ports.iter_mut().flatten() {
        let (seen, control, status) = look(port);
        let step = port.slot.step(now, seen);
        // Quiet what is below before the slot loses power (and QEMU lets it go).
        if step.left { leave(port); }
        let wanted = pcie::control(control, step.power, step.led, None);
        // Every write to slot control is a command and sets "command done": that bit
        // alone is not a reason to write again.
        if wanted != control || status & pcie::status::EVENTS & !pcie::status::COMMAND_DONE != 0 {
            write_control(port, wanted, pcie::status::EVENTS);
        }
        let (b, d, f) = name(&port.at);
        if step.power.is_some() {
            crate::serial_println!("[hotplug.power] port={:02x}:{:02x}.{} on={}", b, d, f, (step.power == Some(true)) as u8);
        }
        if step.arrived { arrive(port); }
        if step.failed { crate::serial_println!("[hotplug.failed] port={:02x}:{:02x}.{} reason=link_never_up", b, d, f); }
        if seen.button && !step.left { crate::serial_println!("[hotplug.button] port={:02x}:{:02x}.{} state={:?}", b, d, f, port.slot.state); }
    }
}

/// Every function present on buses `first..=last`, in bus order.
fn each_below(first: u8, last: u8, mut f: impl FnMut(&PciDevice)) {
    for bus in first..=last {
        for dev in 0..32u8 {
            for func in 0..8u8 {
                let d = at(bus, dev, func);
                let id = d.read_u32(0);
                if id & 0xffff == 0xffff { if func == 0 { break; } continue; }
                f(&d);
                if func == 0 && byte(&d, 0x0e) & 0x80 == 0 { break; }
            }
        }
    }
}

/// Firmware gave the port a bus range of its own; otherwise a walk would run over
/// the root bus (or another port's) and quiet devices that never left.
fn below(port: &PciDevice, first: u8, last: u8) -> bool {
    let ok = first > port.bus && last >= first;
    if !ok { crate::serial_println!("[hotplug.buses.unset] port={:02x}:{:02x}.{} secondary={:02x} subordinate={:02x}", port.bus, port.dev, port.func, first, last); }
    ok
}

fn leave(port: &Port) {
    let buses = port.at.read_u32(0x18);
    let (first, last) = ((buses >> 8) as u8, (buses >> 16) as u8);
    if !below(&port.at, first, last) { return; }
    let mut n = 0;
    // Gone already on a surprise removal; on a button removal, quiet before power-off.
    // A zero status half clears nothing (its bits are write-one-to-clear).
    each_below(first, last.max(first), |d| { d.write_u32(0x04, 0); n += 1; });
    let (b, d, f) = name(&port.at);
    crate::serial_println!("[hotplug.left] port={:02x}:{:02x}.{} functions_quieted={}", b, d, f, n);
}

/// Size one BAR with decode off; returns its request and how many registers it used.
fn size_bar(d: &PciDevice, index: u8) -> (plan::Bar, u8) {
    let at = 0x10 + index * 4;
    let low = d.read_u32(at);
    if low & 1 != 0 { return (plan::Bar::default(), 1); }
    let wide = (low >> 1) & 3 == 2 && index < 5;
    let high = if wide { d.read_u32(at + 4) } else { 0 };
    d.write_u32(at, u32::MAX);
    if wide { d.write_u32(at + 4, u32::MAX); }
    let (ml, mh) = (d.read_u32(at), if wide { d.read_u32(at + 4) } else { 0 });
    d.write_u32(at, low);
    if wide { d.write_u32(at + 4, high); }
    (plan::bar_size(low, ml, mh).unwrap_or_default(), if wide { 2 } else { 1 })
}

fn arrive(port: &Port) {
    let p = &port.at;
    let buses = p.read_u32(0x18);
    let (secondary, subordinate) = ((buses >> 8) as u8, (buses >> 16) as u8);
    if !below(p, secondary, subordinate) { return; }
    let (mem, pref) = plan::windows(p.read_u32(0x20), p.read_u32(0x24), p.read_u32(0x28), p.read_u32(0x2c));
    // The port itself forwards memory and DMA (firmware may have left an empty port off).
    p.write_u32(0x04, (p.read_u32(0x04) & 0xffff) | 0x0006);
    let Some(mut work) = WORK.try_lock() else { return };
    let Work { nodes, at: where_, placed } = &mut *work;
    let mut n = 0usize;
    let mut next_bus = secondary.wrapping_add(1);
    let mut overflow = false;
    // Depth-first: a bridge's buses are numbered before its children are read.
    fn walk(bus: u8, parent: Option<u8>, last: u8, next_bus: &mut u8, nodes: &mut [plan::Node], where_: &mut [PciDevice], n: &mut usize, overflow: &mut bool, depth: u8) {
        for dev in 0..32u8 {
            for func in 0..8u8 {
                let d = at(bus, dev, func);
                let id = d.read_u32(0);
                if id & 0xffff == 0xffff { if func == 0 { break; } continue; }
                let header = byte(&d, 0x0e);
                if *n == nodes.len() { *overflow = true; return; }
                let me = *n;
                *n += 1;
                d.write_u32(0x04, 0);
                let bridge = header & 0x7f == 1;
                let mut bars = [plan::Bar::default(); 6];
                let mut i = 0;
                while i < if bridge { 2 } else { 6 } { let (bar, used) = size_bar(&d, i); bars[i as usize] = bar; i += used; }
                nodes[me] = plan::Node { parent, bridge, bars };
                where_[me] = d;
                if bridge {
                    if *next_bus > last || *next_bus == 0 || depth > 8 { *overflow = true; }
                    else {
                        let sec = *next_bus;
                        *next_bus = next_bus.wrapping_add(1);
                        d.write_u32(0x18, (d.read_u32(0x18) & 0xff00_0000) | (last as u32) << 16 | (sec as u32) << 8 | bus as u32);
                        walk(sec, Some(me as u8), last, next_bus, nodes, where_, n, overflow, depth + 1);
                        let sub = next_bus.wrapping_sub(1);
                        d.write_u32(0x18, (d.read_u32(0x18) & 0xff00_0000) | (sub as u32) << 16 | (sec as u32) << 8 | bus as u32);
                    }
                }
                if func == 0 && header & 0x80 == 0 { break; }
            }
        }
    }
    walk(secondary, None, subordinate, &mut next_bus, &mut nodes[..], &mut where_[..], &mut n, &mut overflow, 0);
    let (b, dv, f) = name(p);
    if overflow { crate::serial_println!("[hotplug.tree.partial] port={:02x}:{:02x}.{} reason=bus_numbers_or_size functions={}", b, dv, f, n); }
    let planned = plan::plan(&nodes[..n], mem, pref, &mut placed[..n]);
    if let Err(e) = planned {
        crate::serial_println!("[hotplug.bar.fail] port={:02x}:{:02x}.{} reason={:?} mem={:#x} pref={:#x}", b, dv, f, e, mem.size, pref.size);
    }
    let mut census = sex_hwdb::Census::default();
    for i in 0..n {
        let d = &where_[i];
        let id = d.read_u32(0);
        let class = d.read_u32(0x08);
        let dev = PciDevice { vendor_id: id as u16, device_id: (id >> 16) as u16, class_id: (class >> 24) as u8,
            subclass_id: (class >> 16) as u8, prog_if: (class >> 8) as u8, ..*d };
        if planned.is_ok() {
            for (index, &address) in placed[i].bars.iter().enumerate() {
                if address == 0 { continue; }
                let at = 0x10 + index as u8 * 4;
                d.write_u32(at, address as u32 | (d.read_u32(at) & 0xf));
                if nodes[i].bars[index].wide { d.write_u32(at + 4, (address >> 32) as u32); }
                crate::serial_println!("[hotplug.bar] {:02x}:{:02x}.{} bar={} address={:#x} bytes={:#x}", d.bus, d.dev, d.func, index, address, nodes[i].bars[index].size);
            }
            if nodes[i].bridge {
                let (lo, bh, lh) = plan::pref_registers(placed[i].pref);
                // I/O window closed (base above limit); secondary status left alone.
                d.write_u32(0x1c, 0x00f0);
                d.write_u32(0x30, 0);
                d.write_u32(0x20, plan::mem_register(placed[i].mem));
                d.write_u32(0x24, lo); d.write_u32(0x28, bh); d.write_u32(0x2c, lh);
                // Memory decode and bus mastering so the tree below passes reads and DMA.
                d.write_u32(0x04, 0x0006);
            }
        }
        crate::devmgr::census_line(&mut census, &dev);
    }
    crate::serial_println!("[hotplug.arrived] port={:02x}:{:02x}.{} functions={} buses={:02x}-{:02x} planned={} lease=deferred",
        b, dv, f, n, secondary, next_bus.wrapping_sub(1).max(secondary), planned.is_ok() as u8);
}
