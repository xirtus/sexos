// SPDX-License-Identifier: GPL-3.0-only
//! Cufflinks: SexOS's Bluetooth host stack, a ring-3 server (no BlueZ, no Linux bridge).
//!
//! SexUSB carries HCI packets to and from the controller through the `sex_pdx::bt_hci`
//! region; this server runs `sex_bt` on them: Intel firmware load (from the firmware
//! pack region when BOOT grants one), controller bring-up, discovery and pairing.
//!
//! Boot log: `[cufflinks.start]`, `[cufflinks.transport]`, `[cufflinks.controller]`,
//! `[cufflinks.firmware] ...`, `[cufflinks.ready] addr=...`, `[cufflinks.device] addr=...
//! name=... rssi=...`, `[cufflinks.pair] ...`.
//!
//! W1 builds this server but BOOT does not start it yet: it needs a BOOT row and the two
//! region grants (kernel `boot_memory.rs`), which belong to a kernel-owning thread.
#![no_std]
#![no_main]
extern crate alloc;

use alloc::boxed::Box;
use sex_bt::hci::{self, Event};
use sex_bt::host::{Host, Note};
use sex_bt::intel::{self, Loader, Outcome};
use sex_bt::report::Transport;
use sex_pdx::bt_hci::{self, Shared};
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::serial_record as log;

const HEAP_BYTES: usize = 1024 * 1024;
#[repr(align(4096))]
#[allow(dead_code)]
struct HeapBytes([u8; HEAP_BYTES]);
static mut HEAP: HeapBytes = HeapBytes([0; HEAP_BYTES]);
#[global_allocator]
static ALLOCATOR: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log!("[cufflinks.panic] {}", info);
    park()
}

fn park() -> ! { loop { let _ = sex_pdx::sys_wait_self_ticks(100); } }

/// Random bytes for keys and nonces: RDRAND, mixed with the time stamp counter when the
/// CPU has none. (W2: take the system entropy grant instead.)
fn random(out: &mut [u8]) {
    for chunk in out.chunks_mut(8) {
        let mut v: u64 = 0;
        let mut ok: u8 = 0;
        for _ in 0..10 {
            unsafe { core::arch::asm!("rdrand {v}", "setc {ok}", v = out(reg) v, ok = out(reg_byte) ok); }
            if ok != 0 { break; }
        }
        if ok == 0 {
            let t = unsafe { core::arch::x86_64::_rdtsc() };
            v ^= t.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(29);
        }
        chunk.copy_from_slice(&v.to_le_bytes()[..chunk.len()]);
    }
}

/// The firmware pack, if BOOT granted it.
fn firmware_pack() -> Option<&'static [u8]> {
    let v = sex_pdx::memory_region_full_view(bt_hci::FIRMWARE_SLOT, QUERY_VIEW | READ).ok()?;
    if v.virtual_address == 0 || v.length == 0 { return None; }
    Some(unsafe { core::slice::from_raw_parts(v.virtual_address as *const u8, v.length as usize) })
}

fn send(ring: &Shared, packet: &[u8], bulk: bool) {
    let mut p = alloc::vec::Vec::from(packet);
    if bulk && p.first() == Some(&hci::COMMAND) { p[0] = bt_hci::COMMAND_BULK; }
    while !ring.tx_push(&p) { let _ = sex_pdx::sys_wait_self_ticks(1); }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe { ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP) as *mut u8, HEAP_BYTES); }
    log!("[cufflinks.start]");
    let view = sex_pdx::memory_region_full_view(bt_hci::SLOT, QUERY_VIEW | READ | WRITE).ok()
        .filter(|v| v.length >= bt_hci::BYTES && v.virtual_address != 0);
    let Some(view) = view else {
        log!("[cufflinks.transport] missing=bt_hci slot={} region={:#x}", bt_hci::SLOT, bt_hci::REGION_ID);
        park();
    };
    let ring = unsafe { Shared::at(view.virtual_address as *mut u8) };
    log!("[cufflinks.transport] ok=1 slot={}", bt_hci::SLOT);
    let mut packet = [0u8; bt_hci::MAX_PACKET];
    loop {
        // Wait for SexUSB to report a controller.
        let ((vendor, product), generation) = loop {
            if let Some(l) = ring.link() { break l; }
            let _ = sex_pdx::sys_wait_self_ticks(10);
        };
        let name = intel::part(vendor, product).map(|p| p.name).unwrap_or("unknown");
        log!("[cufflinks.controller] vendor={:#06x} product={:#06x} part=\"{}\"", vendor, product, name);
        if vendor == intel::INTEL_VENDOR && !load_intel_firmware(&ring, generation, &mut packet) { continue; }
        run_host(&ring, generation, &mut packet);
    }
}

fn still(ring: &Shared, generation: u32) -> bool { ring.link().is_some_and(|l| l.1 == generation) }

