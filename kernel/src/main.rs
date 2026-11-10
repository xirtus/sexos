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
    // The stop screen, a record for the next start, then a restart.
    sex_kernel::console::stop(format_args!("{}", info))
}
