// SPDX-License-Identifier: GPL-3.0-only
//! Presentation deck model: slides, aspect ratios, metadata, and slide collections.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use crate::animation::{BuildStep, BuildTrigger};
use crate::elements::{Background, SlideElement, Transition};
use crate::layout::SlideLayout;
use crate::morph::ElementMeta;
use crate::theme::Theme;
use crate::Result;

/// Deck aspect ratio and logical canvas dimension.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AspectRatio {
    Ratio16x9,
    Ratio16x10,
    Ratio4x3,
    Custom { width: f32, height: f32 },
}

impl AspectRatio {
    pub fn width(&self) -> f32 {
        match self {
            Self::Ratio16x9 => 1920.0,
            Self::Ratio16x10 => 1920.0,
            Self::Ratio4x3 => 1440.0,
            Self::Custom { width, .. } => if width.is_finite() && *width > 0.0 { *width } else { 1920.0 },
        }
    }

    pub fn height(&self) -> f32 {
        match self {
            Self::Ratio16x9 => 1080.0,
            Self::Ratio16x10 => 1200.0,
            Self::Ratio4x3 => 1080.0,
            Self::Custom { height, .. } => if height.is_finite() && *height > 0.0 { *height } else { 1080.0 },
        }
    }

    pub fn ratio(&self) -> f32 {
        let h = self.height();
        if h > 0.0 && h.is_finite() {
            let w = self.width();
            if w > 0.0 && w.is_finite() {
                w / h
            } else {
                16.0 / 9.0
            }
        } else {
            16.0 / 9.0
        }
    }
}

impl Default for AspectRatio {
    fn default() -> Self {
        Self::Ratio16x9
    }
}

/// Deck metadata.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub title: String,
    pub author: String,
    pub date: String,
    pub description: String,
}

/// A single presentation slide.
#[derive(Clone, Debug, PartialEq)]
pub struct Slide {
    pub id: u64,
    pub title: String,
    pub layout: SlideLayout,
    pub elements: Vec<SlideElement>,
    pub speaker_notes: String,
    pub notes: String,
    pub transition: Transition,
    pub transition_duration_ms: u32,
    pub hidden: bool,
    pub background_override: Option<Background>,
    pub builds: Vec<BuildStep>,
    pub rehearsed_dwell_ms: Option<u32>,
    pub element_meta: BTreeMap<usize, ElementMeta>,
    pub build_delays: BTreeMap<usize, u32>,
}

impl Slide {
    pub fn new(id: u64, layout: SlideLayout) -> Self {
        Self {
            id,
            title: String::new(),
            layout,
            elements: Vec::new(),
            speaker_notes: String::new(),
            notes: String::new(),
            transition: Transition::None,
            transition_duration_ms: 0,
            hidden: false,
            background_override: None,
            builds: Vec::new(),
            rehearsed_dwell_ms: None,
            element_meta: BTreeMap::new(),
            build_delays: BTreeMap::new(),
        }
    }

    pub fn add_element(&mut self, element: SlideElement) {
        self.elements.push(element);
    }

    /// Add an element together with explicit morph/transform metadata (`element_id`, `morph_key`, `rotation_deg`, `opacity`).
    pub fn add_element_with_meta(&mut self, mut element: SlideElement, meta: ElementMeta) -> usize {
        if let Some(ref mk) = meta.morph_key {
            if element.magic_key().is_none() {
                element.set_magic_key(Some(mk.clone()));
            }
        }
        let idx = self.elements.len();
        self.elements.push(element);
        self.element_meta.insert(idx, meta);
        idx
    }

    /// Remove an element and shift per-element metadata keys cleanly.
    pub fn remove_element(&mut self, index: usize) -> Option<SlideElement> {
        if index >= self.elements.len() {
            return None;
        }
        let elem = self.elements.remove(index);
        let old_meta = core::mem::take(&mut self.element_meta);
        for (k, v) in old_meta {
            if k < index {
                self.element_meta.insert(k, v);
            } else if k > index {
                self.element_meta.insert(k - 1, v);
            }
        }
        Some(elem)
    }

    pub fn element_meta(&self, index: usize) -> ElementMeta {
        let mut meta = self.element_meta.get(&index).cloned().unwrap_or_default();
        if meta.morph_key.is_none() {
            if let Some(el) = self.elements.get(index) {
                if let Some(mk) = el.magic_key() {
                    meta.morph_key = Some(String::from(mk));
                }
            }
        }
        meta
    }

    pub fn set_element_meta(&mut self, index: usize, meta: ElementMeta) {
        if let Some(ref mk) = meta.morph_key {
            if let Some(el) = self.elements.get_mut(index) {
                el.set_magic_key(Some(mk.clone()));
            }
        }
        self.element_meta.insert(index, meta);
    }

