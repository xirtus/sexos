// SPDX-License-Identifier: GPL-3.0-only
//! View: shows a picture. `run view @photo.png`. The decoder runs here, inside the
//! program's own confinement, not in Display: a hostile file can only crash this window.
#![no_std]
#![no_main]

use sex_app::{files, Event, Window};
use sex_pdx::pixel_region::{MAX_H, MAX_W, PIXELS};

/// The file as read; a PNG this large holds more than the window can show anyway.
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

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"View");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args);
    let file = given.split(|b| *b == b' ').find(|w| !w.is_empty());
    let input = unsafe { &mut *core::ptr::addr_of_mut!(INPUT) };
    match file {
        None => fail(&mut window, b"Usage: run view @PICTURE.png"),
        Some(file) => match files::read(&mut window, file, input) {
            Err(e) => fail(&mut window, files::message(e)),
            Ok(len) if len == input.len() => fail(&mut window, b"That file is larger than 600 KiB."),
            Ok(len) => {
                // The region holds the picture, then the decoder's inflated scanlines.
                let region = window.pixels();
                let (out, rest) = region.split_at_mut(PIXELS);
                let scratch = unsafe { core::slice::from_raw_parts_mut(rest.as_mut_ptr() as *mut u8, rest.len() * 4) };
                match sex_graphics::png::decode_into(&input[..len], scratch, out, MAX_W, MAX_H) {
                    Ok((w, h)) => {
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
                        sex_pdx::serial_record!("[view.shown] w={} h={} bytes={}", w, h, len);
                    }
                    Err(sex_graphics::png::Error::Size) => fail(&mut window, b"That picture is larger than 800 by 600."),
                    Err(sex_graphics::png::Error::Signature) => fail(&mut window, b"That file is not a PNG picture."),
                    Err(_) => fail(&mut window, b"That picture is damaged or uses an unsupported PNG feature."),
                }
            }
        },
    }
    sex_pdx::serial_record!("[view.ready] target={} task={}", window.target, window.task);
    loop {
        if let Event::Close = window.next() { sex_pdx::serial_record!("[view.closed] target={} task={}", window.target, window.task); window.close() }
    }
}
