// SPDX-License-Identifier: GPL-3.0-only
//! A memory sample, like a swatch cut from the cloth: one serial record of the kernel
//! heap, the machine's free frames and free program memory, then a line per program row
//! that holds pages, and it ends. Leak gates run it in the background (`spawn swatch`)
//! before and after open/close cycles and compare the numbers.
#![no_std]
#![no_main]
use sex_pdx::system_stats as stats;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { sex_pdx::task_finish(1) }

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let k = stats::kernel().unwrap_or_default();
    let o = stats::overview().unwrap_or_default();
    let (mut rows, mut pages, mut pd) = (0u64, 0u64, 0u32);
    while let Some(next) = stats::next(pd) {
        pd = next;
        let Some(d) = stats::domain(pd) else { continue };
        if d.pages == stats::UNKNOWN || d.pages == 0 { continue; }
        rows += 1;
        pages += d.pages;
        sex_pdx::serial_record!("[swatch.row] pd={} task={} pages={}", pd, d.task, d.pages);
    }
    sex_pdx::serial_record!(
        "[swatch.sample] heap_used={} heap_mapped={} heap_blocks={} heap_peak={} free_frames={} free_pages={} rows={} row_pages={}",
        k.heap_used, k.heap_mapped, k.heap_blocks, k.heap_peak, k.free_frames, o.free_pages, rows, pages
    );
    sex_pdx::task_finish(0)
}
