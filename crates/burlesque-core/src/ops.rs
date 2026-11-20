// SPDX-License-Identifier: GPL-3.0-only
//! OmaShow-style deterministic headless JSON inspection and batch ops engine.
//!
//! Provides:
//! - `#![no_std]` recursive-descent JSON parser and deterministic serializer
//! - [`inspect_deck_json`] to dump the full presentation structure deterministically
//! - [`apply_ops_json`] to atomically apply a batch of JSON mutations to a [`Deck`]
//!   with zero window/UI dependencies, plus automatic [`review_deck`] diagnostics.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use sex_canvas::{Color, Rect};

use crate::animation::{BuildEffect, BuildStep};
use crate::builds::{
    add_build_step, duplicate_slides, move_build_step, remove_build_step, reorder_slides,
    set_build_effect, set_build_timing, set_build_trigger, set_slides_skipped, Trigger,
};
use crate::deck::{AspectRatio, Deck, Slide};
use crate::elements::{
    Background, CodeBlock, Paragraph, Shape, ShapeKind, SlideElement, StrokeStyle, TableCell,
    TableElement, TextBox, TextRun, Transition,
};
use crate::layout::SlideLayout;
use crate::morph::ElementMeta;
use crate::pptx::parse_hex_color;
use crate::review::{review_deck, write_escaped_json_str, ReviewReport};
use crate::theme::Theme;

const MAX_JSON_DEPTH: usize = 64;

/// Dynamic JSON value with deterministic field order.
#[derive(Clone, Debug, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

/// JSON parse error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JSON error at byte {}: {}", self.offset, self.message)
    }
}

impl JsonValue {
    pub fn parse(input: &str) -> core::result::Result<Self, JsonError> {
        let mut p = JsonParser::new(input);
        p.skip_ws();
        let v = p.parse_value(0)?;
        p.skip_ws();
        if !p.eof() {
            return Err(JsonError {
                offset: p.pos,
                message: String::from("trailing characters after JSON"),
            });
        }
        Ok(v)
    }

    pub fn obj() -> Self {
        Self::Object(Vec::new())
    }

    pub fn arr() -> Self {
        Self::Array(Vec::new())
    }

    pub fn push(&mut self, val: JsonValue) {
        if let Self::Array(ref mut items) = self {
            items.push(val);
        }
    }

    pub fn insert(&mut self, key: impl Into<String>, val: JsonValue) {
        if let Self::Object(ref mut entries) = self {
            let k = key.into();
            if let Some((_, slot)) = entries.iter_mut().find(|(ek, _)| ek == &k) {
                *slot = val;
            } else {
                entries.push((k, val));
            }
        }
    }

    pub fn with(mut self, key: impl Into<String>, val: impl Into<JsonValue>) -> Self {
        self.insert(key, val.into());
        self
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        let Self::Object(entries) = self else {
            return None;
        };
        entries.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        self.as_f64().and_then(|n| if n.is_finite() { Some(n as f32) } else { None })
    }

    pub fn as_usize(&self) -> Option<usize> {
        let n = self.as_f64()?;
        if !n.is_finite() || n < 0.0 || libm::trunc(n) != n || n > (usize::MAX as f64) {
            return None;
        }
        Some(n as usize)
    }

    pub fn as_u32(&self) -> Option<u32> {
        let n = self.as_f64()?;
        if !n.is_finite() || n < 0.0 || libm::trunc(n) != n || n > (u32::MAX as f64) {
            return None;
        }
        Some(n as u32)
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            Self::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Self::as_str)
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Self::as_bool)
    }

    pub fn get_f32(&self, key: &str) -> Option<f32> {
        self.get(key).and_then(Self::as_f32)
    }

    pub fn get_usize(&self, key: &str) -> Option<usize> {
        self.get(key).and_then(Self::as_usize)
    }

    pub fn get_u32(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(Self::as_u32)
    }

    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    pub fn write_json(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Number(n) => {
                if !n.is_finite() {
                    out.push_str("null");
                } else if libm::trunc(*n) == *n && libm::fabs(*n) < 1e15 {
                    out.push_str(&format!("{}", *n as i64));
                } else {
                    let s = format!("{:.4}", n);
                    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
                    if trimmed.is_empty() || trimmed == "-0" {
                        out.push('0');
                    } else {
                        out.push_str(trimmed);
                    }
                }
            }
            Self::String(s) => write_escaped_json_str(s, out),
            Self::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
            Self::Object(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_escaped_json_str(k, out);
                    out.push(':');
                    v.write_json(out);
                }
                out.push('}');
            }
        }
    }
}

impl From<&str> for JsonValue {
    fn from(v: &str) -> Self {
        Self::String(String::from(v))
    }
}

impl From<String> for JsonValue {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

impl From<bool> for JsonValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<usize> for JsonValue {
    fn from(v: usize) -> Self {
        Self::Number(v as f64)
    }
}

impl From<u64> for JsonValue {
    fn from(v: u64) -> Self {
        Self::Number(v as f64)
    }
}

impl From<u32> for JsonValue {
    fn from(v: u32) -> Self {
        Self::Number(v as f64)
    }
}

impl From<u8> for JsonValue {
    fn from(v: u8) -> Self {
        Self::Number(v as f64)
    }
}

impl From<f32> for JsonValue {
    fn from(v: f32) -> Self {
        Self::Number(v as f64)
    }
}

impl<T: Into<JsonValue>> From<Option<T>> for JsonValue {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(inner) => inner.into(),
            None => Self::Null,
        }
    }
}

