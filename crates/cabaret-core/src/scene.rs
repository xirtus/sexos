//! Studios, cycle-checked nested studios, 16k groups, source kinds, copy/duplicate,
//! reversible undo/redo, deterministic serialization, profiles, layout rescaling, and Crash-Guard.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::audio::ChannelLayout;
use crate::compositor::{
    Anchor, BlendMode, BoundsMode, Compositor, CompositorLayer, CropRect, FrameBuffer, LinearRgba,
    MixColorSpace, Scaler, Transform2D, MAX_COMPOSITOR_DIM,
};
use crate::filters::{
    process_filter_chain, AiMatteInput, FilterChain, FilterContext, VideoFilter,
};
use crate::transitions::{TransitionKind, TransitionSpec, WipeDirection, WipePattern};

/// Maximum dimension allowed on a sub-composite `Group` (16k x 16k).
pub const MAX_GROUP_DIM: u32 = MAX_COMPOSITOR_DIM;

/// Unique identifier for a Studio (scene).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StudioId(pub u64);

/// Unique identifier for a reusable Source definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u64);

/// Unique identifier for a SceneItem placement inside a Studio or Group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(pub u64);

/// Errors emitted by scene graph mutations, cycle validation, and serialization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneError {
    StudioNotFound(StudioId),
    SourceNotFound(SourceId),
    ItemNotFound(ItemId),
    CycleDetected { from: StudioId, to: StudioId },
    GroupSizeOutOfRange { width: u32, height: u32 },
    InvalidDimensions { width: u32, height: u32 },
    IndexOutOfBounds,
    ParseError { line: usize, message: String },
}

/// UVC webcam pixel format preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum UvcPixelFormat {
    #[default]
    Auto,
    Yuy2,
    Nv12,
    Mjpeg,
    Rgba8,
    Bgr24,
    I420,
}

/// Phone camera ingest protocol (including Hookup direct stream).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PhoneProtocol {
    #[default]
    Srt,
    Rtmp,
    WebRtc,
    HookupDirect,
}

/// Media playback looping mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LoopMode {
    Once,
    #[default]
    Loop,
    PingPong,
}

/// Outline styling for [`SourceKind::RichText`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextOutline {
    pub width_px: f32,
    pub color: LinearRgba,
}

/// Drop shadow styling for [`SourceKind::RichText`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadow {
    pub offset_x: f32,
    pub offset_y: f32,
    pub blur_px: f32,
    pub color: LinearRgba,
}

/// All supported live broadcast source types.
#[derive(Clone, Debug, PartialEq)]
pub enum SourceKind {
    Display {
        display_id: u32,
        show_cursor: bool,
    },
    SilkWindow {
        frame_id: u64,
        follow_focused: bool,
        hide_self: bool,
    },
    DisplayRegion {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
    },
    UvcCamera {
        device_id: String,
        pixel_fmt: UvcPixelFormat,
        range_auto: bool,
    },
    PhoneStream {
        protocol: PhoneProtocol,
        port: u16,
        passphrase_or_key: String,
        hookup_device: String,
    },
    MediaFile {
        path: String,
        speed_025_to_4x: f32,
        pitch_preserve: bool,
        loop_mode: LoopMode,
        alpha: bool,
    },
    Browser {
        url: String,
        width: u32,
        height: u32,
        custom_css: String,
        shutdown_when_hidden: bool,
        private_audio: bool,
    },
    PresentationDeck {
        path: String,
        slide_idx: u32,
        build_step: u32,
    },
    ImageSlideshow {
        paths: Vec<String>,
        interval_ms: u32,
        shuffle: bool,
    },
    RichText {
        content: String,
        font_px: f32,
        color: LinearRgba,
        outline: Option<TextOutline>,
        shadow: Option<TextShadow>,
        tail_lines: Option<u32>,
    },
    ColorSolid {
        rgba: LinearRgba,
    },
    NestedStudio {
        studio_id: StudioId,
    },
    Group {
        width: u32,
        height: u32,
        collapsed: bool,
        items: Vec<SceneItem>,
    },
}

