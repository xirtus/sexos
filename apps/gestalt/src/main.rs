// SPDX-License-Identifier: GPL-3.0-only
//! Gestalt: the SexOS vector editor. `run gestalt NAME.svg` opens a drawing in the program's
//! folder (SVG, PDF, `.ai`, EPS or `.vectorcraft`; a name that does not exist yet starts an
//! empty 800 × 600 pt artboard saved there). With no name it opens `welcome.svg`, writing it
//! first if it is missing.
//!
//! The engine is VectorCraft's (github.com/storytold/vectorcraft, MIT OR Apache-2.0): its
//! document, tools, Smart Guides, Pathfinder, undo, renderer and file formats run unchanged.
//! Its egui interface follows when egui runs on sex-app; until then [`ui`] draws the canvas.
//!
//! Keys (Illustrator's): V Selection, A Direct Selection, P Pen, N Pencil, M Rectangle,
//! L Ellipse, \ Line, T Type, I Eyedropper, H Hand, Shift+M Shape Builder; arrows nudge
//! (Shift ×10), Delete removes; Ctrl+Z / Ctrl+Shift+Z undo and redo; Ctrl+C/X/V/D; Ctrl+A;
//! Ctrl+G / Ctrl+Shift+G group and ungroup; Ctrl+Shift+1..4 Pathfinder unite, minus front,
//! intersect, exclude; Ctrl+=/-/0/1 zoom; Ctrl+S saves; Ctrl+E exports PNG, Ctrl+Shift+E PDF.
#![cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod drawing;
#[cfg(target_os = "hermit")]
mod ui;

/// The first drawing.
#[cfg_attr(not(target_os = "hermit"), allow(dead_code))]
const WELCOME: &[u8] = include_bytes!("../assets/welcome.svg");

#[cfg(target_os = "hermit")]
fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| {
        if std::fs::metadata("welcome.svg").is_err() {
            if let Err(e) = std::fs::write("welcome.svg", WELCOME) { eprintln!("Could not write welcome.svg: {e}"); }
        }
        "welcome.svg".to_string()
    });
    let opened = match std::fs::read(&name) {
        Ok(bytes) => drawing::Drawing::open(&name, &bytes),
        Err(_) => Ok(drawing::Drawing::blank(&name, 800.0, 600.0)),
    };
    let d = match opened {
        Ok(d) => d,
        Err(message) => {
            println!("Gestalt could not open {name}: {message}\n\nPut a drawing in this program's folder and run: gestalt NAME.svg");
            std::process::exit(1);
        }
    };
    let (target, task) = sex_std::identity();
    let a = d.artboard();
    sex_std::record(&format!("[gestalt.open] target={target} task={task} name={name} objects={} artboard={:.0}x{:.0}", d.objects(), a.width(), a.height()));
    let window = sex_std::window();
    ui::Ui::new(d, name).run(window);
    std::process::exit(0);
}

/// On the host the engine runs only under `cargo test`.
#[cfg(not(target_os = "hermit"))]
fn main() {}

#[cfg(test)]
mod tests {
    #[test]
    fn welcome_opens() {
        let d = crate::drawing::Drawing::open("welcome.svg", crate::WELCOME).unwrap();
        assert_eq!(d.objects(), 4);
        assert_eq!(d.artboard().width(), 800.0);
    }
}
