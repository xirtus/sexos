// SPDX-License-Identifier: GPL-3.0-only
//! The editors an area can show, with Blender's names and header layouts.
//!
//! Every editor module has the same shape: a `State` (per-area view state such
//! as the 3D view's camera), `header` (draws the editor's header contents right
//! of the editor-type dropdown), `body` (draws and handles the main region) and
//! `hotkey` (keys pressed while the pointer is over the area).
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::input::Input;
use crate::theme as t;
use crate::ui::{MenuItem, Op, Ui};

pub mod image;
pub mod node;
pub mod outliner;
pub mod properties;
pub mod simple;
pub mod text;
pub mod timeline;
pub mod view3d;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorKind {
    View3D,
    ImageEditor,
    UvEditor,
    ShaderEditor,
    CompositorEditor,
    GeometryNodeEditor,
    Timeline,
    DopeSheet,
    GraphEditor,
    TextEditor,
    PythonConsole,
    Info,
    Outliner,
    Properties,
    FileBrowser,
    Spreadsheet,
    Preferences,
}

impl EditorKind {
    pub fn label(self) -> &'static str {
        match self {
            EditorKind::View3D => "3D Viewport",
            EditorKind::ImageEditor => "Image Editor",
            EditorKind::UvEditor => "UV Editor",
            EditorKind::ShaderEditor => "Shader Editor",
            EditorKind::CompositorEditor => "Compositor",
            EditorKind::GeometryNodeEditor => "Geometry Node Editor",
            EditorKind::Timeline => "Timeline",
            EditorKind::DopeSheet => "Dope Sheet",
            EditorKind::GraphEditor => "Graph Editor",
            EditorKind::TextEditor => "Text Editor",
            EditorKind::PythonConsole => "Python Console",
            EditorKind::Info => "Info",
            EditorKind::Outliner => "Outliner",
            EditorKind::Properties => "Properties",
            EditorKind::FileBrowser => "File Browser",
            EditorKind::Spreadsheet => "Spreadsheet",
            EditorKind::Preferences => "Preferences",
        }
    }
    /// Blender's editor-type menu, grouped as it groups them.
    pub fn menu() -> Vec<MenuItem> {
        use EditorKind as K;
        let item = |k: K| MenuItem::op(k.label(), "", Op::with("screen.area_type", k.label()));
        vec![
            MenuItem::Heading("General".into()),
            item(K::View3D),
            item(K::ImageEditor),
            item(K::UvEditor),
            item(K::CompositorEditor),
            item(K::ShaderEditor),
            item(K::GeometryNodeEditor),
            MenuItem::Separator,
            MenuItem::Heading("Animation".into()),
            item(K::DopeSheet),
            item(K::Timeline),
            item(K::GraphEditor),
            MenuItem::Separator,
            MenuItem::Heading("Scripting".into()),
            item(K::TextEditor),
            item(K::PythonConsole),
            item(K::Info),
            MenuItem::Separator,
            MenuItem::Heading("Data".into()),
            item(K::Outliner),
            item(K::Properties),
            item(K::FileBrowser),
            item(K::Spreadsheet),
            item(K::Preferences),
        ]
    }
    pub fn from_label(s: &str) -> Option<EditorKind> {
        use EditorKind as K;
        [
            K::View3D, K::ImageEditor, K::UvEditor, K::ShaderEditor, K::CompositorEditor, K::GeometryNodeEditor, K::Timeline, K::DopeSheet,
            K::GraphEditor, K::TextEditor, K::PythonConsole, K::Info, K::Outliner, K::Properties, K::FileBrowser, K::Spreadsheet, K::Preferences,
        ]
        .into_iter()
        .find(|k| k.label() == s)
    }
    /// Blender puts the header at the bottom for the timeline-like editors? No:
    /// since 2.8 every header is on top; the status bar is the only bottom bar.
    pub fn header_on_top(self) -> bool {
        true
    }
}