struct JsonParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn err(&self, msg: &str) -> JsonError {
        JsonError {
            offset: self.pos,
            message: String::from(msg),
        }
    }

    fn parse_value(&mut self, depth: usize) -> core::result::Result<JsonValue, JsonError> {
        if depth > MAX_JSON_DEPTH {
            return Err(self.err("JSON nesting depth exceeded"));
        }
        self.skip_ws();
        let Some(b) = self.peek() else {
            return Err(self.err("unexpected end of JSON input"));
        };
        match b {
            b'n' => {
                if self.bytes[self.pos..].starts_with(b"null") {
                    self.pos += 4;
                    Ok(JsonValue::Null)
                } else {
                    Err(self.err("expected 'null'"))
                }
            }
            b't' => {
                if self.bytes[self.pos..].starts_with(b"true") {
                    self.pos += 4;
                    Ok(JsonValue::Bool(true))
                } else {
                    Err(self.err("expected 'true'"))
                }
            }
            b'f' => {
                if self.bytes[self.pos..].starts_with(b"false") {
                    self.pos += 5;
                    Ok(JsonValue::Bool(false))
                } else {
                    Err(self.err("expected 'false'"))
                }
            }
            b'"' => self.parse_string().map(JsonValue::String),
            b'[' => self.parse_array(depth + 1),
            b'{' => self.parse_object(depth + 1),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(self.err("unexpected token in JSON")),
        }
    }

    fn parse_string(&mut self) -> core::result::Result<String, JsonError> {
        if self.peek() != Some(b'"') {
            return Err(self.err("expected '\"'"));
        }
        self.pos += 1;
        let mut out = String::new();
        while !self.eof() {
            let b = self.bytes[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Ok(out);
            }
            if b == b'\\' {
                self.pos += 1;
                let Some(esc) = self.peek() else {
                    return Err(self.err("unterminated escape sequence"));
                };
                self.pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000C}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let cp = self.parse_hex4()?;
                        if (0xD800..=0xDBFF).contains(&cp) {
                            if self.bytes[self.pos..].starts_with(b"\\u") {
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    let combined =
                                        0x10000 + (((cp as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                    out.push(core::char::from_u32(combined).unwrap_or('\u{FFFD}'));
                                } else {
                                    out.push('\u{FFFD}');
                                }
                            } else {
                                out.push('\u{FFFD}');
                            }
                        } else {
                            out.push(core::char::from_u32(cp as u32).unwrap_or('\u{FFFD}'));
                        }
                    }
                    _ => return Err(self.err("invalid JSON string escape")),
                }
            } else if b < 0x20 {
                return Err(self.err("unescaped control character in JSON string"));
            } else {
                let ch = self.src[self.pos..]
                    .chars()
                    .next()
                    .ok_or_else(|| self.err("invalid UTF-8 in JSON string"))?;
                out.push(ch);
                self.pos += ch.len_utf8();
            }
        }
        Err(self.err("unterminated JSON string"))
    }

    fn parse_hex4(&mut self) -> core::result::Result<u16, JsonError> {
        if self.pos + 4 > self.bytes.len() {
            return Err(self.err("incomplete \\uXXXX escape"));
        }
        let s = core::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
            .map_err(|_| self.err("invalid \\uXXXX escape"))?;
        let v = u16::from_str_radix(s, 16).map_err(|_| self.err("invalid hex in \\uXXXX"))?;
        self.pos += 4;
        Ok(v)
    }

    fn parse_number(&mut self) -> core::result::Result<JsonValue, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        let mut digits = 0usize;
        while let Some(b'0'..=b'9') = self.peek() {
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 {
            return Err(self.err("invalid JSON number"));
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            let mut frac_digits = 0usize;
            while let Some(b'0'..=b'9') = self.peek() {
                self.pos += 1;
                frac_digits += 1;
            }
            if frac_digits == 0 {
                return Err(self.err("expected fractional digits in JSON number"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let mut exp_digits = 0usize;
            while let Some(b'0'..=b'9') = self.peek() {
                self.pos += 1;
                exp_digits += 1;
            }
            if exp_digits == 0 {
                return Err(self.err("expected exponent digits in JSON number"));
            }
        }
        let slice = &self.src[start..self.pos];
        let val = slice.parse::<f64>().map_err(|_| self.err("invalid f64 literal"))?;
        Ok(JsonValue::Number(val))
    }

    fn parse_array(&mut self, depth: usize) -> core::result::Result<JsonValue, JsonError> {
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value(depth)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(self.err("expected ',' or ']' in JSON array")),
            }
        }
    }

    fn parse_object(&mut self, depth: usize) -> core::result::Result<JsonValue, JsonError> {
        self.pos += 1; // '{'
        let mut entries = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(entries));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(self.err("expected ':' after object key"));
            }
            self.pos += 1;
            let val = self.parse_value(depth)?;
            entries.push((key, val));
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(entries));
                }
                _ => return Err(self.err("expected ',' or '}' in JSON object")),
            }
        }
    }
}

/// Format a Color as `#RRGGBB` or `#AARRGGBB` hex string.
fn color_hex(c: Color) -> String {
    if c.a() == 255 {
        format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
    } else {
        format!("#{:02X}{:02X}{:02X}{:02X}", c.a(), c.r(), c.g(), c.b())
    }
}

fn layout_str(l: SlideLayout) -> &'static str {
    match l {
        SlideLayout::TitleSlide => "title_slide",
        SlideLayout::TitleAndBody => "title_and_body",
        SlideLayout::SectionHeader => "section_header",
        SlideLayout::TwoColumn => "two_column",
        SlideLayout::TitleAndImage => "title_and_image",
        SlideLayout::Blank => "blank",
    }
}

fn parse_layout_loose(s: &str) -> Option<SlideLayout> {
    let clean = s.trim();
    if clean.eq_ignore_ascii_case("title_slide") || clean.eq_ignore_ascii_case("title") {
        Some(SlideLayout::TitleSlide)
    } else if clean.eq_ignore_ascii_case("title_and_body") || clean.eq_ignore_ascii_case("body") {
        Some(SlideLayout::TitleAndBody)
    } else if clean.eq_ignore_ascii_case("section_header") || clean.eq_ignore_ascii_case("section") {
        Some(SlideLayout::SectionHeader)
    } else if clean.eq_ignore_ascii_case("two_column") || clean.eq_ignore_ascii_case("twocolumn") || clean.eq_ignore_ascii_case("2col") {
        Some(SlideLayout::TwoColumn)
    } else if clean.eq_ignore_ascii_case("title_and_image") || clean.eq_ignore_ascii_case("image") {
        Some(SlideLayout::TitleAndImage)
    } else if clean.eq_ignore_ascii_case("blank") {
        Some(SlideLayout::Blank)
    } else {
        None
    }
}

