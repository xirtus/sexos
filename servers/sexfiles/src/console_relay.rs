// SPDX-License-Identifier: GPL-3.0-only
//! Consoles attached to terminals (`sex_pdx::console_relay`): per terminal row, the run it
//! started and two byte rings. Files only moves bytes; Shell starts the run, and only for
//! the terminal that prepared it.

use sex_pdx::console_relay as c;
use sex_pdx::fs as w;
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};

const ROWS: usize = sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Program);

struct Relay {
    state: u8,
    /// The attached run: its program row, target and task.
    child: Option<(usize, u32, u32)>,
    code: u32,
    interrupt: bool,
    hung_up: bool,
    input: c::Ring<{ c::INPUT_BYTES }>,
    output: c::Ring<{ c::OUTPUT_BYTES }>,
}
impl Relay {
    const fn new() -> Relay { Relay { state: c::NONE, child: None, code: 0, interrupt: false, hung_up: false, input: c::Ring::new(), output: c::Ring::new() } }
}
static mut RELAYS: [Relay; ROWS] = [const { Relay::new() }; ROWS];
fn relays() -> &'static mut [Relay; ROWS] { unsafe { &mut *core::ptr::addr_of_mut!(RELAYS) } }

/// Whether terminal `k` may prepare a run: none is attached to it now.
pub fn may_prepare(k: usize) -> bool {
    let r = &mut relays()[k];
    refresh(r);
    matches!(r.state, c::NONE | c::ENDED) || r.state == c::STARTING
}
/// Terminal `k` prepared a run: the next one Shell binds for it is attached.
pub fn prepared(k: usize) {
    let r = &mut relays()[k];
    *r = Relay::new();
    r.state = c::STARTING;
}
/// Shell bound program row `index` as `target`'s `task`; `launcher` is the terminal row
/// that prepared it, if one did. A row bound again is no longer anyone's attached run.
pub fn bound(launcher: Option<usize>, index: usize, target: u32, task: u32) {
    for r in relays().iter_mut() {
        if r.child.is_some_and(|(i, ..)| i == index) && r.state == c::RUNNING { r.state = c::ENDED; r.code = c::CRASHED; r.child = None; }
    }
    let Some(k) = launcher else { return };
    let r = &mut relays()[k];
    r.child = Some((index, target, task));
    r.state = c::RUNNING;
    sex_pdx::serial_record!("[files.console.attached] terminal_row={} target={} task={}", k, target, task);
}
/// The run terminal `k` prepared could not be loaded.
pub fn not_started(k: usize) {
    let r = &mut relays()[k];
    if r.state == c::STARTING { r.state = c::ENDED; r.code = c::NOT_STARTED; }
}
/// A run that went away without saying how it ended.
fn refresh(r: &mut Relay) {
    if r.state != c::RUNNING { return; }
    let Some((_, target, task)) = r.child else { return };
    if !crate::fs_service::live_run(target, task) { r.state = c::ENDED; r.code = c::CRASHED; }
}
/// The terminal row whose attached run is program row `k`.
fn terminal_of(k: usize) -> Option<usize> {
    relays().iter().position(|r| r.state == c::RUNNING && r.child.is_some_and(|(i, ..)| i == k))
}

/// One request from program row `k` (`launcher`: it holds `+launch`). `slot` is its region.
pub fn handle(k: usize, launcher: bool, r: w::Request, slot: u64) -> u64 {
    let Ok(view) = sex_pdx::memory_region_view(slot, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE) else { return w::reply(r.seq, w::IO, 0) };
    let region = unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) };
    let data = &mut region[w::PATH_BYTES..w::CACHE_AT];
    let want = (r.length as usize).min(data.len());
    match r.offset {
        c::ATTACHED | c::WRITE | c::READ | c::EXIT => {
            let Some(t) = terminal_of(k) else { return w::reply(r.seq, w::UNSUPPORTED, 0) };
            let relay = &mut relays()[t];
            match r.offset {
                c::ATTACHED => w::reply(r.seq, w::OK, 0),
                c::WRITE => {
                    if core::mem::take(&mut relay.interrupt) { return w::reply(r.seq, c::INTERRUPTED, 0); }
                    w::reply(r.seq, w::OK, relay.output.push(&data[..want]) as u32)
                }
                c::READ => {
                    if core::mem::take(&mut relay.interrupt) { return w::reply(r.seq, c::INTERRUPTED, 0); }
                    let n = relay.input.pop(&mut data[..want]);
                    if n == 0 && relay.hung_up { return w::reply(r.seq, c::HUNG_UP, 0); }
                    w::reply(r.seq, w::OK, n as u32)
                }
                _ => {
                    relay.state = c::ENDED;
                    relay.code = r.length as u32 & 0xFFFF;
                    w::reply(r.seq, w::OK, 0)
                }
            }
        }
        c::PUT | c::TAKE | c::INTERRUPT | c::HANG_UP | c::RELEASE if launcher => {
            let relay = &mut relays()[k];
            refresh(relay);
            match r.offset {
                c::PUT => {
                    if relay.state != c::RUNNING && relay.state != c::STARTING { return w::reply(r.seq, w::NOT_FOUND, 0); }
                    w::reply(r.seq, w::OK, relay.input.push(&data[..want]) as u32)
                }
                c::TAKE => {
                    let room = want.saturating_sub(c::STATUS_BYTES);
                    let n = relay.output.pop(&mut data[c::STATUS_BYTES..c::STATUS_BYTES + room]);
                    let target = relay.child.map_or(0, |(_, t, _)| t);
                    c::Status { state: relay.state, code: relay.code, target }.encode(data);
                    w::reply(r.seq, w::OK, n as u32)
                }
                c::INTERRUPT => { relay.interrupt = true; w::reply(r.seq, w::OK, 0) }
                c::HANG_UP => { relay.hung_up = true; w::reply(r.seq, w::OK, 0) }
                _ => {
                    if relay.state == c::RUNNING { return w::reply(r.seq, w::PENDING, 0); }
                    *relay = Relay::new();
                    w::reply(r.seq, w::OK, 0)
                }
            }
        }
        _ => w::reply(r.seq, w::DENIED, 0),
    }
}
