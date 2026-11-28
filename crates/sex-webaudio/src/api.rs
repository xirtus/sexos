// SPDX-License-Identifier: GPL-3.0-only
//! What page script may ask (sex-js `op::AUDIO` and `op::AUDIO_DATA`), as text verbs.
//! Every context belongs to the page that made it: another page's ids find nothing.
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use crate::node::{self, Buffer, Filter, Kind, Mode, Node, Wave};
use crate::param::{Event, Kind as Ev};
use crate::{Context, Hub, State, MAX_CHANNELS, MIC};

fn num(s: Option<&&str>) -> Option<f64> { s.and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite()) }
fn idx(s: Option<&&str>) -> Option<usize> { s.and_then(|v| v.trim().parse().ok()) }

fn floats(b: &[u8]) -> Vec<f32> { b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect() }
fn put(out: &mut Vec<u8>, v: f32) { out.extend_from_slice(&v.to_le_bytes()); }

/// Frame for a time in seconds, not before now.
fn frame_at(c: &Context, when: f64) -> u64 {
    if !(when > 0.0) { return c.frame; }
    ((when * c.rate as f64) as u64).max(c.frame)
}

impl Hub {
    /// A text verb from page `owner` (see sex-js `op::AUDIO`). `alive` lives as long
    /// as the page does.
    pub fn call(&mut self, owner: u64, alive: &Rc<()>, a: &[&str]) -> Option<String> {
        let verb = *a.first()?;
        if verb == "new" {
            let offline = a.get(1).is_some_and(|v| *v == "1");
            let spec = if offline { Some((idx(a.get(2))?, num(a.get(3))? as u64, num(a.get(4))? as u32)) } else { None };
            let id = self.create(owner, Rc::downgrade(alive), spec)?;
            let rate = self.context(owner, id)?.rate;
            return Some(format!("{id}\t{rate}"));
        }
        let id: u32 = a.get(1)?.parse().ok()?;
        let Hub { contexts, taps, .. } = self;
        let c = contexts.iter_mut().find(|c| c.id == id && c.owner == owner)?;
        match verb {
            "time" => Some(format!("{}", c.time())),
            "state" => {
                match *a.get(2)? {
                    "suspend" if c.state == State::Running && c.offline.is_none() => c.state = State::Suspended,
                    "resume" if c.state == State::Suspended && c.offline.is_none() => c.state = State::Running,
                    "close" => c.state = State::Closed,
                    _ => {}
                }
                Some(String::from(c.state.name()))
            }
            "node" => {
                let rate = c.rate;
                let kind = match *a.get(2)? {
                    "gain" => Kind::Gain,
                    "oscillator" => Node::oscillator(),
                    "buffersource" => Node::buffer_source(),
                    "constant" => Kind::Constant,
                    "biquad" => Node::biquad(),
                    "delay" => Node::delay(num(a.get(3)).unwrap_or(1.0), rate),
                    "panner" => Kind::Panner,
                    "splitter" => Kind::Splitter(idx(a.get(3)).unwrap_or(6).clamp(1, MAX_CHANNELS)),
                    "merger" => Kind::Merger(idx(a.get(3)).unwrap_or(6).clamp(1, MAX_CHANNELS)),
                    "compressor" => Node::compressor(),
                    "analyser" => Node::analyser(),
                    "streamsource" => Kind::StreamSource { tap: a.get(3).and_then(|t| t.parse().ok()).unwrap_or(MIC), cursor: None },
                    "streamdestination" => Kind::StreamDestination { tap: 0 },
                    "elementsource" => Kind::ElementSource,
                    _ => return None,
                };
                if c.nodes.len() >= 100_000 { return None; }
                let n = c.add(kind, taps);
                Some(match c.nodes[n].kind { Kind::StreamDestination { tap } => format!("{n}\t{tap}"), _ => format!("{n}") })
            }
            "connect" | "connectparam" => {
                let (from, out, to, port) = (idx(a.get(2))?, idx(a.get(3))?, idx(a.get(4))?, idx(a.get(5))?);
                let ok = if verb == "connect" { c.connect(from, out, to, port) } else { c.connect_param(from, out, to, port) };
                Some(String::from(if ok { "1" } else { "0" }))
            }
            "disconnect" => {
                c.disconnect(idx(a.get(2))?, idx(a.get(3)), idx(a.get(4)), idx(a.get(5)), idx(a.get(6)));
                None
            }
            "param" => {
                let now = c.time();
                let p = c.nodes.get_mut(idx(a.get(2))?)?.params.get_mut(idx(a.get(3))?)?;
                let v = num(a.get(5));
                let t = num(a.get(6)).unwrap_or(0.0).max(0.0);
                match *a.get(4)? {
                    "value" => {
                        let v = v? as f32;
                        p.value = v;
                        if !p.events.is_empty() { p.push(Event { kind: Ev::Set, time: now, value: v }); }
                    }
                    "set" => p.push(Event { kind: Ev::Set, time: t, value: v? as f32 }),
                    "linear" => p.push(Event { kind: Ev::Linear, time: t, value: v? as f32 }),
                    "exp" => { if v? == 0.0 { return None; } p.push(Event { kind: Ev::Exp, time: t, value: v? as f32 }) }
                    "target" => p.push(Event { kind: Ev::Target(num(a.get(7))?.max(0.0)), time: t, value: v? as f32 }),
                    "curve" => {
                        let values: Vec<f32> = a.get(7)?.split(',').filter_map(|x| x.parse().ok()).collect();
                        if values.len() < 2 { return None; }
                        p.push(Event { kind: Ev::Curve(values, num(a.get(8))?.max(0.0)), time: t, value: 0.0 })
                    }
                    "cancel" => p.cancel(num(a.get(5)).unwrap_or(0.0)),
                    "hold" => p.hold(num(a.get(5)).unwrap_or(now)),
                    _ => return None,
                }
                p.settle(now);
                Some(String::from("1"))
            }
            "get" => {
                let now = c.time();
                let p = c.nodes.get(idx(a.get(2))?)?.params.get(idx(a.get(3))?)?;
                Some(format!("{}", p.at(now)))
            }
            "set" => {
                let rate = c.rate;
                let n = c.nodes.get_mut(idx(a.get(2))?)?;
                let val = *a.get(4)?;
                let ok = match (*a.get(3)?, &mut n.kind) {
                    ("type", Kind::Oscillator { wave, .. }) => {
                        *wave = match val { "sine" => Wave::Sine, "square" => Wave::Square, "sawtooth" => Wave::Sawtooth, "triangle" => Wave::Triangle, _ => return None };
                        true
                    }
                    ("type", Kind::Biquad(b)) => { b.retype(Filter::parse(val)?); true }
                    ("fftSize", Kind::Analyser(an)) => an.resize(val.parse().ok()?),
                    ("smoothing", Kind::Analyser(an)) => { let v = val.parse::<f64>().ok()?; if !(0.0..=1.0).contains(&v) { return Some(String::from("0")); } an.smoothing = v; true }
                    ("minDb", Kind::Analyser(an)) => { let v = val.parse::<f64>().ok()?; if v >= an.max_db { return Some(String::from("0")); } an.min_db = v; true }
                    ("maxDb", Kind::Analyser(an)) => { let v = val.parse::<f64>().ok()?; if v <= an.min_db { return Some(String::from("0")); } an.max_db = v; true }
                    ("loop", Kind::BufferSource(b)) => { b.looping = val == "1"; true }
                    ("loopStart", Kind::BufferSource(b)) => { b.loop_start = val.parse().ok()?; true }
                    ("loopEnd", Kind::BufferSource(b)) => { b.loop_end = val.parse().ok()?; true }
                    ("count", _) => { let v: usize = val.parse().ok()?; if !node::valid_count(v) { return Some(String::from("0")); } n.count = v; true }
                    ("mode", _) => { n.mode = match val { "max" => Mode::Max, "clamped-max" => Mode::ClampedMax, "explicit" => Mode::Explicit, _ => return None }; true }
                    ("interp", _) => { n.discrete = val == "discrete"; true }
                    _ => false,
                };
                let _ = rate;
                Some(String::from(if ok { "1" } else { "0" }))
            }
            "start" => {
                let f = frame_at(c, num(a.get(3)).unwrap_or(0.0));
                let n = c.nodes.get_mut(idx(a.get(2))?)?;
                if !n.is_source() || n.start.is_some() { return Some(String::from("0")); }
                n.start = Some(f);
                if let Kind::BufferSource(b) = &mut n.kind {
                    b.offset = num(a.get(4)).unwrap_or(0.0).max(0.0);
                    b.duration = num(a.get(5)).filter(|d| *d >= 0.0);
                    if let Some(d) = b.duration.filter(|_| b.looping) { n.stop = Some(f + (d * c.rate as f64) as u64); }
                }
                Some(String::from("1"))
            }
            "stop" => {
                let f = frame_at(c, num(a.get(3)).unwrap_or(0.0));
                let n = c.nodes.get_mut(idx(a.get(2))?)?;
                if !n.is_source() || n.start.is_none() { return Some(String::from("0")); }
                n.stop = Some(n.stop.map_or(f, |s| s.min(f)).max(n.start.unwrap_or(0)));
                Some(String::from("1"))
            }
            "reduction" => match &c.nodes.get(idx(a.get(2))?)?.kind { Kind::Compressor(cp) => Some(format!("{}", cp.reduction)), _ => None },
            "count" => Some(format!("{}", c.nodes.len())),
            _ => None,
        }
    }

