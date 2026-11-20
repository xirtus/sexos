// SPDX-License-Identifier: GPL-3.0-only
//! Headless Deck Lint & Accessibility Review (`review_deck`).
//!
//! Checks:
//! - Slide bounds overflow (elements extending outside canvas or invalid non-positive size)
//! - Low text/background WCAG 2.1 contrast ratio (< 4.5:1 normal text, < 3.0:1 large text)
//! - Empty placeholders (empty TextBox / CodeBlock / Table or empty non-Blank slide)
//! - Missing or undecodable images (`ImageElement` without valid decoded image data)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_canvas::{Canvas, Color, Rect};
use sex_font::{Font, GEIST_SANS};

use crate::deck::Deck;
use crate::elements::{Background, SlideElement};
use crate::layout::SlideLayout;
use crate::render::draw_text_run;

/// Severity level of a deck review issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueSeverity {
    Info,
    Warning,
    Error,
}

impl IssueSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// Specific lint check that flagged an issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewIssueKind {
    BoundsOverflow,
    LowContrast,
    EmptyPlaceholder,
    MissingImage,
    EmptyDeck,
}

impl ReviewIssueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BoundsOverflow => "bounds_overflow",
            Self::LowContrast => "low_contrast",
            Self::EmptyPlaceholder => "empty_placeholder",
            Self::MissingImage => "missing_image",
            Self::EmptyDeck => "empty_deck",
        }
    }
}

/// Single diagnostic produced by [`review_deck`].
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewIssue {
    pub severity: IssueSeverity,
    pub kind: ReviewIssueKind,
    pub slide_index: usize,
    pub element_index: Option<usize>,
    pub message: String,
    pub contrast_ratio: Option<f32>,
}

/// Aggregated deck review report.
#[derive(Clone, Debug, PartialEq)]
pub struct ReviewReport {
    pub slide_count: usize,
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
    pub issues: Vec<ReviewIssue>,
}

impl ReviewReport {
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        self.error_count > 0
    }

    /// One-line summary suitable for UI toast banners or status lines.
    pub fn summary_line(&self) -> String {
        if self.issues.is_empty() {
            format!("Review OK: {} slide(s), 0 issues", self.slide_count)
        } else if let Some(first) = self.issues.first() {
            format!(
                "Review: {} err, {} warn — S{}: {}",
                self.error_count,
                self.warning_count,
                first.slide_index + 1,
                first.message
            )
        } else {
            format!(
                "Review: {} error(s), {} warning(s)",
                self.error_count, self.warning_count
            )
        }
    }

    /// Serialize this report to deterministic compact JSON.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        out.push_str("{\"ok\":");
        out.push_str(if self.issues.is_empty() { "true" } else { "false" });
        out.push_str(&format!(
            ",\"slide_count\":{},\"error_count\":{},\"warning_count\":{},\"info_count\":{},\"issues\":[",
            self.slide_count, self.error_count, self.warning_count, self.info_count
        ));
        for (i, issue) in self.issues.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"severity\":\"");
            out.push_str(issue.severity.as_str());
            out.push_str("\",\"kind\":\"");
            out.push_str(issue.kind.as_str());
            out.push_str(&format!("\",\"slide_index\":{},\"element_index\":", issue.slide_index));
            match issue.element_index {
                Some(ei) => out.push_str(&format!("{}", ei)),
                None => out.push_str("null"),
            }
            out.push_str(",\"contrast_ratio\":");
            match issue.contrast_ratio {
                Some(cr) => out.push_str(&format!("{:.2}", cr)),
                None => out.push_str("null"),
            }
            out.push_str(",\"message\":");
            write_escaped_json_str(&issue.message, &mut out);
            out.push('}');
        }
        out.push_str("]}");
        out
    }
}

/// Compute WCAG 2.1 relative luminance for an sRGB color in `[0.0, 1.0]`.
pub fn relative_luminance(c: Color) -> f32 {
    fn channel(v: u8) -> f32 {
        let s = v as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            libm::powf((s + 0.055) / 1.055, 2.4)
        }
    }
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