impl SourceKind {
    /// Clamps or validates invariants (e.g. MediaFile playback speed `0.25..=4.0`, Group dims `1..=16384`).
    pub fn validate(&mut self) -> Result<(), SceneError> {
        match self {
            SourceKind::MediaFile {
                speed_025_to_4x, ..
            } => {
                *speed_025_to_4x = speed_025_to_4x.clamp(0.25, 4.0);
            }
            SourceKind::Group {
                width,
                height,
                items,
                ..
            } => {
                if *width == 0
                    || *height == 0
                    || *width > MAX_GROUP_DIM
                    || *height > MAX_GROUP_DIM
                {
                    return Err(SceneError::GroupSizeOutOfRange {
                        width: *width,
                        height: *height,
                    });
                }
                let _ = items;
            }
            SourceKind::DisplayRegion { w, h, .. } => {
                if *w == 0 || *h == 0 {
                    return Err(SceneError::InvalidDimensions {
                        width: *w,
                        height: *h,
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Reusable source asset + attached filter chain.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceDef {
    pub id: SourceId,
    pub name: String,
    pub kind: SourceKind,
    pub filters: FilterChain,
}

/// Studio UI presentation metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StudioLayoutMode {
    #[default]
    List,
    Grid {
        columns: u32,
    },
}

/// Instance of a source placed inside a Studio or Group.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneItem {
    pub id: ItemId,
    pub source_id: SourceId,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub transform: Transform2D,
    pub blend_mode: BlendMode,
    pub opacity: f32,
    pub opacity_space: MixColorSpace,
    pub scaler: Scaler,
    pub show_transition: Option<TransitionSpec>,
    pub hide_transition: Option<TransitionSpec>,
}

impl SceneItem {
    pub fn new(id: ItemId, source_id: SourceId, name: impl Into<String>) -> Self {
        Self {
            id,
            source_id,
            name: name.into(),
            visible: true,
            locked: false,
            transform: Transform2D::default(),
            blend_mode: BlendMode::Normal,
            opacity: 1.0,
            opacity_space: MixColorSpace::Linear,
            scaler: Scaler::Auto,
            show_transition: None,
            hide_transition: None,
        }
    }
}

/// A Studio (scene) with ordered painter's-order items, list/grid UI mode, and transition override.
#[derive(Clone, Debug, PartialEq)]
pub struct Studio {
    pub id: StudioId,
    pub name: String,
    pub layout: StudioLayoutMode,
    pub items: Vec<SceneItem>,
    pub override_transition: Option<TransitionSpec>,
}

impl Studio {
    pub fn new(id: StudioId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            layout: StudioLayoutMode::List,
            items: Vec::new(),
            override_transition: None,
        }
    }
}

/// Clipboard item holding a copied reference to a [`SourceId`] + item visual properties.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipboardEntry {
    pub source_id: SourceId,
    pub template_item: SceneItem,
}

/// Canvas layout rescaling policy when base resolution changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RescalePolicy {
    #[default]
    Stretch,
    KeepAspectCenter,
}

/// Complete SceneCollection containing Studios, Sources, and canvas configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneCollection {
    pub name: String,
    pub base_width: u32,
    pub base_height: u32,
    pub studios: Vec<Studio>,
    pub sources: BTreeMap<SourceId, SourceDef>,
    next_id: u64,
}

impl SceneCollection {
    pub fn new(name: impl Into<String>, base_width: u32, base_height: u32) -> Self {
        Self {
            name: name.into(),
            base_width: base_width.max(1),
            base_height: base_height.max(1),
            studios: Vec::new(),
            sources: BTreeMap::new(),
            next_id: 1,
        }
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn add_studio(&mut self, name: impl Into<String>) -> StudioId {
        let id = StudioId(self.alloc_id());
        self.studios.push(Studio::new(id, name));
        id
    }

    pub fn add_source(
        &mut self,
        name: impl Into<String>,
        mut kind: SourceKind,
    ) -> Result<SourceId, SceneError> {
        kind.validate()?;
        let id = SourceId(self.alloc_id());
        let def = SourceDef {
            id,
            name: name.into(),
            kind,
            filters: FilterChain::new(),
        };
        self.sources.insert(id, def);
        self.validate_acyclic()?;
        Ok(id)
    }

    pub fn studio(&self, id: StudioId) -> Option<&Studio> {
        self.studios.iter().find(|s| s.id == id)
    }

    pub fn studio_mut(&mut self, id: StudioId) -> Option<&mut Studio> {
        self.studios.iter_mut().find(|s| s.id == id)
    }

    pub fn source(&self, id: SourceId) -> Option<&SourceDef> {
        self.sources.get(&id)
    }

    pub fn source_mut(&mut self, id: SourceId) -> Option<&mut SourceDef> {
        self.sources.get_mut(&id)
    }

    pub fn replace_studio_in_place(&mut self, studio: Studio) {
        if let Some(slot) = self.studios.iter_mut().find(|s| s.id == studio.id) {
            *slot = studio;
        }
    }

    /// Inserts a `SceneItem` referencing `source_id` into `studio_id`, rejecting nested-studio cycles.
    pub fn add_item(
        &mut self,
        studio_id: StudioId,
        source_id: SourceId,
        name: impl Into<String>,
    ) -> Result<ItemId, SceneError> {
        if !self.sources.contains_key(&source_id) {
            return Err(SceneError::SourceNotFound(source_id));
        }
        let item_id = ItemId(self.alloc_id());
        let item = SceneItem::new(item_id, source_id, name);

        let studio = self
            .studio_mut(studio_id)
            .ok_or(SceneError::StudioNotFound(studio_id))?;
        studio.items.push(item);

        if let Err(err) = self.validate_acyclic() {
            if let Some(st) = self.studio_mut(studio_id) {
                st.items.pop();
            }
            return Err(err);
        }
        Ok(item_id)
    }

    /// Inserts a child `SceneItem` inside a `Group` source, rejecting invalid group sizes or cycles.
    pub fn add_item_to_group(
        &mut self,
        group_source_id: SourceId,
        child_source_id: SourceId,
        name: impl Into<String>,
    ) -> Result<ItemId, SceneError> {
        if !self.sources.contains_key(&child_source_id) {
            return Err(SceneError::SourceNotFound(child_source_id));
        }
        let item_id = ItemId(self.alloc_id());
        let item = SceneItem::new(item_id, child_source_id, name);
        let Some(group_src) = self.sources.get_mut(&group_source_id) else {
            return Err(SceneError::SourceNotFound(group_source_id));
        };
        if let SourceKind::Group { items, .. } = &mut group_src.kind {
            items.push(item);
        } else {
            return Err(SceneError::SourceNotFound(group_source_id));
        }
        if let Err(err) = self.validate_acyclic() {
            if let Some(gs) = self.sources.get_mut(&group_source_id) {
                if let SourceKind::Group { items, .. } = &mut gs.kind {
                    items.pop();
                }
            }
            return Err(err);
        }
        Ok(item_id)
    }

    /// Validates that no nested studio path forms a cycle (`A -> ... -> A`) and all groups fit `1..=16384`.
    pub fn validate_acyclic(&self) -> Result<(), SceneError> {
        for (sid, src) in &self.sources {
            if let SourceKind::Group { width, height, .. } = &src.kind {
                if *width == 0
                    || *height == 0
                    || *width > MAX_GROUP_DIM
                    || *height > MAX_GROUP_DIM
                {
                    return Err(SceneError::GroupSizeOutOfRange {
                        width: *width,
                        height: *height,
                    });
                }
            }
            let _ = sid;
        }

        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        for studio in &self.studios {
            self.dfs_studio(studio.id, &mut visiting, &mut visited)?;
        }
        Ok(())
    }

    fn dfs_studio(
        &self,
        studio_id: StudioId,
        visiting: &mut BTreeSet<StudioId>,
        visited: &mut BTreeSet<StudioId>,
    ) -> Result<(), SceneError> {
        if visited.contains(&studio_id) {
            return Ok(());
        }
        visiting.insert(studio_id);

        if let Some(studio) = self.studio(studio_id) {
            self.dfs_items(studio_id, &studio.items, visiting, visited)?;
        }

        visiting.remove(&studio_id);
        visited.insert(studio_id);
        Ok(())
    }

    fn dfs_items(
        &self,
        owner: StudioId,
        items: &[SceneItem],
        visiting: &mut BTreeSet<StudioId>,
        visited: &mut BTreeSet<StudioId>,
    ) -> Result<(), SceneError> {
        for item in items {
            if let Some(src) = self.sources.get(&item.source_id) {
                match &src.kind {
                    SourceKind::NestedStudio { studio_id: target } => {
                        if visiting.contains(target) {
                            return Err(SceneError::CycleDetected {
                                from: owner,
                                to: *target,
                            });
                        }
                        self.dfs_studio(*target, visiting, visited)?;
                    }
                    SourceKind::Group { items: sub, .. } => {
                        self.dfs_items(owner, sub, visiting, visited)?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Copies a reference to an item's existing `SourceId` + visual layout.
    pub fn copy_reference(
        &self,
        studio_id: StudioId,
        item_id: ItemId,
    ) -> Result<ClipboardEntry, SceneError> {
        let studio = self
            .studio(studio_id)
            .ok_or(SceneError::StudioNotFound(studio_id))?;
        let item = studio
            .items
            .iter()
            .find(|i| i.id == item_id)
            .ok_or(SceneError::ItemNotFound(item_id))?;
        Ok(ClipboardEntry {
            source_id: item.source_id,
            template_item: item.clone(),
        })
    }

    /// Pastes a new `SceneItem` sharing the exact same underlying `SourceId`.
    pub fn paste_reference(
        &mut self,
        target_studio_id: StudioId,
        entry: &ClipboardEntry,
    ) -> Result<ItemId, SceneError> {
        if !self.sources.contains_key(&entry.source_id) {
            return Err(SceneError::SourceNotFound(entry.source_id));
        }
        let new_item_id = ItemId(self.alloc_id());
        let mut item = entry.template_item.clone();
        item.id = new_item_id;
        item.source_id = entry.source_id;

        let studio = self
            .studio_mut(target_studio_id)
            .ok_or(SceneError::StudioNotFound(target_studio_id))?;
        studio.items.push(item);

        if let Err(err) = self.validate_acyclic() {
            if let Some(st) = self.studio_mut(target_studio_id) {
                st.items.pop();
            }
            return Err(err);
        }
        Ok(new_item_id)
    }

    /// Deep-clones the referenced `SourceDef` under a fresh `SourceId` and inserts a new `SceneItem`.
    pub fn paste_duplicate(
        &mut self,
        target_studio_id: StudioId,
        entry: &ClipboardEntry,
    ) -> Result<(ItemId, SourceId), SceneError> {
        let orig = self
            .sources
            .get(&entry.source_id)
            .cloned()
            .ok_or(SceneError::SourceNotFound(entry.source_id))?;
        let new_source_id = SourceId(self.alloc_id());
        let new_item_id = ItemId(self.alloc_id());

        let mut cloned_src = orig;
        cloned_src.id = new_source_id;
        cloned_src.name = format!("{} (Copy)", cloned_src.name);
        self.sources.insert(new_source_id, cloned_src);

        let mut item = entry.template_item.clone();
        item.id = new_item_id;
        item.source_id = new_source_id;
        item.name = format!("{} (Copy)", item.name);

        let studio = self
            .studio_mut(target_studio_id)
            .ok_or(SceneError::StudioNotFound(target_studio_id))?;
        studio.items.push(item);

        if let Err(err) = self.validate_acyclic() {
            if let Some(st) = self.studio_mut(target_studio_id) {
                st.items.pop();
            }
            self.sources.remove(&new_source_id);
            return Err(err);
        }
        Ok((new_item_id, new_source_id))
    }

    /// Automatically rescales all studio items' positions, scales, and bounds across canvas resolutions.
    pub fn rescale_canvas(
        &mut self,
        new_width: u32,
        new_height: u32,
        policy: RescalePolicy,
    ) -> Result<(), SceneError> {
        if new_width == 0 || new_height == 0 {
            return Err(SceneError::InvalidDimensions {
                width: new_width,
                height: new_height,
            });
        }
        let sx = new_width as f32 / self.base_width as f32;
        let sy = new_height as f32 / self.base_height as f32;
        let (eff_sx, eff_sy, off_x, off_y) = match policy {
            RescalePolicy::Stretch => (sx, sy, 0.0_f32, 0.0_f32),
            RescalePolicy::KeepAspectCenter => {
                let u = sx.min(sy);
                let ox = (new_width as f32 - self.base_width as f32 * u) * 0.5;
                let oy = (new_height as f32 - self.base_height as f32 * u) * 0.5;
                (u, u, ox, oy)
            }
        };

        for studio in &mut self.studios {
            for item in &mut studio.items {
                item.transform.rescale(eff_sx, eff_sy);
                item.transform.pos[0] += off_x;
                item.transform.pos[1] += off_y;
            }
        }
        self.base_width = new_width;
        self.base_height = new_height;
        Ok(())
    }

    /// Deterministic TOML-like text serialization of the complete [`SceneCollection`].
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("[collection]\n");
        out.push_str(&format!("name = {}\n", quote_str(&self.name)));
        out.push_str(&format!("base_width = {}\n", self.base_width));
        out.push_str(&format!("base_height = {}\n", self.base_height));
        out.push_str(&format!("next_id = {}\n\n", self.next_id));

        for (sid, src) in &self.sources {
            out.push_str("[[source]]\n");
            out.push_str(&format!("id = {}\n", sid.0));
            out.push_str(&format!("name = {}\n", quote_str(&src.name)));
            out.push_str(&format!("filter_count = {}\n", src.filters.len()));
            serialize_source_kind(&src.kind, &mut out);
            out.push('\n');

            if let SourceKind::Group { items, .. } = &src.kind {
                for item in items {
                    out.push_str("[[source.group_item]]\n");
                    out.push_str(&format!("group_source_id = {}\n", sid.0));
                    serialize_item(item, &mut out);
                    out.push('\n');
                }
            }
        }

        for studio in &self.studios {
            out.push_str("[[studio]]\n");
            out.push_str(&format!("id = {}\n", studio.id.0));
            out.push_str(&format!("name = {}\n", quote_str(&studio.name)));
            match studio.layout {
                StudioLayoutMode::List => out.push_str("layout = \"List\"\n"),
                StudioLayoutMode::Grid { columns } => {
                    out.push_str(&format!("layout = \"Grid:{columns}\"\n"))
                }
            }
            if let Some(tr) = &studio.override_transition {
                out.push_str(&format!("override_transition = {}\n", serialize_transition(tr)));
            }
            out.push('\n');

            for item in &studio.items {
                out.push_str("[[studio.item]]\n");
                out.push_str(&format!("studio_id = {}\n", studio.id.0));
                serialize_item(item, &mut out);
                out.push('\n');
            }
        }
        out
    }

    /// Parses a [`SceneCollection`] previously serialized via [`SceneCollection::to_text`].
    pub fn from_text(text: &str) -> Result<Self, SceneError> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Section {
            None,
            Collection,
            Source,
            GroupItem,
            Studio,
            StudioItem,
        }

        let mut col = SceneCollection::new("Untitled", 1920, 1080);
        let mut sec = Section::None;
        let mut cur_source: Option<(u64, String, BTreeMap<String, String>)> = None;
        let mut cur_group_item: Option<(u64, BTreeMap<String, String>)> = None;
        let mut cur_studio: Option<Studio> = None;
        let mut cur_item: Option<(u64, BTreeMap<String, String>)> = None;

        let flush_source = |col: &mut SceneCollection,
                            src: Option<(u64, String, BTreeMap<String, String>)>,
                            line: usize|
         -> Result<(), SceneError> {
            if let Some((id, name, map)) = src {
                let kind = deserialize_source_kind(&map, line)?;
                col.sources.insert(
                    SourceId(id),
                    SourceDef {
                        id: SourceId(id),
                        name,
                        kind,
                        filters: FilterChain::new(),
                    },
                );
            }
            Ok(())
        };

        let flush_group_item = |col: &mut SceneCollection,
                                git: Option<(u64, BTreeMap<String, String>)>,
                                line: usize|
         -> Result<(), SceneError> {
            if let Some((gsid, map)) = git {
                let item = deserialize_item(&map, line)?;
                let src = col
                    .sources
                    .get_mut(&SourceId(gsid))
                    .ok_or(SceneError::SourceNotFound(SourceId(gsid)))?;
                if let SourceKind::Group { items, .. } = &mut src.kind {
                    items.push(item);
                }
            }
            Ok(())
        };

        let flush_studio = |col: &mut SceneCollection, st: Option<Studio>| {
            if let Some(s) = st {
                col.studios.push(s);
            }
        };

        let flush_item = |col: &mut SceneCollection,
                          it: Option<(u64, BTreeMap<String, String>)>,
                          line: usize|
         -> Result<(), SceneError> {
            if let Some((studio_id, map)) = it {
                let item = deserialize_item(&map, line)?;
                let st = col
                    .studio_mut(StudioId(studio_id))
                    .ok_or(SceneError::StudioNotFound(StudioId(studio_id)))?;
                st.items.push(item);
            }
            Ok(())
        };

        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[collection]" {
                sec = Section::Collection;
                continue;
            }
            if line == "[[source]]" {
                flush_source(&mut col, cur_source.take(), line_no)?;
                flush_group_item(&mut col, cur_group_item.take(), line_no)?;
                flush_studio(&mut col, cur_studio.take());
                flush_item(&mut col, cur_item.take(), line_no)?;
                sec = Section::Source;
                cur_source = Some((0, String::new(), BTreeMap::new()));
                continue;
            }
            if line == "[[source.group_item]]" {
                flush_source(&mut col, cur_source.take(), line_no)?;
                flush_group_item(&mut col, cur_group_item.take(), line_no)?;
                flush_studio(&mut col, cur_studio.take());
                flush_item(&mut col, cur_item.take(), line_no)?;
                sec = Section::GroupItem;
                cur_group_item = Some((0, BTreeMap::new()));
                continue;
            }
            if line == "[[studio]]" {
                flush_source(&mut col, cur_source.take(), line_no)?;
                flush_group_item(&mut col, cur_group_item.take(), line_no)?;
                flush_studio(&mut col, cur_studio.take());
                flush_item(&mut col, cur_item.take(), line_no)?;
                sec = Section::Studio;
                cur_studio = Some(Studio::new(StudioId(0), ""));
                continue;
            }
            if line == "[[studio.item]]" {
                flush_source(&mut col, cur_source.take(), line_no)?;
                flush_group_item(&mut col, cur_group_item.take(), line_no)?;
                flush_studio(&mut col, cur_studio.take());
                flush_item(&mut col, cur_item.take(), line_no)?;
                sec = Section::StudioItem;
                cur_item = Some((0, BTreeMap::new()));
                continue;
            }

            let Some((k, v)) = parse_kv(line) else {
                return Err(SceneError::ParseError {
                    line: line_no,
                    message: String::from("expected key = value"),
                });
            };

            match sec {
                Section::Collection => match k {
                    "name" => col.name = unquote_str(v, line_no)?,
                    "base_width" => col.base_width = parse_u32(v, line_no)?,
                    "base_height" => col.base_height = parse_u32(v, line_no)?,
                    "next_id" => col.next_id = parse_u64(v, line_no)?,
                    _ => {}
                },
                Section::Source => {
                    if let Some((ref mut id, ref mut name, ref mut map)) = cur_source {
                        match k {
                            "id" => *id = parse_u64(v, line_no)?,
                            "name" => *name = unquote_str(v, line_no)?,
                            _ => {
                                map.insert(String::from(k), String::from(v));
                            }
                        }
                    }
                }
                Section::GroupItem => {
                    if let Some((ref mut gsid, ref mut map)) = cur_group_item {
                        if k == "group_source_id" {
                            *gsid = parse_u64(v, line_no)?;
                        } else {
                            map.insert(String::from(k), String::from(v));
                        }
                    }
                }
                Section::Studio => {
                    if let Some(ref mut st) = cur_studio {
                        match k {
                            "id" => st.id = StudioId(parse_u64(v, line_no)?),
                            "name" => st.name = unquote_str(v, line_no)?,
                            "layout" => {
                                let lv = unquote_str(v, line_no)?;
                                if let Some(cols) = lv.strip_prefix("Grid:") {
                                    st.layout = StudioLayoutMode::Grid {
                                        columns: parse_u32(cols, line_no)?,
                                    };
                                } else {
                                    st.layout = StudioLayoutMode::List;
                                }
                            }
                            "override_transition" => {
                                st.override_transition = Some(deserialize_transition(v, line_no)?);
                            }
                            _ => {}
                        }
                    }
                }
                Section::StudioItem => {
                    if let Some((ref mut st_id, ref mut map)) = cur_item {
                        if k == "studio_id" {
                            *st_id = parse_u64(v, line_no)?;
                        } else {
                            map.insert(String::from(k), String::from(v));
                        }
                    }
                }
                Section::None => {}
            }
        }

        let last_line = text.lines().count().max(1);
        flush_source(&mut col, cur_source.take(), last_line)?;
        flush_group_item(&mut col, cur_group_item.take(), last_line)?;
        flush_studio(&mut col, cur_studio.take());
        flush_item(&mut col, cur_item.take(), last_line)?;

        col.validate_acyclic()?;
        Ok(col)
    }
}

/// Reversible editing commands tracked by [`UndoStack`].
#[derive(Clone, Debug, PartialEq)]
pub enum UndoCommand {
    AddStudio {
        studio: Studio,
    },
    RemoveStudio {
        studio: Studio,
        index: usize,
    },
    RenameStudio {
        studio_id: StudioId,
        old_name: String,
        new_name: String,
    },
    AddItem {
        studio_id: StudioId,
        item: SceneItem,
        index: usize,
    },
    RemoveItem {
        studio_id: StudioId,
        item: SceneItem,
        index: usize,
    },
    SetItemTransform {
        studio_id: StudioId,
        item_id: ItemId,
        old_transform: Transform2D,
        new_transform: Transform2D,
    },
    SetItemVisibility {
        studio_id: StudioId,
        item_id: ItemId,
        old_visible: bool,
        new_visible: bool,
    },
    SetItemLocked {
        studio_id: StudioId,
        item_id: ItemId,
        old_locked: bool,
        new_locked: bool,
    },
    ReorderItem {
        studio_id: StudioId,
        old_index: usize,
        new_index: usize,
    },
    RescaleCanvas {
        old_w: u32,
        old_h: u32,
        new_w: u32,
        new_h: u32,
        policy: RescalePolicy,
        old_transforms: Vec<(StudioId, ItemId, Transform2D)>,
    },
    Batch {
        label: String,
        commands: Vec<UndoCommand>,
    },
}

impl UndoCommand {
    pub fn label(&self) -> &str {
        match self {
            Self::AddStudio { .. } => "Add Studio",
            Self::RemoveStudio { .. } => "Remove Studio",
            Self::RenameStudio { .. } => "Rename Studio",
            Self::AddItem { .. } => "Add Item",
            Self::RemoveItem { .. } => "Remove Item",
            Self::SetItemTransform { .. } => "Transform Item",
            Self::SetItemVisibility { .. } => "Toggle Item Visibility",
            Self::SetItemLocked { .. } => "Toggle Item Lock",
            Self::ReorderItem { .. } => "Reorder Item",
            Self::RescaleCanvas { .. } => "Rescale Canvas",
            Self::Batch { label, .. } => label.as_str(),
        }
    }

    fn apply(&self, col: &mut SceneCollection) -> Result<(), SceneError> {
        match self {
            Self::AddStudio { studio } => {
                col.studios.push(studio.clone());
                col.validate_acyclic()
            }
            Self::RemoveStudio { studio, .. } => {
                let pos = col
                    .studios
                    .iter()
                    .position(|s| s.id == studio.id)
                    .ok_or(SceneError::StudioNotFound(studio.id))?;
                col.studios.remove(pos);
                Ok(())
            }
            Self::RenameStudio {
                studio_id,
                new_name,
                ..
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                st.name = new_name.clone();
                Ok(())
            }
            Self::AddItem {
                studio_id,
                item,
                index,
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                let idx = (*index).min(st.items.len());
                st.items.insert(idx, item.clone());
                if let Err(e) = col.validate_acyclic() {
                    if let Some(st2) = col.studio_mut(*studio_id) {
                        st2.items.remove(idx);
                    }
                    return Err(e);
                }
                Ok(())
            }
            Self::RemoveItem {
                studio_id, index, ..
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                if *index >= st.items.len() {
                    return Err(SceneError::IndexOutOfBounds);
                }
                st.items.remove(*index);
                Ok(())
            }
            Self::SetItemTransform {
                studio_id,
                item_id,
                new_transform,
                ..
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                let it = st
                    .items
                    .iter_mut()
                    .find(|i| i.id == *item_id)
                    .ok_or(SceneError::ItemNotFound(*item_id))?;
                it.transform = *new_transform;
                Ok(())
            }
            Self::SetItemVisibility {
                studio_id,
                item_id,
                new_visible,
                ..
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                let it = st
                    .items
                    .iter_mut()
                    .find(|i| i.id == *item_id)
                    .ok_or(SceneError::ItemNotFound(*item_id))?;
                it.visible = *new_visible;
                Ok(())
            }
            Self::SetItemLocked {
                studio_id,
                item_id,
                new_locked,
                ..
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                let it = st
                    .items
                    .iter_mut()
                    .find(|i| i.id == *item_id)
                    .ok_or(SceneError::ItemNotFound(*item_id))?;
                it.locked = *new_locked;
                Ok(())
            }
            Self::ReorderItem {
                studio_id,
                old_index,
                new_index,
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                if *old_index >= st.items.len() || *new_index >= st.items.len() {
                    return Err(SceneError::IndexOutOfBounds);
                }
                let item = st.items.remove(*old_index);
                st.items.insert(*new_index, item);
                Ok(())
            }
            Self::RescaleCanvas {
                new_w,
                new_h,
                policy,
                ..
            } => col.rescale_canvas(*new_w, *new_h, *policy),
            Self::Batch { commands, .. } => {
                for c in commands {
                    c.apply(col)?;
                }
                Ok(())
            }
        }
    }

    fn revert(&self, col: &mut SceneCollection) -> Result<(), SceneError> {
        match self {
            Self::AddStudio { studio } => {
                let pos = col
                    .studios
                    .iter()
                    .position(|s| s.id == studio.id)
                    .ok_or(SceneError::StudioNotFound(studio.id))?;
                col.studios.remove(pos);
                Ok(())
            }
            Self::RemoveStudio { studio, index } => {
                let idx = (*index).min(col.studios.len());
                col.studios.insert(idx, studio.clone());
                col.validate_acyclic()
            }
            Self::RenameStudio {
                studio_id,
                old_name,
                ..
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                st.name = old_name.clone();
                Ok(())
            }
            Self::AddItem {
                studio_id, index, ..
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                if *index >= st.items.len() {
                    return Err(SceneError::IndexOutOfBounds);
                }
                st.items.remove(*index);
                Ok(())
            }
            Self::RemoveItem {
                studio_id,
                item,
                index,
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                let idx = (*index).min(st.items.len());
                st.items.insert(idx, item.clone());
                Ok(())
            }
            Self::SetItemTransform {
                studio_id,
                item_id,
                old_transform,
                ..
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                let it = st
                    .items
                    .iter_mut()
                    .find(|i| i.id == *item_id)
                    .ok_or(SceneError::ItemNotFound(*item_id))?;
                it.transform = *old_transform;
                Ok(())
            }
            Self::SetItemVisibility {
                studio_id,
                item_id,
                old_visible,
                ..
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                let it = st
                    .items
                    .iter_mut()
                    .find(|i| i.id == *item_id)
                    .ok_or(SceneError::ItemNotFound(*item_id))?;
                it.visible = *old_visible;
                Ok(())
            }
            Self::SetItemLocked {
                studio_id,
                item_id,
                old_locked,
                ..
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                let it = st
                    .items
                    .iter_mut()
                    .find(|i| i.id == *item_id)
                    .ok_or(SceneError::ItemNotFound(*item_id))?;
                it.locked = *old_locked;
                Ok(())
            }
            Self::ReorderItem {
                studio_id,
                old_index,
                new_index,
            } => {
                let st = col
                    .studio_mut(*studio_id)
                    .ok_or(SceneError::StudioNotFound(*studio_id))?;
                if *old_index >= st.items.len() || *new_index >= st.items.len() {
                    return Err(SceneError::IndexOutOfBounds);
                }
                let item = st.items.remove(*new_index);
                st.items.insert(*old_index, item);
                Ok(())
            }
            Self::RescaleCanvas {
                old_w,
                old_h,
                old_transforms,
                ..
            } => {
                col.base_width = *old_w;
                col.base_height = *old_h;
                for &(sid, iid, tr) in old_transforms {
                    if let Some(st) = col.studio_mut(sid) {
                        if let Some(it) = st.items.iter_mut().find(|i| i.id == iid) {
                            it.transform = tr;
                        }
                    }
                }
                Ok(())
            }
            Self::Batch { commands, .. } => {
                for c in commands.iter().rev() {
                    c.revert(col)?;
                }
                Ok(())
            }
        }
    }
}

/// Bounded reversible command history for [`SceneCollection`].
#[derive(Clone, Debug)]
pub struct UndoStack {
    undo_stack: Vec<UndoCommand>,
    redo_stack: Vec<UndoCommand>,
    pub max_depth: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(256)
    }
}

impl UndoStack {
    pub fn new(max_depth: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_depth: max_depth.max(1),
        }
    }

