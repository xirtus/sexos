// SPDX-License-Identifier: GPL-3.0-only
//! Denim: compressed memory. Cold heap pages of program runs are packed into Denim's
//! own memory (`crates/denim`) and unpacked when the run touches them again, so the
//! programs' memory can add up to more than the machine's RAM.
//!
//! Policy lives here; the kernel only moves bytes and leaves (`sex_pdx::denim`):
//! - A clock hand sweeps the program runs' heaps, reading accessed bits. A page not
//!   touched for `COLD` passes goes out while memory pressure is low or worse (or
//!   always, in a build with `SEXOS_DENIM_EAGER=1`, which the gate uses). The shorter
//!   memory is, the faster the hand turns and the more a round takes (`pace`).
//! - A fault on a page out brings it back at once, for a worker as for the main task;
//!   a recall (the run makes code) brings back all of the run's pages. Once pressure
//!   is normal and memory roomy again, pages come back a batch per round.
//! - Packed pages live in slabs of five sizes, 256 B to 4 KiB, carved from pages of
//!   Denim's arena. Zero pages and same-filled pages take no slab. When slabs grow
//!   sparse, blocks move into fewer pages and the emptied pages go back to the machine
//!   (holes, refilled before the arena grows again).
//! - A restarted Denim first ends the runs whose pages the old one held (`RESET`).
#![no_std]
#![no_main]
extern crate alloc;

use alloc::vec::Vec;
use denim::{pack, unpack, Packed, MAX_PACKED, PAGE};
use sex_pdx::denim as d;
use sex_pdx::program_memory as m;
use sex_pdx::serial_println;

// Denim's own bookkeeping (a byte and an entry per page of every run it watches) starts
// in this static heap and grows from the service's arena when a big program needs more:
// a fixed heap ran out at about 170 MiB of program heap and stopped Denim.
const HEAP_BYTES: usize = 2 * 1024 * 1024;
#[repr(align(4096))]
#[allow(dead_code)]
struct HeapBytes([u8; HEAP_BYTES]);
static mut HEAP: HeapBytes = HeapBytes([0; HEAP_BYTES]);

/// Denim runs on one task; the lock only satisfies talc.
struct Lock(core::sync::atomic::AtomicBool);
unsafe impl talc::lock_api::RawMutex for Lock {
    #[allow(clippy::declare_interior_mutable_const)]
    const INIT: Self = Lock(core::sync::atomic::AtomicBool::new(false));
    type GuardMarker = talc::lock_api::GuardSend;
    fn lock(&self) { while !self.try_lock() { core::hint::spin_loop(); } }
    fn try_lock(&self) -> bool {
        self.0.compare_exchange(false, true, core::sync::atomic::Ordering::Acquire, core::sync::atomic::Ordering::Relaxed).is_ok()
    }
    unsafe fn unlock(&self) { self.0.store(false, core::sync::atomic::Ordering::Release); }
}

/// More heap from the service arena, claimed as a new span. The slab store grows from
/// the same arena between allocations, so each span stands alone; within one acquire
/// the grows follow each other and make one contiguous span.
#[derive(Debug)]
struct Arena;
unsafe impl talc::source::Source for Arena {
    fn acquire<B: talc::base::binning::Binning>(talc: &mut talc::base::Talc<Self, B>, layout: core::alloc::Layout) -> Result<(), ()> {
        let bytes = layout.size().checked_add(layout.align()).and_then(|b| b.checked_add(4096)).ok_or(())? as u64;
        // At least 1 MiB at a time, so a growing table does not cost a call per step.
        let pages = bytes.div_ceil(m::PAGE).max(256);
        let (mut start, mut left) = (0u64, pages);
        while left > 0 {
            let n = left.min(m::MAX_PAGES_PER_CALL);
            let got = m::grow(n).map_err(|e| serial_println!("[denim.heap.refused] bytes={} status={}", layout.size(), e))?;
            let first = got.base + (got.pages - n) * m::PAGE;
            if start == 0 { start = first }
            else if first != start + (pages - left) * m::PAGE { return Err(()) }
            left -= n;
        }
        serial_println!("[denim.heap.grown] pages={}", pages);
        unsafe { talc.claim(start as *mut u8, (pages * m::PAGE) as usize) }.map(|_| ()).ok_or(())
    }
}