fn parse_theme_loose(name: &str) -> Option<Theme> {
    if let Some(t) = Theme::by_name(name) {
        return Some(t);
    }
    let clean = name.trim();
    if clean.eq_ignore_ascii_case("burlesque_velvet") || clean.eq_ignore_ascii_case("velvet") {
        Some(Theme::burlesque_velvet())
    } else if clean.eq_ignore_ascii_case("editorial_light") || clean.eq_ignore_ascii_case("editorial") || clean.eq_ignore_ascii_case("light") {
        Some(Theme::editorial_light())
    } else if clean.eq_ignore_ascii_case("midnight_slate") || clean.eq_ignore_ascii_case("midnight") || clean.eq_ignore_ascii_case("dark") {
        Some(Theme::midnight_slate())
    } else if clean.eq_ignore_ascii_case("champagne_ivory") || clean.eq_ignore_ascii_case("champagne") {
        Some(Theme::champagne_ivory())
    } else if clean.eq_ignore_ascii_case("cyber_prism") || clean.eq_ignore_ascii_case("cyber") {
        Some(Theme::cyber_prism())
    } else if clean.eq_ignore_ascii_case("swiss_modern") || clean.eq_ignore_ascii_case("swiss") {
        Some(Theme::swiss_modern())
    } else {
        None
    }
}

/// Produce a deterministic JSON snapshot of `deck` (slides, elements, builds, theme, metadata, and review).
pub fn inspect_deck_json(deck: &Deck) -> String {
    let mut root = JsonValue::obj()
        .with("format", "burlesque-deck-v1")
        .with(
            "metadata",
            JsonValue::obj()
                .with("title", deck.metadata.title.as_str())
                .with("author", deck.metadata.author.as_str())
                .with("date", deck.metadata.date.as_str())
                .with("description", deck.metadata.description.as_str()),
        )
        .with(
            "aspect_ratio",
            match deck.aspect_ratio {
                AspectRatio::Ratio16x9 => JsonValue::from("16:9"),
                AspectRatio::Ratio16x10 => JsonValue::from("16:10"),
                AspectRatio::Ratio4x3 => JsonValue::from("4:3"),
                AspectRatio::Custom { width, height } => {
                    JsonValue::obj().with("width", width).with("height", height)
                }
            },
        )
        .with("width", deck.width())
        .with("height", deck.height())
        .with(
            "theme",
            JsonValue::obj()
                .with("name", deck.theme.name.as_str())
                .with("show_master_chrome", deck.theme.show_master_chrome)
                .with(
                    "palette",
                    JsonValue::obj()
                        .with("background", color_hex(deck.theme.palette.background))
                        .with("foreground", color_hex(deck.theme.palette.foreground))
                        .with("primary_accent", color_hex(deck.theme.palette.primary_accent))
                        .with("secondary_accent", color_hex(deck.theme.palette.secondary_accent))
                        .with("muted", color_hex(deck.theme.palette.muted))
                        .with("surface", color_hex(deck.theme.palette.surface))
                        .with("border", color_hex(deck.theme.palette.border)),
                ),
        )
        .with("slide_count", deck.slide_count());

    let mut slides_arr = JsonValue::arr();
    for (s_idx, slide) in deck.slides.iter().enumerate() {
        let mut elems_arr = JsonValue::arr();
        for (e_idx, elem) in slide.elements.iter().enumerate() {
            let b = elem.bounds();
            let meta = slide.element_meta(e_idx);
            let mut e_obj = JsonValue::obj()
                .with("index", e_idx)
                .with(
                    "bounds",
                    JsonValue::obj()
                        .with("x", b.x)
                        .with("y", b.y)
                        .with("w", b.w)
                        .with("h", b.h),
                )
                .with("magic_key", elem.magic_key())
                .with("morph_key", meta.morph_key.as_deref().or_else(|| elem.magic_key()))
                .with("element_id", meta.element_id.as_deref())
                .with("rotation_deg", meta.rotation_deg)
                .with("opacity", meta.opacity);

            match elem {
                SlideElement::TextBox(tb) => {
                    e_obj = e_obj
                        .with("kind", "textbox")
                        .with("text", tb.plain_text())
                        .with("background", tb.background.map(color_hex))
                        .with("padding", tb.padding);
                    let mut p_arr = JsonValue::arr();
                    for p in &tb.paragraphs {
                        let first = p.runs.first();
                        p_arr.push(
                            JsonValue::obj()
                                .with("text", p.plain_text())
                                .with("bullet_level", p.bullet_level)
                                .with("font_size", first.map(|r| r.font_size).unwrap_or(24.0))
                                .with("bold", first.map(|r| r.bold).unwrap_or(false))
                                .with("italic", first.map(|r| r.italic).unwrap_or(false))
                                .with("color", first.and_then(|r| r.color).map(color_hex)),
                        );
                    }
                    e_obj.insert("paragraphs", p_arr);
                }
                SlideElement::Shape(s) => {
                    let (sk, rad) = match s.kind {
                        ShapeKind::Rect => ("rect", 0.0),
                        ShapeKind::RoundedRect { radius } => ("round_rect", radius),
                        ShapeKind::Ellipse => ("ellipse", 0.0),
                        ShapeKind::Line => ("line", 0.0),
                        ShapeKind::Arrow => ("arrow", 0.0),
                    };
                    e_obj = e_obj
                        .with("kind", "shape")
                        .with("shape_kind", sk)
                        .with("corner_radius", rad)
                        .with("fill", s.fill.map(color_hex))
                        .with("stroke_color", s.stroke.map(|st| color_hex(st.color)))
                        .with("stroke_width", s.stroke.map(|st| st.width));
                }
                SlideElement::Image(img) => {
                    let (iw, ih) = img
                        .decoded
                        .as_ref()
                        .map(|d| (d.width, d.height))
                        .unwrap_or((0, 0));
                    e_obj = e_obj
                        .with("kind", "image")
                        .with("has_decoded", img.decoded.is_some())
                        .with("width", iw)
                        .with("height", ih)
                        .with("data_len", img.data.len());
                }
                SlideElement::Table(tbl) => {
                    let mut rows_arr = JsonValue::arr();
                    for r in &tbl.cells {
                        let mut row_arr = JsonValue::arr();
                        for c in r {
                            row_arr.push(JsonValue::from(c.text.as_str()));
                        }
                        rows_arr.push(row_arr);
                    }
                    e_obj = e_obj
                        .with("kind", "table")
                        .with("rows", tbl.rows)
                        .with("cols", tbl.cols)
                        .with("cells", rows_arr);
                }
                SlideElement::Code(cb) => {
                    e_obj = e_obj
                        .with("kind", "code")
                        .with("language", cb.language.as_deref())
                        .with("code", cb.code.as_str())
                        .with("font_size", cb.font_size);
                }
            }
            elems_arr.push(e_obj);
        }

        let mut builds_arr = JsonValue::arr();
        for (b_idx, b) in slide.builds.iter().enumerate() {
            let trig: Trigger = b.trigger.into();
            builds_arr.push(
                JsonValue::obj()
                    .with("index", b_idx)
                    .with("element_index", b.element_index)
                    .with("paragraph_index", b.paragraph_index)
                    .with("effect", b.effect.kind_str())
                    .with("trigger", trig.kind_str())
                    .with("delay_ms", slide.build_delay_ms(b_idx))
                    .with("duration_ms", b.duration_ms),
            );
        }

        let bg_json = match slide.background_override {
            None => JsonValue::Null,
            Some(Background::Solid(c)) => JsonValue::obj()
                .with("kind", "solid")
                .with("color", color_hex(c)),
            Some(Background::Gradient(top, bottom)) => JsonValue::obj()
                .with("kind", "gradient")
                .with("top", color_hex(top))
                .with("bottom", color_hex(bottom)),
        };

        slides_arr.push(
            JsonValue::obj()
                .with("index", s_idx)
                .with("id", slide.id)
                .with("title", slide.title.as_str())
                .with("layout", layout_str(slide.layout))
                .with("hidden", slide.hidden)
                .with("transition", slide.transition.kind_str())
                .with("transition_duration_ms", slide.transition_duration_ms())
                .with("notes", slide.notes())
                .with("rehearsed_dwell_ms", slide.rehearsed_dwell_ms)
                .with("background_override", bg_json)
                .with("elements", elems_arr)
                .with("builds", builds_arr),
        );
    }

    root.insert("slides", slides_arr);

    let review = review_deck(deck);
    if let Ok(review_val) = JsonValue::parse(&review.to_json()) {
        root.insert("review", review_val);
    }

    root.to_json()
}

