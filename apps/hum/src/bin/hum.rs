// SPDX-License-Identifier: GPL-3.0-only
//! A small background program, for finding out how many can stay alive. It sleeps
//! and wakes at its own pace, and checks that its memory is still what it wrote.
#![no_std]
#![no_main]
use sex_pdx::program_store as store;

const WORDS: usize = 512;
static mut MEMORY: [u64; WORDS] = [0; WORDS];

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { sex_pdx::task_finish(1) }

fn word(seed: u64, i: usize) -> u64 { seed.rotate_left(i as u32 % 64) ^ i as u64 }

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Each copy runs at its own address, so the address tells them apart.
    let seed = core::ptr::addr_of!(MEMORY) as u64;
    for i in 0..WORDS { unsafe { core::ptr::addr_of_mut!(MEMORY[i]).write_volatile(word(seed, i)) }; }
    let mut clock = seed >> 21;
    loop {
        clock = clock.wrapping_mul(0x9E3779B97F4A7C15).wrapping_add(1);
        let _ = sex_pdx::sys_wait_self_ticks((clock >> 58) + 1);
        for i in 0..WORDS {
            if unsafe { core::ptr::addr_of!(MEMORY[i]).read_volatile() } != word(seed, i) {
                sex_pdx::serial_record!("[hum.corrupt] seed={:#x} word={}", seed, i);
                sex_pdx::task_finish(3);
            }
        }
        if store::record_call(store::RECORD_STOP_REQUESTED, 0, 0).1 != 0 { sex_pdx::task_finish(0); }
    }
}
