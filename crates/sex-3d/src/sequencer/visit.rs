// SPDX-License-Identifier: GPL-3.0-only
//! Strip settings as [`Visit`] walks, for the file format and the
//! Sequencer sidebar.
use super::*;
use crate::props::{pick, NameKind, Range, Visit};

fn blends() -> Vec<(StripBlend, String, &'static str)> {
    StripBlend::ALL.iter().map(|b| (*b, b.id(), b.label())).collect()
}

/// [`pick`] for the strip blend modes (their ids are computed).
pub fn pick_blend(v: &mut dyn Visit, key: &str, label: &str, cur: &mut StripBlend) {
    let all = blends();
    let items: Vec<(StripBlend, &str, &str)> = all.iter().map(|(b, i, l)| (*b, i.as_str(), *l)).collect();
    pick(v, key, label, &items, cur);
}

const WIPES: [(WipeType, &str, &str); 4] = [(WipeType::Single, "SINGLE", "Single"), (WipeType::Double, "DOUBLE", "Double"), (WipeType::Iris, "IRIS", "Iris"), (WipeType::Clock, "CLOCK", "Clock")];
const SPEEDS: [(SpeedMode, &str, &str); 3] = [(SpeedMode::Stretch, "STRETCH", "Stretch"), (SpeedMode::Multiply, "MULTIPLY", "Multiply"), (SpeedMode::FrameNumber, "FRAME_NUMBER", "Frame Number")];
const ALIGNS: [(TextAlign, &str, &str); 3] = [(TextAlign::Left, "LEFT", "Left"), (TextAlign::Center, "CENTER", "Center"), (TextAlign::Right, "RIGHT", "Right")];
const ALIGNS_Y: [(TextAlign, &str, &str); 3] = [(TextAlign::Left, "BOTTOM", "Bottom"), (TextAlign::Center, "CENTER", "Center"), (TextAlign::Right, "TOP", "Top")];

impl Effect {
    pub fn visit(&mut self, v: &mut dyn Visit) {
        match self {
            Effect::Wipe { kind, angle, blur, wipe_in } => {
                pick(v, "transition_type", "Transition", &WIPES, kind);
                v.float("angle", "Angle", angle, Range::ANGLE);
                v.float("blur_width", "Blur Width", blur, Range::FACTOR);
                let mut d = *wipe_in as usize;
                v.choice("direction", "Direction", &[("OUT", "Out"), ("IN", "In")], &mut d);
                *wipe_in = d == 1;
            }
            Effect::Transform { translate, scale, rotation } => {
                v.floats("translate", "Position", translate, Range::ANY, false);
                v.floats("scale_xy", "Scale", scale, Range::ANY, false);
                v.float("rotation_start", "Rotation", rotation, Range::ANGLE);
            }
            Effect::Speed { mode, factor } => {
                pick(v, "speed_control", "Speed Control", &SPEEDS, mode);
                v.float("speed_factor", "Multiply Factor", factor, Range::ANY);
            }
            Effect::Glow { threshold, clamp, boost, blur, only_boost } => {
                v.float("threshold", "Threshold", threshold, Range::FACTOR);
                v.float("clamp", "Clamp", clamp, Range::FACTOR);
                v.float("boost_factor", "Boost Factor", boost, Range::new(0.0, 10.0));
                v.float("blur_radius", "Blur Distance", blur, Range::new(0.5, 20.0));
                v.boolean("use_only_boost", "Only Boost", only_boost);
            }
            Effect::GaussianBlur { size } => v.floats("size", "Size", size, Range::POSITIVE, false),
            Effect::ColorMix { blend, factor } => {
                pick_blend(v, "blend_effect", "Blend", blend);
                v.float("factor", "Factor", factor, Range::FACTOR);
            }
            _ => {}
        }
    }
}

impl TextStrip {
    pub fn visit(&mut self, v: &mut dyn Visit) {
        v.name("text", "Text", &mut self.text, NameKind::Text);
        v.float("font_size", "Size", &mut self.font_size, Range::new(1.0, 2000.0));
        v.floats("color", "Color", &mut self.color, Range::FACTOR, true);
        v.floats("location", "Location", &mut self.location, Range::ANY, false);
        pick(v, "align_x", "Alignment X", &ALIGNS, &mut self.align_x);
        pick(v, "align_y", "Anchor Y", &ALIGNS_Y, &mut self.align_y);
        v.boolean("use_shadow", "Shadow", &mut self.use_shadow);
        v.floats("shadow_color", "Shadow Color", &mut self.shadow_color, Range::FACTOR, true);
        v.boolean("use_box", "Box", &mut self.use_box);
        v.floats("box_color", "Box Color", &mut self.box_color, Range::FACTOR, true);
        v.float("box_margin", "Box Margin", &mut self.box_margin, Range::new(0.0, 1.0));
    }
}

