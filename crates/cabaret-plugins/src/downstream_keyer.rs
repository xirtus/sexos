//! Pure-Rust port of **downstream-keyer** for Cabaret.
//!
//! Manages persistent multi-channel overlay scenes (station bugs, tickers, lower-thirds, alerts)
//! pinned *above* main Studio Mode transitions, with `tie_to_transition`, independent
//! `Cut`/`Fade`/`Slide` transitions, exclusive-group radio switching, and auto-hide timers.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use cabaret_core::compositor::{
    BlendMode, Compositor, CompositorLayer, FrameBuffer, MixColorSpace, Scaler, Transform2D,
};
use cabaret_core::filters::FilterContext;
use cabaret_core::scene::{render_studio_frame, SceneCollection, SceneError, SourceId, StudioId};
use cabaret_core::transitions::WipeDirection;

use crate::util::clamp01;

/// Independent transition style for a [`DskChannel`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DskTransition {
    #[default]
    Cut,
    Fade {
        duration_ms: u32,
    },
    Slide {
        direction: WipeDirection,
        duration_ms: u32,
    },
}

impl DskTransition {
    pub const fn duration_ms(self) -> u32 {
        match self {
            Self::Cut => 0,
            Self::Fade { duration_ms } | Self::Slide { duration_ms, .. } => duration_ms,
        }
    }
}

/// One downstream keyer overlay channel pinning a [`StudioId`] above main program transitions.
#[derive(Clone, Debug, PartialEq)]
pub struct DskChannel {
    pub id: u64,
    pub name: String,
    pub studio_id: StudioId,
    pub active: bool,
    pub tie_to_transition: bool,
    pub tied_armed: Option<bool>,
    pub transition: DskTransition,
    pub exclusive_group: Option<String>,
    pub auto_hide_ms: Option<u32>,
    pub z_order: i32,
    /// Current normalized visibility `[0.0 = off, 1.0 = fully on air]`.
    pub visibility: f32,
    auto_hide_remaining_ms: Option<f32>,
    running: Option<DskRunningTransition>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DskRunningTransition {
    from_vis: f32,
    to_vis: f32,
    elapsed_ms: f32,
    duration_ms: u32,
}

impl DskChannel {
    pub fn new(id: u64, name: impl Into<String>, studio_id: StudioId) -> Self {
        Self {
            id,
            name: name.into(),
            studio_id,
            active: false,
            tie_to_transition: false,
            tied_armed: None,
            transition: DskTransition::Cut,
            exclusive_group: None,
            auto_hide_ms: None,
            z_order: 0,
            visibility: 0.0,
            auto_hide_remaining_ms: None,
            running: None,
        }
    }

    pub fn with_transition(mut self, transition: DskTransition) -> Self {
        self.transition = transition;
        self
    }

    pub fn with_exclusive_group(mut self, group: impl Into<String>) -> Self {
        self.exclusive_group = Some(group.into());
        self
    }

    pub fn with_auto_hide_ms(mut self, ms: u32) -> Self {
        self.auto_hide_ms = Some(ms);
        self
    }

    pub fn with_tie_to_transition(mut self, tie: bool) -> Self {
        self.tie_to_transition = tie;
        self
    }

    pub fn with_z_order(mut self, z: i32) -> Self {
        self.z_order = z;
        self
    }

    /// Returns `true` if the channel currently contributes pixels (`visibility > 0`).
    pub fn is_visible_on_air(&self) -> bool {
        self.visibility > 1e-4
    }

    pub fn remaining_auto_hide_ms(&self) -> Option<f32> {
        self.auto_hide_remaining_ms
    }

    fn begin_transition(&mut self, target_active: bool) {
        self.active = target_active;
        let to_vis = if target_active { 1.0 } else { 0.0 };
        let dur = self.transition.duration_ms();
        if matches!(self.transition, DskTransition::Cut) || dur == 0 {
            self.visibility = to_vis;
            self.running = None;
        } else {
            self.running = Some(DskRunningTransition {
                from_vis: self.visibility,
                to_vis,
                elapsed_ms: 0.0,
                duration_ms: dur,
            });
        }

        if target_active {
            self.auto_hide_remaining_ms = self.auto_hide_ms.map(|ms| ms as f32);
        } else {
            self.auto_hide_remaining_ms = None;
        }
    }
}

/// Multi-channel downstream keyer bank pinned above main Studio transitions.
#[derive(Clone, Debug, Default)]
pub struct DownstreamKeyerBank {
    pub channels: Vec<DskChannel>,
}

impl DownstreamKeyerBank {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_channel(&mut self, channel: DskChannel) {
        self.channels.push(channel);
        self.channels.sort_by_key(|c| (c.z_order, c.id));
    }

    pub fn remove_channel(&mut self, id: u64) -> Option<DskChannel> {
        let pos = self.channels.iter().position(|c| c.id == id)?;
        Some(self.channels.remove(pos))
    }

