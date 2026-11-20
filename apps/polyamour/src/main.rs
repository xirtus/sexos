// SPDX-License-Identifier: GPL-3.0-only
//! Polyamour as a SexOS program: `run polyamour [scene.blend|.glb|.obj|.poly]`.
//! On the host, `polyamour --screenshot out.png [scene] [workspace]` draws the
//! window once into a PNG (used for docs and visual checks); `--node-demo`
//! gives the startup material a few texture nodes and opens Shading;
//! `--rig-demo` poses a skinned, keyed armature in the Animation workspace
//! with the Graph Editor below; `--sculpt-demo` shows a sculpted sphere with
//! a mask and face sets in the Sculpting workspace; `--gp-demo` draws Grease
//! Pencil objects in Draw Mode in the 2D Animation workspace; `--seq-demo`
//! shows a small edit in the Video Editing workspace; `--physics-demo` steps
//! falling boxes, a draped cloth, particles and smoke to frame 40;
//! `--liquid-demo` steps a liquid dam break (with an obstacle, a tap and a
//! drain) and a Dynamic Paint ripple to frame 24.
use polyamour::input::Input;
#[cfg(target_os = "hermit")]
use polyamour::input::{Button, Key, Mods};
use polyamour::App;

fn open_arg(app: &mut App, path: &str) {
    polyamour::app::open_path(app, path);
}

/// Blender's `--python FILE` and `--python-expr EXPR`: run a script once the
/// window is up, through the live kernel where pipes work (batch otherwise).
fn run_python_arg(app: &mut App, flag: &str, value: &str) {
    match flag {
        "--python" => match std::fs::read_to_string(value) {
            Ok(src) => polyamour::python::run_script(&mut app.doc, value, &src),
            Err(e) => app.doc.report = format!("Cannot open {value}: {e}"),
        },
        _ => polyamour::python::run_script(&mut app.doc, "<expr>", value),
    }
}

#[cfg(not(target_os = "hermit"))]
fn print_usage() {
    eprintln!(
        "usage: polyamour [--screenshot out.png] [--workspace Name] [--wasm FILE.wasm] [--python FILE.py] [--python-expr EXPR] [--node-demo] [--rig-demo] [--sculpt-demo] [--physics-demo] [--liquid-demo] [--gp-demo] [--seq-demo] [--browse open|save|import:EXT,..|export:EXT] [scene file]"
    );
}

