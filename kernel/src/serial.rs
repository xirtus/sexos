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
/// A line is gathered first and goes out a FIFO at a time: one line-status read, then
/// up to 16 bytes in one `rep outsb`. The byte loop it replaces read the line status
/// before every byte, two port accesses (two exits to QEMU under KVM) per byte, which
/// made the kernel's own boot log most of its time from entry to handing over to services.
struct Tee<'a> { port: Option<&'a mut SerialPort>, line: [u8; 256], len: usize }
impl Tee<'_> {
    fn flush(&mut self) {
        let bytes = &self.line[..self.len];
        if let Some(port) = self.port.as_mut() {
            // The crate's writer spells backspace and DEL out; keep that for such lines.
            if bytes.iter().any(|b| *b == 8 || *b == 0x7F) {
                for b in bytes { port.send(*b); }
            } else {
                write_fifo(bytes);
            }
        }
        crate::console::write(bytes);
        self.len = 0;
    }
}
impl fmt::Write for Tee<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for chunk in s.as_bytes().chunks(self.line.len()) {
            if self.len + chunk.len() > self.line.len() { self.flush(); }
            self.line[self.len..self.len + chunk.len()].copy_from_slice(chunk);
            self.len += chunk.len();
        }
        Ok(())
    }
}

/// COM1 with its FIFO on (`SerialPort::init` sets FCR 0xC7): an empty transmitter
/// takes 16 bytes.
fn write_fifo(bytes: &[u8]) {
    use x86_64::instructions::port::Port;
    let mut line_status: Port<u8> = Port::new(0x3F8 + 5);
    for chunk in bytes.chunks(16) {
        unsafe {
            for _ in 0..100_000 { if line_status.read() & 0x20 != 0 { break; } }
            core::arch::asm!("rep outsb", in("dx") 0x3f8u16, inout("rsi") chunk.as_ptr() => _,
                inout("rcx") chunk.len() => _, options(nostack, preserves_flags, readonly));
        }
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
        let mut tee = Tee { port: guard.as_mut(), line: [0; 256], len: 0 };
        let _ = tee.write_fmt(args);
        tee.flush();
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
