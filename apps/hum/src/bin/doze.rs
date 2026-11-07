// SPDX-License-Identifier: GPL-3.0-only
//! A program with nothing to do, like most programs most of the time. It sleeps
//! with no timer until told to stop, and an idle machine must not pay for it.
#![no_std]
#![no_main]
use sex_pdx::program_store as store;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { sex_pdx::task_finish(1) }

#[no_mangle]
pub extern "C" fn _start() -> ! {
    loop {
        sex_pdx::sys_wait_self();
        if store::record_call(store::RECORD_STOP_REQUESTED, 0, 0).1 != 0 { sex_pdx::task_finish(0); }
    }
}
