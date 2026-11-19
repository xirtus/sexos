use moan::Studio;
use weft::{Strings, Theme, Ui};
fn main() {
    let (w, h) = (720, 600);
    let mut ui = Ui::new(w, h, Theme::dark(), Strings::new("en"));
    let mut s = Studio::build(&mut ui);
    s.session.tone().unwrap();
    s.session.tone().unwrap();
    s.session.cursor = 48000;
    s.session.split().unwrap();
    s.refresh(&mut ui);
    let mut pixels = vec![0; w as usize * h as usize];
    ui.paint(&mut pixels);
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in pixels {
        ppm.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
    }
    std::fs::write(std::env::args().nth(1).unwrap(), ppm).unwrap();
}
