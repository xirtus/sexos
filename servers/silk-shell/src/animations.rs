// SPDX-License-Identifier: GPL-3.0-only
//! Desktop look switch and window hit masks (WS6). Display draws the look;
//! the Shell picks it and routes clicks with the same window shape.
//!
//! Super+Shift+A steps through three looks:
//! full (animations, rounded corners, shadows) -> still (no animations) ->
//! plain (no animations, square corners, no shadows) -> full.

use sex_graphics::window_shape as shape;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look { Full, Still, Plain }

static mut LOOK: Look = Look::Full;

pub fn look() -> Look { unsafe { LOOK } }
/// True while windows animate.
pub fn motion() -> bool { look() == Look::Full }
/// `OP_MOTION` arg2: bit 0 = plain look.
pub fn motion_arg2() -> u64 { (look() == Look::Plain) as u64 }

/// Next look; returns it with the notice to show.
pub fn cycle() -> (Look, &'static [u8]) {
    let next = match look() { Look::Full => Look::Still, Look::Still => Look::Plain, Look::Plain => Look::Full };
    unsafe { LOOK = next; }
    (next, match next {
        Look::Full => b"Animations and effects on",
        Look::Still => b"Animations off",
        Look::Plain => b"Plain look: no animations, square corners, no shadows",
    })
}

/// Hit mask: false only when (`x`, `y`) lands in a see-through rounded corner
/// of a window at `bounds` (x, y, w, h); clicks there go to what is drawn.
pub fn in_shape(x: i32, y: i32, bounds: Option<(i32, i32, u32, u32)>, scale: u32) -> bool {
    if look() == Look::Plain { return true; }
    let Some((bx, by, bw, bh)) = bounds else { return true };
    let (lx, ly) = (x - bx, y - by);
    if lx < 0 || ly < 0 || lx >= bw as i32 || ly >= bh as i32 { return true; }
    shape::contains(lx, ly, bw, bh, shape::radius(scale, bw, bh))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cycle_and_masks() {
        unsafe { LOOK = Look::Full; }
        let b = Some((100, 100, 300, 200));
        assert!(!in_shape(100, 100, b, 1));
        assert!(in_shape(105, 105, b, 1));
        assert!(in_shape(250, 100, b, 1));
        assert!(!in_shape(399, 299, b, 1));
        assert_eq!(cycle().0, Look::Still);
        assert!(!motion() && motion_arg2() == 0);
        assert_eq!(cycle().0, Look::Plain);
        assert_eq!(motion_arg2(), 1);
        assert!(in_shape(100, 100, b, 1));
        assert_eq!(cycle().0, Look::Full);
        assert!(motion());
    }
}
