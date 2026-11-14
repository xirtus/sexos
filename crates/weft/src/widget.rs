// SPDX-License-Identifier: GPL-3.0-only
//! What a widget is: its kind and state, how it lays out, and the data models behind
//! lists, tables, trees and tab strips.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use taffy::prelude::*;

use crate::edit::LineEdit;
use crate::menu::Menu;
use crate::theme::Color;

/// A widget's handle. Ids stay valid until the widget is removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(pub(crate) u32);

impl Id {
    /// The number behind the handle (the accessibility tree uses it + 1).
    pub fn index(self) -> u32 { self.0 }
}

/// Text sizes and faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle { Body, Caption, Heading, Title, Display, Mono }

impl TextStyle {
    /// Size in layout units for a body size of `body`.
    pub fn size(self, body: f32) -> f32 {
        match self { TextStyle::Body | TextStyle::Mono => body, TextStyle::Caption => body * 0.86, TextStyle::Heading => body * 1.15, TextStyle::Title => body * 1.45, TextStyle::Display => body * 2.4 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align { Start, Center, End }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill { None, Window, Surface, Field, Control, Accent }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle { Normal, Primary, Flat, Danger }

/// Rows behind a list, table or tree. Weft asks only for the rows it shows, so a
/// model may hold millions.
pub trait ListModel: Any {
    fn len(&self) -> usize;
    /// The text of `row` in `column` (column 0 for plain lists).
    fn cell(&self, row: usize, column: usize, out: &mut String);
    /// Tree depth of `row` (0 = top level).
    fn depth(&self, _row: usize) -> u8 { 0 }
    /// For trees: `Some(open)` when `row` has children.
    fn expandable(&self, _row: usize) -> Option<bool> { None }
    /// An icon name for `row`.
    fn icon(&self, _row: usize) -> Option<&str> { None }
}

/// The simplest model: rows of strings.
#[derive(Clone, Debug, Default)]
pub struct Rows(pub Vec<Vec<String>>);

impl Rows {
    pub fn from_strs(items: &[&str]) -> Rows { Rows(items.iter().map(|s| alloc::vec![String::from(*s)]).collect()) }
}

impl ListModel for Rows {
    fn len(&self) -> usize { self.0.len() }
    fn cell(&self, row: usize, column: usize, out: &mut String) {
        if let Some(c) = self.0.get(row).and_then(|r| r.get(column)) { out.push_str(c); }
    }
}

/// A table column.
#[derive(Clone, Debug)]
pub struct Column {
    pub title: String,
    /// Width in layout units; the last column takes what is left.
    pub width: f32,
    pub align: Align,
}

impl Column {
    pub fn new(title: &str, width: f32) -> Column { Column { title: title.into(), width, align: Align::Start } }
    pub fn right(mut self) -> Column { self.align = Align::End; self }
}

/// Which rows are selected, as sorted, disjoint inclusive ranges (a million rows
/// selected costs one range).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection { ranges: Vec<(usize, usize)> }

impl Selection {
    pub fn clear(&mut self) { self.ranges.clear(); }
    pub fn is_empty(&self) -> bool { self.ranges.is_empty() }
    pub fn contains(&self, row: usize) -> bool {
        // Binary search over the ranges.
        let i = self.ranges.partition_point(|r| r.1 < row);
        self.ranges.get(i).map_or(false, |r| r.0 <= row)
    }
    pub fn count(&self) -> usize { self.ranges.iter().map(|r| r.1 - r.0 + 1).sum() }
    pub fn rows(&self) -> impl Iterator<Item = usize> + '_ { self.ranges.iter().flat_map(|r| r.0..=r.1) }
    pub fn first(&self) -> Option<usize> { self.ranges.first().map(|r| r.0) }
    pub fn set(&mut self, row: usize) { self.ranges.clear(); self.ranges.push((row, row)); }
    pub fn set_range(&mut self, a: usize, b: usize) { self.ranges.clear(); self.ranges.push((a.min(b), a.max(b))); }
    pub fn add_range(&mut self, a: usize, b: usize) {
        let (mut lo, mut hi) = (a.min(b), a.max(b));
        let mut out = Vec::with_capacity(self.ranges.len() + 1);
        for &(s, e) in &self.ranges {
            if e + 1 < lo || s > hi + 1 { out.push((s, e)); } else { lo = lo.min(s); hi = hi.max(e); }
        }
        out.push((lo, hi));
        out.sort_unstable();
        self.ranges = out;
    }
    pub fn toggle(&mut self, row: usize) {
        if !self.contains(row) { return self.add_range(row, row); }
        let mut out = Vec::with_capacity(self.ranges.len() + 1);
        for &(s, e) in &self.ranges {
            if row < s || row > e { out.push((s, e)); continue; }
            if s < row { out.push((s, row - 1)); }
            if row < e { out.push((row + 1, e)); }
        }
        self.ranges = out;
    }
    /// Keeps rows below `len` only.
    pub fn clamp(&mut self, len: usize) {
        self.ranges.retain(|r| r.0 < len);
        if let Some(last) = self.ranges.last_mut() { last.1 = last.1.min(len.saturating_sub(1)); }
    }
}

pub(crate) struct ListState {
    pub model: Box<dyn ListModel>,
    pub columns: Vec<Column>,
    pub selection: Selection,
    pub cursor: usize,
    pub anchor: usize,
    pub multi: bool,
    pub scroll: f32,
    pub sort: Option<(usize, bool)>,
    /// Type-to-find: what was typed and when.
    pub find: String,
    pub find_at: u64,
    /// Scroll the cursor row into view at the next layout.
    pub reveal: bool,
}

/// One tab in a tab strip. Programs keep these to save and restore their tabs.
#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub title: String,
    pub icon: Option<String>,
    /// Pinned tabs sit first, show only their icon and can't be closed by a click.
    pub pinned: bool,
    /// A dot instead of the close button until hovered: unsaved changes.
    pub modified: bool,
    /// Tabs in a group share a coloured line.
    pub group: Option<Color>,
    pub tooltip: Option<String>,
}

impl Tab {
    pub fn new(title: &str) -> Tab { Tab { title: title.into(), icon: None, pinned: false, modified: false, group: None, tooltip: None } }
    pub fn icon(mut self, name: &str) -> Tab { self.icon = Some(name.into()); self }
}

pub(crate) struct TabsState {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub scroll: f32,
    pub closable: bool,
    pub reorderable: bool,
    pub new_button: bool,
    /// The strip answers Ctrl+Tab, Ctrl+W, Ctrl+T and Ctrl+1..9 anywhere in the window.
    pub document: bool,
    /// Tabs laid out at the last paint: (x, width) in strip coordinates.
    pub spans: Vec<(f32, f32)>,
    pub hover_close: Option<usize>,
    pub hover: Option<usize>,
    /// Width of all tabs laid end to end, and the room the strip has for them.
    pub total: f32,
    pub room: f32,
    /// Scroll the active tab into view at the next layout.
    pub reveal: bool,
}

pub(crate) struct FieldState {
    pub edit: LineEdit,
    pub placeholder: String,
    pub password: bool,
    pub scroll: f32,
}

pub(crate) struct MenuRowState {
    pub menu: usize,
    pub index: usize,
    pub label: String,
    pub shortcut: String,
    pub icon: Option<String>,
    pub check: Option<bool>,
    pub submenu: bool,
    pub separator: bool,
    pub enabled: bool,
}

pub(crate) enum Kind {
    Panel { fill: Fill, border: bool, radius: f32 },
    Label { text: String, style: TextStyle, wrap: bool, align: Align, color: Option<Color> },
    Button { text: String, icon: Option<String>, style: ButtonStyle },
    Field(Box<FieldState>),
    Check { text: String, on: bool, switch: bool },
    Radio { text: String, group: u32, on: bool },
    Slider { value: f32, min: f32, max: f32, step: f32 },
    Progress { value: Option<f32> },
    Scroll { y: f32, content: f32 },
    List(Box<ListState>),
    Tabs(Box<TabsState>),
    Split { vertical: bool, ratio: f32 },
    Icon { name: String, size: f32 },
    Image { w: usize, h: usize, pixels: Vec<u32> },
    Separator,
    MenuBar { menus: Vec<Menu>, open: Option<usize> },
    /// A row of an open menu (internal): which open menu and which item, and what it shows.
    MenuRow(Box<MenuRowState>),
}

impl Kind {
    pub fn focusable(&self) -> bool {
        matches!(self, Kind::Button { .. } | Kind::Field(_) | Kind::Check { .. } | Kind::Radio { .. } | Kind::Slider { .. } | Kind::List(_) | Kind::Tabs(_) | Kind::MenuBar { .. })
    }
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Panel { .. } => "panel", Kind::Label { .. } => "label", Kind::Button { .. } => "button", Kind::Field(_) => "field",
            Kind::Check { switch: true, .. } => "switch", Kind::Check { .. } => "check", Kind::Radio { .. } => "radio", Kind::Slider { .. } => "slider",
            Kind::Progress { .. } => "progress", Kind::Scroll { .. } => "scroll", Kind::List(_) => "list", Kind::Tabs(_) => "tabs",
            Kind::Split { .. } => "split", Kind::Icon { .. } => "icon", Kind::Image { .. } => "image", Kind::Separator => "separator",
            Kind::MenuBar { .. } => "menubar", Kind::MenuRow(_) => "menurow",
        }
    }
}

