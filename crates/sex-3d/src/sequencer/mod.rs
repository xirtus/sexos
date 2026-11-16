// SPDX-License-Identifier: GPL-3.0-only
//! The Video Sequencer (Blender's VSE): strips on numbered channels, their
//! timing (content start and length, trimmed by the handles' offsets), the
//! edit operations (move with shuffling, extend, soft and hard split,
//! duplicate, delete, meta strips, mute, lock) and, in [`render`], the
//! composited frame.
pub mod render;
#[cfg(test)]
mod tests;

pub use render::{render_frame, RenderCtx};

/// How a strip combines with what is below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StripBlend {
    Replace,
    Cross,
    #[default]
    AlphaOver,
    AlphaUnder,
    Add,
    Subtract,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    Difference,
    Exclusion,
    SoftLight,
    HardLight,
    Dodge,
    Burn,
}

impl StripBlend {
    pub const ALL: [StripBlend; 17] = [
        StripBlend::Replace,
        StripBlend::Cross,
        StripBlend::AlphaOver,
        StripBlend::AlphaUnder,
        StripBlend::Add,
        StripBlend::Subtract,
        StripBlend::Multiply,
        StripBlend::Screen,
        StripBlend::Overlay,
        StripBlend::Darken,
        StripBlend::Lighten,
        StripBlend::Difference,
        StripBlend::Exclusion,
        StripBlend::SoftLight,
        StripBlend::HardLight,
        StripBlend::Dodge,
        StripBlend::Burn,
    ];
    pub fn label(self) -> &'static str {
        match self {
            StripBlend::Replace => "Replace",
            StripBlend::Cross => "Cross",
            StripBlend::AlphaOver => "Alpha Over",
            StripBlend::AlphaUnder => "Alpha Under",
            StripBlend::Add => "Add",
            StripBlend::Subtract => "Subtract",
            StripBlend::Multiply => "Multiply",
            StripBlend::Screen => "Screen",
            StripBlend::Overlay => "Overlay",
            StripBlend::Darken => "Darken",
            StripBlend::Lighten => "Lighten",
            StripBlend::Difference => "Difference",
            StripBlend::Exclusion => "Exclusion",
            StripBlend::SoftLight => "Soft Light",
            StripBlend::HardLight => "Hard Light",
            StripBlend::Dodge => "Color Dodge",
            StripBlend::Burn => "Color Burn",
        }
    }
    pub fn id(self) -> String {
        self.label().to_ascii_uppercase().replace(' ', "_")
    }
    pub fn from_id(s: &str) -> Option<StripBlend> {
        StripBlend::ALL.into_iter().find(|b| b.id().eq_ignore_ascii_case(s) || b.label() == s)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WipeType {
    #[default]
    Single,
    Double,
    Iris,
    Clock,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpeedMode {
    /// Fit the input into the effect's length.
    #[default]
    Stretch,
    Multiply,
    /// Show this input frame.
    FrameNumber,
}

/// Effect strip types.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Cross,
    GammaCross,
    Wipe { kind: WipeType, angle: f32, blur: f32, wipe_in: bool },
    Add,
    Subtract,
    Multiply,
    AlphaOver,
    AlphaUnder,
    Transform { translate: [f32; 2], scale: [f32; 2], rotation: f32 },
    Speed { mode: SpeedMode, factor: f32 },
    Glow { threshold: f32, clamp: f32, boost: f32, blur: f32, only_boost: bool },
    GaussianBlur { size: [f32; 2] },
    ColorMix { blend: StripBlend, factor: f32 },
}

impl Effect {
    pub const IDS: [&'static str; 13] = ["CROSS", "GAMMA_CROSS", "WIPE", "ADD", "SUBTRACT", "MULTIPLY", "ALPHA_OVER", "ALPHA_UNDER", "TRANSFORM", "SPEED", "GLOW", "GAUSSIAN_BLUR", "COLORMIX"];
    pub fn new(id: &str) -> Option<Effect> {
        Some(match id.to_ascii_uppercase().as_str() {
            "CROSS" => Effect::Cross,
            "GAMMA_CROSS" => Effect::GammaCross,
            "WIPE" => Effect::Wipe { kind: WipeType::Single, angle: 0.0, blur: 0.0, wipe_in: false },
            "ADD" => Effect::Add,
            "SUBTRACT" => Effect::Subtract,
            "MULTIPLY" => Effect::Multiply,
            "ALPHA_OVER" => Effect::AlphaOver,
            "ALPHA_UNDER" => Effect::AlphaUnder,
            "TRANSFORM" => Effect::Transform { translate: [0.0; 2], scale: [1.0; 2], rotation: 0.0 },
            "SPEED" => Effect::Speed { mode: SpeedMode::Stretch, factor: 1.0 },
            "GLOW" => Effect::Glow { threshold: 0.5, clamp: 1.0, boost: 0.5, blur: 3.0, only_boost: false },
            "GAUSSIAN_BLUR" => Effect::GaussianBlur { size: [10.0, 10.0] },
            "COLORMIX" | "COLOR_MIX" => Effect::ColorMix { blend: StripBlend::Overlay, factor: 1.0 },
            _ => return None,
        })
    }
    pub fn id(&self) -> &'static str {
        match self {
            Effect::Cross => "CROSS",
            Effect::GammaCross => "GAMMA_CROSS",
            Effect::Wipe { .. } => "WIPE",
            Effect::Add => "ADD",
            Effect::Subtract => "SUBTRACT",
            Effect::Multiply => "MULTIPLY",
            Effect::AlphaOver => "ALPHA_OVER",
            Effect::AlphaUnder => "ALPHA_UNDER",
            Effect::Transform { .. } => "TRANSFORM",
            Effect::Speed { .. } => "SPEED",
            Effect::Glow { .. } => "GLOW",
            Effect::GaussianBlur { .. } => "GAUSSIAN_BLUR",
            Effect::ColorMix { .. } => "COLORMIX",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Effect::Cross => "Cross",
            Effect::GammaCross => "Gamma Cross",
            Effect::Wipe { .. } => "Wipe",
            Effect::Add => "Add",
            Effect::Subtract => "Subtract",
            Effect::Multiply => "Multiply",
            Effect::AlphaOver => "Alpha Over",
            Effect::AlphaUnder => "Alpha Under",
            Effect::Transform { .. } => "Transform",
            Effect::Speed { .. } => "Speed Control",
            Effect::Glow { .. } => "Glow",
            Effect::GaussianBlur { .. } => "Gaussian Blur",
            Effect::ColorMix { .. } => "Color Mix",
        }
    }
    /// Inputs the effect takes.
    pub fn inputs(&self) -> usize {
        match self {
            Effect::Transform { .. } | Effect::Speed { .. } | Effect::Glow { .. } | Effect::GaussianBlur { .. } => 1,
            _ => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStrip {
    pub text: String,
    /// Pixels at 1080 lines.
    pub font_size: f32,
    pub color: [f32; 4],
    /// Position as a fraction of the frame (0,0 bottom-left).
    pub location: [f32; 2],
    pub align_x: TextAlign,
    pub align_y: TextAlign,
    pub use_shadow: bool,
    pub shadow_color: [f32; 4],
    pub use_box: bool,
    pub box_color: [f32; 4],
    pub box_margin: f32,
}

impl Default for TextStrip {
    fn default() -> Self {
        TextStrip {
            text: "Text".into(),
            font_size: 60.0,
            color: [1.0; 4],
            location: [0.5, 0.5],
            align_x: TextAlign::Center,
            align_y: TextAlign::Center,
            use_shadow: false,
            shadow_color: [0.0, 0.0, 0.0, 1.0],
            use_box: false,
            box_color: [0.2, 0.2, 0.2, 1.0],
            box_margin: 0.01,
        }
    }
}

/// What a strip shows.
#[derive(Clone, Debug, PartialEq)]
pub enum StripKind {
    Color { color: [f32; 3] },
    /// An image or an image sequence (one file per frame).
    Image { directory: String, files: Vec<String> },
    /// Video: no decoder is linked into Polyamour yet, the strip draws a placeholder.
    Movie { path: String },
    /// Audio: WAV files are read for their length and waveform.
    Sound { path: String, volume: f32, pan: f32, waveform: Vec<f32> },
    /// The scene seen through its camera (Polyamour has one scene).
    Scene { scene: String, camera: String },
    Text(TextStrip),
    Adjustment,
    Effect { effect: Effect, inputs: Vec<String>, fader: Option<f32> },
    Meta { strips: Vec<Strip> },
}

impl StripKind {
    pub fn type_id(&self) -> &'static str {
        match self {
            StripKind::Color { .. } => "COLOR",
            StripKind::Image { .. } => "IMAGE",
            StripKind::Movie { .. } => "MOVIE",
            StripKind::Sound { .. } => "SOUND",
            StripKind::Scene { .. } => "SCENE",
            StripKind::Text(_) => "TEXT",
            StripKind::Adjustment => "ADJUSTMENT",
            StripKind::Effect { effect, .. } => effect.id(),
            StripKind::Meta { .. } => "META",
        }
    }
    pub fn is_effect(&self) -> bool {
        matches!(self, StripKind::Effect { .. })
    }
}

/// Strip modifiers (colour correction applied to the strip's image).
#[derive(Clone, Debug, PartialEq)]
pub enum ModKind {
    ColorBalance { lift: [f32; 3], gamma: [f32; 3], gain: [f32; 3] },
    /// RGB curve: (input, output) points, sorted.
    Curves { points: Vec<(f32, f32)> },
    BrightContrast { bright: f32, contrast: f32 },
    WhiteBalance { white: [f32; 3] },
}

#[derive(Clone, Debug, PartialEq)]
pub struct StripModifier {
    pub name: String,
    pub mute: bool,
    pub kind: ModKind,
}

impl StripModifier {
    pub fn new(id: &str) -> Option<StripModifier> {
        let (name, kind) = match id.to_ascii_uppercase().as_str() {
            "COLOR_BALANCE" => ("Color Balance", ModKind::ColorBalance { lift: [1.0; 3], gamma: [1.0; 3], gain: [1.0; 3] }),
            "CURVES" => ("Curves", ModKind::Curves { points: vec![(0.0, 0.0), (1.0, 1.0)] }),
            "BRIGHT_CONTRAST" => ("Brightness/Contrast", ModKind::BrightContrast { bright: 0.0, contrast: 0.0 }),
            "WHITE_BALANCE" => ("White Balance", ModKind::WhiteBalance { white: [1.0; 3] }),
            _ => return None,
        };
        Some(StripModifier { name: name.into(), mute: false, kind })
    }
    pub fn type_id(&self) -> &'static str {
        match self.kind {
            ModKind::ColorBalance { .. } => "COLOR_BALANCE",
            ModKind::Curves { .. } => "CURVES",
            ModKind::BrightContrast { .. } => "BRIGHT_CONTRAST",
            ModKind::WhiteBalance { .. } => "WHITE_BALANCE",
        }
    }
}

/// Image placement inside the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StripTransform {
    /// Pixels at the output resolution.
    pub offset: [f32; 2],
    pub scale: [f32; 2],
    /// Radians.
    pub rotation: f32,
    /// Pivot as a fraction of the image.
    pub origin: [f32; 2],
}

impl Default for StripTransform {
    fn default() -> Self {
        StripTransform { offset: [0.0; 2], scale: [1.0; 2], rotation: 0.0, origin: [0.5; 2] }
    }
}

/// A strip.
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    pub name: String,
    pub kind: StripKind,
    /// Channel 1 is the bottom.
    pub channel: u32,
    /// Frame where the content starts (before trimming).
    pub start: f32,
    /// Content length in frames.
    pub length: f32,
    /// Trimmed from the start and the end (the handles).
    pub offset_start: f32,
    pub offset_end: f32,
    /// Content frames skipped before `start` (hard splits).
    pub content_offset: f32,
    pub select: bool,
    pub select_left: bool,
    pub select_right: bool,
    pub mute: bool,
    pub lock: bool,
    pub blend: StripBlend,
    pub opacity: f32,
    pub transform: StripTransform,
    /// Pixels cropped: left, right, bottom, top.
    pub crop: [u32; 4],
    pub flip_x: bool,
    pub flip_y: bool,
    pub reverse: bool,
    pub saturation: f32,
    pub multiply: f32,
    pub color_tag: u8,
    pub modifiers: Vec<StripModifier>,
}

