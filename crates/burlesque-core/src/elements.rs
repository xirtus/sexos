// SPDX-License-Identifier: GPL-3.0-only
//! Rich slide elements: text boxes, shapes, images, tables, code blocks.

use alloc::string::String;
use alloc::vec::Vec;
use sex_canvas::{Color, Rect};
use sex_image::Image;

/// Horizontal text alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
}

impl Default for TextAlign {
    fn default() -> Self {
        Self::Left
    }
}

/// Font weight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontWeight {
    Normal,
    Bold,
}

/// Font style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontStyle {
    Normal,
    Italic,
}

/// A formatted run of text within a paragraph.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub font_size: f32,
    pub color: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
}

impl TextRun {
    pub fn plain(text: impl Into<String>, font_size: f32) -> Self {
        Self {
            text: text.into(),
            font_size,
            color: None,
            bold: false,
            italic: false,
            mono: false,
        }
    }

    pub fn with_color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
}

/// A single paragraph with bullet level, alignment, and text runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Paragraph {
    pub runs: Vec<TextRun>,
    pub bullet_level: u8,
    pub alignment: TextAlign,
    pub line_spacing: f32,
}

impl Paragraph {
    pub fn new() -> Self {
        Self {
            runs: Vec::new(),
            bullet_level: 0,
            alignment: TextAlign::Left,
            line_spacing: 1.25,
        }
    }

    pub fn from_text(text: impl Into<String>, font_size: f32) -> Self {
        Self {
            runs: alloc::vec![TextRun::plain(text, font_size)],
            bullet_level: 0,
            alignment: TextAlign::Left,
            line_spacing: 1.25,
        }
    }

    pub fn bullet(text: impl Into<String>, font_size: f32, level: u8) -> Self {
        Self {
            runs: alloc::vec![TextRun::plain(text, font_size)],
            bullet_level: level,
            alignment: TextAlign::Left,
            line_spacing: 1.25,
        }
    }

    pub fn push_run(&mut self, run: TextRun) {
        self.runs.push(run);
    }

    pub fn plain_text(&self) -> String {
        let mut s = String::new();
        for r in &self.runs {
            s.push_str(&r.text);
        }
        s
    }
}

impl Default for Paragraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Text box slide element.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBox {
    pub bounds: Rect,
    pub paragraphs: Vec<Paragraph>,
    pub background: Option<Color>,
    pub padding: f32,
}

impl TextBox {
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            paragraphs: Vec::new(),
            background: None,
            padding: 8.0,
        }
    }

    pub fn with_paragraph(mut self, p: Paragraph) -> Self {
        self.paragraphs.push(p);
        self
    }

    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for (i, p) in self.paragraphs.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&p.plain_text());
        }
        out
    }
}

/// Geometric shape kinds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShapeKind {
    Rect,
    RoundedRect { radius: f32 },
    Ellipse,
    Line,
    Arrow,
}

/// Outline stroke styling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    pub color: Color,
    pub width: f32,
}

/// Shape slide element.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub kind: ShapeKind,
    pub bounds: Rect,
    pub fill: Option<Color>,
    pub stroke: Option<StrokeStyle>,
}

impl Shape {
    pub fn rect(bounds: Rect, fill: Option<Color>, stroke: Option<StrokeStyle>) -> Self {
        Self {
            kind: ShapeKind::Rect,
            bounds,
            fill,
            stroke,
        }
    }

    pub fn rounded_rect(bounds: Rect, radius: f32, fill: Option<Color>, stroke: Option<StrokeStyle>) -> Self {
        Self {
            kind: ShapeKind::RoundedRect { radius },
            bounds,
            fill,
            stroke,
        }
    }

    pub fn ellipse(bounds: Rect, fill: Option<Color>, stroke: Option<StrokeStyle>) -> Self {
        Self {
            kind: ShapeKind::Ellipse,
            bounds,
            fill,
            stroke,
        }
    }

