// SPDX-License-Identifier: GPL-3.0-only
//! Trusted recorder; BOOT capture authority remains separately provisioned.
#![no_std]
#![no_main]
mod capture;
mod files;
mod runtime;
mod session;
use sex_app::{Event, Window};
use sex_pdx::desktop_recording::HEAP_LIMIT;
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[record.fatal] {}", info);
    // Main-task retirement is independently fenced by Display's held lifecycle.
    sex_pdx::task_finish(1)
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    if sex_app::heap::set_limit(HEAP_LIMIT).is_err() {
        sex_pdx::task_finish(1)
    }
    let mut window = Window::open();
    if window.target != sex_pdx::desktop_recording::TARGET {
        sex_pdx::task_finish(1)
    }
    window.title(b"Screen recorder");
    window.show(b"Whole desktop recording: 640 x 360, up to 5 fps.\nVideo only; maximum 60 seconds.\nStart/active/saved status appears in Spindle.\nStop from the desktop recording indicator or record-stop.");
    let mut recorder = runtime::Runtime::new();
    let mut closing = false;
    loop {
        // Bound input draining so continuous inbox traffic cannot starve revoke.
        for _ in 0..16 {
            let Some(message) = sex_pdx::pdx_try_listen_raw(0) else {
                break;
            };
            recorder.receive(&message, window.task);
            if message.caller_pd == sex_pdx::desktop_recording::SHELL
                && message.caller_task == sex_pdx::desktop_recording::SHELL
            {
                match window.receive(&message) {
                    Some(Event::Close) => {
                        closing = true;
                        recorder.stop()
                    }
                    Some(Event::Key(key)) if key.code == 1 => recorder.stop(),
                    _ => {}
                }
            }
        }
        recorder.poll(sex_pdx::get_ticks());
        if closing && recorder.can_exit() {
            sex_pdx::task_finish(0)
        }
        sex_pdx::sys_yield();
    }
}
