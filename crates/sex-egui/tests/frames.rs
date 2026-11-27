// SPDX-License-Identifier: GPL-3.0-only
//! Whole frames on the host: egui input from SexOS events, the CPU rasterizer's pixels.
use sex_egui::keys::{self, PointerKind};
use sex_egui::raster::Target;
use sex_egui::{raw_input, Painter};

const W: usize = 320;
const H: usize = 200;

struct Rig { ctx: egui::Context, painter: Painter, pixels: Vec<u32>, time: f64 }

impl Rig {
    fn new() -> Self { Self { ctx: sex_egui::new_context(), painter: Painter::new(), pixels: vec![0; W * H], time: 0.0 } }
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

#[test]
fn wheel_steps_scroll_a_scroll_area_under_the_pointer() {
    let mut rig = Rig::new();
    let first = std::cell::Cell::new(0usize);
    // The area keeps its full width (auto_shrink would narrow it to the text, and a
    // wheel over the blank part would miss it).
    let mut ui = |u: &mut egui::Ui| { {
        let mut list = u.new_child(egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(egui::pos2(10.0, 30.0), egui::vec2(300.0, 120.0))));
        let f = egui::ScrollArea::vertical().id_salt("lines").auto_shrink([false, false]).show(&mut list, |ui| {
            let mut f = None;
            for i in 0..60 {
                let r = ui.label(format!("Line {i}"));
                if f.is_none() && ui.clip_rect().contains(r.rect.center()) { f = Some(i); }
            }
            f.unwrap_or(0)
        }).inner;
        first.set(f);
    } };
    rig.frame(vec![], &mut ui);
    let mut e = Vec::new();
    keys::pointer_events(100, 90, PointerKind::Move, 1.0, egui::Modifiers::NONE, &mut e);
    rig.frame(e, &mut ui);
    for _ in 0..4 {
        let mut e = Vec::new();
        keys::pointer_events(100, 90, PointerKind::Wheel(-1), 1.0, egui::Modifiers::NONE, &mut e);
        rig.frame(e, &mut ui);
    }
    for _ in 0..20 { rig.frame(vec![], &mut ui); }
    assert!(first.get() > 0, "first visible line stayed {}", first.get());
}

#[test]
fn noir_fonts_and_obsidian_visuals_apply_cleanly() {
    let defs = sex_egui::noir_font_definitions();
    let prop = &defs.families[&egui::FontFamily::Proportional];
    let semibold = &defs.families[&egui::FontFamily::Name("SemiBold".into())];
    let mono = &defs.families[&egui::FontFamily::Monospace];
    assert_eq!(prop.first().map(String::as_str), Some("Geist-Medium"));
    assert_eq!(semibold.first().map(String::as_str), Some("Geist-SemiBold"));
    assert_eq!(mono.first().map(String::as_str), Some("GeistMono-Medium"));
    assert!(prop.iter().any(|f| f == "egui-icons"));

    let ctx = sex_egui::new_context();
    let style = ctx.global_style();
    assert_eq!(style.visuals.panel_fill, egui::Color32::from_rgb(0x0B, 0x0D, 0x12));
    assert_eq!(style.visuals.window_fill, egui::Color32::from_rgb(0x13, 0x16, 0x1F));
    assert_eq!(style.visuals.extreme_bg_color, egui::Color32::from_rgb(0x08, 0x0A, 0x0E));
    assert_eq!(style.visuals.widgets.inactive.bg_fill, egui::Color32::from_rgb(0x1B, 0x20, 0x2C));
    assert_eq!(style.visuals.widgets.hovered.bg_fill, egui::Color32::from_rgb(0x23, 0x29, 0x38));
    assert_eq!(style.text_styles[&egui::TextStyle::Button].family, egui::FontFamily::Name("SemiBold".into()));
    assert_eq!(style.text_styles[&egui::TextStyle::Heading].family, egui::FontFamily::Name("SemiBold".into()));
    assert_eq!(style.text_styles[&egui::TextStyle::Body].family, egui::FontFamily::Proportional);

    // Distinct active fills across all 4 NoirAccent variants.
    let actives: Vec<_> = sex_egui::NoirAccent::ALL
        .into_iter()
        .map(|a| {
            sex_egui::apply_noir_theme_with_accent(&ctx, a);
            ctx.global_style().visuals.widgets.active.bg_fill
        })
        .collect();
    for i in 0..4 {
        for j in (i + 1)..4 {
            assert_ne!(actives[i], actives[j]);
        }
    }
}

