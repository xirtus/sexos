// SPDX-License-Identifier: GPL-3.0-only
//! Two bars of eighth-note steps. Same notes/PCM as Piano Roll, not a second song.
use alloc::{vec, vec::Vec};
use moan_core::midi::{step_frame, Pattern};
pub const WIDTH: usize = 1120;
pub const LABEL: usize = 64;
pub const GRID: usize = WIDTH - LABEL;
pub const STEPS: usize = 16;
pub const HEADER: usize = 24;
pub const ROW: usize = 22;
pub const KEYS: [u8; 8] = [72, 67, 64, 60, 48, 46, 42, 36];
pub const HEIGHT: usize = HEADER + KEYS.len() * ROW;
pub fn cell(x: usize, y: usize) -> Option<(usize, u8)> {
    if x < LABEL || x >= WIDTH || y < HEADER || y >= HEIGHT {
        return None;
    }
    Some(((x - LABEL) * STEPS / GRID, KEYS[(y - HEADER) / ROW]))
}
pub fn render(pattern: Option<&Pattern>, play_frame: Option<usize>) -> Vec<u32> {
    render_width(pattern, play_frame, WIDTH)
}
pub fn render_width(
    pattern: Option<&Pattern>,
    play_frame: Option<usize>,
    width: usize,
) -> Vec<u32> {
    let width = width.clamp(160, 4096);
    let label = width * LABEL / WIDTH;
    let grid = width - label;
    let mut pixels = vec![0x121b28; width * HEIGHT];
    let mut fonts = weft::text::Fonts::new();
    let mut painter = weft::paint::Painter::new(&mut pixels, width, HEIGHT, 1.0);
    for row in 0..KEYS.len() {
        let top = HEADER + row * ROW;
        painter.text(
            &mut fonts,
            weft::text::Face::Mono,
            12.0,
            5.0,
            (top + 15) as f32,
            &crate::piano::note_name(KEYS[row]),
            weft::Color::hex(0xd2dce8),
        );
        for step in 0..STEPS {
            let left = label + step * grid / STEPS;
            let right = label + (step + 1) * grid / STEPS;
            let range = pattern.and_then(|p| {
                Some((
                    step_frame(p.bpm, step as u32).ok()?,
                    step_frame(p.bpm, step as u32 + 1).ok()?,
                ))
            });
            let enabled = pattern
                .zip(range)
                .is_some_and(|(p, (start, _))| start < p.frames);
            let note = pattern.zip(range).and_then(|(p, (start, end))| {
                p.notes
                    .iter()
                    .find(|n| n.key == KEYS[row] && n.start >= start && n.start < end)
            });
            let colour = if note.is_some() {
                0x388d74
            } else if !enabled {
                0x20232b
            } else if step % 4 == 0 {
                0x394c64
            } else {
                0x28384d
            };
            painter.fill_rect(
                weft::Rect::new(
                    (left + 2) as f32,
                    (top + 2) as f32,
                    (right - left - 4) as f32,
                    (ROW - 4) as f32,
                ),
                weft::Color::hex(colour),
            );
            if let Some(note) = note {
                let height = 1 + note.velocity as usize * (ROW - 7) / 127;
                painter.fill_rect(
                    weft::Rect::new(
                        (left + 4) as f32,
                        (top + ROW - 3 - height) as f32,
                        3.0,
                        height as f32,
                    ),
                    weft::Color::hex(0xaff4cf),
                );
            }
        }
    }
    for step in 0..STEPS {
        let left = label + step * grid / STEPS;
        let name = alloc::format!("{}.{}.{}", step / 8 + 1, (step % 8) / 2 + 1, step % 2 + 1);
        painter.text(
            &mut fonts,
            weft::text::Face::Mono,
            10.0,
            (left + 2) as f32,
            16.0,
            &name,
            weft::Color::hex(0xa8b9cd),
        );
    }
    if let Some((pattern, position)) = pattern.zip(play_frame) {
        if let Ok(end) = step_frame(pattern.bpm, STEPS as u32) {
            if position < end as usize {
                let x = label + position * grid / end as usize;
                painter.fill_rect(
                    weft::Rect::new(x as f32, 0.0, 2.0, HEIGHT as f32),
                    weft::Color::hex(0xffcc66),
                );
            }
        }
    }
    pixels
}
