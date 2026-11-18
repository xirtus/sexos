// SPDX-License-Identifier: GPL-3.0-only
//! A Windows CLAP plugin in a cell, driven from the host: the bridge part of what
//! yabridge does on Linux (CW8, `docs/handoff/CROSSDRESS_2026_11_16.md`). yabridge runs
//! the Windows plugin in a Wine process and talks to it over sockets; here the plugin is
//! the cell's only module and the host calls its functions directly, one VM entry per
//! call, with the audio in cell memory.
//!
//! The host side of the CLAP ABI lives in a bridge area of the cell: the `clap_host_t`,
//! the `clap_process_t` with its audio buffers and event lists, and the strings the
//! plugin is given. Every function pointer in them is a host callback thunk
//! (`process::callback_va`), so `get_extension`, `request_*`, `in_events.size/get` and
//! `out_events.try_push` reach `Calls::callback` here.
//!
//! Offsets are those of the upstream CLAP headers (1.x, x64); `test/gain.c` pins each of
//! them with a `_Static_assert` when the test plugin is built.
use crate::cell::{Access, Cell, PAGE};
use crate::process::{callback_va, Host, LoadError, Stop, WinProcess};
use alloc::string::String;
use alloc::vec::Vec;
use drag_vmm::memory::Ram;
use drag_vmm::vmm::Vcpu;

/// The bridge area: host structures and audio, user read/write in the cell.
pub const BRIDGE_VA: u64 = 0x0000_7FF7_E000_0000;
const BRIDGE_BYTES: u64 = 1 << 20;
const HOST: u64 = BRIDGE_VA;
const STRINGS: u64 = BRIDGE_VA + 0x100;
const PROCESS: u64 = BRIDGE_VA + 0x1000;
const AUDIO_IN: u64 = PROCESS + 0x40;
const AUDIO_OUT: u64 = PROCESS + 0x60;
const IN_PTRS: u64 = PROCESS + 0x80;
const OUT_PTRS: u64 = PROCESS + 0x90;
const IN_EVENTS: u64 = PROCESS + 0xA0;
const OUT_EVENTS: u64 = PROCESS + 0xC0;
const SCRATCH: u64 = BRIDGE_VA + 0x2000; // a clap_param_info_t (1320 bytes) or a double
const EVENTS: u64 = BRIDGE_VA + 0x3000;
const EVENT_SIZE: u64 = 56; // clap_event_param_value_t
/// Most parameter changes one `process` call delivers.
pub const MAX_EVENTS: usize = 256;
const SAMPLES: u64 = BRIDGE_VA + 0x8000;
/// Most frames one `process` call takes (four channels of f32 fit the bridge area).
pub const MAX_FRAMES: u32 = (((BRIDGE_BYTES - 0x8000) / 16) & !0xF) as u32;

const PARAM_INFO_SIZE: usize = 1320;
const CLAP_VERSION: [u32; 3] = [1, 2, 0];
const CORE_EVENT_SPACE: u16 = 0;
const EVENT_PARAM_VALUE: u16 = 5;
const PROCESS_ERROR: u32 = 0;