impl StripModifier {
    pub fn visit(&mut self, v: &mut dyn Visit) {
        v.boolean("mute", "Mute", &mut self.mute);
        match &mut self.kind {
            ModKind::ColorBalance { lift, gamma, gain } => {
                v.floats("lift", "Lift", lift, Range::new(0.0, 2.0), true);
                v.floats("gamma", "Gamma", gamma, Range::new(0.0, 2.0), true);
                v.floats("gain", "Gain", gain, Range::new(0.0, 2.0), true);
            }
            ModKind::Curves { points } => {
                let mut flat: Vec<f32> = points.iter().flat_map(|p| [p.0, p.1]).collect();
                let mut n = points.len() as u32;
                v.uint("points", "Points", &mut n, 2, 64);
                flat.resize(n.max(2) as usize * 2, 1.0);
                for i in 0..n.max(2) as usize {
                    v.floats(&format!("p{i}"), &format!("Point {}", i + 1), &mut flat[i * 2..i * 2 + 2], Range::FACTOR, false);
                }
                *points = flat.chunks(2).map(|c| (c[0], c[1])).collect();
                points.sort_by(|a, b| a.0.total_cmp(&b.0));
            }
            ModKind::BrightContrast { bright, contrast } => {
                v.float("bright", "Brightness", bright, Range::new(-100.0, 100.0));
                v.float("contrast", "Contrast", contrast, Range::new(-100.0, 100.0));
            }
            ModKind::WhiteBalance { white } => v.floats("white_value", "White Value", white, Range::FACTOR, true),
        }
    }
}

impl Strip {
    /// Timing (frames and channel).
    pub fn visit_time(&mut self, v: &mut dyn Visit) {
        v.uint("channel", "Channel", &mut self.channel, 1, MAX_CHANNELS);
        v.float("start", "Start", &mut self.start, Range { step: 1.0, ..Range::ANY });
        v.float("length", "Length", &mut self.length, Range { step: 1.0, ..Range::new(1.0, f32::INFINITY) });
        v.float("offset_start", "Strip Offset Start", &mut self.offset_start, Range { step: 1.0, ..Range::POSITIVE });
        v.float("offset_end", "Strip Offset End", &mut self.offset_end, Range { step: 1.0, ..Range::POSITIVE });
        v.float("content_offset", "Content Offset", &mut self.content_offset, Range { step: 1.0, ..Range::ANY });
        v.boolean("lock", "Lock", &mut self.lock);
    }
    /// Compositing, transform and colour settings.
    pub fn visit_look(&mut self, v: &mut dyn Visit) {
        v.heading("Compositing");
        pick_blend(v, "blend_type", "Blend", &mut self.blend);
        v.float("blend_alpha", "Opacity", &mut self.opacity, Range::FACTOR);
        v.boolean("mute", "Mute", &mut self.mute);
        v.byte("color_tag", "Color Tag", &mut self.color_tag, 9);
        v.heading("Transform");
        let t = &mut self.transform;
        v.floats("offset", "Position", &mut t.offset, Range::ANY, false);
        v.floats("scale", "Scale", &mut t.scale, Range::ANY, false);
        v.float("rotation", "Rotation", &mut t.rotation, Range::ANGLE);
        v.floats("origin", "Origin", &mut t.origin, Range::FACTOR, false);
        v.boolean("use_flip_x", "Flip X", &mut self.flip_x);
        v.boolean("use_flip_y", "Flip Y", &mut self.flip_y);
        v.uints("crop", "Crop", &mut self.crop);
        v.heading("Color");
        v.float("color_saturation", "Saturation", &mut self.saturation, Range::new(0.0, 20.0));
        v.float("color_multiply", "Multiply", &mut self.multiply, Range::new(0.0, 20.0));
        v.boolean("use_reverse_frames", "Reverse Frames", &mut self.reverse);
    }
    /// The strip type's own settings.
    pub fn visit_kind(&mut self, v: &mut dyn Visit) {
        match &mut self.kind {
            StripKind::Color { color } => v.floats("color", "Color", color, Range::FACTOR, true),
            StripKind::Image { directory, files } => {
                v.name("directory", "Directory", directory, NameKind::File);
                let mut s = files.join("\n");
                v.name("files", "Files", &mut s, NameKind::Text);
                *files = s.split('\n').filter(|x| !x.is_empty()).map(String::from).collect();
            }
            StripKind::Movie { path } => v.name("filepath", "File", path, NameKind::File),
            StripKind::Sound { path, volume, pan, .. } => {
                v.name("filepath", "File", path, NameKind::File);
                v.float("volume", "Volume", volume, Range::new(0.0, 100.0));
                v.float("pan", "Pan", pan, Range::new(-2.0, 2.0));
            }
            StripKind::Scene { scene, camera } => {
                v.name("scene", "Scene", scene, NameKind::Text);
                v.name("scene_camera", "Camera", camera, NameKind::Object);
            }
            StripKind::Text(t) => t.visit(v),
            StripKind::Adjustment | StripKind::Meta { .. } => {}
            StripKind::Effect { effect, inputs, fader } => {
                let mut s = inputs.join("\n");
                v.name("inputs", "Inputs", &mut s, NameKind::Text);
                *inputs = s.split('\n').filter(|x| !x.is_empty()).map(String::from).collect();
                let mut auto = fader.is_none();
                v.boolean("use_default_fade", "Default Fade", &mut auto);
                let mut f = fader.unwrap_or(0.5);
                // The fader only means something when not automatic.
                v.float("effect_fader", "Effect Fader", &mut f, Range::FACTOR);
                *fader = if auto { None } else { Some(f) };
                effect.visit(v);
            }
        }
    }
}

impl Channel {
    pub fn visit(&mut self, v: &mut dyn Visit) {
        v.boolean("lock", "Lock", &mut self.lock);
        v.boolean("mute", "Mute", &mut self.mute);
    }
}
