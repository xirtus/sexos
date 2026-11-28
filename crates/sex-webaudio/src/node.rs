// SPDX-License-Identifier: GPL-3.0-only
//! The nodes and what each does to a quantum. Samples are Q15 in i32, params Q16.
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use crate::fft::Analyser;
use crate::param::{Param, Values};
use crate::{Block, Bus, Tables, Tap, MAX_CHANNELS, QUANTUM};

const BIG: f32 = 3.402_823_5e38;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Max, ClampedMax, Explicit }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave { Sine, Square, Sawtooth, Triangle, Custom }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter { Lowpass, Highpass, Bandpass, Lowshelf, Highshelf, Peaking, Notch, Allpass }

impl Filter {
    pub fn parse(s: &str) -> Option<Filter> {
        Some(match s {
            "lowpass" => Filter::Lowpass, "highpass" => Filter::Highpass, "bandpass" => Filter::Bandpass, "lowshelf" => Filter::Lowshelf,
            "highshelf" => Filter::Highshelf, "peaking" => Filter::Peaking, "notch" => Filter::Notch, "allpass" => Filter::Allpass, _ => return None,
        })
    }
}

/// Sound for an `AudioBufferSourceNode`: Q15 channels at `rate`.
pub struct Buffer { pub rate: u32, pub channels: Vec<Vec<i32>> }

impl Buffer {
    pub fn frames(&self) -> usize { self.channels.first().map_or(0, |c| c.len()) }
}

pub struct BufferPlay {
    pub buffer: Option<Rc<Buffer>>,
    /// Where in the buffer, frames in Q32.
    pos: i64,
    pub looping: bool,
    pub loop_start: f64,
    pub loop_end: f64,
    pub offset: f64,
    pub duration: Option<f64>,
    end_pos: Option<i64>,
    began: bool,
}

pub struct Biquad {
    pub ty: Filter,
    /// b0 b1 b2 a1 a2, Q26.
    coef: [i64; 5],
    /// As floats, for `getFrequencyResponse`.
    pub float: [f64; 5],
    last: [i32; 4],
    /// x1 x2 y1 y2 per channel, Q21.
    state: Vec<[i64; 4]>,
}

pub struct Compressor { env_db: f32, gain: i32, pub reduction: f32 }

pub enum Kind {
    Destination,
    Gain,
    Oscillator { wave: Wave, phase: u32, table: Option<Rc<Vec<i32>>> },
    BufferSource(Box<BufferPlay>),
    Constant,
    Biquad(Box<Biquad>),
    Delay { lines: Vec<Vec<i32>>, at: usize, max: usize, quiet: usize },
    Panner,
    Splitter(usize),
    Merger(usize),
    Compressor(Box<Compressor>),
    Analyser(Box<Analyser>),
    /// The microphone or another context's stream.
    StreamSource { tap: u32, cursor: Option<u64> },
    StreamDestination { tap: u32 },
    /// A `<video>` or `<audio>` element: the element plays its own sound; this gives
    /// the graph nothing yet.
    ElementSource,
}

pub struct Node {
    pub kind: Kind,
    pub inputs: Vec<Vec<(usize, usize)>>,
    pub params: Vec<Param>,
    pub outputs: Vec<Bus>,
    pub count: usize,
    pub mode: Mode,
    pub discrete: bool,
    pub start: Option<u64>,
    pub stop: Option<u64>,
    pub ended: bool,
    /// The context's rate.
    pub rate: u32,
}

