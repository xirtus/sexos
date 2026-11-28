// SPDX-License-Identifier: GPL-3.0-only
//! Web Audio for Kaleidoscope pages. Page script only builds the graph (sex-js's
//! `AudioContext` over `op::AUDIO`); the graph runs here, in the browser, one render
//! quantum (128 frames) at a time.
//!
//! SexOS programs are soft-float, so samples stay integer: Q15 (1.0 = 32768) in i32
//! with headroom, params Q16 per sample, filter coefficients Q26 with i64 sums.
//! Floating point runs only once per quantum (automation, filter coefficients) or when
//! something is set up (FFT tables, buffers).
//!
//! The browser feeds the microphone into tap 0 ([`Hub::feed`]) and plays what
//! [`Hub::render`] mixes from the running contexts. A `MediaStreamAudioDestinationNode`
//! writes its sound to a tap of its own, which other parts of the browser (recording,
//! WebRTC) read with [`Hub::read_tap`].
#![no_std]

extern crate alloc;

pub mod api;
pub mod fft;
pub mod node;
pub mod param;

use alloc::rc::{Rc, Weak};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub use node::{Kind, Node};
pub use param::Param;

pub const QUANTUM: usize = 128;
pub type Block = [i32; QUANTUM];
/// The rate of every real-time context (the program's sound ring).
pub const RATE: u32 = 48_000;
pub const MAX_CHANNELS: usize = 32;
/// The microphone's tap.
pub const MIC: u32 = 0;
/// Frames a tap keeps (one second).
const TAP_FRAMES: usize = 48_000;
/// Longest offline render: ten minutes at 48 kHz.
const OFFLINE_MAX: u64 = 48_000 * 600;
/// Most contexts a page may hold (Chrome stops at about the same).
const CONTEXTS_PER_OWNER: usize = 64;

/// Some channels of one quantum. `silent` when all are zero.
#[derive(Clone, Default)]
pub struct Bus { pub ch: Vec<Block>, pub silent: bool }

