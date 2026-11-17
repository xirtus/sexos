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

pub mod animation;
pub mod builds;
pub mod command;
pub mod deck;
pub mod elements;
pub mod keynote;
pub mod layout;
pub mod live_source;
pub mod markdown;
pub mod morph;
pub mod odp;
pub mod ops;
pub mod pdf_export;
pub mod pptx;
pub mod presenter;
pub mod rehearse;
pub mod render;
pub mod review;
pub mod theme;
pub mod xml_utils;

pub use animation::{BuildEffect, BuildStep, BuildTrigger};
pub use builds::{
    add_build_step, delete_slides, duplicate_slides, light_table_cards, move_build_step,
    remove_build_step, render_build_timeline_strip, render_light_table, reorder_slides,
    set_build_effect, set_build_timing, set_build_trigger, set_slides_skipped,
    toggle_slides_skipped, visible_slide_indices, BuildTimeline, BuildTimelineItem, LightTableCard,
    Trigger,
};
pub use command::{Command, CommandStack};
pub use deck::{AspectRatio, Deck, Metadata, Slide};
pub use elements::{
    Background, CodeBlock, FitMode, FontStyle, FontWeight, ImageElement, Paragraph, Shape,
    ShapeKind, SlideElement, StrokeStyle, TableCell, TableElement, TextAlign, TextBox, TextRun,
    Transition,
};
pub use keynote::{import_keynote, import_keynote_with_stats, KeynoteImportStats};
pub use layout::{resolve_placeholders, PlaceholderBox, PlaceholderKind, SlideLayout};
pub use live_source::{LivePresentationSource, PresentationOverlayMode};
pub use markdown::{export_markdown, import_markdown};
pub use morph::{
    compute_morph_plan, evaluate_morph, evaluate_morph_with_easing, interpolate_pair,
    render_deck_morph, render_morph_frame, render_morph_frame_with_options, ElementMeta,
    InterpolatedElementState, MorphEasing, MorphFrameItem, MorphMatchReason, MorphPair, MorphPlan,
};
pub use odp::{export_odp, import_odp};
pub use ops::{
    apply_ops_and_inspect_json, apply_ops_json, inspect_deck_json, ApplyOpsResult, JsonError,
    JsonValue, OpsError,
};
pub use pdf_export::{export_pdf, export_pdf_with_notes};
pub use pptx::{export_pptx, import_pptx};
pub use presenter::{BlankMode, PresenterSession, render_presenter_view};
pub use rehearse::{RehearsalSession, SlideTiming, TimelineFrame, VideoTimeline, render_rehearsal_hud};
pub use render::{
    render_deck_transition, render_magic_move_frame, render_slide, render_slide_at_build,
    render_slide_at_build_with_options, render_transition_canvas, render_transition_frame,
};
pub use review::{
    composite_over, relative_luminance, render_review_toast, review_deck, wcag_contrast_ratio,
    IssueSeverity, ReviewIssue, ReviewIssueKind, ReviewReport,
};
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
