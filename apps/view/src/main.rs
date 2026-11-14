// SPDX-License-Identifier: GPL-3.0-only
//! View: shows a picture. `run view @photo.png`, `.jpg`, `.webp` or `.gif`. The decoders
//! (zune-png, zune-jpeg, image-webp and sex-image's GIF, through sex-image) run here,
//! inside the program's own confinement, not in Display: a hostile file can only crash
//! this window. A picture larger than the window is shrunk to fit, as far as the
//! program's heap allows. Animated GIF and WebP play, with their own timing and loops.
//!
//! Keys: R turns the picture a quarter clockwise, Shift+R back; + and - zoom, 0 fits
//! the window again, 1 shows it at 100 %; when zoomed the arrows (or a drag) move
//! around it. Given a folder (`run view @pictures`), View shows its pictures in name
//! order: N or Right for the next, P or Left for the one before, Space starts or
//! stops a slideshow. Given one file, N and P step through its folder when the run
//! may read that folder (`run view @pictures` grants it; `@one.png` grants only one).
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use sex_image::view as look;
use sex_image::{Animation, Image};

use sex_app::{files, Event, Window};
use sex_image::{Error, Limits};
use sex_pdx::app_pointer::Kind;
use sex_pdx::pixel_region::{MAX_H, MAX_W, PIXELS};

/// The file as read.
static mut INPUT: [u8; 4 << 20] = [0; 4 << 20];
const BACK: u32 = 0x000D_1420;
/// How long each picture stays up in a slideshow.
const SLIDE_MS: u64 = 2500;

