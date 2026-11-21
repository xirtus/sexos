// SPDX-License-Identifier: GPL-3.0-only
//! A line graph of one history: newest sample at the right edge, `step` pixels apart,
//! scaled so `top` reaches the graph's top. Area under the line is shaded.
use crate::canvas::Canvas;
use crate::history::History;

#[derive(Clone, Copy)]
pub struct Style { pub back: u32, pub grid: u32, pub line: u32, pub fill: u32 }

/// The rectangle a graph fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area { pub x: i32, pub y: i32, pub w: i32, pub h: i32 }

/// The row for `value` inside `area` (its top row when `value >= top`).
pub fn row_of(area: Area, value: u32, top: u32) -> i32 {
    let inner = (area.h - 3).max(0) as i64;
    let v = (value as u64).min(top.max(1) as u64) as i64;
    area.y + 1 + (inner - v * inner / top.max(1) as i64) as i32
}

/// Draws the frame, quarter lines and the history. Returns how many samples showed.
pub fn plot<const N: usize>(c: &mut Canvas, area: Area, history: &History<N>, top: u32, step: i32, style: Style) -> usize {
    let Area { x, y, w, h } = area;
    if w < 3 || h < 3 { return 0; }
    c.fill(x, y, w, h, style.back);
    for q in 1..4 { c.fill(x + 1, y + h * q / 4, w - 2, 1, style.grid); }
    c.frame(x, y, w, h, style.grid);
    let step = step.max(1);
    let right = x + w - 2;
    let fits = ((w - 2 - 1) / step + 1).max(0) as usize;
    let shown = history.len().min(fits);
    let first = history.len() - shown;
    let bottom = y + h - 2;
    let point = |i: usize| (right - (shown - 1 - i) as i32 * step, row_of(area, history.get(first + i).unwrap_or(0), top));
    // Shade under the line column by column, then draw the line over it.
    for i in 0..shown {
        let (x1, y1) = point(i);
        let (x0, y0) = if i == 0 { (x1, y1) } else { point(i - 1) };
        for cx in x0..=x1 {
            let cy = if x1 == x0 { y1 } else { y0 + (y1 - y0) * (cx - x0) / (x1 - x0) };
            c.fill(cx, cy + 1, 1, bottom - cy, style.fill);
        }
    }
    for i in 0..shown {
        let (x1, y1) = point(i);
        let (x0, y0) = if i == 0 { (x1, y1) } else { point(i - 1) };
        c.line(x0, y0, x1, y1, style.line);
    }
    shown
}

#[cfg(test)]
mod tests {
    use super::*;
    const S: Style = Style { back: 1, grid: 2, line: 3, fill: 4 };

    #[test]
    fn rows_scale_and_clamp() {
        let a = Area { x: 0, y: 10, w: 20, h: 103 };
        assert_eq!(row_of(a, 0, 100), 111);
        assert_eq!(row_of(a, 100, 100), 11);
        assert_eq!(row_of(a, 50, 100), 61);
        assert_eq!(row_of(a, 500, 100), 11);
        assert_eq!(row_of(a, 3, 0), 11);
    }

    #[test]
    fn newest_sample_sits_at_the_right_edge() {
        let mut px = vec![0u32; 40 * 23];
        let mut c = Canvas::new(&mut px, 40, 23).unwrap();
        let area = Area { x: 0, y: 0, w: 40, h: 23 };
        let mut h: History<8> = History::new();
        assert_eq!(plot(&mut c, area, &h, 100, 4, S), 0);
        assert_eq!((c.pixel(20, 7), c.pixel(20, 5)), (Some(1), Some(2)));
        assert_eq!(c.pixel(0, 0), Some(2));
        for v in [0, 0, 100, 50] { h.push(v); }
        assert_eq!(plot(&mut c, area, &h, 100, 4, S), 4);
        // Newest (50) at x=38, half way; the one before (100) at x=34, the top row.
        assert_eq!(c.pixel(38, row_of(area, 50, 100)), Some(3));
        assert_eq!(c.pixel(34, row_of(area, 100, 100)), Some(3));
        // Under the line is shaded, above it is not.
        assert_eq!(c.pixel(38, row_of(area, 50, 100) + 3), Some(4));
        assert_eq!(c.pixel(38, row_of(area, 50, 100) - 3), Some(1));
        // Left of the oldest sample stays background.
        assert_eq!(c.pixel(10, 20), Some(1));
    }

    #[test]
    fn only_what_fits_is_drawn() {
        let mut px = vec![0u32; 12 * 10];
        let mut c = Canvas::new(&mut px, 12, 10).unwrap();
        let mut h: History<64> = History::new();
        for v in 0..64 { h.push(v); }
        // Ten inner columns, one sample each: the ten newest.
        assert_eq!(plot(&mut c, Area { x: 0, y: 0, w: 12, h: 10 }, &h, 64, 1, S), 10);
        assert_eq!(plot(&mut c, Area { x: 0, y: 0, w: 2, h: 10 }, &h, 64, 1, S), 0);
    }
}