    pub fn line(x0: f32, y0: f32, x1: f32, y1: f32, stroke: StrokeStyle) -> Self {
        let x = x0.min(x1);
        let y = y0.min(y1);
        let w = (x1 - x0).abs().max(1.0);
        let h = (y1 - y0).abs().max(1.0);
        Self {
            kind: ShapeKind::Line,
            bounds: Rect::new(x, y, w, h),
            fill: None,
            stroke: Some(stroke),
        }
    }

    pub fn arrow(x0: f32, y0: f32, x1: f32, y1: f32, stroke: StrokeStyle) -> Self {
        let x = x0.min(x1);
        let y = y0.min(y1);
        let w = (x1 - x0).abs().max(1.0);
        let h = (y1 - y0).abs().max(1.0);
        Self {
            kind: ShapeKind::Arrow,
            bounds: Rect::new(x, y, w, h),
            fill: Some(stroke.color),
            stroke: Some(stroke),
        }
    }
}

/// Image fit mode inside target bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitMode {
    Contain,
    Cover,
    Stretch,
}

impl Default for FitMode {
    fn default() -> Self {
        Self::Contain
    }
}

/// Image slide element.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageElement {
    pub bounds: Rect,
    pub data: Vec<u8>,
    pub fit: FitMode,
    pub decoded: Option<Image>,
}

impl ImageElement {
    pub fn new(bounds: Rect, data: Vec<u8>, fit: FitMode) -> Self {
        let decoded = sex_image::decode(&data, &sex_image::Limits::PHOTO).ok();
        Self {
            bounds,
            data,
            fit,
            decoded,
        }
    }

    pub fn from_image(bounds: Rect, image: Image, fit: FitMode) -> Self {
        Self {
            bounds,
            data: Vec::new(),
            fit,
            decoded: Some(image),
        }
    }

    pub fn ensure_decoded(&mut self) -> Option<&Image> {
        if self.decoded.is_none() && !self.data.is_empty() {
            self.decoded = sex_image::decode(&self.data, &sex_image::Limits::PHOTO).ok();
        }
        self.decoded.as_ref()
    }
}

/// Single table cell.
#[derive(Clone, Debug, PartialEq)]
pub struct TableCell {
    pub text: String,
    pub bold: bool,
    pub alignment: TextAlign,
    pub text_color: Option<Color>,
    pub background_color: Option<Color>,
}

impl TableCell {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bold: false,
            alignment: TextAlign::Left,
            text_color: None,
            background_color: None,
        }
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn align(mut self, alignment: TextAlign) -> Self {
        self.alignment = alignment;
        self
    }
}

/// Table slide element.
#[derive(Clone, Debug, PartialEq)]
pub struct TableElement {
    pub bounds: Rect,
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<Vec<TableCell>>,
    pub has_header_row: bool,
    pub header_background: Option<Color>,
    pub border_color: Color,
    pub border_width: f32,
}

impl TableElement {
    pub fn new(bounds: Rect, rows: usize, cols: usize) -> Self {
        let rows = rows.min(100);
        let cols = cols.min(100);
        let mut cells = Vec::with_capacity(rows);
        for _ in 0..rows {
            let mut row = Vec::with_capacity(cols);
            for _ in 0..cols {
                row.push(TableCell::new(""));
            }
            cells.push(row);
        }
        Self {
            bounds,
            rows,
            cols,
            cells,
            has_header_row: true,
            header_background: Some(Color::rgb(0x33, 0x41, 0x55)),
            border_color: Color::rgb(0x64, 0x74, 0x8B),
            border_width: 1.5,
        }
    }

    pub fn set_cell(&mut self, row: usize, col: usize, cell: TableCell) {
        if row < self.rows && col < self.cols {
            self.cells[row][col] = cell;
        }
    }
}

/// Code block slide element.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeBlock {
    pub bounds: Rect,
    pub code: String,
    pub language: Option<String>,
    pub font_size: f32,
    pub background: Color,
    pub text_color: Color,
    pub show_line_numbers: bool,
}

impl CodeBlock {
    pub fn new(bounds: Rect, code: impl Into<String>) -> Self {
        Self {
            bounds,
            code: code.into(),
            language: None,
            font_size: 20.0,
            background: Color::rgb(0x1E, 0x29, 0x3B),
            text_color: Color::rgb(0xF8, 0xFA, 0xFC),
            show_line_numbers: true,
        }
    }

