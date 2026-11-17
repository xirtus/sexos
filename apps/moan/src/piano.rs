// SPDX-License-Identifier: GPL-3.0-only
//! Editable note data rendered as document content inside a standard Weft Image.
use alloc::{vec, vec::Vec};
use moan_core::midi::Pattern;
pub const WIDTH: usize = 560;
pub const ROWS: usize = 24;
pub const ROW: usize = 10;
pub const KEYBOARD: usize = 32;
pub const GRID: usize = WIDTH - KEYBOARD;
pub const HEIGHT: usize = ROWS * ROW;
pub const STEPS: usize = 16;
pub const LOW: u8 = 48;
pub const SLIDE_COLOR: u32 = 0xffcc66;
pub fn key(y: usize) -> u8 {
    LOW + (ROWS - 1 - (y / ROW).min(ROWS - 1)) as u8
}
pub fn step(x: usize) -> usize {
    (x.saturating_sub(KEYBOARD).min(GRID - 1) * STEPS / GRID).min(STEPS - 1)
}
fn draw_slide_line(
    pixels: &mut [u32],
    width: usize,
    height: usize,
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    color: u32,
) {
    let mut cx = x0 as isize;
    let mut cy = y0 as isize;
    let tx = x1 as isize;
    let ty = y1 as isize;
    let dx = (tx - cx).abs();
    let dy = -(ty - cy).abs();
    let sx = if cx < tx { 1 } else { -1 };
    let sy = if cy < ty { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if cx >= 0 && (cx as usize) < width && cy >= 0 && (cy as usize) < height {
            let ux = cx as usize;
            let uy = cy as usize;
            pixels[uy * width + ux] = color;
            if uy + 1 < height {
                pixels[(uy + 1) * width + ux] = color;
            }
        }
        if cx == tx && cy == ty {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            cx += sx;
        }
        if e2 <= dx {
            err += dx;
            cy += sy;
        }
    }
}
pub fn render(pattern: Option<&Pattern>) -> Vec<u32> {
    render_width(pattern, WIDTH)
}
pub fn render_width(pattern: Option<&Pattern>, width: usize) -> Vec<u32> {
    let width = width.clamp(160, 4096);
    let keyboard = width * KEYBOARD / WIDTH;
    let grid = width - keyboard;
    let mut pixels = vec![0x121b28; width * HEIGHT];
    for y in 0..HEIGHT {
        let key = key(y);
        let black = [1, 3, 6, 8, 10].contains(&(key % 12));
        for x in 0..width {
            pixels[y * width + x] =
                if y % ROW == 0 || x >= keyboard && (x - keyboard) * STEPS % grid < STEPS {
                    0x39465c
                } else if black {
                    0x182233
                } else {
                    0x212e41
                };
        }
    }
    if let Some(pattern) = pattern {
        for note in &pattern.notes {
            if !(LOW..LOW + ROWS as u8).contains(&note.key) {
                continue;
            }
            let left =
                keyboard + (note.start as u64 * grid as u64 / pattern.frames as u64) as usize;
            let right = keyboard
                + ((note.start + note.frames) as u64 * grid as u64 / pattern.frames as u64)
                    as usize;
            let y = (LOW + ROWS as u8 - 1 - note.key) as usize * ROW;
            for yy in y + 1..y + ROW {
                for x in left..right.min(width) {
                    pixels[yy * width + x] = if x == left || yy == y + 1 {
                        0xaff4cf
                    } else {
                        0x388d74 + (note.velocity as u32 / 4) * 0x000100
                    };
                }
            }
        }
        if pattern.frames > 0 && pattern.notes.len() >= 2 {
            let mut sorted = pattern.notes.clone();
            sorted.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)));
            for i in 1..sorted.len() {
                let b = sorted[i];
                let Some(a) = sorted[..i]
                    .iter()
                    .rev()
                    .find(|a| a.start < b.start && a.start + a.frames > b.start && a.key != b.key)
                else {
                    continue;
                };
                let a_in = (LOW..LOW + ROWS as u8).contains(&a.key);
                let b_in = (LOW..LOW + ROWS as u8).contains(&b.key);
                if !a_in && !b_in {
                    continue;
                }
                let row_y = |k: u8| -> usize {
                    let clamped = k.clamp(LOW, LOW + ROWS as u8 - 1);
                    (LOW + ROWS as u8 - 1 - clamped) as usize * ROW + ROW / 2
                };
                let y0 = row_y(a.key);
                let y1 = row_y(b.key);
                if y0 == y1 {
                    continue;
                }
                let x0 = (keyboard
                    + (b.start as u64 * grid as u64 / pattern.frames as u64) as usize)
                    .clamp(keyboard, width.saturating_sub(1));
                let overlap_end = (a.start + a.frames).min(b.start + b.frames);
                let x1 = (keyboard
                    + (overlap_end as u64 * grid as u64 / pattern.frames as u64) as usize)
                    .max(x0 + 6)
                    .min(width.saturating_sub(1));
                draw_slide_line(&mut pixels, width, HEIGHT, x0, y0, x1, y1, SLIDE_COLOR);
            }
        }
    }
    let mut fonts = weft::text::Fonts::new();
    let mut painter = weft::paint::Painter::new(&mut pixels, width, HEIGHT, 1.0);
    for row in 0..ROWS {
        let key = key(row * ROW);
        let black = [1, 3, 6, 8, 10].contains(&(key % 12));
        painter.fill_rect(
            weft::Rect::new(0.0, (row * ROW) as f32, keyboard as f32, ROW as f32),
            weft::Color(if black { 0xff182233 } else { 0xffb9c4ce }),
        );
        painter.text(
            &mut fonts,
            weft::text::Face::Mono,
            9.0,
            2.0,
            (row * ROW + 8) as f32,
            &note_name(key),
            weft::Color(if black { 0xffc6d1df } else { 0xff182233 }),
        );
    }
    pixels
}
pub fn note_name(key: u8) -> alloc::string::String {
    let name = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ][key as usize % 12];
    alloc::format!("{}{}", name, key as i16 / 12 - 1)
}