impl Bus {
    pub fn new(n: usize) -> Bus { Bus { ch: vec![[0; QUANTUM]; n.max(1)], silent: true } }
    pub fn channels(&self) -> usize { self.ch.len() }
    pub fn reset(&mut self, n: usize) {
        self.ch.resize(n.clamp(1, MAX_CHANNELS), [0; QUANTUM]);
        for c in self.ch.iter_mut() { *c = [0; QUANTUM]; }
        self.silent = true;
    }
    /// Adds `src`, up- or down-mixed to this bus's channels (speaker rules for mono,
    /// stereo, quad and 5.1 unless `discrete`).
    pub fn mix(&mut self, src: &Bus, discrete: bool) {
        if src.silent { return; }
        self.silent = false;
        let (s, d) = (src.channels(), self.channels());
        let add = |dst: &mut Block, a: &Block, q: i32| { for (o, v) in dst.iter_mut().zip(a) { *o += if q == 32768 { *v } else { ((*v as i64 * q as i64) >> 15) as i32 }; } };
        const H: i32 = 23170; // 1/sqrt(2)
        if s == d || discrete {
            for k in 0..s.min(d) { add(&mut self.ch[k], &src.ch[k], 32768); }
            return;
        }
        match (s, d) {
            (1, 2) | (1, 4) => { add(&mut self.ch[0], &src.ch[0], 32768); add(&mut self.ch[1], &src.ch[0], 32768); }
            (1, 6) => add(&mut self.ch[2], &src.ch[0], 32768),
            (2, 1) => { add(&mut self.ch[0], &src.ch[0], 16384); add(&mut self.ch[0], &src.ch[1], 16384); }
            (2, 4) | (2, 6) => { add(&mut self.ch[0], &src.ch[0], 32768); add(&mut self.ch[1], &src.ch[1], 32768); }
            (4, 1) => for k in 0..4 { add(&mut self.ch[0], &src.ch[k], 8192); },
            (4, 2) => { add(&mut self.ch[0], &src.ch[0], 16384); add(&mut self.ch[0], &src.ch[2], 16384); add(&mut self.ch[1], &src.ch[1], 16384); add(&mut self.ch[1], &src.ch[3], 16384); }
            (6, 1) => { add(&mut self.ch[0], &src.ch[0], H); add(&mut self.ch[0], &src.ch[1], H); add(&mut self.ch[0], &src.ch[2], 32768); add(&mut self.ch[0], &src.ch[4], 16384); add(&mut self.ch[0], &src.ch[5], 16384); }
            (6, 2) => {
                add(&mut self.ch[0], &src.ch[0], 32768); add(&mut self.ch[0], &src.ch[2], H); add(&mut self.ch[0], &src.ch[4], H);
                add(&mut self.ch[1], &src.ch[1], 32768); add(&mut self.ch[1], &src.ch[2], H); add(&mut self.ch[1], &src.ch[5], H);
            }
            _ => for k in 0..s.min(d) { add(&mut self.ch[k], &src.ch[k], 32768); },
        }
    }
    /// The mono mix, into `out`.
    pub fn mono(&self, out: &mut Block) {
        *out = [0; QUANTUM];
        if self.silent { return; }
        let mut m = Bus::new(1);
        m.mix(self, false);
        *out = m.ch[0];
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Suspended, Running, Closed }

impl State {
    pub fn name(self) -> &'static str { match self { State::Suspended => "suspended", State::Running => "running", State::Closed => "closed" } }
}

/// Sound passed between parts of the browser: the microphone, or a
/// `MediaStreamAudioDestinationNode`'s stream. Readers keep their own place.
pub struct Tap {
    pub id: u32,
    ring: Vec<[i16; 2]>,
    /// Frames written since the tap began.
    pub written: u64,
    /// The context writing it went away.
    pub ended: bool,
}

impl Tap {
    fn new(id: u32) -> Tap { Tap { id, ring: vec![[0; 2]; TAP_FRAMES], written: 0, ended: false } }
    pub fn push(&mut self, frames: &[[i16; 2]]) {
        for f in frames { let at = (self.written % TAP_FRAMES as u64) as usize; self.ring[at] = *f; self.written += 1; }
    }
    /// Frames from `*cursor` on, up to `out.len()`; a reader that fell more than the
    /// ring behind skips what is gone.
    pub fn read(&self, cursor: &mut u64, out: &mut [[i16; 2]]) -> usize {
        if self.written.saturating_sub(*cursor) > TAP_FRAMES as u64 { *cursor = self.written - TAP_FRAMES as u64; }
        let n = (self.written.saturating_sub(*cursor) as usize).min(out.len());
        for (i, o) in out[..n].iter_mut().enumerate() { *o = self.ring[((*cursor + i as u64) % TAP_FRAMES as u64) as usize]; }
        *cursor += n as u64;
        n
    }
}

/// Sound decoded from a file: interleaved stereo at `rate`, and how many channels
/// the file had.
pub struct Decoded { pub rate: u32, pub channels: u32, pub frames: Vec<[i16; 2]> }

/// Shared tables.
pub struct Tables {
    /// One sine cycle, 2048 steps and one more, Q15.
    pub sine: Vec<i32>,
}

impl Tables {
    fn new() -> Tables {
        Tables { sine: (0..=2048).map(|i| (libm::sin(i as f64 * core::f64::consts::PI / 1024.0) * 32767.0) as i32).collect() }
    }
    /// sin(2 pi phase / 2^32), Q15.
    #[inline]
    pub fn sin(&self, phase: u32) -> i32 {
        let i = (phase >> 21) as usize;
        let f = ((phase >> 5) & 0xFFFF) as i32;
        let (a, b) = (self.sine[i], self.sine[i + 1]);
        a + (((b - a) * f) >> 16)
    }
}

pub struct Context {
    pub id: u32,
    pub owner: u64,
    alive: Weak<()>,
    pub rate: u32,
    /// Offline: (channels, length in frames).
    pub offline: Option<(usize, u64)>,
    pub state: State,
    /// Frames rendered: `currentTime` is this over `rate`.
    pub frame: u64,
    pub nodes: Vec<Node>,
    order: Vec<usize>,
    dirty: bool,
    pub tables: Rc<Tables>,
    scratch: Vec<Bus>,
}

impl Context {
    pub fn time(&self) -> f64 { self.frame as f64 / self.rate as f64 }
    pub fn alive(&self) -> bool { self.alive.strong_count() > 0 }
    /// The destination's last quantum.
    pub fn output(&self) -> &Bus { &self.nodes[0].outputs[0] }