#[cfg(not(target_os = "hermit"))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut app = App::new(1600, 900);
    let mut out = None;
    let mut rig_demo = false;
    let mut browse: Option<String> = None;
    let mut sculpt_demo = false;
    let mut physics_demo = false;
    let mut gp_demo = false;
    let mut python: Vec<(String, String)> = Vec::new();
    let mut wasm_files: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print_usage();
                return;
            }
            "--screenshot" => out = it.next().cloned(),
            "--workspace" => {
                if let Some(w) = it.next() {
                    app.doc.want_workspace = Some(w.clone());
                }
            }
            "--node-demo" => {
                polyamour::nodetree::demo_textures(&mut app.doc);
                app.doc.want_workspace = Some("Shading".into());
            }
            "--sculpt-demo" => {
                polyamour::sculpt::demo(&mut app.doc);
                sculpt_demo = true;
            }
            "--physics-demo" => {
                polyamour::physics::demo(&mut app.doc);
                physics_demo = true;
            }
            "--liquid-demo" => {
                polyamour::physics::liquid_demo(&mut app.doc);
                physics_demo = true;
            }
            "--gp-demo" => {
                polyamour::gpencil::demo(&mut app.doc);
                gp_demo = true;
            }
            "--seq-demo" => polyamour::seqedit::demo(&mut app.doc),
            // `--browse open|save|import:fbx,glb|export:fbx`: show the File Browser.
            "--browse" => browse = it.next().cloned(),
            "--rig-demo" => {
                polyamour::rig::demo(&mut app.doc);
                app.doc.want_workspace = Some("Animation".into());
                rig_demo = true;
            }
            f @ ("--python" | "--python-expr") => {
                if let Some(v) = it.next() {
                    python.push((f.to_string(), v.clone()));
                }
            }
            "--wasm" => {
                if let Some(v) = it.next() {
                    wasm_files.push(v.clone());
                }
            }
            // macOS Finder passes `-psn_0_...` when launching `.app` bundles.
            p if p.starts_with("-psn_") => {}
            p => open_arg(&mut app, p),
        }
    }
    app.frame(Input::Tick);
    for (f, v) in &python {
        run_python_arg(&mut app, f, v);
    }
    for w in &wasm_files {
        polyamour::python::run_wasm_file(&mut app.doc, std::path::Path::new(w), &[]);
    }
    app.frame(Input::Tick);
    if let Some(b) = &browse {
        use polyamour::filebrowser::Purpose;
        let (kind, exts) = b.split_once(':').unwrap_or((b.as_str(), ""));
        let purpose = match kind {
            "save" => Purpose::SaveAs,
            "import" => Purpose::Import(exts.split(',').filter(|e| !e.is_empty()).map(String::from).collect()),
            "export" => Purpose::Export(if exts.is_empty() { "fbx".into() } else { exts.into() }),
            _ => Purpose::Open,
        };
        app.browse(purpose);
        app.frame(Input::Tick);
    }
    if sculpt_demo {
        // The Sculpting workspace's view frames the sphere, the sidebar
        // shows the brush settings.
        app.doc.view_ops.push(polyamour::ui::Op::new("view3d.view_selected"));
        app.doc.view_ops.push(polyamour::ui::Op::with("view3d.sidebar_tab", "TOOL"));
        app.frame(Input::Tick);
        app.frame(Input::Tick);
    }
    if gp_demo {
        // Look at the drawing from the front, as the 2D Animation workspace does.
        app.doc.view_ops.push(polyamour::ui::Op::with("view3d.view_axis", "FRONT"));
        // Frame both drawings (keeping the cat active).
        let act = app.doc.scene.active_object;
        app.doc.scene.selected = app.doc.scene.objects.iter().enumerate().filter(|(_, o)| matches!(o.data, sex_3d::scene::ObjectData::GreasePencil(_))).map(|(i, _)| i).collect();
        app.doc.view_ops.push(polyamour::ui::Op::new("view3d.view_selected"));
        app.frame(Input::Tick);
        app.doc.scene.selected = act.into_iter().collect();
        app.frame(Input::Tick);
    }
    if rig_demo {
        use polyamour::editors::{Editor, EditorKind};
        let ws = app.active_ws;
        for area in app.workspaces[ws].areas_mut() {
            if area.editor.kind() == EditorKind::DopeSheet {
                area.editor = Editor::new(EditorKind::GraphEditor);
            }
        }
        app.frame(Input::Tick);
    }
    if physics_demo {
        let ws = app.active_ws;
        for area in app.workspaces[ws].areas_mut() {
            if let polyamour::editors::Editor::Properties(s) = &mut area.editor {
                s.tab = polyamour::editors::properties::Tab::Physics;
            }
        }
        app.frame(Input::Tick);
    }
    if let Some(out) = out {
        // Let progressive views (Material Preview, Rendered) settle.
        for _ in 0..64 {
            if !app.wants_tick() {
                break;
            }
            app.frame(Input::Tick);
        }
        let f = std::fs::File::create(&out).expect("create png");
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), app.painter.width as u32, app.painter.height as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header().unwrap().write_image_data(&app.painter.to_rgba()).unwrap();
        return;
    }
    if let Err(e) = polyamour::desktop::run(app) {
        eprintln!("polyamour: {e}");
        print_usage();
        std::process::exit(1);
    }
}

