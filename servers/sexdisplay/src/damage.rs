//! One bounded rectangle per drain. Structural changes still request a full composite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub right: usize,
    pub bottom: usize,
}
#[derive(Clone, Copy, Default)]
pub struct Damage {
    rect: Option<Rect>,
}
impl Damage {
    pub const fn new() -> Self {
        Self { rect: None }
    }
    pub fn add(&mut self, (x, y, w, h): (usize, usize, usize, usize), width: usize, height: usize) {
        let right = x.saturating_add(w).min(width);
        let bottom = y.saturating_add(h).min(height);
        let x = x.min(width);
        let y = y.max(51).min(height);
        if x >= right || y >= bottom {
            return;
        }
        let r = Rect {
            x,
            y,
            right,
            bottom,
        };
        self.rect = Some(match self.rect {
            None => r,
            Some(old) => Rect {
                x: old.x.min(r.x),
                y: old.y.min(r.y),
                right: old.right.max(r.right),
                bottom: old.bottom.max(r.bottom),
            },
        });
    }
    pub fn rect(&self) -> Option<Rect> {
        self.rect
    }
}