impl Node {
    pub fn new(kind: Kind, rate: u32) -> Node {
        let nyq = rate as f32 / 2.0;
        let p = Param::new;
        let (inputs, outputs, params, count, mode, discrete): (usize, Vec<usize>, Vec<Param>, usize, Mode, bool) = match &kind {
            Kind::Destination => (1, vec![2], vec![], 2, Mode::Explicit, false),
            Kind::Gain => (1, vec![1], vec![p(1.0, -BIG, BIG)], 2, Mode::Max, false),
            Kind::Oscillator { .. } => (0, vec![1], vec![p(440.0, -nyq, nyq), p(0.0, -153_600.0, 153_600.0)], 2, Mode::Max, false),
            Kind::BufferSource(_) => (0, vec![1], vec![p(1.0, -BIG, BIG), p(0.0, -BIG, BIG)], 2, Mode::Max, false),
            Kind::Constant => (0, vec![1], vec![p(1.0, -BIG, BIG)], 2, Mode::Max, false),
            Kind::Biquad(_) => (1, vec![1], vec![p(350.0, 0.0, nyq), p(0.0, -153_600.0, 153_600.0), p(1.0, -BIG, BIG), p(0.0, -BIG, 1541.0)], 2, Mode::Max, false),
            Kind::Delay { max, .. } => { let m = *max as f32 / rate as f32; (1, vec![1], vec![p(0.0, 0.0, m)], 2, Mode::Max, false) }
            Kind::Panner => (1, vec![2], vec![p(0.0, -1.0, 1.0)], 2, Mode::ClampedMax, false),
            Kind::Splitter(n) => (1, vec![1; *n], vec![], *n, Mode::Explicit, true),
            Kind::Merger(n) => (*n, vec![*n], vec![], 1, Mode::Explicit, false),
            Kind::Compressor(_) => (1, vec![1], vec![p(-24.0, -100.0, 0.0), p(30.0, 0.0, 40.0), p(12.0, 1.0, 20.0), p(0.003, 0.0, 1.0), p(0.25, 0.0, 1.0)], 2, Mode::ClampedMax, false),
            Kind::Analyser(_) => (1, vec![1], vec![], 2, Mode::Max, false),
            Kind::StreamSource { .. } | Kind::ElementSource => (0, vec![2], vec![], 2, Mode::Max, false),
            Kind::StreamDestination { .. } => (1, vec![], vec![], 2, Mode::Explicit, false),
        };
        let start = match kind { Kind::StreamSource { .. } | Kind::ElementSource => Some(0), _ => None };
        Node { kind, inputs: vec![Vec::new(); inputs], params, outputs: outputs.into_iter().map(Bus::new).collect(), count, mode, discrete, start, stop: None, ended: false, rate }
    }

    pub fn oscillator() -> Kind { Kind::Oscillator { wave: Wave::Sine, phase: 0, table: None } }
    pub fn buffer_source() -> Kind {
        Kind::BufferSource(Box::new(BufferPlay { buffer: None, pos: 0, looping: false, loop_start: 0.0, loop_end: 0.0, offset: 0.0, duration: None, end_pos: None, began: false }))
    }
    pub fn biquad() -> Kind { Kind::Biquad(Box::new(Biquad { ty: Filter::Lowpass, coef: [0; 5], float: [0.0; 5], last: [i32::MIN; 4], state: Vec::new() })) }
    pub fn delay(max_seconds: f64, rate: u32) -> Kind {
        let max = ((max_seconds.clamp(0.0, 180.0)) * rate as f64) as usize;
        Kind::Delay { lines: Vec::new(), at: 0, max: max.max(1), quiet: usize::MAX / 2 }
    }
    pub fn compressor() -> Kind { Kind::Compressor(Box::new(Compressor { env_db: -200.0, gain: 65536, reduction: 0.0 })) }
    pub fn analyser() -> Kind { Kind::Analyser(Box::new(Analyser::new())) }

    pub fn is_source(&self) -> bool { matches!(self.kind, Kind::Oscillator { .. } | Kind::BufferSource(_) | Kind::Constant) }

    /// The channels input `p` mixes to, given the channels each connected output has
    /// (None when silent).
    pub fn input_channels(&self, p: usize, has: impl Fn(usize, usize) -> Option<usize>) -> usize {
        if let Kind::Merger(_) = self.kind { return 1; }
        if self.mode == Mode::Explicit { return self.count; }
        let max = self.inputs[p].iter().filter_map(|&(j, o)| has(j, o)).max().unwrap_or(1);
        if self.mode == Mode::ClampedMax { max.min(self.count) } else { max }
    }

