// SPDX-License-Identifier: GPL-3.0-only
//! Consoles attached to terminals (`sex_pdx::console_relay`): per terminal row and slot
//! (a tab or pane), the run it started, the window's size and the run's terminal mode.
//! The byte rings come from a pool shared by every slot: no more runs than there are
//! program rows can be attached at once, so there are only that many ring pairs, the
//! same memory as one pair per row. Files only moves bytes; Shell starts the run, and
//! only for the terminal that prepared it.

use sex_pdx::console_relay as c;
use sex_pdx::fs as w;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};

const ROWS: usize = sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Program);
/// One relay per terminal row and slot: relay `k * SLOTS + slot`.
const RELAYS_N: usize = ROWS * c::SLOTS;
const _: () = assert!(ROWS <= 64);

struct Relay {
    state: u8,
    /// The attached run: its program row, target and task.
    child: Option<(usize, u32, u32)>,
    code: u32,
    interrupt: bool,
    hung_up: bool,
    mode: u8,
    /// The terminal's window, kept across the slot's runs.
    size: c::Size,
    /// The ring pair it holds from PREPARE until RELEASE.
    rings: Option<usize>,
}
impl Relay {
    const fn new() -> Relay { Relay { state: c::NONE, child: None, code: 0, interrupt: false, hung_up: false, mode: 0, size: c::Size { cols: 0, rows: 0, changes: 0 }, rings: None } }
}
struct Rings { input: c::Ring<{ c::INPUT_BYTES }>, output: c::Ring<{ c::OUTPUT_BYTES }> }
static mut RELAYS: [Relay; RELAYS_N] = [const { Relay::new() }; RELAYS_N];
static mut RINGS: [Rings; ROWS] = [const { Rings { input: c::Ring::new(), output: c::Ring::new() } }; ROWS];
static mut POOL: c::Pool<ROWS> = c::Pool::new();
fn relays() -> &'static mut [Relay; RELAYS_N] { unsafe { &mut *core::ptr::addr_of_mut!(RELAYS) } }
fn rings(i: usize) -> &'static mut Rings { unsafe { &mut (*core::ptr::addr_of_mut!(RINGS))[i] } }
fn pool() -> &'static mut c::Pool<ROWS> { unsafe { &mut *core::ptr::addr_of_mut!(POOL) } }

/// The relay of terminal row `k`'s `slot`.
pub fn relay_id(k: usize, slot: usize) -> usize { k * c::SLOTS + slot.min(c::SLOTS - 1) }

/// Forget relay `id`'s run and give back its rings; its size stays.
fn reset(id: usize) {
    let r = &mut relays()[id];
    if let Some(i) = r.rings.take() { pool().give(i); }
    let size = r.size;
    *r = Relay::new();
    r.size = size;
}

/// Whether relay `id` may prepare a run: none is attached to it now (`PENDING` if one
/// is), and a ring pair is free (`UNAVAILABLE` if none is, even after taking back the
/// pairs of runs that ended and were never released).
pub fn may_prepare(id: usize) -> Result<(), u8> {
    let r = &mut relays()[id];
    refresh(r);
    if !matches!(r.state, c::NONE | c::ENDED | c::STARTING) { return Err(w::PENDING); }
    if r.rings.is_some() || pool().free() > 0 { return Ok(()); }
    // Ended runs whose terminal never released them hold pairs: the oldest output is
    // the least wanted, but any ended run will do.
    let idle = relays().iter().position(|r| r.state == c::ENDED && r.rings.is_some());
    match idle { Some(other) => { reset(other); Ok(()) } None => Err(w::UNAVAILABLE) }
}
/// Relay `id` prepared a run: the next one Shell binds for it is attached.
pub fn prepared(id: usize) {
    reset(id);
    let r = &mut relays()[id];
    r.rings = pool().take();
    if let Some(i) = r.rings { rings(i).input.clear(); rings(i).output.clear(); }
    r.state = c::STARTING;
}
/// Shell bound program row `index` as `target`'s `task`; `launcher` is the relay that
/// prepared it, if one did. A row bound again is no longer anyone's attached run, nor
/// the terminal it was: that terminal's slots are forgotten.
pub fn bound(launcher: Option<usize>, index: usize, target: u32, task: u32) {
    for r in relays().iter_mut() {
        if r.child.is_some_and(|(i, ..)| i == index) && r.state == c::RUNNING { r.state = c::ENDED; r.code = c::CRASHED; r.child = None; }
    }
    // A terminal that ran in this row is gone with its slots (it cannot be the launcher).
    for slot in 0..c::SLOTS { reset(relay_id(index, slot)); relays()[relay_id(index, slot)].size = c::Size::default(); }
    let Some(id) = launcher else { return };
    let r = &mut relays()[id];
    r.child = Some((index, target, task));
    r.state = c::RUNNING;
    sex_pdx::serial_record!("[files.console.attached] terminal_row={} slot={} target={} task={}", id / c::SLOTS, id % c::SLOTS, target, task);
}
/// The run relay `id` prepared could not be loaded.
pub fn not_started(id: usize) {
    let r = &mut relays()[id];
    if r.state == c::STARTING { r.state = c::ENDED; r.code = c::NOT_STARTED; }
}
/// A run that went away without saying how it ended.
fn refresh(r: &mut Relay) {
    if r.state != c::RUNNING { return; }
    let Some((_, target, task)) = r.child else { return };
    if !crate::fs_service::live_run(target, task) { r.state = c::ENDED; r.code = c::CRASHED; }
}
/// The relay whose attached run is program row `k`.
fn terminal_of(k: usize) -> Option<usize> {
    relays().iter().position(|r| r.state == c::RUNNING && r.child.is_some_and(|(i, ..)| i == k))
}