#[global_allocator]
static ALLOCATOR: talc::TalcLock<Lock, Arena> = talc::TalcLock::new(Arena);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[denim.panic] {}", info);
    loop { let _ = sex_pdx::sys_wait_self_ticks(64); }
}

/// Passes a page must stay untouched before it goes out.
const COLD: u8 = 2;
const EAGER: bool = match option_env!("SEXOS_DENIM_EAGER") { Some(v) => v.len() == 1 && v.as_bytes()[0] == b'1', None => false };
const CLASSES: [usize; 5] = [256, 512, 1024, 2048, 4096];
/// An arena page Denim gave back.
const HOLE: u8 = 0xFF;
/// Pages that come back per round once memory is roomy again.
const RECALL_BATCH: usize = 1024;

/// How a round goes at `pressure`: ticks to the next, pages the hand scans, most pages
/// it takes out, and the passes a page must stay cold.
fn pace(pressure: u64, recalling: bool) -> (u64, usize, usize, u8) {
    if pressure >= m::PRESSURE_CRITICAL { (50, 262144, 16384, 1) }
    else if pressure >= m::PRESSURE_LOW { (200, 262144, 8192, COLD) }
    else if EAGER { (500, 65536, 4096, COLD) }
    else { (if recalling { 250 } else { 2000 }, 16384, 0, COLD) }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind { None, Zero, Fill, Lz, Raw }
/// Where a page out lives: `at` is a slab slot (or the fill word), `len` the block.
#[derive(Clone, Copy)]
struct Entry { kind: Kind, class: u8, len: u16, at: u64 }
const EMPTY: Entry = Entry { kind: Kind::None, class: 0, len: 0, at: 0 };
impl Entry { fn slab(&self) -> bool { matches!(self.kind, Kind::Lz | Kind::Raw) } }

/// One program row's current run as Denim knows it.
struct Run { target: u32, run: u32, base: u64, cold: Vec<u8>, held: Vec<Entry>, out: usize }

struct Store {
    /// Denim's arena: `kind` and `used` say, per page, what it is (0 heap or window, a
    /// slab class + 1, or `HOLE`) and how many of its slots hold blocks.
    base: u64,
    kind: Vec<u8>,
    used: Vec<u16>,
    holes: Vec<u64>,
    free: [Vec<u64>; 5],
    window: u64,
    /// Slab bytes in use, pages out (any kind), slab pages, pages given back so far.
    bytes: u64,
    pages_out: u64,
    slabs: u64,
    returned: u64,
}

impl Store {
    fn at(&self, va: u64) -> usize { ((va - self.base) / PAGE as u64) as usize }
    /// A page of the arena: a hole refilled, or a new one.
    fn page(&mut self) -> Option<u64> {
        if let Some(hole) = self.holes.pop() {
            if d::call(d::REFILL, 0, hole, 0, 0)[0] == d::OK { let i = self.at(hole); if i < self.kind.len() { self.kind[i] = 0; } return Some(hole); }
            self.holes.push(hole);
            return None;
        }
        let got = m::grow(1).ok()?;
        Some(got.base + (got.pages - 1) * m::PAGE)
    }
    fn take(&mut self, class: usize) -> Option<u64> {
        if self.free[class].is_empty() {
            let page = self.page()?;
            let i = self.at(page);
            let size = CLASSES[class] as u64;
            let room = self.kind.len().max(i + 1);
            if self.kind.try_reserve(room - self.kind.len()).is_err() || self.used.try_reserve(room - self.used.len()).is_err()
                || self.free[class].try_reserve((m::PAGE / size) as usize).is_err() {
                self.give_page(page);
                return None;
            }
            self.kind.resize(room, 0); self.used.resize(room, 0);
            self.kind[i] = class as u8 + 1;
            self.slabs += 1;
            for slot in (0..m::PAGE / size).rev() { self.free[class].push(page + slot * size); }
        }
        let at = self.free[class].pop()?;
        let i = self.at(at);
        self.used[i] += 1;
        self.bytes += CLASSES[class] as u64;
        Some(at)
    }
    /// An empty arena page goes back to the machine.
    fn give_page(&mut self, page: u64) -> bool {
        if self.holes.try_reserve(1).is_err() || d::call(d::RETURN, 0, page, 0, 0)[0] != d::OK { return false; }
        let i = self.at(page);
        if i < self.kind.len() { self.kind[i] = HOLE; }
        self.holes.push(page);
        self.returned += 1;
        true
    }
    fn give(&mut self, e: Entry) {
        if e.slab() {
            self.free[e.class as usize].push(e.at);
            let i = self.at(e.at);
            self.used[i] -= 1;
            self.bytes -= CLASSES[e.class as usize] as u64;
        }
        if e.kind != Kind::None { self.pages_out -= 1; }
    }
    fn window(&self) -> &'static mut [u8; PAGE] { unsafe { &mut *(self.window as *mut [u8; PAGE]) } }
    /// Bytes of slab slots holding nothing.
    fn spare(&self) -> u64 { self.slabs * m::PAGE - self.bytes }

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

    /// Moves blocks out of the sparsest slab pages into free slots of fuller ones, and
    /// gives the emptied pages back. Returns how many went back.
    fn compact(&mut self, runs: &mut [Run]) -> u64 {
        const MOVING: u8 = 0x80;
        let mut back = 0;
        for c in 0..CLASSES.len() {
            let slots = (PAGE / CLASSES[c]) as u64;
            let mut pages: Vec<(u16, usize)> = Vec::new();
            for (i, &k) in self.kind.iter().enumerate() {
                if k == c as u8 + 1 { if pages.try_reserve(1).is_err() { return back; } pages.push((self.used[i], i)); }
            }
            pages.sort_unstable();
            // Emptiest first, while the slots left in the other pages hold their blocks.
            let mut spare: u64 = pages.iter().map(|&(u, _)| slots - u as u64).sum();
            let mut n = 0;
            while n < pages.len() && spare >= slots { spare -= slots; n += 1; }
            if n == 0 { continue; }
            for &(_, i) in &pages[..n] { self.kind[i] |= MOVING; }
            let (base, kind) = (self.base, &self.kind);
            self.free[c].retain(|&a| kind[((a - base) / PAGE as u64) as usize] & MOVING == 0);
            for e in runs.iter_mut().flat_map(|r| r.held.iter_mut()) {
                if !e.slab() || e.class as usize != c || self.kind[self.at(e.at)] & MOVING == 0 { continue; }
                let Some(to) = self.free[c].pop() else { break };
                unsafe { core::ptr::copy_nonoverlapping(e.at as *const u8, to as *mut u8, e.len as usize); }
                let (from, into) = (self.at(e.at), self.at(to));
                self.used[from] -= 1; self.used[into] += 1;
                e.at = to;
            }
            for &(_, i) in &pages[..n] {
                let page = self.base + (i * PAGE) as u64;
                self.kind[i] = c as u8 + 1;
                // A page still holding blocks (the moves ran out of slots) keeps them, and
                // goes at a later compaction once empty.
                if self.used[i] != 0 { continue; }
                if self.give_page(page) { self.slabs -= 1; back += 1; continue; }
                for slot in 0..slots { self.free[c].push(page + slot * CLASSES[c] as u64); }
            }
        }
        back
    }
}

struct Denim { store: Store, runs: Vec<Run>, scratch: [u8; MAX_PACKED], ins: u64, faults: u64, outs: u64, recalled: u64, waiting: Vec<(u32, u64, u64)> }

impl Denim {
    fn run(&mut self, target: u32) -> Option<&mut Run> { self.runs.iter_mut().find(|r| r.target == target) }

    /// The run ended or another began: what Denim held for it is gone with it.
    fn forget(&mut self, target: u32) {
        if let Some(i) = self.runs.iter().position(|r| r.target == target) {
            let run = self.runs.swap_remove(i);
            for e in run.held { self.store.give(e); }
        }
    }

    /// Scans 64 heap pages of `target` from `first`, taking out up to `take` cold ones.
    /// The next page to scan, or None once the run's heap (or the run) ends.
    fn scan(&mut self, target: u32, first: u64, take: &mut usize, cold: u8) -> Option<u64> {
        let r = d::call(d::SCAN, target, first, 0, 0);
        // DENIED: the run ended (or another began). RETRY is only a busy table: keep what is held.
        if r[0] != d::OK { if r[0] == d::DENIED { self.forget(target); } return None; }
        let (grown, run, base) = ((r[3] & 0xFFFF_FFFF) as usize, (r[3] >> 32) as u32, r[4]);
        if self.run(target).is_some_and(|s| s.run != run || s.base != base) { self.forget(target); }
        if self.run(target).is_none() {
            if self.runs.try_reserve(1).is_err() { return None; }
            self.runs.push(Run { target, run, base, cold: Vec::new(), held: Vec::new(), out: 0 });
        }
        let mut chosen = [0usize; 64];
        let mut n = 0;
        {
            let state = self.run(target).unwrap();
            if state.cold.len() < grown {
                // No room to track more pages: leave the rest of this run alone for now.
                if state.cold.try_reserve(grown - state.cold.len()).is_err() || state.held.try_reserve(grown - state.held.len()).is_err() { return None; }
                state.cold.resize(grown, 0); state.held.resize(grown, EMPTY);
            }
            for bit in 0..64usize {
                let i = first as usize + bit;
                if i >= grown { break; }
                if r[1] & 1 << bit != 0 { state.cold[i] = 0; }
                else if r[2] & 1 << bit != 0 { state.cold[i] = state.cold[i].saturating_add(1); }
                if n < *take && r[2] & 1 << bit != 0 && state.cold[i] >= cold { chosen[n] = i; n += 1; }
            }
        }
        for &i in &chosen[..n] { if self.page_out(target, base, i) { *take -= 1; } }
        let next = first + 64;
        if next as usize >= grown { None } else { Some(next) }
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
        let held = self.runs.iter().find(|r| r.target == target).and_then(|state| {
            let i = ((page.wrapping_sub(state.base)) / PAGE as u64) as usize;
            state.held.get(i).filter(|e| e.kind != Kind::None).map(|&e| (i, e))
        });
        let Some((i, e)) = held else {
            // Another task of the run already brought it back (or the run ended): the
            // kernel finds no marker there and lets this task go on, to touch it again.
            if token != 0 && d::call(d::IN, target, page, self.store.window, token)[0] == d::DENIED { return true; }
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
            if token != 0 { self.faults += 1; }
            if self.ins.is_power_of_two() { serial_println!("[denim.in] target={} page={:#x} fault={} ins={}", target, page, (token != 0) as u8, self.ins); }
        }
        true
    }

    /// Up to `most` of `target`'s pages come back (all, for a recall). How many did.
    fn bring_back(&mut self, target: u32, most: usize, wait: bool) -> usize {
        let Some(state) = self.run(target) else { return 0 };
        let pages: Vec<u64> = state.held.iter().enumerate().filter(|(_, e)| e.kind != Kind::None)
            .take(most).map(|(i, _)| state.base + (i * PAGE) as u64).collect();
        let mut n = 0;
        for page in pages {
            while !self.page_in(target, page, 0) {
                if !wait { return n; }
                let _ = sex_pdx::sys_wait_self_ticks(8);
            }
            n += 1;
        }
        n
    }

    fn recall(&mut self, target: u32) {
        let n = self.bring_back(target, usize::MAX, true);
        if n > 0 { serial_println!("[denim.recall] target={} pages={}", target, n); }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe { ALLOCATOR.lock().claim(core::ptr::addr_of_mut!(HEAP) as *mut u8, HEAP_BYTES); }
    // A new run first ends the runs whose pages an old run held, and frees its store.
    let reset = d::call(d::RESET, 0, 0, 0, 0);
    let arena = m::grow(0).unwrap_or(m::Memory { base: 0, pages: 0 });
    let mut store = Store { base: arena.base, kind: Vec::new(), used: Vec::new(), holes: Vec::new(), free: Default::default(),
        window: 0, bytes: 0, pages_out: 0, slabs: 0, returned: 0 };
    // Every page an old run grew is a hole now.
    if arena.pages > 0 && store.kind.try_reserve(arena.pages as usize).is_ok() && store.holes.try_reserve(arena.pages as usize).is_ok() {
        store.kind.resize(arena.pages as usize, HOLE);
        store.used.resize(arena.pages as usize, 0);
        for i in (0..arena.pages).rev() { store.holes.push(arena.base + i * m::PAGE); }
    }
    let Some(window) = store.page() else {
        serial_println!("[denim.ready] ok=0 reason=no_memory");
        loop { let _ = sex_pdx::sys_wait_self_ticks(64); }
    };
    if store.base == 0 { store.base = window; }
    store.window = window;
    let mut denim = Denim { store, runs: Vec::new(), scratch: [0; MAX_PACKED], ins: 0, faults: 0, outs: 0, recalled: 0, waiting: Vec::new() };
    let targets: Vec<u32> = sex_pdx::app_catalog::INCARNATIONS.iter()
        .filter(|r| r.app == sex_pdx::app_catalog::App::Program).map(|r| r.target).collect();
    serial_println!("[denim.ready] ok=1 rows={} eager={} cold_scans={} restarts={} ended={} holes={}",
        targets.len(), EAGER as u8, COLD, sex_pdx::crash_log::restarts(), reset[1], arena.pages);
    let mut next = sex_pdx::get_ticks() + 500;
    let (mut hand, mut hand_first) = (0usize, 0u64);
    // The most free memory seen, to judge "roomy" as the kernel judges pressure.
    let mut most_free = 0u64;
    let mut shown = (0u64, 0u64, 0u64);
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
        if now >= next && !targets.is_empty() {
            let pressure = m::pressure();
            let free = sex_pdx::system_stats::overview().map_or(0, |o| o.free_pages);
            most_free = most_free.max(free);
            // Roomy: twice the kernel's low mark free, after a batch comes back.
            let roomy = !EAGER && pressure == m::PRESSURE_NORMAL && free > 2 * (most_free / 8).max(8192) + RECALL_BATCH as u64;
            let (ticks, scan, mut take, cold) = pace(pressure, roomy && denim.store.pages_out > 0);
            // The clock hand: on from where the last round stopped, across the rows.
            let mut calls = 0;
            while calls < scan / 64 + targets.len() {
                calls += 1;
                match denim.scan(targets[hand], hand_first, &mut take, cold) {
                    Some(first) => hand_first = first,
                    None => { hand = (hand + 1) % targets.len(); hand_first = 0; }
                }
            }
            if roomy && denim.store.pages_out > 0 {
                let mut left = RECALL_BATCH;
                let ts: Vec<u32> = denim.runs.iter().filter(|r| r.out > 0).map(|r| r.target).collect();
                for t in ts { if left == 0 { break; } let n = denim.bring_back(t, left, false); left -= n; denim.recalled += n as u64; }
            }
            let mut returned = 0;
            let spare = denim.store.spare();
            if spare >= 64 * m::PAGE && spare >= denim.store.bytes / 8 {
                let Denim { store, runs, .. } = &mut denim;
                returned = store.compact(runs);
            }
            if (denim.outs, denim.ins, denim.store.returned) != shown || returned > 0 {
                shown = (denim.outs, denim.ins, denim.store.returned);
                let s = &denim.store;
                // `faults`: the pages that came back because the program touched them.
                serial_println!("[denim.round] outs={} ins={} pages_out={} slab_bytes={} store_pages={} saved_pages={} pressure={} faults={} recalled={} returned={} holes={} runs={}",
                    denim.outs, denim.ins, s.pages_out, s.bytes, s.slabs, s.pages_out as i64 - s.slabs as i64, pressure, denim.faults,
                    denim.recalled, s.returned, s.holes.len(), denim.runs.iter().filter(|r| r.out > 0).count());
            }
            next = now + ticks;
        }
        let _ = sex_pdx::sys_wait_self_ticks(if denim.waiting.is_empty() { 64 } else { 8 });
    }
}