    /// The part of this quantum a source plays in, `[a, b)`; None when it does not.
    fn span(&self, frame: u64) -> Option<(usize, usize)> {
        let start = self.start?;
        if self.ended || start >= frame + QUANTUM as u64 { return None; }
        let a = start.saturating_sub(frame) as usize;
        let b = self.stop.map_or(QUANTUM, |s| (s.saturating_sub(frame) as usize).min(QUANTUM));
        (a < b).then_some((a, b))
    }

    /// Renders this node's quantum from its mixed inputs and param values. True
    /// when a source ended in it.
    pub fn process(&mut self, ins: &[Bus], v: &[Values; 5], frame: u64, t: &Tables, taps: &mut [Tap]) -> bool {
        let rate = self.rate;
        let span = self.span(frame);
        let mut ended_now = false;
        let stop_due = self.stop.is_some_and(|s| s <= frame + QUANTUM as u64);
        match &mut self.kind {
            Kind::Destination | Kind::Analyser(_) => {
                let input = &ins[0];
                if let Kind::Analyser(a) = &mut self.kind {
                    let mut m = [0; QUANTUM];
                    input.mono(&mut m);
                    a.push(&m);
                }
                let out = &mut self.outputs[0];
                if out.channels() != input.channels() && !matches!(self.kind, Kind::Destination) { out.reset(input.channels()); }
                out.reset(out.channels());
                out.mix(input, false);
            }
            Kind::StreamDestination { tap } => {
                let input = &ins[0];
                let mut f = [[0i16; 2]; QUANTUM];
                if !input.silent {
                    let r = input.channels().min(2) - 1;
                    for (k, o) in f.iter_mut().enumerate() { *o = [crate::clip(input.ch[0][k]), crate::clip(input.ch[r][k])]; }
                }
                if let Some(tp) = taps.iter_mut().find(|x| x.id == *tap) { tp.push(&f); }
            }
            Kind::Gain => {
                let input = &ins[0];
                let out = &mut self.outputs[0];
                out.reset(input.channels());
                if input.silent || (v[0].constant && v[0].at[0] == 0) { return false; }
                out.silent = false;
                for (o, i) in out.ch.iter_mut().zip(&input.ch) {
                    if v[0].constant && v[0].at[0] == 65536 { *o = *i; continue; }
                    for k in 0..QUANTUM { o[k] = ((i[k] as i64 * v[0].at[k] as i64) >> 16) as i32; }
                }
            }
            Kind::Oscillator { wave, phase, table } => {
                let out = &mut self.outputs[0];
                out.reset(1);
                if let Some((a, b)) = span {
                    out.silent = false;
                    let k_inc = (1i64 << 40) / rate as i64;
                    let nyq = (rate as i64 / 2) << 16;
                    let o = &mut out.ch[0];
                    let mut factor = detune_factor(v[1].at[0]);
                    for k in a..b {
                        if !v[1].constant && k % 16 == 0 { factor = detune_factor(v[1].at[k]); }
                        let f = ((v[0].at[k] as i64 * factor) >> 16).clamp(-nyq, nyq);
                        let inc = ((f * k_inc) >> 24) as i32 as u32;
                        let p = *phase;
                        o[k] = match wave {
                            Wave::Sine => t.sin(p),
                            Wave::Square => if p < 0x8000_0000 { 32767 } else { -32767 },
                            Wave::Sawtooth => ((p ^ 0x8000_0000) >> 16) as i32 - 32768,
                            Wave::Triangle => { let x = (p >> 15) as i32; if x < 65536 { x - 32768 } else { 98304 - x } }
                            Wave::Custom => match table { Some(tb) => lerp_table(tb, p), None => 0 },
                        };
                        *phase = p.wrapping_add(inc);
                    }
                }
                if stop_due && self.start.is_some() { ended_now = true; }
            }
            Kind::Constant => {
                let out = &mut self.outputs[0];
                out.reset(1);
                if let Some((a, b)) = span {
                    out.silent = false;
                    for k in a..b { out.ch[0][k] = v[0].at[k] >> 1; }
                }
                if stop_due && self.start.is_some() { ended_now = true; }
            }
            Kind::BufferSource(bp) => {
                let ch = bp.buffer.as_ref().map_or(1, |b| b.channels.len().max(1));
                let out = &mut self.outputs[0];
                out.reset(ch);
                if let Some((a, b)) = span {
                    if let Some(buf) = bp.buffer.clone().filter(|b| b.frames() > 0) {
                        let frames = buf.frames() as i64;
                        let br = buf.rate as f64;
                        if !bp.began {
                            bp.began = true;
                            bp.pos = ((bp.offset.max(0.0) * br) as i64).min(frames) << 32;
                            if let Some(d) = bp.duration.filter(|_| !bp.looping) { bp.end_pos = Some(bp.pos + (((d.max(0.0) * br) as i64) << 32)); }
                        }
                        let speed = v[0].at[0] as f64 / 65536.0 * libm::exp2(v[1].at[0] as f64 / 65536.0 / 1200.0) * br / rate as f64;
                        let step = (speed * 4_294_967_296.0) as i64;
                        let (mut ls, mut le) = (((bp.loop_start * br) as i64).clamp(0, frames), ((bp.loop_end * br) as i64).clamp(0, frames));
                        if !(bp.loop_end > 0.0 && ls < le) { ls = 0; le = frames; }
                        let (ls, le) = (ls << 32, le << 32);
                        let end = bp.end_pos.unwrap_or(frames << 32).min(frames << 32);
                        out.silent = false;
                        let mut done = false;
                        for k in a..b {
                            if bp.looping && le > ls {
                                if bp.pos >= le { bp.pos = ls + (bp.pos - le) % (le - ls); }
                                if bp.pos < ls && step < 0 { bp.pos = le - (ls - bp.pos) % (le - ls); }
                            } else if bp.pos >= end || bp.pos < 0 { done = true; break; }
                            let i = (bp.pos >> 32) as usize;
                            let f = ((bp.pos >> 16) & 0xFFFF) as i64;
                            for (c, o) in buf.channels.iter().zip(out.ch.iter_mut()) {
                                let x0 = c[i.min(c.len() - 1)] as i64;
                                let next = if i + 1 < c.len() { i + 1 } else if bp.looping { (ls >> 32) as usize } else { i };
                                let x1 = c[next.min(c.len() - 1)] as i64;
                                o[k] = (x0 + (((x1 - x0) * f) >> 16)) as i32;
                            }
                            bp.pos += step;
                        }
                        if done { ended_now = true; }
                    }
                }
                if stop_due && self.start.is_some() { ended_now = true; }
            }
            Kind::Biquad(bq) => {
                let input = &ins[0];
                let ch = input.channels();
                let out = &mut self.outputs[0];
                out.reset(ch);
                let p = [v[0].at[0], v[1].at[0], v[2].at[0], v[3].at[0]];
                if p != bq.last { bq.last = p; bq.set(rate, p); }
                if bq.state.len() != ch { bq.state.resize(ch, [0; 4]); }
                let resting = bq.state.iter().all(|s| s.iter().all(|x| x.abs() < 64));
                if input.silent && resting { for s in bq.state.iter_mut() { *s = [0; 4]; } return false; }
                out.silent = false;
                let [b0, b1, b2, a1, a2] = bq.coef;
                let lim = 1i64 << 40;
                for (c, o) in out.ch.iter_mut().enumerate() {
                    let s = &mut bq.state[c];
                    let i = &input.ch[c];
                    for k in 0..QUANTUM {
                        let x = if input.silent { 0 } else { (i[k] as i64) << 6 };
                        let y = ((b0 * x + b1 * s[0] + b2 * s[1] - a1 * s[2] - a2 * s[3]) >> 26).clamp(-lim, lim);
                        s[1] = s[0]; s[0] = x; s[3] = s[2]; s[2] = y;
                        o[k] = (y >> 6) as i32;
                    }
                }
            }
            Kind::Delay { lines, at, max, quiet } => {
                let input = &ins[0];
                let len = *max + QUANTUM + 2;
                let ch = if input.silent { lines.len().max(1) } else { input.channels() };
                if lines.len() != ch { lines.resize(ch, vec![0; len]); }
                if input.silent { *quiet += QUANTUM; } else { *quiet = 0; }
                let out = &mut self.outputs[0];
                out.reset(ch);
                if *quiet > len + QUANTUM { return false; }
                out.silent = false;
                let w0 = *at;
                for (c, line) in lines.iter_mut().enumerate() {
                    let o = &mut out.ch[c];
                    for k in 0..QUANTUM {
                        let w = (w0 + k) % len;
                        line[w] = if input.silent || c >= input.channels() { 0 } else { input.ch[c][k] };
                        // Delay in frames, Q16.
                        let d = (v[0].at[k].max(0) as i64 * rate as i64).min((*max as i64) << 16);
                        let back = (d >> 16) as usize;
                        let f = d & 0xFFFF;
                        let r0 = (w + len - back) % len;
                        let r1 = (r0 + len - 1) % len;
                        let (x0, x1) = (line[r0] as i64, line[r1] as i64);
                        o[k] = (x0 + (((x1 - x0) * f) >> 16)) as i32;
                    }
                }
                *at = (w0 + QUANTUM) % len;
            }
            Kind::Panner => {
                let input = &ins[0];
                let out = &mut self.outputs[0];
                out.reset(2);
                if input.silent { return false; }
                out.silent = false;
                let stereo = input.channels() >= 2;
                for k in 0..QUANTUM {
                    let pan = v[0].at[k].clamp(-65536, 65536) as i64;
                    let (l, r) = (input.ch[0][k] as i64, input.ch[if stereo { 1 } else { 0 }][k] as i64);
                    // Equal power: x in [0, 1] is a quarter turn.
                    let x = if !stereo { (pan + 65536) / 2 } else if pan <= 0 { pan + 65536 } else { pan };
                    let ph = (x as u64 * (1u64 << 30) / 65536) as u32;
                    let (gl, gr) = (t.sin(ph.wrapping_add(1 << 30)) as i64, t.sin(ph) as i64);
                    let (ol, or) = if !stereo { (l * gl >> 15, l * gr >> 15) } else if pan <= 0 { (l + (r * gl >> 15), r * gr >> 15) } else { (l * gl >> 15, r + (l * gr >> 15)) };
                    out.ch[0][k] = ol as i32;
                    out.ch[1][k] = or as i32;
                }
            }
            Kind::Splitter(n) => {
                let input = &ins[0];
                for (k, o) in self.outputs.iter_mut().enumerate().take(*n) {
                    o.reset(1);
                    if !input.silent && k < input.channels() { o.ch[0] = input.ch[k]; o.silent = o.ch[0].iter().all(|x| *x == 0); }
                }
            }
            Kind::Merger(n) => {
                let out = &mut self.outputs[0];
                out.reset(*n);
                for (k, i) in ins.iter().enumerate().take(*n) {
                    if i.silent { continue; }
                    out.silent = false;
                    out.ch[k] = i.ch[0];
                }
            }
            Kind::Compressor(cp) => {
                let input = &ins[0];
                let out = &mut self.outputs[0];
                out.reset(input.channels());
                let [th, knee, ratio, attack, release] = [0, 1, 2, 3, 4].map(|i| v[i].at[0] as f32 / 65536.0);
                let curve = |x: f32| -> f32 {
                    let r = ratio.max(1.0);
                    if x <= th - knee / 2.0 { x } else if knee > 0.0 && x < th + knee / 2.0 { let d = x - th + knee / 2.0; x + (1.0 / r - 1.0) * d * d / (2.0 * knee) } else { th + (x - th) / r }
                };
                let makeup = -0.6 * (curve(0.0) - 0.0);
                const CHUNK: usize = 32;
                let ca = libm::expf(-(CHUNK as f32) / (attack.max(0.0001) * rate as f32));
                let cr = libm::expf(-(CHUNK as f32) / (release.max(0.0001) * rate as f32));
                let silent = input.silent;
                if silent && cp.reduction > -0.01 { cp.reduction = 0.0; cp.env_db = -200.0; return false; }
                out.silent = silent;
                for c0 in (0..QUANTUM).step_by(CHUNK) {
                    let mut peak = 0i32;
                    if !silent { for ch in &input.ch { for s in &ch[c0..c0 + CHUNK] { peak = peak.max(s.abs()); } } }
                    let db = if peak == 0 { -200.0 } else { 20.0 * libm::log10f(peak as f32 / 32768.0) };
                    let want = curve(db) - db;
                    let coef = if want < cp.env_db { ca } else { cr };
                    cp.env_db = want + (cp.env_db - want) * coef;
                    if cp.env_db < -200.0 { cp.env_db = -200.0; }
                    cp.reduction = cp.env_db.min(0.0);
                    let g = (libm::powf(10.0, (cp.reduction + makeup) / 20.0) * 65536.0).min(1e9) as i32;
                    if !silent {
                        for (o, i) in out.ch.iter_mut().zip(&input.ch) {
                            for j in 0..CHUNK {
                                let gj = cp.gain as i64 + (g as i64 - cp.gain as i64) * j as i64 / CHUNK as i64;
                                o[c0 + j] = ((i[c0 + j] as i64 * gj) >> 16) as i32;
                            }
                        }
                    }
                    cp.gain = g;
                }
            }
            Kind::StreamSource { tap, cursor } => {
                let out = &mut self.outputs[0];
                out.reset(2);
                let Some(tp) = taps.iter().find(|x| x.id == *tap) else { return false };
                let cur = cursor.get_or_insert(tp.written);
                // Keep the delay short when the graph fell behind (or was suspended).
                if tp.written.saturating_sub(*cur) > 9600 { *cur = tp.written - 1920; }
                let mut f = [[0i16; 2]; QUANTUM];
                let n = tp.read(cur, &mut f);
                if n > 0 {
                    out.silent = false;
                    for (k, s) in f.iter().enumerate().take(n) { out.ch[0][k] = s[0] as i32; out.ch[1][k] = s[1] as i32; }
                }
            }
            Kind::ElementSource => { self.outputs[0].reset(2); }
        }
        if ended_now { self.ended = true; }
        ended_now
    }
}