/// Shell's key codes (PS/2 set 1) to Blender's key names. `keypad`: Shell marks
/// numpad keys (asked for with `want_keys(KEYPAD)`), so the grey cursor block that
/// shares their codes is told apart.
#[cfg(target_os = "hermit")]
fn key_of(code: u8, keypad: bool, ctrl: bool, ch: Option<char>) -> Option<Key> {
    if keypad {
        return Some(match code {
            0x47 => Key::Num(7),
            0x48 => Key::Num(8),
            0x49 => Key::Num(9),
            0x4A => Key::NumMinus,
            0x4B => Key::Num(4),
            0x4C => Key::Num(5),
            0x4D => Key::Num(6),
            0x4E => Key::NumPlus,
            0x4F => Key::Num(1),
            0x50 => Key::Num(2),
            0x51 => Key::Num(3),
            0x52 => Key::Num(0),
            0x53 => Key::NumDot,
            0x37 => Key::NumStar,
            0x35 => Key::NumSlash,
            0x1C => Key::NumEnter,
            _ => return None,
        });
    }
    Some(match code {
        // Without the keypad mark (an older Shell), the grey Page Up/Down share the
        // numpad's codes: with Ctrl they are Blender's workspace switch, alone the
        // numpad views.
        0x49 if ctrl => Key::PageUp,
        0x51 if ctrl => Key::PageDown,
        0x01 => Key::Esc,
        0x0E => Key::Backspace,
        0x0F => Key::Tab,
        0x1C => Key::Enter,
        0x39 => Key::Space,
        0x3B..=0x44 => Key::F(code - 0x3A),
        0x57 => Key::F(11),
        0x58 => Key::F(12),
        0x47 => Key::Num(7),
        0x48 => Key::Num(8),
        0x49 => Key::Num(9),
        0x4A => Key::NumMinus,
        0x4B => Key::Num(4),
        0x4C => Key::Num(5),
        0x4D => Key::Num(6),
        0x4E => Key::NumPlus,
        0x4F => Key::Num(1),
        0x50 => Key::Num(2),
        0x51 => Key::Num(3),
        0x52 => Key::Num(0),
        0x53 => Key::NumDot,
        0x37 => Key::NumStar,
        // Shell's codes for the grey cursor block.
        0x67 => Key::Up,
        0x6C => Key::Down,
        0x69 => Key::Left,
        0x6A => Key::Right,
        0x66 => Key::Home,
        0x6B => Key::End,
        0x68 => Key::PageUp,
        0x6D => Key::PageDown,
        0x6F => Key::Delete,
        _ => {
            let c = sex_std::sex_pdx::app_key::us_char(sex_std::sex_pdx::app_key::Key::new(code, false, false)).map(|b| b as char).or(ch)?;
            Key::Char(c.to_ascii_lowercase())
        }
    })
}

