// SPDX-License-Identifier: GPL-3.0-only
//! Timeline pixels are document content in a Weft image, not custom widgets.
//! Clip backgrounds, waveform extrema, fade envelopes, selection and playhead.
use crate::Session;
use alloc::{vec, vec::Vec};
use moan_core::RATE;
pub const WIDTH: usize = 560;
pub const HEADER: usize = 20;
pub const ROW: usize = 56;
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub start: usize,
    pub span: usize,
}
impl Default for View {
    fn default() -> Self {
        Self {
            start: 0,
            span: RATE as usize * 4,
        }
    }
}
impl View {
    pub fn frame(&self, x: usize) -> usize {
        self.start
            .saturating_add(
                ((x.min(WIDTH - 1) as u128 * self.span as u128) / (WIDTH - 1) as u128) as usize,
            )
            .min(crate::project::MAX_TIMELINE)
    }
    pub fn x(&self, frame: usize) -> usize {
        ((frame.saturating_sub(self.start) as u128 * (WIDTH - 1) as u128)
            / self.span.max(1) as u128)
            .min((WIDTH - 1) as u128) as usize
    }
}
fn pixel(pixels: &mut [u32], height: usize, x: usize, y: usize, color: u32) {
    if x < WIDTH && y < height {
        pixels[y * WIDTH + x] = color;
    }
}
fn line(pixels: &mut [u32], height: usize, x: usize, top: usize, bottom: usize, color: u32) {
    for y in top.min(height)..bottom.min(height) {
        pixel(pixels, height, x, y, color);
    }
}
pub fn render(session: &Session, view: View) -> (usize, Vec<u32>) {
    let height = HEADER + ROW * session.tracks.len().min(crate::MAX_TRACKS).max(1);
    let mut pixels = vec![0x111923; WIDTH * height];
    for x in 0..WIDTH {
        let a = view.frame(x);
        let b = view.frame((x + 1).min(WIDTH - 1));
        if a / RATE as usize != b / RATE as usize || x == 0 {
            line(&mut pixels, height, x, 0, height, 0x344254);
        }
    }
    for (index, track) in session.tracks.iter().take(crate::MAX_TRACKS).enumerate() {
        let top = HEADER + index * ROW;
        let mid = top + ROW / 2;
        for x in 0..WIDTH {
            pixel(&mut pixels, height, x, mid, 0x344254);
        }
        for clip in &track.clips {
            if clip.end() <= view.start || clip.start() >= view.start.saturating_add(view.span) {
                continue;
            }
            let left = view.x(clip.start());
            let right = view.x(clip.end()).max(left + 1).min(WIDTH);
            for x in left..right {
                let selected = index == session.selected;
                let color = if selected { 0x22465A } else { 0x213442 };
                line(&mut pixels, height, x, top + 3, top + ROW - 3, color);
                let from = view.frame(x).max(clip.start());
                let to = view
                    .frame((x + 1).min(WIDTH - 1))
                    .max(from + 1)
                    .min(clip.end());
                if from >= to {
                    continue;
                }
                let samples = &clip.source_frames()[clip.source_offset() + from - clip.start()
                    ..clip.source_offset() + to - clip.start()];
                let (mut lo, mut hi) = (0i32, 0i32);
                for frame in samples {
                    lo = lo.min(frame[0] as i32).min(frame[1] as i32);
                    hi = hi.max(frame[0] as i32).max(frame[1] as i32);
                }
                let offset = from - clip.start();
                let (incoming, outgoing) = clip.fades();
                let envelope = |at: usize, len: usize| -> i64 {
                    if len == 0 || at >= len {
                        65536
                    } else if len == 1 {
                        0
                    } else {
                        (at as u128 * 65536 / (len - 1) as u128) as i64
                    }
                };
                let scale = clip.gain() as i64 * envelope(offset, incoming) / 100
                    * envelope(clip.len() - 1 - offset, outgoing)
                    / 65536;
                let amplitude = |value: i32| -> i32 {
                    ((value as i64 * scale / 65536).clamp(-32768, 32767) * (ROW as i64 / 2 - 7)
                        / 32768) as i32
                };
                let y0 = (mid as i32 - amplitude(hi)).max(0) as usize;
                let y1 = (mid as i32 - amplitude(lo)).max(y0 as i32 + 1) as usize;
                line(
                    &mut pixels,
                    height,
                    x,
                    y0,
                    y1,
                    if selected { 0x63D4CD } else { 0x91A9CE },
                );
            }
            line(&mut pixels, height, left, top + 2, top + ROW - 2, 0xB1C6DB);
            if right < WIDTH {
                line(&mut pixels, height, right, top + 2, top + ROW - 2, 0xB1C6DB);
            }
        }
        if session.from < session.to {
            for x in view.x(session.from)..view.x(session.to) {
                if index == session.selected {
                    for y in top..top + ROW {
                        let p = &mut pixels[y * WIDTH + x];
                        *p = ((*p & 0xFEFEFE) >> 1) + 0x263A3C;
                    }
                }
            }
        }
    }
    if session.cursor >= view.start && session.cursor <= view.start.saturating_add(view.span) {
        let x = view.x(session.cursor);
        line(&mut pixels, height, x, 0, height, 0xFFCC66);
    }
    (height, pixels)
}