/// How a widget sits in its parent: a small builder over taffy's flexbox and grid.
/// Values are layout units.
#[derive(Clone, Debug)]
pub struct Layout(pub(crate) Style);

impl Default for Layout {
    fn default() -> Self { Layout(Style::default()) }
}

impl Layout {
    /// Children stacked top to bottom.
    pub fn column() -> Layout { Layout(Style { display: Display::Flex, flex_direction: FlexDirection::Column, ..Default::default() }) }
    /// Children side by side.
    pub fn row() -> Layout { Layout(Style { display: Display::Flex, flex_direction: FlexDirection::Row, align_items: AlignItems::CENTER, ..Default::default() }) }
    /// A grid of `columns` equal columns; children fill it row by row.
    pub fn grid(columns: u16) -> Layout {
        Layout(Style { display: Display::Grid, grid_template_columns: evenly_sized_tracks(columns), ..Default::default() })
    }
    pub fn gap(mut self, g: f32) -> Layout { self.0.gap = Size { width: length(g), height: length(g) }; self }
    pub fn padding(mut self, p: f32) -> Layout { self.0.padding = taffy::Rect { left: length(p), right: length(p), top: length(p), bottom: length(p) }; self }
    pub fn padding_xy(mut self, x: f32, y: f32) -> Layout { self.0.padding = taffy::Rect { left: length(x), right: length(x), top: length(y), bottom: length(y) }; self }
    pub fn grow(mut self, g: f32) -> Layout { self.0.flex_grow = g; self }
    pub fn shrink(mut self, s: f32) -> Layout { self.0.flex_shrink = s; self }
    pub fn width(mut self, w: f32) -> Layout { self.0.size.width = length(w); self }
    pub fn height(mut self, h: f32) -> Layout { self.0.size.height = length(h); self }
    pub fn min_width(mut self, w: f32) -> Layout { self.0.min_size.width = length(w); self }
    pub fn min_height(mut self, h: f32) -> Layout { self.0.min_size.height = length(h); self }
    pub fn max_width(mut self, w: f32) -> Layout { self.0.max_size.width = length(w); self }
    /// Fills the parent's width (and grows along a column).
    pub fn fill(mut self) -> Layout { self.0.flex_grow = 1.0; self.0.align_self = Some(AlignSelf::STRETCH); self.0.min_size.height = length(0.0); self.0.min_size.width = length(0.0); self }
    pub fn fill_width(mut self) -> Layout { self.0.size.width = percent(1.0); self }
    pub fn center(mut self) -> Layout { self.0.align_items = AlignItems::CENTER; self.0.justify_content = JustifyContent::CENTER; self }
    pub fn align_items(mut self, a: AlignItems) -> Layout { self.0.align_items = a; self }
    pub fn justify(mut self, j: JustifyContent) -> Layout { self.0.justify_content = j; self }
    /// Spans `n` grid columns.
    pub fn span(mut self, n: u16) -> Layout { self.0.grid_column = Line { start: GridPlacement::Auto, end: GridPlacement::Span(n) }; self }
    pub fn wrap(mut self) -> Layout { self.0.flex_wrap = FlexWrap::Wrap; self }
    /// The taffy style itself, for anything this builder doesn't name.
    pub fn style(&mut self) -> &mut Style { &mut self.0 }
}
