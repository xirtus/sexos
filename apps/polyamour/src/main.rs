// SPDX-License-Identifier: GPL-3.0-only
//! Polyamour as a SexOS program: `run polyamour [scene.blend|.glb|.obj|.poly]`.
//! On the host, `polyamour --screenshot out.png [scene] [workspace]` draws the
//! window once into a PNG (used for docs and visual checks).
use polyamour::input::Input;
#[cfg(target_os = "hermit")]
use polyamour::input::{Button, Key, Mods};
use polyamour::App;

fn open_arg(app: &mut App, path: &str) {
    match std::fs::read(path) {
        Ok(bytes) => {
            let dir = std::path::Path::new(path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let resolve = move |p: &str| std::fs::read(dir.join(p)).ok();
            if let Err(e) = app.open_bytes(path, &bytes, &resolve) {
                app.doc.report = e;
            }
        }
        Err(e) => app.doc.report = format!("Cannot open {path}: {e}"),
    }
}

#[cfg(not(target_os = "hermit"))]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut app = App::new(1600, 900);
    let mut out = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--screenshot" => out = it.next().cloned(),
            "--workspace" => {
                if let Some(w) = it.next() {
                    app.doc.want_workspace = Some(w.clone());
                }
            }
            p => open_arg(&mut app, p),
        }
    }
    app.frame(Input::Tick);
    app.frame(Input::Tick);
    let Some(out) = out else {
        eprintln!("usage: polyamour --screenshot out.png [--workspace Name] [scene file]");
        return;
    };
    let f = std::fs::File::create(&out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), app.painter.width as u32, app.painter.height as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.write_header().unwrap().write_image_data(&app.painter.to_rgba()).unwrap();
}

/// Shell's key codes (PS/2 set 1) to Blender's key names.
#[cfg(target_os = "hermit")]
fn key_of(code: u8, ch: Option<char>) -> Option<Key> {
    Some(match code {
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
    let (w, h) = window.size();
    let mut app = App::new(w.max(640), h.max(400));
    if let Some(p) = std::env::args().nth(1) {
        open_arg(&mut app, &p);
    }
    let mut mods = Mods::NONE;
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
    while !app.quit {
        let ev = if app.doc.playing { window.next_within(1000 / 24) } else { Some(window.next()) };
        let input = match ev {
            None => {
                app.doc.frame = if app.doc.frame >= app.doc.scene.frame_end { app.doc.scene.frame_start } else { app.doc.frame + 1 };
                Some(Input::Tick)
            }
            Some(Event::Close) => {
                sex_std::sex_pdx::serial_record!("[polyamour.closed]");
                break;
            }
            Some(Event::Resize { width, height }) => Some(Input::Resize { width, height }),
            Some(Event::Key(k)) => {
                mods = Mods { shift: k.shift, ctrl: k.control, alt: false };
                let text = sex_std::sex_app::text(k);
                key_of(k.code, text).map(|key| Input::Key { key, mods, text })
            }
            Some(Event::Pointer(p)) => {
                let (x, y) = (p.x as i32, p.y as i32);
                let button = |b: u8| match b {
                    1 => Button::Middle,
                    2 => Button::Right,
                    _ => Button::Left,
                };
                // Laptops without a middle button: Alt is not delivered yet, so
                // Ctrl+left drag stands in for middle-drag orbit.
                Some(match p.kind {
                    Kind::Move => Input::Move { x, y },
                    Kind::Down { button: b } => {
                        emulated_middle = b == 0 && mods.ctrl;
                        let b = if emulated_middle { Button::Middle } else { button(b) };
                        Input::Down { x, y, button: b, mods }
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