    pub fn channel(&self, id: u64) -> Option<&DskChannel> {
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn channel_mut(&mut self, id: u64) -> Option<&mut DskChannel> {
        self.channels.iter_mut().find(|c| c.id == id)
    }

    /// Sets channel `id` active/inactive immediately (starting its independent DSK transition)
    /// and enforces exclusive-group radio switching when activating.
    pub fn set_active(&mut self, id: u64, active: bool) -> bool {
        let Some(idx) = self.channels.iter().position(|c| c.id == id) else {
            return false;
        };

        let group = self.channels[idx].exclusive_group.clone();
        self.channels[idx].tied_armed = None;
        self.channels[idx].begin_transition(active);

        if active {
            if let Some(grp) = group {
                for ch in &mut self.channels {
                    if ch.id != id && ch.exclusive_group.as_deref() == Some(grp.as_str()) {
                        if ch.active || ch.visibility > 0.0 {
                            ch.begin_transition(false);
                        }
                    }
                }
            }
        }
        true
    }

    /// Arms a `tie_to_transition` channel so it commits when the main Studio transition triggers.
    /// If `tie_to_transition` is `false`, returns `false`.
    pub fn arm_tied(&mut self, id: u64, next_active: bool) -> bool {
        let Some(ch) = self.channel_mut(id) else {
            return false;
        };
        if !ch.tie_to_transition {
            return false;
        }
        ch.tied_armed = Some(next_active);
        true
    }

    /// Arms a toggle for a `tie_to_transition` channel.
    pub fn arm_tied_toggle(&mut self, id: u64) -> bool {
        let Some(ch) = self.channel_mut(id) else {
            return false;
        };
        if !ch.tie_to_transition {
            return false;
        }
        let next = !ch.active;
        ch.tied_armed = Some(next);
        true
    }

    /// Executes any armed `tie_to_transition` DSK state changes synchronously when main `TAKE` fires.
    pub fn on_main_transition_triggered(&mut self) -> Vec<(u64, bool)> {
        let armed: Vec<(u64, bool)> = self
            .channels
            .iter()
            .filter_map(|c| {
                if c.tie_to_transition {
                    c.tied_armed.map(|target| (c.id, target))
                } else {
                    None
                }
            })
            .collect();

        for &(id, target) in &armed {
            self.set_active(id, target);
        }
        armed
    }

    /// Advances all running DSK transitions and auto-hide timers by `dt_ms`.
    pub fn tick(&mut self, dt_ms: f32) {
        let dt = dt_ms.max(0.0);
        let mut to_hide = Vec::new();

        for ch in &mut self.channels {
            if let Some(mut run) = ch.running {
                run.elapsed_ms += dt;
                let t = clamp01(run.elapsed_ms / (run.duration_ms.max(1) as f32));
                ch.visibility = clamp01(run.from_vis + (run.to_vis - run.from_vis) * t);
                if t >= 1.0 {
                    ch.visibility = run.to_vis;
                    ch.running = None;
                } else {
                    ch.running = Some(run);
                }
            }

            if ch.active {
                if let Some(rem) = ch.auto_hide_remaining_ms {
                    let next_rem = rem - dt;
                    if next_rem <= 0.0 {
                        ch.auto_hide_remaining_ms = None;
                        to_hide.push(ch.id);
                    } else {
                        ch.auto_hide_remaining_ms = Some(next_rem);
                    }
                }
            }
        }

        for id in to_hide {
            self.set_active(id, false);
        }
    }

    /// Composites pre-rendered channel frames above `base_frame` (e.g. a main Studio transition frame).
    pub fn composite_overlays(
        &self,
        base_frame: &FrameBuffer,
        channel_frames: &BTreeMap<u64, FrameBuffer>,
    ) -> FrameBuffer {
        let mut canvas = base_frame.clone();
        let compositor = Compositor::new([0.0, 0.0, 0.0, 0.0]);

        for ch in &self.channels {
            if !ch.is_visible_on_air() {
                continue;
            }
            let Some(ov) = channel_frames.get(&ch.id) else {
                continue;
            };
            composite_single_channel(&compositor, &mut canvas, ov, ch);
        }
        canvas
    }

    /// Renders each visible DSK channel's [`StudioId`] from `collection` and composites them
    /// above `base_frame` in deterministic `z_order`.
    pub fn composite_above(
        &self,
        base_frame: &FrameBuffer,
        collection: &SceneCollection,
        raw_sources: &BTreeMap<SourceId, FrameBuffer>,
        filter_contexts: &mut BTreeMap<SourceId, FilterContext>,
        ai_inputs: &BTreeMap<SourceId, Vec<f32>>,
    ) -> Result<FrameBuffer, SceneError> {
        let mut canvas = base_frame.clone();
        let compositor = Compositor::new([0.0, 0.0, 0.0, 0.0]);

        for ch in &self.channels {
            if !ch.is_visible_on_air() {
                continue;
            }
            let ov = render_studio_frame(
                collection,
                ch.studio_id,
                base_frame.width,
                base_frame.height,
                raw_sources,
                filter_contexts,
                ai_inputs,
            )?;
            composite_single_channel(&compositor, &mut canvas, &ov, ch);
        }
        Ok(canvas)
    }
}

fn composite_single_channel(
    compositor: &Compositor,
    canvas: &mut FrameBuffer,
    overlay: &FrameBuffer,
    ch: &DskChannel,
) {
    let p = clamp01(ch.visibility);
    let (pos, opacity) = match ch.transition {
        DskTransition::Cut => ([0.0, 0.0], if p >= 0.5 { 1.0 } else { 0.0 }),
        DskTransition::Fade { .. } => ([0.0, 0.0], p),
        DskTransition::Slide { direction, .. } => {
            let dir = direction.vector();
            let offset_x = -dir[0] * (1.0 - p) * canvas.width as f32;
            let offset_y = -dir[1] * (1.0 - p) * canvas.height as f32;
            ([offset_x, offset_y], 1.0)
        }
    };

    if opacity <= 1e-4 {
        return;
    }

    let mut transform = Transform2D::default();
    transform.pos = pos;

    compositor.draw_layer(
        canvas,
        &CompositorLayer {
            frame: overlay,
            transform,
            blend_mode: BlendMode::Normal,
            opacity,
            mix_space: MixColorSpace::Linear,
            scaler: Scaler::Bilinear,
            visible: true,
        },
    );
}
