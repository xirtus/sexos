// SPDX-License-Identifier: GPL-3.0-only
//! Gaze (View): shows a picture. `run view @photo.jpg` (PNG, JPEG, WebP, GIF, JPEG XL,
//! SVG, and camera RAW through its embedded preview). The decoders (sex-image and
//! gaze-codecs) run here, inside the program's own confinement, not in Display: a
//! hostile file can only crash this window. Photos are turned upright as their EXIF
//! says. A picture larger than the window is shrunk to fit, as far as the program's
//! heap allows. Animated GIF and WebP play, with their own timing and loops.
//!
//! Keys: R turns the picture a quarter clockwise, Shift+R back; + and - zoom, 0 fits
//! the window again, 1 shows it at 100 %; when zoomed the arrows (or a drag) move
//! around it. I shows or hides the photo's details (camera, lens, exposure, date and
//! place taken). Given a folder (`run view @pictures`), View shows its pictures in
//! name order: N or Right for the next, P or Left for the one before, Space starts or
//! stops a slideshow. Given one file, N and P step through its folder when the run
//! may read that folder (`run view @pictures` grants it; `@one.png` grants only one).
//!
//! `view --thumb @IN @OUT SIZE` opens no window: it writes a thumbnail (see `thumb`).
//!
//! An ordinary `std` program (x86_64-sex-std): JPEG XL and SVG need the standard library.
#[cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod info;
#[cfg_attr(not(target_os = "hermit"), allow(dead_code))]
mod thumb;
#[cfg(target_os = "hermit")]
mod app;

fn main() {
    #[cfg(target_os = "hermit")]
    app::main();
}
