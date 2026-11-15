// SPDX-License-Identifier: GPL-3.0-only
//! F-curves the way Blender keeps them (`fcurve.cc`): keyframes with
//! per-segment interpolation (Constant, Linear, Bezier and Robert Penner's
//! easing equations), handle types (Free, Aligned, Vector, Automatic,
//! Auto Clamped), keyframe types, constant or linear extrapolation, and the
//! Cycles and Noise F-modifiers.
use super::path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Constant,
    Linear,
    Bezier,
    Sine,
    Quad,
    Cubic,
    Quart,
    Quint,
    Expo,
    Circ,
    Back,
    Bounce,
    Elastic,
}

impl Interp {
    pub const ALL: [Interp; 13] = [
        Interp::Constant,
        Interp::Linear,
        Interp::Bezier,
        Interp::Sine,
        Interp::Quad,
        Interp::Cubic,
        Interp::Quart,
        Interp::Quint,
        Interp::Expo,
        Interp::Circ,
        Interp::Back,
        Interp::Bounce,
        Interp::Elastic,
    ];
    /// Blender's identifiers (`CONSTANT`, `LINEAR`, `BEZIER`, `SINE`, ...).
    pub fn id(self) -> &'static str {
        match self {
            Interp::Constant => "CONSTANT",
            Interp::Linear => "LINEAR",
            Interp::Bezier => "BEZIER",
            Interp::Sine => "SINE",
            Interp::Quad => "QUAD",
            Interp::Cubic => "CUBIC",
            Interp::Quart => "QUART",
            Interp::Quint => "QUINT",
            Interp::Expo => "EXPO",
            Interp::Circ => "CIRC",
            Interp::Back => "BACK",
            Interp::Bounce => "BOUNCE",
            Interp::Elastic => "ELASTIC",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Interp::Constant => "Constant",
            Interp::Linear => "Linear",
            Interp::Bezier => "Bezier",
            Interp::Sine => "Sinusoidal",
            Interp::Quad => "Quadratic",
            Interp::Cubic => "Cubic",
            Interp::Quart => "Quartic",
            Interp::Quint => "Quintic",
            Interp::Expo => "Exponential",
            Interp::Circ => "Circular",
            Interp::Back => "Back",
            Interp::Bounce => "Bounce",
            Interp::Elastic => "Elastic",
        }
    }
    pub fn from_arg(s: &str) -> Option<Interp> {
        Interp::ALL.into_iter().find(|i| i.id() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Easing {
    Auto,
    In,
    Out,
    InOut,
}

impl Easing {
    pub const ALL: [Easing; 4] = [Easing::Auto, Easing::In, Easing::Out, Easing::InOut];
    pub fn id(self) -> &'static str {
        match self {
            Easing::Auto => "AUTO",
            Easing::In => "EASE_IN",
            Easing::Out => "EASE_OUT",
            Easing::InOut => "EASE_IN_OUT",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Easing::Auto => "Automatic Easing",
            Easing::In => "Ease In",
            Easing::Out => "Ease Out",
            Easing::InOut => "Ease In and Out",
        }
    }
    pub fn from_arg(s: &str) -> Option<Easing> {
        Easing::ALL.into_iter().find(|e| e.id() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleType {
    Free,
    Aligned,
    Vector,
    Auto,
    AutoClamped,
}

impl HandleType {
    pub const ALL: [HandleType; 5] = [HandleType::Free, HandleType::Vector, HandleType::Aligned, HandleType::Auto, HandleType::AutoClamped];
    pub fn id(self) -> &'static str {
        match self {
            HandleType::Free => "FREE",
            HandleType::Aligned => "ALIGNED",
            HandleType::Vector => "VECTOR",
            HandleType::Auto => "AUTO",
            HandleType::AutoClamped => "AUTO_CLAMPED",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            HandleType::Free => "Free",
            HandleType::Aligned => "Aligned",
            HandleType::Vector => "Vector",
            HandleType::Auto => "Automatic",
            HandleType::AutoClamped => "Auto Clamped",
        }
    }
    pub fn from_arg(s: &str) -> Option<HandleType> {
        HandleType::ALL.into_iter().find(|h| h.id() == s)
    }
    fn is_auto(self) -> bool {
        matches!(self, HandleType::Auto | HandleType::AutoClamped)
    }
}

/// Blender's keyframe types (R in the Dope Sheet), drawn with their colours
/// and sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyType {
    Keyframe,
    Breakdown,
    MovingHold,
    Extreme,
    Jitter,
    Generated,
}

impl KeyType {
    pub const ALL: [KeyType; 6] = [KeyType::Keyframe, KeyType::Breakdown, KeyType::MovingHold, KeyType::Extreme, KeyType::Jitter, KeyType::Generated];
    pub fn id(self) -> &'static str {
        match self {
            KeyType::Keyframe => "KEYFRAME",
            KeyType::Breakdown => "BREAKDOWN",
            KeyType::MovingHold => "MOVING_HOLD",
            KeyType::Extreme => "EXTREME",
            KeyType::Jitter => "JITTER",
            KeyType::Generated => "GENERATED",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            KeyType::Keyframe => "Keyframe",
            KeyType::Breakdown => "Breakdown",
            KeyType::MovingHold => "Moving Hold",
            KeyType::Extreme => "Extreme",
            KeyType::Jitter => "Jitter",
            KeyType::Generated => "Generated",
        }
    }
    pub fn from_arg(s: &str) -> Option<KeyType> {
        KeyType::ALL.into_iter().find(|k| k.id() == s)
    }
    /// (fill, selected fill) as Blender's theme draws the diamond.
    pub fn colors(self) -> (u32, u32) {
        match self {
            KeyType::Keyframe => (0xe8e8e8, 0xffbe33),
            KeyType::Breakdown => (0x54bfed, 0x84e6ff),
            KeyType::MovingHold => (0x5c5656, 0xffaf23),
            KeyType::Extreme => (0xe8b3cc, 0xf28080),
            KeyType::Jitter => (0x94e575, 0x61c042),
            KeyType::Generated => (0x585858, 0xa28962),
        }
    }
    /// Diamond size relative to a regular keyframe.
    pub fn scale(self) -> f32 {
        match self {
            KeyType::Breakdown | KeyType::Jitter | KeyType::Generated => 0.85,
            KeyType::MovingHold => 0.8,
            _ => 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extrapolation {
    Constant,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CycleMode {
    None,
    Repeat,
    RepeatOffset,
    RepeatMirror,
}

impl CycleMode {
    pub const ALL: [CycleMode; 4] = [CycleMode::None, CycleMode::Repeat, CycleMode::RepeatOffset, CycleMode::RepeatMirror];
    pub fn label(self) -> &'static str {
        match self {
            CycleMode::None => "No Cycles",
            CycleMode::Repeat => "Repeat Motion",
            CycleMode::RepeatOffset => "Repeat with Offset",
            CycleMode::RepeatMirror => "Repeat Mirrored",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            CycleMode::None => "NONE",
            CycleMode::Repeat => "REPEAT",
            CycleMode::RepeatOffset => "REPEAT_OFFSET",
            CycleMode::RepeatMirror => "MIRROR",
        }
    }
    pub fn from_arg(s: &str) -> Option<CycleMode> {
        CycleMode::ALL.into_iter().find(|c| c.id() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseBlend {
    Replace,
    Add,
    Subtract,
    Multiply,
}

impl NoiseBlend {
    pub const ALL: [NoiseBlend; 4] = [NoiseBlend::Replace, NoiseBlend::Add, NoiseBlend::Subtract, NoiseBlend::Multiply];
    pub fn id(self) -> &'static str {
        match self {
            NoiseBlend::Replace => "REPLACE",
            NoiseBlend::Add => "ADD",
            NoiseBlend::Subtract => "SUBTRACT",
            NoiseBlend::Multiply => "MULTIPLY",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            NoiseBlend::Replace => "Replace",
            NoiseBlend::Add => "Add",
            NoiseBlend::Subtract => "Subtract",
            NoiseBlend::Multiply => "Multiply",
        }
    }
    pub fn from_arg(s: &str) -> Option<NoiseBlend> {
        NoiseBlend::ALL.into_iter().find(|c| c.id() == s)
    }
}

/// F-curve modifiers (the Graph Editor's Modifiers tab).
#[derive(Clone, Debug, PartialEq)]
pub enum FModifier {
    Cycles { before: CycleMode, after: CycleMode, mute: bool },
    Noise { blend: NoiseBlend, scale: f32, strength: f32, phase: f32, offset: f32, depth: u32, mute: bool },
}

impl FModifier {
    pub fn cycles() -> FModifier {
        FModifier::Cycles { before: CycleMode::Repeat, after: CycleMode::Repeat, mute: false }
    }
    pub fn noise() -> FModifier {
        FModifier::Noise { blend: NoiseBlend::Replace, scale: 1.0, strength: 1.0, phase: 1.0, offset: 0.0, depth: 0, mute: false }
    }
    pub fn name(&self) -> &'static str {
        match self {
            FModifier::Cycles { .. } => "Cycles",
            FModifier::Noise { .. } => "Noise",
        }
    }
    pub fn muted(&self) -> bool {
        match self {
            FModifier::Cycles { mute, .. } | FModifier::Noise { mute, .. } => *mute,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    pub frame: f32,
    pub value: f32,
    /// Interpolation towards the next key.
    pub interp: Interp,
    pub easing: Easing,
    /// Handles as absolute (frame, value) points.
    pub left: [f32; 2],
    pub right: [f32; 2],
    pub h1: HandleType,
    pub h2: HandleType,
    pub key_type: KeyType,
    /// The key (its centre) is selected.
    pub select: bool,
    pub select_left: bool,
    pub select_right: bool,
}

impl Keyframe {
    pub fn new(frame: f32, value: f32, interp: Interp) -> Keyframe {
        Keyframe {
            frame,
            value,
            interp,
            easing: Easing::Auto,
            left: [frame - 1.0, value],
            right: [frame + 1.0, value],
            h1: HandleType::AutoClamped,
            h2: HandleType::AutoClamped,
            key_type: KeyType::Keyframe,
            select: false,
            select_left: false,
            select_right: false,
        }
    }
    /// Move the key and its handles by (df, dv).
    pub fn shift(&mut self, df: f32, dv: f32) {
        self.frame += df;
        self.value += dv;
        self.left = [self.left[0] + df, self.left[1] + dv];
        self.right = [self.right[0] + df, self.right[1] + dv];
    }
    pub fn any_selected(&self) -> bool {
        self.select || self.select_left || self.select_right
    }
}

/// One animated channel: `path[index]`.
#[derive(Clone, Debug, PartialEq)]
pub struct FCurve {
    pub path: String,
    pub index: usize,
    /// Channel group ("Object Transforms", a bone name, "Key").
    pub group: String,
    /// Sorted by frame, at most one key per frame.
    pub keys: Vec<Keyframe>,
    pub select: bool,
    pub mute: bool,
    pub hide: bool,
    pub lock: bool,
    pub extrapolation: Extrapolation,
    pub modifiers: Vec<FModifier>,
}

impl FCurve {
    pub fn new(path: &str, index: usize) -> FCurve {
        FCurve {
            path: path.into(),
            index,
            group: path::default_group(path),
            keys: Vec::new(),
            select: false,
            mute: false,
            hide: false,
            lock: false,
            extrapolation: Extrapolation::Constant,
            modifiers: Vec::new(),
        }
    }
    /// Blender's channel name: "X Location", "W Quaternion Rotation (Bone)".
    pub fn label(&self) -> String {
        let len = match path::parse(&self.path).last() {
            Some(path::Seg::Name(p)) if matches!(p.as_str(), "location" | "rotation_euler" | "scale") => 3,
            Some(path::Seg::Name(p)) if matches!(p.as_str(), "rotation_quaternion" | "rotation_axis_angle") => 4,
            _ => 1,
        };
        path::channel_label(&self.path, self.index, len)
    }
    /// The Graph Editor's curve colour: XYZ as red/green/blue (W a pale
    /// yellow), anything else a hue from its path.
    pub fn color(&self) -> u32 {
        let last = self.path.rsplit('.').next().unwrap_or("");
        let quat = last.starts_with("rotation_quaternion") || last.starts_with("rotation_axis_angle");
        let xyz = matches!(last, "location" | "rotation_euler" | "scale" | "color" | "diffuse_color") || quat;
        if xyz {
            let i = if quat { self.index as i32 - 1 } else { self.index as i32 };
            return match i {
                -1 => 0xd8d27a,
                0 => 0xe8574d,
                1 => 0x7ec34a,
                2 => 0x4c8ef0,
                _ => 0xc0c0c0,
            };
        }
        let h = self.path.bytes().fold(self.index as u32 * 97, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        let hue = (h % 360) as f32;
        hsv(hue, 0.55, 0.9)
    }

    /// Add a key, replacing the value of one already on that frame.
    pub fn insert(&mut self, frame: f32, value: f32, interp: Interp) -> usize {
        let i = match self.keys.iter().position(|k| (k.frame - frame).abs() < 0.01) {
            Some(i) => {
                self.keys[i].value = value;
                i
            }
            None => {
                let at = self.keys.partition_point(|k| k.frame < frame);
                self.keys.insert(at, Keyframe::new(frame, value, interp));
                at
            }
        };
        self.recalc_handles();
        i
    }

    /// Sort keys by frame and merge keys that landed on the same frame (a
    /// moved, selected key wins), then recompute handles.
    pub fn sort(&mut self) {
        self.keys.sort_by(|a, b| a.frame.total_cmp(&b.frame).then(a.select.cmp(&b.select)));
        let mut out: Vec<Keyframe> = Vec::with_capacity(self.keys.len());
        for k in self.keys.drain(..) {
            match out.last_mut() {
                Some(last) if (last.frame - k.frame).abs() < 0.01 => {
                    if k.select || !last.select {
                        *last = k;
                    }
                }
                _ => out.push(k),
            }
        }
        self.keys = out;
        self.recalc_handles();
    }

    /// Recompute automatic and vector handles (Blender's
    /// `BKE_fcurve_handles_recalc`): Automatic handles follow the
    /// neighbours' slope, Auto Clamped ones also go flat at extremes and
    /// never overshoot a neighbour; Vector handles point a third of the way
    /// at their neighbour. Free and Aligned handles stay where they are.
    pub fn recalc_handles(&mut self) {
        let n = self.keys.len();
        for i in 0..n {
            let k = self.keys[i];
            let prev = (i > 0).then(|| self.keys[i - 1]);
            let next = (i + 1 < n).then(|| self.keys[i + 1]);
            let dl = prev.map_or(0.0, |p| (k.frame - p.frame) / 3.0);
            let dr = next.map_or(0.0, |q| (q.frame - k.frame) / 3.0);
            let clamped = k.h1 == HandleType::AutoClamped || k.h2 == HandleType::AutoClamped;
            let mut slope = 0.0;
            if let (Some(p), Some(q)) = (prev, next) {
                let extreme = (k.value >= p.value && k.value >= q.value) || (k.value <= p.value && k.value <= q.value);
                if q.frame > p.frame && !(clamped && extreme) {
                    slope = (q.value - p.value) / (q.frame - p.frame);
                    if clamped {
                        let lim_l = if dl > 0.0 { (k.value - p.value).abs() / dl } else { f32::INFINITY };
                        let lim_r = if dr > 0.0 { (q.value - k.value).abs() / dr } else { f32::INFINITY };
                        let lim = lim_l.min(lim_r);
                        if slope.abs() > lim {
                            slope = lim * slope.signum();
                        }
                    }
                }
            }
            let dl2 = if prev.is_some() { dl } else { dr.max(1.0 / 3.0) };
            let dr2 = if next.is_some() { dr } else { dl.max(1.0 / 3.0) };
            if k.h1.is_auto() {
                self.keys[i].left = [k.frame - dl2, k.value - slope * dl2];
            } else if k.h1 == HandleType::Vector {
                self.keys[i].left = match prev {
                    Some(p) => [k.frame - dl, k.value + (p.value - k.value) / 3.0],
                    None => [k.frame - dl2, k.value],
                };
            }
            if k.h2.is_auto() {
                self.keys[i].right = [k.frame + dr2, k.value + slope * dr2];
            } else if k.h2 == HandleType::Vector {
                self.keys[i].right = match next {
                    Some(q) => [k.frame + dr, k.value + (q.value - k.value) / 3.0],
                    None => [k.frame + dr2, k.value],
                };
            }
        }
    }

    /// Keep Aligned handles in line after `left` (`which` false) or `right`
    /// moved: the other handle turns to point the opposite way, keeping its
    /// length.
    pub fn align_handles(&mut self, k: usize, moved_right: bool) {
        let Some(key) = self.keys.get_mut(k) else { return };
        let (a, b, ta) = if moved_right { (key.right, key.left, key.h1) } else { (key.left, key.right, key.h2) };
        if ta != HandleType::Aligned {
            return;
        }
        let d = [a[0] - key.frame, a[1] - key.value];
        let len_d = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let len_b = ((b[0] - key.frame).powi(2) + (b[1] - key.value).powi(2)).sqrt();
        if len_d < 1e-9 {
            return;
        }
        let nb = [key.frame - d[0] / len_d * len_b, key.value - d[1] / len_d * len_b];
        if moved_right {
            key.left = nb;
        } else {
            key.right = nb;
        }
    }

    fn eval_keys(&self, frame: f32) -> f32 {
        let keys = &self.keys;
        let (Some(first), Some(last)) = (keys.first(), keys.last()) else { return 0.0 };
        if frame <= first.frame {
            if self.extrapolation == Extrapolation::Linear && keys.len() > 1 {
                let slope = if first.interp == Interp::Bezier && first.frame - first.left[0] > 1e-6 {
                    (first.value - first.left[1]) / (first.frame - first.left[0])
                } else if first.interp == Interp::Constant {
                    0.0
                } else {
                    (keys[1].value - first.value) / (keys[1].frame - first.frame).max(1e-6)
                };
                return first.value + slope * (frame - first.frame);
            }
            return first.value;
        }
        if frame >= last.frame {
            if self.extrapolation == Extrapolation::Linear && keys.len() > 1 {
                let p = &keys[keys.len() - 2];
                let slope = if p.interp == Interp::Bezier && last.right[0] - last.frame > 1e-6 {
                    (last.right[1] - last.value) / (last.right[0] - last.frame)
                } else if p.interp == Interp::Constant {
                    0.0
                } else {
                    (last.value - p.value) / (last.frame - p.frame).max(1e-6)
                };
                return last.value + slope * (frame - last.frame);
            }
            return last.value;
        }
        let i = keys.partition_point(|k| k.frame <= frame).saturating_sub(1);
        let (a, b) = (keys[i], keys[i + 1]);
        let span = (b.frame - a.frame).max(1e-9);
        let t = (frame - a.frame) / span;
        match a.interp {
            Interp::Constant => a.value,
            Interp::Linear => a.value + (b.value - a.value) * t,
            Interp::Bezier => {
                let x1 = a.right[0].clamp(a.frame, b.frame);
                let x2 = b.left[0].clamp(a.frame, b.frame);
                let bez = |p0: f32, p1: f32, p2: f32, p3: f32, t: f32| {
                    let u = 1.0 - t;
                    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
                };
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..32 {
                    let mid = 0.5 * (lo + hi);
                    if bez(a.frame, x1, x2, b.frame, mid) < frame {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                bez(a.value, a.right[1], b.left[1], b.value, 0.5 * (lo + hi))
            }
            ease => a.value + (b.value - a.value) * ease_fraction(ease, a.easing, t),
        }
    }

    /// The curve's value at `frame`, with its modifiers.
    pub fn evaluate(&self, frame: f32) -> f32 {
        let mut t = frame;
        let mut offset = 0.0;
        if let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) {
            for m in self.modifiers.iter().filter(|m| !m.muted()) {
                if let FModifier::Cycles { before, after, .. } = *m {
                    let (f0, f1) = (first.frame, last.frame);
                    let len = f1 - f0;
                    if len <= 1e-6 {
                        continue;
                    }
                    let (mode, side) = if t < f0 {
                        (before, -1.0)
                    } else if t > f1 {
                        (after, 1.0)
                    } else {
                        (CycleMode::None, 0.0)
                    };
                    if mode == CycleMode::None {
                        continue;
                    }
                    let cycle = ((t - f0) / len).floor();
                    let local = t - f0 - cycle * len;
                    let _ = side;
                    t = if mode == CycleMode::RepeatMirror && (cycle as i64).rem_euclid(2) == 1 { f1 - local } else { f0 + local };
                    if mode == CycleMode::RepeatOffset {
                        offset += (last.value - first.value) * cycle;
                    }
                }
            }
        }
        let mut v = self.eval_keys(t) + offset;
        for m in &self.modifiers {
            if let FModifier::Noise { blend, scale, strength, phase, offset, depth, mute } = *m {
                if mute {
                    continue;
                }
                let nz = turbulence((frame - offset) / scale.max(1e-4), phase, depth);
                v = match blend {
                    NoiseBlend::Replace => v + (nz - 0.5) * strength,
                    NoiseBlend::Add => v + nz * strength,
                    NoiseBlend::Subtract => v - nz * strength,
                    NoiseBlend::Multiply => v * nz * strength,
                };
            }
        }
        v
    }

    /// (first, last) key frames.
    pub fn range(&self) -> Option<(f32, f32)> {
        Some((self.keys.first()?.frame, self.keys.last()?.frame))
    }

    /// Make the curve cyclic (Shift+E > Make Cyclic) or not.
    pub fn set_cyclic(&mut self, on: bool) {
        self.modifiers.retain(|m| !matches!(m, FModifier::Cycles { .. }));
        if on {
            self.modifiers.insert(0, FModifier::cycles());
        }
    }
}

/// Robert Penner's easing equations as Blender uses them
/// (`BLI_easing_*`), as a 0..1 fraction of the change at time `t` in 0..1.
pub fn ease_fraction(interp: Interp, easing: Easing, t: f32) -> f32 {
    use core::f32::consts::PI;
    let dynamic = matches!(interp, Interp::Back | Interp::Bounce | Interp::Elastic);
    let easing = match easing {
        Easing::Auto if dynamic => Easing::Out,
        Easing::Auto => Easing::In,
        e => e,
    };
    let ein = |t: f32| -> f32 {
        match interp {
            Interp::Sine => 1.0 - (t * PI / 2.0).cos(),
            Interp::Quad => t * t,
            Interp::Cubic => t * t * t,
            Interp::Quart => t.powi(4),
            Interp::Quint => t.powi(5),
            Interp::Expo => {
                if t <= 0.0 {
                    0.0
                } else {
                    2f32.powf(10.0 * t - 10.0)
                }
            }
            Interp::Circ => 1.0 - (1.0 - t * t).max(0.0).sqrt(),
            Interp::Back => {
                let s = 1.70158;
                t * t * ((s + 1.0) * t - s)
            }
            Interp::Bounce => 1.0 - bounce_out(1.0 - t),
            Interp::Elastic => {
                if t <= 0.0 || t >= 1.0 {
                    t
                } else {
                    let c4 = 2.0 * PI / 3.0;
                    -(2f32.powf(10.0 * t - 10.0)) * ((t * 10.0 - 10.75) * c4).sin()
                }
            }
            _ => t,
        }
    };
    match easing {
        Easing::In | Easing::Auto => ein(t),
        Easing::Out => 1.0 - ein(1.0 - t),
        Easing::InOut => {
            if t < 0.5 {
                ein(2.0 * t) / 2.0
            } else {
                1.0 - ein(2.0 - 2.0 * t) / 2.0
            }
        }
    }
}

fn bounce_out(t: f32) -> f32 {
    let (n1, d1) = (7.5625, 2.75);
    if t < 1.0 / d1 {
        n1 * t * t
    } else if t < 2.0 / d1 {
        let t = t - 1.5 / d1;
        n1 * t * t + 0.75
    } else if t < 2.5 / d1 {
        let t = t - 2.25 / d1;
        n1 * t * t + 0.9375
    } else {
        let t = t - 2.625 / d1;
        n1 * t * t + 0.984375
    }
}

fn hash(i: i64, seed: u32) -> f32 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (seed as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// Smooth 1D value noise in 0..1, summed over `depth + 1` octaves like
/// Blender's `BLI_noise_turbulence`.
fn turbulence(x: f32, phase: f32, depth: u32) -> f32 {
    let seed = (phase * 1000.0) as i32 as u32;
    let mut sum = 0.0;
    let mut amp = 1.0;
    let mut norm = 0.0;
    let mut f = 1.0;
    for _ in 0..=depth.min(8) {
        let xx = x * f;
        let i = xx.floor();
        let t = xx - i;
        let s = t * t * (3.0 - 2.0 * t);
        let a = hash(i as i64, seed);
        let b = hash(i as i64 + 1, seed);
        sum += (a + (b - a) * s) * amp;
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

/// HSV (hue in degrees) to 0xRRGGBB.
pub fn hsv(h: f32, s: f32, v: f32) -> u32 {
    let c = v * s;
    let hp = (h / 60.0) % 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let q = |f: f32| ((f + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    q(r) << 16 | q(g) << 8 | q(b)
}

/// One action (Blender's `bAction`): named F-curves shared by whoever uses it.
#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub name: String,
    pub curves: Vec<FCurve>,
    /// Kept when nothing uses it (Blender's fake user).
    pub fake_user: bool,
}

impl Action {
    pub fn new(name: &str) -> Action {
        Action { name: name.into(), curves: Vec::new(), fake_user: false }
    }
    pub fn find(&self, path: &str, index: usize) -> Option<usize> {
        self.curves.iter().position(|c| c.path == path && c.index == index)
    }
    /// The curve for `path[index]`, created (in its group, next to its
    /// siblings) when missing.
    pub fn curve_mut(&mut self, path: &str, index: usize) -> &mut FCurve {
        let i = match self.find(path, index) {
            Some(i) => i,
            None => {
                let c = FCurve::new(path, index);
                // After the last curve of the same path with a lower index,
                // else after the last curve of the same group, else at the end.
                let at = self
                    .curves
                    .iter()
                    .rposition(|x| x.path == path && x.index < index)
                    .or_else(|| self.curves.iter().rposition(|x| x.group == c.group && !c.group.is_empty()))
                    .map_or(self.curves.len(), |i| i + 1);
                self.curves.insert(at, c);
                at
            }
        };
        &mut self.curves[i]
    }
    /// Sorted unique frames that carry a key on any channel.
    pub fn key_frames(&self) -> Vec<f32> {
        let mut f: Vec<f32> = self.curves.iter().flat_map(|c| c.keys.iter().map(|k| k.frame)).collect();
        f.sort_by(f32::total_cmp);
        f.dedup_by(|a, b| (*a - *b).abs() < 0.01);
        f
    }
    /// (first, last) keyed frames.
    pub fn frame_range(&self) -> (f32, f32) {
        let f = self.key_frames();
        match (f.first(), f.last()) {
            (Some(&a), Some(&b)) if b > a => (a, b),
            (Some(&a), _) => (a, a + 1.0),
            _ => (1.0, 2.0),
        }
    }
    /// Group names in order of first appearance.
    pub fn groups(&self) -> Vec<String> {
        let mut g: Vec<String> = Vec::new();
        for c in &self.curves {
            if !g.contains(&c.group) {
                g.push(c.group.clone());
            }
        }
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_curve_is_smooth_and_clamped() {
        let mut c = FCurve::new("location", 0);
        c.insert(1.0, 0.0, Interp::Bezier);
        c.insert(21.0, 10.0, Interp::Bezier);
        assert_eq!(c.evaluate(1.0), 0.0);
        assert_eq!(c.evaluate(21.0), 10.0);
        assert!((c.evaluate(11.0) - 5.0).abs() < 1e-3);
        assert!(c.evaluate(3.0) < 1.0, "eases in");
        c.insert(31.0, 10.5, Interp::Bezier);
        for f in 1..=31 {
            let v = c.evaluate(f as f32);
            assert!((-1e-4..=10.5 + 1e-4).contains(&v), "{f}: {v}");
        }
        c.keys[0].interp = Interp::Linear;
        assert!((c.evaluate(6.0) - 2.5).abs() < 1e-4);
        c.keys[0].interp = Interp::Constant;
        assert_eq!(c.evaluate(20.0), 0.0);
    }

    #[test]
    fn easing_extrapolation_and_cycles() {
        let mut c = FCurve::new("location", 0);
        c.insert(0.0, 0.0, Interp::Linear);
        c.insert(10.0, 10.0, Interp::Linear);
        assert_eq!(c.evaluate(20.0), 10.0);
        c.extrapolation = Extrapolation::Linear;
        assert!((c.evaluate(20.0) - 20.0).abs() < 1e-4);
        assert!((c.evaluate(-5.0) + 5.0).abs() < 1e-4);
        c.extrapolation = Extrapolation::Constant;
        c.set_cyclic(true);
        assert!((c.evaluate(15.0) - 5.0).abs() < 1e-4, "{}", c.evaluate(15.0));
        if let FModifier::Cycles { after, .. } = &mut c.modifiers[0] {
            *after = CycleMode::RepeatOffset;
        }
        assert!((c.evaluate(15.0) - 15.0).abs() < 1e-4);
        c.set_cyclic(false);
        c.keys[0].interp = Interp::Quad;
        c.keys[0].easing = Easing::In;
        assert!((c.evaluate(5.0) - 2.5).abs() < 1e-4);
        c.keys[0].interp = Interp::Bounce;
        assert!((c.evaluate(10.0) - 10.0).abs() < 1e-4);
        c.modifiers.push(FModifier::noise());
        let a = c.evaluate(3.0);
        let b = c.evaluate(3.3);
        assert!(a.is_finite() && b.is_finite() && a != b);
    }

    #[test]
    fn vector_handles_point_at_neighbours() {
        let mut c = FCurve::new("location", 0);
        c.insert(0.0, 0.0, Interp::Bezier);
        c.insert(3.0, 3.0, Interp::Bezier);
        c.keys[0].h2 = HandleType::Vector;
        c.keys[1].h1 = HandleType::Vector;
        c.recalc_handles();
        assert!((c.keys[0].right[1] - 1.0).abs() < 1e-5);
        assert!((c.keys[1].left[1] - 2.0).abs() < 1e-5);
        assert!((c.evaluate(1.5) - 1.5).abs() < 1e-3, "straight line {}", c.evaluate(1.5));
    }
}
