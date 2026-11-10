// SPDX-License-Identifier: GPL-3.0-only
//! View: shows a picture. `run view @photo.png`, `.jpg`, `.webp` or `.gif`. The decoders
//! (zune-png, zune-jpeg, image-webp and sex-image's GIF, through sex-image) run here,
//! inside the program's own confinement, not in Display: a hostile file can only crash
//! this window. A picture larger than the window is shrunk to fit, as far as the
//! program's heap allows. Animated GIF and WebP play, with their own timing and loops.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use sex_image::{Animation, Image};

use sex_app::{files, Event, Window};
use sex_image::{Error, Limits};
use sex_pdx::pixel_region::{MAX_H, MAX_W, PIXELS};

/// The file as read.
static mut INPUT: [u8; 4 << 20] = [0; 4 << 20];
const BACK: u32 = 0x000D_1420;

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

/// Shows one frame of an animation, shrunk to the window when it is larger.
fn show_frame(window: &mut Window, canvas: &[u32], (w, h): (usize, usize)) -> bool {
    let (aw, ah) = window.size();
    let out = window.pixels();
    let (w, h) = if w <= aw && h <= ah {
        out[..w * h].copy_from_slice(canvas);
        (w, h)
    } else {
        let mut pixels = Vec::new();
        if pixels.try_reserve_exact(canvas.len()).is_err() { return false; }
        pixels.extend_from_slice(canvas);
        let Ok(image) = (Image { width: w, height: h, pixels }).fit_within(aw.max(1), ah.max(1)) else { return false };
        out[..image.pixels.len()].copy_from_slice(&image.pixels);
        (image.width, image.height)
    };
    flatten(&mut out[..w * h]);
    window.show_pixels(w, h);
    true
}

/// A moving picture: its frames, the canvas they draw on, when the next is due.
struct Playing<'a> {
    animation: Animation<&'a [u8]>,
    canvas: Vec<u32>,
    due: u64,
    shown: u64,
}

impl<'a> Playing<'a> {
    /// Some when the file is an animation with more than one frame.
    fn start(bytes: &'a [u8]) -> Option<Self> {
        let animation = Animation::open(bytes, &SHRINKABLE).ok().filter(|a| a.frames() > 1)?;
        let count = animation.width * animation.height;
        let mut canvas = Vec::new();
        canvas.try_reserve_exact(count).ok()?;
        canvas.resize(count, 0);
        sex_pdx::serial_record!("[view.animated] frames={} loops={}", animation.frames(), animation.loops().map_or(-1, |n| n as i64));
        Some(Playing { animation, canvas, due: sex_pdx::get_ticks(), shown: 0 })
    }

    /// Shows the next frame when it is due. False once the animation is over.
    fn tick(&mut self, window: &mut Window) -> bool {
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
        if !show_frame(window, &self.canvas, (self.animation.width, self.animation.height)) {
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

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"View");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let file = given.split(|b| *b == b' ').find(|w| !w.is_empty());
    let input = unsafe { &mut *core::ptr::addr_of_mut!(INPUT) };
    let mut shown = 0;
    match file {
        None => fail(&mut window, b"Usage: run view @PICTURE.png, .jpg, .webp or .gif"),
        Some(file) => match files::read(&mut window, file, input) {
            Err(e) => fail(&mut window, files::message(e)),
            Ok(len) if len == input.len() => fail(&mut window, b"That file is larger than 4 MiB."),
            Ok(len) => if present(&mut window, &input[..len]) { shown = len },
        },
    }
    let input: &[u8] = input;
    let mut playing = if shown > 0 { Playing::start(&input[..shown]) } else { None };
    sex_pdx::serial_record!("[view.ready] target={} task={}", window.target, window.task);
    loop {
        let event = match playing.as_mut() {
            Some(p) => {
                if !p.tick(&mut window) {
                    playing = None;
                    continue;
                }
                match window.poll() {
                    Some(event) => event,
                    None => { let _ = sex_pdx::sys_wait_self_ticks(p.wait()); continue; }
                }
            }
            None => window.next(),
        };
        match event {
            Event::Close => { sex_pdx::serial_record!("[view.closed] target={} task={}", window.target, window.task); window.close() }
            // A new window size: fit the picture to it again (an animation's next frame does).
            Event::Resize { .. } if shown > 0 && playing.is_none() => { present(&mut window, &input[..shown]); }
            _ => {}
        }
    }
}