    pub fn set_element_rotation(&mut self, index: usize, rotation_deg: f32) {
        let mut meta = self.element_meta(index);
        meta.rotation_deg = if rotation_deg.is_finite() { rotation_deg } else { 0.0 };
        self.element_meta.insert(index, meta);
    }

    pub fn element_rotation(&self, index: usize) -> f32 {
        self.element_meta.get(&index).map(|m| m.rotation_deg).unwrap_or(0.0)
    }

    pub fn set_element_opacity(&mut self, index: usize, opacity: f32) {
        let mut meta = self.element_meta(index);
        meta.opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
        self.element_meta.insert(index, meta);
    }

    pub fn element_opacity(&self, index: usize) -> f32 {
        self.element_meta.get(&index).map(|m| m.opacity).unwrap_or(1.0)
    }

    pub fn set_element_id(&mut self, index: usize, id: Option<String>) {
        let mut meta = self.element_meta(index);
        meta.element_id = id;
        self.element_meta.insert(index, meta);
    }

    pub fn element_id(&self, index: usize) -> Option<&str> {
        self.element_meta.get(&index).and_then(|m| m.element_id.as_deref())
    }

    pub fn set_element_morph_key(&mut self, index: usize, key: Option<String>) {
        if let Some(el) = self.elements.get_mut(index) {
            el.set_magic_key(key.clone());
        }
        let mut meta = self.element_meta(index);
        meta.morph_key = key;
        self.element_meta.insert(index, meta);
    }

    pub fn element_morph_key(&self, index: usize) -> Option<&str> {
        self.element_meta
            .get(&index)
            .and_then(|m| m.morph_key.as_deref())
            .or_else(|| self.elements.get(index).and_then(|e| e.magic_key()))
    }

    pub fn add_build(&mut self, build: BuildStep) {
        self.builds.push(build);
    }

    pub fn builds(&self) -> &[BuildStep] {
        &self.builds
    }

    pub fn build_delay_ms(&self, build_index: usize) -> u32 {
        if let Some(&d) = self.build_delays.get(&build_index) {
            return d;
        }
        if let Some(step) = self.builds.get(build_index) {
            if let BuildTrigger::AfterPrevious { delay_ms } = step.trigger {
                return delay_ms;
            }
        }
        0
    }

    pub fn set_build_delay_ms(&mut self, build_index: usize, delay_ms: u32) {
        if delay_ms == 0 {
            self.build_delays.remove(&build_index);
        } else {
            self.build_delays.insert(build_index, delay_ms);
        }
        if let Some(step) = self.builds.get_mut(build_index) {
            if let BuildTrigger::AfterPrevious { .. } = step.trigger {
                step.trigger = BuildTrigger::AfterPrevious { delay_ms };
            }
        }
    }

    pub fn build_timeline(&self) -> crate::builds::BuildTimeline {
        crate::builds::BuildTimeline::from_slide(self)
    }

    pub fn set_speaker_notes(&mut self, notes: impl Into<String>) {
        let n = notes.into();
        self.notes = n.clone();
        self.speaker_notes = n;
    }

    pub fn set_notes(&mut self, notes: impl Into<String>) {
        self.set_speaker_notes(notes);
    }

    pub fn notes(&self) -> &str {
        if !self.notes.is_empty() {
            &self.notes
        } else {
            &self.speaker_notes
        }
    }

    pub fn transition_duration_ms(&self) -> u32 {
        if self.transition_duration_ms > 0 {
            self.transition_duration_ms
        } else {
            self.transition.duration_ms()
        }
    }
}

/// Presentation deck holding slides, theme, aspect ratio, and metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct Deck {
    pub metadata: Metadata,
    pub aspect_ratio: AspectRatio,
    pub theme: Theme,
    pub slides: Vec<Slide>,
    next_slide_id: u64,
}

impl Deck {
    pub fn new(aspect_ratio: AspectRatio) -> Self {
        Self {
            metadata: Metadata::default(),
            aspect_ratio,
            theme: Theme::burlesque_velvet(),
            slides: Vec::new(),
            next_slide_id: 1,
        }
    }

    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub fn width(&self) -> f32 {
        self.aspect_ratio.width()
    }

    pub fn height(&self) -> f32 {
        self.aspect_ratio.height()
    }

    pub fn slide_count(&self) -> usize {
        self.slides.len()
    }

    pub fn add_slide(&mut self, layout: SlideLayout) -> &mut Slide {
        let id = self.next_slide_id;
        self.next_slide_id += 1;
        let slide = Slide::new(id, layout);
        self.slides.push(slide);
        self.slides.last_mut().expect("slide just pushed")
    }

