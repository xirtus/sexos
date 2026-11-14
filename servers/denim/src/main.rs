// SPDX-License-Identifier: GPL-3.0-only
//! Denim: compressed memory. Cold heap pages of program runs are packed into Denim's
//! own memory (`crates/denim`) and unpacked when the run touches them again, so the
//! programs' memory can add up to more than the machine's RAM.
//!
//! Policy lives here; the kernel only moves bytes and leaves (`sex_pdx::denim`):
//! - Every few seconds each program run's heap is scanned. A page not touched for
//!   `COLD` scans in a row goes out, while memory pressure is low or worse (or always,
//!   in a build with `SEXOS_DENIM_EAGER=1`, which the gate uses).
//! - A fault on a page out brings it back at once; a recall (the run starts a worker
//!   or makes code) brings back all of the run's pages.
//! - Packed pages live in slabs of five sizes, 256 B to 4 KiB, carved from pages this
//!   service grows. Zero pages and same-filled pages take no slab.
#![no_std]
#![no_main]
extern crate alloc;

use alloc::vec::Vec;
use denim::{pack, unpack, Packed, MAX_PACKED, PAGE};
use sex_pdx::denim as d;
use sex_pdx::program_memory as m;
use sex_pdx::serial_println;

const HEAP_BYTES: usize = 2 * 1024 * 1024;
#[repr(align(4096))]
#[allow(dead_code)]
struct HeapBytes([u8; HEAP_BYTES]);
static mut HEAP: HeapBytes = HeapBytes([0; HEAP_BYTES]);
#[global_allocator]
static ALLOCATOR: linked_list_allocator::LockedHeap = linked_list_allocator::LockedHeap::empty();

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[denim.panic] {}", info);
    loop { let _ = sex_pdx::sys_wait_self_ticks(64); }
}

/// Scans a page must stay untouched before it goes out.
const COLD: u8 = 2;
const EAGER: bool = match option_env!("SEXOS_DENIM_EAGER") { Some(v) => v.len() == 1 && v.as_bytes()[0] == b'1', None => false };
const SCAN_TICKS: u64 = if EAGER { 500 } else { 2000 };
/// Most pages one scan round takes, so a round stays short.
const ROUND_BUDGET: usize = 1024;
const CLASSES: [usize; 5] = [256, 512, 1024, 2048, 4096];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind { None, Zero, Fill, Lz, Raw }
/// Where a page out lives: `at` is a slab slot (or the fill word), `len` the block.
#[derive(Clone, Copy)]
struct Entry { kind: Kind, class: u8, len: u16, at: u64 }
const EMPTY: Entry = Entry { kind: Kind::None, class: 0, len: 0, at: 0 };

/// One program row's current run as Denim knows it.
struct Run { target: u32, run: u32, base: u64, cold: Vec<u8>, held: Vec<Entry>, out: usize }

struct Store {
    free: [Vec<u64>; 5],
    window: u64,
    /// Slab bytes in use, and pages out (any kind).
    used: u64,
    pages_out: u64,
    grown: u64,
}

impl Store {
    fn page(&mut self) -> Option<u64> {
        let got = m::grow(1).ok()?;
        self.grown += 1;
        Some(got.base + (got.pages - 1) * m::PAGE)
    }
    fn take(&mut self, class: usize) -> Option<u64> {
        if self.free[class].is_empty() {
            let page = self.page()?;
            let size = CLASSES[class] as u64;
            for slot in (0..m::PAGE / size).rev() { self.free[class].push(page + slot * size); }
        }
        self.used += CLASSES[class] as u64;
        self.free[class].pop()
    }
    fn give(&mut self, e: Entry) {
        if matches!(e.kind, Kind::Lz | Kind::Raw) {
            self.free[e.class as usize].push(e.at);
            self.used -= CLASSES[e.class as usize] as u64;
        }
        if e.kind != Kind::None { self.pages_out -= 1; }
    }
    fn window(&self) -> &'static mut [u8; PAGE] { unsafe { &mut *(self.window as *mut [u8; PAGE]) } }

