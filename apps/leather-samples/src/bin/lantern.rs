// SPDX-License-Identifier: GPL-3.0-only
#![no_std]
#![no_main]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[leather.fatal] {}", info);
    sex_pdx::task_finish(1)
}
extern "C" fn main() -> ! {
    leather_samples::program::run(leather_samples::Kind::Lantern)
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    sex_app::with_stack(1 << 20, main)
}
