// SPDX-License-Identifier: GPL-3.0-only
//! AudioParam: an intrinsic value and a timeline of automation events. Values are
//! worked out in floating point a few times per render quantum and handed to the
//! nodes as one Q16 integer per sample.
use alloc::vec::Vec;
use crate::{Block, QUANTUM};

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Set,
    Linear,
    Exp,
    /// Time constant (seconds).
    Target(f64),
    /// Values spread over a duration (seconds).
    Curve(Vec<f32>, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event { pub kind: Kind, pub time: f64, pub value: f32 }

#[derive(Clone, Debug)]
pub struct Param {
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub events: Vec<Event>,
    /// Inputs connected to the param: (node, output).
    pub inputs: Vec<(usize, usize)>,
}

/// One quantum of a param: Q16 per sample; `constant` when all are the same.
pub struct Values { pub at: Block, pub constant: bool }

impl Values {
    pub fn new() -> Values { Values { at: [0; QUANTUM], constant: true } }
}

impl Default for Values { fn default() -> Self { Self::new() } }

impl Param {
    pub fn new(value: f32, min: f32, max: f32) -> Param { Param { value, min, max, events: Vec::new(), inputs: Vec::new() } }

    /// Adds an event, kept in time order (one at the same time and of the same kind
    /// as another replaces it).
    pub fn push(&mut self, e: Event) {
        if !e.time.is_finite() || !e.value.is_finite() || e.time < 0.0 { return; }
        let same = |x: &Event| x.time == e.time && core::mem::discriminant(&x.kind) == core::mem::discriminant(&e.kind);
        if let Some(i) = self.events.iter().position(same) { self.events[i] = e; return; }
        let i = self.events.iter().position(|x| x.time > e.time).unwrap_or(self.events.len());
        self.events.insert(i, e);
        if self.events.len() > 4096 { self.events.remove(0); }
    }

    /// Forgets events at or after `t`.
    pub fn cancel(&mut self, t: f64) { self.events.retain(|e| e.time < t); }

    /// Forgets events at or after `t` and holds the value it has then.
    pub fn hold(&mut self, t: f64) {
        let v = self.at(t);
        self.cancel(t);
        self.push(Event { kind: Kind::Set, time: t, value: v });
    }

    /// Events that ended before `t` and no longer matter, folded into the value.
    pub fn settle(&mut self, t: f64) {
        while self.events.len() > 1 {
            let e = &self.events[1];
            let done = match e.kind { Kind::Set | Kind::Linear | Kind::Exp => e.time < t, _ => false };
            let first_done = matches!(self.events[0].kind, Kind::Set | Kind::Linear | Kind::Exp);
            if !(done && first_done) { break; }
            self.events.remove(0);
        }
    }

    /// The value at time `t` (seconds), clamped to the param's range.
    pub fn at(&self, t: f64) -> f32 {
        let v = self.raw(t);
        if v.is_nan() { return self.value; }
        v.clamp(self.min, self.max)
    }

    fn raw(&self, t: f64) -> f32 {
        let (mut pv, mut pt) = (self.value, 0.0f64);
        let ev = &self.events;
        for (i, e) in ev.iter().enumerate() {
            match &e.kind {
                Kind::Set => { if t < e.time { return pv; } pv = e.value; pt = e.time; }
                Kind::Linear | Kind::Exp => {
                    if t < e.time {
                        if t < pt { return pv; }
                        let f = ((t - pt) / (e.time - pt).max(1e-12)) as f32;
                        return if e.kind == Kind::Linear { pv + (e.value - pv) * f }
                            else if pv == 0.0 || e.value == 0.0 || (pv < 0.0) != (e.value < 0.0) { pv }
                            else { pv * libm::powf(e.value / pv, f) };
                    }
                    pv = e.value; pt = e.time;
                }
                Kind::Target(tau) => {
                    if t < e.time { return pv; }
                    let from = pv;
                    let target = |at: f64| if *tau <= 0.0 { e.value } else { e.value + (from - e.value) * libm::exp(-(at - e.time) / *tau) as f32 };
                    match ev.get(i + 1) {
                        Some(n) if t >= n.time => { pv = target(n.time); pt = n.time; }
                        _ => return target(t),
                    }
                }
                Kind::Curve(values, dur) => {
                    if t < e.time || values.is_empty() { if values.is_empty() { continue; } return pv; }
                    let end = e.time + dur;
                    if t < end && *dur > 0.0 {
                        let pos = (t - e.time) / dur * (values.len() - 1) as f64;
                        let k = (pos as usize).min(values.len() - 1);
                        let f = (pos - k as f64) as f32;
                        let a = values[k];
                        let b = values.get(k + 1).copied().unwrap_or(a);
                        return a + (b - a) * f;
                    }
                    pv = values[values.len() - 1]; pt = end;
                }
            }
        }
        pv
    }

    /// Whether the value can still change at or after `t`.
    fn moving(&self, t: f64) -> bool {
        match self.events.last() {
            None => false,
            Some(e) => match e.kind { Kind::Target(_) => true, Kind::Curve(_, d) => e.time + d >= t, _ => e.time >= t },
        }
    }

    /// The quantum from frame `frame` at `rate`, in Q16, without inputs.
    pub fn render(&self, frame: u64, rate: u32, out: &mut Values) {
        let t0 = frame as f64 / rate as f64;
        if !self.moving(t0) {
            let v = q16(self.at(t0));
            out.at = [v; QUANTUM];
            out.constant = true;
            return;
        }
        // Exact every 16 frames, linear between: a set lands within a third of a ms.
        const STEP: usize = 16;
        let mut prev = q16(self.at(t0));
        let first = prev;
        let mut all = true;
        for k in 0..QUANTUM / STEP {
            let next = q16(self.at((frame + ((k + 1) * STEP) as u64) as f64 / rate as f64));
            all &= next == first;
            // A set (a jump) lands on its own step rather than sliding into it.
            let jump = self.events.iter().any(|e| e.kind == Kind::Set && {
                let f = e.time * rate as f64; f > (frame + (k * STEP) as u64) as f64 && f <= (frame + ((k + 1) * STEP) as u64) as f64 });
            for j in 0..STEP {
                out.at[k * STEP + j] = if jump { self.q_at(frame + (k * STEP + j) as u64, rate) }
                    else { (prev as i64 + (next as i64 - prev as i64) * j as i64 / STEP as i64) as i32 };
            }
            prev = next;
        }
        out.constant = all;
    }

    fn q_at(&self, frame: u64, rate: u32) -> i32 { q16(self.at(frame as f64 / rate as f64)) }
}

/// A float as Q16, saturating.
pub fn q16(v: f32) -> i32 {
    let x = v * 65536.0;
    if x >= i32::MAX as f32 { i32::MAX } else if x <= i32::MIN as f32 { i32::MIN } else { x as i32 }
}
