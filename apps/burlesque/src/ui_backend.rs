// SPDX-License-Identifier: GPL-3.0-only
//! Weft-compatible UI backend for Burlesque on SexOS and host.
//! Built on sex-font and sex-canvas, providing full window and widget capabilities
//! without depending on the unmerged crates/weft.

#![allow(dead_code)]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

pub mod widget {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Id(pub u32);

    impl Id {
        pub fn index(self) -> u32 {
            self.0
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum TextStyle {
        Title,
        Heading,
        Body,
        Caption,
        Display,
        Mono,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum FlexDirection {
        Row,
        Column,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Layout {
        pub flex_direction: FlexDirection,
        pub width: Option<f32>,
        pub height: Option<f32>,
        pub grow: f32,
        pub shrink: f32,
        pub gap: f32,
        pub padding: f32,
        pub fill_parent: bool,
    }

    impl Default for Layout {
        fn default() -> Self {
            Self {
                flex_direction: FlexDirection::Column,
                width: None,
                height: None,
                grow: 0.0,
                shrink: 0.0,
                gap: 6.0,
                padding: 6.0,
                fill_parent: false,
            }
        }
    }

    impl Layout {
        pub fn column() -> Self {
            Self {
                flex_direction: FlexDirection::Column,
                ..Default::default()
            }
        }
        pub fn row() -> Self {
            Self {
                flex_direction: FlexDirection::Row,
                ..Default::default()
            }
        }
        pub fn width(mut self, w: f32) -> Self {
            self.width = Some(w);
            self
        }
        pub fn height(mut self, h: f32) -> Self {
            self.height = Some(h);
            self
        }
        pub fn grow(mut self, g: f32) -> Self {
            self.grow = g;
            self
        }
        pub fn shrink(mut self, s: f32) -> Self {
            self.shrink = s;
            self
        }
        pub fn gap(mut self, g: f32) -> Self {
            self.gap = g;
            self
        }
        pub fn padding(mut self, p: f32) -> Self {
            self.padding = p;
            self
        }
        pub fn fill(mut self) -> Self {
            self.fill_parent = true;
            self.grow = 1.0;
            self
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct Rows(pub Vec<Vec<String>>);

    impl Rows {
        pub fn from_strs(items: &[&str]) -> Self {
            Rows(items.iter().map(|s| vec![s.to_string()]).collect())
        }
    }
}

pub mod geom {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Point {
        pub x: f32,
        pub y: f32,
    }

    impl Point {
        pub const fn new(x: f32, y: f32) -> Self {
            Self { x, y }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Rect {
        pub x: f32,
        pub y: f32,
        pub w: f32,
        pub h: f32,
    }

    impl Rect {
        pub fn center(&self) -> Point {
            Point {
                x: self.x + self.w * 0.5,
                y: self.y + self.h * 0.5,
            }
        }
        pub fn contains(&self, p: Point) -> bool {
            p.x >= self.x && p.x <= self.x + self.w && p.y >= self.y && p.y <= self.y + self.h
        }
    }
}

pub mod input {

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Button {
        Left,
        Right,
        Middle,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum PointerKind {
        Move,
        Down(Button),
        Up(Button),
        Wheel(f32),
        Drag,
        Leave,
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Pointer {
        pub x: f32,
        pub y: f32,
        pub kind: PointerKind,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct Key {
        pub code: u8,
        pub shift: bool,
        pub control: bool,
        pub text: Option<char>,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum Event {
        Key(Key),
        Close,
        Resize { width: u32, height: u32 },
        Pointer(Pointer),
        Drop { kind: u8, x: i16, y: i16 },
        DragEnded { target: u32 },
    }
}

pub mod theme {
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Theme {
        pub scale: f32,
        pub bg: u32,
        pub card_bg: u32,
        pub text: u32,
        pub text_muted: u32,
        pub button: u32,
        pub button_hover: u32,
        pub button_primary: u32,
        pub border: u32,
        pub accent: u32,
    }

    impl Theme {
        pub fn light() -> Self {
            Self {
                scale: 1.0,
                bg: 0xFFF1F5F9,           // Slate 100
                card_bg: 0xFFFFFFFF,      // White
                text: 0xFF0F172A,         // Slate 900
                text_muted: 0xFF475569,   // Slate 600
                button: 0xFFE2E8F0,       // Slate 200
                button_hover: 0xFFCBD5E1, // Slate 300
                button_primary: 0xFF2563EB, // Blue 600
                border: 0xFF94A3B8,       // Slate 400
                accent: 0xFF3B82F6,       // Blue 500
            }
        }
    }
}

pub mod i18n {
    use alloc::string::{String, ToString};
    #[derive(Clone, Debug)]
    pub struct Strings {
        pub lang: String,
    }
    impl Strings {
        pub fn new(lang: &str) -> Self {
            Self {
                lang: lang.to_string(),
            }
        }
    }
}

pub mod menu {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    pub struct Command {
        pub id: String,
        pub label: String,
        pub shortcut: Option<String>,
    }

    impl Command {
        pub fn new(id: &str, label: &str) -> Self {
            Self {
                id: id.to_string(),
                label: label.to_string(),
                shortcut: None,
            }
        }
        pub fn shortcut(mut self, chord: &str) -> Self {
            self.shortcut = Some(chord.to_string());
            self
        }
    }

    #[derive(Clone, Debug, Default)]
    pub struct Commands {
        list: Vec<Command>,
    }

    impl Commands {
        pub fn add(&mut self, c: Command) {
            if let Some(x) = self.list.iter_mut().find(|x| x.id == c.id) {
                *x = c;
            } else {
                self.list.push(c);
            }
        }
        pub fn find(&self, id: &str) -> Option<&Command> {
            self.list.iter().find(|c| c.id == id)
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum MenuItem {
        Command(String),
        Submenu(Menu),
        Separator,
    }

    #[derive(Clone, Debug, PartialEq, Default)]
    pub struct Menu {
        pub title: String,
        pub items: Vec<MenuItem>,
    }

    impl Menu {
        pub fn new(title: &str) -> Self {
            Self {
                title: title.to_string(),
                items: Vec::new(),
            }
        }
        pub fn command(mut self, id: &str) -> Self {
            self.items.push(MenuItem::Command(id.to_string()));
            self
        }
        pub fn submenu(mut self, m: Menu) -> Self {
            self.items.push(MenuItem::Submenu(m));
            self
        }
        pub fn separator(mut self) -> Self {
            self.items.push(MenuItem::Separator);
            self
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Clicked(widget::Id),
    Command(String),
    Changed(widget::Id),
    Submitted(widget::Id),
    Close,
}

pub use geom::{Point, Rect};
pub use i18n::Strings;
pub use input::{Button, Event, Key, Pointer, PointerKind};
pub use menu::{Command, Menu};
pub use theme::Theme;
pub use widget::{Id, Layout, Rows, TextStyle};

#[derive(Clone, Debug)]
pub enum NodeKind {
    Root,
    Row,
    Column,
    Button { text: String },
    Label { text: String },
    Text { text: String, style: TextStyle },
    Paragraph { text: String },
    Separator,
    Field { text: String, placeholder: String },
    List { rows: Vec<String>, selected: usize },
    Image { w: usize, h: usize, pixels: Vec<u32> },
    MenuBar { menus: Vec<Menu> },
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: Id,
    pub kind: NodeKind,
    pub parent: Option<Id>,
    pub children: Vec<Id>,
    pub layout: Layout,
    pub rect: Rect,
    pub visible: bool,
    pub enabled: bool,
}

pub struct Ui {
    pub nodes: Vec<Option<Node>>,
    pub commands: menu::Commands,
    theme: Theme,
    strings: Strings,
    size: (u32, u32),
    dirty: bool,
    hovered: Option<Id>,
    pressed: Option<Id>,
}

impl Ui {
    pub fn new(w: u32, h: u32, theme: Theme, strings: Strings) -> Self {
        let root = Node {
            id: Id(0),
            kind: NodeKind::Root,
            parent: None,
            children: Vec::new(),
            layout: Layout::column(),
            rect: Rect {
                x: 10.0,
                y: 8.0,
                w: (w as f32 - 20.0).max(100.0),
                h: (h as f32 - 16.0).max(100.0),
            },
            visible: true,
            enabled: true,
        };
        Self {
            nodes: vec![Some(root)],
            commands: menu::Commands::default(),
            theme,
            strings,
            size: (w, h),
            dirty: true,
            hovered: None,
            pressed: None,
        }
    }

    pub fn root(&self) -> Id {
        Id(0)
    }

    fn alloc_node(&mut self, parent: Id, kind: NodeKind, layout: Layout) -> Id {
        let id = Id(self.nodes.len() as u32);
        let node = Node {
            id,
            kind,
            parent: Some(parent),
            children: Vec::new(),
            layout,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
            visible: true,
            enabled: true,
        };
        self.nodes.push(Some(node));
        if let Some(Some(p)) = self.nodes.get_mut(parent.0 as usize) {
            p.children.push(id);
        }
        self.dirty = true;
        id
    }

    pub fn row(&mut self, parent: Id) -> Id {
        self.alloc_node(parent, NodeKind::Row, Layout::row())
    }

    pub fn column(&mut self, parent: Id) -> Id {
        self.alloc_node(parent, NodeKind::Column, Layout::column())
    }

    pub fn button(&mut self, parent: Id, text: &str) -> Id {
        self.alloc_node(
            parent,
            NodeKind::Button {
                text: text.to_string(),
            },
            Layout::default(),
        )
    }

    pub fn label(&mut self, parent: Id, text: &str) -> Id {
        self.alloc_node(
            parent,
            NodeKind::Label {
                text: text.to_string(),
            },
            Layout::default(),
        )
    }

    pub fn text(&mut self, parent: Id, text: &str, style: TextStyle) -> Id {
        self.alloc_node(
            parent,
            NodeKind::Text {
                text: text.to_string(),
                style,
            },
            Layout::default(),
        )
    }

    pub fn paragraph(&mut self, parent: Id, text: &str) -> Id {
        self.alloc_node(
            parent,
            NodeKind::Paragraph {
                text: text.to_string(),
            },
            Layout::default(),
        )
    }

    pub fn separator(&mut self, parent: Id) {
        let _ = self.alloc_node(parent, NodeKind::Separator, Layout::default());
    }

    pub fn field(&mut self, parent: Id, value: &str, placeholder: &str) -> Id {
        self.alloc_node(
            parent,
            NodeKind::Field {
                text: value.to_string(),
                placeholder: placeholder.to_string(),
            },
            Layout::default(),
        )
    }

    pub fn list(&mut self, parent: Id, items: &[&str]) -> Id {
        self.alloc_node(
            parent,
            NodeKind::List {
                rows: items.iter().map(|s| s.to_string()).collect(),
                selected: 0,
            },
            Layout::default(),
        )
    }

    pub fn image(&mut self, parent: Id, w: usize, h: usize, pixels: Vec<u32>) -> Id {
        self.alloc_node(
            parent,
            NodeKind::Image { w, h, pixels },
            Layout::default().width(w as f32).height(h as f32),
        )
    }

    pub fn menubar(&mut self, parent: Id, menus: impl AsRef<[Menu]>) {
        let _ = self.alloc_node(
            parent,
            NodeKind::MenuBar {
                menus: menus.as_ref().to_vec(),
            },
            Layout::default(),
        );
    }

    pub fn set_layout(&mut self, id: Id, layout: Layout) {
        if let Some(Some(n)) = self.nodes.get_mut(id.0 as usize) {
            n.layout = layout;
            self.layout();
        }
    }

    pub fn set_text(&mut self, id: Id, text: &str) {
        let mut changed = false;
        if let Some(Some(n)) = self.nodes.get_mut(id.0 as usize) {
            match &mut n.kind {
                NodeKind::Button { text: t }
                | NodeKind::Label { text: t }
                | NodeKind::Text { text: t, .. }
                | NodeKind::Paragraph { text: t }
                | NodeKind::Field { text: t, .. } => {
                    *t = text.to_string();
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            self.layout();
        }
    }

    pub fn text_of(&self, id: Id) -> &str {
        if let Some(Some(n)) = self.nodes.get(id.0 as usize) {
            match &n.kind {
                NodeKind::Button { text: t }
                | NodeKind::Label { text: t }
                | NodeKind::Text { text: t, .. }
                | NodeKind::Paragraph { text: t }
                | NodeKind::Field { text: t, .. } => t.as_str(),
                _ => "",
            }
        } else {
            ""
        }
    }

    pub fn set_image(&mut self, id: Id, w: usize, h: usize, pixels: Vec<u32>) {
        if let Some(Some(n)) = self.nodes.get_mut(id.0 as usize) {
            n.kind = NodeKind::Image { w, h, pixels };
            n.layout.width = Some(w as f32);
            n.layout.height = Some(h as f32);
            self.layout();
        }
    }

    pub fn set_rows(&mut self, id: Id, rows: Rows) {
        if let Some(Some(n)) = self.nodes.get_mut(id.0 as usize) {
            if let NodeKind::List { rows: r, .. } = &mut n.kind {
                *r = rows.0.into_iter().flatten().collect();
                self.layout();
            }
        }
    }

    pub fn set_visible(&mut self, id: Id, visible: bool) {
        if let Some(Some(n)) = self.nodes.get_mut(id.0 as usize) {
            if n.visible != visible {
                n.visible = visible;
                self.layout();
            }
        }
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn rect(&self, id: Id) -> Rect {
        if let Some(Some(n)) = self.nodes.get(id.0 as usize) {
            n.rect
        } else {
            Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            }
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn needs_paint(&self) -> bool {
        self.dirty
    }

    pub fn layout(&mut self) {
        let (w, h) = self.size;
        let root_rect = Rect {
            x: 12.0,
            y: 8.0,
            w: (w as f32 - 24.0).max(100.0),
            h: (h as f32 - 16.0).max(100.0),
        };
        if let Some(Some(r)) = self.nodes.get_mut(0) {
            r.rect = root_rect;
        }
        self.layout_node(Id(0));
        self.dirty = false;
    }

    fn layout_node(&mut self, id: Id) {
        let (children, is_row, parent_rect, gap, padding) = {
            let Some(Some(node)) = self.nodes.get(id.0 as usize) else {
                return;
            };
            if !node.visible {
                return;
            }
            let is_row = node.layout.flex_direction == widget::FlexDirection::Row
                || matches!(node.kind, NodeKind::Row);
            (
                node.children.clone(),
                is_row,
                node.rect,
                node.layout.gap,
                node.layout.padding,
            )
        };

        if is_row {
            let mut curr_x = parent_rect.x + padding;
            let curr_y = parent_rect.y + padding;
            for child_id in children {
                let Some(Some(child)) = self.nodes.get_mut(child_id.0 as usize) else {
                    continue;
                };
                if !child.visible {
                    child.rect = Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 0.0,
                        h: 0.0,
                    };
                    continue;
                }

                let (w, h) = match &child.kind {
                    NodeKind::Button { text } => {
                        let w = child
                            .layout
                            .width
                            .unwrap_or_else(|| (text.len() as f32 * 8.5 + 24.0).max(64.0));
                        let h = child.layout.height.unwrap_or(28.0);
                        (w, h)
                    }
                    NodeKind::Label { text } => {
                        let w = child
                            .layout
                            .width
                            .unwrap_or_else(|| (text.len() as f32 * 8.0 + 8.0).max(40.0));
                        let h = child.layout.height.unwrap_or(24.0);
                        (w, h)
                    }
                    NodeKind::Separator => (6.0, child.layout.height.unwrap_or(24.0)),
                    NodeKind::Image { w, h, .. } => {
                        let cw = child.layout.width.unwrap_or(*w as f32);
                        let ch = child.layout.height.unwrap_or(*h as f32);
                        (cw, ch)
                    }
                    NodeKind::Column => {
                        let cw = child.layout.width.unwrap_or(180.0);
                        let ch = child.layout.height.unwrap_or(parent_rect.h - 2.0 * padding);
                        (cw, ch)
                    }
                    _ => {
                        let w = child.layout.width.unwrap_or(80.0);
                        let h = child.layout.height.unwrap_or(28.0);
                        (w, h)
                    }
                };

                child.rect = Rect {
                    x: curr_x,
                    y: curr_y,
                    w,
                    h,
                };
                curr_x += w + gap;
                self.layout_node(child_id);
            }
        } else {
            // Column
            let mut curr_y = parent_rect.y + padding;
            let curr_x = parent_rect.x + padding;
            let avail_w = (parent_rect.w - 2.0 * padding).max(50.0);

            for child_id in children {
                let Some(Some(child)) = self.nodes.get_mut(child_id.0 as usize) else {
                    continue;
                };
                if !child.visible {
                    child.rect = Rect {
                        x: 0.0,
                        y: 0.0,
                        w: 0.0,
                        h: 0.0,
                    };
                    continue;
                }

                let (w, h) = match &child.kind {
                    NodeKind::Button { text } => {
                        let w = child
                            .layout
                            .width
                            .unwrap_or_else(|| (text.len() as f32 * 8.5 + 24.0).max(64.0));
                        let h = child.layout.height.unwrap_or(28.0);
                        (w, h)
                    }
                    NodeKind::Label { text } => {
                        let w = child
                            .layout
                            .width
                            .unwrap_or_else(|| (text.len() as f32 * 8.0 + 8.0).min(avail_w));
                        let h = child.layout.height.unwrap_or(22.0);
                        (w, h)
                    }
                    NodeKind::Text { text, style } => {
                        let h = match style {
                            TextStyle::Title => 30.0,
                            TextStyle::Heading => 26.0,
                            _ => 22.0,
                        };
                        let w = child
                            .layout
                            .width
                            .unwrap_or_else(|| (text.len() as f32 * 9.0 + 10.0).min(avail_w));
                        (w, h)
                    }
                    NodeKind::Paragraph { text } => {
                        let lines = text.lines().count().max(1);
                        let h = child
                            .layout
                            .height
                            .unwrap_or_else(|| (lines as f32 * 18.0 + 20.0).max(40.0));
                        let w = child.layout.width.unwrap_or(avail_w);
                        (w, h)
                    }
                    NodeKind::Separator => (avail_w, 4.0),
                    NodeKind::Field { .. } => {
                        let w = child.layout.width.unwrap_or(avail_w.min(220.0));
                        let h = child.layout.height.unwrap_or(28.0);
                        (w, h)
                    }
                    NodeKind::List { .. } => {
                        let w = child.layout.width.unwrap_or(avail_w.min(220.0));
                        let h = child.layout.height.unwrap_or(140.0);
                        (w, h)
                    }
                    NodeKind::Image { w, h, .. } => {
                        let cw = child.layout.width.unwrap_or(*w as f32);
                        let ch = child.layout.height.unwrap_or(*h as f32);
                        (cw, ch)
                    }
                    NodeKind::Row => {
                        let w = child.layout.width.unwrap_or(avail_w);
                        let h = child.layout.height.unwrap_or(36.0);
                        (w, h)
                    }
                    NodeKind::Column => {
                        let w = child.layout.width.unwrap_or(avail_w);
                        let h = child.layout.height.unwrap_or(200.0);
                        (w, h)
                    }
                    NodeKind::MenuBar { .. } => (avail_w, 26.0),
                    _ => (avail_w, 30.0),
                };

                child.rect = Rect {
                    x: curr_x,
                    y: curr_y,
                    w,
                    h,
                };
                curr_y += h + gap;
                self.layout_node(child_id);
            }
        }
    }

    pub fn handle(&mut self, event: Event) -> Vec<Response> {
        self.layout();
        let mut responses = Vec::new();

        match event {
            Event::Close => responses.push(Response::Close),
            Event::Resize { width, height } => {
                self.size = (width, height);
                self.layout();
            }
            Event::Pointer(p) => {
                let point = Point { x: p.x, y: p.y };
                match p.kind {
                    PointerKind::Move => {
                        self.hovered = self.hit_test(point);
                    }
                    PointerKind::Down(button) => {
                        if button == Button::Left {
                            self.pressed = self.hit_test(point);
                        }
                    }
                    PointerKind::Up(button) => {
                        if button == Button::Left {
                            let hit = self.hit_test(point);
                            if let Some(id) = hit {
                                if let Some(Some(node)) = self.nodes.get(id.0 as usize) {
                                    if node.enabled && node.visible {
                                        match &node.kind {
                                            NodeKind::Button { .. } => {
                                                responses.push(Response::Clicked(id));
                                            }
                                            NodeKind::List { .. } => {
                                                responses.push(Response::Clicked(id));
                                                responses.push(Response::Changed(id));
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            self.pressed = None;
                        }
                    }
                    _ => {}
                }
            }
            Event::Key(_k) => {
                // If enter or return on field, emit submitted
            }
            _ => {}
        }
        responses
    }

    fn hit_test(&self, p: Point) -> Option<Id> {
        // Find deepest visible enabled node containing p
        for id in (0..self.nodes.len()).rev() {
            if let Some(Some(node)) = self.nodes.get(id) {
                if node.visible && node.enabled && node.rect.contains(p) {
                    if matches!(
                        node.kind,
                        NodeKind::Button { .. }
                            | NodeKind::Field { .. }
                            | NodeKind::List { .. }
                            | NodeKind::Image { .. }
                    ) {
                        return Some(Id(id as u32));
                    }
                }
            }
        }
        None
    }

    pub fn tick(&mut self, _now_ms: u64) -> Option<u64> {
        None
    }

    pub fn paint(&mut self, pixels: &mut [u32]) -> Option<()> {
        self.layout();
        let (win_w, win_h) = (self.size.0 as usize, self.size.1 as usize);
        if pixels.len() < win_w * win_h {
            return None;
        }

        // Fill background
        pixels[..win_w * win_h].fill(self.theme.bg);

        // Header accent bar
        fill_rect(pixels, win_w, win_h, 0, 0, win_w as i32, 4, 0xFF3B82F6);

        // Render each node
        for i in 0..self.nodes.len() {
            let Some(Some(node)) = self.nodes.get(i) else {
                continue;
            };
            if !node.visible {
                continue;
            }

            let rx = node.rect.x as i32;
            let ry = node.rect.y as i32;
            let rw = node.rect.w as i32;
            let rh = node.rect.h as i32;

            match &node.kind {
                NodeKind::Button { text } => {
                    let is_hover = self.hovered == Some(node.id);
                    let (bg, border, fg) = if text.contains("Run Suite") || text.contains("Benchmarks") {
                        (0xFF2563EB, 0xFF1D4ED8, 0xFFFFFFFF)
                    } else if text.contains("Thermal") || text.contains("Start Step") || text.contains("Burst") {
                        (0xFF059669, 0xFF047857, 0xFFFFFFFF)
                    } else if text.contains("Reset") || text.contains("Delete") {
                        (0xFFDC2626, 0xFFB91C1C, 0xFFFFFFFF)
                    } else if text == "Red" {
                        (0xFFEF4444, 0xFFB91C1C, 0xFFFFFFFF)
                    } else if text == "Green" {
                        (0xFF22C55E, 0xFF15803D, 0xFFFFFFFF)
                    } else if text == "Blue" {
                        (0xFF3B82F6, 0xFF1D4ED8, 0xFFFFFFFF)
                    } else if text == "Yellow" {
                        (0xFFEAB308, 0xFFA16207, 0xFF000000)
                    } else if text == "Black" {
                        (0xFF18181B, 0xFF09090B, 0xFFFFFFFF)
                    } else if text == "White" {
                        (0xFFFFFFFF, 0xFF94A3B8, 0xFF0F172A)
                    } else if is_hover {
                        (self.theme.button_hover, self.theme.border, self.theme.text)
                    } else {
                        (self.theme.button, self.theme.border, self.theme.text)
                    };

                    fill_rect(pixels, win_w, win_h, rx, ry, rw, rh, bg);
                    stroke_rect(pixels, win_w, win_h, rx, ry, rw, rh, border);

                    // Text centered in button
                    let tx = rx + (rw - (text.len() as i32 * 8)) / 2;
                    let ty = ry + (rh - 14) / 2;
                    draw_text(pixels, win_w, win_h, tx, ty, text, fg);
                }
                NodeKind::Text { text, style } => {
                    let fg = match style {
                        TextStyle::Title => 0xFF0F172A,
                        TextStyle::Heading => 0xFF1E293B,
                        _ => 0xFF334155,
                    };
                    draw_text(pixels, win_w, win_h, rx + 2, ry + 4, text, fg);
                }
                NodeKind::Label { text } => {
                    let fg = 0xFF475569;
                    draw_text(pixels, win_w, win_h, rx + 2, ry + 2, text, fg);
                }
                NodeKind::Paragraph { text } => {
                    // Draw panel background and border
                    fill_rect(pixels, win_w, win_h, rx, ry, rw, rh, 0xFF1E293B);
                    stroke_rect(pixels, win_w, win_h, rx, ry, rw, rh, 0xFF334155);

                    let mut ty = ry + 8;
                    for line in text.lines() {
                        if ty + 16 > ry + rh {
                            break;
                        }
                        let fg = if line.starts_with("===") {
                            0xFFF8FAFC
                        } else if line.starts_with('•') {
                            0xFF38BDF8
                        } else if line.starts_with("Composite") {
                            0xFF34D399
                        } else if line.contains("THROTTLED") || line.contains("FAIL") {
                            0xFFF87171
                        } else {
                            0xFFCBD5E1
                        };
                        draw_text(pixels, win_w, win_h, rx + 8, ty, line, fg);
                        ty += 16;
                    }
                }
                NodeKind::Separator => {
                    fill_rect(pixels, win_w, win_h, rx, ry, rw, rh, 0xFFCBD5E1);
                }
                NodeKind::Field { text, placeholder } => {
                    fill_rect(pixels, win_w, win_h, rx, ry, rw, rh, 0xFFFFFFFF);
                    stroke_rect(pixels, win_w, win_h, rx, ry, rw, rh, 0xFF94A3B8);
                    let max_chars = ((rw - 12).max(0) / 8) as usize;
                    let (raw, fg) = if text.is_empty() {
                        (placeholder.as_str(), 0xFF94A3B8)
                    } else {
                        (text.as_str(), 0xFF0F172A)
                    };
                    let clipped: String = raw
                        .chars()
                        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                        .take(max_chars)
                        .collect();
                    draw_text(pixels, win_w, win_h, rx + 6, ry + 8, &clipped, fg);
                }
                NodeKind::List { rows, selected } => {
                    fill_rect(pixels, win_w, win_h, rx, ry, rw, rh, 0xFFFFFFFF);
                    stroke_rect(pixels, win_w, win_h, rx, ry, rw, rh, 0xFF94A3B8);
                    let max_chars = ((rw - 12).max(0) / 8) as usize;
                    let mut item_y = ry + 4;
                    for (idx, item) in rows.iter().enumerate() {
                        if item_y + 18 > ry + rh {
                            break;
                        }
                        let clipped: String = item.chars().take(max_chars).collect();
                        if idx == *selected {
                            fill_rect(pixels, win_w, win_h, rx + 2, item_y, rw - 4, 18, 0xFFDBEAFE);
                            draw_text(pixels, win_w, win_h, rx + 6, item_y + 3, &clipped, 0xFF1D4ED8);
                        } else {
                            draw_text(pixels, win_w, win_h, rx + 6, item_y + 3, &clipped, 0xFF334155);
                        }
                        item_y += 20;
                    }
                }
                NodeKind::Image { w, h, pixels: src } => {
                    stroke_rect(pixels, win_w, win_h, rx - 1, ry - 1, *w as i32 + 2, *h as i32 + 2, 0xFF94A3B8);
                    let iw = *w;
                    let ih = *h;
                    for row in 0..ih {
                        let dst_y = ry + row as i32;
                        if dst_y < 0 || dst_y >= win_h as i32 {
                            continue;
                        }
                        let dst_row = dst_y as usize * win_w;
                        let src_row = row * iw;
                        for col in 0..iw {
                            let dst_x = rx + col as i32;
                            if dst_x >= 0 && dst_x < win_w as i32 {
                                let px = src[src_row + col];
                                pixels[dst_row + dst_x as usize] = px;
                            }
                        }
                    }
                }
                NodeKind::MenuBar { menus } => {
                    fill_rect(pixels, win_w, win_h, rx, ry, rw, rh, 0xFF1E293B);
                    let mut mx = rx + 8;
                    for m in menus {
                        draw_text(pixels, win_w, win_h, mx, ry + 6, &m.title, 0xFFF8FAFC);
                        mx += (m.title.len() as i32 * 8) + 16;
                    }
                }
                _ => {}
            }
        }

        Some(())
    }
}

fn fill_rect(pixels: &mut [u32], win_w: usize, win_h: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let x0 = x.clamp(0, win_w as i32) as usize;
    let y0 = y.clamp(0, win_h as i32) as usize;
    let x1 = (x + w).clamp(0, win_w as i32) as usize;
    let y1 = (y + h).clamp(0, win_h as i32) as usize;
    for row in y0..y1 {
        let start = row * win_w + x0;
        let end = row * win_w + x1;
        if start < pixels.len() && end <= pixels.len() {
            pixels[start..end].fill(color);
        }
    }
}

fn stroke_rect(pixels: &mut [u32], win_w: usize, win_h: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    fill_rect(pixels, win_w, win_h, x, y, w, 1, color);
    fill_rect(pixels, win_w, win_h, x, y + h - 1, w, 1, color);
    fill_rect(pixels, win_w, win_h, x, y, 1, h, color);
    fill_rect(pixels, win_w, win_h, x + w - 1, y, 1, h, color);
}

fn draw_text(pixels: &mut [u32], win_w: usize, win_h: usize, x: i32, y: i32, text: &str, color: u32) {
    let mut cx = x;
    for ch in text.chars() {
        if cx + 8 > win_w as i32 {
            break;
        }
        draw_char(pixels, win_w, win_h, cx, y, ch, color);
        cx += 8;
    }
}

fn draw_char(pixels: &mut [u32], win_w: usize, win_h: usize, x: i32, y: i32, ch: char, color: u32) {
    let bitmap = glyph_bits(ch);
    for row in 0..12 {
        let py = y + row as i32;
        if py < 0 || py >= win_h as i32 {
            continue;
        }
        let bits = bitmap[row];
        for col in 0..8 {
            let px = x + col as i32;
            if px >= 0 && px < win_w as i32 && (bits & (0x80 >> col)) != 0 {
                pixels[py as usize * win_w + px as usize] = color;
            }
        }
    }
}

/// Compact 8x12 readable bitmap glyphs for common ASCII characters.
fn glyph_bits(ch: char) -> [u8; 12] {
    let code = ch as u32;
    match code {
        0x20 => [0; 12],
        0x21 => [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x18, 0, 0], // !
        0x22 => [0x66, 0x66, 0x66, 0x00, 0, 0, 0, 0, 0, 0, 0, 0],                   // "
        0x23 => [0x6C, 0x6C, 0xFE, 0x6C, 0xFE, 0x6C, 0x6C, 0, 0, 0, 0, 0],           // #
        0x25 => [0x62, 0x66, 0x0C, 0x18, 0x30, 0x66, 0x46, 0, 0, 0, 0, 0],           // %
        0x26 => [0x38, 0x6C, 0x38, 0x76, 0xDC, 0xCC, 0x76, 0, 0, 0, 0, 0],           // &
        0x27 => [0x18, 0x18, 0x30, 0, 0, 0, 0, 0, 0, 0, 0, 0],                       // '
        0x28 => [0x0C, 0x18, 0x30, 0x30, 0x30, 0x30, 0x30, 0x18, 0x0C, 0, 0, 0],     // (
        0x29 => [0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x18, 0x30, 0, 0, 0],     // )
        0x2A | 0x2022 => [0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0, 0, 0, 0, 0, 0],      // * or bullet
        0x2B => [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0, 0, 0, 0, 0, 0],              // +
        0x2C => [0x00, 0, 0, 0, 0, 0, 0, 0x18, 0x18, 0x30, 0, 0],                     // ,
        0x2D | 0x2014 => [0x00, 0, 0, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0],                  // - or em-dash
        0x2E => [0x00, 0, 0, 0, 0, 0, 0, 0x18, 0x18, 0, 0, 0],                         // .
        0x2F => [0x02, 0x06, 0x0C, 0x18, 0x30, 0x60, 0xC0, 0, 0, 0, 0, 0],           // /
        0x30 => [0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // 0
        0x31 => [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0, 0, 0, 0, 0],           // 1
        0x32 => [0x3C, 0x66, 0x06, 0x0C, 0x18, 0x30, 0x7E, 0, 0, 0, 0, 0],           // 2
        0x33 => [0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0, 0, 0, 0, 0],           // 3
        0x34 => [0x0C, 0x1C, 0x34, 0x64, 0x7E, 0x04, 0x04, 0, 0, 0, 0, 0],           // 4
        0x35 => [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0, 0, 0, 0, 0],           // 5
        0x36 => [0x1C, 0x30, 0x60, 0x7C, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // 6
        0x37 => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x30, 0x30, 0, 0, 0, 0, 0],           // 7
        0x38 => [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // 8
        0x39 => [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x0C, 0x38, 0, 0, 0, 0, 0],           // 9
        0x3A => [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0, 0, 0, 0, 0, 0],              // :
        0x3B => [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x30, 0, 0, 0, 0, 0],           // ;
        0x3C => [0x06, 0x0C, 0x18, 0x30, 0x18, 0x0C, 0x06, 0, 0, 0, 0, 0],           // <
        0x3D => [0x00, 0x7E, 0x00, 0x7E, 0, 0, 0, 0, 0, 0, 0, 0],                     // =
        0x3E => [0x60, 0x30, 0x18, 0x0C, 0x18, 0x30, 0x60, 0, 0, 0, 0, 0],           // >
        0x3F => [0x3C, 0x66, 0x06, 0x0C, 0x18, 0x00, 0x18, 0, 0, 0, 0, 0],           // ?
        0x41 => [0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0, 0, 0, 0, 0],           // A
        0x42 => [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0, 0, 0, 0, 0],           // B
        0x43 => [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0, 0, 0, 0, 0],           // C
        0x44 => [0x78, 0x6C, 0x66, 0x66, 0x66, 0x6C, 0x78, 0, 0, 0, 0, 0],           // D
        0x45 => [0x7E, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x7E, 0, 0, 0, 0, 0],           // E
        0x46 => [0x7E, 0x60, 0x60, 0x7C, 0x60, 0x60, 0x60, 0, 0, 0, 0, 0],           // F
        0x47 => [0x3C, 0x66, 0x60, 0x6E, 0x66, 0x66, 0x3A, 0, 0, 0, 0, 0],           // G
        0x48 => [0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0, 0, 0, 0, 0],           // H
        0x49 => [0x3C, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0, 0, 0, 0, 0],           // I
        0x4A => [0x0E, 0x06, 0x06, 0x06, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // J
        0x4B => [0x66, 0x6C, 0x78, 0x70, 0x78, 0x6C, 0x66, 0, 0, 0, 0, 0],           // K
        0x4C => [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0, 0, 0, 0, 0],           // L
        0x4D => [0x63, 0x77, 0x7F, 0x6B, 0x63, 0x63, 0x63, 0, 0, 0, 0, 0],           // M
        0x4E => [0x66, 0x76, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0, 0, 0, 0, 0],           // N
        0x4F => [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // O
        0x50 => [0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0, 0, 0, 0, 0],           // P
        0x51 => [0x3C, 0x66, 0x66, 0x66, 0x6A, 0x6C, 0x36, 0, 0, 0, 0, 0],           // Q
        0x52 => [0x7C, 0x66, 0x66, 0x7C, 0x78, 0x6C, 0x66, 0, 0, 0, 0, 0],           // R
        0x53 => [0x3C, 0x66, 0x60, 0x3C, 0x06, 0x66, 0x3C, 0, 0, 0, 0, 0],           // S
        0x54 => [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0, 0, 0, 0, 0],           // T
        0x55 => [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // U
        0x56 => [0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0, 0, 0, 0, 0],           // V
        0x57 => [0x63, 0x63, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0, 0, 0, 0, 0],           // W
        0x58 => [0x66, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x66, 0, 0, 0, 0, 0],           // X
        0x59 => [0x66, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0, 0, 0, 0, 0],           // Y
        0x5A => [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0, 0, 0, 0, 0],           // Z
        0x5B => [0x3C, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x3C, 0, 0, 0],     // [
        0x5D => [0x3C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x3C, 0, 0, 0],     // ]
        0x5F => [0x00, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0, 0],                           // _
        0x61 => [0x00, 0x00, 0x3C, 0x06, 0x3E, 0x66, 0x3B, 0, 0, 0, 0, 0],           // a
        0x62 => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0, 0, 0, 0, 0],           // b
        0x63 => [0x00, 0x00, 0x3C, 0x66, 0x60, 0x66, 0x3C, 0, 0, 0, 0, 0],           // c
        0x64 => [0x06, 0x06, 0x3E, 0x66, 0x66, 0x66, 0x3E, 0, 0, 0, 0, 0],           // d
        0x65 => [0x00, 0x00, 0x3C, 0x66, 0x7E, 0x60, 0x3C, 0, 0, 0, 0, 0],           // e
        0x66 => [0x0C, 0x18, 0x7E, 0x18, 0x18, 0x18, 0x18, 0, 0, 0, 0, 0],           // f
        0x67 => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x3C, 0, 0, 0, 0],         // g
        0x68 => [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x66, 0, 0, 0, 0, 0],           // h
        0x69 => [0x18, 0x00, 0x38, 0x18, 0x18, 0x18, 0x3C, 0, 0, 0, 0, 0],           // i
        0x6A => [0x06, 0x00, 0x0E, 0x06, 0x06, 0x66, 0x3C, 0, 0, 0, 0, 0],           // j
        0x6B => [0x60, 0x60, 0x66, 0x6C, 0x78, 0x6C, 0x66, 0, 0, 0, 0, 0],           // k
        0x6C => [0x38, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0, 0, 0, 0, 0],           // l
        0x6D => [0x00, 0x00, 0x66, 0x7F, 0x7F, 0x6B, 0x63, 0, 0, 0, 0, 0],           // m
        0x6E => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x66, 0, 0, 0, 0, 0],           // n
        0x6F => [0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0, 0, 0, 0, 0],           // o
        0x70 => [0x00, 0x00, 0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0, 0, 0, 0],         // p
        0x71 => [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x06, 0, 0, 0, 0],         // q
        0x72 => [0x00, 0x00, 0x7C, 0x66, 0x60, 0x60, 0x60, 0, 0, 0, 0, 0],           // r
        0x73 => [0x00, 0x00, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0, 0, 0, 0, 0],           // s
        0x74 => [0x18, 0x18, 0x7E, 0x18, 0x18, 0x18, 0x0E, 0, 0, 0, 0, 0],           // t
        0x75 => [0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x3E, 0, 0, 0, 0, 0],           // u
        0x76 => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3C, 0x18, 0, 0, 0, 0, 0],           // v
        0x77 => [0x00, 0x00, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0, 0, 0, 0, 0],           // w
        0x78 => [0x00, 0x00, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0, 0, 0, 0, 0],           // x
        0x79 => [0x00, 0x00, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x3C, 0, 0, 0, 0],         // y
        0x7A => [0x00, 0x00, 0x7E, 0x0C, 0x18, 0x30, 0x7E, 0, 0, 0, 0, 0],           // z
        0x7C => [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0, 0, 0],     // |
        _ => [0x00, 0x7E, 0x42, 0x42, 0x42, 0x7E, 0, 0, 0, 0, 0, 0],                  // default rect
    }
}

#[cfg(feature = "sexos")]
pub mod sexos {
    use super::*;
    use alloc::vec::Vec;
    use sex_app::{Event as AppEvent, Window};
    use sex_pdx::app_pointer::Kind as PKind;

    pub fn now() -> u64 {
        sex_pdx::get_ticks()
    }

    pub fn event(e: AppEvent) -> Event {
        match e {
            AppEvent::Key(k) => Event::Key(Key {
                code: k.code,
                shift: k.shift,
                control: k.control,
                text: k.char(),
            }),
            AppEvent::Close => Event::Close,
            AppEvent::Resize { width, height } => Event::Resize {
                width: width as u32,
                height: height as u32,
            },
            AppEvent::Pointer(p) => {
                let button = |b: u8| match b {
                    1 => Button::Middle,
                    2 => Button::Right,
                    _ => Button::Left,
                };
                let kind = match p.kind {
                    PKind::Move => PointerKind::Move,
                    PKind::Down { button: b } => PointerKind::Down(button(b)),
                    PKind::Up { button: b } => PointerKind::Up(button(b)),
                    PKind::Wheel { steps } => PointerKind::Wheel(steps as f32),
                    PKind::Leave => PointerKind::Leave,
                };
                Event::Pointer(Pointer {
                    x: p.x as f32,
                    y: p.y as f32,
                    kind,
                })
            }
            _ => Event::Close,
        }
    }

    pub struct App {
        pub window: Window,
        pub ui: Ui,
        pub frames: u64,
    }

    impl App {
        pub fn open(title: &str, theme: Theme, strings: Strings) -> App {
            let mut window = Window::open();
            window.title(title.as_bytes());
            let (w, h) = window.size();
            let ui = Ui::new(w as u32, h as u32, theme, strings);
            App {
                window,
                ui,
                frames: 0,
            }
        }

        pub fn present(&mut self) {
            let (w, h) = self.ui.size();
            let pixels = self.window.pixels();
            if pixels.len() < (w * h) as usize {
                return;
            }
            if self.ui.paint(&mut pixels[..(w * h) as usize]).is_some() {
                self.window.show_pixels(w as usize, h as usize);
                self.frames += 1;
            }
        }

        pub fn next(&mut self) -> Vec<Response> {
            self.present();
            let wait = self.ui.tick(now());
            let e = match wait {
                Some(ms) => self.window.next_within(ms.max(1)),
                None => Some(self.window.next()),
            };
            self.ui.tick(now());
            let mut out = Vec::new();
            if let Some(e) = e {
                out.extend(self.ui.handle(event(e)));
                for _ in 0..64 {
                    let Some(e) = self.window.poll() else { break };
                    out.extend(self.ui.handle(event(e)));
                }
            }
            self.present();
            out
        }
    }
}
