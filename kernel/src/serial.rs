use uart_16550::SerialPort;
use spinning_top::Spinlock;
use lazy_static::lazy_static;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

/// True if COM1 at 0x3F8 probed successfully.
static SERIAL_PRESENT: AtomicBool = AtomicBool::new(false);

/// Probe COM1 (0x3F8) for existence before full init.
/// Writes to the scratch register (offset 7) and reads back.
/// If the port doesn't exist or is non-responsive, returns false.
fn probe_com1() -> bool {
    use x86_64::instructions::port::Port;
    let mut scratch: Port<u8> = unsafe { Port::new(0x3F8 + 7) };
    let probe_val: u8 = 0x5A;
    unsafe { scratch.write(probe_val); }
    let readback = unsafe { scratch.read() };
    readback == probe_val
}

lazy_static! {
    pub static ref SERIAL1: Spinlock<Option<SerialPort>> = {
        if probe_com1() {
            let mut serial_port = unsafe { SerialPort::new(0x3F8) };
            serial_port.init();
            SERIAL_PRESENT.store(true, Ordering::Release);
            Spinlock::new(Some(serial_port))
        } else {
            Spinlock::new(None)
        }
    };
}

/// Check if serial port was successfully probed and initialized.
pub fn is_serial_present() -> bool {
    SERIAL_PRESENT.load(Ordering::Acquire)
}

/// Writes to the serial port and to the kernel's log (and its screen, while it has one).
struct Tee<'a>(Option<&'a mut SerialPort>);
impl fmt::Write for Tee<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if let Some(port) = self.0.as_mut() { let _ = port.write_str(s); }
        crate::console::write(s.as_bytes());
        Ok(())
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    use core::fmt::Write;
    use x86_64::instructions::interrupts;

    if crate::console::stopping() { return; }
    interrupts::without_interrupts(|| {
        let mut guard = SERIAL1.lock();
        // With no serial port the log and the screen still get every line.
        let _ = Tee(guard.as_mut()).write_fmt(args);
    });
}

/// The stop path's own writes: no lock, since the core that held it may be halted.
pub fn write_stopped(bytes: &[u8]) {
    if !is_serial_present() { return; }
    use x86_64::instructions::port::Port;
    let mut line_status: Port<u8> = Port::new(0x3F8 + 5);
    let mut data: Port<u8> = Port::new(0x3F8);
    for &b in bytes {
        unsafe {
            for _ in 0..100_000 { if line_status.read() & 0x20 != 0 { break; } }
            data.write(b);
        }
    }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {
        $crate::serial::_print(format_args!($($arg)*));
    };
}

#[macro_export]
macro_rules! serial_println {
    () => ($crate::serial_print!("\n"));
    ($($arg:tt)*) => ($crate::serial_print!("{}\n", format_args!($($arg)*)));
}

/// Successful boot internals are opt-in; faults and structured authority records
/// keep using `serial_println!`. Arguments are not evaluated in normal builds.
#[macro_export]
macro_rules! boot_trace {
    ($($arg:tt)*) => {
        if option_env!("SEXOS_VERBOSE_BOOT") == Some("1") {
            $crate::serial_println!($($arg)*);
        }
    };
}