/// Error returned when applying headless JSON operations to a [`Deck`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpsError {
    pub op_index: Option<usize>,
    pub message: String,
}

impl OpsError {
    fn at(idx: usize, msg: impl Into<String>) -> Self {
        Self {
            op_index: Some(idx),
            message: msg.into(),
        }
    }

    fn root(msg: impl Into<String>) -> Self {
        Self {
            op_index: None,
            message: msg.into(),
        }
    }

    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"ok\":false,\"op_index\":");
        match self.op_index {
            Some(i) => out.push_str(&format!("{}", i)),
            None => out.push_str("null"),
        }
        out.push_str(",\"error\":");
        write_escaped_json_str(&self.message, &mut out);
        out.push('}');
        out
    }
}

impl fmt::Display for OpsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.op_index {
            Some(idx) => write!(f, "op[{}]: {}", idx, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

/// Summary returned after atomically applying a batch of JSON ops to a [`Deck`].
#[derive(Clone, Debug, PartialEq)]
pub struct ApplyOpsResult {
    pub applied_ops: usize,
    pub slide_count: usize,
    pub review: ReviewReport,
}

impl ApplyOpsResult {
    pub fn to_json(&self) -> String {
        let mut out = format!(
            "{{\"ok\":true,\"applied_ops\":{},\"slide_count\":{},\"review\":",
            self.applied_ops, self.slide_count
        );
        out.push_str(&self.review.to_json());
        out.push('}');
        out
    }
}

/// Atomically apply OmaShow-style JSON operations to `deck`.
///
/// Accepts either `[ { "op": "..." }, ... ]`, `{ "ops": [ ... ] }`, or a single `{ "op": "..." }`.
/// If any operation fails, `deck` is left completely unchanged.
pub fn apply_ops_json(
    deck: &mut Deck,
    ops_json: &str,
) -> core::result::Result<ApplyOpsResult, OpsError> {
    let val = JsonValue::parse(ops_json).map_err(|e| OpsError::root(e.to_string()))?;
    let ops_slice: Vec<JsonValue> = match &val {
        JsonValue::Array(arr) => arr.clone(),
        JsonValue::Object(_) => {
            if let Some(arr) = val.get("ops").and_then(JsonValue::as_array) {
                arr.to_vec()
            } else if val.get_str("op").is_some() {
                vec![val]
            } else {
                return Err(OpsError::root("expected array of ops or object with 'ops' or 'op'"));
            }
        }
        _ => return Err(OpsError::root("expected JSON array or object")),
    };

    let mut working = deck.clone();
    for (idx, op_val) in ops_slice.iter().enumerate() {
        apply_single_op(&mut working, op_val).map_err(|msg| OpsError::at(idx, msg))?;
    }

    let review = review_deck(&working);
    let slide_count = working.slide_count();
    *deck = working;

    Ok(ApplyOpsResult {
        applied_ops: ops_slice.len(),
        slide_count,
        review,
    })
}

/// Apply JSON ops atomically and return the resulting full [`inspect_deck_json`] snapshot.
pub fn apply_ops_and_inspect_json(
    deck: &mut Deck,
    ops_json: &str,
) -> core::result::Result<String, OpsError> {
    let _ = apply_ops_json(deck, ops_json)?;
    Ok(inspect_deck_json(deck))
}

fn parse_indices_field(op: &JsonValue) -> core::result::Result<Vec<usize>, String> {
    if let Some(arr) = op.get("indices").and_then(JsonValue::as_array) {
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            let idx = item
                .as_usize()
                .ok_or_else(|| String::from("'indices' must contain non-negative integers"))?;
            out.push(idx);
        }
        return Ok(out);
    }
    if let Some(idx) = op.get_usize("slide_index") {
        return Ok(vec![idx]);
    }
    Err(String::from("missing 'indices' or 'slide_index'"))
}

fn apply_single_op(deck: &mut Deck, op: &JsonValue) -> core::result::Result<(), String> {
    let op_name = op
        .get_str("op")
        .ok_or_else(|| String::from("missing string field 'op'"))?;

    match op_name {
        "set_metadata" => {
            if let Some(t) = op.get_str("title") {
                deck.metadata.title = t.to_string();
            }
            if let Some(a) = op.get_str("author") {
                deck.metadata.author = a.to_string();
            }
            if let Some(d) = op.get_str("date") {
                deck.metadata.date = d.to_string();
            }
            if let Some(desc) = op.get_str("description") {
                deck.metadata.description = desc.to_string();
            }
            Ok(())
        }
        "set_theme" => {
            let name = op
                .get_str("name")
                .ok_or_else(|| String::from("set_theme requires 'name'"))?;
            let theme = parse_theme_loose(name)
                .ok_or_else(|| format!("unknown theme '{}'", name))?;
            deck.set_theme(theme);
            Ok(())
        }
        "set_aspect_ratio" => {
            if let Some(r) = op.get_str("ratio") {
                deck.aspect_ratio = match r.trim() {
                    "16:9" => AspectRatio::Ratio16x9,
                    "16:10" => AspectRatio::Ratio16x10,
                    "4:3" => AspectRatio::Ratio4x3,
                    other => return Err(format!("unknown aspect ratio '{}'", other)),
                };
                return Ok(());
            }
            if let (Some(w), Some(h)) = (op.get_f32("width"), op.get_f32("height")) {
                if w > 0.0 && h > 0.0 {
                    deck.aspect_ratio = AspectRatio::Custom { width: w, height: h };
                    return Ok(());
                }
            }
            Err(String::from("set_aspect_ratio requires 'ratio' or positive 'width'+'height'"))
        }
        "add_slide" => {
            let layout = match op.get_str("layout") {
                Some(ls) => parse_layout_loose(ls)
                    .ok_or_else(|| format!("unknown layout '{}'", ls))?,
                None => SlideLayout::TitleAndBody,
            };
            let mut slide = Slide::new(0, layout);
            if let Some(title) = op.get_str("title") {
                slide.title = title.to_string();
                let tb = build_title_textbox(deck.width(), deck.height(), layout, title);
                slide.add_element(SlideElement::TextBox(tb));
            }
            if let Some(body) = op.get_str("body") {
                let tb = build_body_textbox(deck.width(), deck.height(), body);
                slide.add_element(SlideElement::TextBox(tb));
            }
            if let Some(notes) = op.get_str("notes") {
                slide.set_speaker_notes(notes);
            }
            if let Some(tr) = op.get_str("transition") {
                slide.transition = Transition::from_str_loose(tr);
            }
            if let Some(hidden) = op.get_bool("hidden") {
                slide.hidden = hidden;
            }
            let pos = op.get_usize("index").unwrap_or(deck.slides.len());
            deck.insert_slide(pos, slide);
            Ok(())
        }
        "remove_slide" | "delete_slide" => {
            let idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("remove_slide requires 'slide_index'"))?;
            if deck.remove_slide(idx).is_none() {
                return Err(format!("slide_index {} out of range", idx));
            }
            Ok(())
        }
        "duplicate_slide" | "duplicate_slides" => {
            let indices = parse_indices_field(op)?;
            if duplicate_slides(deck, &indices).is_empty() {
                return Err(String::from("no valid slide indices to duplicate"));
            }
            Ok(())
        }
        "move_slide" => {
            let from = op
                .get_usize("from")
                .ok_or_else(|| String::from("move_slide requires 'from'"))?;
            let to = op
                .get_usize("to")
                .ok_or_else(|| String::from("move_slide requires 'to'"))?;
            if !deck.move_slide(from, to) {
                return Err(format!("invalid move_slide {} -> {}", from, to));
            }
            Ok(())
        }
        "reorder_slides" => {
            let indices = parse_indices_field(op)?;
            let target = op
                .get_usize("target_index")
                .ok_or_else(|| String::from("reorder_slides requires 'target_index'"))?;
            if reorder_slides(deck, &indices, target).is_empty() {
                return Err(String::from("reorder_slides matched no valid indices"));
            }
            Ok(())
        }
        "set_slide_skipped" | "skip_slides" => {
            let indices = parse_indices_field(op)?;
            let skipped = op.get_bool("skipped").unwrap_or(true);
            if set_slides_skipped(deck, &indices, skipped) == 0 {
                return Err(String::from("no valid slide indices to update"));
            }
            Ok(())
        }
        "set_slide" => {
            let idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("set_slide requires 'slide_index'"))?;
            let slide = deck
                .get_slide_mut(idx)
                .ok_or_else(|| format!("slide_index {} out of range", idx))?;
            if let Some(title) = op.get_str("title") {
                slide.title = title.to_string();
            }
            if let Some(notes) = op.get_str("notes") {
                slide.set_speaker_notes(notes);
            }
            if let Some(tr) = op.get_str("transition") {
                slide.transition = Transition::from_str_loose(tr);
            }
            if let Some(dur) = op.get_u32("transition_duration_ms") {
                slide.transition_duration_ms = dur;
            }
            if let Some(hidden) = op.get_bool("hidden") {
                slide.hidden = hidden;
            }
            if let Some(ls) = op.get_str("layout") {
                if let Some(l) = parse_layout_loose(ls) {
                    slide.layout = l;
                } else {
                    return Err(format!("unknown layout '{}'", ls));
                }
            }
            if let Some(bg_val) = op.get("background") {
                match bg_val {
                    JsonValue::Null => slide.background_override = None,
                    JsonValue::String(hex) => {
                        let c = parse_hex_color(hex)
                            .ok_or_else(|| format!("invalid background color '{}'", hex))?;
                        slide.background_override = Some(Background::Solid(c));
                    }
                    _ => return Err(String::from("background must be hex string or null")),
                }
            }
            Ok(())
        }
        "set_notes" => {
            let idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("set_notes requires 'slide_index'"))?;
            let notes = op
                .get_str("notes")
                .ok_or_else(|| String::from("set_notes requires 'notes'"))?;
            let slide = deck
                .get_slide_mut(idx)
                .ok_or_else(|| format!("slide_index {} out of range", idx))?;
            slide.set_speaker_notes(notes);
            Ok(())
        }
        "set_transition" => {
            let idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("set_transition requires 'slide_index'"))?;
            let tr = op
                .get_str("transition")
                .ok_or_else(|| String::from("set_transition requires 'transition'"))?;
            let slide = deck
                .get_slide_mut(idx)
                .ok_or_else(|| format!("slide_index {} out of range", idx))?;
            slide.transition = Transition::from_str_loose(tr);
            if let Some(dur) = op.get_u32("duration_ms") {
                slide.transition_duration_ms = dur;
            }
            Ok(())
        }
        "add_textbox" => {
            let s_idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("add_textbox requires 'slide_index'"))?;
            let slide = deck
                .get_slide_mut(s_idx)
                .ok_or_else(|| format!("slide_index {} out of range", s_idx))?;
            let bounds = Rect::new(
                op.get_f32("x").unwrap_or(120.0),
                op.get_f32("y").unwrap_or(120.0),
                op.get_f32("w").unwrap_or(800.0),
                op.get_f32("h").unwrap_or(120.0),
            );
            let text = op.get_str("text").unwrap_or("");
            let font_size = op.get_f32("font_size").unwrap_or(28.0).max(4.0);
            let mut run = TextRun::plain(text, font_size);
            if op.get_bool("bold").unwrap_or(false) {
                run = run.bold();
            }
            if op.get_bool("italic").unwrap_or(false) {
                run = run.italic();
            }
            if op.get_bool("mono").unwrap_or(false) {
                run = run.mono();
            }
            if let Some(col_hex) = op.get_str("color") {
                let c = parse_hex_color(col_hex)
                    .ok_or_else(|| format!("invalid color '{}'", col_hex))?;
                run = run.with_color(c);
            }
            let mut p = Paragraph::new();
            p.push_run(run);
            let mut tb = TextBox::new(bounds).with_paragraph(p);
            if let Some(bg_hex) = op.get_str("background") {
                let bg = parse_hex_color(bg_hex)
                    .ok_or_else(|| format!("invalid background '{}'", bg_hex))?;
                tb.background = Some(bg);
            }
            let key = op.get_str("morph_key").or_else(|| op.get_str("magic_key"));
            if let Some(k) = key {
                tb.magic_key = Some(k.to_string());
            }
            let meta = meta_from_op(op, key);
            let e_idx = slide.elements.len();
            slide.add_element(SlideElement::TextBox(tb));
            slide.set_element_meta(e_idx, meta);
            Ok(())
        }
        "add_shape" => {
            let s_idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("add_shape requires 'slide_index'"))?;
            let slide = deck
                .get_slide_mut(s_idx)
                .ok_or_else(|| format!("slide_index {} out of range", s_idx))?;
            let bounds = Rect::new(
                op.get_f32("x").unwrap_or(200.0),
                op.get_f32("y").unwrap_or(200.0),
                op.get_f32("w").unwrap_or(240.0),
                op.get_f32("h").unwrap_or(160.0),
            );
            let shape_name = op.get_str("shape").or_else(|| op.get_str("shape_kind")).unwrap_or("rect");
            let radius = op.get_f32("corner_radius").unwrap_or(16.0);
            let kind = match shape_name.trim().to_ascii_lowercase().as_str() {
                "round_rect" | "rounded_rect" | "roundrect" => ShapeKind::RoundedRect { radius },
                "ellipse" | "circle" => ShapeKind::Ellipse,
                "line" => ShapeKind::Line,
                "arrow" => ShapeKind::Arrow,
                _ => ShapeKind::Rect,
            };
            let fill = match op.get_str("fill") {
                Some(h) => Some(parse_hex_color(h).ok_or_else(|| format!("invalid fill '{}'", h))?),
                None => Some(Color::rgb(0x3B, 0x82, 0xF6)),
            };
            let stroke = if let Some(sc_hex) = op.get_str("stroke_color") {
                let sc = parse_hex_color(sc_hex)
                    .ok_or_else(|| format!("invalid stroke_color '{}'", sc_hex))?;
                Some(StrokeStyle {
                    color: sc,
                    width: op.get_f32("stroke_width").unwrap_or(2.0),
                })
            } else {
                None
            };
            let key = op.get_str("morph_key").or_else(|| op.get_str("magic_key"));
            let shape = Shape {
                kind,
                bounds,
                fill,
                stroke,
                magic_key: key.map(String::from),
            };
            let meta = meta_from_op(op, key);
            let e_idx = slide.elements.len();
            slide.add_element(SlideElement::Shape(shape));
            slide.set_element_meta(e_idx, meta);
            Ok(())
        }
        "add_code" => {
            let s_idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("add_code requires 'slide_index'"))?;
            let slide = deck
                .get_slide_mut(s_idx)
                .ok_or_else(|| format!("slide_index {} out of range", s_idx))?;
            let bounds = Rect::new(
                op.get_f32("x").unwrap_or(140.0),
                op.get_f32("y").unwrap_or(260.0),
                op.get_f32("w").unwrap_or(1000.0),
                op.get_f32("h").unwrap_or(320.0),
            );
            let code = op.get_str("code").unwrap_or("");
            let mut cb = CodeBlock::new(bounds, code);
            if let Some(lang) = op.get_str("language") {
                cb = cb.with_language(lang);
            }
            if let Some(fs) = op.get_f32("font_size") {
                cb.font_size = fs.max(6.0);
            }
            let key = op.get_str("morph_key").or_else(|| op.get_str("magic_key"));
            if let Some(k) = key {
                cb.magic_key = Some(k.to_string());
            }
            let meta = meta_from_op(op, key);
            let e_idx = slide.elements.len();
            slide.add_element(SlideElement::Code(cb));
            slide.set_element_meta(e_idx, meta);
            Ok(())
        }
        "add_table" => {
            let s_idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("add_table requires 'slide_index'"))?;
            let slide = deck
                .get_slide_mut(s_idx)
                .ok_or_else(|| format!("slide_index {} out of range", s_idx))?;
            let bounds = Rect::new(
                op.get_f32("x").unwrap_or(140.0),
                op.get_f32("y").unwrap_or(260.0),
                op.get_f32("w").unwrap_or(1000.0),
                op.get_f32("h").unwrap_or(360.0),
            );
            let rows = op.get_usize("rows").unwrap_or(2).clamp(1, 50);
            let cols = op.get_usize("cols").unwrap_or(2).clamp(1, 50);
            let mut tbl = TableElement::new(bounds, rows, cols);
            if let Some(cell_rows) = op.get("cells").and_then(JsonValue::as_array) {
                for (r, rv) in cell_rows.iter().enumerate().take(rows) {
                    if let Some(cols_arr) = rv.as_array() {
                        for (c, cv) in cols_arr.iter().enumerate().take(cols) {
                            if let Some(txt) = cv.as_str() {
                                let mut cell = TableCell::new(txt);
                                cell.bold = r == 0;
                                tbl.set_cell(r, c, cell);
                            }
                        }
                    }
                }
            }
            let key = op.get_str("morph_key").or_else(|| op.get_str("magic_key"));
            if let Some(k) = key {
                tbl.magic_key = Some(k.to_string());
            }
            let meta = meta_from_op(op, key);
            let e_idx = slide.elements.len();
            slide.add_element(SlideElement::Table(tbl));
            slide.set_element_meta(e_idx, meta);
            Ok(())
        }
        "update_element" => {
            let s_idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("update_element requires 'slide_index'"))?;
            let e_idx = op
                .get_usize("element_index")
                .ok_or_else(|| String::from("update_element requires 'element_index'"))?;
            let slide = deck
                .get_slide_mut(s_idx)
                .ok_or_else(|| format!("slide_index {} out of range", s_idx))?;
            if e_idx >= slide.elements.len() {
                return Err(format!("element_index {} out of range", e_idx));
            }
            let mut b = slide.elements[e_idx].bounds();
            if let Some(x) = op.get_f32("x") { b.x = x; }
            if let Some(y) = op.get_f32("y") { b.y = y; }
            if let Some(w) = op.get_f32("w") { b.w = w; }
            if let Some(h) = op.get_f32("h") { b.h = h; }
            slide.elements[e_idx].set_bounds(b);

            if let Some(k) = op.get_str("morph_key").or_else(|| op.get_str("magic_key")) {
                slide.set_element_morph_key(e_idx, Some(k.to_string()));
            }
            if let Some(id) = op.get_str("element_id") {
                slide.set_element_id(e_idx, Some(id.to_string()));
            }
            if let Some(rot) = op.get_f32("rotation_deg") {
                slide.set_element_rotation(e_idx, rot);
            }
            if let Some(opacity) = op.get_f32("opacity") {
                slide.set_element_opacity(e_idx, opacity);
            }

            match &mut slide.elements[e_idx] {
                SlideElement::TextBox(tb) => {
                    if let Some(text) = op.get_str("text") {
                        let fs = op.get_f32("font_size").unwrap_or_else(|| {
                            tb.paragraphs
                                .first()
                                .and_then(|p| p.runs.first())
                                .map(|r| r.font_size)
                                .unwrap_or(28.0)
                        });
                        let col = if let Some(ch) = op.get_str("color") {
                            Some(parse_hex_color(ch).ok_or_else(|| format!("invalid color '{}'", ch))?)
                        } else {
                            tb.paragraphs.first().and_then(|p| p.runs.first()).and_then(|r| r.color)
                        };
                        let mut run = TextRun::plain(text, fs);
                        if let Some(c) = col {
                            run = run.with_color(c);
                        }
                        let mut p = Paragraph::new();
                        p.push_run(run);
                        tb.paragraphs = vec![p];
                    } else {
                        if let Some(fs) = op.get_f32("font_size") {
                            for p in &mut tb.paragraphs {
                                for r in &mut p.runs {
                                    r.font_size = fs.max(4.0);
                                }
                            }
                        }
                        if let Some(ch) = op.get_str("color") {
                            let c = parse_hex_color(ch).ok_or_else(|| format!("invalid color '{}'", ch))?;
                            for p in &mut tb.paragraphs {
                                for r in &mut p.runs {
                                    r.color = Some(c);
                                }
                            }
                        }
                    }
                }
                SlideElement::Shape(s) => {
                    if let Some(rad) = op.get_f32("corner_radius") {
                        if matches!(s.kind, ShapeKind::Rect | ShapeKind::RoundedRect { .. }) {
                            s.kind = ShapeKind::RoundedRect { radius: rad.max(0.0) };
                        }
                    }
                    if let Some(fh) = op.get_str("fill") {
                        s.fill = Some(parse_hex_color(fh).ok_or_else(|| format!("invalid fill '{}'", fh))?);
                    }
                    if let Some(sh) = op.get_str("stroke_color") {
                        let sc = parse_hex_color(sh).ok_or_else(|| format!("invalid stroke_color '{}'", sh))?;
                        let sw = op.get_f32("stroke_width").unwrap_or(s.stroke.map(|x| x.width).unwrap_or(2.0));
                        s.stroke = Some(StrokeStyle { color: sc, width: sw });
                    }
                }
                _ => {}
            }
            Ok(())
        }
        "remove_element" | "delete_element" => {
            let s_idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("remove_element requires 'slide_index'"))?;
            let e_idx = op
                .get_usize("element_index")
                .ok_or_else(|| String::from("remove_element requires 'element_index'"))?;
            let slide = deck
                .get_slide_mut(s_idx)
                .ok_or_else(|| format!("slide_index {} out of range", s_idx))?;
            if slide.remove_element(e_idx).is_none() {
                return Err(format!("element_index {} out of range", e_idx));
            }
            Ok(())
        }
        "add_build" => {
            let s_idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("add_build requires 'slide_index'"))?;
            let e_idx = op
                .get_usize("element_index")
                .ok_or_else(|| String::from("add_build requires 'element_index'"))?;
            let slide = deck
                .get_slide_mut(s_idx)
                .ok_or_else(|| format!("slide_index {} out of range", s_idx))?;
            if e_idx >= slide.elements.len() {
                return Err(format!("element_index {} out of range", e_idx));
            }
            let effect = op
                .get_str("effect")
                .map(BuildEffect::from_str_loose)
                .unwrap_or(BuildEffect::Appear);
            let trigger = op
                .get_str("trigger")
                .map(Trigger::from_str_loose)
                .unwrap_or(Trigger::OnClick);
            let delay_ms = op.get_u32("delay_ms").unwrap_or(0);
            let duration_ms = op.get_u32("duration_ms").unwrap_or(400);
            if let Some(p_idx) = op.get_usize("paragraph_index") {
                let step = BuildStep::new(e_idx, effect)
                    .with_paragraph(p_idx)
                    .with_trigger(trigger.to_build_trigger(delay_ms))
                    .with_duration(duration_ms);
                let b_idx = slide.builds.len();
                slide.builds.push(step);
                if delay_ms > 0 {
                    slide.set_build_delay_ms(b_idx, delay_ms);
                }
            } else {
                add_build_step(slide, e_idx, effect, trigger, delay_ms, duration_ms);
            }
            Ok(())
        }
        "move_build" => {
            let s_idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("move_build requires 'slide_index'"))?;
            let from = op
                .get_usize("from")
                .ok_or_else(|| String::from("move_build requires 'from'"))?;
            let to = op
                .get_usize("to")
                .ok_or_else(|| String::from("move_build requires 'to'"))?;
            let slide = deck
                .get_slide_mut(s_idx)
                .ok_or_else(|| format!("slide_index {} out of range", s_idx))?;
            if !move_build_step(slide, from, to) {
                return Err(format!("invalid move_build {} -> {}", from, to));
            }
            Ok(())
        }
        "remove_build" => {
            let s_idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("remove_build requires 'slide_index'"))?;
            let b_idx = op
                .get_usize("build_index")
                .ok_or_else(|| String::from("remove_build requires 'build_index'"))?;
            let slide = deck
                .get_slide_mut(s_idx)
                .ok_or_else(|| format!("slide_index {} out of range", s_idx))?;
            if remove_build_step(slide, b_idx).is_none() {
                return Err(format!("build_index {} out of range", b_idx));
            }
            Ok(())
        }
        "update_build" => {
            let s_idx = op
                .get_usize("slide_index")
                .ok_or_else(|| String::from("update_build requires 'slide_index'"))?;
            let b_idx = op
                .get_usize("build_index")
                .ok_or_else(|| String::from("update_build requires 'build_index'"))?;
            let slide = deck
                .get_slide_mut(s_idx)
                .ok_or_else(|| format!("slide_index {} out of range", s_idx))?;
            if b_idx >= slide.builds.len() {
                return Err(format!("build_index {} out of range", b_idx));
            }
            if let Some(eff) = op.get_str("effect") {
                set_build_effect(slide, b_idx, BuildEffect::from_str_loose(eff));
            }
            if let Some(trig) = op.get_str("trigger") {
                let d = op.get_u32("delay_ms").unwrap_or_else(|| slide.build_delay_ms(b_idx));
                set_build_trigger(slide, b_idx, Trigger::from_str_loose(trig), d);
            }
            if op.get_u32("delay_ms").is_some() || op.get_u32("duration_ms").is_some() {
                let d = op.get_u32("delay_ms").unwrap_or_else(|| slide.build_delay_ms(b_idx));
                let dur = op.get_u32("duration_ms").unwrap_or(slide.builds[b_idx].duration_ms);
                set_build_timing(slide, b_idx, d, dur);
            }
            Ok(())
        }
        other => Err(format!("unknown op '{}'", other)),
    }
}