    pub fn insert_slide(&mut self, index: usize, mut slide: Slide) {
        if slide.id == 0 {
            slide.id = self.next_slide_id;
            self.next_slide_id += 1;
        } else if slide.id >= self.next_slide_id {
            self.next_slide_id = slide.id + 1;
        }
        let pos = index.min(self.slides.len());
        self.slides.insert(pos, slide);
    }

    pub fn remove_slide(&mut self, index: usize) -> Option<Slide> {
        if index < self.slides.len() {
            Some(self.slides.remove(index))
        } else {
            None
        }
    }

    pub fn move_slide(&mut self, from: usize, to: usize) -> bool {
        if from >= self.slides.len() || to >= self.slides.len() || from == to {
            return false;
        }
        let slide = self.slides.remove(from);
        self.slides.insert(to, slide);
        true
    }

    /// Duplicate a single slide right after `index`, returning the new slide's index.
    pub fn duplicate_slide(&mut self, index: usize) -> Option<usize> {
        let created = crate::builds::duplicate_slides(self, &[index]);
        created.first().copied()
    }

    /// Duplicate multiple slides in Light-Table mode.
    pub fn duplicate_slides(&mut self, indices: &[usize]) -> Vec<usize> {
        crate::builds::duplicate_slides(self, indices)
    }

    /// Reorder multiple slides as a contiguous block before `target_index`.
    pub fn reorder_slides(&mut self, indices: &[usize], target_index: usize) -> Vec<usize> {
        crate::builds::reorder_slides(self, indices, target_index)
    }

    /// Set or clear the skipped (`hidden`) flag across multiple slides.
    pub fn set_slides_skipped(&mut self, indices: &[usize], skipped: bool) -> usize {
        crate::builds::set_slides_skipped(self, indices, skipped)
    }

    /// Return indices of all non-skipped slides.
    pub fn visible_slide_indices(&self) -> Vec<usize> {
        crate::builds::visible_slide_indices(self)
    }

    pub fn get_slide(&self, index: usize) -> Option<&Slide> {
        self.slides.get(index)
    }

    pub fn get_slide_mut(&mut self, index: usize) -> Option<&mut Slide> {
        self.slides.get_mut(index)
    }

    /// Import a PPTX presentation container from raw bytes.
    pub fn open_pptx(bytes: &[u8]) -> Result<Self> {
        crate::pptx::import_pptx(bytes)
    }

    /// Export this deck to an OOXML PPTX container.
    pub fn export_pptx(&self) -> Result<Vec<u8>> {
        crate::pptx::export_pptx(self)
    }

    /// Import an OpenDocument Presentation (ODP) from raw bytes.
    pub fn open_odp(bytes: &[u8]) -> Result<Self> {
        crate::odp::import_odp(bytes)
    }

    /// Export this deck to an OpenDocument Presentation (ODP) container.
    pub fn export_odp(&self) -> Result<Vec<u8>> {
        crate::odp::export_odp(self)
    }

    /// Import an Apple Keynote (`.key`) ZIP bundle from raw bytes.
    pub fn open_keynote(bytes: &[u8]) -> Result<Self> {
        crate::keynote::import_keynote(bytes)
    }

    /// Import a Markdown presentation.
    pub fn from_markdown(md: &str) -> Result<Self> {
        crate::markdown::import_markdown(md)
    }

    /// Export this deck to Markdown.
    pub fn to_markdown(&self) -> String {
        crate::markdown::export_markdown(self)
    }

    /// Export this deck to a PDF-1.4 multi-page document.
    pub fn export_pdf(&self) -> Result<Vec<u8>> {
        crate::pdf_export::export_pdf(self)
    }

    /// Export this deck to a PDF-1.4 document, optionally appending speaker notes.
    pub fn export_pdf_with_notes(&self, include_notes: bool) -> Result<Vec<u8>> {
        crate::pdf_export::export_pdf_with_notes(self, include_notes)
    }

    /// Run headless lint and WCAG contrast checks on this deck.
    pub fn review(&self) -> crate::review::ReviewReport {
        crate::review::review_deck(self)
    }

    /// Produce a deterministic JSON inspection snapshot of this deck.
    pub fn inspect_json(&self) -> String {
        crate::ops::inspect_deck_json(self)
    }

    /// Atomically apply OmaShow-style headless JSON ops to this deck.
    pub fn apply_ops_json(
        &mut self,
        ops_json: &str,
    ) -> core::result::Result<crate::ops::ApplyOpsResult, crate::ops::OpsError> {
        crate::ops::apply_ops_json(self, ops_json)
    }
}

impl Default for Deck {
    fn default() -> Self {
        Self::new(AspectRatio::Ratio16x9)
    }
}
