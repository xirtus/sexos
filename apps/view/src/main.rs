// SPDX-License-Identifier: GPL-3.0-only
//! View: shows a picture. `run view @photo.png` or `@photo.jpg`. The decoders (zune-png,
//! zune-jpeg, through sex-image) run here, inside the program's own confinement, not in
//! Display: a hostile file can only crash this window. A picture larger than the window
//! is shrunk to fit, as far as the program's heap allows.
#![no_std]
#![no_main]

use sex_app::{files, Event, Window};
use sex_image::{Error, Limits};
use sex_pdx::pixel_region::{MAX_H, MAX_W, PIXELS};

/// The file as read.
static mut INPUT: [u8; 600 * 1024] = [0; 600 * 1024];
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

/// Shows the picture at the window's current size. False if it could not be.
fn present(window: &mut Window, bytes: &[u8]) -> bool {
    let area = window.size();
    let out = window.pixels();
    match open(bytes, out, area) {
        Ok((w, h, scaled)) => {
            // Transparent parts over the window's own colour.
            for p in out[..w * h].iter_mut() {
                let a = *p >> 24;
                if a != 255 {
                    let mix = |s: u32| ((((*p >> s) & 0xFF) * a + ((BACK >> s) & 0xFF) * (255 - a)) / 255) << s;
                    *p = mix(16) | mix(8) | mix(0);
                }
                *p &= 0x00FF_FFFF;
            }
            window.show_pixels(w, h);
            sex_pdx::serial_record!("[view.shown] w={} h={} bytes={} scaled={} fit={}x{}", w, h, bytes.len(), scaled as u8, area.0, area.1);
            true
        }
        Err(Error::UnknownFormat) => { fail(window, b"That file is not a PNG or JPEG picture."); false }
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
        None => fail(&mut window, b"Usage: run view @PICTURE.png or .jpg"),
        Some(file) => match files::read(&mut window, file, input) {
            Err(e) => fail(&mut window, files::message(e)),
            Ok(len) if len == input.len() => fail(&mut window, b"That file is larger than 600 KiB."),
            Ok(len) => if present(&mut window, &input[..len]) { shown = len },
        },
    }
    sex_pdx::serial_record!("[view.ready] target={} task={}", window.target, window.task);
    loop {
        match window.next() {
            Event::Close => { sex_pdx::serial_record!("[view.closed] target={} task={}", window.target, window.task); window.close() }
            // A new window size: fit the picture to it again.
            Event::Resize { .. } if shown > 0 => { present(&mut window, &input[..shown]); }
            _ => {}
        }
    }
}