pub enum Editor {
    View3D(Box<view3d::State>),
    Outliner(outliner::State),
    Properties(properties::State),
    Timeline(timeline::State),
    Text(text::State),
    Node(node::State),
    Image(image::State),
    Simple(simple::State),
}

impl Editor {
    pub fn new(kind: EditorKind) -> Editor {
        use EditorKind as K;
        match kind {
            K::View3D => Editor::View3D(Box::default()),
            K::Outliner => Editor::Outliner(Default::default()),
            K::Properties => Editor::Properties(Default::default()),
            K::Timeline | K::DopeSheet | K::GraphEditor => Editor::Timeline(timeline::State::new(kind)),
            K::TextEditor => Editor::Text(Default::default()),
            K::ShaderEditor | K::CompositorEditor | K::GeometryNodeEditor => Editor::Node(node::State::new(kind)),
            K::ImageEditor | K::UvEditor => Editor::Image(image::State::new(kind)),
            K::PythonConsole | K::Info | K::FileBrowser | K::Spreadsheet | K::Preferences => Editor::Simple(simple::State::new(kind)),
        }
    }
    pub fn kind(&self) -> EditorKind {
        match self {
            Editor::View3D(_) => EditorKind::View3D,
            Editor::Outliner(_) => EditorKind::Outliner,
            Editor::Properties(_) => EditorKind::Properties,
            Editor::Timeline(s) => s.kind,
            Editor::Text(_) => EditorKind::TextEditor,
            Editor::Node(s) => s.kind,
            Editor::Image(s) => s.kind,
            Editor::Simple(s) => s.kind,
        }
    }

    /// Draw the whole area: header (with the editor-type dropdown) and body.
    pub fn draw(&mut self, doc: &mut Doc, ui: &mut Ui, r: Rect, area_id: u32) {
        let (hr, body) = r.take_top(t::HEADER_H);
        ui.p.push_clip(r);
        // Areas have rounded corners in Blender's 2.8+ UI.
        ui.p.round_rect(r, 6, body_color(self.kind()));
        ui.p.round_rect(Rect::new(hr.x, hr.y, hr.w, hr.h + 6), 6, t::HEADER);
        ui.p.fill(Rect::new(hr.x, hr.bottom(), hr.w, 1), 0x262626);
        let tr = Rect::new(hr.x + 4, hr.y + 3, 46, hr.h - 6);
        if ui.dropdown(tr, editor_glyph(self.kind())) {
            ui.open_menu(tr, "Editor Type", EditorKind::menu());
            if let Some(p) = &mut ui.st.open_popup {
                // The area the editor-type menu belongs to rides in each op's argument.
                for it in &mut p.items {
                    if let MenuItem::Op(_, _, op) = it {
                        op.arg = format!("{area_id}:{}", op.arg);
                    }
                }
            }
        }
        let hr = Rect::new(tr.right() + 4, hr.y, hr.w - (tr.right() + 4 - hr.x), hr.h);
        ui.p.push_clip(hr);
        match self {
            Editor::View3D(s) => view3d::header(s, doc, ui, hr),
            Editor::Outliner(s) => outliner::header(s, doc, ui, hr),
            Editor::Properties(s) => properties::header(s, doc, ui, hr),
            Editor::Timeline(s) => timeline::header(s, doc, ui, hr),
            Editor::Text(s) => text::header(s, doc, ui, hr),
            Editor::Node(s) => node::header(s, doc, ui, hr),
            Editor::Image(s) => image::header(s, doc, ui, hr),
            Editor::Simple(s) => simple::header(s, doc, ui, hr),
        }
        ui.p.pop_clip();
        let body = Rect::new(body.x, body.y + 1, body.w, body.h - 1);
        ui.p.push_clip(body);
        match self {
            Editor::View3D(s) => view3d::body(s, doc, ui, body),
            Editor::Outliner(s) => outliner::body(s, doc, ui, body),
            Editor::Properties(s) => properties::body(s, doc, ui, body),
            Editor::Timeline(s) => timeline::body(s, doc, ui, body),
            Editor::Text(s) => text::body(s, doc, ui, body),
            Editor::Node(s) => node::body(s, doc, ui, body),
            Editor::Image(s) => image::body(s, doc, ui, body),
            Editor::Simple(s) => simple::body(s, doc, ui, body),
        }
        ui.p.pop_clip();
        ui.p.pop_clip();
    }