/// One request from program row `k` (`launcher`: it holds `+launch`). `slot` is its region.
pub fn handle(k: usize, launcher: bool, r: w::Request, slot: u64) -> u64 {
    let Ok(view) = sex_pdx::memory_region_view(slot, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE) else { return w::reply(r.seq, w::IO, 0) };
    let region = unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) };
    let data = &mut region[w::PATH_BYTES..w::CACHE_AT];
    let want = (r.length as usize).min(data.len());
    let Some((op, at)) = c::split(r.offset) else { return w::reply(r.seq, w::INVALID, 0) };
    match op {
        c::ATTACHED | c::WRITE | c::READ | c::EXIT | c::SIZE | c::SET_MODE | c::POLL if at == 0 => {
            let Some(t) = terminal_of(k) else { return w::reply(r.seq, w::UNSUPPORTED, 0) };
            let relay = &mut relays()[t];
            let Some(ri) = relay.rings else { return w::reply(r.seq, w::UNSUPPORTED, 0) };
            let ring = rings(ri);
            match op {
                c::ATTACHED => w::reply(r.seq, w::OK, 0),
                c::WRITE => {
                    if core::mem::take(&mut relay.interrupt) { return w::reply(r.seq, c::INTERRUPTED, 0); }
                    w::reply(r.seq, w::OK, ring.output.push(&data[..want]) as u32)
                }
                c::READ => {
                    if core::mem::take(&mut relay.interrupt) { return w::reply(r.seq, c::INTERRUPTED, 0); }
                    let n = ring.input.pop(&mut data[..want]);
                    if n == 0 && relay.hung_up { return w::reply(r.seq, c::HUNG_UP, 0); }
                    w::reply(r.seq, w::OK, n as u32)
                }
                c::POLL => {
                    if core::mem::take(&mut relay.interrupt) { return w::reply(r.seq, c::INTERRUPTED, 0); }
                    if ring.input.is_empty() && relay.hung_up { return w::reply(r.seq, c::HUNG_UP, 0); }
                    w::reply(r.seq, w::OK, ring.input.len() as u32)
                }
                c::SIZE => {
                    if data.len() < c::SIZE_BYTES { return w::reply(r.seq, w::INVALID, 0); }
                    relay.size.encode(data);
                    w::reply(r.seq, w::OK, c::SIZE_BYTES as u32)
                }
                c::SET_MODE => {
                    if r.length > c::MODES as u64 { return w::reply(r.seq, w::INVALID, 0); }
                    let before = core::mem::replace(&mut relay.mode, r.length as u8);
                    if before != relay.mode { sex_pdx::serial_record!("[files.console.mode] terminal_row={} slot={} mode={}", t / c::SLOTS, t % c::SLOTS, relay.mode); }
                    w::reply(r.seq, w::OK, before as u32)
                }
                _ => {
                    relay.state = c::ENDED;
                    relay.code = r.length as u32 & 0xFFFF;
                    w::reply(r.seq, w::OK, 0)
                }
            }
        }
        c::PUT | c::TAKE | c::INTERRUPT | c::HANG_UP | c::RELEASE | c::SET_SIZE if launcher => {
            let id = relay_id(k, at);
            let relay = &mut relays()[id];
            refresh(relay);
            match op {
                c::PUT => {
                    let Some(ri) = relay.rings.filter(|_| matches!(relay.state, c::RUNNING | c::STARTING)) else { return w::reply(r.seq, w::NOT_FOUND, 0) };
                    w::reply(r.seq, w::OK, rings(ri).input.push(&data[..want]) as u32)
                }
                c::TAKE => {
                    let room = want.saturating_sub(c::STATUS_BYTES);
                    let n = match relay.rings { Some(ri) => rings(ri).output.pop(&mut data[c::STATUS_BYTES..c::STATUS_BYTES + room]), None => 0 };
                    let (target, task) = relay.child.map_or((0, 0), |(_, t, task)| (t, task));
                    c::Status { state: relay.state, mode: relay.mode, code: relay.code, target, task }.encode(data);
                    w::reply(r.seq, w::OK, n as u32)
                }
                c::INTERRUPT => { relay.interrupt = true; w::reply(r.seq, w::OK, 0) }
                c::HANG_UP => { relay.hung_up = true; w::reply(r.seq, w::OK, 0) }
                c::SET_SIZE => {
                    let Some((cols, rows)) = c::split_size(r.length) else { return w::reply(r.seq, w::INVALID, 0) };
                    relay.size.set(cols, rows);
                    w::reply(r.seq, w::OK, 0)
                }
                _ => {
                    if relay.state == c::RUNNING { return w::reply(r.seq, w::PENDING, 0); }
                    reset(id);
                    w::reply(r.seq, w::OK, 0)
                }
            }
        }
        _ => w::reply(r.seq, w::DENIED, 0),
    }
}
