// SPDX-License-Identifier: GPL-3.0-only
//! A hostile program: reads where the next and previous programs live. The hardware
//! must stop it, and only it, on the first read.
#![no_std]
#![no_main]

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { sex_pdx::task_finish(1) }

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let here = _start as usize as u64 & !0x1F_FFFF;
    sex_pdx::serial_record!("[snoop.reading] from={:#x}", here);
    let next = unsafe { ((here + 0x20_0000 + 0x1000) as *const u64).read_volatile() };
    sex_pdx::serial_record!("[snoop.survived] value={:#x}", next);
    sex_pdx::task_finish(0)
}
