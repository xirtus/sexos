// SPDX-License-Identifier: GPL-3.0-only
//! The first program outside the three built-in apps: it opens a window, shows
//! text through its BOOT text region, and exits when closed.
#![no_std]
#![no_main]

use sex_pdx::text_batch::{hash, Request};
use sex_pdx::{app_lifecycle as lifecycle, pdx_call_checked, SLOT_DISPLAY, SLOT_SHELL};

const TITLE_ID: u64 = 300;
const TEXT: &[u8] = b"Hello from a SexOS program.\n\nThis window belongs to its own protection domain.\nIt holds only what BOOT granted its catalog row.\n\nClose it with the red light.\n";

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[hello.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// Retries only while the service queue is full; anything else ends the program.
fn call(slot: u64, op: u64, a: u64, b: u64, c: u64) -> u64 {
    loop {
        match pdx_call_checked(slot, op, a, b, c) {
            Ok(value) => return value,
            Err(sex_pdx::ERR_SERVICE_NOT_READY) => sex_pdx::sys_yield(),
            Err(_) => sex_pdx::task_finish(2),
        }
    }
}

fn show(surface: u64, region: u64) {
    for (i, byte) in TEXT.iter().enumerate() {
        unsafe { core::ptr::write_volatile((region as *mut u8).add(i), *byte) };
    }
    let sequence = 1;
    let begin = Request::Begin { sequence, len: TEXT.len(), color: sex_pdx::desktop_style::TEXT };
    let commit = Request::Commit { sequence, hash: hash(TEXT) };
    for request in [begin, Request::Fill { sequence }, commit] {
        let (op, a, b) = request.wire();
        call(SLOT_DISPLAY, op, surface, a, b);
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let identity = lifecycle::catalog(lifecycle::SELF_CATALOG_SLOT).expect("HELLO_SELF_AUTHORITY");
    let row = sex_pdx::app_catalog::by_surface(identity.target_pd, identity.surface_id).expect("HELLO_CATALOG_ROW");
    // The first row only anchors the chain of program rows: it runs at BOOT, so
    // it leaves at once and a launch activates the next row.
    if row.ordinal == 0 {
        sex_pdx::serial_record!("[hello.anchor] target={} task={}", identity.target_pd, identity.task_id);
        sex_pdx::task_finish(0);
    }
    let surface = identity.surface_id;
    let region = sex_pdx::text_region_client().expect("HELLO_TEXT_REGION");
    // The surface must be ours before Shell frames it and before text goes to it.
    call(SLOT_DISPLAY, 0xEC, surface, (56u64 << 32) | 200, (400u64 << 32) | 640);
    call(SLOT_SHELL, 0xFA, surface, TITLE_ID, (identity.target_pd as u64) << 8);
    show(surface, region);
    call(SLOT_DISPLAY, lifecycle::OP_APP_PRESENT, surface, 1, 0);
    sex_pdx::serial_record!("[hello.ready] target={} task={} surface={} bytes={}", identity.target_pd, identity.task_id, surface, TEXT.len());
    loop {
        let message = sex_pdx::pdx_listen_raw(0);
        if message.caller_pd == 3
            && lifecycle::close_requested(message.type_id, message.caller_pd, [message.arg0, message.arg1, message.arg2], identity.target_pd, identity.task_id)
        {
            sex_pdx::serial_record!("[hello.closed] target={} task={}", identity.target_pd, identity.task_id);
            sex_pdx::task_finish(0);
        }
    }
}
