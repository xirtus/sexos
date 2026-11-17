//! Pure-Rust ports of **aitum-vertical** and **aitum-multistream** for Cabaret.
//!
//! Manages simultaneous linked secondary canvases (e.g. `1080x1920` `9:16` Vertical and
//! `1080x1080` `1:1` Square) sharing live [`SourceId`] feeds from `cabaret-core` while
//! preserving independent per-canvas [`Transform2D`] layouts, backtrack [`ReplayBuffer`]s,
//! and [`StreamDest`] / [`MultiStreamRouter`] outputs.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use cabaret_core::compositor::{
    BlendMode, Compositor, CompositorError, CompositorLayer, FrameBuffer, LinearRgba,
    MixColorSpace, Scaler, Transform2D,
};
use cabaret_core::filters::{process_filter_chain, AiMatteInput, FilterContext};
use cabaret_core::outputs::{
    AudioEncodeProfile, AudioTrackMask, ContainerFormat, DestId, EncodedPacket, MultiStreamRouter,
    OutputError, RecordingSession, RecordingStep, ReplayBuffer, ServicePlatform,
    ServiceValidationIssue, StreamDest, StreamTarget, VideoEncodeProfile,
};
use cabaret_core::scene::{
    render_studio_frame, SceneCollection, SceneError, SourceId, SourceKind, StudioId,
};

/// Unique identifier for a secondary output canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanvasId(pub u64);

/// Standard social / multi-platform secondary canvas dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CanvasPreset {
    Vertical9x16,
    Square1x1,
    Portrait4x5,
    Landscape16x9,
    Custom { width: u32, height: u32 },
}

impl CanvasPreset {
    pub const fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Vertical9x16 => (1080, 1920),
            Self::Square1x1 => (1080, 1080),
            Self::Portrait4x5 => (1080, 1350),
            Self::Landscape16x9 => (1920, 1080),
            Self::Custom { width, height } => (width, height),
        }
    }
}

/// Errors returned by [`SecondaryCanvasManager`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MultiCanvasError {
    CanvasNotFound(CanvasId),
    Scene(SceneError),
    Output(OutputError),
    Compositor(CompositorError),
}

impl From<SceneError> for MultiCanvasError {
    fn from(e: SceneError) -> Self {
        Self::Scene(e)
    }
}

impl From<OutputError> for MultiCanvasError {
    fn from(e: OutputError) -> Self {
        Self::Output(e)
    }
}

impl From<CompositorError> for MultiCanvasError {
    fn from(e: CompositorError) -> Self {
        Self::Compositor(e)
    }
}

/// Shared live [`SourceId`] placed on a secondary canvas with its own independent layout.
#[derive(Clone, Debug, PartialEq)]
pub struct SecondaryCanvasItem {
    pub source_id: SourceId,
    pub name: String,
    pub visible: bool,
    pub transform: Transform2D,
    pub blend_mode: BlendMode,
    pub opacity: f32,
    pub opacity_space: MixColorSpace,
    pub scaler: Scaler,
}

impl SecondaryCanvasItem {
    pub fn new(source_id: SourceId, name: impl Into<String>, transform: Transform2D) -> Self {
        Self {
            source_id,
            name: name.into(),
            visible: true,
            transform,
            blend_mode: BlendMode::Normal,
            opacity: 1.0,
            opacity_space: MixColorSpace::Linear,
            scaler: Scaler::Auto,
        }
    }
}

/// Independent secondary canvas sharing Cabaret live sources with dedicated backtrack
/// [`ReplayBuffer`] and [`MultiStreamRouter`] outputs.
#[derive(Clone, Debug)]
pub struct SecondaryCanvas {
    pub id: CanvasId,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub background: LinearRgba,
    pub linked_to_main_studio: bool,
    pub active_studio_id: Option<StudioId>,
    pub studio_layouts: BTreeMap<StudioId, Vec<SecondaryCanvasItem>>,
    pub default_items: Vec<SecondaryCanvasItem>,
    pub replay_buffer: ReplayBuffer,
    pub recording: Option<RecordingSession>,
    pub router: MultiStreamRouter,
    pub available_disk_bytes: u64,
}