/// Alpha-composite `fg` over opaque `bg` to obtain the effective rendered foreground color.
pub fn composite_over(fg: Color, bg: Color) -> Color {
    let a = fg.a() as f32 / 255.0;
    if a >= 0.999 {
        return Color::rgb(fg.r(), fg.g(), fg.b());
    }
    let inv = 1.0 - a;
    let r = (fg.r() as f32 * a + bg.r() as f32 * inv + 0.5).clamp(0.0, 255.0) as u8;
    let g = (fg.g() as f32 * a + bg.g() as f32 * inv + 0.5).clamp(0.0, 255.0) as u8;
    let b = (fg.b() as f32 * a + bg.b() as f32 * inv + 0.5).clamp(0.0, 255.0) as u8;
    Color::rgb(r, g, b)
}

/// Compute WCAG 2.1 contrast ratio in `[1.0, 21.0]` between foreground and background.
pub fn wcag_contrast_ratio(fg: Color, bg: Color) -> f32 {
    let eff_fg = composite_over(fg, bg);
    let l1 = relative_luminance(eff_fg);
    let l2 = relative_luminance(bg);
    let (hi, lo) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

fn slide_bg_samples(bg: Option<Background>, theme_bg: Color) -> (Color, Option<Color>) {
    match bg {
        Some(Background::Solid(c)) => (c, None),
        Some(Background::Gradient(c1, c2)) => (c1, Some(c2)),
        None => (theme_bg, None),
    }
}

fn worst_contrast_against_slide(fg: Color, elem_bg: Option<Color>, slide_bg: (Color, Option<Color>)) -> f32 {
    let eval = |base_bg: Color| -> f32 {
        let effective_bg = match elem_bg {
            Some(eb) => composite_over(eb, base_bg),
            None => base_bg,
        };
        wcag_contrast_ratio(fg, effective_bg)
    };
    let c1 = eval(slide_bg.0);
    if let Some(b2) = slide_bg.1 {
        c1.min(eval(b2))
    } else {
        c1
    }
}

fn is_placeholder_sentinel(text: &str) -> bool {
    let clean = text.trim().to_ascii_lowercase();
    clean.is_empty()
        || clean == "click to add title"
        || clean == "click to add subtitle"
        || clean == "click to add text"
        || clean == "[placeholder]"
}

/// Lint a presentation deck for layout overflow, WCAG contrast, empty placeholders, and missing images.
pub fn review_deck(deck: &Deck) -> ReviewReport {
    let mut issues = Vec::new();
    let slide_w = deck.width();
    let slide_h = deck.height();

    if deck.slides.is_empty() {
        issues.push(ReviewIssue {
            severity: IssueSeverity::Warning,
            kind: ReviewIssueKind::EmptyDeck,
            slide_index: 0,
            element_index: None,
            message: String::from("Deck has no slides"),
            contrast_ratio: None,
        });
    }

    for (slide_idx, slide) in deck.slides.iter().enumerate() {
        if slide.elements.is_empty() && slide.title.trim().is_empty() && slide.layout != SlideLayout::Blank {
            issues.push(ReviewIssue {
                severity: IssueSeverity::Warning,
                kind: ReviewIssueKind::EmptyPlaceholder,
                slide_index: slide_idx,
                element_index: None,
                message: String::from("Slide has no title or elements"),
                contrast_ratio: None,
            });
        }

        let slide_bg = slide_bg_samples(slide.background_override, deck.theme.palette.background);

        for (elem_idx, elem) in slide.elements.iter().enumerate() {
            let b = elem.bounds();

            // 1. Slide bounds overflow check
            if !b.x.is_finite() || !b.y.is_finite() || !b.w.is_finite() || !b.h.is_finite()
                || b.w <= 0.0
                || b.h <= 0.0
            {
                issues.push(ReviewIssue {
                    severity: IssueSeverity::Error,
                    kind: ReviewIssueKind::BoundsOverflow,
                    slide_index: slide_idx,
                    element_index: Some(elem_idx),
                    message: format!("Element {} has invalid/non-positive dimensions ({:.1}x{:.1})", elem_idx, b.w, b.h),
                    contrast_ratio: None,
                });
            } else if b.x < -0.5 || b.y < -0.5 || b.right() > slide_w + 0.5 || b.bottom() > slide_h + 0.5 {
                issues.push(ReviewIssue {
                    severity: IssueSeverity::Warning,
                    kind: ReviewIssueKind::BoundsOverflow,
                    slide_index: slide_idx,
                    element_index: Some(elem_idx),
                    message: format!(
                        "Element {} overflows slide canvas ({:.0},{:.0}..{:.0},{:.0} vs {:.0}x{:.0})",
                        elem_idx, b.x, b.y, b.right(), b.bottom(), slide_w, slide_h
                    ),
                    contrast_ratio: None,
                });
            }

            // 2. Element-specific checks
            match elem {
                SlideElement::TextBox(tb) => {
                    let text = tb.plain_text();
                    if is_placeholder_sentinel(&text) {
                        issues.push(ReviewIssue {
                            severity: IssueSeverity::Warning,
                            kind: ReviewIssueKind::EmptyPlaceholder,
                            slide_index: slide_idx,
                            element_index: Some(elem_idx),
                            message: format!("TextBox {} is empty or unedited placeholder", elem_idx),
                            contrast_ratio: None,
                        });
                    } else {
                        let mut worst_cr: Option<(f32, f32)> = None;
                        for p in &tb.paragraphs {
                            for r in &p.runs {
                                if r.text.trim().is_empty() {
                                    continue;
                                }
                                let fg = r.color.unwrap_or(deck.theme.palette.foreground);
                                let cr = worst_contrast_against_slide(fg, tb.background, slide_bg);
                                let min_req = if r.font_size >= 24.0 || (r.bold && r.font_size >= 18.66) {
                                    3.0
                                } else {
                                    4.5
                                };
                                if cr < min_req {
                                    if worst_cr.map_or(true, |(prev, _)| cr < prev) {
                                        worst_cr = Some((cr, min_req));
                                    }
                                }
                            }
                        }
                        if let Some((cr, req)) = worst_cr {
                            issues.push(ReviewIssue {
                                severity: IssueSeverity::Warning,
                                kind: ReviewIssueKind::LowContrast,
                                slide_index: slide_idx,
                                element_index: Some(elem_idx),
                                message: format!(
                                    "Low WCAG text contrast {:.2}:1 (minimum {:.1}:1 required)",
                                    cr, req
                                ),
                                contrast_ratio: Some(cr),
                            });
                        }
                    }
                }
                SlideElement::Code(cb) => {
                    if cb.code.trim().is_empty() {
                        issues.push(ReviewIssue {
                            severity: IssueSeverity::Warning,
                            kind: ReviewIssueKind::EmptyPlaceholder,
                            slide_index: slide_idx,
                            element_index: Some(elem_idx),
                            message: format!("CodeBlock {} is empty", elem_idx),
                            contrast_ratio: None,
                        });
                    } else {
                        let cr = wcag_contrast_ratio(cb.text_color, cb.background);
                        let req = if cb.font_size >= 24.0 { 3.0 } else { 4.5 };
                        if cr < req {
                            issues.push(ReviewIssue {
                                severity: IssueSeverity::Warning,
                                kind: ReviewIssueKind::LowContrast,
                                slide_index: slide_idx,
                                element_index: Some(elem_idx),
                                message: format!(
                                    "Low WCAG code contrast {:.2}:1 (minimum {:.1}:1 required)",
                                    cr, req
                                ),
                                contrast_ratio: Some(cr),
                            });
                        }
                    }
                }
                SlideElement::Table(tbl) => {
                    let all_empty = tbl.rows == 0
                        || tbl.cols == 0
                        || tbl.cells.iter().all(|row| row.iter().all(|c| c.text.trim().is_empty()));
                    if all_empty {
                        issues.push(ReviewIssue {
                            severity: IssueSeverity::Warning,
                            kind: ReviewIssueKind::EmptyPlaceholder,
                            slide_index: slide_idx,
                            element_index: Some(elem_idx),
                            message: format!("Table {} has no content", elem_idx),
                            contrast_ratio: None,
                        });
                    } else {
                        let mut worst_cr: Option<f32> = None;
                        for (r_idx, row) in tbl.cells.iter().enumerate() {
                            for cell in row {
                                if cell.text.trim().is_empty() {
                                    continue;
                                }
                                let is_hdr = r_idx == 0 && tbl.has_header_row;
                                let fg = cell.text_color.unwrap_or(if is_hdr {
                                    Color::WHITE
                                } else {
                                    deck.theme.palette.foreground
                                });
                                let cell_bg = cell.background_color.or(if is_hdr {
                                    tbl.header_background
                                } else {
                                    None
                                });
                                let cr = worst_contrast_against_slide(fg, cell_bg, slide_bg);
                                if cr < 4.5 && worst_cr.map_or(true, |p| cr < p) {
                                    worst_cr = Some(cr);
                                }
                            }
                        }
                        if let Some(cr) = worst_cr {
                            issues.push(ReviewIssue {
                                severity: IssueSeverity::Warning,
                                kind: ReviewIssueKind::LowContrast,
                                slide_index: slide_idx,
                                element_index: Some(elem_idx),
                                message: format!("Low WCAG table cell contrast {:.2}:1 (minimum 4.5:1)", cr),
                                contrast_ratio: Some(cr),
                            });
                        }
                    }
                }
                SlideElement::Image(img) => {
                    let valid_decoded = match &img.decoded {
                        Some(d) => d.width > 0 && d.height > 0 && d.pixels.len() >= d.width * d.height,
                        None => {
                            !img.data.is_empty()
                                && sex_image::decode(&img.data, &sex_image::Limits::PHOTO).is_ok()
                        }
                    };
                    if !valid_decoded {
                        issues.push(ReviewIssue {
                            severity: IssueSeverity::Error,
                            kind: ReviewIssueKind::MissingImage,
                            slide_index: slide_idx,
                            element_index: Some(elem_idx),
                            message: format!("Image {} has missing or undecodable image data", elem_idx),
                            contrast_ratio: None,
                        });
                    }
                }
                SlideElement::Shape(_) => {}
            }
        }
    }

    let mut error_count = 0usize;
    let mut warning_count = 0usize;
    let mut info_count = 0usize;
    for issue in &issues {
        match issue.severity {
            IssueSeverity::Error => error_count += 1,
            IssueSeverity::Warning => warning_count += 1,
            IssueSeverity::Info => info_count += 1,
        }
    }

    ReviewReport {
        slide_count: deck.slide_count(),
        error_count,
        warning_count,
        info_count,
        issues,
    }
}

/// Render a compact Deck Review toast banner over the top-right corner of a pixel buffer.
pub fn render_review_toast(report: &ReviewReport, buffer: &mut [u32], width: u32, height: u32) {
    let needed = match (width as usize).checked_mul(height as usize) {
        Some(n) => n,
        None => return,
    };
    if width < 160 || height < 48 || buffer.len() < needed {
        return;
    }
    let mut canvas = match Canvas::new(buffer, width, height) {
        Some(c) => c,
        None => return,
    };

    let banner_w = (width as f32 * 0.72).clamp(140.0, width as f32 - 20.0);
    let banner_h = 34.0;
    let bx = (width as f32 - banner_w - 10.0).max(10.0);
    let by = 10.0;
    let rect = Rect::new(bx, by, banner_w, banner_h);

    let (bg, border) = if report.has_errors() {
        (Color::rgba(0x7F, 0x1D, 0x1D, 240), Color::rgb(0xEF, 0x44, 0x44))
    } else if report.warning_count > 0 {
        (Color::rgba(0x78, 0x35, 0x0F, 240), Color::rgb(0xF5, 0x9E, 0x0B))
    } else {
        (Color::rgba(0x06, 0x4E, 0x3B, 240), Color::rgb(0x10, 0xB9, 0x81))
    };

    canvas.fill_round_rect(rect, 6.0, bg);
    canvas.stroke_round_rect(rect, 6.0, 1.5, border);

    if let Some(font) = Font::new(GEIST_SANS) {
        let msg = report.summary_line();
        let clipped: String = msg.chars().take(((banner_w - 20.0) / 6.5).max(16.0) as usize).collect();
        draw_text_run(
            &font,
            &clipped,
            12.0,
            bx + 10.0,
            by + 21.0,
            Color::WHITE,
            true,
            false,
            canvas.pixels_mut(),
            width,
            height,
        );
    }
}

pub(crate) fn write_escaped_json_str(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