impl Strip {
    pub fn new(name: &str, kind: StripKind, channel: u32, start: f32, length: f32) -> Strip {
        let blend = if matches!(kind, StripKind::Effect { .. } | StripKind::Adjustment | StripKind::Color { .. }) { StripBlend::Replace } else { StripBlend::AlphaOver };
        Strip {
            name: name.into(),
            kind,
            channel: channel.max(1),
            start,
            length: length.max(1.0),
            offset_start: 0.0,
            offset_end: 0.0,
            content_offset: 0.0,
            select: false,
            select_left: false,
            select_right: false,
            mute: false,
            lock: false,
            blend,
            opacity: 1.0,
            transform: StripTransform::default(),
            crop: [0; 4],
            flip_x: false,
            flip_y: false,
            reverse: false,
            saturation: 1.0,
            multiply: 1.0,
            color_tag: 0,
            modifiers: Vec::new(),
        }
    }
    /// First frame shown.
    pub fn final_start(&self) -> f32 {
        self.start + self.offset_start
    }
    /// One past the last frame shown.
    pub fn final_end(&self) -> f32 {
        self.start + self.length - self.offset_end
    }
    pub fn duration(&self) -> f32 {
        self.final_end() - self.final_start()
    }
    pub fn shown_at(&self, frame: f32) -> bool {
        frame >= self.final_start() && frame < self.final_end()
    }
    /// The content frame (0-based) shown at timeline `frame`.
    pub fn content_frame(&self, frame: f32) -> f32 {
        let local = (frame - self.start).clamp(0.0, (self.length - 1.0).max(0.0));
        let local = if self.reverse { self.length - 1.0 - local } else { local };
        local + self.content_offset
    }
    /// Move both handles to `[s, e)` (keeping the content).
    pub fn set_range(&mut self, s: f32, e: f32) {
        self.offset_start = (s - self.start).max(0.0);
        self.offset_end = (self.start + self.length - e).max(0.0);
    }
    pub fn effect_inputs(&self) -> &[String] {
        match &self.kind {
            StripKind::Effect { inputs, .. } => inputs,
            _ => &[],
        }
    }
    /// Whether the content can grow (still images and colours can).
    fn stretchy(&self) -> bool {
        matches!(&self.kind, StripKind::Color { .. } | StripKind::Text(_) | StripKind::Adjustment | StripKind::Effect { .. }) || matches!(&self.kind, StripKind::Image { files, .. } if files.len() <= 1)
    }
}

