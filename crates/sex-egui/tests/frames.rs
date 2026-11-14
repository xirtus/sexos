// SPDX-License-Identifier: GPL-3.0-only
//! Whole frames on the host: egui input from SexOS events, the CPU rasterizer's pixels.
use sex_egui::keys::{self, PointerKind};
use sex_egui::raster::Target;
use sex_egui::{raw_input, Painter};

const W: usize = 320;
const H: usize = 200;

struct Rig { ctx: egui::Context, painter: Painter, pixels: Vec<u32>, time: f64 }

impl Rig {
    fn new() -> Self { Self { ctx: egui::Context::default(), painter: Painter::new(), pixels: vec![0; W * H], time: 0.0 } }
    fn frame(&mut self, events: Vec<egui::Event>, ui: &mut dyn FnMut(&mut egui::Ui)) {
        self.time += 0.1;
        let out = self.ctx.run_ui(raw_input(W, H, 1.0, 1.0, self.time, events), |u| ui(u));
        self.painter.paint(&self.ctx, out, &mut Target { pixels: &mut self.pixels, width: W, height: H, stride: W });
    }
}

fn click(x: i16, y: i16) -> Vec<egui::Event> {
    let mut e = Vec::new();
    keys::pointer_events(x, y, PointerKind::Down(0), 1.0, egui::Modifiers::NONE, &mut e);
    keys::pointer_events(x, y, PointerKind::Up(0), 1.0, egui::Modifiers::NONE, &mut e);
    e
}

#[test]
fn a_click_on_a_button_clicks_it_and_typing_fills_a_field() {
    let mut rig = Rig::new();
    let mut clicks = 0;
    let mut text = String::new();
    let button_rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let field_rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let mut ui = |u: &mut egui::Ui| {
        let r = u.button("Press");
        button_rect.set(r.rect);
        if r.clicked() { clicks += 1; }
        field_rect.set(u.text_edit_singleline(&mut text).rect);
    };
    rig.frame(vec![], &mut ui);
    rig.frame(vec![], &mut ui);
    let c = button_rect.get().center();
    rig.frame(click(c.x as i16, c.y as i16), &mut ui);
    rig.frame(vec![], &mut ui);
    let f = field_rect.get().center();
    rig.frame(click(f.x as i16, f.y as i16), &mut ui);
    let mut typed = Vec::new();
    for (code, ch) in [(0x23u8, 'h'), (0x17, 'i')] { keys::key_events(code, false, false, Some(ch), "", &mut typed); }
    rig.frame(typed, &mut ui);
    // Backspace, then a character only a layout types.
    let mut more = Vec::new();
    keys::key_events(0x0E, false, false, None, "", &mut more);
    keys::key_events(0x1A, false, false, Some('ü'), "", &mut more);
    rig.frame(more, &mut ui);
    drop(ui);
    assert_eq!(clicks, 1);
    assert_eq!(text, "hü");
}

#[test]
fn copy_and_paste_between_fields() {
    let mut rig = Rig::new();
    let mut a = String::from("copied");
    let mut b = String::new();
    let rects = [std::cell::Cell::new(egui::Rect::NOTHING), std::cell::Cell::new(egui::Rect::NOTHING)];
    let mut copied = None;
    let mut ui = |u: &mut egui::Ui| {
        rects[0].set(u.text_edit_singleline(&mut a).rect);
        rects[1].set(u.text_edit_singleline(&mut b).rect);
    };
    rig.frame(vec![], &mut ui);
    let c = rects[0].get().center();
    rig.frame(click(c.x as i16, c.y as i16), &mut ui);
    // Ctrl+A, Ctrl+C.
    let mut e = Vec::new();
    keys::key_events(0x1E, false, true, None, "", &mut e);
    keys::key_events(0x2E, false, true, None, "", &mut e);
    rig.time += 0.1;
    let out = rig.ctx.run_ui(raw_input(W, H, 1.0, 1.0, rig.time, e), |u| ui(u));
    for cmd in &out.platform_output.commands {
        if let egui::OutputCommand::CopyText(t) = cmd { copied = Some(t.clone()); }
    }
    rig.painter.paint(&rig.ctx, out, &mut Target { pixels: &mut rig.pixels, width: W, height: H, stride: W });
    let clip = copied.clone().expect("Ctrl+C copied the selection");
    let d = rects[1].get().center();
    rig.frame(click(d.x as i16, d.y as i16), &mut ui);
    let mut v = Vec::new();
    keys::key_events(0x2F, false, true, None, &clip, &mut v);
    rig.frame(v, &mut ui);
    drop(ui);
    assert_eq!(b, "copied");
}

#[test]
fn the_unmodified_demo_paints_text_and_widgets() {
    let mut rig = Rig { pixels: vec![0; 1280 * 800], ..Rig::new() };
    let mut demo = egui_demo_lib::DemoWindows::default();
    let mut painter = Painter::new();
    let mut stats = Default::default();
    for _ in 0..3 {
        rig.time += 0.1;
        let out = rig.ctx.run_ui(raw_input(1280, 800, 1.0, 1.0, rig.time, vec![]), |u| demo.ui(u));
        stats = painter.paint(&rig.ctx, out, &mut Target { pixels: &mut rig.pixels, width: 1280, height: 800, stride: 1280 });
    }
    assert!(stats.triangles > 1000, "{stats:?}");
    let bg = rig.pixels[1280 * 799 + 640];
    let distinct = rig.pixels.iter().filter(|p| **p != bg).count();
    assert!(distinct > 20_000, "only {distinct} pixels differ from the background");
}