/// 2^(cents / 1200) in Q16 from Q16 cents.
fn detune_factor(cents_q16: i32) -> i64 {
    if cents_q16 == 0 { return 65536; }
    (libm::exp2(cents_q16 as f64 / 65536.0 / 1200.0) * 65536.0) as i64
}

fn lerp_table(t: &[i32], p: u32) -> i32 {
    let i = (p >> 21) as usize;
    let f = ((p >> 5) & 0xFFFF) as i64;
    let (a, b) = (t[i] as i64, t[(i + 1) & 2047] as i64);
    (a + (((b - a) * f) >> 16)) as i32
}

/// A wavetable (2048 steps, Q15) from Fourier coefficients, as `createPeriodicWave`.
pub fn periodic_wave(real: &[f32], imag: &[f32], normalize: bool, t: &Tables) -> Vec<i32> {
    let n = real.len().max(imag.len()).min(2048);
    let mut acc = vec![0i64; 2048];
    for h in 1..n {
        let (a, b) = (real.get(h).copied().unwrap_or(0.0), imag.get(h).copied().unwrap_or(0.0));
        if a == 0.0 && b == 0.0 { continue; }
        let (qa, qb) = ((a * 32768.0) as i64, (b * 32768.0) as i64);
        for (i, s) in acc.iter_mut().enumerate() {
            let ph = ((i * h) % 2048) as u32 * (1 << 21);
            *s += qa * t.sin(ph.wrapping_add(1 << 30)) as i64 + qb * t.sin(ph) as i64;
        }
    }
    let peak = acc.iter().map(|x| x.abs()).max().unwrap_or(0).max(1);
    acc.iter().map(|&x| if normalize { (x * 32767 / peak) as i32 } else { (x >> 15).clamp(-(1 << 24), 1 << 24) as i32 }).collect()
}