/// A channel's name and switches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Channel {
    pub name: String,
    pub lock: bool,
    pub mute: bool,
}

pub const MAX_CHANNELS: u32 = 128;

/// The scene's sequence editor.
#[derive(Clone, Debug, PartialEq)]
pub struct Sequencer {
    pub strips: Vec<Strip>,
    /// Index into `strips` of the active strip.
    pub active: Option<usize>,
    /// Channels by number (index 0 is channel 1); missing ones are defaults.
    pub channels: Vec<Channel>,
    /// Use the sequencer's output for renders (Output Properties > Post Processing > Sequencer).
    pub use_sequencer: bool,
}

impl Default for Sequencer {
    fn default() -> Self {
        Sequencer { strips: Vec::new(), active: None, channels: Vec::new(), use_sequencer: true }
    }
}

impl Sequencer {
    pub fn channel(&self, c: u32) -> Channel {
        self.channels.get(c.saturating_sub(1) as usize).cloned().unwrap_or_else(|| Channel { name: format!("Channel {c}"), ..Default::default() })
    }
    pub fn channel_mut(&mut self, c: u32) -> &mut Channel {
        let i = c.saturating_sub(1) as usize;
        while self.channels.len() <= i {
            let n = self.channels.len() as u32 + 1;
            self.channels.push(Channel { name: format!("Channel {n}"), ..Default::default() });
        }
        &mut self.channels[i]
    }
    pub fn find(&self, name: &str) -> Option<usize> {
        self.strips.iter().position(|s| s.name == name)
    }
    pub fn unique_name(&self, base: &str) -> String {
        crate::scene::unique_name(base, |n| self.strips.iter().any(|s| s.name == n || meta_has(&s.kind, n)))
    }
    /// Whether `[s, e)` on `channel` is free (ignoring strips in `skip`).
    pub fn is_free(&self, channel: u32, s: f32, e: f32, skip: &[usize]) -> bool {
        !self.strips.iter().enumerate().any(|(i, x)| !skip.contains(&i) && x.channel == channel && x.final_start() < e && s < x.final_end())
    }
    /// The first free channel at or above `from` for `[s, e)`.
    pub fn free_channel(&self, from: u32, s: f32, e: f32, skip: &[usize]) -> u32 {
        (from.max(1)..=MAX_CHANNELS).find(|&c| self.is_free(c, s, e, skip)).unwrap_or(MAX_CHANNELS)
    }
    /// Add a strip (unique name, first free channel from its own); it becomes
    /// the only selection and active.
    pub fn add(&mut self, mut s: Strip) -> usize {
        s.name = self.unique_name(&s.name);
        s.channel = self.free_channel(s.channel, s.final_start(), s.final_end(), &[]);
        self.strips.iter_mut().for_each(|x| x.select = false);
        s.select = true;
        self.strips.push(s);
        let i = self.strips.len() - 1;
        self.active = Some(i);
        self.sync_effects();
        i
    }
    /// An effect strip over the given inputs (spanning their overlap).
    pub fn add_effect(&mut self, effect: Effect, inputs: &[usize], channel: u32, frame: f32) -> Option<usize> {
        if inputs.len() < effect.inputs() {
            return None;
        }
        let ins: Vec<&Strip> = inputs.iter().take(effect.inputs()).map(|&i| &self.strips[i]).collect();
        let s = ins.iter().map(|x| x.final_start()).fold(f32::NEG_INFINITY, f32::max);
        let e = ins.iter().map(|x| x.final_end()).fold(f32::INFINITY, f32::min);
        let (s, e) = if e > s { (s, e) } else { (frame, frame + 25.0) };
        let top = ins.iter().map(|x| x.channel).max().unwrap_or(1) + 1;
        let names: Vec<String> = ins.iter().map(|x| x.name.clone()).collect();
        let mut st = Strip::new(effect.label(), StripKind::Effect { effect, inputs: names, fader: None }, channel.max(top), s, e - s);
        st.color_tag = 0;
        Some(self.add(st))
    }
    /// Effect strips follow their inputs' overlap.
    pub fn sync_effects(&mut self) {
        for i in 0..self.strips.len() {
            let ins: Vec<String> = self.strips[i].effect_inputs().to_vec();
            if ins.is_empty() {
                continue;
            }
            let found: Vec<&Strip> = ins.iter().filter_map(|n| self.strips.iter().find(|s| &s.name == n)).collect();
            if found.len() != ins.len() {
                continue;
            }
            let s = found.iter().map(|x| x.final_start()).fold(f32::NEG_INFINITY, f32::max);
            let e = found.iter().map(|x| x.final_end()).fold(f32::INFINITY, f32::min);
            if e > s {
                let st = &mut self.strips[i];
                st.start = s;
                st.length = e - s;
                st.offset_start = 0.0;
                st.offset_end = 0.0;
            }
        }
    }
    pub fn selected(&self) -> Vec<usize> {
        (0..self.strips.len()).filter(|&i| self.strips[i].select).collect()
    }
    pub fn select_all(&mut self, on: Option<bool>) {
        let on = on.unwrap_or_else(|| !self.strips.iter().any(|s| s.select));
        for s in &mut self.strips {
            s.select = on;
            s.select_left = false;
            s.select_right = false;
        }
    }
    /// The strip under (frame, channel).
    pub fn strip_at(&self, frame: f32, channel: u32) -> Option<usize> {
        self.strips.iter().rposition(|s| s.channel == channel && s.shown_at(frame))
    }
    /// Click select: the strip at (frame, channel); `extend` toggles.
    pub fn select_pick(&mut self, frame: f32, channel: u32, extend: bool) -> Option<usize> {
        let hit = self.strip_at(frame, channel);
        if !extend {
            self.select_all(Some(false));
        }
        if let Some(i) = hit {
            let on = !(extend && self.strips[i].select);
            self.strips[i].select = on;
            if on {
                self.active = Some(i);
            }
        }
        hit
    }
    /// Box select over frames `f0..f1` and channels `c0..=c1`.
    pub fn select_box(&mut self, f0: f32, f1: f32, c0: u32, c1: u32, extend: bool) {
        let (f0, f1) = (f0.min(f1), f0.max(f1));
        let (c0, c1) = (c0.min(c1), c0.max(c1));
        for s in &mut self.strips {
            let hit = s.channel >= c0 && s.channel <= c1 && s.final_start() < f1 && f0 < s.final_end();
            if hit {
                s.select = true;
            } else if !extend {
                s.select = false;
            }
        }
    }
    /// Select strips on one side of `frame` (Ctrl+[ / ]).
    pub fn select_side(&mut self, frame: f32, right: bool) {
        for s in &mut self.strips {
            s.select = if right { s.final_start() >= frame } else { s.final_end() <= frame };
        }
    }
    /// Move a strip up to the first free channel (Blender's shuffle).
    fn shuffle(&mut self, i: usize) {
        let s = &self.strips[i];
        let (st, en) = (s.final_start(), s.final_end());
        if !self.is_free(s.channel, st, en, &[i]) {
            let c = self.free_channel(s.channel, st, en, &[i]);
            self.strips[i].channel = c;
        }
    }
    /// G: move selected (unlocked) strips by `df` frames and `dc` channels;
    /// overlaps shuffle up. Effects follow their inputs.
    pub fn translate(&mut self, df: f32, dc: i32) -> usize {
        let sel: Vec<usize> = self.selected().into_iter().filter(|&i| !self.locked(i)).collect();
        for &i in &sel {
            let s = &mut self.strips[i];
            s.start += df;
            s.channel = (s.channel as i32 + dc).clamp(1, MAX_CHANNELS as i32) as u32;
        }
        self.sync_effects();
        for &i in &sel {
            self.shuffle(i);
        }
        sel.len()
    }
    pub fn locked(&self, i: usize) -> bool {
        let s = &self.strips[i];
        s.lock || self.channels.get(s.channel as usize - 1).is_some_and(|c| c.lock)
    }
    /// E: extend from `frame` by `df`: strips crossing it move their handle
    /// on the `right` side; strips wholly on that side move.
    pub fn extend(&mut self, frame: f32, df: f32, right: bool) {
        for i in self.selected() {
            if self.locked(i) {
                continue;
            }
            let stretchy = self.strips[i].stretchy();
            let s = &mut self.strips[i];
            let crosses = s.final_start() < frame && frame < s.final_end();
            if crosses {
                if right {
                    let e = s.final_end() + df;
                    if stretchy && e > s.start + s.length {
                        s.length = e - s.start;
                    }
                    s.offset_end = (s.start + s.length - e.max(s.final_start() + 1.0)).max(0.0);
                } else {
                    let b = s.final_start() + df;
                    if stretchy && b < s.start {
                        let grow = s.start - b;
                        s.start = b;
                        s.length += grow;
                    }
                    s.offset_start = (b.min(s.final_end() - 1.0) - s.start).max(0.0);
                }
            } else if (right && s.final_start() >= frame) || (!right && s.final_end() <= frame) {
                s.start += df;
            }
        }
        self.sync_effects();
    }
    /// Move one handle of strip `i` to frame `to` (handle drag).
    pub fn set_handle(&mut self, i: usize, right: bool, to: f32) {
        let stretchy = self.strips[i].stretchy();
        let s = &mut self.strips[i];
        if right {
            let to = to.max(s.final_start() + 1.0);
            if stretchy && to > s.start + s.length {
                s.length = to - s.start;
            }
            s.offset_end = (s.start + s.length - to).max(0.0);
        } else {
            let to = to.min(s.final_end() - 1.0);
            if stretchy && to < s.start {
                s.length += s.start - to;
                s.start = to;
            }
            s.offset_start = (to - s.start).max(0.0);
        }
        self.sync_effects();
    }
    /// K / Shift+K: split selected strips at `frame`. Soft keeps the hidden
    /// content behind the new handles; hard cuts it. Returns the new strips.
    pub fn split(&mut self, frame: f32, hard: bool) -> Vec<usize> {
        let mut made = Vec::new();
        for i in self.selected() {
            let s = &self.strips[i];
            if self.locked(i) || s.kind.is_effect() || !(s.final_start() < frame && frame < s.final_end()) {
                continue;
            }
            let mut right = s.clone();
            right.name = self.unique_name(&s.name);
            let left = &mut self.strips[i];
            if hard {
                let content_end = left.start + left.length;
                right.content_offset = left.content_offset + (frame - left.start);
                right.start = frame;
                right.length = content_end - frame;
                right.offset_start = 0.0;
                left.length = frame - left.start;
                left.offset_end = 0.0;
            } else {
                right.offset_start = frame - right.start;
                left.offset_end = left.start + left.length - frame;
            }
            left.select = false;
            right.select = true;
            self.strips.push(right);
            made.push(self.strips.len() - 1);
        }
        made
    }
    /// Shift+D: copies of the selected strips (selected, others not), on free channels.
    pub fn duplicate(&mut self) -> Vec<usize> {
        let sel = self.selected();
        let mut made = Vec::new();
        let mut renames = Vec::new();
        for &i in &sel {
            let mut c = self.strips[i].clone();
            let old = c.name.clone();
            c.name = self.unique_name(&c.name);
            renames.push((old, c.name.clone()));
            self.strips[i].select = false;
            c.select = true;
            self.strips.push(c);
            made.push(self.strips.len() - 1);
        }
        // Duplicated effects point at the duplicated inputs.
        for &j in &made {
            if let StripKind::Effect { inputs, .. } = &mut self.strips[j].kind {
                for n in inputs.iter_mut() {
                    if let Some((_, new)) = renames.iter().find(|(o, _)| o == n) {
                        *n = new.clone();
                    }
                }
            }
        }
        for &j in &made {
            self.shuffle(j);
        }
        self.active = made.last().copied().or(self.active);
        made
    }
    /// X: delete selected strips, and effects that lose an input.
    pub fn delete(&mut self) -> usize {
        let before = self.strips.len();
        loop {
            let gone: Vec<String> = self.strips.iter().filter(|s| s.select).map(|s| s.name.clone()).collect();
            self.strips.retain(|s| !s.select);
            let mut more = false;
            for s in &mut self.strips {
                if s.effect_inputs().iter().any(|n| gone.contains(n)) {
                    s.select = true;
                    more = true;
                }
            }
            if !more {
                break;
            }
        }
        self.active = None;
        before - self.strips.len()
    }
    /// Ctrl+G: put the selected strips in a meta strip.
    pub fn make_meta(&mut self) -> Option<usize> {
        let sel = self.selected();
        if sel.is_empty() {
            return None;
        }
        let s = sel.iter().map(|&i| self.strips[i].final_start()).fold(f32::INFINITY, f32::min);
        let e = sel.iter().map(|&i| self.strips[i].final_end()).fold(f32::NEG_INFINITY, f32::max);
        let ch = sel.iter().map(|&i| self.strips[i].channel).min().unwrap_or(1);
        let mut inner = Vec::new();
        for &i in sel.iter().rev() {
            let mut x = self.strips.remove(i);
            x.select = false;
            inner.insert(0, x);
        }
        let mut m = Strip::new("MetaStrip", StripKind::Meta { strips: inner }, ch, s, e - s);
        m.blend = StripBlend::AlphaOver;
        Some(self.add(m))
    }
    /// Ctrl+Alt+G: break selected meta strips apart.
    pub fn separate_meta(&mut self) -> usize {
        let mut n = 0;
        let mut i = 0;
        while i < self.strips.len() {
            if self.strips[i].select && matches!(self.strips[i].kind, StripKind::Meta { .. }) {
                let m = self.strips.remove(i);
                if let StripKind::Meta { strips } = m.kind {
                    for mut s in strips {
                        s.select = true;
                        let c = self.free_channel(s.channel, s.final_start(), s.final_end(), &[]);
                        s.channel = c;
                        self.strips.push(s);
                    }
                }
                n += 1;
            } else {
                i += 1;
            }
        }
        self.active = None;
        n
    }
    /// H / Alt+H (and Shift+H for unselected).
    pub fn set_mute(&mut self, mute: bool, unselected: bool) {
        for s in &mut self.strips {
            if s.select != unselected {
                s.mute = mute;
            }
        }
    }
    pub fn set_lock(&mut self, lock: bool) {
        for s in self.strips.iter_mut().filter(|s| s.select) {
            s.lock = lock;
        }
    }
    /// Shift+S: snap selected strips' start to `frame`.
    pub fn snap(&mut self, frame: f32) {
        for i in self.selected() {
            let s = &mut self.strips[i];
            s.start += frame - s.final_start();
        }
        self.sync_effects();
    }
    /// The frame range covered by every strip.
    pub fn range(&self) -> Option<(f32, f32)> {
        let s = self.strips.iter().map(|x| x.final_start()).fold(f32::INFINITY, f32::min);
        let e = self.strips.iter().map(|x| x.final_end()).fold(f32::NEG_INFINITY, f32::max);
        (e > s).then_some((s, e))
    }
    /// Whether anything would render (non-muted, non-sound strips).
    pub fn has_visual_strips(&self) -> bool {
        self.strips.iter().any(|s| !s.mute && !matches!(s.kind, StripKind::Sound { .. }))
    }
}

