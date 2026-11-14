// SPDX-License-Identifier: GPL-3.0-only
//! Paints a window of every widget to a PNG: `cargo run -p weft --example gallery -- out.png [dark|hc] [de]`.
use weft::input::{code, Event, Key};
use weft::menu::{Command, Menu};
use weft::widget::{Column, Layout, Rows, Tab, TextStyle};
use weft::{Strings, Theme, Ui};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args.get(1).cloned().unwrap_or("gallery.png".into());
    let theme = match args.get(2).map(|s| s.as_str()) { Some("dark") => Theme::dark(), Some("hc") => Theme::high_contrast(), _ => Theme::light() };
    let lang = args.get(3).cloned().unwrap_or("en".into());
    let (w, h) = (1000u32, 680u32);
    let mut ui = Ui::new(w, h, theme, Strings::new(&lang));
    for (id, label, key) in [("app.new", "New", "Ctrl+N"), ("app.open", "Open…", "Ctrl+O"), ("app.save", "Save", "Ctrl+S"), ("app.quit", "Quit", "Ctrl+Q")] {
        ui.commands.add(Command::new(id, label).shortcut(key));
    }
    ui.commands.add(Command::new("app.wrap", "Word Wrap").checked(true));
    let root = ui.root();
    ui.menubar(root, vec![
        Menu::new("File").command("app.new").command("app.open").command("app.save").separator().command("app.quit"),
        Menu::new("Edit").command("weft.undo").command("weft.redo"),
        Menu::new("View").command("app.wrap"),
    ]);
    let tabs = ui.document_tabs(root);
    let page = ui.tab_page(tabs, Tab::new("Controls").icon("settings"));
    let mut t2 = Tab::new("notes.txt"); t2.modified = true;
    ui.tab_page(tabs, t2);
    let mut t3 = Tab::new("Pinned"); t3.pinned = true; t3.icon = Some("star".into());
    ui.tab_page(tabs, t3);
    let split = ui.split(page, false, 0.45);
    let left = ui.column(split);
    ui.text(left, "Weft widgets", TextStyle::Title);
    ui.paragraph(left, "Buttons, fields, checks, switches, radios, sliders and progress bars, laid out by taffy and painted with sex-font text.");
    let row = ui.row(left);
    let b = ui.button(row, "Primary"); ui.set_button_style(b, weft::ButtonStyle::Primary);
    ui.button(row, "Normal");
    let d = ui.button(row, "Disabled"); ui.set_enabled(d, false);
    ui.icon_button(row, "trash", "");
    let f = ui.field(left, "Hello, Grüße", "Name"); ui.set_layout(f, Layout::default().fill_width());
    ui.password(left, "Password");
    ui.checkbox(left, "Remember me", true);
    ui.switch(left, "Wi-Fi", true);
    ui.radio(left, "Light", 1, true);
    ui.radio(left, "Dark", 1, false);
    ui.slider(left, 0.0, 100.0, 40.0);
    ui.progress(left, Some(0.65));
    let right = ui.column(split);
    let list = ui.list_with(right, Box::new(Rows((0..200).map(|i| vec![format!("File {i}.txt"), format!("{} KB", i * 3)]).collect())), vec![Column::new("Name", 220.0), Column::new("Size", 80.0).right()]);
    ui.set_sort(list, Some((0, true)));
    ui.select_row(list, Some(2));
    ui.focus(Some(f));
    ui.handle(Event::Key(Key::new(code::TAB)));
    ui.handle(Event::Key(Key::new(code::F10)));
    ui.handle(Event::Key(Key::new(code::DOWN)));
    ui.tick(1000);
    let mut px = vec![0u32; (w * h) as usize];
    ui.paint(&mut px);
    let file = std::fs::File::create(&out).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    enc.write_header().unwrap().write_image_data(&data).unwrap();
    println!("{out}");
}
