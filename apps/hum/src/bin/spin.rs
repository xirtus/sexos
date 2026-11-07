// SPDX-License-Identifier: GPL-3.0-only
//! A rude program: never sleeps and ignores every request to stop.
#![no_std]
#![no_main]

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { sex_pdx::task_finish(1) }

#[no_mangle]
pub extern "C" fn _start() -> ! {
    loop { core::hint::spin_loop(); }
}