fn click_with(x: i16, y: i16, button: u8, mods: egui::Modifiers) -> Vec<egui::Event> {
    let mut e = Vec::new();
    keys::pointer_events(x, y, PointerKind::Down(button), 1.0, mods, &mut e);
    keys::pointer_events(x, y, PointerKind::Up(button), 1.0, mods, &mut e);
    e
}

fn press_key(code: u8, shift: bool, ctrl: bool, alt: bool, sup: bool, ch: Option<char>, clip: &str) -> Vec<egui::Event> {
    let mut e = Vec::new();
    keys::key_events_with(code, shift, ctrl, alt, sup, ch, clip, &mut e);
    e
}

#[test]
fn super_and_ctrl_shortcuts_work_plus_alt_and_super_navigation() {
    use std::cell::RefCell;
    let mut rig = Rig::new();
    let text = RefCell::new(String::from("alpha beta gamma"));
    let rect = std::cell::Cell::new(egui::Rect::NOTHING);
    let mut ui = |u: &mut egui::Ui| {
        let mut s = text.borrow_mut();
        rect.set(u.add(sex_egui::TextEdit::singleline(&mut *s)).rect);
    };
    rig.frame(vec![], &mut ui);
    let c = rect.get().center();
    rig.frame(click(c.x as i16, c.y as i16), &mut ui);

    // Super+Right jumps to end, Alt+Backspace deletes last word
    rig.frame(press_key(0x4D, false, false, false, true, None, ""), &mut ui);
    rig.frame(press_key(0x0E, false, false, true, false, None, ""), &mut ui);
    assert_eq!(*text.borrow(), "alpha beta ");

    // Super+A, Super+C, Super+X, Super+V, Super+Z, Super+Shift+Z
    rig.frame(press_key(0x1E, false, false, false, true, Some('a'), ""), &mut ui);
    let out = rig.ctx.run_ui(raw_input(W, H, 1.0, 1.0, rig.time + 0.1, press_key(0x2E, false, false, false, true, Some('c'), "")), |u| ui(u));
    let copied = out.platform_output.commands.iter().find_map(|c| match c {
        egui::OutputCommand::CopyText(t) => Some(t.clone()),
        _ => None,
    });
    assert_eq!(copied.as_deref(), Some("alpha beta "));
    rig.frame(press_key(0x2D, false, false, false, true, Some('x'), ""), &mut ui);
    assert!(text.borrow().is_empty());
    // Allow egui's 1s undo coalescing window to seal before pasting.
    rig.time += 1.2;
    rig.frame(vec![], &mut ui);
    rig.frame(press_key(0x2F, false, false, false, true, Some('v'), "noir"), &mut ui);
    assert_eq!(*text.borrow(), "noir");
    rig.time += 1.2;
    rig.frame(vec![], &mut ui);
    rig.frame(press_key(0x2C, false, false, false, true, Some('z'), ""), &mut ui);
    assert_eq!(*text.borrow(), "");
    rig.frame(press_key(0x2C, true, false, false, true, Some('z'), ""), &mut ui);
    assert_eq!(*text.borrow(), "noir");
    rig.frame(press_key(0x2C, false, false, false, true, Some('z'), ""), &mut ui);
    assert_eq!(*text.borrow(), "");
    rig.frame(press_key(0x15, false, false, false, true, Some('y'), ""), &mut ui);
    assert_eq!(*text.borrow(), "noir");
}