    pub fn with_language(mut self, lang: impl Into<String>) -> Self {
        self.language = Some(lang.into());
        self
    }
}

/// Unified slide element enum.
#[derive(Clone, Debug, PartialEq)]
pub enum SlideElement {
    TextBox(TextBox),
    Shape(Shape),
    Image(ImageElement),
    Table(TableElement),
    Code(CodeBlock),
}

impl SlideElement {
    pub fn bounds(&self) -> Rect {
        match self {
            SlideElement::TextBox(tb) => tb.bounds,
            SlideElement::Shape(s) => s.bounds,
            SlideElement::Image(img) => img.bounds,
            SlideElement::Table(t) => t.bounds,
            SlideElement::Code(c) => c.bounds,
        }
    }

    pub fn set_bounds(&mut self, bounds: Rect) {
        match self {
            SlideElement::TextBox(tb) => tb.bounds = bounds,
            SlideElement::Shape(s) => s.bounds = bounds,
            SlideElement::Image(img) => img.bounds = bounds,
            SlideElement::Table(t) => t.bounds = bounds,
            SlideElement::Code(c) => c.bounds = bounds,
        }
    }
}

/// Slide transitions supported by the burlesque presentations engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    None,
    Dissolve,
    PushLeft,
    PushRight,
    WipeRight,
    SlideUp,
    ZoomIn,
    Fade,
    SlideLeft,
}

impl Transition {
    /// Default transition playback duration in milliseconds.
    pub const DEFAULT_DURATION_MS: u32 = 400;

    /// Return nominal animation duration in milliseconds.
    pub fn duration_ms(&self) -> u32 {
        match self {
            Self::None => 0,
            _ => Self::DEFAULT_DURATION_MS,
        }
    }

    /// User-visible name of the transition.
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Dissolve => "Dissolve",
            Self::PushLeft => "PushLeft",
            Self::PushRight => "PushRight",
            Self::WipeRight => "WipeRight",
            Self::SlideUp => "SlideUp",
            Self::ZoomIn => "ZoomIn",
            Self::Fade => "Fade",
            Self::SlideLeft => "SlideLeft",
        }
    }

    /// Short serialized kind identifier.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Dissolve => "dissolve",
            Self::PushLeft => "push_left",
            Self::PushRight => "push_right",
            Self::WipeRight => "wipe_right",
            Self::SlideUp => "slide_up",
            Self::ZoomIn => "zoom_in",
            Self::Fade => "fade",
            Self::SlideLeft => "slide_left",
        }
    }

    /// Parse transition name or loose string keyword.
    pub fn from_str_loose(s: &str) -> Self {
        let clean = s.trim();
        if clean.eq_ignore_ascii_case("dissolve") {
            Self::Dissolve
        } else if clean.eq_ignore_ascii_case("push_left") || clean.eq_ignore_ascii_case("pushleft") || clean.eq_ignore_ascii_case("push") {
            Self::PushLeft
        } else if clean.eq_ignore_ascii_case("push_right") || clean.eq_ignore_ascii_case("pushright") {
            Self::PushRight
        } else if clean.eq_ignore_ascii_case("wipe_right") || clean.eq_ignore_ascii_case("wiperight") || clean.eq_ignore_ascii_case("wipe") {
            Self::WipeRight
        } else if clean.eq_ignore_ascii_case("slide_up") || clean.eq_ignore_ascii_case("slideup") {
            Self::SlideUp
        } else if clean.eq_ignore_ascii_case("zoom_in") || clean.eq_ignore_ascii_case("zoomin") || clean.eq_ignore_ascii_case("zoom") {
            Self::ZoomIn
        } else if clean.eq_ignore_ascii_case("fade") {
            Self::Fade
        } else if clean.eq_ignore_ascii_case("slide_left") || clean.eq_ignore_ascii_case("slideleft") {
            Self::SlideLeft
        } else {
            Self::None
        }
    }
}

impl Default for Transition {
    fn default() -> Self {
        Self::None
    }
}

/// Slide background style override.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    Solid(Color),
    Gradient(Color, Color),
}
