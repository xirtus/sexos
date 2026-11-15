// SPDX-License-Identifier: GPL-3.0-only
//! The window: Blender's top bar (File, Edit, Render, Window, Help and the
//! workspace tabs), the active workspace's areas, the status bar, popup menus,
//! area-border dragging and the window-level keymap.
use crate::doc::Doc;
use crate::draw::{Align, Painter, Rect};
use crate::editors::{Editor, EditorKind};
use crate::input::{Button, Input, Key, Mods};
use crate::screen::{default_workspaces, Border, Workspace};
use crate::theme as t;
use crate::ui::{MenuItem, Op, Popup, Ui, UiState};
use sex_3d::scene::ObjectData;

pub struct App {
    pub doc: Doc,
    pub workspaces: Vec<Workspace>,
    pub active_ws: usize,
    pub painter: Painter,
    pub ui: UiState,
    pub popup: Option<Popup>,
    borders: Vec<Border>,
    dragging_border: Option<Border>,
    /// Area that owns the pointer while one of its modal actions runs.
    captured: Option<u32>,
    /// Asks the program loop to end.
    pub quit: bool,
    /// Operators run since the program loop last drained this (serial record, gates).
    pub op_log: Vec<String>,
}

impl App {
    pub fn new(width: usize, height: usize) -> App {
        App {
            doc: Doc::default(),
            workspaces: default_workspaces(),
            active_ws: 0,
            painter: Painter::new(width, height),
            ui: UiState::default(),
            popup: None,
            borders: Vec::new(),
            dragging_border: None,
            captured: None,
            quit: false,
            op_log: Vec::new(),
        }
    }