    /// Packs the window's page. None when there is no room: the caller puts it back.
    fn keep(&mut self, scratch: &mut [u8; MAX_PACKED]) -> Option<Entry> {
        let page = self.window();
        let (kind, len, bytes): (Kind, usize, &[u8]) = match pack(page, scratch) {
            Packed::Zero => (Kind::Zero, 0, &[]),
            Packed::Fill(w) => { self.pages_out += 1; return Some(Entry { kind: Kind::Fill, class: 0, len: 0, at: w }) }
            Packed::Lz(n) if n <= PAGE - PAGE / 8 => (Kind::Lz, n, &scratch[..n]),
            Packed::Lz(_) => (Kind::Raw, PAGE, &page[..]),
        };
        if kind == Kind::Zero { self.pages_out += 1; return Some(Entry { kind, class: 0, len: 0, at: 0 }); }
        let class = CLASSES.iter().position(|&c| c >= len)?;
        let at = self.take(class)?;
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), at as *mut u8, len); }
        self.pages_out += 1;
        Some(Entry { kind, class: class as u8, len: len as u16, at })
    }
    /// Unpacks `e` into the window.
    fn restore(&self, e: Entry) -> bool {
        let page = self.window();
        let block = |len: usize| unsafe { core::slice::from_raw_parts(e.at as *const u8, len) };
        match e.kind {
            Kind::None => false,
            Kind::Zero => unpack(Packed::Zero, &[], page).is_some(),
            Kind::Fill => unpack(Packed::Fill(e.at), &[], page).is_some(),
            Kind::Lz => unpack(Packed::Lz(e.len as usize), block(e.len as usize), page).is_some(),
            Kind::Raw => { page.copy_from_slice(block(PAGE)); true }
        }
    }
}

struct Denim { store: Store, runs: Vec<Run>, scratch: [u8; MAX_PACKED], ins: u64, outs: u64, waiting: Vec<(u32, u64, u64)> }

impl Denim {
    fn run(&mut self, target: u32) -> Option<&mut Run> { self.runs.iter_mut().find(|r| r.target == target) }

    /// The run ended or another began: what Denim held for it is gone with it.
    fn forget(&mut self, target: u32) {
        if let Some(i) = self.runs.iter().position(|r| r.target == target) {
            let run = self.runs.swap_remove(i);
            for e in run.held { self.store.give(e); }
        }
    }

    fn scan(&mut self, target: u32, active: bool, budget: &mut usize) {
        let mut first = 0u64;
        loop {
            let r = d::call(d::SCAN, target, first, 0, 0);
            if r[0] != d::OK { self.forget(target); return; }
            let (grown, run, base) = ((r[3] & 0xFFFF_FFFF) as usize, (r[3] >> 32) as u32, r[4]);
            if self.run(target).is_some_and(|s| s.run != run || s.base != base) { self.forget(target); }
            if self.run(target).is_none() {
                self.runs.push(Run { target, run, base, cold: Vec::new(), held: Vec::new(), out: 0 });
            }
            let mut take = Vec::new();
            {
                let state = self.run(target).unwrap();
                if state.cold.len() < grown { state.cold.resize(grown, 0); state.held.resize(grown, EMPTY); }
                for bit in 0..64usize {
                    let i = first as usize + bit;
                    if i >= grown { break; }
                    if r[1] & 1 << bit != 0 { state.cold[i] = 0; }
                    else if r[2] & 1 << bit != 0 { state.cold[i] = state.cold[i].saturating_add(1); }
                    if active && r[2] & 1 << bit != 0 && state.cold[i] >= COLD && take.len() < *budget { take.push(i); }
                }
            }
            for i in take { if self.page_out(target, base, i) { *budget -= 1; } }
            first += 64;
            if first as usize >= grown { return; }
        }
    }

    fn page_out(&mut self, target: u32, base: u64, i: usize) -> bool {
        let va = base + (i * PAGE) as u64;
        if d::call(d::OUT, target, va, self.store.window, 0)[0] != d::OK { return false; }
        match self.store.keep(&mut self.scratch) {
            Some(e) => {
                let state = self.run(target).unwrap();
                state.held[i] = e; state.cold[i] = 0; state.out += 1;
                self.outs += 1;
                true
            }
            None => {
                // No room to keep it: the window still holds the page, so it goes straight back.
                let back = d::call(d::IN, target, va, self.store.window, 0)[0];
                serial_println!("[denim.full] target={} page={:#x} back={}", target, va, back);
                false
            }
        }
    }