#[cfg(target_os = "hermit")]
fn main() {
    use sex_std::sex_app::Event;
    use sex_std::sex_pdx::app_pointer::Kind;
    let window = sex_std::window();
    window.title(b"Polyamour");
    // Tab (Edit Mode), F1-F10 and F12 (Render), backtick (view pie) and a numpad
    // told apart from the grey keys, as in Blender.
    window.want_keys(sex_std::sex_pdx::app_key::DESKTOP_KEYS | sex_std::sex_pdx::app_key::KEYPAD);
    let (w, h) = window.size();
    let mut app = App::new(w.max(640), h.max(400));
    let mut python: Vec<(String, String)> = Vec::new();
    let mut wasm_files: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--python" | "--python-expr" => {
                if let Some(v) = args.next() {
                    python.push((a.clone(), v));
                }
            }
            "--wasm" => {
                if let Some(v) = args.next() {
                    wasm_files.push(v);
                }
            }
            p => open_arg(&mut app, p),
        }
    }
    let mut mods;
    let mut last_ws = String::from("Layout");
    // A Ctrl+left press stands in for the middle button until it is released.
    let mut emulated_middle = false;
    let show = |app: &App, window: &mut sex_std::sex_app::Window| {
        let px = window.pixels();
        let n = app.painter.pixels.len().min(px.len());
        px[..n].copy_from_slice(&app.painter.pixels[..n]);
        window.show_pixels(app.painter.width, app.painter.height);
    };
    app.frame(Input::Tick);
    show(&app, window);
    let (target, task) = sex_std::identity();
    sex_std::sex_pdx::serial_record!("[polyamour.ready] target={} task={} size={}x{}", target, task, app.painter.width, app.painter.height);
    for (f, v) in &python {
        run_python_arg(&mut app, f, v);
        let mode = if app.doc.python.live() { "live" } else { "batch" };
        sex_std::sex_pdx::serial_record!("[polyamour.python] mode={} objects={} why={}", mode, app.doc.scene.objects.len(), app.doc.python.unavailable_reason().replace('\n', " | "));
        sex_std::sex_pdx::serial_record!("[polyamour.report] {}", app.doc.report);
        app.frame(Input::Tick);
        show(&app, window);
    }
    for w in &wasm_files {
        polyamour::python::run_wasm_file(&mut app.doc, std::path::Path::new(w), &[]);
        sex_std::sex_pdx::serial_record!("[polyamour.wasm] file={} objects={} report={}", w, app.doc.scene.objects.len(), app.doc.report);
        app.frame(Input::Tick);
        show(&app, window);
    }
    while !app.quit {
        let ev = if app.doc.playing {
            window.next_within(1000 / 24)
        } else if app.wants_tick() {
            // A viewport is refining its render: wake up soon if nothing happens.
            window.next_within(10)
        } else {
            Some(window.next())
        };
        let input = match ev {
            None => {
                if app.doc.playing {
                    app.doc.frame = if app.doc.frame >= app.doc.scene.frame_end { app.doc.scene.frame_start } else { app.doc.frame + 1 };
                }
                Some(Input::Tick)
            }
            Some(Event::Close) => {
                sex_std::sex_pdx::serial_record!("[polyamour.closed]");
                break;
            }
            Some(Event::Resize { width, height }) => Some(Input::Resize { width, height }),
            Some(Event::Key(k)) => {
                mods = Mods { shift: k.shift, ctrl: k.control, alt: k.alt };
                let text = sex_std::sex_app::text(k);
                // Ctrl+Shift+R stays a stand-in for F12 (Render Image) where Shell
                // keeps the function keys.
                if k.control && k.shift && k.code == 0x13 {
                    Some(Input::Key { key: Key::F(12), mods: Mods::NONE, text: None })
                } else {
                    key_of(k.code, k.keypad, k.control, text).map(|key| Input::Key { key, mods, text })
                }
            }
            Some(Event::Pointer(p)) => {
                let (x, y) = (p.x as i32, p.y as i32);
                // Keys held during the click ride in the pointer word.
                {
                    use sex_std::sex_pdx::app_pointer::{ALT, CONTROL, SHIFT};
                    mods = Mods { shift: p.mods & SHIFT != 0, ctrl: p.mods & CONTROL != 0, alt: p.mods & ALT != 0 };
                }
                let button = |b: u8| match b {
                    1 => Button::Middle,
                    2 => Button::Right,
                    _ => Button::Left,
                };
                // Blender's "Emulate 3 Button Mouse": Alt+left drag is middle drag
                // (orbit; Shift pans, Ctrl zooms), for laptops without a middle button.
                Some(match p.kind {
                    Kind::Move => Input::Move { x, y },
                    Kind::Down { button: b } => {
                        emulated_middle = b == 0 && mods.alt;
                        let b = if emulated_middle { Button::Middle } else { button(b) };
                        Input::Down { x, y, button: b, mods: if emulated_middle { Mods { alt: false, ..mods } } else { mods } }
                    }
                    Kind::Up { button: b } => {
                        let b = if b == 0 && emulated_middle { Button::Middle } else { button(b) };
                        emulated_middle = false;
                        Input::Up { x, y, button: b }
                    }
                    Kind::Wheel { steps } => Input::Wheel { x, y, steps: steps as i32, mods },
                    Kind::Leave => continue,
                })
            }
            Some(_) => None,
        };
        if let Some(input) = input {
            let report = app.doc.report.clone();
            app.frame(input);
            show(&app, window);
            for op in app.op_log.drain(..) {
                sex_std::sex_pdx::serial_record!("[polyamour.op] {}", op);
            }
            if app.doc.report != report {
                sex_std::sex_pdx::serial_record!("[polyamour.report] {}", app.doc.report);
            }
            let ws = &app.workspaces[app.active_ws].name;
            if *ws != last_ws {
                last_ws = ws.clone();
                sex_std::sex_pdx::serial_record!("[polyamour.workspace] {}", last_ws);
            }
        }
    }
    std::process::exit(0);
}