    pub fn execute(
        &mut self,
        col: &mut SceneCollection,
        cmd: UndoCommand,
    ) -> Result<(), SceneError> {
        cmd.apply(col)?;
        self.redo_stack.clear();
        if self.undo_stack.len() >= self.max_depth {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(cmd);
        Ok(())
    }

    pub fn undo(&mut self, col: &mut SceneCollection) -> Result<Option<String>, SceneError> {
        let Some(cmd) = self.undo_stack.pop() else {
            return Ok(None);
        };
        let label = String::from(cmd.label());
        cmd.revert(col)?;
        self.redo_stack.push(cmd);
        Ok(Some(label))
    }

    pub fn redo(&mut self, col: &mut SceneCollection) -> Result<Option<String>, SceneError> {
        let Some(cmd) = self.redo_stack.pop() else {
            return Ok(None);
        };
        let label = String::from(cmd.label());
        cmd.apply(col)?;
        self.undo_stack.push(cmd);
        Ok(Some(label))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

/// Video color space configuration in a [`Profile`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorSpaceProfile {
    #[default]
    Rec709,
    Rec2020Pq,
    Rec2020Hlg,
    Srgb,
}

/// Video signal luminance range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorRange {
    #[default]
    Limited,
    Full,
}

/// Persistent technical encoding, video, and audio settings profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub base_width: u32,
    pub base_height: u32,
    pub output_width: u32,
    pub output_height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub color_space: ColorSpaceProfile,
    pub color_range: ColorRange,
    pub default_scaler: Scaler,
    pub sample_rate_hz: u32,
    pub channel_layout: ChannelLayout,
    pub monitoring_device: String,
    pub video_bitrate_kbps: u32,
    pub audio_bitrate_kbps: u32,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: String::from("Default"),
            base_width: 1920,
            base_height: 1080,
            output_width: 1920,
            output_height: 1080,
            fps_num: 60,
            fps_den: 1,
            color_space: ColorSpaceProfile::Rec709,
            color_range: ColorRange::Limited,
            default_scaler: Scaler::Auto,
            sample_rate_hz: 48_000,
            channel_layout: ChannelLayout::Stereo,
            monitoring_device: String::from("default"),
            video_bitrate_kbps: 6000,
            audio_bitrate_kbps: 160,
        }
    }
}