    /// Brings `page` back; with a token its task goes on. False when it must wait for memory.
    fn page_in(&mut self, target: u32, page: u64, token: u64) -> bool {
        let Some(state) = self.runs.iter().find(|r| r.target == target) else {
            serial_println!("[denim.lost] target={} page={:#x} token={}", target, page, token);
            return true;
        };
        let i = ((page.wrapping_sub(state.base)) / PAGE as u64) as usize;
        let Some(&e) = state.held.get(i).filter(|e| e.kind != Kind::None) else {
            serial_println!("[denim.lost] target={} page={:#x} token={}", target, page, token);
            return true;
        };
        if !self.store.restore(e) {
            serial_println!("[denim.damaged] target={} page={:#x}", target, page);
            return true;
        }
        let status = d::call(d::IN, target, page, self.store.window, token)[0];
        if status == d::FULL || status == d::RETRY { return false; }
        // Back (OK), or its run is gone (DENIED): either way Denim no longer holds it.
        let state = self.run(target).unwrap();
        state.held[i] = EMPTY; state.out -= 1;
        self.store.give(e);
        if status == d::OK {
            self.ins += 1;
            if self.ins.is_power_of_two() { serial_println!("[denim.in] target={} page={:#x} fault={} ins={}", target, page, (token != 0) as u8, self.ins); }
        }
        true
    }

    fn recall(&mut self, target: u32) {
        let Some(state) = self.run(target) else { return };
        let pages: Vec<u64> = state.held.iter().enumerate().filter(|(_, e)| e.kind != Kind::None)
            .map(|(i, _)| state.base + (i * PAGE) as u64).collect();
        let n = pages.len();
        for page in pages { while !self.page_in(target, page, 0) { let _ = sex_pdx::sys_wait_self_ticks(8); } }
        if n > 0 { serial_println!("[denim.recall] target={} pages={}", target, n); }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe { ALLOCATOR.lock().init(core::ptr::addr_of_mut!(HEAP) as *mut u8, HEAP_BYTES); }
    let mut store = Store { free: Default::default(), window: 0, used: 0, pages_out: 0, grown: 0 };
    let Some(window) = store.page() else {
        serial_println!("[denim.ready] ok=0 reason=no_memory");
        loop { let _ = sex_pdx::sys_wait_self_ticks(64); }
    };
    store.window = window;
    let mut denim = Denim { store, runs: Vec::new(), scratch: [0; MAX_PACKED], ins: 0, outs: 0, waiting: Vec::new() };
    let targets: Vec<u32> = sex_pdx::app_catalog::INCARNATIONS.iter()
        .filter(|r| r.app == sex_pdx::app_catalog::App::Program).map(|r| r.target).collect();
    serial_println!("[denim.ready] ok=1 rows={} eager={} scan_ticks={} cold_scans={}", targets.len(), EAGER as u8, SCAN_TICKS, COLD);
    let mut next = sex_pdx::get_ticks() + SCAN_TICKS;
    let (mut shown_out, mut shown_in) = (0u64, 0u64);
    loop {
        while let Some(msg) = sex_pdx::pdx_try_listen_raw(0) {
            if msg.caller_pd != 0 { continue; }
            match msg.type_id {
                d::OP_FAULT => if !denim.page_in(msg.arg0 as u32, msg.arg1, msg.arg2) { denim.waiting.push((msg.arg0 as u32, msg.arg1, msg.arg2)); },
                d::OP_RECALL => denim.recall(msg.arg0 as u32),
                _ => {}
            }
        }
        // Faults that found no free memory try again.
        let waiting = core::mem::take(&mut denim.waiting);
        for (t, page, token) in waiting { if !denim.page_in(t, page, token) { denim.waiting.push((t, page, token)); } }
        let now = sex_pdx::get_ticks();
        if now >= next {
            let pressure = m::pressure();
            let active = EAGER || pressure >= m::PRESSURE_LOW;
            let mut budget = ROUND_BUDGET;
            for &t in &targets { denim.scan(t, active, &mut budget); }
            if denim.outs != shown_out || denim.ins != shown_in {
                shown_out = denim.outs; shown_in = denim.ins;
                let s = &denim.store;
                serial_println!("[denim.round] outs={} ins={} pages_out={} slab_bytes={} store_pages={} saved_pages={} pressure={}",
                    denim.outs, denim.ins, s.pages_out, s.used, s.grown, s.pages_out.saturating_sub(s.grown), pressure);
            }
            next = now + SCAN_TICKS;
        }
        let _ = sex_pdx::sys_wait_self_ticks(if denim.waiting.is_empty() { 64 } else { 8 });
    }
}