    /// Open a scene file (any format sex-3d reads) in place of the current one.
    pub fn open_bytes(&mut self, name: &str, bytes: &[u8], resolver: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<(), String> {
        let doc = if name.ends_with(".poly") || bytes.starts_with(b"polyops ") {
            crate::persist::load(&String::from_utf8_lossy(bytes))?
        } else {
            let fmt = sex_3d::io::Format::from_path(name).or_else(|| sex_3d::io::Format::sniff(bytes)).ok_or_else(|| format!("{name}: unknown file type"))?;
            if fmt == sex_3d::io::Format::Blend {
                let (scene, actions) = sex_3d::io::blend::import_blend_full(bytes)?;
                let mut doc = Doc::new(scene);
                crate::anim::adopt_blend_actions(&mut doc, &actions);
                doc
            } else {
                Doc::new(sex_3d::io::import(bytes, fmt, resolver).map_err(|e| e.to_string())?)
            }
        };
        self.doc = doc;
        self.doc.path = Some(name.to_string());
        self.doc.report = format!("Opened {name}");
        Ok(())
    }

    fn layout(&mut self) -> (Rect, Rect, Rect) {
        let full = self.painter.bounds();
        let (top, rest) = full.take_top(t::TOPBAR_H);
        let (status, main) = rest.take_bottom(t::STATUSBAR_H);
        let main = Rect::new(main.x + t::GAP, main.y + t::GAP, main.w - 2 * t::GAP, main.h - 2 * t::GAP);
        self.borders = self.workspaces[self.active_ws].layout(main);
        (top, main, status)
    }

    /// Feed one input event and redraw the window.
    pub fn frame(&mut self, input: Input) {
        if let Input::Resize { width, height } = input {
            self.painter.resize(width, height);
        }
        if let Some(pos) = input.pos() {
            self.ui.mouse = pos;
        }
        crate::anim::tick(&mut self.doc);
        let mut input = Some(input);
        let had_popup = self.popup.is_some();

        // 1. An open menu sees everything first.
        if let (Some(pop), Some(inp)) = (&mut self.popup, &input) {
            if matches!(inp, Input::Down { .. } | Input::Key { .. } | Input::Move { .. }) {
                let (open, op) = pop.handle(&mut self.painter, inp);
                if let Some(op) = op {
                    self.doc.queue.push(op);
                }
                if !open {
                    self.popup = None;
                }
                input = None;
            }
        }

        // 2. Area borders.
        let _ = self.layout();
        match input {
            Some(Input::Down { x, y, button: Button::Left, .. }) => {
                if let Some(b) = self.borders.iter().find(|b| b.rect.contains(x, y)).cloned() {
                    self.dragging_border = Some(b);
                    input = None;
                }
            }
            Some(Input::Move { x, y }) => {
                if let Some(b) = self.dragging_border.clone() {
                    self.workspaces[self.active_ws].drag_border(&b, x, y);
                    input = None;
                }
            }
            Some(Input::Up { button: Button::Left, .. }) if self.dragging_border.is_some() => {
                self.dragging_border = None;
                input = None;
            }
            _ => {}
        }

        // 3. Draw, letting widgets take the event.
        let leftover = self.draw(input);

        // 4. Keys and other unclaimed events go to the area under the pointer,
        //    then to the window keymap.
        if let Some(inp) = leftover {
            let (mx, my) = self.ui.mouse;
            let ws = &mut self.workspaces[self.active_ws];
            let target = self.captured.or_else(|| ws.area_at(mx, my));
            let mut used = false;
            if let Some(id) = target {
                if let Some(area) = ws.area_mut(id) {
                    let r = area.rect;
                    used = area.editor.hotkey(&mut self.doc, &inp, r);
                }
            }
            if !used {
                self.window_keys(&inp);
            }
        }

        // 5. Menus asked for by widgets, and queued operators.
        if let Some(p) = self.ui.open_popup.take() {
            self.popup = Some(p);
        }
        if let Some(p) = self.doc.menu.take() {
            self.popup = Some(p);
        }
        let ran = self.run_queue();
        let ran = crate::anim::tick(&mut self.doc) || ran;
        let ran = crate::python::tick(&mut self.doc) || ran;
        if let Some(name) = self.doc.want_workspace.take() {
            if let Some(i) = self.workspaces.iter().position(|w| w.name == name) {
                self.active_ws = i;
            }
        }
        // Something changed after the widgets drew: draw again so the screen is current.
        if ran || had_popup != self.popup.is_some() {
            self.draw(None);
            // A menu an editor asked for while handling ops in that redraw.
            if let Some(p) = self.ui.open_popup.take().or_else(|| self.doc.menu.take()) {
                self.popup = Some(p);
            }
        }
        if let Some(p) = &self.popup {
            p.draw(&mut self.painter, self.ui.mouse);
        }
    }

    /// Whether an editor in the active workspace wants `Input::Tick` events
    /// (a viewport refining a progressive render).
    pub fn wants_tick(&self) -> bool {
        self.workspaces[self.active_ws].areas().iter().any(|a| a.editor.wants_tick()) || self.doc.python.wants_tick()
    }

    /// Draw the whole window, returning the event if no widget took it.
    fn draw(&mut self, input: Option<Input>) -> Option<Input> {
        let (top, _main, status) = self.layout();
        let App { painter, ui: st, doc, workspaces, active_ws, captured, .. } = self;
        painter.fill(painter.bounds(), t::AREA_GAP);
        let mut ui = Ui { p: painter, st, input };
        topbar(&mut ui, doc, workspaces, active_ws, top);
        let ws = &mut workspaces[*active_ws];
        *captured = None;
        for area in ws.areas_mut() {
            if area.editor.is_modal() {
                *captured = Some(area.id);
            }
        }
        for area in ws.areas_mut() {
            let r = area.rect;
            area.editor.draw(doc, &mut ui, r, area.id);
        }
        statusbar(&mut ui, doc, status);
        ui.input
    }

    fn window_keys(&mut self, input: &Input) {
        let Input::Key { key, mods, .. } = *input else { return };
        let op = match (key, mods) {
            (Key::Char('z'), Mods::CTRL) => Op::new("ed.undo"),
            (Key::Char('z'), Mods::CTRL_SHIFT) | (Key::Char('y'), Mods::CTRL) => Op::new("ed.redo"),
            (Key::Char('s'), Mods::CTRL) => Op::new("wm.save_mainfile"),
            (Key::Char('n'), Mods::CTRL) => Op::new("wm.read_homefile"),
            (Key::Char('q'), Mods::CTRL) => Op::new("wm.quit_blender"),
            (Key::F(12), Mods::NONE) => Op::new("render.render"),
            (Key::F(12), Mods::CTRL) => Op::new("render.render_animation"),
            (Key::F(3), Mods::NONE) => Op::new("wm.search_menu"),
            (Key::Space, Mods::CTRL) => Op::new("screen.screen_full_area"),
            (Key::PageUp, Mods::CTRL) => Op::with("screen.workspace_cycle", "PREV"),
            (Key::PageDown, Mods::CTRL) => Op::with("screen.workspace_cycle", "NEXT"),
            (Key::Space, Mods::NONE) => Op::new("screen.animation_play"),
            (Key::Left, Mods::SHIFT) => Op::with("screen.frame_jump", "START"),
            (Key::Right, Mods::SHIFT) => Op::with("screen.frame_jump", "END"),
            (Key::Left, Mods::NONE) => Op::with("screen.frame_offset", "-1"),
            (Key::Right, Mods::NONE) => Op::with("screen.frame_offset", "1"),
            (Key::Up, Mods::NONE) => Op::with("screen.keyframe_jump", "NEXT"),
            (Key::Down, Mods::NONE) => Op::with("screen.keyframe_jump", "PREV"),
            (Key::Space, Mods::CTRL_SHIFT) => Op::new("screen.animation_play_reverse"),
            _ => return,
        };
        self.doc.queue.push(op);
    }

    /// Run queued operators; window-level ones here, the rest on the document.
    fn run_queue(&mut self) -> bool {
        let ops: Vec<Op> = core::mem::take(&mut self.doc.queue);
        let ran = !ops.is_empty();
        for op in ops {
            self.op_log.push(if op.arg.is_empty() { op.id.clone() } else { format!("{} {}", op.id, op.arg) });
            match op.id.as_str() {
                "screen.area_type" => {
                    if let Some((id, label)) = op.arg.split_once(':') {
                        if let (Ok(id), Some(kind)) = (id.parse::<u32>(), EditorKind::from_label(label)) {
                            if let Some(a) = self.workspaces[self.active_ws].area_mut(id) {
                                a.editor = Editor::new(kind);
                            }
                        }
                    }
                }
                "screen.screen_full_area" => {
                    let (mx, my) = self.ui.mouse;
                    let ws = &mut self.workspaces[self.active_ws];
                    ws.maximized = if ws.maximized.is_some() { None } else { ws.area_at(mx, my) };
                }
                "screen.workspace_cycle" => {
                    let n = self.workspaces.len();
                    self.active_ws = if op.arg == "PREV" { (self.active_ws + n - 1) % n } else { (self.active_ws + 1) % n };
                }
                "wm.call_menu" => {
                    // ADD:x:y opens the Add menu at a point (Shift+A).
                    let mut parts = op.arg.split(':');
                    if parts.next() == Some("ADD") {
                        let x = parts.next().and_then(|v| v.parse().ok()).unwrap_or(self.ui.mouse.0);
                        let y = parts.next().and_then(|v| v.parse().ok()).unwrap_or(self.ui.mouse.1);
                        let (mx, my) = self.ui.mouse;
                        let _ = (x, y);
                        self.popup = Some(Popup::new("Add", add_menu_items(), mx - 20, my - 10));
                    }
                }
                "wm.read_homefile" => {
                    self.doc = Doc::default();
                    self.doc.report = "New file".into();
                }
                "wm.quit_blender" => self.quit = true,
                "wm.save_mainfile" | "wm.save_as_mainfile" => self.save(None),
                "wm.export" => self.save(Some(op.arg.clone())),
                "render.render" => self.render(),
                "render.render_animation" => self.render_animation(),
                "wm.search_menu" => {
                    let (mx, my) = self.ui.mouse;
                    let mut items = search_items();
                    // Operators scripts and add-ons registered.
                    let script = crate::python::search_items(&self.doc);
                    if !script.is_empty() {
                        items.push(MenuItem::Separator);
                        items.extend(script);
                    }
                    self.popup = Some(Popup::new("Search", items, mx - 100, my - 10));
                }
                _ => self.doc.run(&op),
            }
        }
        ran
    }

    /// Save as Polyamour's own text format (`.poly`, the polyops scene language),
    /// or export to `ext` (glb, gltf, obj, stl, ply).
    fn save(&mut self, ext: Option<String>) {
        let stem = self.doc.path.as_deref().map(|p| p.rsplit_once('.').map_or(p, |x| x.0).to_string()).unwrap_or_else(|| "untitled".into());
        let result: Result<Vec<(String, Vec<u8>)>, String> = match ext.as_deref() {
            None => Ok(vec![(format!("{stem}.poly"), crate::persist::save(&self.doc).into_bytes())]),
            Some(e) => match sex_3d::io::Format::from_extension(e) {
                Some(f) => sex_3d::io::export_files(&self.doc.scene, f, &stem).map_err(|e| e.to_string()),
                None => Err(format!("cannot export .{e}")),
            },
        };
        match result {
            Ok(files) => {
                let mut wrote = Vec::new();
                for (name, bytes) in files {
                    match std::fs::write(&name, &bytes) {
                        Ok(()) => wrote.push(name),
                        Err(e) => {
                            self.doc.report = format!("Cannot write {name}: {e}");
                            return;
                        }
                    }
                }
                if ext.is_none() {
                    self.doc.path = wrote.first().cloned();
                    self.doc.dirty = false;
                }
                self.doc.report = format!("Saved \"{}\"", wrote.join(", "));
            }
            Err(e) => self.doc.report = e,
        }
    }

    /// F12: path-trace the active camera's view and show it in the Rendering workspace.
    fn render(&mut self) {
        // Render Properties (engine, samples, bounces, denoise, clamp) and Output
        // Properties (resolution and %), with render visibility applied.
        let settings = sex_3d::render::RenderSettings { frame: self.doc.frame, ..self.doc.settings.render_settings() };
        let base = self.doc.settings.render_scene(&self.doc.scene);
        let scene = crate::anim::scene_at(&self.doc.anim, &base, self.doc.frame as f32, self.doc.settings.motion_blur);
        let r = sex_3d::render::render(&scene, &settings);
        let n = r.passes.len();
        let passes = match n {
            0 => String::new(),
            1 => ", 1 pass".to_string(),
            n => format!(", {n} passes"),
        };
        self.doc.report = format!("Rendered {}x{}, {} samples{passes}", r.color.width, r.color.height, r.samples);
        self.doc.render = Some(r.color);
        self.doc.want_workspace = Some("Rendering".into());
    }

    /// Ctrl F12: render the frame range (Output Properties), writing each
    /// frame as `<output path><frame>.png`, colour managed as on screen.
    fn render_animation(&mut self) {
        let d = &self.doc;
        let settings = d.settings.render_settings();
        let base = d.settings.render_scene(&d.scene);
        let cm = d.settings.color_management();
        let motion = d.settings.motion_blur;
        let frames: Vec<i32> = (d.scene.frame_start..=d.scene.frame_end).step_by(d.settings.frame_step.max(1) as usize).collect();
        let stem = if d.settings.output_path.is_empty() { "/tmp/".to_string() } else { d.settings.output_path.clone() };
        let (mut written, mut error, mut last) = (Vec::new(), None, None);
        let anim = &d.anim;
        sex_3d::render::render_frames(
            frames.iter().copied(),
            &settings,
            |f| crate::anim::scene_at(anim, &base, f as f32, motion),
            |f, r| {
                let name = format!("{stem}{f:04}.png");
                let png = crate::editors::image::encode_png(r.color.width, r.color.height, &sex_3d::render::to_display8(&r.color, &cm));
                match std::fs::write(&name, png) {
                    Ok(()) => written.push(name),
                    Err(e) => error = Some(format!("Cannot write {name}: {e}")),
                }
                last = Some(r.color);
                error.is_none()
            },
            None,
        );
        if let Some(img) = last {
            self.doc.render = Some(img);
            self.doc.want_workspace = Some("Rendering".into());
        }
        self.doc.report = match (error, written.as_slice()) {
            (Some(e), _) => e,
            (None, []) => "No frames to render".into(),
            (None, [one]) => format!("Rendered 1 frame to \"{one}\""),
            (None, w) => format!("Rendered {} frames to \"{}\" .. \"{}\"", w.len(), w[0], w[w.len() - 1]),
        };
    }
}

fn add_menu_items() -> Vec<MenuItem> {
    let p = |l: &str, a: &str| MenuItem::op(l, "", Op::with("mesh.primitive_add", a));
    vec![
        MenuItem::Sub("Mesh".into(), vec![p("Plane", "plane"), p("Cube", "cube"), p("Circle", "circle"), p("UV Sphere", "uv_sphere"), p("Ico Sphere", "ico_sphere"), p("Cylinder", "cylinder"), p("Cone", "cone"), p("Torus", "torus"), MenuItem::Separator, p("Grid", "grid")]),
        MenuItem::op("Empty", "", Op::new("object.empty_add")),
        MenuItem::Separator,
        MenuItem::op("Camera", "", Op::new("object.camera_add")),
        MenuItem::Sub(
            "Light".into(),
            vec![
                MenuItem::op("Point", "", Op::with("object.light_add", "POINT")),
                MenuItem::op("Sun", "", Op::with("object.light_add", "SUN")),
                MenuItem::op("Spot", "", Op::with("object.light_add", "SPOT")),
                MenuItem::op("Area", "", Op::with("object.light_add", "AREA")),
            ],
        ),
    ]
}

fn search_items() -> Vec<MenuItem> {
    vec![
        MenuItem::op("Add Cube", "", Op::with("mesh.primitive_add", "cube")),
        MenuItem::op("Render Image", "F12", Op::new("render.render")),
        MenuItem::op("Save", "Ctrl S", Op::new("wm.save_mainfile")),
        MenuItem::op("Undo", "Ctrl Z", Op::new("ed.undo")),
        MenuItem::op("Redo", "Shift Ctrl Z", Op::new("ed.redo")),
    ]
}

fn file_menu(_d: &Doc) -> Vec<MenuItem> {
    let ex = |l: &str, e: &str| MenuItem::op(l, "", Op::with("wm.export", e));
    vec![
        MenuItem::op("New", "Ctrl N", Op::new("wm.read_homefile")),
        MenuItem::op("Open...", "Ctrl O", Op::new("wm.open_mainfile")),
        MenuItem::Separator,
        MenuItem::op("Save", "Ctrl S", Op::new("wm.save_mainfile")),
        MenuItem::op("Save As...", "Shift Ctrl S", Op::new("wm.save_as_mainfile")),
        MenuItem::Separator,
        MenuItem::Sub("Import".into(), vec![MenuItem::op("glTF 2.0 (.glb/.gltf)", "", Op::new("wm.import")), MenuItem::op("Wavefront (.obj)", "", Op::new("wm.import")), MenuItem::op("Blender (.blend)", "", Op::new("wm.import"))]),
        MenuItem::Sub("Export".into(), vec![ex("glTF 2.0 (.glb)", "glb"), ex("glTF 2.0 (.gltf)", "gltf"), ex("Wavefront (.obj)", "obj"), ex("STL (.stl)", "stl"), ex("Stanford PLY (.ply)", "ply")]),
        MenuItem::Separator,
        MenuItem::op("Quit", "Ctrl Q", Op::new("wm.quit_blender")),
    ]
}

fn edit_menu(d: &Doc) -> Vec<MenuItem> {
    let undo = d.undo.undo_label().map(|l| format!("Undo {l}")).unwrap_or_else(|| "Undo".into());
    let redo = d.undo.redo_label().map(|l| format!("Redo {l}")).unwrap_or_else(|| "Redo".into());
    vec![
        MenuItem::op(&undo, "Ctrl Z", Op::new("ed.undo")),
        MenuItem::op(&redo, "Shift Ctrl Z", Op::new("ed.redo")),
        MenuItem::Separator,
        MenuItem::op("Menu Search...", "F3", Op::new("wm.search_menu")),
        MenuItem::Separator,
        MenuItem::op("Preferences...", "", Op::with("screen.workspace", "Preferences")),
    ]
}

fn render_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::op("Render Image", "F12", Op::new("render.render")),
        MenuItem::op("Render Animation", "Ctrl F12", Op::new("render.render_animation")),
        MenuItem::Separator,
        MenuItem::op("View Render", "F11", Op::with("screen.workspace", "Rendering")),
    ]
}