impl Profile {
    /// Deterministic TOML-like serialization of this [`Profile`].
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("[profile]\n");
        out.push_str(&format!("name = {}\n", quote_str(&self.name)));
        out.push_str(&format!("base_width = {}\n", self.base_width));
        out.push_str(&format!("base_height = {}\n", self.base_height));
        out.push_str(&format!("output_width = {}\n", self.output_width));
        out.push_str(&format!("output_height = {}\n", self.output_height));
        out.push_str(&format!("fps_num = {}\n", self.fps_num));
        out.push_str(&format!("fps_den = {}\n", self.fps_den));
        out.push_str(&format!("color_space = {:?}\n", self.color_space));
        out.push_str(&format!("color_range = {:?}\n", self.color_range));
        out.push_str(&format!("default_scaler = {:?}\n", self.default_scaler));
        out.push_str(&format!("sample_rate_hz = {}\n", self.sample_rate_hz));
        out.push_str(&format!("channel_layout = {:?}\n", self.channel_layout));
        out.push_str(&format!(
            "monitoring_device = {}\n",
            quote_str(&self.monitoring_device)
        ));
        out.push_str(&format!("video_bitrate_kbps = {}\n", self.video_bitrate_kbps));
        out.push_str(&format!("audio_bitrate_kbps = {}\n", self.audio_bitrate_kbps));
        out
    }

    pub fn from_text(text: &str) -> Result<Self, SceneError> {
        let mut p = Self::default();
        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let Some((k, v)) = parse_kv(line) else {
                return Err(SceneError::ParseError {
                    line: line_no,
                    message: String::from("expected key = value"),
                });
            };
            match k {
                "name" => p.name = unquote_str(v, line_no)?,
                "base_width" => p.base_width = parse_u32(v, line_no)?,
                "base_height" => p.base_height = parse_u32(v, line_no)?,
                "output_width" => p.output_width = parse_u32(v, line_no)?,
                "output_height" => p.output_height = parse_u32(v, line_no)?,
                "fps_num" => p.fps_num = parse_u32(v, line_no)?,
                "fps_den" => p.fps_den = parse_u32(v, line_no)?,
                "color_space" => {
                    p.color_space = match v {
                        "Rec709" => ColorSpaceProfile::Rec709,
                        "Rec2020Pq" => ColorSpaceProfile::Rec2020Pq,
                        "Rec2020Hlg" => ColorSpaceProfile::Rec2020Hlg,
                        "Srgb" => ColorSpaceProfile::Srgb,
                        _ => ColorSpaceProfile::Rec709,
                    };
                }
                "color_range" => {
                    p.color_range = match v {
                        "Full" => ColorRange::Full,
                        _ => ColorRange::Limited,
                    };
                }
                "default_scaler" => p.default_scaler = parse_scaler(v),
                "sample_rate_hz" => p.sample_rate_hz = parse_u32(v, line_no)?,
                "channel_layout" => {
                    p.channel_layout = match v {
                        "Mono" => ChannelLayout::Mono,
                        "Surround5_1" => ChannelLayout::Surround5_1,
                        "Surround7_1" => ChannelLayout::Surround7_1,
                        _ => ChannelLayout::Stereo,
                    };
                }
                "monitoring_device" => p.monitoring_device = unquote_str(v, line_no)?,
                "video_bitrate_kbps" => p.video_bitrate_kbps = parse_u32(v, line_no)?,
                "audio_bitrate_kbps" => p.audio_bitrate_kbps = parse_u32(v, line_no)?,
                _ => {}
            }
        }
        Ok(p)
    }
}