    /// A byte verb from page `owner` (see sex-js `op::AUDIO_DATA`).
    pub fn call_bytes(&mut self, owner: u64, a: &[&[u8]]) -> Option<Vec<u8>> {
        let text = |i: usize| a.get(i).and_then(|b| core::str::from_utf8(b).ok()).unwrap_or("");
        let verb = text(0);
        if verb == "decode" {
            let rate: u32 = text(1).parse().ok().filter(|r| (3000..=768_000).contains(r))?;
            let d = (self.decoder?)(a.get(2)?)?;
            return Some(decoded(&d, rate));
        }
        let id: u32 = text(1).parse().ok()?;
        if verb == "render" { return self.render_offline(owner, id); }
        let tables = self.tables.clone();
        let c = self.context(owner, id)?;
        let (rate, now) = (c.rate, c.frame);
        let n = c.nodes.get_mut(text(2).parse::<usize>().ok()?)?;
        match verb {
            "analyser" => {
                let Kind::Analyser(an) = &mut n.kind else { return None };
                let mut out = Vec::new();
                match text(3) {
                    "bf" => { an.update(now); let mut b = vec![0u8; an.bins()]; an.byte_frequency(&mut b); out = b; }
                    "ff" => { an.update(now); let mut f = vec![0f32; an.bins()]; an.float_frequency(&mut f); for v in f { put(&mut out, v); } }
                    "bt" => { let mut b = vec![0u8; an.size]; an.byte_time(&mut b); out = b; }
                    "ft" => { let mut f = vec![0f32; an.size]; an.float_time(&mut f); for v in f { put(&mut out, v); } }
                    _ => return None,
                }
                Some(out)
            }
            "buffer" => {
                let Kind::BufferSource(b) = &mut n.kind else { return None };
                let channels: usize = text(3).parse().ok().filter(|c| (1..=MAX_CHANNELS).contains(c))?;
                let brate: f64 = text(4).parse().ok().filter(|r: &f64| *r >= 1.0)?;
                let all = floats(a.get(5)?);
                let frames = all.len() / channels;
                let planes = (0..channels).map(|k| all[k * frames..(k + 1) * frames].iter().map(|v| {
                    let x = v * 32768.0; if x.is_nan() { 0 } else { x.clamp(-(1 << 24) as f32, (1 << 24) as f32) as i32 }
                }).collect()).collect();
                b.buffer = Some(Rc::new(Buffer { rate: brate as u32, channels: planes }));
                Some(Vec::new())
            }
            "wave" => {
                let Kind::Oscillator { wave, table, .. } = &mut n.kind else { return None };
                let (re, im) = (floats(a.get(3)?), floats(a.get(4)?));
                *table = Some(Rc::new(node::periodic_wave(&re, &im, text(5) != "0", &tables)));
                *wave = Wave::Custom;
                Some(Vec::new())
            }
            "response" => {
                let t = now as f64 / rate as f64;
                let p: Vec<i32> = n.params.iter().map(|p| crate::param::q16(p.at(t))).collect();
                let Kind::Biquad(bq) = &mut n.kind else { return None };
                bq.set(rate, [p[0], p[1], p[2], p[3]]);
                let hz = floats(a.get(3)?);
                let mut out = Vec::with_capacity(hz.len() * 8);
                let r: Vec<(f32, f32)> = hz.iter().map(|h| bq.response(*h as f64, rate)).collect();
                for (m, _) in &r { put(&mut out, *m); }
                for (_, p) in &r { put(&mut out, *p); }
                Some(out)
            }
            _ => None,
        }
    }
}

/// `decodeAudioData`'s answer: channels, rate, frames (u32 each), then planar f32,
/// resampled to the context's rate.
fn decoded(d: &crate::Decoded, rate: u32) -> Vec<u8> {
    let ch = if d.channels == 1 { 1 } else { 2 };
    let src = &d.frames;
    let frames = if d.rate == rate || src.is_empty() { src.len() } else { (src.len() as u64 * rate as u64 / d.rate.max(1) as u64) as usize };
    let mut out = Vec::with_capacity(12 + ch * frames * 4);
    out.extend_from_slice(&(ch as u32).to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(frames as u32).to_le_bytes());
    // Position in the source, Q16 frames per output frame.
    let step = ((d.rate as u64) << 16) / rate.max(1) as u64;
    for k in 0..ch {
        for i in 0..frames {
            let p = i as u64 * step;
            let j = (p >> 16) as usize;
            let f = (p & 0xFFFF) as i32;
            let a = src[j.min(src.len() - 1)][k] as i32;
            let b = src[(j + 1).min(src.len() - 1)][k] as i32;
            put(&mut out, (a + (((b - a) * f) >> 16)) as f32 / 32768.0);
        }
    }
    out
}