    /// A new node; its index is its id.
    pub fn add(&mut self, kind: Kind, taps: &mut Vec<Tap>) -> usize {
        let mut n = Node::new(kind, self.rate);
        if let Kind::StreamDestination { tap } = &mut n.kind {
            let id = taps.iter().map(|t| t.id).max().unwrap_or(MIC) + 1;
            taps.push(Tap::new(id));
            *tap = id;
        }
        self.nodes.push(n);
        self.dirty = true;
        self.nodes.len() - 1
    }

    pub fn connect(&mut self, from: usize, output: usize, to: usize, input: usize) -> bool {
        if from >= self.nodes.len() || to >= self.nodes.len() || output >= self.nodes[from].outputs.len() || input >= self.nodes[to].inputs.len() { return false; }
        let c = (from, output);
        if !self.nodes[to].inputs[input].contains(&c) { self.nodes[to].inputs[input].push(c); }
        self.dirty = true;
        true
    }

    pub fn connect_param(&mut self, from: usize, output: usize, to: usize, param: usize) -> bool {
        if from >= self.nodes.len() || to >= self.nodes.len() || output >= self.nodes[from].outputs.len() || param >= self.nodes[to].params.len() { return false; }
        let c = (from, output);
        if !self.nodes[to].params[param].inputs.contains(&c) { self.nodes[to].params[param].inputs.push(c); }
        self.dirty = true;
        true
    }

    /// Takes away connections from `from`: those of `output` (or all), to `to` (or
    /// any), into `input` or `param` (or any).
    pub fn disconnect(&mut self, from: usize, output: Option<usize>, to: Option<usize>, input: Option<usize>, param: Option<usize>) {
        let hit = |c: &(usize, usize)| c.0 == from && output.is_none_or(|o| c.1 == o);
        for (j, n) in self.nodes.iter_mut().enumerate() {
            if to.is_some_and(|t| t != j) { continue; }
            if param.is_none() { for (k, i) in n.inputs.iter_mut().enumerate() { if input.is_none_or(|x| x == k) { i.retain(|c| !hit(c)); } } }
            if input.is_none() { for (k, p) in n.params.iter_mut().enumerate() { if param.is_none_or(|x| x == k) { p.inputs.retain(|c| !hit(c)); } } }
        }
        self.dirty = true;
    }

    /// Inputs before the nodes they feed; a cycle (through a delay) is cut where it
    /// closes, so that edge hears the quantum before.
    fn sort(&mut self) {
        let n = self.nodes.len();
        let mut mark = vec![0u8; n];
        let mut order = Vec::with_capacity(n);
        for root in 0..n {
            if mark[root] != 0 { continue; }
            // (node, next input to look at)
            let mut stack = vec![(root, 0usize)];
            mark[root] = 1;
            while let Some(&mut (i, ref mut k)) = stack.last_mut() {
                let deps: usize = self.nodes[i].inputs.iter().map(|v| v.len()).sum::<usize>() + self.nodes[i].params.iter().map(|p| p.inputs.len()).sum::<usize>();
                if *k < deps {
                    let at = *k;
                    *k += 1;
                    let mut j = None;
                    let mut c = at;
                    for v in self.nodes[i].inputs.iter().chain(self.nodes[i].params.iter().map(|p| &p.inputs)) {
                        if c < v.len() { j = Some(v[c].0); break; }
                        c -= v.len();
                    }
                    if let Some(j) = j { if mark[j] == 0 { mark[j] = 1; stack.push((j, 0)); } }
                } else {
                    mark[i] = 2;
                    order.push(i);
                    stack.pop();
                }
            }
        }
        self.order = order;
        self.dirty = false;
    }