fn meta_from_op(op: &JsonValue, key: Option<&str>) -> ElementMeta {
    let mut meta = ElementMeta::default();
    meta.morph_key = key.map(String::from);
    meta.element_id = op.get_str("element_id").map(String::from);
    if let Some(r) = op.get_f32("rotation_deg") {
        meta.rotation_deg = r;
    }
    if let Some(o) = op.get_f32("opacity") {
        meta.opacity = o.clamp(0.0, 1.0);
    }
    meta
}

fn build_title_textbox(width: f32, height: f32, layout: SlideLayout, title: &str) -> TextBox {
    let mx = width * 0.08;
    let my = height * 0.08;
    let cw = width - mx * 2.0;
    let y = if layout == SlideLayout::TitleSlide {
        height * 0.30
    } else {
        my
    };
    let h = if layout == SlideLayout::TitleSlide {
        height * 0.22
    } else {
        height * 0.14
    };
    let mut tb = TextBox::new(Rect::new(mx, y, cw, h)).with_magic_key("hero-title");
    let mut p = Paragraph::new();
    p.push_run(TextRun::plain(title, 48.0).bold());
    tb.paragraphs.push(p);
    tb
}

fn build_body_textbox(width: f32, height: f32, body: &str) -> TextBox {
    let mx = width * 0.08;
    let my = height * 0.08;
    let cw = width - mx * 2.0;
    let by = my + height * 0.18;
    let bh = height - by - my;
    let mut tb = TextBox::new(Rect::new(mx, by, cw, bh));
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            tb.paragraphs.push(Paragraph::bullet(item, 26.0, 1));
        } else if !trimmed.is_empty() {
            tb.paragraphs.push(Paragraph::from_text(trimmed, 26.0));
        }
    }
    tb
}