impl SecondaryCanvas {
    pub fn new(
        id: CanvasId,
        name: impl Into<String>,
        preset: CanvasPreset,
        backtrack_seconds: u32,
        backtrack_max_bytes: usize,
    ) -> Self {
        let (width, height) = preset.dimensions();
        Self {
            id,
            name: name.into(),
            width: width.max(1),
            height: height.max(1),
            background: [0.0, 0.0, 0.0, 1.0],
            linked_to_main_studio: true,
            active_studio_id: None,
            studio_layouts: BTreeMap::new(),
            default_items: Vec::new(),
            replay_buffer: ReplayBuffer::new(backtrack_seconds, backtrack_max_bytes),
            recording: None,
            router: MultiStreamRouter::new(),
            available_disk_bytes: 16 * 1024 * 1024 * 1024,
        }
    }

    pub fn set_studio_layout(
        &mut self,
        studio_id: StudioId,
        items: Vec<SecondaryCanvasItem>,
    ) {
        self.studio_layouts.insert(studio_id, items);
    }

    pub fn add_default_item(&mut self, item: SecondaryCanvasItem) {
        self.default_items.push(item);
    }

    pub fn active_items(&self) -> &[SecondaryCanvasItem] {
        if let Some(sid) = self.active_studio_id {
            if let Some(items) = self.studio_layouts.get(&sid) {
                return items.as_slice();
            }
        }
        self.default_items.as_slice()
    }

    pub fn add_stream_target(&mut self, target: StreamTarget) {
        self.router.add_target(target);
    }

    pub fn add_stream_dest(
        &mut self,
        id: DestId,
        name: impl Into<String>,
        platform: ServicePlatform,
        dest: StreamDest,
        video: VideoEncodeProfile,
        audio: AudioEncodeProfile,
    ) {
        self.router.add_target(StreamTarget {
            id,
            name: name.into(),
            platform,
            dest,
            video,
            audio,
            enabled: true,
            delay_sec: 0,
        });
    }

    pub fn enable_recording(&mut self, format: ContainerFormat, track_mask: AudioTrackMask) {
        let mut rec = RecordingSession::new(format, track_mask);
        rec.start();
        self.recording = Some(rec);
    }

    /// Feeds an encoded AV packet from this canvas's encode pass into its independent
    /// backtrack [`ReplayBuffer`], per-destination [`StreamDelayQueue`](cabaret_core::outputs::StreamDelayQueue)s,
    /// and optional [`RecordingSession`].
    pub fn push_encoded_packet(&mut self, pkt: EncodedPacket) -> Option<RecordingStep> {
        self.replay_buffer.push_packet(pkt.clone());
        for t in &self.router.targets {
            if t.enabled {
                if let Some(q) = self.router.delay_queues.get_mut(&t.id) {
                    q.push(pkt.clone());
                }
            }
        }
        self.recording
            .as_mut()
            .map(|rec| rec.push_packet(&pkt, self.available_disk_bytes))
    }

    /// Saves the last `seconds` from this canvas's independent backtrack replay buffer.
    pub fn save_backtrack(&self, seconds: f32) -> Result<Vec<EncodedPacket>, OutputError> {
        self.replay_buffer.save_last_seconds(seconds)
    }
}

/// Manages multiple linked secondary canvases (`aitum-vertical` + `aitum-multistream`).
#[derive(Clone, Debug, Default)]
pub struct SecondaryCanvasManager {
    pub canvases: Vec<SecondaryCanvas>,
    next_id: u64,
}

impl SecondaryCanvasManager {
    pub fn new() -> Self {
        Self {
            canvases: Vec::new(),
            next_id: 1,
        }
    }

    /// Creates and registers a new secondary canvas, returning its [`CanvasId`].
    pub fn create_canvas(
        &mut self,
        name: impl Into<String>,
        preset: CanvasPreset,
        backtrack_seconds: u32,
        backtrack_max_bytes: usize,
    ) -> CanvasId {
        let id = CanvasId(self.next_id);
        self.next_id += 1;
        let canvas =
            SecondaryCanvas::new(id, name, preset, backtrack_seconds, backtrack_max_bytes);
        self.canvases.push(canvas);
        id
    }

    pub fn canvas(&self, id: CanvasId) -> Option<&SecondaryCanvas> {
        self.canvases.iter().find(|c| c.id == id)
    }

