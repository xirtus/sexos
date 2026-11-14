// SPDX-License-Identifier: GPL-3.0-only
//! How large a page is drawn: pure arithmetic, tested on the host.
//!
//! A scale is pixels per PDF point (1/72 inch), so 1.0 is "100 %": a US Letter page is
//! 612 pixels wide. Fit modes follow the window as it is resized; a fixed scale stays.

/// What the person asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// The page's width fills the area (the default, as before).
    FitWidth,
    /// The whole page is visible.
    FitPage,
    /// A fixed number of pixels per point.
    Scale(f32),
}

/// The steps + and - walk, in percent: the ones most readers offer.
const LADDER: [u32; 16] = [25, 33, 50, 67, 75, 90, 100, 110, 125, 150, 175, 200, 250, 300, 400, 500];
const MIN: f32 = 0.10;
const MAX: f32 = 8.0;

/// Pixels per point for a `pw` by `ph` point page in an `aw` by `ah` pixel area.
pub fn scale(zoom: Zoom, (pw, ph): (f32, f32), (aw, ah): (usize, usize)) -> f32 {
    let (pw, ph) = (pw.max(1.0), ph.max(1.0));
    let width = aw.max(1) as f32 / pw;
    let s = match zoom {
        Zoom::FitWidth => width,
        Zoom::FitPage => width.min(ah.max(1) as f32 / ph),
        Zoom::Scale(s) => s,
    };
    s.clamp(MIN, MAX)
}

/// The next step up from the scale now shown, as a fixed scale.
pub fn zoom_in(now: f32) -> Zoom {
    let pct = now * 100.0;
    Zoom::Scale(LADDER.iter().map(|p| *p as f32).find(|p| *p > pct * 1.01).unwrap_or(LADDER[LADDER.len() - 1] as f32) / 100.0)
}

/// The next step down from the scale now shown.
pub fn zoom_out(now: f32) -> Zoom {
    let pct = now * 100.0;
    Zoom::Scale(LADDER.iter().rev().map(|p| *p as f32).find(|p| *p < pct * 0.99).unwrap_or(LADDER[0] as f32) / 100.0)
}

/// The scale as the serial record and title give it.
pub fn percent(scale: f32) -> u32 { (scale * 100.0).round() as u32 }

/// How far to scroll so a point `y` points from the page's top sits a quarter of the
/// way down an `ah` pixel area, never past the page's end (`page_px` high, `margin`
/// around it): where a search hit is brought into view.
pub fn scroll_to(y: f32, scale: f32, page_px: usize, ah: usize, margin: usize) -> usize {
    let want = (y * scale) as isize + margin as isize - (ah / 4) as isize;
    let most = (page_px + 2 * margin).saturating_sub(ah) as isize;
    want.clamp(0, most.max(0)) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    const LETTER: (f32, f32) = (612.0, 792.0);

    #[test]
    fn fits_width_and_page() {
        assert!((scale(Zoom::FitWidth, LETTER, (1224, 400)) - 2.0).abs() < 1e-4);
        // A short wide area: the height decides.
        assert!((scale(Zoom::FitPage, LETTER, (1224, 396)) - 0.5).abs() < 1e-4);
        // A tall narrow one: the width does.
        assert!((scale(Zoom::FitPage, LETTER, (306, 4000)) - 0.5).abs() < 1e-4);
        assert_eq!(scale(Zoom::Scale(1.5), LETTER, (10, 10)), 1.5);
        // Never zero or absurd, even for a degenerate area.
        assert_eq!(scale(Zoom::FitWidth, LETTER, (0, 0)), MIN);
        assert_eq!(scale(Zoom::Scale(100.0), LETTER, (10, 10)), MAX);
    }

    #[test]
    fn steps_walk_the_ladder() {
        assert_eq!(zoom_in(1.0), Zoom::Scale(1.1));
        assert_eq!(zoom_out(1.0), Zoom::Scale(0.9));
        // From a fit scale between steps, to the neighbouring steps.
        assert_eq!(zoom_in(0.8), Zoom::Scale(0.9));
        assert_eq!(zoom_out(0.8), Zoom::Scale(0.75));
        assert_eq!(zoom_in(5.0), Zoom::Scale(5.0));
        assert_eq!(zoom_out(0.25), Zoom::Scale(0.25));
        assert_eq!(percent(0.666), 67);
    }

    #[test]
    fn scroll_brings_a_point_into_view() {
        // 1000 px page in a 400 px area with 12 px margins: at most 624 down.
        assert_eq!(scroll_to(0.0, 1.0, 1000, 400, 12), 0);
        assert_eq!(scroll_to(500.0, 1.0, 1000, 400, 12), 412);
        assert_eq!(scroll_to(990.0, 1.0, 1000, 400, 12), 624);
        // A page shorter than the area never scrolls.
        assert_eq!(scroll_to(300.0, 1.0, 300, 400, 12), 0);
    }
}