/// Host callbacks: the function pointers in the host's structures.
mod cb {
    pub const GET_EXTENSION: u32 = 0;
    pub const REQUEST_RESTART: u32 = 1;
    pub const REQUEST_PROCESS: u32 = 2;
    pub const REQUEST_CALLBACK: u32 = 3;
    pub const IN_SIZE: u32 = 4;
    pub const IN_GET: u32 = 5;
    pub const OUT_TRY_PUSH: u32 = 6;
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Descriptor { pub id: String, pub name: String, pub vendor: String, pub version: String, pub features: Vec<String> }

#[derive(Clone, Debug, PartialEq)]
pub struct ParamInfo { pub id: u32, pub flags: u32, pub name: String, pub min: f64, pub max: f64, pub default: f64 }

/// A parameter value at a frame of a block (CLAP_EVENT_PARAM_VALUE), either way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamChange { pub time: u32, pub id: u32, pub value: f64 }

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Load(LoadError),
    /// The DLL has no `clap_entry` export.
    NotClap,
    /// The cell stopped inside a plugin call.
    Stopped(Stop),
    /// The plugin said no (the step that refused).
    Refused(&'static str),
    /// The plugin gave the host something it cannot use (what).
    Bad(&'static str),
    /// The host's request does not fit (too many frames or events, wrong state).
    Host(&'static str),
}
impl From<Stop> for Error { fn from(s: Stop) -> Self { Error::Stopped(s) } }
impl From<LoadError> for Error { fn from(e: LoadError) -> Self { Error::Load(e) } }

/// Counts of what the plugin asked of the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Requests { pub extensions: u32, pub restart: u32, pub process: u32, pub callback: u32 }

/// The host wrapper a plugin call runs under: CLAP callbacks here, the rest to `inner`.
struct Calls<'a, H: Host> { inner: &'a mut H, events: u32, pushed: Vec<ParamChange>, requests: &'a mut Requests, bad: bool }

impl<H: Host> Host for Calls<'_, H> {
    fn output(&mut self, stream: u32, bytes: &[u8]) { self.inner.output(stream, bytes) }
    fn now_ms(&mut self) -> u64 { self.inner.now_ms() }
    fn sleep(&mut self, ms: u32) { self.inner.sleep(ms) }
    fn keep_going(&mut self) -> bool { self.inner.keep_going() }
    fn callback(&mut self, n: u32, args: [u64; 4], cell: &mut Cell) -> u64 {
        match n {
            // No host extensions are offered yet: the plugin must cope (CLAP allows it).
            cb::GET_EXTENSION => { self.requests.extensions += 1; 0 }
            cb::REQUEST_RESTART => { self.requests.restart += 1; 0 }
            cb::REQUEST_PROCESS => { self.requests.process += 1; 0 }
            cb::REQUEST_CALLBACK => { self.requests.callback += 1; 0 }
            cb::IN_SIZE => self.events as u64,
            cb::IN_GET => if args[1] < self.events as u64 { EVENTS + args[1] * EVENT_SIZE } else { 0 },
            cb::OUT_TRY_PUSH => {
                let mut e = [0u8; EVENT_SIZE as usize];
                let Some(()) = cell.read_user(args[1], &mut e[..16]) else { self.bad = true; return 0 };
                let size = u32_le(&e, 0);
                let (space, kind) = (u16::from_le_bytes([e[8], e[9]]), u16::from_le_bytes([e[10], e[11]]));
                if space != CORE_EVENT_SPACE || kind != EVENT_PARAM_VALUE || size as u64 != EVENT_SIZE { return 1; } // accepted, not kept
                if cell.read_user(args[1], &mut e).is_none() || self.pushed.len() >= MAX_EVENTS { self.bad = true; return 0; }
                self.pushed.push(ParamChange { time: u32_le(&e, 4), id: u32_le(&e, 16), value: f64::from_bits(u64_le(&e, 48)) });
                1
            }
            _ => 0,
        }
    }
}

fn u32_le(b: &[u8], at: usize) -> u32 { u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) }
fn u64_le(b: &[u8], at: usize) -> u64 { let mut x = [0u8; 8]; x.copy_from_slice(&b[at..at + 8]); u64::from_le_bytes(x) }

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Phase { Loaded, Entered, Created, Active, Processing }

/// One Windows CLAP plugin instance in its own cell.
pub struct ClapPlugin {
    pub process: WinProcess,
    pub descriptor: Descriptor,
    pub params: Vec<ParamInfo>,
    pub requests: Requests,
    /// How many times `on_main_thread` ran.
    pub main_thread_calls: u32,
    served_callbacks: u32,
    entry: u64,
    plugin: u64,
    params_ext: u64,
    phase: Phase,
    max_frames: u32,
    steady_time: i64,
    strings_next: u64,
}

impl ClapPlugin {
    /// Loads a `.clap` (a 64-bit Windows DLL) into a fresh cell over `ram` and lays out
    /// the host side. Nothing in the plugin runs until `open`.
    pub fn load(ram: Ram, file: &[u8]) -> Result<ClapPlugin, Error> {
        let mut process = WinProcess::load_dll(ram, file)?;
        let entry = process.export("clap_entry").ok_or(Error::NotClap)?;
        process.cell.map(BRIDGE_VA, BRIDGE_BYTES / PAGE, Access::USER_RW).map_err(|e| Error::Load(LoadError::Cell(e)))?;
        let mut p = ClapPlugin {
            process, descriptor: Descriptor::default(), params: Vec::new(), requests: Requests::default(), main_thread_calls: 0, served_callbacks: 0,
            entry, plugin: 0, params_ext: 0, phase: Phase::Loaded, max_frames: 0, steady_time: 0, strings_next: STRINGS,
        };
        // clap_host_t: version, host_data, name, vendor, url, version, then the callbacks.
        let strs = ["SexOS Crossdress", "SexOS", "https://github.com/xirtus/sexos", "0.1.0"].map(|s| p.string(s));
        let mut host = [0u8; 88];
        for (i, v) in CLAP_VERSION.iter().enumerate() { host[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes()); }
        let mut q = |at: usize, v: u64| host[at..at + 8].copy_from_slice(&v.to_le_bytes());
        q(16, 0);
        for (i, s) in strs.iter().enumerate() { q(24 + 8 * i, s.ok_or(Error::Host("strings"))?); }
        for (i, c) in [cb::GET_EXTENSION, cb::REQUEST_RESTART, cb::REQUEST_PROCESS, cb::REQUEST_CALLBACK].iter().enumerate() { q(56 + 8 * i, callback_va(*c)); }
        p.process.cell.write_user(HOST, &host).ok_or(Error::Host("host"))?;
        // The event lists: ctx, then size/get or try_push.
        let mut ins = [0u8; 24];
        ins[8..16].copy_from_slice(&callback_va(cb::IN_SIZE).to_le_bytes());
        ins[16..24].copy_from_slice(&callback_va(cb::IN_GET).to_le_bytes());
        let mut outs = [0u8; 16];
        outs[8..16].copy_from_slice(&callback_va(cb::OUT_TRY_PUSH).to_le_bytes());
        p.process.cell.write_user(IN_EVENTS, &ins).ok_or(Error::Host("events"))?;
        p.process.cell.write_user(OUT_EVENTS, &outs).ok_or(Error::Host("events"))?;
        Ok(p)
    }

