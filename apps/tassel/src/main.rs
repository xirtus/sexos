// SPDX-License-Identifier: GPL-3.0-only
//! Tassel: egui's own demo app, unmodified, as a SexOS program (`run tassel`).
//!
//! It proves the egui backend (crates/sex-egui): every widget in egui's demo, with the
//! mouse, wheel and keyboard. `run tassel probe` shows a small fixed panel instead,
//! which the boot gate drives with clicks and keys and reads back from the serial record.

fn main() {
    let probe = std::env::args().any(|a| a == "probe");
    let mut options = sex_egui::Options::new("Tassel");
    if probe {
        options.trace_frames = true;
        let mut clicks = 0u32;
        let mut text = String::new();
        let mut checked = false;
        let mut said = String::new();
        let mut scrolled = 0i32;
        sex_egui::run_ui(options, move |ui| {
            {
                // A fixed layout, so the gate knows where each widget is (points from
                // the window's top-left): button 10,10 100x28; field 10,50 200x28;
                // checkbox 10,90; a scrolling list of 60 lines at 10,130 300x120.
                let at = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
                if ui.put(at(10.0, 10.0, 100.0, 28.0), egui::Button::new("Click me")).clicked() {
                    clicks += 1;
                    sex_egui::record(&format!("[tassel.clicked] count={clicks}"));
                }
                let r = ui.put(at(10.0, 50.0, 200.0, 28.0), egui::TextEdit::singleline(&mut text));
                if r.changed() && text != said {
                    said = text.clone();
                    sex_egui::record(&format!("[tassel.text] value={text}"));
                }
                if ui.put(at(10.0, 90.0, 200.0, 24.0), egui::Checkbox::new(&mut checked, "Checked")).changed() {
                    sex_egui::record(&format!("[tassel.checked] value={checked}"));
                }
                let mut list = ui.new_child(egui::UiBuilder::new().max_rect(at(10.0, 130.0, 300.0, 120.0)));
                let first = egui::ScrollArea::vertical().id_salt("lines").auto_shrink([false, false]).show(&mut list, |ui| {
                    let mut first = None;
                    for i in 0..60 {
                        let r = ui.label(format!("Line {i}"));
                        if first.is_none() && ui.clip_rect().contains(r.rect.center()) { first = Some(i); }
                    }
                    first.unwrap_or(0)
                }).inner;
                if first != scrolled {
                    scrolled = first;
                    sex_egui::record(&format!("[tassel.scrolled] first={first}"));
                }
            }
        });
    }
    let mut demo = egui_demo_lib::DemoWindows::default();
    let mut loaders = false;
    sex_egui::run_ui(options, move |ui| {
        if !loaders { egui_extras::install_image_loaders(ui.ctx()); loaders = true; }
        demo.ui(ui)
    });
}
