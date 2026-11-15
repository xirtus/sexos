// SPDX-License-Identifier: GPL-3.0-only
//! Cufflinks: SexOS's Bluetooth host stack, a ring-3 server (no BlueZ, no Linux bridge).
//!
//! SexUSB carries HCI packets to and from the controller through the `sex_pdx::bt_hci`
//! region; this server runs `sex_bt` on them: Intel firmware load (from the firmware
//! pack region when BOOT grants one), controller bring-up, discovery and pairing.
//!
//! Boot log: `[cufflinks.start]`, `[cufflinks.transport]`, `[cufflinks.controller]`,
//! `[cufflinks.firmware] ...`, `[cufflinks.ready] addr=...`, `[cufflinks.bonds] ...`,
//! `[cufflinks.device] addr=... name=... rssi=...`, `[cufflinks.pair] ...`,
//! `[cufflinks.hid] addr=... keyboard=1 pointer=0 ...`.
//!
//! W2: keyboards and mice (classic HID boot protocol, HID over GATT) send their input
//! through the region's HID mailbox to SexUSB, which hands it to Input with USB's own.
//! Bonds are kept by Files (`store.rs`); who may pair is decided in `pairing.rs`.
//!
//! W3: headphones (A2DP Source, SBC): Bell's mix comes through the `bt_audio` region
//! shared with Bell and goes to the headset (`audio.rs`).
//!
//! BOOT starts it as domain `bt_hci::CUFFLINKS_PD` (Senses organ) with the HCI region
//! shared with SexUSB, the firmware pack and a route to Files. With no controller it
//! logs `[cufflinks.idle] controller=none` and sleeps in `sys_wait_self` until SexUSB
//! sends `bt_hci::OP_LINK`: an idle Cufflinks takes no CPU.
#![no_std]
#![no_main]
extern crate alloc;

mod audio;
mod pairing;
mod store;

use alloc::boxed::Box;
use sex_bt::hci::{self, Event};
use sex_bt::hid::Input;
use sex_bt::host::{Host, Note};
use sex_bt::intel::{self, Loader, Outcome};
use sex_bt::report::Transport;
use sex_pdx::bt_hci::{self, HidInput, Shared};
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

/// Sleep for good: only messages wake us, and each is answered.
fn park() -> ! { loop { serve(); sex_pdx::sys_wait_self(); } }