    /// A NUL-terminated string in the bridge area's string space.
    fn string(&mut self, s: &str) -> Option<u64> {
        let at = self.strings_next;
        if at + s.len() as u64 + 1 > PROCESS { return None; }
        self.process.cell.write_user(at, s.as_bytes())?;
        self.process.cell.write_user(at + s.len() as u64, &[0])?;
        self.strings_next = (at + s.len() as u64 + 1).next_multiple_of(8);
        Some(at)
    }

    fn ptr(&self, va: u64) -> Result<u64, Error> { let mut b = [0u8; 8]; self.process.cell.read_user(va, &mut b).ok_or(Error::Bad("pointer"))?; Ok(u64::from_le_bytes(b)) }

    fn text(&self, va: u64, max: usize) -> Result<String, Error> {
        let mut out = Vec::new();
        let mut b = [0u8; 1];
        for i in 0..max as u64 {
            self.process.cell.read_user(va + i, &mut b).ok_or(Error::Bad("string"))?;
            if b[0] == 0 { return String::from_utf8(out).map_err(|_| Error::Bad("utf-8")); }
            out.push(b[0]);
        }
        Err(Error::Bad("string too long"))
    }

    /// Calls a plugin function under the CLAP host callbacks. A `request_callback` made
    /// during the call is answered with `plugin.on_main_thread` once the call returns
    /// (the host's one thread is its main thread).
    fn call(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, function: u64, args: &[u64], floats: Option<[u64; 4]>, events: u32) -> Result<(u64, Vec<ParamChange>), Error> {
        let mut calls = Calls { inner: host, events, pushed: Vec::new(), requests: &mut self.requests, bad: false };
        let v = self.process.call(cpu, &mut calls, function, args, floats)?;
        if calls.bad { return Err(Error::Bad("event")); }
        let pushed = calls.pushed;
        self.main_thread(cpu, host)?;
        Ok((v, pushed))
    }

    /// Runs `plugin.on_main_thread` while callbacks are requested (at most 8 rounds, so a
    /// plugin that asks again every time cannot hold the host).
    fn main_thread(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host) -> Result<(), Error> {
        for _ in 0..8 {
            if self.requests.callback == self.served_callbacks || self.plugin == 0 || self.phase < Phase::Created { return Ok(()); }
            self.served_callbacks = self.requests.callback;
            let (f, plugin) = (self.method(88)?, self.plugin);
            let mut calls = Calls { inner: &mut *host, events: 0, pushed: Vec::new(), requests: &mut self.requests, bad: false };
            self.process.call(cpu, &mut calls, f, &[plugin], None)?;
            self.main_thread_calls += 1;
        }
        Ok(())
    }

