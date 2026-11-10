#![no_std]
#![no_main]

use sex_kernel::kernel_init;

#[no_mangle]
extern "C" fn _start() -> ! {
    kernel_init();
    loop {
        core::hint::spin_loop();
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use sex_kernel::serial_println;
    // Firmware AML runs quarantined: its panics return to the ACPI code instead.
    sex_kernel::hal::acpi::panic_escape(info);
    serial_println!("KERNEL PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}