fn window_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::op("Next Workspace", "Ctrl PageDown", Op::with("screen.workspace_cycle", "NEXT")),
        MenuItem::op("Previous Workspace", "Ctrl PageUp", Op::with("screen.workspace_cycle", "PREV")),
        MenuItem::Separator,
        MenuItem::op("Toggle Maximize Area", "Ctrl Space", Op::new("screen.screen_full_area")),
    ]
}

fn help_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![MenuItem::Heading("Polyamour 0.1: Blender's layout and keymap on SexOS".into())]
}

fn topbar(ui: &mut Ui, doc: &Doc, workspaces: &[Workspace], active: &mut usize, r: Rect) {
    ui.p.fill(r, t::TOPBAR);
    // App menu button where Blender shows its logo: ours is a plain diamond.
    let logo = Rect::new(r.x + 4, r.y + 3, 26, r.h - 6);
    let (lx, ly) = logo.center();
    let open_app_menu = ui.menu_title(logo, "");
    ui.p.diamond(lx as f32, ly as f32, 7.0, 0xff7a3d);
    if open_app_menu {
        ui.open_menu(logo, "Polyamour", help_menu(doc));
    }
    let menus: [(&str, fn(&Doc) -> Vec<MenuItem>); 5] = [("File", file_menu), ("Edit", edit_menu), ("Render", render_menu), ("Window", window_menu), ("Help", help_menu)];
    let mut x = logo.right() + 4;
    for (title, items) in menus {
        let w = ui.p.text_width(title) + 14;
        let mr = Rect::new(x, r.y + 3, w, r.h - 6);
        if ui.menu_title(mr, title) {
            ui.open_menu(mr, "", items(doc));
        }
        x += w + 2;
    }
    x += 12;
    for (i, ws) in workspaces.iter().enumerate() {
        let w = ui.p.text_width(&ws.name) + 20;
        let tr = Rect::new(x, r.y + 2, w, r.h - 2);
        let on = i == *active;
        let hover = ui.hovered(tr);
        if on || hover {
            ui.p.round_rect(Rect::new(tr.x, tr.y, tr.w, tr.h + 6), 5, if on { t::TAB_ACTIVE } else { 0x282828 });
        }
        ui.p.label(tr, &ws.name, if on { t::TEXT_HI } else { t::TEXT_DIM }, Align::Center);
        if ui.clicked(tr) {
            *active = i;
        }
        x += w + 1;
    }
    // Scene and view layer selectors at the right.
    let vr = Rect::new(r.right() - 150, r.y + 3, 140, r.h - 6);
    ui.dropdown(vr, "ViewLayer");
    let sr = Rect::new(vr.x - 150, r.y + 3, 140, r.h - 6);
    ui.dropdown(sr, &doc.scene.name);
}