    fn method(&self, offset: u64) -> Result<u64, Error> {
        let f = self.ptr(self.plugin + offset)?;
        if f == 0 { Err(Error::Bad("null method")) } else { Ok(f) }
    }

    /// DllMain, `clap_entry.init(path)`, the plugin factory, the first plugin's
    /// descriptor, `create_plugin`, `plugin.init` and the parameter list. `path` is the
    /// Windows path the plugin is told it was loaded from.
    pub fn open(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, path: &str) -> Result<(), Error> {
        if self.phase != Phase::Loaded { return Err(Error::Host("already open")); }
        let mut calls = Calls { inner: host, events: 0, pushed: Vec::new(), requests: &mut self.requests, bad: false };
        if !self.process.dll_main(cpu, &mut calls, 1)? { return Err(Error::Refused("DllMain")); }
        let entry = self.entry;
        let mut version = [0u8; 12];
        self.process.cell.read_user(entry, &mut version).ok_or(Error::Bad("clap_entry"))?;
        if u32_le(&version, 0) != 1 { return Err(Error::Bad("CLAP major version")); }
        let path = self.string(path).ok_or(Error::Host("path"))?;
        let init = self.ptr(entry + 16)?;
        if self.call(cpu, host, init, &[path], None, 0)?.0 as u8 == 0 { return Err(Error::Refused("clap_entry.init")); }
        self.phase = Phase::Entered;
        let factory_id = self.string("clap.plugin-factory").ok_or(Error::Host("strings"))?;
        let get_factory = self.ptr(entry + 32)?;
        let factory = self.call(cpu, host, get_factory, &[factory_id], None, 0)?.0;
        if factory == 0 { return Err(Error::Refused("plugin factory")); }
        let count = self.ptr(factory)?;
        if self.call(cpu, host, count, &[factory], None, 0)?.0 as u32 == 0 { return Err(Error::Refused("no plugins")); }
        let get_descriptor = self.ptr(factory + 8)?;
        let d = self.call(cpu, host, get_descriptor, &[factory, 0], None, 0)?.0;
        if d == 0 { return Err(Error::Bad("descriptor")); }
        let field = |s: &Self, at: u64| -> Result<String, Error> { let p = s.ptr(d + at)?; if p == 0 { Ok(String::new()) } else { s.text(p, 512) } };
        self.descriptor = Descriptor { id: field(self, 16)?, name: field(self, 24)?, vendor: field(self, 32)?, version: field(self, 64)?, features: Vec::new() };
        let features = self.ptr(d + 80)?;
        if features != 0 {
            for i in 0..32 {
                let f = self.ptr(features + 8 * i)?;
                if f == 0 { break; }
                let t = self.text(f, 128)?;
                self.descriptor.features.push(t);
            }
        }
        let id = self.ptr(d + 16)?;
        let create = self.ptr(factory + 16)?;
        self.plugin = self.call(cpu, host, create, &[factory, HOST, id], None, 0)?.0;
        if self.plugin == 0 { return Err(Error::Refused("create_plugin")); }
        let init = self.method(16)?;
        let plugin = self.plugin;
        if self.call(cpu, host, init, &[plugin], None, 0)?.0 as u8 == 0 { return Err(Error::Refused("plugin.init")); }
        self.phase = Phase::Created;
        // Requests made during init are served now that the plugin may be called.
        self.main_thread(cpu, host)?;
        // Parameters (clap.params), when the plugin has them.
        let ext_id = self.string("clap.params").ok_or(Error::Host("strings"))?;
        let get_extension = self.method(80)?;
        self.params_ext = self.call(cpu, host, get_extension, &[plugin, ext_id], None, 0)?.0;
        if self.params_ext != 0 {
            let (count, info) = (self.ptr(self.params_ext)?, self.ptr(self.params_ext + 8)?);
            let n = (self.call(cpu, host, count, &[plugin], None, 0)?.0 as u32).min(1024);
            for i in 0..n {
                self.process.cell.write_user(SCRATCH, &[0u8; PARAM_INFO_SIZE]).ok_or(Error::Host("scratch"))?;
                if self.call(cpu, host, info, &[plugin, i as u64, SCRATCH], None, 0)?.0 as u8 == 0 { continue; }
                let mut b = [0u8; PARAM_INFO_SIZE];
                self.process.cell.read_user(SCRATCH, &mut b).ok_or(Error::Bad("param info"))?;
                let name = &b[16..16 + 256];
                let name = String::from_utf8_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(256)]).into_owned();
                self.params.push(ParamInfo { id: u32_le(&b, 0), flags: u32_le(&b, 4), name,
                    min: f64::from_bits(u64_le(&b, 1296)), max: f64::from_bits(u64_le(&b, 1304)), default: f64::from_bits(u64_le(&b, 1312)) });
            }
        }
        Ok(())
    }

    /// The plugin's current value of parameter `id` (`params.get_value`).
    pub fn param_value(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, id: u32) -> Result<Option<f64>, Error> {
        if self.params_ext == 0 || self.phase < Phase::Created { return Ok(None); }
        let get = self.ptr(self.params_ext + 16)?;
        let plugin = self.plugin;
        if self.call(cpu, host, get, &[plugin, id as u64, SCRATCH], None, 0)?.0 as u8 == 0 { return Ok(None); }
        Ok(Some(f64::from_bits(self.ptr(SCRATCH)?)))
    }

    /// `plugin.activate(sample_rate, 1, max_frames)` then `start_processing`.
    pub fn activate(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, sample_rate: f64, max_frames: u32) -> Result<(), Error> {
        if self.phase != Phase::Created { return Err(Error::Host("not created")); }
        if max_frames == 0 || max_frames > MAX_FRAMES { return Err(Error::Host("max_frames")); }
        let (activate, start, plugin) = (self.method(32)?, self.method(48)?, self.plugin);
        // activate(plugin, double sample_rate, uint32 min, uint32 max): the double is
        // argument 2, so it travels in XMM1.
        if self.call(cpu, host, activate, &[plugin, 0, 1, max_frames as u64], Some([0, sample_rate.to_bits(), 0, 0]), 0)?.0 as u8 == 0 {
            return Err(Error::Refused("activate"));
        }
        self.phase = Phase::Active;
        self.max_frames = max_frames;
        if self.call(cpu, host, start, &[plugin], None, 0)?.0 as u8 == 0 { return Err(Error::Refused("start_processing")); }
        self.phase = Phase::Processing;
        Ok(())
    }

    /// One `plugin.process` block: stereo `input` to stereo `output` (same length), with
    /// `changes` (sorted by time) as CLAP_EVENT_PARAM_VALUE input events. Returns the
    /// parameter changes the plugin pushed to its output events.
    pub fn process(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host, input: [&[f32]; 2], output: [&mut [f32]; 2], changes: &[ParamChange]) -> Result<Vec<ParamChange>, Error> {
        if self.phase != Phase::Processing { return Err(Error::Host("not processing")); }
        let frames = input[0].len();
        if frames == 0 || frames > self.max_frames as usize || input[1].len() != frames || output[0].len() != frames || output[1].len() != frames {
            return Err(Error::Host("frames"));
        }
        if changes.len() > MAX_EVENTS || changes.windows(2).any(|w| w[0].time > w[1].time) { return Err(Error::Host("events")); }
        let n = frames as u64 * 4;
        let channel = |c: u64| SAMPLES + c * self.max_frames as u64 * 4;
        let cell = &mut self.process.cell;
        for (c, data) in input.iter().enumerate() {
            let bytes: Vec<u8> = data.iter().flat_map(|x| x.to_le_bytes()).collect();
            cell.write_user(channel(c as u64), &bytes).ok_or(Error::Host("audio"))?;
        }
        // Clear the outputs so a plugin that skips them leaves silence, not old audio.
        let zero = alloc::vec![0u8; n as usize];
        for c in 2..4 { cell.write_user(channel(c), &zero).ok_or(Error::Host("audio"))?; }
        let q = |cell: &mut Cell, at: u64, v: u64| cell.write_user(at, &v.to_le_bytes()).ok_or(Error::Host("bridge"));
        q(cell, IN_PTRS, channel(0))?; q(cell, IN_PTRS + 8, channel(1))?;
        q(cell, OUT_PTRS, channel(2))?; q(cell, OUT_PTRS + 8, channel(3))?;
        // clap_audio_buffer_t: data32, data64, channel_count, latency, constant_mask.
        for (at, ptrs) in [(AUDIO_IN, IN_PTRS), (AUDIO_OUT, OUT_PTRS)] {
            let mut b = [0u8; 32];
            b[0..8].copy_from_slice(&ptrs.to_le_bytes());
            b[16..20].copy_from_slice(&2u32.to_le_bytes());
            cell.write_user(at, &b).ok_or(Error::Host("bridge"))?;
        }
        for (i, ch) in changes.iter().enumerate() {
            let mut e = [0u8; EVENT_SIZE as usize];
            e[0..4].copy_from_slice(&(EVENT_SIZE as u32).to_le_bytes());
            e[4..8].copy_from_slice(&ch.time.to_le_bytes());
            e[8..10].copy_from_slice(&CORE_EVENT_SPACE.to_le_bytes());
            e[10..12].copy_from_slice(&EVENT_PARAM_VALUE.to_le_bytes());
            e[16..20].copy_from_slice(&ch.id.to_le_bytes());
            e[32..40].copy_from_slice(&[0xFF; 8]); // note_id, port_index, channel: -1 (wildcards)
            e[40..42].copy_from_slice(&[0xFF; 2]); // key: -1
            e[48..56].copy_from_slice(&ch.value.to_bits().to_le_bytes());
            cell.write_user(EVENTS + i as u64 * EVENT_SIZE, &e).ok_or(Error::Host("events"))?;
        }
        // clap_process_t.
        let mut p = [0u8; 64];
        p[0..8].copy_from_slice(&self.steady_time.to_le_bytes());
        p[8..12].copy_from_slice(&(frames as u32).to_le_bytes());
        p[24..32].copy_from_slice(&AUDIO_IN.to_le_bytes());
        p[32..40].copy_from_slice(&AUDIO_OUT.to_le_bytes());
        p[40..44].copy_from_slice(&1u32.to_le_bytes());
        p[44..48].copy_from_slice(&1u32.to_le_bytes());
        p[48..56].copy_from_slice(&IN_EVENTS.to_le_bytes());
        p[56..64].copy_from_slice(&OUT_EVENTS.to_le_bytes());
        cell.write_user(PROCESS, &p).ok_or(Error::Host("bridge"))?;
        let (process, plugin) = (self.method(72)?, self.plugin);
        let (status, pushed) = self.call(cpu, host, process, &[plugin, PROCESS], None, changes.len() as u32)?;
        if status as u32 == PROCESS_ERROR { return Err(Error::Refused("process")); }
        self.steady_time += frames as i64;
        let mut bytes = alloc::vec![0u8; n as usize];
        let max = self.max_frames as u64;
        for (c, out) in output.into_iter().enumerate() {
            self.process.cell.read_user(SAMPLES + (2 + c as u64) * max * 4, &mut bytes).ok_or(Error::Bad("audio"))?;
            for (i, x) in out.iter_mut().enumerate() { *x = f32::from_le_bytes([bytes[4 * i], bytes[4 * i + 1], bytes[4 * i + 2], bytes[4 * i + 3]]); }
        }
        Ok(pushed)
    }

    /// Stops, deactivates and destroys the plugin, then `clap_entry.deinit` and DllMain
    /// (detach). Safe in any phase.
    pub fn close(&mut self, cpu: &mut impl Vcpu, host: &mut impl Host) -> Result<(), Error> {
        let plugin = self.plugin;
        if self.phase == Phase::Processing { let f = self.method(56)?; self.call(cpu, host, f, &[plugin], None, 0)?; self.phase = Phase::Active; }
        if self.phase == Phase::Active { let f = self.method(40)?; self.call(cpu, host, f, &[plugin], None, 0)?; self.phase = Phase::Created; }
        if self.phase == Phase::Created { let f = self.method(24)?; self.call(cpu, host, f, &[plugin], None, 0)?; self.plugin = 0; self.phase = Phase::Entered; }
        if self.phase == Phase::Entered {
            let f = self.ptr(self.entry + 24)?;
            self.call(cpu, host, f, &[], None, 0)?;
            self.phase = Phase::Loaded;
            let mut calls = Calls { inner: host, events: 0, pushed: Vec::new(), requests: &mut self.requests, bad: false };
            self.process.dll_main(cpu, &mut calls, 0)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_plugin_loads_with_its_entry_and_the_host_side_laid_out() {
        let p = ClapPlugin::load(Ram::new(16 << 20).unwrap(), crate::TEST_CLAP).unwrap();
        assert_eq!(p.process.base, 0x1_8000_0000);
        assert!(p.process.entry != 0, "DllMain");
        assert_eq!(p.entry, p.process.export("clap_entry").unwrap());
        assert!(p.process.imports.iter().all(|i| i.2.is_some()), "every import of the test plugin is implemented");
        // clap_host_t: version 1.2.0, name, get_extension at the callback thunk.
        let cell = &p.process.cell;
        assert_eq!(cell.read_u64(HOST).unwrap() as u32, 1);
        let name = cell.read_u64(HOST + 24).unwrap();
        let mut b = [0u8; 16];
        cell.read_user(name, &mut b).unwrap();
        assert_eq!(&b, b"SexOS Crossdress");
        assert_eq!(cell.read_u64(HOST + 56), Some(callback_va(cb::GET_EXTENSION)));
        assert_eq!(cell.read_u64(IN_EVENTS + 16), Some(callback_va(cb::IN_GET)));
        assert_eq!(cell.read_u64(OUT_EVENTS + 8), Some(callback_va(cb::OUT_TRY_PUSH)));
        // The callback thunk is `mov eax, CALLBACK|n; out 0xE0, eax; ret`.
        let mut code = [0u8; 8];
        cell.read_user(callback_va(cb::OUT_TRY_PUSH), &mut code).unwrap();
        assert_eq!(code[0], 0xB8);
        assert_eq!(u32_le(&code, 1), crate::cell::CALLBACK | cb::OUT_TRY_PUSH);
        assert_eq!(&code[5..8], &[0xE7, 0xE0, 0xC3]);
        for m in cell.maps() { assert!(!(m.access.write && m.access.execute), "W^X in the cell at {:#x}", m.va); }
        assert!(MAX_FRAMES >= 8192);
    }

    #[test]
    fn an_exe_is_not_a_plugin_and_a_plugin_is_not_an_exe() {
        assert!(matches!(ClapPlugin::load(Ram::new(16 << 20).unwrap(), crate::TEST_EXE), Err(Error::Load(_))));
        assert!(WinProcess::load(Ram::new(16 << 20).unwrap(), crate::TEST_CLAP).is_err());
    }

    #[test]
    fn output_events_are_read_from_the_guest_and_checked() {
        let mut p = ClapPlugin::load(Ram::new(16 << 20).unwrap(), crate::TEST_CLAP).unwrap();
        struct Nop;
        impl Host for Nop { fn output(&mut self, _: u32, _: &[u8]) {} fn now_ms(&mut self) -> u64 { 0 } fn sleep(&mut self, _: u32) {} }
        let mut nop = Nop;
        let mut req = Requests::default();
        let mut calls = Calls { inner: &mut nop, events: 2, pushed: Vec::new(), requests: &mut req, bad: false };
        let cell = &mut p.process.cell;
        assert_eq!(calls.callback(cb::IN_SIZE, [IN_EVENTS, 0, 0, 0], cell), 2);
        assert_eq!(calls.callback(cb::IN_GET, [IN_EVENTS, 1, 0, 0], cell), EVENTS + EVENT_SIZE);
        assert_eq!(calls.callback(cb::IN_GET, [IN_EVENTS, 2, 0, 0], cell), 0, "past the end");
        let mut e = [0u8; 56];
        e[0] = 56; e[4] = 7; e[10] = EVENT_PARAM_VALUE as u8; e[16] = 3;
        e[48..56].copy_from_slice(&0.25f64.to_bits().to_le_bytes());
        cell.write_user(SCRATCH, &e).unwrap();
        assert_eq!(calls.callback(cb::OUT_TRY_PUSH, [OUT_EVENTS, SCRATCH, 0, 0], cell), 1);
        assert_eq!(calls.pushed, [ParamChange { time: 7, id: 3, value: 0.25 }]);
        // A pointer outside the guest's user memory is refused, not followed.
        assert_eq!(calls.callback(cb::OUT_TRY_PUSH, [OUT_EVENTS, crate::cell::SYS_VA, 0, 0], cell), 0);
        assert!(calls.bad);
        assert_eq!(calls.callback(cb::GET_EXTENSION, [HOST, 0, 0, 0], cell), 0);
        assert_eq!(req.extensions, 1);
    }
}
