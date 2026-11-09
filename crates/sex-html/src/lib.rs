// SPDX-License-Identifier: GPL-3.0-only
//! The SexOS web engine core: HTML in, positioned display list out.
//!
//! `parse` builds a [`dom::Dom`] from any input, `css` and `style` run the cascade,
//! `layout` produces a [`layout::Page`]: rectangles, text runs, images and list
//! markers at integer pixel positions, plus link rectangles and anchors. Drawing is
//! the caller's: it only needs to measure text ([`layout::Host`]). No `std`, no
//! floating point, no authority.
#![no_std]
extern crate alloc;

pub mod css;
pub mod dom;
pub mod layout;
pub mod parse;
pub mod style;
pub mod url;

pub use layout::{render, FontKey, Host, Item, Link, Page};

use alloc::vec::Vec;
use dom::{Dom, Kind};
use style::Style;

/// Computed style for every element (text nodes and the document get `None`).
/// `extra_css` are external sheets fetched for the page, applied before its `<style>`s.
pub fn styles(dom: &Dom, extra_css: &[&str], width: i32, height: i32) -> Vec<Option<Style>> {
    styles_at(dom, extra_css, width, height, 0, &mut Timeline::default()).0
}

/// When each element's CSS animation started, kept from one styling to the next.
#[derive(Default)]
pub struct Timeline {
    starts: alloc::collections::BTreeMap<(dom::NodeId, alloc::string::String), u64>,
    transitions: style::TransitionState,
    /// When animations found by the first styling start; the time of that styling if None.
    pub begun: Option<u64>,
}

impl Timeline {
    /// Animations that apply when first styled count from `ms`.
    pub fn starting_at(ms: u64) -> Self { Timeline { starts: Default::default(), transitions: Default::default(), begun: Some(ms) } }
}

/// [`styles`] at `now_ms`, with CSS animations at that moment: each runs from when it
/// first applied to its element (as `timeline` remembers). The flag is true while an
/// animation is still running (style again soon).
pub fn styles_at(dom: &Dom, extra_css: &[&str], width: i32, height: i32, now_ms: u64, timeline: &mut Timeline) -> (Vec<Option<Style>>, bool) {
    let ua = css::parse_sheet(style::USER_AGENT_CSS, width);
    let mut author: Vec<css::Sheet> = extra_css.iter().map(|c| css::parse_sheet(c, width)).collect();
    for (text, media) in page_styles(dom) {
        if media.map_or(true, |m| css::media_matches(&m, width)) { author.push(css::parse_sheet(&text, width)); }
    }
    let mut sheets: Vec<&css::Sheet> = Vec::with_capacity(author.len() + 1);
    sheets.push(&ua);
    sheets.extend(author.iter());
    let mut cascade = style::Cascade::new(sheets);
    cascade.now_ms = now_ms;
    cascade.start_default = timeline.begun.take().unwrap_or(now_ms);
    cascade.starts = core::cell::RefCell::new(core::mem::take(&mut timeline.starts));
    cascade.transitions_before = core::cell::RefCell::new(core::mem::take(&mut timeline.transitions));
    let mut out: Vec<Option<Style>> = alloc::vec![None; dom.nodes.len()];
    let root = Style::initial();
    let mut root_font = 16;
    // Document order: scripts may move nodes, so arena order is not tree order.
    let order = dom.descendants(Dom::ROOT);
    for &id in &order {
        if !matches!(dom.nodes[id].kind, Kind::Element { .. }) { continue; }
        let parent = dom.nodes[id].parent.and_then(|p| out[p].as_ref()).unwrap_or(&root);
        let s = style::compute(dom, &cascade, id, &parent.clone(), root_font, (width, height));
        if dom.tag(id) == Some("html") { root_font = s.font_size; }
        out[id] = Some(s);
    }
    // display:none hides the subtree; carry it down so layout can skip quickly.
    for &id in &order {
        if let Some(p) = dom.nodes[id].parent {
            if out[p].as_ref().map_or(false, |s| s.display == style::Display::None) {
                if let Some(s) = out[id].as_mut() { s.display = style::Display::None; }
            }
        }
    }
    timeline.starts = cascade.seen.take();
    timeline.transitions = cascade.transitions_after.take();
    (out, cascade.animating.get())
}

/// `<style>` contents with their `media` attribute.
fn page_styles(dom: &Dom) -> Vec<(alloc::string::String, Option<alloc::string::String>)> {
    dom.descendants(Dom::ROOT).into_iter().filter(|&i| dom.tag(i) == Some("style"))
        .map(|i| (dom.text_content(i), dom.attr(i, "media").map(alloc::string::String::from))).collect()
}

/// `<link rel=stylesheet href=...>` addresses, in document order, for the caller to fetch.
pub fn stylesheet_links(dom: &Dom) -> Vec<&str> {
    dom.descendants(Dom::ROOT).into_iter().filter(|&i| dom.tag(i) == Some("link"))
        .filter(|&i| dom.attr(i, "rel").map_or(false, |r| r.to_ascii_lowercase().split_ascii_whitespace().any(|w| w == "stylesheet")))
        .filter(|&i| dom.attr(i, "media").map_or(true, |m| { let m = m.to_ascii_lowercase(); !m.contains("print") || m.contains("screen") || m.contains("all") }))
        .filter_map(|i| dom.attr(i, "href")).collect()
}

/// `<img src>` addresses, in document order.
pub fn image_sources(dom: &Dom) -> Vec<&str> {
    dom.descendants(Dom::ROOT).into_iter().filter(|&i| dom.tag(i) == Some("img")).filter_map(|i| dom.attr(i, "src")).collect()
}