/// The buffer the current file is read into. Whatever borrowed the last file (an
/// animation) is dropped before the next file is read over it.
fn input() -> &'static mut [u8] { unsafe { &mut *core::ptr::addr_of_mut!(INPUT) } }

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[view.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn fail(window: &mut Window, why: &[u8]) {
    sex_pdx::serial_record!("[view.failed] {}", core::str::from_utf8(why).unwrap_or("?"));
    let mut text = [0u8; 160];
    let head = b"View. F11 closes.\n\n";
    let n = (head.len() + why.len()).min(text.len());
    text[..head.len()].copy_from_slice(head);
    text[head.len()..n].copy_from_slice(&why[..n - head.len()]);
    window.show(&text[..n]);
}

/// Up to 3200 by 2400 decodes on the heap and is shrunk; the heap, not this
/// limit, is what usually refuses a big photo.
const SHRINKABLE: Limits = Limits { max_width: 4 * MAX_W, max_height: 4 * MAX_H, max_pixels: 16 * PIXELS, max_input_bytes: usize::MAX };

/// Decodes `bytes` into the window's pixels: straight into them when the picture
/// fits the window's pixel area, otherwise on the heap and then shrunk to fit it.
/// Returns width, height, shrunk.
fn open(bytes: &[u8], out: &mut [u32], (aw, ah): (usize, usize)) -> Result<(usize, usize, bool), Error> {
    let (aw, ah) = (aw.max(1), ah.max(1));
    let surface = Limits { max_width: aw, max_height: ah, max_pixels: aw * ah, max_input_bytes: usize::MAX };
    match sex_image::decode_into(bytes, &surface, out) {
        Err(Error::TooLarge) => {}
        other => return other.map(|(w, h)| (w, h, false)),
    }
    let image = sex_image::decode(bytes, &SHRINKABLE)?.fit_within(aw, ah)?;
    out[..image.pixels.len()].copy_from_slice(&image.pixels);
    Ok((image.width, image.height, true))
}

/// Lays transparent parts over the window's own colour.
fn flatten(pixels: &mut [u32]) {
    for p in pixels.iter_mut() {
        let a = *p >> 24;
        if a != 255 {
            let mix = |s: u32| ((((*p >> s) & 0xFF) * a + ((BACK >> s) & 0xFF) * (255 - a)) / 255) << s;
            *p = mix(16) | mix(8) | mix(0);
        }
        *p &= 0x00FF_FFFF;
    }
}

/// How the person is looking at the picture: turned, zoomed, and where when zoomed.
#[derive(Clone, Copy, Default)]
struct Look {
    /// Quarter turns clockwise.
    turns: u8,
    /// Percent; `None` fits the window (shrinking only), as a picture first opens.
    zoom: Option<u32>,
    /// The scaled picture's pixel at the window's top left.
    pan: (isize, isize),
}

impl Look {
    fn plain(&self) -> bool { self.turns % 4 == 0 && self.zoom.is_none() }
}

/// Shows `src` (`w` by `h`) turned and zoomed as `look` says, in the window. `scratch`
/// holds the turned copy, kept between calls so an animation reuses it. Returns the
/// size shown, or `None` when the heap could not hold a copy.
fn show(window: &mut Window, src: &[u32], (w, h): (usize, usize), look: &mut Look, scratch: &mut Vec<u32>) -> Option<(usize, usize)> {
    let (aw, ah) = window.size();
    let (aw, ah) = (aw.max(1), ah.max(1));
    let out = window.pixels();
    let (src, (w, h)) = if look.turns % 4 != 0 {
        if scratch.len() != w * h {
            scratch.clear();
            scratch.try_reserve_exact(w * h).ok()?;
            scratch.resize(w * h, 0);
        }
        let size = look::rotate_into(src, (w, h), look.turns, scratch);
        (&scratch[..], size)
    } else {
        (src, (w, h))
    };
    let (ow, oh) = match look.zoom {
        None if w <= aw && h <= ah => { out[..w * h].copy_from_slice(src); (w, h) }
        None => {
            let mut pixels = Vec::new();
            pixels.try_reserve_exact(src.len()).ok()?;
            pixels.extend_from_slice(src);
            let image = (Image { width: w, height: h, pixels }).fit_within(aw, ah).ok()?;
            out[..image.pixels.len()].copy_from_slice(&image.pixels);
            (image.width, image.height)
        }
        Some(pct) => {
            let pan = look::clamp_pan(look.pan, look::scaled((w, h), pct), (aw, ah));
            look.pan = (pan.0 as isize, pan.1 as isize);
            look::view_into(src, (w, h), pct, pan, out, (aw, ah))
        }
    };
    flatten(&mut out[..ow * oh]);
    window.show_pixels(ow, oh);
    Some((ow, oh))
}

/// A moving picture: its frames, the canvas they draw on, when the next is due.
struct Playing {
    animation: Animation<&'static [u8]>,
    canvas: Vec<u32>,
    due: u64,
    shown: u64,
}

impl Playing {
    /// Some when the file is an animation with more than one frame.
    fn start(bytes: &'static [u8]) -> Option<Self> {
        let animation = Animation::open(bytes, &SHRINKABLE).ok().filter(|a| a.frames() > 1)?;
        let count = animation.width * animation.height;
        let mut canvas = Vec::new();
        canvas.try_reserve_exact(count).ok()?;
        canvas.resize(count, 0);
        sex_pdx::serial_record!("[view.animated] frames={} loops={}", animation.frames(), animation.loops().map_or(-1, |n| n as i64));
        Some(Playing { animation, canvas, due: sex_pdx::get_ticks(), shown: 0 })
    }

    /// Shows the next frame when it is due. False once the animation is over.
    fn tick(&mut self, window: &mut Window, look: &mut Look, scratch: &mut Vec<u32>) -> bool {
        let now = sex_pdx::get_ticks();
        if now < self.due {
            return true;
        }
        let frames = self.animation.frames() as u64;
        if let Some(loops) = self.animation.loops() {
            if self.shown >= frames * (loops as u64 + 1) {
                return false;
            }
        }
        let Ok(delay) = self.animation.next_into(&mut self.canvas) else { return false };
        if show(window, &self.canvas, (self.animation.width, self.animation.height), look, scratch).is_none() {
            return false;
        }
        self.shown += 1;
        if self.shown <= frames + 1 {
            sex_pdx::serial_record!("[view.frame] n={} delay={}", self.shown, delay);
        }
        // Browsers hold each frame at least 20 ms.
        self.due = now + (delay as u64).max(20);
        true
    }

    fn wait(&self) -> u64 { self.due.saturating_sub(sex_pdx::get_ticks()).clamp(1, 16) }
}

/// Shows the picture at the window's current size. False if it could not be.
fn present(window: &mut Window, bytes: &[u8]) -> bool {
    let area = window.size();
    let out = window.pixels();
    match open(bytes, out, area) {
        Ok((w, h, scaled)) => {
            flatten(&mut out[..w * h]);
            window.show_pixels(w, h);
            sex_pdx::serial_record!("[view.shown] w={} h={} bytes={} scaled={} fit={}x{}", w, h, bytes.len(), scaled as u8, area.0, area.1);
            true
        }
        Err(Error::UnknownFormat) => { fail(window, b"That file is not a PNG, JPEG, WebP or GIF picture."); false }
        Err(Error::TooLarge) => { fail(window, b"That picture is too large to open."); false }
        Err(Error::OutOfMemory) => { fail(window, b"That picture needs more memory than View has."); false }
        Err(_) => { fail(window, b"That picture is damaged or uses an unsupported feature."); false }
    }
}

/// The pictures of one folder, in name order, and which is shown.
struct Slides { folder: Vec<u8>, names: Vec<Vec<u8>>, index: usize }

impl Slides {
    fn path(&self, i: usize) -> Vec<u8> {
        let mut p = self.folder.clone();
        p.push(b'/');
        p.extend_from_slice(&self.names[i]);
        p
    }
    /// The folder's pictures, or `None` when it cannot be listed (not granted) or has none.
    fn list(window: &mut Window, folder: &[u8]) -> Option<Slides> {
        let mut names: Vec<Vec<u8>> = files::list(window, folder).ok()?.into_iter()
            .filter(|e| !e.dir && look::is_picture_name(&e.name)).map(|e| e.name).collect();
        if names.is_empty() { return None; }
        names.sort();
        Some(Slides { folder: folder.to_vec(), names, index: 0 })
    }
}

/// Everything View is showing.
struct State {
    /// Bytes of the current file in [`input`]; 0 when nothing is shown.
    shown: usize,
    playing: Option<Playing>,
    look: Look,
    /// The still picture decoded whole, once it is turned or zoomed.
    still: Option<Image>,
    scratch: Vec<u32>,
    slides: Option<Slides>,
    /// The slideshow: when the next picture is due, if it is running.
    auto: Option<u64>,
    drag: Option<(i16, i16)>,
}

impl State {
    fn bytes(&self) -> &'static [u8] { &input()[..self.shown] }

    /// Reads and shows a file, fresh: no turn, fitted.
    fn load(&mut self, window: &mut Window, path: &[u8]) {
        // The animation borrows the buffer the file is about to be read into.
        self.playing = None;
        self.still = None;
        self.look = Look::default();
        self.shown = 0;
        let buffer = input();
        match files::read(window, path, buffer) {
            Err(e) => fail(window, files::message(e)),
            Ok(len) if len == buffer.len() => fail(window, b"That file is larger than 4 MiB."),
            Ok(len) => if present(window, &buffer[..len]) { self.shown = len },
        }
        if self.shown > 0 { self.playing = Playing::start(self.bytes()); }
    }

    fn slide(&mut self, window: &mut Window, step: isize) {
        let Some(s) = self.slides.as_mut() else { return };
        let n = s.names.len() as isize;
        s.index = (s.index as isize + step).rem_euclid(n) as usize;
        let (path, index, name) = (s.path(s.index), s.index, s.names[s.index].clone());
        self.load(window, &path);
        sex_pdx::serial_record!("[view.slide] index={} of={} name={}", index + 1, n, core::str::from_utf8(&name).unwrap_or("?"));
    }

    /// Shows the current picture again after the look or the window changed.
    fn redraw(&mut self, window: &mut Window) {
        if self.shown == 0 { return; }
        // An animation shows the new look with its next frame.
        if self.playing.is_some() { if let Some(p) = self.playing.as_mut() { p.due = 0; } return; }
        if self.look.plain() { present(window, self.bytes()); return; }
        if self.still.is_none() {
            match sex_image::decode(self.bytes(), &SHRINKABLE) {
                Ok(image) => self.still = Some(image),
                Err(_) => { sex_pdx::serial_record!("[view.failed] That picture needs more memory than View has."); return; }
            }
        }
        let Some(still) = self.still.as_ref() else { return };
        match show(window, &still.pixels, (still.width, still.height), &mut self.look, &mut self.scratch) {
            Some((w, h)) => sex_pdx::serial_record!("[view.drawn] w={} h={} deg={} pct={}", w, h, self.look.turns as u32 * 90,
                self.look.zoom.unwrap_or(0)),
            None => sex_pdx::serial_record!("[view.failed] That picture needs more memory than View has."),
        }
    }

    /// The size of the picture as turned, for zoom steps from "fit".
    fn turned_size(&self) -> (usize, usize) {
        let (w, h) = match (&self.playing, &self.still) {
            (Some(p), _) => (p.animation.width, p.animation.height),
            (None, Some(s)) => (s.width, s.height),
            _ => sex_image::dimensions(self.bytes()).map(|(_, w, h)| (w, h)).unwrap_or((1, 1)),
        };
        if self.look.turns % 2 == 1 { (h, w) } else { (w, h) }
    }

    fn zoom(&mut self, window: &mut Window, pct: Option<u32>) {
        self.look.zoom = pct;
        self.look.pan = (0, 0);
        sex_pdx::serial_record!("[view.zoom] pct={}", pct.unwrap_or(0));
        self.redraw(window);
    }

    fn pan(&mut self, window: &mut Window, dx: isize, dy: isize) {
        if self.look.zoom.is_none() { return; }
        self.look.pan = (self.look.pan.0 + dx, self.look.pan.1 + dy);
        self.redraw(window);
    }

    /// A key; returns nothing, as each action draws what it changed.
    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let (aw, ah) = window.size();
        let zoomed = self.look.zoom.is_some();
        match (sex_app::nav(key), sex_app::typed(key)) {
            (_, Some(b'r' | b'R')) if self.shown > 0 => {
                self.look.turns = (self.look.turns + if key.shift { 3 } else { 1 }) % 4;
                self.look.pan = (0, 0);
                sex_pdx::serial_record!("[view.rotated] deg={}", self.look.turns as u32 * 90);
                self.redraw(window);
            }
            (_, Some(b'+' | b'=')) if self.shown > 0 => {
                let now = self.look.zoom.unwrap_or_else(|| look::fit_percent(self.turned_size(), (aw, ah)));
                self.zoom(window, Some(look::zoom_in(now)));
            }
            (_, Some(b'-')) if self.shown > 0 => {
                let now = self.look.zoom.unwrap_or_else(|| look::fit_percent(self.turned_size(), (aw, ah)));
                self.zoom(window, Some(look::zoom_out(now)));
            }
            (_, Some(b'0')) if self.shown > 0 => self.zoom(window, None),
            (_, Some(b'1')) if self.shown > 0 => self.zoom(window, Some(100)),
            (Some(sex_app::Nav::Left), _) if zoomed => self.pan(window, -(aw as isize / 4), 0),
            (Some(sex_app::Nav::Right), _) if zoomed => self.pan(window, aw as isize / 4, 0),
            (Some(sex_app::Nav::Up), _) if zoomed => self.pan(window, 0, -(ah as isize / 4)),
            (Some(sex_app::Nav::Down), _) if zoomed => self.pan(window, 0, ah as isize / 4),
            (Some(sex_app::Nav::Right), _) | (_, Some(b'n')) => self.slide(window, 1),
            (Some(sex_app::Nav::Left), _) | (_, Some(b'p')) => self.slide(window, -1),
            (_, Some(b' ')) if self.slides.is_some() => {
                self.auto = match self.auto { Some(_) => None, None => Some(sex_pdx::get_ticks() + SLIDE_MS) };
                sex_pdx::serial_record!("[view.slideshow] on={} ms={}", self.auto.is_some() as u8, SLIDE_MS);
            }
            _ => {}
        }
    }

    /// Dragging with the left button moves a zoomed picture under the pointer.
    fn pointer(&mut self, window: &mut Window, p: sex_pdx::app_pointer::Pointer) {
        match p.kind {
            Kind::Down { button: 0 } => self.drag = Some((p.x, p.y)),
            Kind::Move if p.buttons & 1 != 0 => if let Some((x, y)) = self.drag {
                self.drag = Some((p.x, p.y));
                self.pan(window, x as isize - p.x as isize, y as isize - p.y as isize);
            },
            Kind::Up { button: 0 } | Kind::Leave => self.drag = None,
            Kind::Wheel { steps } if self.look.zoom.is_some() => self.pan(window, 0, -(steps as isize) * 48),
            _ => {}
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"View");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let file = given.split(|b| *b == b' ').find(|w| !w.is_empty()).map(|f| f.to_vec());
    let mut state = State { shown: 0, playing: None, look: Look::default(), still: None, scratch: Vec::new(), slides: None, auto: None, drag: None };
    match file {
        None => fail(&mut window, b"Usage: run view @PICTURE.png, .jpg, .webp or .gif, or run view @FOLDER"),
        Some(file) => {
            let folder = files::stat(&mut window, &file).is_ok_and(|s| s.dir);
            if folder {
                match Slides::list(&mut window, &file) {
                    Some(slides) => { state.slides = Some(slides); state.slide(&mut window, 0); }
                    None => fail(&mut window, b"That folder has no PNG, JPEG, WebP or GIF pictures."),
                }
            } else {
                state.load(&mut window, &file);
                // Its folder too, when this run may read it: N and P step through it.
                let cut = file.iter().rposition(|b| *b == b'/').unwrap_or(0);
                let name = &file[cut + 1..];
                if state.shown > 0 && cut > 0 {
                    if let Some(mut slides) = Slides::list(&mut window, &file[..cut]) {
                        slides.index = slides.names.iter().position(|n| n == name).unwrap_or(0);
                        sex_pdx::serial_record!("[view.slide] index={} of={} name={}", slides.index + 1, slides.names.len(), core::str::from_utf8(name).unwrap_or("?"));
                        state.slides = Some(slides);
                    }
                }
            }
        }
    }
    sex_pdx::serial_record!("[view.ready] target={} task={}", window.target, window.task);
    loop {
        let now = sex_pdx::get_ticks();
        if state.auto.is_some_and(|due| now >= due) {
            state.slide(&mut window, 1);
            state.auto = Some(sex_pdx::get_ticks() + SLIDE_MS);
            continue;
        }
        let until_slide = state.auto.map(|due| due.saturating_sub(now));
        let event = if let Some(p) = state.playing.as_mut() {
            if !p.tick(&mut window, &mut state.look, &mut state.scratch) {
                state.playing = None;
                continue;
            }
            match window.poll() {
                Some(event) => event,
                None => {
                    let wait = until_slide.map_or(p.wait(), |s| p.wait().min(s.max(1)));
                    let _ = sex_pdx::sys_wait_self_ticks(wait);
                    continue;
                }
            }
        } else if let Some(ms) = until_slide {
            match window.next_within(ms.max(1)) { Some(event) => event, None => continue }
        } else {
            window.next()
        };
        match event {
            Event::Close => { sex_pdx::serial_record!("[view.closed] target={} task={}", window.target, window.task); window.close() }
            // A new window size: fit the picture to it again (an animation's next frame does).
            Event::Resize { .. } if state.shown > 0 && state.playing.is_none() => state.redraw(&mut window),
            Event::Key(key) => state.key(&mut window, key),
            Event::Pointer(p) => state.pointer(&mut window, p),
            _ => {}
        }
    }
}