impl Biquad {
    /// New coefficients from (frequency, detune, Q, gain) in Q16.
    pub fn set(&mut self, rate: u32, p: [i32; 4]) {
        let fs = rate as f64;
        let f0 = (p[0] as f64 / 65536.0 * libm::exp2(p[1] as f64 / 65536.0 / 1200.0)).clamp(1.0, fs / 2.0 - 1.0);
        let q = p[2] as f64 / 65536.0;
        let gain = p[3] as f64 / 65536.0;
        let a = libm::pow(10.0, gain / 40.0);
        let w0 = 2.0 * core::f64::consts::PI * f0 / fs;
        let (cw, sw) = (libm::cos(w0), libm::sin(w0));
        let aq = sw / (2.0 * q.max(1e-4));
        let aqdb = sw / (2.0 * libm::pow(10.0, q / 20.0));
        let as_ = sw / 2.0 * libm::sqrt(2.0);
        let sa = libm::sqrt(a);
        let (b0, b1, b2, a0, a1, a2) = match self.ty {
            Filter::Lowpass => ((1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0, 1.0 + aqdb, -2.0 * cw, 1.0 - aqdb),
            Filter::Highpass => ((1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0, 1.0 + aqdb, -2.0 * cw, 1.0 - aqdb),
            Filter::Bandpass => (aq, 0.0, -aq, 1.0 + aq, -2.0 * cw, 1.0 - aq),
            Filter::Notch => (1.0, -2.0 * cw, 1.0, 1.0 + aq, -2.0 * cw, 1.0 - aq),
            Filter::Allpass => (1.0 - aq, -2.0 * cw, 1.0 + aq, 1.0 + aq, -2.0 * cw, 1.0 - aq),
            Filter::Peaking => (1.0 + aq * a, -2.0 * cw, 1.0 - aq * a, 1.0 + aq / a, -2.0 * cw, 1.0 - aq / a),
            Filter::Lowshelf => (
                a * ((a + 1.0) - (a - 1.0) * cw + 2.0 * as_ * sa), 2.0 * a * ((a - 1.0) - (a + 1.0) * cw), a * ((a + 1.0) - (a - 1.0) * cw - 2.0 * as_ * sa),
                (a + 1.0) + (a - 1.0) * cw + 2.0 * as_ * sa, -2.0 * ((a - 1.0) + (a + 1.0) * cw), (a + 1.0) + (a - 1.0) * cw - 2.0 * as_ * sa),
            Filter::Highshelf => (
                a * ((a + 1.0) + (a - 1.0) * cw + 2.0 * as_ * sa), -2.0 * a * ((a - 1.0) + (a + 1.0) * cw), a * ((a + 1.0) + (a - 1.0) * cw - 2.0 * as_ * sa),
                (a + 1.0) - (a - 1.0) * cw + 2.0 * as_ * sa, 2.0 * ((a - 1.0) - (a + 1.0) * cw), (a + 1.0) - (a - 1.0) * cw - 2.0 * as_ * sa),
        };
        self.float = [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0];
        let lim = (1i64 << 26) as f64 * 1024.0;
        self.coef = self.float.map(|c| (c * (1u64 << 26) as f64).clamp(-lim, lim) as i64);
    }

    pub fn retype(&mut self, ty: Filter) { if self.ty != ty { self.ty = ty; self.last = [i32::MIN; 4]; } }

    /// Magnitude and phase at `hz` (`getFrequencyResponse`).
    pub fn response(&self, hz: f64, rate: u32) -> (f32, f32) {
        let [b0, b1, b2, a1, a2] = self.float;
        let w = 2.0 * core::f64::consts::PI * hz / rate as f64;
        if !(0.0..=core::f64::consts::PI).contains(&w) { return (f32::NAN, f32::NAN); }
        let (c1, s1, c2, s2) = (libm::cos(w), -libm::sin(w), libm::cos(2.0 * w), -libm::sin(2.0 * w));
        let (nr, ni) = (b0 + b1 * c1 + b2 * c2, b1 * s1 + b2 * s2);
        let (dr, di) = (1.0 + a1 * c1 + a2 * c2, a1 * s1 + a2 * s2);
        let d = dr * dr + di * di;
        let (hr, hi) = ((nr * dr + ni * di) / d, (ni * dr - nr * di) / d);
        (libm::sqrt(hr * hr + hi * hi) as f32, libm::atan2(hi, hr) as f32)
    }
}

/// Channel count settings a page may give (`channelCount` 1 to 32).
pub fn valid_count(n: usize) -> bool { (1..=MAX_CHANNELS).contains(&n) }

/// For the analyser in the API.
pub fn mono_of(b: &Bus) -> Block { let mut m = [0; QUANTUM]; b.mono(&mut m); m }