#[test]
fn noir_text_edit_ghost_completion_tab_and_right_arrow_plus_autocorrect_revert() {
    use std::cell::RefCell;
    let mut rig = Rig::new();
    let text = RefCell::new(String::new());
    let other = RefCell::new(String::new());
    let field_rect = std::cell::Cell::new(egui::Rect::NOTHING);

    let mut ui = |u: &mut egui::Ui| {
        field_rect.set(u.add(sex_egui::TextEdit::singleline(&mut *text.borrow_mut())).rect);
        u.add(sex_egui::TextEdit::singleline(&mut *other.borrow_mut()));
    };

    rig.frame(vec![], &mut ui);
    let c = field_rect.get().center();
    rig.frame(click(c.x as i16, c.y as i16), &mut ui);

    // Type "pref" -> ghost completion "erences" shown -> Tab accepts without moving focus!
    let mut evs = Vec::new();
    for (code, ch) in [(0x19u8, 'p'), (0x13, 'r'), (0x12, 'e'), (0x21, 'f')] {
        keys::key_events(code, false, false, Some(ch), "", &mut evs);
    }
    rig.frame(evs, &mut ui);
    assert_eq!(*text.borrow(), "pref");

    // Tab accepts -> "preferences"
    rig.frame(press_key(0x0F, false, false, false, false, None, ""), &mut ui);
    assert_eq!(*text.borrow(), "preferences");

    // Type " avail" -> Right Arrow accepts -> "preferences available"
    let mut evs2 = Vec::new();
    for (code, ch) in [(0x39u8, ' '), (0x1E, 'a'), (0x2F, 'v'), (0x1E, 'a'), (0x17, 'i'), (0x26, 'l')] {
        keys::key_events(code, false, false, Some(ch), "", &mut evs2);
    }
    rig.frame(evs2, &mut ui);
    rig.frame(press_key(0x4D, false, false, false, false, None, ""), &mut ui);
    assert_eq!(*text.borrow(), "preferences available");

    // Type " teh" then Space -> autocorrect to "preferences available the ", then 1 Backspace reverts!
    let mut evs3 = Vec::new();
    for (code, ch) in [(0x39u8, ' '), (0x14, 't'), (0x12, 'e'), (0x23, 'h')] {
        keys::key_events(code, false, false, Some(ch), "", &mut evs3);
    }
    rig.frame(evs3, &mut ui);
    rig.frame(press_key(0x39, false, false, false, false, Some(' '), ""), &mut ui);
    assert_eq!(*text.borrow(), "preferences available the ");

    // 1 Backspace reverts "the " -> "teh "
    rig.frame(press_key(0x0E, false, false, false, false, None, ""), &mut ui);
    assert_eq!(*text.borrow(), "preferences available teh ");
    assert!(other.borrow().is_empty(), "Tab stayed inside field while accepting ghost text");
}

#[test]
fn noir_text_edit_ctrl_click_context_menu_replaces_misspelling() {
    use std::cell::RefCell;
    let mut rig = Rig::new();
    let text = RefCell::new(String::from("recieve"));
    let field_rect = std::cell::Cell::new(egui::Rect::NOTHING);

    let mut ui = |u: &mut egui::Ui| {
        field_rect.set(u.add(sex_egui::TextEdit::singleline(&mut *text.borrow_mut())).rect);
    };
    rig.frame(vec![], &mut ui);

    let r = field_rect.get();
    // Ctrl + Left-Click near start of "recieve" opens Noir context popup with "receive" at top!
    let ctrl_mod = keys::modifiers_with(false, true, false, false);
    rig.frame(click_with((r.left() + 18.0) as i16, r.center().y as i16, 0, ctrl_mod), &mut ui);
    rig.frame(vec![], &mut ui);

    // Click first row ("receive") inside the popup
    rig.frame(click((r.left() + 45.0) as i16, (r.center().y + 18.0) as i16), &mut ui);
    assert_eq!(*text.borrow(), "receive");
}

#[test]
fn noir_text_edit_code_mode_disables_autocorrect_and_ghost() {
    use std::cell::RefCell;
    let mut rig = Rig::new();
    let text = RefCell::new(String::new());
    let field_rect = std::cell::Cell::new(egui::Rect::NOTHING);

    let mut ui = |u: &mut egui::Ui| {
        field_rect.set(
            u.add(sex_egui::TextEdit::singleline(&mut *text.borrow_mut()).mode(sex_egui::TextMode::Code))
                .rect,
        );
    };
    rig.frame(vec![], &mut ui);
    let c = field_rect.get().center();
    rig.frame(click(c.x as i16, c.y as i16), &mut ui);

    let mut evs = Vec::new();
    for (code, ch) in [(0x14u8, 't'), (0x12, 'e'), (0x23, 'h'), (0x39, ' ')] {
        keys::key_events(code, false, false, Some(ch), "", &mut evs);
    }
    rig.frame(evs, &mut ui);
    assert_eq!(*text.borrow(), "teh ");
}
