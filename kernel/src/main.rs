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
    serial_println!("KERNEL PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}
