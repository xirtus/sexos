// SPDX-License-Identifier: GPL-3.0-only
//! The Nonlinear Animation system (Blender's `anim_sys.cc` NLA part): tracks
//! of action strips stacked bottom to top, each strip mapping scene time
//! into its action (offset, scale, repeat, reverse), with blend-in/out,
//! influence, extrapolation (Hold, Hold Forward, Nothing) and blending
//! (Replace, Combine, Add, Subtract, Multiply). The object's active action
//! is evaluated last, on top of the tracks, with its own blend settings.
use super::curve::Action;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Replace,
    Combine,
    Add,
    Subtract,
    Multiply,
}

impl Blend {
    pub const ALL: [Blend; 5] = [Blend::Replace, Blend::Combine, Blend::Add, Blend::Subtract, Blend::Multiply];
    pub fn id(self) -> &'static str {
        match self {
            Blend::Replace => "REPLACE",
            Blend::Combine => "COMBINE",
            Blend::Add => "ADD",
            Blend::Subtract => "SUBTRACT",
            Blend::Multiply => "MULTIPLY",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Blend::Replace => "Replace",
            Blend::Combine => "Combine",
            Blend::Add => "Add",
            Blend::Subtract => "Subtract",
            Blend::Multiply => "Multiply",
        }
    }
    pub fn from_arg(s: &str) -> Option<Blend> {
        Blend::ALL.into_iter().find(|b| b.id() == s || b.label() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripExtrap {
    Hold,
    HoldForward,
    Nothing,
}

impl StripExtrap {
    pub const ALL: [StripExtrap; 3] = [StripExtrap::Hold, StripExtrap::HoldForward, StripExtrap::Nothing];
    pub fn id(self) -> &'static str {
        match self {
            StripExtrap::Hold => "HOLD",
            StripExtrap::HoldForward => "HOLD_FORWARD",
            StripExtrap::Nothing => "NOTHING",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            StripExtrap::Hold => "Hold",
            StripExtrap::HoldForward => "Hold Forward",
            StripExtrap::Nothing => "Nothing",
        }
    }
    pub fn from_arg(s: &str) -> Option<StripExtrap> {
        StripExtrap::ALL.into_iter().find(|b| b.id() == s || b.label() == s)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NlaStrip {
    pub name: String,
    /// Index into [`super::Anim::actions`].
    pub action: usize,
    pub frame_start: f32,
    pub frame_end: f32,
    pub action_start: f32,
    pub action_end: f32,
    pub scale: f32,
    pub repeat: f32,
    pub blend_in: f32,
    pub blend_out: f32,
    pub extrapolation: StripExtrap,
    pub blend: Blend,
    /// Used when `use_influence` (animated influence); otherwise blend-in/out ramps.
    pub influence: f32,
    pub use_influence: bool,
    pub mute: bool,
    pub reversed: bool,
    pub select: bool,
}

impl NlaStrip {
    pub fn new(name: &str, action: usize, act: &Action, at: f32) -> NlaStrip {
        let (a, b) = act.frame_range();
        let mut s = NlaStrip {
            name: name.into(),
            action,
            frame_start: at,
            frame_end: at,
            action_start: a,
            action_end: b,
            scale: 1.0,
            repeat: 1.0,
            blend_in: 0.0,
            blend_out: 0.0,
            extrapolation: StripExtrap::HoldForward,
            blend: Blend::Replace,
            influence: 1.0,
            use_influence: false,
            mute: false,
            reversed: false,
            select: false,
        };
        s.update_end();
        s
    }
    /// Recompute `frame_end` from the action range, scale and repeat.
    pub fn update_end(&mut self) {
        self.scale = if self.scale.abs() < 1e-4 { 1.0 } else { self.scale };
        self.repeat = self.repeat.max(0.01);
        self.frame_end = self.frame_start + (self.action_end - self.action_start).max(0.0) * self.scale * self.repeat;
    }
    /// Action time for scene frame `f` (Blender's `nlastrip_get_frame`).
    pub fn action_time(&self, f: f32) -> f32 {
        let len = (self.action_end - self.action_start).max(0.0);
        let mut t = (f - self.frame_start) / self.scale;
        if len > 0.0 {
            let total = len * self.repeat;
            t = t.clamp(0.0, total);
            if t >= total {
                t = len * (self.repeat - (self.repeat - 1e-6).floor());
                t = t.min(len);
            } else {
                t %= len;
            }
        } else {
            t = 0.0;
        }
        if self.reversed {
            self.action_end - t
        } else {
            self.action_start + t
        }
    }
    /// Scene frame for action time `t` (inverse of [`Self::action_time`] in
    /// the first repeat), for inserting keys while tweaking.
    pub fn scene_time(&self, t: f32) -> f32 {
        let local = if self.reversed { self.action_end - t } else { t - self.action_start };
        self.frame_start + local * self.scale
    }
    /// Influence at frame `f` from blend in/out (or the animated value).
    pub fn influence_at(&self, f: f32) -> f32 {
        if self.use_influence {
            return self.influence.clamp(0.0, 1.0);
        }
        let mut k = 1.0f32;
        if self.blend_in > 0.0 && f < self.frame_start + self.blend_in {
            k = k.min(((f - self.frame_start) / self.blend_in).clamp(0.0, 1.0));
        }
        if self.blend_out > 0.0 && f > self.frame_end - self.blend_out {
            k = k.min(((self.frame_end - f) / self.blend_out).clamp(0.0, 1.0));
        }
        k
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct NlaTrack {
    pub name: String,
    pub strips: Vec<NlaStrip>,
    pub mute: bool,
    pub solo: bool,
    pub lock: bool,
    pub select: bool,
}

impl NlaTrack {
    pub fn new(name: &str) -> NlaTrack {
        NlaTrack { name: name.into(), strips: Vec::new(), mute: false, solo: false, lock: false, select: false }
    }
    /// Does a strip fit at `[a, b]` without overlapping another one?
    pub fn has_space(&self, a: f32, b: f32) -> bool {
        self.strips.iter().all(|s| b <= s.frame_start + 1e-4 || a >= s.frame_end - 1e-4)
    }
    pub fn sort(&mut self) {
        self.strips.sort_by(|a, b| a.frame_start.total_cmp(&b.frame_start));
    }
    /// The strip that applies at frame `f` and the frame to evaluate it at:
    /// the strip under `f`, or a neighbour holding its end/start.
    pub fn strip_at(&self, f: f32) -> Option<(usize, f32)> {
        let n = self.strips.len();
        for (i, s) in self.strips.iter().enumerate() {
            if s.mute {
                continue;
            }
            if f >= s.frame_start && f <= s.frame_end {
                return Some((i, f));
            }
        }
        // Before the first strip: Hold extends it backwards.
        if let Some((i, s)) = self.strips.iter().enumerate().find(|(_, s)| !s.mute) {
            if f < s.frame_start {
                return (s.extrapolation == StripExtrap::Hold).then_some((i, s.frame_start));
            }
        }
        // After a strip: the last one that ended before f holds forward.
        let mut best: Option<usize> = None;
        for i in 0..n {
            let s = &self.strips[i];
            if !s.mute && s.frame_end < f {
                best = Some(i);
            }
        }
        let i = best?;
        let s = &self.strips[i];
        matches!(s.extrapolation, StripExtrap::Hold | StripExtrap::HoldForward).then_some((i, s.frame_end))
    }
}

/// Channel values being accumulated: (path, index) -> value.
pub type Channels = BTreeMap<(String, usize), f32>;

/// The value a channel starts from before any layer: 1 for scale and a
/// quaternion's W, 0 otherwise (Blender's property defaults).
pub fn default_value(path: &str, index: usize) -> f32 {
    let last = path.rsplit('.').next().unwrap_or(path);
    match last {
        "scale" => 1.0,
        "rotation_quaternion" if index == 0 => 1.0,
        "rotation_axis_angle" if index == 2 => 0.0,
        "slider_max" => 1.0,
        _ => 0.0,
    }
}

/// Blend one layer (`layer` channel values at `influence`) onto `acc`.
pub fn blend_layer(acc: &mut Channels, layer: &Channels, blend: Blend, influence: f32) {
    let k = influence.clamp(0.0, 1.0);
    // Quaternions combine as rotations.
    let mut done: Vec<(String, usize)> = Vec::new();
    if blend == Blend::Combine {
        let quats: Vec<String> = layer.keys().filter(|(p, i)| *i == 0 && p.ends_with("rotation_quaternion")).map(|(p, _)| p.clone()).collect();
        for p in quats {
            let get = |m: &Channels, i: usize| m.get(&(p.clone(), i)).copied();
            let l: Option<Vec<f32>> = (0..4).map(|i| get(layer, i)).collect();
            let Some(l) = l else { continue };
            let a: Vec<f32> = (0..4).map(|i| get(acc, i).unwrap_or_else(|| default_value(&p, i))).collect();
            let qa = sex_3d::Quat { w: a[0], x: a[1], y: a[2], z: a[3] }.normalize();
            let ql = sex_3d::Quat { w: l[0], x: l[1], y: l[2], z: l[3] }.normalize();
            let qr = sex_3d::Quat::IDENTITY.slerp(ql, k);
            let q = (qa * qr).normalize();
            for (i, v) in [q.w, q.x, q.y, q.z].into_iter().enumerate() {
                acc.insert((p.clone(), i), v);
                done.push((p.clone(), i));
            }
        }
    }
    for ((p, i), &v) in layer {
        if done.iter().any(|(dp, di)| dp == p && di == i) {
            continue;
        }
        let prev = acc.get(&(p.clone(), *i)).copied().unwrap_or_else(|| default_value(p, *i));
        let scale_like = p.ends_with("scale");
        let out = match blend {
            Blend::Replace => prev + (v - prev) * k,
            Blend::Add => prev + v * k,
            Blend::Subtract => prev - v * k,
            Blend::Multiply => prev * (1.0 + (v - 1.0) * k),
            Blend::Combine if scale_like => prev * (1.0 + (v - 1.0) * k),
            Blend::Combine => prev + (v - default_value(p, *i)) * k,
        };
        acc.insert((p.clone(), *i), out);
    }
}

/// The channel values of an action at action time `t`.
pub fn sample(act: &Action, t: f32) -> Channels {
    let mut out = Channels::new();
    for c in act.curves.iter().filter(|c| !c.mute && !c.keys.is_empty()) {
        out.insert((c.path.clone(), c.index), c.evaluate(t));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::curve::Interp;

    #[test]
    fn strip_time_mapping() {
        let mut act = Action::new("Act");
        let c = act.curve_mut("location", 0);
        c.insert(1.0, 0.0, Interp::Linear);
        c.insert(11.0, 10.0, Interp::Linear);
        let mut s = NlaStrip::new("Act", 0, &act, 21.0);
        assert_eq!(s.frame_end, 31.0);
        assert_eq!(s.action_time(26.0), 6.0);
        s.scale = 2.0;
        s.update_end();
        assert_eq!(s.frame_end, 41.0);
        assert_eq!(s.action_time(31.0), 6.0);
        s.scale = 1.0;
        s.repeat = 2.0;
        s.update_end();
        assert_eq!(s.frame_end, 41.0);
        assert!((s.action_time(36.0) - 6.0).abs() < 1e-4);
        assert!((s.action_time(41.0) - 11.0).abs() < 1e-4);
        s.blend_in = 5.0;
        assert!((s.influence_at(23.5) - 0.5).abs() < 1e-5);
        let mut t = NlaTrack::new("NlaTrack");
        t.strips.push(s);
        assert_eq!(t.strip_at(50.0), Some((0, 41.0)));
        assert_eq!(t.strip_at(0.0), None);
    }

    #[test]
    fn blending_modes() {
        let mut acc = Channels::new();
        let mut l = Channels::new();
        l.insert(("location".into(), 0), 4.0);
        blend_layer(&mut acc, &l, Blend::Replace, 0.5);
        assert_eq!(acc[&("location".to_string(), 0)], 2.0);
        blend_layer(&mut acc, &l, Blend::Add, 1.0);
        assert_eq!(acc[&("location".to_string(), 0)], 6.0);
        let mut s = Channels::new();
        s.insert(("scale".into(), 0), 2.0);
        blend_layer(&mut acc, &s, Blend::Combine, 1.0);
        blend_layer(&mut acc, &s, Blend::Combine, 1.0);
        assert_eq!(acc[&("scale".to_string(), 0)], 4.0);
    }
}