/// Safe-mode feature isolation flags activated after an unclean shutdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SafeModeFlags {
    pub safe_mode: bool,
    pub disable_third_party_plugins: bool,
    pub disable_browser_sources: bool,
    pub disable_virtual_cam: bool,
}

impl SafeModeFlags {
    pub const fn normal() -> Self {
        Self {
            safe_mode: false,
            disable_third_party_plugins: false,
            disable_browser_sources: false,
            disable_virtual_cam: false,
        }
    }

    pub const fn safe_defaults() -> Self {
        Self {
            safe_mode: true,
            disable_third_party_plugins: true,
            disable_browser_sources: true,
            disable_virtual_cam: true,
        }
    }
}

/// Crash-Guard startup inspection outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrashRecoveryDecision {
    CleanStart(SafeModeFlags),
    UncleanShutdownDetected {
        previous_session_id: String,
        recommended: SafeModeFlags,
    },
}

/// Crash-Guard sentinel marker generator and startup recovery evaluator.
pub struct CrashGuard;

impl CrashGuard {
    /// Produces sentinel file content marking a live active session (`clean_exit = false`).
    pub fn begin_session(
        session_id: &str,
        started_us: u64,
        collection: &str,
        profile: &str,
    ) -> String {
        format!(
            "[crash_guard]\nsession_id = {}\nclean_exit = false\nstarted_us = {}\ncollection = {}\nprofile = {}\n",
            quote_str(session_id),
            started_us,
            quote_str(collection),
            quote_str(profile)
        )
    }

    /// Produces sentinel file content marking a normal clean exit (`clean_exit = true`).
    pub fn mark_clean_exit(session_id: &str, ended_us: u64) -> String {
        format!(
            "[crash_guard]\nsession_id = {}\nclean_exit = true\nended_us = {}\n",
            quote_str(session_id),
            ended_us
        )
    }

    /// Evaluates sentinel marker content on startup. Missing or `clean_exit = true` -> CleanStart;
    /// `clean_exit = false` or corrupt marker -> `UncleanShutdownDetected` recommending `SafeMode`.
    pub fn evaluate_marker(marker_text: Option<&str>) -> CrashRecoveryDecision {
        let Some(text) = marker_text else {
            return CrashRecoveryDecision::CleanStart(SafeModeFlags::normal());
        };
        let mut session_id = String::from("unknown");
        let mut clean_exit = None;

        for line in text.lines() {
            let trimmed = line.trim();
            if let Some((k, v)) = parse_kv(trimmed) {
                match k {
                    "session_id" => {
                        if let Ok(s) = unquote_str(v, 1) {
                            session_id = s;
                        }
                    }
                    "clean_exit" => {
                        clean_exit = match v {
                            "true" => Some(true),
                            "false" => Some(false),
                            _ => None,
                        };
                    }
                    _ => {}
                }
            }
        }

        match clean_exit {
            Some(true) => CrashRecoveryDecision::CleanStart(SafeModeFlags::normal()),
            _ => CrashRecoveryDecision::UncleanShutdownDetected {
                previous_session_id: session_id,
                recommended: SafeModeFlags::safe_defaults(),
            },
        }
    }
}