    pub fn canvas_mut(&mut self, id: CanvasId) -> Option<&mut SecondaryCanvas> {
        self.canvases.iter_mut().find(|c| c.id == id)
    }

    pub fn remove_canvas(&mut self, id: CanvasId) -> Option<SecondaryCanvas> {
        let pos = self.canvases.iter().position(|c| c.id == id)?;
        Some(self.canvases.remove(pos))
    }

    /// Propagates a main `Cabaret` Studio change to all canvases with `linked_to_main_studio = true`.
    pub fn on_main_studio_switched(&mut self, studio_id: StudioId) {
        for c in &mut self.canvases {
            if c.linked_to_main_studio {
                c.active_studio_id = Some(studio_id);
            }
        }
    }

    /// Renders a single secondary canvas using live `SourceId` feeds + `SourceDef` filter chains
    /// from `collection` and `raw_sources`, laid out with that canvas's independent `Transform2D`s.
    pub fn render_canvas(
        &self,
        id: CanvasId,
        collection: &SceneCollection,
        raw_sources: &BTreeMap<SourceId, FrameBuffer>,
        filter_contexts: &mut BTreeMap<SourceId, FilterContext>,
        ai_inputs: &BTreeMap<SourceId, Vec<f32>>,
    ) -> Result<FrameBuffer, MultiCanvasError> {
        let sec = self
            .canvas(id)
            .ok_or(MultiCanvasError::CanvasNotFound(id))?;
        let mut out = FrameBuffer::filled(sec.width, sec.height, sec.background)?;
        let compositor = Compositor::new(sec.background);

        for item in sec.active_items() {
            if !item.visible || item.opacity <= 0.0 {
                continue;
            }
            let Some(src_def) = collection.sources.get(&item.source_id) else {
                continue;
            };
            let base = match &src_def.kind {
                SourceKind::ColorSolid { rgba } => {
                    FrameBuffer::filled(sec.width, sec.height, *rgba)?
                }
                SourceKind::NestedStudio { studio_id } => render_studio_frame(
                    collection,
                    *studio_id,
                    sec.width,
                    sec.height,
                    raw_sources,
                    filter_contexts,
                    ai_inputs,
                )?,
                _ => match raw_sources.get(&src_def.id) {
                    Some(fb) => fb.clone(),
                    None => continue,
                },
            };

            let filtered = if src_def.filters.is_empty() {
                base
            } else {
                let ctx = filter_contexts
                    .entry(src_def.id)
                    .or_insert_with(FilterContext::new);
                let ai = match ai_inputs.get(&src_def.id) {
                    Some(mask) => AiMatteInput::Success(mask.as_slice()),
                    None => AiMatteInput::Unavailable,
                };
                process_filter_chain(&src_def.filters, &base, ctx, ai, Some(&out)).unwrap_or(base)
            };

            compositor.draw_layer(
                &mut out,
                &CompositorLayer {
                    frame: &filtered,
                    transform: item.transform,
                    blend_mode: item.blend_mode,
                    opacity: item.opacity,
                    mix_space: item.opacity_space,
                    scaler: item.scaler,
                    visible: true,
                },
            );
        }

        Ok(out)
    }

    /// Simultaneously renders all registered secondary canvases.
    pub fn render_all(
        &self,
        collection: &SceneCollection,
        raw_sources: &BTreeMap<SourceId, FrameBuffer>,
        filter_contexts: &mut BTreeMap<SourceId, FilterContext>,
        ai_inputs: &BTreeMap<SourceId, Vec<f32>>,
    ) -> Result<BTreeMap<CanvasId, FrameBuffer>, MultiCanvasError> {
        let mut results = BTreeMap::new();
        for c in &self.canvases {
            let fb = self.render_canvas(c.id, collection, raw_sources, filter_contexts, ai_inputs)?;
            results.insert(c.id, fb);
        }
        Ok(results)
    }

    /// Validates streaming service compliance across every secondary canvas router.
    pub fn validate_all_streams(&self) -> BTreeMap<CanvasId, Vec<ServiceValidationIssue>> {
        let mut map = BTreeMap::new();
        for c in &self.canvases {
            map.insert(c.id, c.router.validate_all());
        }
        map
    }
}
