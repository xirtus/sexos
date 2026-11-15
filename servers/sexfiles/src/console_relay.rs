// SPDX-License-Identifier: GPL-3.0-only
//! Consoles attached to terminals (`sex_pdx::console_relay`): per terminal row and slot
//! (a tab or pane), the run it started, the window's size and the run's terminal mode.
//! The byte rings come from a pool shared by every slot: no more runs than there are
//! program rows can be attached at once, so there are only that many ring pairs, the
//! same memory as one pair per row. Files only moves bytes; Shell starts the run, and
//! only for the terminal that prepared it.
//!
//! Any program's slot may also hold a pipe run (`console_relay::pipe_prepare`): a child
//! of its own whose stdin, stdout and stderr it handed over, with an environment and a
//! working folder. Then the slot also keeps a stderr ring and the environment block, and
//! the child's working folder becomes a grant of the child's when Shell binds it.

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
    /// A pipe run's stdio word (`console_relay::stdio`); None for a terminal's run.
    pipe: Option<u8>,
    /// Streams the starter stopped reading (`CLOSE_READ`): bit 0 stdout, bit 1 stderr.
    deaf: u8,
}
impl Relay {
    const fn new() -> Relay { Relay { state: c::NONE, child: None, code: 0, interrupt: false, hung_up: false, mode: 0, size: c::Size { cols: 0, rows: 0, changes: 0 }, rings: None, pipe: None, deaf: 0 } }
}
struct Rings {
    input: c::Ring<{ c::INPUT_BYTES }>,
    output: c::Ring<{ c::OUTPUT_BYTES }>,
    /// A pipe run's stderr, its environment block and working folder (`@/absolute/path`).
    error: c::Ring<{ c::ERROR_BYTES }>,
    env: [u8; c::ENV_BYTES],
    env_len: usize,
    cwd: [u8; w::PATH_BYTES + 1],
    cwd_len: usize,
}
impl Rings {
    const fn new() -> Rings { Rings { input: c::Ring::new(), output: c::Ring::new(), error: c::Ring::new(), env: [0; c::ENV_BYTES], env_len: 0, cwd: [0; w::PATH_BYTES + 1], cwd_len: 0 } }
    fn clear(&mut self) { self.input.clear(); self.output.clear(); self.error.clear(); self.env_len = 0; self.cwd_len = 0; }
}
static mut RELAYS: [Relay; RELAYS_N] = [const { Relay::new() }; RELAYS_N];
static mut RINGS: [Rings; ROWS] = [const { Rings::new() }; ROWS];
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
    // the least wanted, but any ended run will do. A pipe run's output not yet read is
    // its starter's data, as a file would be: it is taken last, and never while unread.
    let unread = |r: &Relay| r.pipe.is_some() && r.rings.is_some_and(|i| !rings(i).output.is_empty() || !rings(i).error.is_empty());
    let ended = |r: &Relay| r.state == c::ENDED && r.rings.is_some();
    let idle = relays().iter().position(|r| ended(r) && r.pipe.is_none())
        .or_else(|| relays().iter().position(|r| ended(r) && !unread(r)));
    match idle { Some(other) => { reset(other); Ok(()) } None => Err(w::UNAVAILABLE) }
}
/// Relay `id` prepared a run: the next one Shell binds for it is attached. `pipe` is
/// a pipe run's stdio word.
pub fn prepared(id: usize, pipe: Option<u8>) {
    reset(id);
    let r = &mut relays()[id];
    r.rings = pool().take();
    if let Some(i) = r.rings { rings(i).clear(); }
    r.state = c::STARTING;
    r.pipe = pipe;
}
/// The working folder relay `id`'s pipe run gets, `@/absolute/path`, if its starter set one.
pub fn cwd_grant(id: usize) -> Option<&'static [u8]> {
    let r = &relays()[id];
    let i = r.rings.filter(|_| r.pipe.is_some())?;
    let ring = rings(i);
    (ring.cwd_len > 0).then(|| &ring.cwd[..ring.cwd_len])
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
    match r.pipe {
        Some(word) => sex_pdx::serial_record!("[files.pipe.attached] starter_row={} slot={} target={} task={} stdio={:#x}", id / c::SLOTS, id % c::SLOTS, target, task, word),
        None => sex_pdx::serial_record!("[files.console.attached] terminal_row={} slot={} target={} task={}", id / c::SLOTS, id % c::SLOTS, target, task),
    }
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
/// `confine` spells a path the row names as the absolute path it holds (SET_CWD).
pub fn handle(k: usize, launcher: bool, r: w::Request, slot: u64, confine: impl FnOnce(&[u8], &mut [u8; w::PATH_BYTES]) -> Option<usize>) -> u64 {
    let Ok(view) = sex_pdx::memory_region_view(slot, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE) else { return w::reply(r.seq, w::IO, 0) };
    let region = unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) };
    let data = &mut region[w::PATH_BYTES..w::CACHE_AT];
    let want = (r.length as usize).min(data.len());
    let Some((op, at)) = c::split(r.offset) else { return w::reply(r.seq, w::INVALID, 0) };
    match op {
        c::ATTACHED | c::WRITE | c::READ | c::EXIT | c::SIZE | c::SET_MODE | c::POLL | c::WRITE_ERR | c::ENV if at == 0 => {
            let Some(t) = terminal_of(k) else { return w::reply(r.seq, w::UNSUPPORTED, 0) };
            let relay = &mut relays()[t];
            let Some(ri) = relay.rings else { return w::reply(r.seq, w::UNSUPPORTED, 0) };
            let ring = rings(ri);
            match op {
                c::ATTACHED => w::reply(r.seq, w::OK, relay.pipe.map_or(0, |word| c::PIPE_RUN | word as u32)),
                c::WRITE => {
                    if core::mem::take(&mut relay.interrupt) { return w::reply(r.seq, c::INTERRUPTED, 0); }
                    if relay.deaf & 1 != 0 { return w::reply(r.seq, c::HUNG_UP, 0); }
                    w::reply(r.seq, w::OK, ring.output.push(&data[..want]) as u32)
                }
                c::WRITE_ERR => {
                    if relay.pipe.is_none() { return w::reply(r.seq, w::UNSUPPORTED, 0); }
                    if relay.deaf & 2 != 0 { return w::reply(r.seq, c::HUNG_UP, 0); }
                    w::reply(r.seq, w::OK, ring.error.push(&data[..want]) as u32)
                }
                c::ENV => {
                    if relay.pipe.is_none() { return w::reply(r.seq, w::UNSUPPORTED, 0); }
                    let n = c::env_block(&ring.cwd[..ring.cwd_len], &ring.env[..ring.env_len], r.length as usize, data);
                    w::reply(r.seq, w::OK, n as u32)
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
        c::PUT | c::TAKE | c::INTERRUPT | c::HANG_UP | c::RELEASE | c::SET_SIZE | c::TAKE_ERR | c::SET_ENV | c::SET_CWD | c::CLOSE_READ => {
            let id = relay_id(k, at);
            let relay = &mut relays()[id];
            // A terminal's slots are the terminal grant's; a pipe run's slot is its
            // starter's, whoever that is (only row `k` names row `k`'s slots).
            let pipe = relay.pipe.is_some() && relay.rings.is_some();
            if !(launcher || pipe) { return w::reply(r.seq, w::DENIED, 0); }
            if matches!(op, c::TAKE_ERR | c::SET_ENV | c::SET_CWD | c::CLOSE_READ) && !pipe { return w::reply(r.seq, w::NOT_FOUND, 0); }
            refresh(relay);
            match op {
                c::PUT => {
                    let Some(ri) = relay.rings.filter(|_| matches!(relay.state, c::RUNNING | c::STARTING)) else { return w::reply(r.seq, w::NOT_FOUND, 0) };
                    w::reply(r.seq, w::OK, rings(ri).input.push(&data[..want]) as u32)
                }
                c::TAKE | c::TAKE_ERR => {
                    let room = want.saturating_sub(c::STATUS_BYTES);
                    let out = &mut data[c::STATUS_BYTES..c::STATUS_BYTES + room];
                    let n = match relay.rings {
                        Some(ri) if op == c::TAKE_ERR => rings(ri).error.pop(out),
                        Some(ri) => rings(ri).output.pop(out),
                        None => 0,
                    };
                    let (target, task) = relay.child.map_or((0, 0), |(_, t, task)| (t, task));
                    c::Status { state: relay.state, mode: relay.mode, code: relay.code, target, task }.encode(data);
                    w::reply(r.seq, w::OK, n as u32)
                }
                c::INTERRUPT => { relay.interrupt = true; w::reply(r.seq, w::OK, 0) }
                c::HANG_UP => { relay.hung_up = true; w::reply(r.seq, w::OK, 0) }
                c::CLOSE_READ => {
                    relay.deaf |= r.length as u8 & 3;
                    // Nobody reads what is waiting: it goes, so the ring does not hold the run back.
                    if let Some(ri) = relay.rings {
                        if r.length & 1 != 0 { rings(ri).output.clear(); }
                        if r.length & 2 != 0 { rings(ri).error.clear(); }
                    }
                    w::reply(r.seq, w::OK, 0)
                }
                c::SET_ENV | c::SET_CWD => {
                    if relay.state != c::STARTING { return w::reply(r.seq, w::PENDING, 0); }
                    let Some(ri) = relay.rings else { return w::reply(r.seq, w::NOT_FOUND, 0) };
                    let ring = rings(ri);
                    if op == c::SET_ENV {
                        if r.length as usize > data.len() || ring.env_len + r.length as usize > c::ENV_BYTES { return w::reply(r.seq, w::INVALID, 0); }
                        let n = r.length as usize;
                        ring.env[ring.env_len..ring.env_len + n].copy_from_slice(&data[..n]);
                        ring.env_len += n;
                        return w::reply(r.seq, w::OK, ring.env_len as u32);
                    }
                    let n = (r.length as usize).min(w::PATH_BYTES);
                    let mut asked = [0u8; w::PATH_BYTES];
                    asked[..n].copy_from_slice(&data[..n]);
                    let mut abs = [0u8; w::PATH_BYTES];
                    let Some(len) = confine(&asked[..n], &mut abs) else { return w::reply(r.seq, w::NAME, 0) };
                    ring.cwd[0] = b'@';
                    ring.cwd[1..1 + len].copy_from_slice(&abs[..len]);
                    ring.cwd_len = 1 + len;
                    sex_pdx::serial_record!("[files.pipe.cwd] starter_row={} slot={} cwd=@{}", k, at, core::str::from_utf8(&abs[..len]).unwrap_or("?"));
                    w::reply(r.seq, w::OK, 0)
                }
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
