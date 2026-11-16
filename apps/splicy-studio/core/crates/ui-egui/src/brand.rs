//! Splicy's wordmark: the name painted in the interface's own type, each letter its own
//! colour (Splicy's tracks are colourful; so is its name). No image files.

const LETTERS: [(char, [u8; 3]); 6] =
    [('S', [0xf4, 0x3f, 0x5e]), ('p', [0xf9, 0x73, 0x16]), ('l', [0xea, 0xb3, 0x08]), ('i', [0x22, 0xc5, 0x5e]), ('c', [0x06, 0xb6, 0xd4]), ('y', [0x8b, 0x5c, 0xf6])];

/// Draw the wordmark `height` points tall with its left edge at `left_center` (vertically centred).
/// Returns the drawn rect.
pub fn paint_wordmark(ui: &egui::Ui, left_center: egui::Pos2, height: f32, _dark: bool) -> Option<egui::Rect> {
    let mut job = egui::text::LayoutJob::default();
    let font = crate::theme::Tokens::semibold(height * 1.15);
    for (c, [r, g, b]) in LETTERS {
        job.append(&c.to_string(), 0.0, egui::TextFormat::simple(font.clone(), egui::Color32::from_rgb(r, g, b)));
    }
    let galley = ui.painter().layout_job(job);
    let r = egui::Rect::from_min_size(egui::pos2(left_center.x, left_center.y - galley.size().y / 2.0), galley.size());
    ui.painter().galley(r.min, galley, egui::Color32::WHITE);
    Some(r)
}