    /// Renders one quantum.
    pub fn quantum(&mut self, taps: &mut [Tap], events: &mut Vec<(u64, String)>) {
        if self.dirty { self.sort(); }
        for k in 0..self.order.len() {
            let i = self.order[k];
            self.process(i, taps, events);
        }
        self.frame += QUANTUM as u64;
    }

    fn process(&mut self, i: usize, taps: &mut [Tap], events: &mut Vec<(u64, String)>) {
        let mut ins = core::mem::take(&mut self.scratch);
        let ports = self.nodes[i].inputs.len();
        ins.resize_with(ports.max(1), Bus::default);
        for (p, bus) in ins.iter_mut().enumerate().take(ports) {
            let n = &self.nodes[i];
            let wanted = n.input_channels(p, |j, o| self.nodes[j].outputs.get(o).filter(|b| !b.silent).map(|b| b.channels()));
            bus.reset(wanted);
            for &(j, o) in &n.inputs[p] {
                if j == i { continue; }
                if let Some(b) = self.nodes[j].outputs.get(o) { bus.mix(b, n.discrete); }
            }
        }
        let mut values: [param::Values; 5] = Default::default();
        let n = &self.nodes[i];
        for (p, v) in values.iter_mut().enumerate().take(n.params.len()) {
            let par = &n.params[p];
            par.render(self.frame, self.rate, v);
            for &(j, o) in &par.inputs {
                let Some(b) = self.nodes[j].outputs.get(o) else { continue };
                if b.silent { continue; }
                let mut m = [0; QUANTUM];
                b.mono(&mut m);
                for (x, s) in v.at.iter_mut().zip(m.iter()) { *x = x.saturating_add(s * 2); }
                v.constant = false;
            }
        }
        let frame = self.frame;
        let tables = self.tables.clone();
        let ended = self.nodes[i].process(&ins[..ports], &values, frame, &tables, taps);
        if ended { events.push((self.owner, alloc::format!("audio\tended\t{}\t{}", self.id, i))); }
        self.scratch = ins;
    }
}

/// Every context of every page, the taps, and what to tell the pages.
pub struct Hub {
    pub contexts: Vec<Context>,
    pub taps: Vec<Tap>,
    /// (page owner, dispatch text).
    pub events: Vec<(u64, String)>,
    /// Decodes a file's bytes (the browser sets this to sex-media's decoders).
    pub decoder: Option<fn(&[u8]) -> Option<Decoded>>,
    next: u32,
    tables: Rc<Tables>,
}

impl Default for Hub { fn default() -> Self { Self::new() } }

impl Hub {
    pub fn new() -> Hub { Hub { contexts: Vec::new(), taps: vec![Tap::new(MIC)], events: Vec::new(), decoder: None, next: 1, tables: Rc::new(Tables::new()) } }

    /// A context for page `owner`, which lives as long as `alive` does.
    pub fn create(&mut self, owner: u64, alive: Weak<()>, offline: Option<(usize, u64, u32)>) -> Option<u32> {
        self.prune();
        if self.contexts.iter().filter(|c| c.owner == owner && c.state != State::Closed).count() >= CONTEXTS_PER_OWNER { return None; }
        let (rate, off) = match offline {
            Some((ch, len, rate)) => {
                if !(1..=MAX_CHANNELS).contains(&ch) || len == 0 || len > OFFLINE_MAX || !(3000..=768_000).contains(&rate) { return None; }
                (rate, Some((ch, len)))
            }
            None => (RATE, None),
        };
        let id = self.next;
        self.next += 1;
        let mut c = Context { id, owner, alive, rate, offline: off, state: if off.is_some() { State::Suspended } else { State::Running }, frame: 0, nodes: Vec::new(), order: Vec::new(), dirty: true, tables: self.tables.clone(), scratch: Vec::new() };
        let mut d = Node::new(Kind::Destination, rate);
        if let Some((ch, _)) = off { d.count = ch; d.outputs[0] = Bus::new(ch); }
        c.nodes.push(d);
        self.contexts.push(c);
        Some(id)
    }

