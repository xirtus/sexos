// SPDX-License-Identifier: GPL-3.0-only
//! Burlesque — SexOS presentation engine (Slice T6).
//!
//! Provides slide deck data models, standard layouts, rich elements (text, vector
//! shapes, images, tables, code blocks), undo/redo command stack, OOXML `.pptx`,
//! OpenDocument `.odp`, and Markdown deck import/export, true software rasterizer
//! with Geist fonts, presenter view session state machine, and vector PDF export.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod command;
pub mod deck;
pub mod elements;
pub mod layout;
pub mod markdown;
pub mod odp;
pub mod pdf_export;
pub mod pptx;
pub mod presenter;
pub mod render;
pub mod theme;
pub mod xml_utils;

pub use command::{Command, CommandStack};
pub use deck::{AspectRatio, Deck, Metadata, Slide};
pub use elements::{
    Background, CodeBlock, FitMode, FontStyle, FontWeight, ImageElement, Paragraph, Shape,
    ShapeKind, SlideElement, StrokeStyle, TableCell, TableElement, TextAlign, TextBox, TextRun,
    Transition,
};
pub use layout::{resolve_placeholders, PlaceholderBox, PlaceholderKind, SlideLayout};
pub use markdown::{export_markdown, import_markdown};
pub use odp::import_odp;
pub use pdf_export::export_pdf;
pub use pptx::import_pptx;
pub use presenter::{BlankMode, PresenterSession, render_presenter_view};
pub use render::render_slide;
pub use theme::{Palette, Theme, Typography};

/// Burlesque presentation errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Format signature or magic bytes not recognized.
    InvalidFormat,
    /// Malformed XML, container structure, or broken slides.
    MalformedDocument,
    /// Corrupt compressed archive member.
    CorruptArchive,
    /// General I/O or decode error.
    Io,
    /// Unsupported feature or codec.
    Unsupported(&'static str),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidFormat => write!(f, "invalid presentation file format"),
            Self::MalformedDocument => write!(f, "malformed presentation document"),
            Self::CorruptArchive => write!(f, "corrupted presentation archive container"),
            Self::Io => write!(f, "I/O or decoding error"),
            Self::Unsupported(feat) => write!(f, "unsupported presentation feature: {}", feat),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;