/// Answer what arrived: SexUSB's link notes (the link itself is read from the region).
fn serve() {
    while let Some(m) = sex_pdx::pdx_try_listen_raw(0) {
        if m.type_id <= 1 { continue; }
        let _ = sex_pdx::pdx_reply(m.caller_pd, if m.type_id == bt_hci::OP_LINK { 0 } else { u64::MAX });
    }
}

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
    // A packet the ring can never take would wait forever: drop it (the host caps ACL
    // data at sex_bt::host::MAX_ACL_DATA, so this is only a guard).
    if p.len() < 2 || p.len() > bt_hci::MAX_PACKET { return; }
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
    audio::Audio::probe();
    let mut packet = [0u8; bt_hci::MAX_PACKET];
    loop {
        // Wait for SexUSB to report a controller: it sends OP_LINK when one comes.
        let mut idle = false;
        let ((vendor, product), generation) = loop {
            serve();
            if let Some(l) = ring.link() { break l; }
            if !idle { log!("[cufflinks.idle] controller=none"); idle = true; }
            sex_pdx::sys_wait_self();
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

/// The HID devices connected now: connection handle -> the small device number SexUSB
/// keys their held keys by.
struct HidDevices { slots: [Option<u16>; 8] }

impl HidDevices {
    fn id(&self, handle: u16) -> Option<u8> { self.slots.iter().position(|s| *s == Some(handle)).map(|i| i as u8 + 1) }
    fn add(&mut self, handle: u16) -> Option<u8> {
        if let Some(id) = self.id(handle) { return Some(id); }
        let at = self.slots.iter().position(|s| s.is_none())?;
        self.slots[at] = Some(handle);
        Some(at as u8 + 1)
    }
    fn remove(&mut self, handle: u16) -> Option<u8> {
        let id = self.id(handle)?;
        self.slots[id as usize - 1] = None;
        Some(id)
    }
    fn count(&self) -> u32 { self.slots.iter().filter(|s| s.is_some()).count() as u32 }
}

fn push_input(ring: &Shared, input: HidInput) {
    // SexUSB drains the mailbox every pass while HID links are up; wait briefly if full.
    for _ in 0..100 { if ring.hid_push(input) { return; } let _ = sex_pdx::sys_wait_self_ticks(1); }
    log!("[cufflinks.hid.dropped]");
}

fn run_host(ring: &Shared, generation: u32, packet: &mut [u8]) {
    let mut h = Host::new(Box::new(random));
    let mut scanning = false;
    let mut policy = pairing::Policy::new(0, 1);
    let mut tried: alloc::vec::Vec<sex_bt::hci::Addr> = alloc::vec::Vec::new();
    let mut hid = HidDevices { slots: [None; 8] };
    ring.set_hid_links(0);
    let mut sound = audio::Audio::new();
    // Classic links from bonded devices that may be headsets: if one opens AVDTP but
    // sets no stream up within two seconds, we do (handle, since).
    // (handle, since, signaling open): the 2 s setup window starts when AVDTP signaling opens.
    let mut quiet: alloc::vec::Vec<(u16, u64, bool)> = alloc::vec::Vec::new();
    while still(ring, generation) {
        let mut moved = false;
        while let Some(p) = h.poll() { send(ring, &p, false); moved = true; }
        while let Some(n) = ring.rx_pop(packet) { h.on_packet(&packet[..n]); moved = true; }
        let now = sex_pdx::get_ticks();
        while let Some(n) = h.note() {
            moved = true;
            match n {
                Note::Ready(i) => {
                    log!("[cufflinks.ready] addr={} hci={} lmp_subversion={:#06x} manufacturer={} le={} ssp={} ext_scan={}",
                        i.addr, i.hci_version, i.lmp_subversion, i.manufacturer, i.le() as u8, i.ssp() as u8, i.le_extended_advertising() as u8);
                    h.set_bonds(store::load());
                    policy = pairing::Policy::new(now, h.bonds.len());
                    log!("[cufflinks.pairing] bonds={} window_s={}", h.bonds.len(), policy.window_seconds(now));
                    if !scanning { h.scan(8); scanning = true; }
                }
                Note::Device(f) => {
                    log!("[cufflinks.device] addr={} type={} name=\"{}\" rssi={} class={:#08x} appearance={:#06x}",
                        f.addr, if f.transport == Transport::Le { "le" } else { "classic" }, f.name.as_deref().unwrap_or(""),
                        f.rssi.unwrap_or(127), f.class.unwrap_or(0), f.appearance.unwrap_or(0));
                    // A new keyboard or mouse: only inside the pairing window (see pairing.rs).
                    let new = h.bonds.le_for(&f.addr).is_none() && h.bonds.classic_for(&f.addr).is_none();
                    if new && Host::is_audio(&f) && f.transport == Transport::Classic && !tried.contains(&f.addr) {
                        tried.push(f.addr);
                        if policy.connect_new(now) {
                            log!("[cufflinks.audio.connect] addr={} name=\"{}\" window=1", f.addr, f.name.as_deref().unwrap_or(""));
                            h.connect_audio(&f);
                        } else {
                            log!("[cufflinks.audio.skip] addr={} reason=no_prompt", f.addr);
                        }
                    }
                    if new && Host::is_hid(&f) && f.connectable && !tried.contains(&f.addr) {
                        if policy.connect_new(now) {
                            tried.push(f.addr);
                            log!("[cufflinks.hid.connect] addr={} name=\"{}\" window=1", f.addr, f.name.as_deref().unwrap_or(""));
                            h.connect_hid(&f);
                        } else {
                            log!("[cufflinks.hid.skip] addr={} reason=no_prompt", f.addr);
                            tried.push(f.addr);
                        }
                    }
                }
                Note::Connected { handle, addr, le } => {
                    log!("[cufflinks.connected] handle={:#06x} addr={} le={}", handle, addr, le as u8);
                    if !le { quiet.push((handle, now, false)); }
                }
                Note::Disconnected { handle, reason } => {
                    log!("[cufflinks.disconnected] handle={:#06x} reason={:#04x}", handle, reason);
                    sound.gone(handle);
                    quiet.retain(|q| q.0 != handle);
                    if let Some(device) = hid.remove(handle) {
                        push_input(ring, HidInput::Gone { device });
                        ring.set_hid_links(hid.count());
                        // Look for it again: a bonded device reconnects when it advertises.
                        h.restart_le_scan();
                    }
                }
                Note::Compare { handle, addr, value } => {
                    // TODO(W2b): ask the desktop (pairing.rs). Until then: the window decides.
                    let yes = policy.accept_compare(now);
                    if yes { log!("[cufflinks.pair] addr={} compare={:06} accepted=auto window=1", addr, value); }
                    else { log!("[cufflinks.pair] addr={} compare={:06} accepted=0 reason=no_prompt", addr, value); }
                    h.confirm(handle, yes);
                }
                Note::Paired { addr, le } => log!("[cufflinks.pair] addr={} le={} paired=1", addr, le as u8),
                Note::PairingFailed { addr, reason } => log!("[cufflinks.pair] addr={} paired=0 reason={:#04x}", addr, reason),
                Note::Encrypted { handle } => log!("[cufflinks.encrypted] handle={:#06x}", handle),
                Note::Refused { opcode, status } => log!("[cufflinks.refused] opcode={:#06x} status={:#04x}", opcode, status),
                Note::Failed(e) => { log!("[cufflinks.failed] step=bringup reason={:?}", e); return; }
                Note::BondsChanged => { store::save(&h.bonds); }
                Note::HidReady { handle, addr, le, keyboard, pointer, boot } => {
                    let device = hid.add(handle).unwrap_or(0);
                    ring.set_hid_links(hid.count());
                    log!("[cufflinks.hid] addr={} handle={:#06x} device={} le={} keyboard={} pointer={} boot={}",
                        addr, handle, device, le as u8, keyboard as u8, pointer as u8, boot as u8);
                }
                Note::HidFailed { handle, code } => log!("[cufflinks.hid.failed] handle={:#06x} code={:#04x}", handle, code),
                Note::AudioSink { handle, sink } => log!("[cufflinks.audio.sink] handle={:#06x} psm={:#06x} avdtp={:#06x} a2dp={:#06x} features={:#06x}",
                    handle, sink.psm, sink.avdtp_version, sink.a2dp_version, sink.features),
                Note::AudioReady { handle, addr, config, frames_per_packet } => {
                    log!("[cufflinks.audio] addr={} handle={:#06x} rate={} mode={:?} blocks={} subbands={} bitpool={} frame_bytes={} frames_per_packet={}",
                        addr, handle, config.rate, config.mode, config.blocks, config.subbands, config.bitpool, config.frame_bytes(), frames_per_packet);
                    sound.ready(handle);
                }
                Note::AudioStreaming { handle } => { log!("[cufflinks.audio.streaming] handle={:#06x}", handle); sound.streaming(handle); }
                Note::AudioSuspended { handle } => log!("[cufflinks.audio.suspended] handle={:#06x}", handle),
                Note::AudioClosed { handle } => { log!("[cufflinks.audio.closed] handle={:#06x}", handle); sound.gone(handle); }
                Note::AudioFailed { handle, code } => log!("[cufflinks.audio.failed] handle={:#06x} code={:#06x}", handle, code),
                Note::Input { handle, input } => {
                    let Some(device) = hid.id(handle) else { continue };
                    push_input(ring, match input {
                        Input::Keys { modifiers, keys } => HidInput::Keys { device, modifiers, keys: Input::packed_keys(&keys) },
                        Input::Pointer { buttons, dx, dy, wheel } => HidInput::Pointer { device, buttons, dx, dy, wheel },
                    });
                }
            }
        }
        // A bonded headset that paged us and opened AVDTP without setting a stream up.
        quiet.retain_mut(|q| {
            let (handle, since, open) = *q;
            if !open {
                // Wait for the headset's signaling channel (for at most 30 s), then give
                // it 2 s to set a stream up itself.
                if h.audio_state(handle).is_some() { *q = (handle, now, true); return true; }
                return now.saturating_sub(since) < 30_000;
            }
            if now.saturating_sub(since) < 2_000 { return true; }
            if h.audio_state(handle) == Some(sex_bt::avdtp::State::Idle) {
                log!("[cufflinks.audio.start] handle={:#06x} initiator=1", handle);
                h.start_audio(handle);
            }
            false
        });
        if sound.pump(&mut h, now) { moved = true; }
        if !moved { serve(); let _ = sex_pdx::sys_wait_self_ticks(1); }
    }
    if let Some(handle) = sound.handle() { sound.gone(handle); }
    for device in 1..=8 { if hid.slots[device as usize - 1].is_some() { push_input(ring, HidInput::Gone { device }); } }
    ring.set_hid_links(0);
    log!("[cufflinks.gone] generation={}", generation);
}