/// Renders a studio (including cycle-guarded nested studios and 16k groups) to a [`FrameBuffer`].
pub fn render_studio_frame(
    collection: &SceneCollection,
    studio_id: StudioId,
    width: u32,
    height: u32,
    raw_sources: &BTreeMap<SourceId, FrameBuffer>,
    filter_contexts: &mut BTreeMap<SourceId, FilterContext>,
    ai_inputs: &BTreeMap<SourceId, Vec<f32>>,
) -> Result<FrameBuffer, SceneError> {
    collection.validate_acyclic()?;
    render_studio_recursive(
        collection,
        studio_id,
        width,
        height,
        raw_sources,
        filter_contexts,
        ai_inputs,
        0,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_studio_recursive(
    collection: &SceneCollection,
    studio_id: StudioId,
    width: u32,
    height: u32,
    raw_sources: &BTreeMap<SourceId, FrameBuffer>,
    filter_contexts: &mut BTreeMap<SourceId, FilterContext>,
    ai_inputs: &BTreeMap<SourceId, Vec<f32>>,
    depth: usize,
) -> Result<FrameBuffer, SceneError> {
    if depth > 32 {
        return Err(SceneError::CycleDetected {
            from: studio_id,
            to: studio_id,
        });
    }
    let studio = collection
        .studio(studio_id)
        .ok_or(SceneError::StudioNotFound(studio_id))?;
    render_items_onto_canvas(
        collection,
        &studio.items,
        width,
        height,
        raw_sources,
        filter_contexts,
        ai_inputs,
        depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_items_onto_canvas(
    collection: &SceneCollection,
    items: &[SceneItem],
    width: u32,
    height: u32,
    raw_sources: &BTreeMap<SourceId, FrameBuffer>,
    filter_contexts: &mut BTreeMap<SourceId, FilterContext>,
    ai_inputs: &BTreeMap<SourceId, Vec<f32>>,
    depth: usize,
) -> Result<FrameBuffer, SceneError> {
    let mut canvas = FrameBuffer::new(width, height).map_err(|_| SceneError::InvalidDimensions {
        width,
        height,
    })?;
    let compositor = Compositor::new([0.0, 0.0, 0.0, 0.0]);

    for item in items {
        if !item.visible || item.opacity <= 0.0 {
            continue;
        }
        let Some(src) = collection.sources.get(&item.source_id) else {
            continue;
        };

        let base_frame = match &src.kind {
            SourceKind::ColorSolid { rgba } => FrameBuffer::filled(width, height, *rgba)
                .map_err(|_| SceneError::InvalidDimensions { width, height })?,
            SourceKind::NestedStudio { studio_id: child } => render_studio_recursive(
                collection,
                *child,
                width,
                height,
                raw_sources,
                filter_contexts,
                ai_inputs,
                depth + 1,
            )?,
            SourceKind::Group {
                width: gw,
                height: gh,
                items: sub_items,
                ..
            } => render_items_onto_canvas(
                collection,
                sub_items,
                *gw,
                *gh,
                raw_sources,
                filter_contexts,
                ai_inputs,
                depth + 1,
            )?,
            _ => match raw_sources.get(&src.id) {
                Some(fb) => fb.clone(),
                None => continue,
            },
        };

        let filtered = if src.filters.is_empty() {
            base_frame
        } else {
            let ctx = filter_contexts
                .entry(src.id)
                .or_insert_with(FilterContext::new);
            let ai_in = match ai_inputs.get(&src.id) {
                Some(mask) => AiMatteInput::Success(mask.as_slice()),
                None => AiMatteInput::Unavailable,
            };
            process_filter_chain(&src.filters, &base_frame, ctx, ai_in, Some(&canvas))
                .unwrap_or(base_frame)
        };

        compositor.draw_layer(
            &mut canvas,
            &CompositorLayer {
                frame: &filtered,
                transform: item.transform,
                blend_mode: item.blend_mode,
                opacity: item.opacity,
                mix_space: item.opacity_space,
                scaler: item.scaler,
                visible: item.visible,
            },
        );
    }

    Ok(canvas)
}

// ---------- Deterministic TOML-like Serialization Helpers ----------

fn quote_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn unquote_str(s: &str, line: usize) -> Result<String, SceneError> {
    let s = s.trim();
    if !(s.starts_with('"') && s.ends_with('"') && s.len() >= 2) {
        return Err(SceneError::ParseError {
            line,
            message: String::from("expected quoted string"),
        });
    }
    let inner = &s[1..s.len() - 1];
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {
                    return Err(SceneError::ParseError {
                        line,
                        message: String::from("dangling escape"),
                    })
                }
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

fn parse_kv(line: &str) -> Option<(&str, &str)> {
    let idx = line.find('=')?;
    let k = line[..idx].trim();
    let v = line[idx + 1..].trim();
    Some((k, v))
}

fn parse_u32(s: &str, line: usize) -> Result<u32, SceneError> {
    s.trim().parse().map_err(|_| SceneError::ParseError {
        line,
        message: format!("invalid u32: {s}"),
    })
}

fn parse_i32(s: &str, line: usize) -> Result<i32, SceneError> {
    s.trim().parse().map_err(|_| SceneError::ParseError {
        line,
        message: format!("invalid i32: {s}"),
    })
}

fn parse_u64(s: &str, line: usize) -> Result<u64, SceneError> {
    s.trim().parse().map_err(|_| SceneError::ParseError {
        line,
        message: format!("invalid u64: {s}"),
    })
}

fn parse_u16(s: &str, line: usize) -> Result<u16, SceneError> {
    s.trim().parse().map_err(|_| SceneError::ParseError {
        line,
        message: format!("invalid u16: {s}"),
    })
}

fn parse_f32(s: &str, line: usize) -> Result<f32, SceneError> {
    s.trim().parse().map_err(|_| SceneError::ParseError {
        line,
        message: format!("invalid f32: {s}"),
    })
}

fn parse_bool(s: &str, line: usize) -> Result<bool, SceneError> {
    match s.trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(SceneError::ParseError {
            line,
            message: format!("invalid bool: {s}"),
        }),
    }
}

fn fmt_f32(v: f32) -> String {
    if v.is_finite() {
        format!("{v:?}")
    } else {
        String::from("0.0")
    }
}

fn fmt_rgba(c: LinearRgba) -> String {
    format!(
        "[{}, {}, {}, {}]",
        fmt_f32(c[0]),
        fmt_f32(c[1]),
        fmt_f32(c[2]),
        fmt_f32(c[3])
    )
}

fn parse_rgba(s: &str, line: usize) -> Result<LinearRgba, SceneError> {
    let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<&str> = trimmed.split(',').collect();
    if parts.len() != 4 {
        return Err(SceneError::ParseError {
            line,
            message: format!("expected 4 floats in rgba: {s}"),
        });
    }
    Ok([
        parse_f32(parts[0], line)?,
        parse_f32(parts[1], line)?,
        parse_f32(parts[2], line)?,
        parse_f32(parts[3], line)?,
    ])
}

fn serialize_str_list(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&quote_str(s));
    }
    out.push(']');
    out
}

fn parse_str_list(s: &str, line: usize) -> Result<Vec<String>, SceneError> {
    let inner = s.trim().trim_start_matches('[').trim_end_matches(']').trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let mut res = Vec::new();
    let mut in_quotes = false;
    let mut esc = false;
    let mut cur = String::new();
    for ch in inner.chars() {
        if in_quotes {
            cur.push(ch);
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_quotes = false;
            }
        } else if ch == '"' {
            in_quotes = true;
            cur.push(ch);
        } else if ch == ',' {
            let trimmed = cur.trim();
            if !trimmed.is_empty() {
                res.push(unquote_str(trimmed, line)?);
            }
            cur.clear();
        }
    }
    let trimmed = cur.trim();
    if !trimmed.is_empty() {
        res.push(unquote_str(trimmed, line)?);
    }
    Ok(res)
}

fn serialize_source_kind(kind: &SourceKind, out: &mut String) {
    match kind {
        SourceKind::Display {
            display_id,
            show_cursor,
        } => {
            out.push_str("kind = \"Display\"\n");
            out.push_str(&format!("display_id = {display_id}\n"));
            out.push_str(&format!("show_cursor = {show_cursor}\n"));
        }
        SourceKind::SilkWindow {
            frame_id,
            follow_focused,
            hide_self,
        } => {
            out.push_str("kind = \"SilkWindow\"\n");
            out.push_str(&format!("frame_id = {frame_id}\n"));
            out.push_str(&format!("follow_focused = {follow_focused}\n"));
            out.push_str(&format!("hide_self = {hide_self}\n"));
        }
        SourceKind::DisplayRegion { x, y, w, h } => {
            out.push_str("kind = \"DisplayRegion\"\n");
            out.push_str(&format!("x = {x}\ny = {y}\nw = {w}\nh = {h}\n"));
        }
        SourceKind::UvcCamera {
            device_id,
            pixel_fmt,
            range_auto,
        } => {
            out.push_str("kind = \"UvcCamera\"\n");
            out.push_str(&format!("device_id = {}\n", quote_str(device_id)));
            out.push_str(&format!("pixel_fmt = {:?}\n", pixel_fmt));
            out.push_str(&format!("range_auto = {range_auto}\n"));
        }
        SourceKind::PhoneStream {
            protocol,
            port,
            passphrase_or_key,
            hookup_device,
        } => {
            out.push_str("kind = \"PhoneStream\"\n");
            out.push_str(&format!("protocol = {:?}\n", protocol));
            out.push_str(&format!("port = {port}\n"));
            out.push_str(&format!(
                "passphrase_or_key = {}\n",
                quote_str(passphrase_or_key)
            ));
            out.push_str(&format!("hookup_device = {}\n", quote_str(hookup_device)));
        }
        SourceKind::MediaFile {
            path,
            speed_025_to_4x,
            pitch_preserve,
            loop_mode,
            alpha,
        } => {
            out.push_str("kind = \"MediaFile\"\n");
            out.push_str(&format!("path = {}\n", quote_str(path)));
            out.push_str(&format!("speed_025_to_4x = {}\n", fmt_f32(*speed_025_to_4x)));
            out.push_str(&format!("pitch_preserve = {pitch_preserve}\n"));
            out.push_str(&format!("loop_mode = {:?}\n", loop_mode));
            out.push_str(&format!("alpha = {alpha}\n"));
        }
        SourceKind::Browser {
            url,
            width,
            height,
            custom_css,
            shutdown_when_hidden,
            private_audio,
        } => {
            out.push_str("kind = \"Browser\"\n");
            out.push_str(&format!("url = {}\n", quote_str(url)));
            out.push_str(&format!("width = {width}\nheight = {height}\n"));
            out.push_str(&format!("custom_css = {}\n", quote_str(custom_css)));
            out.push_str(&format!("shutdown_when_hidden = {shutdown_when_hidden}\n"));
            out.push_str(&format!("private_audio = {private_audio}\n"));
        }
        SourceKind::PresentationDeck {
            path,
            slide_idx,
            build_step,
        } => {
            out.push_str("kind = \"PresentationDeck\"\n");
            out.push_str(&format!("path = {}\n", quote_str(path)));
            out.push_str(&format!("slide_idx = {slide_idx}\nbuild_step = {build_step}\n"));
        }
        SourceKind::ImageSlideshow {
            paths,
            interval_ms,
            shuffle,
        } => {
            out.push_str("kind = \"ImageSlideshow\"\n");
            out.push_str(&format!("paths = {}\n", serialize_str_list(paths)));
            out.push_str(&format!("interval_ms = {interval_ms}\nshuffle = {shuffle}\n"));
        }
        SourceKind::RichText {
            content,
            font_px,
            color,
            outline,
            shadow,
            tail_lines,
        } => {
            out.push_str("kind = \"RichText\"\n");
            out.push_str(&format!("content = {}\n", quote_str(content)));
            out.push_str(&format!("font_px = {}\n", fmt_f32(*font_px)));
            out.push_str(&format!("color = {}\n", fmt_rgba(*color)));
            if let Some(ol) = outline {
                out.push_str(&format!(
                    "outline = {}|{}\n",
                    fmt_f32(ol.width_px),
                    fmt_rgba(ol.color)
                ));
            }
            if let Some(sh) = shadow {
                out.push_str(&format!(
                    "shadow = {}|{}|{}|{}\n",
                    fmt_f32(sh.offset_x),
                    fmt_f32(sh.offset_y),
                    fmt_f32(sh.blur_px),
                    fmt_rgba(sh.color)
                ));
            }
            if let Some(tl) = tail_lines {
                out.push_str(&format!("tail_lines = {tl}\n"));
            }
        }
        SourceKind::ColorSolid { rgba } => {
            out.push_str("kind = \"ColorSolid\"\n");
            out.push_str(&format!("rgba = {}\n", fmt_rgba(*rgba)));
        }
        SourceKind::NestedStudio { studio_id } => {
            out.push_str("kind = \"NestedStudio\"\n");
            out.push_str(&format!("target_studio_id = {}\n", studio_id.0));
        }
        SourceKind::Group {
            width,
            height,
            collapsed,
            ..
        } => {
            out.push_str("kind = \"Group\"\n");
            out.push_str(&format!(
                "width = {width}\nheight = {height}\ncollapsed = {collapsed}\n"
            ));
        }
    }
}

fn deserialize_source_kind(
    map: &BTreeMap<String, String>,
    line: usize,
) -> Result<SourceKind, SceneError> {
    let kind_raw = map
        .get("kind")
        .ok_or_else(|| SceneError::ParseError {
            line,
            message: String::from("missing source kind"),
        })?;
    let kind = unquote_str(kind_raw, line)?;

    let get = |k: &str| -> Result<&str, SceneError> {
        map.get(k).map(|s| s.as_str()).ok_or_else(|| SceneError::ParseError {
            line,
            message: format!("missing field {k}"),
        })
    };

    let mut sk = match kind.as_str() {
        "Display" => SourceKind::Display {
            display_id: parse_u32(get("display_id")?, line)?,
            show_cursor: parse_bool(get("show_cursor")?, line)?,
        },
        "SilkWindow" => SourceKind::SilkWindow {
            frame_id: parse_u64(get("frame_id")?, line)?,
            follow_focused: parse_bool(get("follow_focused")?, line)?,
            hide_self: parse_bool(get("hide_self")?, line)?,
        },
        "DisplayRegion" => SourceKind::DisplayRegion {
            x: parse_i32(get("x")?, line)?,
            y: parse_i32(get("y")?, line)?,
            w: parse_u32(get("w")?, line)?,
            h: parse_u32(get("h")?, line)?,
        },
        "UvcCamera" => SourceKind::UvcCamera {
            device_id: unquote_str(get("device_id")?, line)?,
            pixel_fmt: match get("pixel_fmt")? {
                "Yuy2" => UvcPixelFormat::Yuy2,
                "Nv12" => UvcPixelFormat::Nv12,
                "Mjpeg" => UvcPixelFormat::Mjpeg,
                "Rgba8" => UvcPixelFormat::Rgba8,
                "Bgr24" => UvcPixelFormat::Bgr24,
                "I420" => UvcPixelFormat::I420,
                _ => UvcPixelFormat::Auto,
            },
            range_auto: parse_bool(get("range_auto")?, line)?,
        },
        "PhoneStream" => SourceKind::PhoneStream {
            protocol: match get("protocol")? {
                "Rtmp" => PhoneProtocol::Rtmp,
                "WebRtc" => PhoneProtocol::WebRtc,
                "HookupDirect" => PhoneProtocol::HookupDirect,
                _ => PhoneProtocol::Srt,
            },
            port: parse_u16(get("port")?, line)?,
            passphrase_or_key: unquote_str(get("passphrase_or_key")?, line)?,
            hookup_device: unquote_str(get("hookup_device")?, line)?,
        },
        "MediaFile" => SourceKind::MediaFile {
            path: unquote_str(get("path")?, line)?,
            speed_025_to_4x: parse_f32(get("speed_025_to_4x")?, line)?,
            pitch_preserve: parse_bool(get("pitch_preserve")?, line)?,
            loop_mode: match get("loop_mode")? {
                "Once" => LoopMode::Once,
                "PingPong" => LoopMode::PingPong,
                _ => LoopMode::Loop,
            },
            alpha: parse_bool(get("alpha")?, line)?,
        },
        "Browser" => SourceKind::Browser {
            url: unquote_str(get("url")?, line)?,
            width: parse_u32(get("width")?, line)?,
            height: parse_u32(get("height")?, line)?,
            custom_css: unquote_str(get("custom_css")?, line)?,
            shutdown_when_hidden: parse_bool(get("shutdown_when_hidden")?, line)?,
            private_audio: parse_bool(get("private_audio")?, line)?,
        },
        "PresentationDeck" => SourceKind::PresentationDeck {
            path: unquote_str(get("path")?, line)?,
            slide_idx: parse_u32(get("slide_idx")?, line)?,
            build_step: parse_u32(get("build_step")?, line)?,
        },
        "ImageSlideshow" => SourceKind::ImageSlideshow {
            paths: parse_str_list(get("paths")?, line)?,
            interval_ms: parse_u32(get("interval_ms")?, line)?,
            shuffle: parse_bool(get("shuffle")?, line)?,
        },
        "RichText" => {
            let outline = if let Some(raw) = map.get("outline") {
                let parts: Vec<&str> = raw.split('|').collect();
                if parts.len() == 2 {
                    Some(TextOutline {
                        width_px: parse_f32(parts[0], line)?,
                        color: parse_rgba(parts[1], line)?,
                    })
                } else {
                    None
                }
            } else {
                None
            };
            let shadow = if let Some(raw) = map.get("shadow") {
                let parts: Vec<&str> = raw.split('|').collect();
                if parts.len() == 4 {
                    Some(TextShadow {
                        offset_x: parse_f32(parts[0], line)?,
                        offset_y: parse_f32(parts[1], line)?,
                        blur_px: parse_f32(parts[2], line)?,
                        color: parse_rgba(parts[3], line)?,
                    })
                } else {
                    None
                }
            } else {
                None
            };
            let tail_lines = match map.get("tail_lines") {
                Some(raw) => Some(parse_u32(raw, line)?),
                None => None,
            };
            SourceKind::RichText {
                content: unquote_str(get("content")?, line)?,
                font_px: parse_f32(get("font_px")?, line)?,
                color: parse_rgba(get("color")?, line)?,
                outline,
                shadow,
                tail_lines,
            }
        }
        "ColorSolid" => SourceKind::ColorSolid {
            rgba: parse_rgba(get("rgba")?, line)?,
        },
        "NestedStudio" => SourceKind::NestedStudio {
            studio_id: StudioId(parse_u64(get("target_studio_id")?, line)?),
        },
        "Group" => SourceKind::Group {
            width: parse_u32(get("width")?, line)?,
            height: parse_u32(get("height")?, line)?,
            collapsed: parse_bool(get("collapsed")?, line)?,
            items: Vec::new(),
        },
        other => {
            return Err(SceneError::ParseError {
                line,
                message: format!("unknown source kind: {other}"),
            })
        }
    };
    sk.validate()?;
    Ok(sk)
}

fn serialize_item(item: &SceneItem, out: &mut String) {
    out.push_str(&format!("id = {}\n", item.id.0));
    out.push_str(&format!("source_id = {}\n", item.source_id.0));
    out.push_str(&format!("name = {}\n", quote_str(&item.name)));
    out.push_str(&format!("visible = {}\n", item.visible));
    out.push_str(&format!("locked = {}\n", item.locked));
    out.push_str(&format!(
        "pos = [{}, {}]\n",
        fmt_f32(item.transform.pos[0]),
        fmt_f32(item.transform.pos[1])
    ));
    out.push_str(&format!(
        "scale = [{}, {}]\n",
        fmt_f32(item.transform.scale[0]),
        fmt_f32(item.transform.scale[1])
    ));
    out.push_str(&format!("rot_deg = {}\n", fmt_f32(item.transform.rot_deg)));
    out.push_str(&format!(
        "crop = [{}, {}, {}, {}]\n",
        fmt_f32(item.transform.crop.top),
        fmt_f32(item.transform.crop.right),
        fmt_f32(item.transform.crop.bottom),
        fmt_f32(item.transform.crop.left),
    ));
    out.push_str(&format!("flip_h = {}\n", item.transform.flip_h));
    out.push_str(&format!("flip_v = {}\n", item.transform.flip_v));
    out.push_str(&format!("bounds_mode = {:?}\n", item.transform.bounds_mode));
    out.push_str(&format!(
        "bounds = [{}, {}]\n",
        fmt_f32(item.transform.bounds[0]),
        fmt_f32(item.transform.bounds[1])
    ));
    out.push_str(&format!("blend_mode = {:?}\n", item.blend_mode));
    out.push_str(&format!("opacity = {}\n", fmt_f32(item.opacity)));
    out.push_str(&format!("opacity_space = {:?}\n", item.opacity_space));
    out.push_str(&format!("scaler = {:?}\n", item.scaler));
}

fn deserialize_item(map: &BTreeMap<String, String>, line: usize) -> Result<SceneItem, SceneError> {
    let get = |k: &str| -> Result<&str, SceneError> {
        map.get(k).map(|s| s.as_str()).ok_or_else(|| SceneError::ParseError {
            line,
            message: format!("missing item field {k}"),
        })
    };
    let mut item = SceneItem::new(
        ItemId(parse_u64(get("id")?, line)?),
        SourceId(parse_u64(get("source_id")?, line)?),
        unquote_str(get("name")?, line)?,
    );
    item.visible = parse_bool(get("visible")?, line)?;
    item.locked = parse_bool(get("locked")?, line)?;
    if let Some(pos_raw) = map.get("pos") {
        let p = parse_pair_f32(pos_raw, line)?;
        item.transform.pos = p;
    }
    if let Some(sc_raw) = map.get("scale") {
        item.transform.scale = parse_pair_f32(sc_raw, line)?;
    }
    if let Some(rot_raw) = map.get("rot_deg") {
        item.transform.rot_deg = parse_f32(rot_raw, line)?;
    }
    if let Some(crop_raw) = map.get("crop") {
        let c = parse_rgba(crop_raw, line)?;
        item.transform.crop = CropRect {
            top: c[0],
            right: c[1],
            bottom: c[2],
            left: c[3],
        };
    }
    if let Some(fh) = map.get("flip_h") {
        item.transform.flip_h = parse_bool(fh, line)?;
    }
    if let Some(fv) = map.get("flip_v") {
        item.transform.flip_v = parse_bool(fv, line)?;
    }
    if let Some(bm) = map.get("bounds_mode") {
        item.transform.bounds_mode = match bm.as_str() {
            "Stretch" => BoundsMode::Stretch,
            "Fit" => BoundsMode::Fit,
            "Fill" => BoundsMode::Fill,
            "Cover" => BoundsMode::Cover,
            "ScaleToWidth" => BoundsMode::ScaleToWidth,
            "ScaleToHeight" => BoundsMode::ScaleToHeight,
            "MaxOnly" => BoundsMode::MaxOnly,
            _ => BoundsMode::None,
        };
    }
    if let Some(b_raw) = map.get("bounds") {
        item.transform.bounds = parse_pair_f32(b_raw, line)?;
    }
    if let Some(bm) = map.get("blend_mode") {
        item.blend_mode = match bm.as_str() {
            "Additive" => BlendMode::Additive,
            "Subtract" => BlendMode::Subtract,
            "Screen" => BlendMode::Screen,
            "Multiply" => BlendMode::Multiply,
            "Lighten" => BlendMode::Lighten,
            "Darken" => BlendMode::Darken,
            "LinearBurn" => BlendMode::LinearBurn,
            "Overlay" => BlendMode::Overlay,
            _ => BlendMode::Normal,
        };
    }
    if let Some(op) = map.get("opacity") {
        item.opacity = parse_f32(op, line)?;
    }
    if let Some(sp) = map.get("opacity_space") {
        item.opacity_space = match sp.as_str() {
            "Srgb" => MixColorSpace::Srgb,
            _ => MixColorSpace::Linear,
        };
    }
    if let Some(sc) = map.get("scaler") {
        item.scaler = parse_scaler(sc);
    }
    let _ = (Anchor::TopLeft, VideoFilter::Sharpen { amount: 0.0 });
    Ok(item)
}

fn parse_pair_f32(s: &str, line: usize) -> Result<[f32; 2], SceneError> {
    let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<&str> = trimmed.split(',').collect();
    if parts.len() != 2 {
        return Err(SceneError::ParseError {
            line,
            message: format!("expected 2 floats: {s}"),
        });
    }
    Ok([parse_f32(parts[0], line)?, parse_f32(parts[1], line)?])
}

fn parse_scaler(s: &str) -> Scaler {
    match s.trim() {
        "Point" => Scaler::Point,
        "Bilinear" => Scaler::Bilinear,
        "Bicubic" => Scaler::Bicubic,
        "Lanczos3" => Scaler::Lanczos3,
        "Area" => Scaler::Area,
        _ => Scaler::Auto,
    }
}

fn serialize_transition(tr: &TransitionSpec) -> String {
    let kind_str = match &tr.kind {
        TransitionKind::Cut => String::from("Cut"),
        TransitionKind::Fade => String::from("Fade"),
        TransitionKind::FadeThroughColor { .. } => String::from("FadeThroughColor"),
        TransitionKind::Swipe { .. } => String::from("Swipe"),
        TransitionKind::Slide { .. } => String::from("Slide"),
        TransitionKind::Cover { .. } => String::from("Cover"),
        TransitionKind::Uncover { .. } => String::from("Uncover"),
        TransitionKind::Wipe29 { pattern, .. } => format!("Wipe:{pattern:?}"),
        TransitionKind::Stinger { .. } => String::from("Stinger"),
    };
    format!("{}|{}|{}", quote_str(&tr.name), kind_str, tr.duration_ms)
}

fn deserialize_transition(s: &str, line: usize) -> Result<TransitionSpec, SceneError> {
    let parts: Vec<&str> = s.split('|').collect();
    if parts.len() != 3 {
        return Err(SceneError::ParseError {
            line,
            message: String::from("invalid transition tuple"),
        });
    }
    let name = unquote_str(parts[0], line)?;
    let duration_ms = parse_u32(parts[2], line)?;
    let kind = match parts[1] {
        "Cut" => TransitionKind::Cut,
        "Fade" => TransitionKind::Fade,
        "Swipe" => TransitionKind::Swipe {
            direction: WipeDirection::Left,
            slide_out: false,
        },
        _ => TransitionKind::Wipe29 {
            pattern: WipePattern::Linear,
            softness: 0.05,
            invert: false,
            angle_deg: 0.0,
        },
    };
    Ok(TransitionSpec {
        name,
        kind,
        duration_ms,
    })
}