fn statusbar(ui: &mut Ui, doc: &Doc, r: Rect) {
    ui.p.fill(r, t::STATUSBAR);
    let hints = "Select    Pan View    Context Menu";
    ui.p.label(Rect::new(r.x + 10, r.y, r.w / 2, r.h), hints, t::TEXT_DIM, Align::Left);
    let s = &doc.scene;
    let mut stats = String::new();
    if let Some(a) = doc.active() {
        stats.push_str(&format!("Collection | {}", s.objects[a].name));
        if let ObjectData::Mesh(mi) = s.objects[a].data {
            let m = &s.meshes[mi];
            stats.push_str(&format!(" | Verts {} | Faces {}", m.vertex_count(), m.face_count()));
        }
        stats.push_str(" | ");
    }
    stats.push_str(&format!("Objects {}/{} | Polyamour 0.1", s.selected.len(), s.objects.len()));
    if !doc.report.is_empty() {
        stats = format!("{}    {}", doc.report, stats);
    }
    ui.p.label(Rect::new(r.x + r.w / 2, r.y, r.w / 2 - 10, r.h), &stats, t::TEXT_DIM, Align::Right);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(app: &mut App, x: i32, y: i32) {
        app.frame(Input::Move { x, y });
        app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
        app.frame(Input::Up { x, y, button: Button::Left });
    }

    #[test]
    fn shift_a_adds_a_mesh_and_ctrl_z_undoes() {
        let mut app = App::new(1280, 720);
        app.frame(Input::Tick);
        let n = app.doc.scene.objects.len();
        // Pointer over the viewport, Shift+A, then Mesh > UV Sphere.
        app.frame(Input::Move { x: 400, y: 300 });
        app.frame(Input::key_mods(Key::Char('a'), Mods::SHIFT));
        let pop = app.popup.clone().expect("add menu open");
        assert_eq!(pop.title, "Add");
        let r = pop.rect(&mut app.painter);
        // Hover "Mesh" (first item) to open the submenu, then click "UV Sphere" (4th).
        let (mx, my) = (r.x + 30, r.y + 2 + 24 + 11);
        app.frame(Input::Move { x: mx, y: my });
        let sub = app.popup.as_ref().unwrap().sub.as_ref().expect("submenu").1.rect(&mut app.painter);
        let (sx, sy) = (sub.x + 30, sub.y + 2 + 3 * 22 + 11);
        click(&mut app, sx, sy);
        assert_eq!(app.doc.scene.objects.len(), n + 1, "{}", app.doc.report);
        assert_eq!(app.doc.scene.objects[app.doc.active().unwrap()].name, "Sphere");
        app.frame(Input::key_mods(Key::Char('z'), Mods::CTRL));
        assert_eq!(app.doc.scene.objects.len(), n);
    }

    #[test]
    fn workspace_tabs_switch() {
        let mut app = App::new(1600, 900);
        app.frame(Input::Tick);
        app.frame(Input::key_mods(Key::PageDown, Mods::CTRL));
        assert_eq!(app.workspaces[app.active_ws].name, "Modeling");
        app.frame(Input::key_mods(Key::PageUp, Mods::CTRL));
        assert_eq!(app.workspaces[app.active_ws].name, "Layout");
    }
}