    /// A key (or other unclaimed input) while the pointer is over this area.
    pub fn hotkey(&mut self, doc: &mut Doc, input: &Input, r: Rect) -> bool {
        let body = r.take_top(t::HEADER_H).1;
        match self {
            Editor::View3D(s) => view3d::hotkey(s, doc, input, body),
            Editor::Outliner(s) => outliner::hotkey(s, doc, input, body),
            Editor::Properties(s) => properties::hotkey(s, doc, input, body),
            Editor::Timeline(s) => timeline::hotkey(s, doc, input, body),
            Editor::Text(s) => text::hotkey(s, doc, input, body),
            Editor::Node(s) => node::hotkey(s, doc, input, body),
            Editor::Image(s) => image::hotkey(s, doc, input, body),
            Editor::Simple(s) => simple::hotkey(s, doc, input, body),
        }
    }

    /// Whether this editor is in the middle of a modal action (dragging a
    /// transform, orbiting) and wants every event until it ends.
    pub fn is_modal(&self) -> bool {
        match self {
            Editor::View3D(s) => view3d::is_modal(s),
            _ => false,
        }
    }
}

fn body_color(k: EditorKind) -> u32 {
    use EditorKind as K;
    match k {
        K::View3D => t::VIEWPORT,
        K::Outliner => t::OUTLINER,
        K::Properties => t::PROPERTIES,
        K::Timeline | K::DopeSheet | K::GraphEditor => t::TIMELINE,
        K::TextEditor | K::PythonConsole | K::Info => t::TEXT_EDITOR,
        K::ShaderEditor | K::CompositorEditor | K::GeometryNodeEditor => t::NODE_BG,
        _ => t::REGION,
    }
}

/// A short stand-in for Blender's editor-type icons.
pub fn editor_glyph(k: EditorKind) -> &'static str {
    use EditorKind as K;
    match k {
        K::View3D => "3D",
        K::ImageEditor => "Img",
        K::UvEditor => "UV",
        K::ShaderEditor => "Shd",
        K::CompositorEditor => "Cmp",
        K::GeometryNodeEditor => "GN",
        K::Timeline => "TL",
        K::DopeSheet => "DS",
        K::GraphEditor => "Grf",
        K::TextEditor => "Txt",
        K::PythonConsole => "Py",
        K::Info => "i",
        K::Outliner => "Out",
        K::Properties => "Prp",
        K::FileBrowser => "Fil",
        K::Spreadsheet => "Spr",
        K::Preferences => "Prf",
    }
}

/// Header menus ("View", "Select", ...) laid out left to right; returns the x after them.
pub fn header_menus(ui: &mut Ui, hr: Rect, x: i32, menus: &[(&str, &dyn Fn(&Doc) -> Vec<MenuItem>)], doc: &Doc) -> i32 {
    let mut x = x;
    for (title, items) in menus {
        let w = ui.p.text_width(title) + 14;
        let r = Rect::new(x, hr.y + 3, w, hr.h - 6);
        if ui.menu_title(r, title) {
            ui.open_menu(r, "", items(doc));
        }
        x += w + 2;
    }
    x
}

/// A centred grey note in an editor's body (for regions with nothing to show yet).
pub fn note(ui: &mut Ui, r: Rect, s: &str) {
    ui.p.label(r, s, t::TEXT_DIM, Align::Center);
}
