// SPDX-License-Identifier: GPL-3.0-only
//! Presentation deck model: slides, aspect ratios, metadata, and slide collections.

use alloc::string::String;
use alloc::vec::Vec;
use crate::elements::{Background, SlideElement, Transition};
use crate::layout::SlideLayout;
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
        }
    }

    pub fn add_element(&mut self, element: SlideElement) {
        self.elements.push(element);
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
}

impl Default for Deck {
    fn default() -> Self {
        Self::new(AspectRatio::Ratio16x9)
    }
}
