// SPDX-License-Identifier: GPL-3.0-only
//! Heap test: a program that uses `alloc` the way real ones will (Vec, Box, String,
//! BTreeMap), fills 12 MiB of heap (past the old 8 MiB window), and shows the
//! numbers. Pressing A then fills all the memory programs may have, until an
//! allocation is refused. `run heaptest`. Its gate runs it over and over on reused rows and side by
//! side, so it also checks that a run starts with no memory and gets only zeroed pages.
#![no_std]
#![no_main]

extern crate alloc;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use sex_app::{heap, Event, Window};
use sex_pdx::program_memory as memory;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[heaptest.fatal] {}", info);
    sex_pdx::task_finish(1);
}

/// Checks the pages the kernel grants before the heap uses them. A run starts with
/// none; each page must be zero, whatever an earlier run on the row wrote. Then this
/// run scribbles on them, for the next run to find zeroed again.
fn fresh_pages() -> Result<u64, &'static str> {
    let start = memory::grow(0).map_err(|_| "no program memory")?;
    if start.pages != 0 { return Err("a run started holding pages"); }
    let m = memory::grow(16).map_err(|_| "first grant refused")?;
    if m.pages != 16 || m.base != start.base { return Err("grant is not the window's start"); }
    let bytes = unsafe { core::slice::from_raw_parts_mut(m.base as *mut u64, (m.pages * memory::PAGE / 8) as usize) };
    if bytes.iter().any(|w| *w != 0) { return Err("granted page was not zero"); }
    bytes.fill(0xA5A5_A5A5_A5A5_A5A5);
    if memory::grow(memory::MAX_PAGES_PER_CALL + 1) != Err(memory::INVALID) { return Err("oversized grant accepted"); }
    Ok(m.base)
}

/// MiB a normal run fills: past the old fixed window, and small enough that every
/// program row can do it at once on a 512 MiB machine.
const TARGET_MIB: usize = 12;

/// Fills memory a mebibyte at a time: `limit` of them, or until the kernel says no.
/// Running out must be a refused allocation, not a fault.
fn fill(limit: usize) -> Result<usize, &'static str> {
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    if chunks.try_reserve_exact(limit).is_err() { return Err("no room to count chunks"); }
    while chunks.len() < limit {
        let mut chunk = Vec::new();
        if chunk.try_reserve_exact(1024 * 1024).is_err() { break; }
        chunk.resize(1024 * 1024, chunks.len() as u8);
        chunks.push(chunk);
    }
    if chunks.iter().enumerate().any(|(i, c)| c.iter().any(|b| *b != i as u8)) { return Err("chunk lost data"); }
    Ok(chunks.len())
}

fn run(task: u32) -> Result<(String, usize, usize, usize), &'static str> {
    fresh_pages()?;
    // Ordinary use: a growing vector, a boxed buffer, a map, formatted text.
    let mut squares: Vec<u64> = Vec::new();
    for i in 0..200_000u64 { squares.push(i * i ^ task as u64); }
    let mut block: Box<[u8]> = alloc::vec![0u8; 1024 * 1024].into_boxed_slice();
    for (i, b) in block.iter_mut().enumerate() { *b = (i as u32 ^ task) as u8; }
    let mut map = BTreeMap::new();
    for i in 0..10_000u32 { map.insert(i.wrapping_mul(2_654_435_761), i); }
    let mut text = String::new();
    for i in 0..1000 { let _ = write!(text, "{i},"); }
    if squares.iter().enumerate().any(|(i, v)| *v != (i as u64 * i as u64) ^ task as u64) { return Err("vector lost data"); }
    if block.iter().enumerate().any(|(i, b)| *b != (i as u32 ^ task) as u8) { return Err("box lost data"); }
    if map.len() != 10_000 || map.values().map(|v| *v as u64).sum::<u64>() != (0..10_000u64).sum() { return Err("map lost data"); }
    if !text.starts_with("0,1,2,") || !text.ends_with("998,999,") { return Err("string lost data"); }
    let used = heap::held();
    drop((squares, block, map, text));
    // Freed memory is used again before the heap asks for more.
    let again: Vec<u8> = alloc::vec![7u8; 1024 * 1024];
    if heap::held() != used || again.iter().any(|b| *b != 7) { return Err("freed memory was not reused"); }
    drop(again);
    let full = fill(TARGET_MIB)?;
    if full < TARGET_MIB { return Err("the heap ran out before 12 MiB"); }
    let peak = heap::held();
    if peak > sex_pdx::program_store::WINDOW_BYTES { return Err("heap passed the window"); }
    // Out of memory once is not out of memory for good.
    let small: Vec<u32> = (0..1000).collect();
    if small.iter().sum::<u32>() != 499_500 { return Err("allocation after running out failed"); }
    Ok((text_of(used, peak, full), used, peak, full))
}

fn text_of(used: usize, peak: usize, full: usize) -> String {
    let mut s = String::new();
    let _ = write!(s, "Heap test. F11 closes.\n\nThis run started with no memory and was given zeroed pages.\nVec, Box, BTreeMap and String: {} KiB.\nFilled {} MiB more.\nPeak: {} KiB.\n\nAll checks passed.\n",
        used / 1024, full, peak / 1024);
    s
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Heap test");
    let (target, task) = (window.target, window.task);
    match run(task) {
        Ok((text, used, peak, full)) => {
            sex_pdx::serial_record!("[heaptest.ok] target={} task={} used={} peak={} full_mib={}", target, task, used, peak, full);
            window.show(text.as_bytes());
        }
        Err(why) => {
            sex_pdx::serial_record!("[heaptest.fail] target={} task={} why={}", target, task, why);
            window.show(why.as_bytes());
        }
    }
    sex_pdx::serial_record!("[heaptest.ready] target={} task={}", target, task);
    loop {
        match window.next() {
            Event::Close => { sex_pdx::serial_record!("[heaptest.closed] target={} task={}", target, task); window.close() }
            // A: take all the memory programs may have.
            Event::Key(key) if key.code == 0x1E => {
                let limit = sex_pdx::program_store::WINDOW_BYTES / (1024 * 1024);
                match fill(limit) {
                    Ok(full) if full < limit => {
                        sex_pdx::serial_record!("[heaptest.all] target={} task={} full_mib={} peak={}", target, task, full, heap::held());
                        let mut s = String::new();
                        let _ = write!(s, "Filled {} MiB until the kernel said no.\nPeak: {} KiB. F11 closes.\n", full, heap::held() / 1024);
                        window.show(s.as_bytes());
                    }
                    Ok(_) => sex_pdx::serial_record!("[heaptest.fail] target={} task={} why=the heap never ran out", target, task),
                    Err(why) => sex_pdx::serial_record!("[heaptest.fail] target={} task={} why={}", target, task, why),
                }
            }
            _ => {}
        }
    }
}