fn meta_has(k: &StripKind, name: &str) -> bool {
    match k {
        StripKind::Meta { strips } => strips.iter().any(|s| s.name == name || meta_has(&s.kind, name)),
        _ => false,
    }
}

/// A WAV file's sample rate, channel count, frame count and a waveform of
/// `peaks` absolute maxima (mixed down). PCM 8/16/24/32-bit and 32-bit float.
pub fn wav_info(bytes: &[u8], peaks: usize) -> Option<(u32, u16, usize, Vec<f32>)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let u16_at = |i: usize| bytes.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let mut pos = 12;
    let (mut fmt, mut data) = (None, None);
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32_at(pos + 4)? as usize;
        let body = pos + 8;
        if id == b"fmt " {
            fmt = Some((u16_at(body)?, u16_at(body + 2)?, u32_at(body + 4)?, u16_at(body + 14)?));
        } else if id == b"data" {
            data = Some((body, len.min(bytes.len().saturating_sub(body))));
        }
        pos = body + len + (len & 1);
    }
    let (format, channels, rate, bits) = fmt?;
    let (start, len) = data?;
    let bps = (bits as usize).div_ceil(8);
    if channels == 0 || bps == 0 {
        return None;
    }
    let frames = len / (bps * channels as usize);
    let sample = |i: usize| -> f32 {
        let o = start + i * bps;
        let b = &bytes[o..o + bps];
        match (format, bits) {
            (3, 32) => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            (_, 8) => (b[0] as f32 - 128.0) / 128.0,
            (_, 16) => i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0,
            (_, 24) => (((b[2] as i32) << 24 | (b[1] as i32) << 16 | (b[0] as i32) << 8) >> 8) as f32 / 8_388_608.0,
            (_, 32) => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0,
            _ => 0.0,
        }
    };
    let n = peaks.max(1);
    let mut wave = vec![0.0f32; n];
    if frames > 0 {
        for f in 0..frames {
            let k = f * n / frames;
            for c in 0..channels as usize {
                wave[k] = wave[k].max(sample(f * channels as usize + c).abs());
            }
        }
    }
    Some((rate, channels, frames, wave))
}