/// Load Intel firmware. False: the controller left or the load failed.
fn load_intel_firmware(ring: &Shared, generation: u32, packet: &mut [u8]) -> bool {
    let mut l = Loader::new();
    let mut idle = 0u32;
    while still(ring, generation) {
        if let Some((sfi, ddc)) = l.wants_firmware() {
            let pack = firmware_pack();
            let Some(file) = pack.and_then(|p| sex_bt::pack::find(p, &sfi)) else {
                log!("[cufflinks.firmware] missing={} pack={}", sfi, pack.is_some() as u8);
                return false;
            };
            let ddc_file = pack.and_then(|p| sex_bt::pack::find(p, &ddc));
            log!("[cufflinks.firmware] file={} bytes={} ddc={}", sfi, file.len(), ddc_file.is_some() as u8);
            if let Err(e) = l.provide(file, ddc_file) { log!("[cufflinks.firmware] failed={:?}", e); return false; }
        }
        if let Some(o) = l.next() { send(ring, &o.packet, o.bulk); }
        let mut moved = false;
        while let Some(n) = ring.rx_pop(packet) {
            moved = true;
            let p = intel::bootloader_event(&packet[..n]).unwrap_or_else(|| packet[..n].to_vec());
            let Some(e) = hci::parse_event(&p) else { continue };
            if let Err(err) = l.on_event(&e) { log!("[cufflinks.firmware] failed={:?}", err); return false; }
            if let (Some(v), true) = (l.version, matches!(e, Event::CommandComplete { opcode: intel::OP_READ_VERSION, .. })) {
                log!("[cufflinks.firmware] variant={:#04x} bootloader={} file={}", v.hw_variant(), v.bootloader() as u8,
                    v.firmware_base().unwrap_or_default());
            }
        }
        match l.outcome() {
            Some(Outcome::AlreadyRunning) => { log!("[cufflinks.firmware] state=operational loaded=0"); return true; }
            Some(Outcome::Loaded) => { log!("[cufflinks.firmware] state=operational loaded=1 boot={:#x} fragments={}", l.boot_addr(), l.sent_fragments); return true; }
            None => {}
        }
        if moved { idle = 0; } else {
            idle += 1;
            // The controller verifies the image and reboots; give it about five seconds.
            if idle > 5000 { log!("[cufflinks.firmware] failed=timeout"); return false; }
            let _ = sex_pdx::sys_wait_self_ticks(1);
        }
    }
    false
}

fn run_host(ring: &Shared, generation: u32, packet: &mut [u8]) {
    let mut h = Host::new(Box::new(random));
    let mut scanning = false;
    while still(ring, generation) {
        let mut moved = false;
        while let Some(p) = h.poll() { send(ring, &p, false); moved = true; }
        while let Some(n) = ring.rx_pop(packet) { h.on_packet(&packet[..n]); moved = true; }
        while let Some(n) = h.note() {
            moved = true;
            match n {
                Note::Ready(i) => {
                    log!("[cufflinks.ready] addr={} hci={} lmp_subversion={:#06x} manufacturer={} le={} ssp={} ext_scan={}",
                        i.addr, i.hci_version, i.lmp_subversion, i.manufacturer, i.le() as u8, i.ssp() as u8, i.le_extended_advertising() as u8);
                    if !scanning { h.scan(8); scanning = true; }
                }
                Note::Device(f) => log!("[cufflinks.device] addr={} type={} name=\"{}\" rssi={} class={:#08x} appearance={:#06x}",
                    f.addr, if f.transport == Transport::Le { "le" } else { "classic" }, f.name.as_deref().unwrap_or(""),
                    f.rssi.unwrap_or(127), f.class.unwrap_or(0), f.appearance.unwrap_or(0)),
                Note::Connected { handle, addr, le } => log!("[cufflinks.connected] handle={:#06x} addr={} le={}", handle, addr, le as u8),
                Note::Disconnected { handle, reason } => log!("[cufflinks.disconnected] handle={:#06x} reason={:#04x}", handle, reason),
                // No desktop prompt until W2: numeric comparison is answered yes and logged.
                Note::Compare { handle, addr, value } => {
                    log!("[cufflinks.pair] addr={} compare={:06} accepted=auto", addr, value);
                    h.confirm(handle, true);
                }
                Note::Paired { addr, le } => log!("[cufflinks.pair] addr={} le={} paired=1", addr, le as u8),
                Note::PairingFailed { addr, reason } => log!("[cufflinks.pair] addr={} paired=0 reason={:#04x}", addr, reason),
                Note::Encrypted { handle } => log!("[cufflinks.encrypted] handle={:#06x}", handle),
                Note::Refused { opcode, status } => log!("[cufflinks.refused] opcode={:#06x} status={:#04x}", opcode, status),
                Note::Failed(e) => { log!("[cufflinks.failed] step=bringup reason={:?}", e); return; }
            }
        }
        if !moved { let _ = sex_pdx::sys_wait_self_ticks(1); }
    }
    log!("[cufflinks.gone] generation={}", generation);
}
