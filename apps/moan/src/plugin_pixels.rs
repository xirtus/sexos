//! Qualification metadata from the actual Weft label model and painted pixels.
//! Domain-result telemetry is intentionally separate from this visible-node view.
extern crate alloc;
use alloc::string::String;
use weft::{widget::Id, Ui};
pub struct Snapshot {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub hash: u64,
    pub text: String,
}
pub fn snapshot(ui: &Ui, id: Id, pixels: &[u32]) -> Option<Snapshot> {
    let (width, height) = ui.size();
    let (width, height) = (width as usize, height as usize);
    let r = ui.rect(id);
    let scale = ui.theme().scale;
    let values = [r.x * scale, r.y * scale, r.w * scale, r.h * scale];
    if values.iter().any(|v| !v.is_finite()) || values[0] < 0.0 || values[1] < 0.0 {
        return None;
    }
    let (x, y) = (values[0] as usize, values[1] as usize);
    let (right, bottom) = (
        (values[0] + values[2] + 0.999) as usize,
        (values[1] + values[3] + 0.999) as usize,
    );
    if right <= x
        || bottom <= y
        || right > width
        || bottom > height
        || pixels.len() < width * height
    {
        return None;
    }
    let text = ui.text_of(id);
    if text.len() > 128 {
        return None;
    }
    let mut hash = 0xcbf29ce484222325u64;
    for row in y..bottom {
        for pixel in &pixels[row * width + x..row * width + right] {
            for byte in [(pixel >> 16) as u8, (pixel >> 8) as u8, *pixel as u8] {
                hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
            }
        }
    }
    Some(Snapshot {
        x,
        y,
        w: right - x,
        h: bottom - y,
        hash,
        text: text.into(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use weft::{Strings, Theme};
    #[test]
    fn real_label_model_rejects_a11y_only_status_update() {
        let mut ui = Ui::new(640, 240, Theme::dark(), Strings::new("en"));
        let root = ui.root();
        let column = ui.column(root);
        let label = ui.label(column, "Ready");
        let mut pixels = vec![0; 640 * 240];
        ui.paint(&mut pixels).unwrap();
        let old = snapshot(&ui, label, &pixels).unwrap();
        ui.set_label(label, "CLAP stereo swap verified");
        ui.paint(&mut pixels);
        let stale = snapshot(&ui, label, &pixels).unwrap();
        assert_eq!(stale.text, "Ready");
        assert_eq!(old.hash, stale.hash);
        assert_ne!(stale.text, "CLAP stereo swap verified");
        ui.set_text(label, "CLAP stereo swap verified");
        ui.paint(&mut pixels).unwrap();
        let real = snapshot(&ui, label, &pixels).unwrap();
        assert_eq!(real.text, "CLAP stereo swap verified");
        assert_ne!(real.hash, old.hash);
    }
    #[test]
    fn painted_wrong_label_keeps_wrong_model_value() {
        let mut ui = Ui::new(640, 240, Theme::dark(), Strings::new("en"));
        let root = ui.root();
        let label = ui.label(root, "Wrong");
        let mut pixels = vec![0; 640 * 240];
        ui.paint(&mut pixels).unwrap();
        let wrong = snapshot(&ui, label, &pixels).unwrap();
        assert_ne!(wrong.text, "VST3 gain 0.25 verified");
        assert_eq!(wrong.text, "Wrong");
    }
}
