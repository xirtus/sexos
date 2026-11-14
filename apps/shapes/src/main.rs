// SPDX-License-Identifier: GPL-3.0-only
//! Shapes: anti-aliased drawing in a program's own window. `run shapes`. It draws
//! sex-canvas's demo scene (tiny-skia, on the program's heap) into the window's pixel
//! surface. The arrow keys move the front window and redraw it, shadow and all.
#![no_std]
#![no_main]

use sex_app::{nav, Event, Nav, Window};
use sex_canvas::{demo, Canvas};

const W: usize = 400;
const H: usize = 320;
const STEP: f32 = 12.0;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[shapes.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn draw(window: &mut Window, dx: f32, dy: f32) {
    let start = sex_pdx::get_ticks();
    let pixels = &mut window.pixels()[..W * H];
    let mut canvas = Canvas::new(pixels, W as u32, H as u32).unwrap_or_else(|| sex_pdx::task_finish(1));
    demo(&mut canvas, dx, dy);
    window.show_pixels(W, H);
    sex_pdx::serial_record!("[shapes.shown] w={} h={} dx={} dy={} ticks={}", W, H, dx as i32, dy as i32, sex_pdx::get_ticks() - start);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Shapes");
    let (mut dx, mut dy) = (0.0f32, 0.0f32);
    draw(&mut window, dx, dy);
    sex_pdx::serial_record!("[shapes.ready] target={} task={}", window.target, window.task);
    loop {
        match window.next() {
            Event::Close => {
                sex_pdx::serial_record!("[shapes.closed] target={} task={}", window.target, window.task);
                window.close()
            }
            Event::Resize { .. } | Event::Pointer(_) | Event::Drop { .. } | Event::DragEnded { .. } => {}
            Event::Key(key) => {
                let (x, y) = match nav(key) {
                    Some(Nav::Left) => (-STEP, 0.0),
                    Some(Nav::Right) => (STEP, 0.0),
                    Some(Nav::Up) => (0.0, -STEP),
                    Some(Nav::Down) => (0.0, STEP),
                    Some(Nav::Home) => (-dx, -dy),
                    _ => continue,
                };
                dx = (dx + x).clamp(-(W as f32) * 0.36, W as f32 * 0.1);
                dy = (dy + y).clamp(-(H as f32) * 0.28, H as f32 * 0.1);
                draw(&mut window, dx, dy);
            }
        }
    }
}