    pub fn context(&mut self, owner: u64, id: u32) -> Option<&mut Context> {
        self.contexts.iter_mut().find(|c| c.id == id && c.owner == owner)
    }

    /// Drops contexts whose page went away, and ends their taps.
    pub fn prune(&mut self) {
        let before = self.contexts.len();
        let mut gone = Vec::new();
        self.contexts.retain(|c| {
            let keep = c.alive();
            if !keep { for n in &c.nodes { if let Kind::StreamDestination { tap } = n.kind { gone.push(tap); } } }
            keep
        });
        if before == self.contexts.len() { return; }
        for t in self.taps.iter_mut() { if gone.contains(&t.id) { t.ended = true; } }
        self.taps.retain(|t| !t.ended || t.id == MIC);
        let owners: Vec<u64> = self.contexts.iter().map(|c| c.owner).collect();
        self.events.retain(|(o, _)| owners.contains(o));
    }

    /// Closes a page's contexts now (it navigated away).
    pub fn page_left(&mut self, owner: u64) {
        for c in self.contexts.iter_mut() { if c.owner == owner { c.alive = Weak::new(); } }
        self.prune();
    }

    /// Microphone frames (48 kHz stereo) for tap 0.
    pub fn feed(&mut self, frames: &[[i16; 2]]) { if let Some(t) = self.taps.iter_mut().find(|t| t.id == MIC) { t.push(frames); } }

    pub fn tap(&self, id: u32) -> Option<&Tap> { self.taps.iter().find(|t| t.id == id) }

    /// Reads a tap (see [`Tap::read`]). None when there is no such tap.
    pub fn read_tap(&self, id: u32, cursor: &mut u64, out: &mut [[i16; 2]]) -> Option<usize> { Some(self.tap(id)?.read(cursor, out)) }

    /// Whether any real-time context is running.
    pub fn running(&self) -> bool { self.contexts.iter().any(|c| c.offline.is_none() && c.state == State::Running && c.alive()) }

    /// One quantum of every running real-time context, mixed into `out` (stereo).
    /// True when anything was heard.
    pub fn render(&mut self, out: &mut [[i32; 2]; QUANTUM]) -> bool {
        *out = [[0; 2]; QUANTUM];
        let mut heard = false;
        let Hub { contexts, taps, events, .. } = self;
        for c in contexts.iter_mut() {
            if c.offline.is_some() || c.state != State::Running || !c.alive() { continue; }
            c.quantum(taps, events);
            let b = c.output();
            if b.silent { continue; }
            heard = true;
            let (l, r) = (&b.ch[0], &b.ch[b.channels().min(2) - 1]);
            for (k, o) in out.iter_mut().enumerate() { o[0] += l[k]; o[1] += r[k]; }
        }
        if events.len() > 512 { events.drain(..events.len() - 512); }
        heard
    }

    /// Renders offline context `id` to its length: planar f32, channel after channel.
    pub fn render_offline(&mut self, owner: u64, id: u32) -> Option<Vec<u8>> {
        let Hub { contexts, taps, events, .. } = self;
        let c = contexts.iter_mut().find(|c| c.id == id && c.owner == owner)?;
        let (ch, len) = c.offline?;
        if c.state == State::Closed { return None; }
        c.state = State::Running;
        let mut planes = vec![Vec::with_capacity(len as usize); ch];
        while c.frame < len {
            let n = ((len - c.frame) as usize).min(QUANTUM);
            c.quantum(taps, events);
            let b = c.output();
            for (k, p) in planes.iter_mut().enumerate() {
                let src = &b.ch[k.min(b.channels() - 1)];
                let src_ok = k < b.channels() && !b.silent;
                for s in &src[..n] { p.push(if src_ok { *s } else { 0 }); }
            }
        }
        c.state = State::Closed;
        let mut out = Vec::with_capacity(ch * len as usize * 4);
        for p in &planes { for s in p { out.extend_from_slice(&(*s as f32 / 32768.0).to_le_bytes()); } }
        Some(out)
    }
}

/// Q15 sample to i16, saturating.
#[inline]
pub fn clip(v: i32) -> i16 { v.clamp(-32768, 32767) as i16 }
