use x86_64::instructions::port::Port;
use core::sync::atomic::{AtomicBool, Ordering};

/// True if i8042 PS/2 controller probed successfully and keyboard was initialized.
static KEYBOARD_READY: AtomicBool = AtomicBool::new(false);

/// Probe i8042 controller at port 0x64 before attempting init.
/// Reads status port; 0xFF on absent hardware indicates no controller.
fn probe_i8042() -> bool {
    let mut status: Port<u8> = unsafe { Port::new(0x64) };
    let s = unsafe { status.read() };
    if s == 0xFF {
        return false;
    }
    true
}

/// Status bits of port 0x64.
const OUTPUT_FULL: u8 = 1 << 0;
const INPUT_FULL: u8 = 1 << 1;
const AUX_DATA: u8 = 1 << 5;
/// Spins while waiting on the controller: real embedded controllers (laptop keyboards)
/// take tens of microseconds per byte, where QEMU's answer is instant.
const SPINS: u32 = 200_000;

fn status() -> u8 { unsafe { Port::<u8>::new(0x64).read() } }

/// Waits until the controller can take a byte.
fn wait_write() -> bool {
    for _ in 0..SPINS { if status() & INPUT_FULL == 0 { return true; } core::hint::spin_loop(); }
    false
}
/// Waits for a byte from the controller.
fn wait_read() -> Option<u8> {
    for _ in 0..SPINS {
        if status() & OUTPUT_FULL != 0 { return Some(unsafe { Port::<u8>::new(0x60).read() }); }
        core::hint::spin_loop();
    }
    None
}
/// Drops whatever the firmware left in the output buffer: a byte left there keeps the
/// IRQ1 line high, so no later key makes an edge and the keyboard looks dead.
fn flush() -> u32 {
    let mut n = 0;
    while status() & OUTPUT_FULL != 0 && n < 64 { unsafe { Port::<u8>::new(0x60).read(); } n += 1; }
    n
}
fn command(byte: u8) -> bool {
    if !wait_write() { return false; }
    unsafe { Port::<u8>::new(0x64).write(byte); }
    true
}
fn data(byte: u8) -> bool {
    if !wait_write() { return false; }
    unsafe { Port::<u8>::new(0x60).write(byte); }
    true
}

pub fn init() {
    if !probe_i8042() {
        crate::serial_println!("[kernel.ps2] present=0 reason=status_ff");
        return;
    }
    let flushed = flush();
    // Read the configuration byte, waiting for the answer.
    let config = if command(0x20) { wait_read() } else { None };
    let Some(mut config) = config.filter(|c| *c != 0xFF) else {
        crate::serial_println!("[kernel.ps2] present=0 reason=no_config flushed={}", flushed);
        return;
    };
    let before = config;
    // Enable IRQ1 (bit 0) and translation (bit 6); clear "keyboard disabled" (bit 4).
    config |= 0x41;
    config &= !0x10;
    let wrote = command(0x60) && data(config);
    let enabled = command(0xAE);
    // Enable scanning and take its ACK (0xFA) here rather than leaving it to IRQ1.
    let ack = if data(0xF4) { wait_read() } else { None };
    let late = flush();
    crate::serial_println!(
        "[kernel.ps2] present=1 config={:#04x}->{:#04x} wrote={} enabled={} ack={:#04x} flushed={} late={}",
        before, config, wrote as u8, enabled as u8, ack.unwrap_or(0), flushed, late);
    KEYBOARD_READY.store(true, Ordering::Release);
}

/// A key byte the controller holds that no IRQ1 brought (the edge was lost, or IRQ1 is
/// not routed on this machine). The input ring's reader calls this when the ring is
/// empty, so a keyboard whose interrupt never arrives still types, at the reader's pace.
pub fn take_pending() -> Option<u8> {
    if !KEYBOARD_READY.load(Ordering::Acquire) { return None; }
    let s = status();
    if s == 0xFF || s & OUTPUT_FULL == 0 || s & AUX_DATA != 0 { return None; }
    let byte = unsafe { Port::<u8>::new(0x60).read() };
    let n = POLLED.fetch_add(1, Ordering::Relaxed) + 1;
    if n <= 4 || n.is_power_of_two() { crate::serial_println!("[kernel.ps2.polled] n={}", n); }
    Some(byte)
}
static POLLED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Returns true if the PS/2 keyboard was successfully probed and initialized.
pub fn is_keyboard_ready() -> bool {
    KEYBOARD_READY.load(Ordering::Acquire)
}

/// Reads a single byte from the PS/2 keyboard data port.
/// Returns None if keyboard was not initialized.
pub fn read_scancode() -> Option<u8> {
    if !KEYBOARD_READY.load(Ordering::Acquire) {
        return None;
    }
    let mut port: Port<u8> = Port::new(0x60);
    Some(unsafe { port.read() })
}

/// Legacy raw read (for callers that don't check readiness — returns 0 if absent).
pub fn read_scancode_raw() -> u8 {
    let mut port: Port<u8> = Port::new(0x60);
    unsafe { port.read() }
}
