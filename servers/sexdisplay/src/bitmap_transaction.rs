//! One display-owned Browser bitmap. Partial or rejected transfers never alter
//! published pixels. No allocation, shared pointer, new surface or capability.
use sex_pdx::bitmap_batch::{hash, Applied, Geometry, Request, CAPACITY};
const OWNER: u32 = 15;
const SURFACE: u64 = 300;
const COVERAGE_WORDS: usize = (CAPACITY / 2 + 63) / 64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub old: Option<Geometry>,
    pub new: Option<Geometry>,
}
/// Actual surface origin/extent and body insets, supplied by the compositor.
/// Geometry never moves or enlarges a surface, even when a bitmap is scaled.
#[derive(Clone, Copy)]
pub struct Viewport {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub inset: u32,
    pub top: u32,
}
impl Viewport {
    pub fn local(self, x: usize, y: usize) -> Option<(i64, i64)> {
        let (x, y) = (
            i64::try_from(x).ok()?.checked_sub(self.x as i64)?,
            i64::try_from(y).ok()?.checked_sub(self.y as i64)?,
        );
        (x >= self.inset as i64
            && y >= self.top as i64
            && x < self.width.saturating_sub(self.inset) as i64
            && y < self.height.saturating_sub(self.inset) as i64)
            .then_some((x, y))
    }
    pub fn screen(
        self,
        geometry: Option<Geometry>,
        width: usize,
        height: usize,
    ) -> (usize, usize, usize, usize) {
        let Some(g) = geometry else {
            return (0, 0, 0, 0);
        };
        let left = (g.x as i64).max(self.inset as i64);
        let top = (g.y as i64).max(self.top as i64);
        let right = (g.x as i64 + g.width as i64 * g.scale as i64)
            .min(self.width.saturating_sub(self.inset) as i64);
        let bottom = (g.y as i64 + g.height as i64 * g.scale as i64)
            .min(self.height.saturating_sub(self.inset) as i64);
        if left >= right || top >= bottom {
            return (0, 0, 0, 0);
        }
        let width = width.min(i32::MAX as usize) as i64;
        let height = height.min(i32::MAX as usize) as i64;
        let x = (self.x as i64 + left).clamp(0, width);
        let y = (self.y as i64 + top).clamp(0, height);
        let right = (self.x as i64 + right).clamp(0, width);
        let bottom = (self.y as i64 + bottom).clamp(0, height);
        (
            x as usize,
            y as usize,
            right.saturating_sub(x) as usize,
            bottom.saturating_sub(y) as usize,
        )
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Completed {
    None,
    Bitmap(u64),
    Clear,
}
pub struct Stage {
    sequence: u32,
    geometry: Option<Geometry>,
    published_geometry: Option<Geometry>,
    coverage: [u64; COVERAGE_WORDS],
    pixels: [u32; CAPACITY],
    published: [u32; CAPACITY],
    completed: Completed,
    poisoned: bool,
    reply_pending: bool,
    retired: bool,
}
fn erase(pixels: &mut [u32]) {
    for pixel in pixels {
        unsafe {
            core::ptr::write_volatile(pixel, 0);
        }
    }
}
impl Stage {
    pub const EMPTY: Self = Self {
        sequence: 0,
        geometry: None,
        published_geometry: None,
        coverage: [0; COVERAGE_WORDS],
        pixels: [0; CAPACITY],
        published: [0; CAPACITY],
        completed: Completed::None,
        poisoned: false,
        reply_pending: false,
        retired: false,
    };
    fn erase_staging(&mut self) {
        erase(&mut self.pixels);
        self.coverage.fill(0);
    }
    fn poison(&mut self) {
        self.erase_staging();
        self.poisoned = true;
    }
    /// Live destroy/recreate erases every pixel while keeping the sequence
    /// fence and any historical publication receipt awaiting admission. That
    /// receipt attests a past state publication, never current visible pixels.
    pub fn reset_surface(&mut self, owner: u32, sid: u64) {
        if (owner, sid) != (OWNER, SURFACE) {
            return;
        }
        self.erase_staging();
        erase(&mut self.published);
        self.geometry = None;
        self.published_geometry = None;
        self.poisoned = true;
    }
    pub fn retire(&mut self, owner: u32) {
        if owner != OWNER {
            return;
        }
        self.reset_surface(OWNER, SURFACE);
        self.completed = Completed::None;
        self.reply_pending = false;
        self.retired = true;
    }
    pub fn geometry(&self) -> Option<Geometry> {
        self.published_geometry
    }
    pub fn sequence(&self) -> u32 {
        self.sequence
    }
    /// The caller also attests an active owned surface and its live BOOT
    /// incarnation before dispatch. This finite store independently restricts
    /// its sole binding, and terminal retirement cannot be reset by a message.
    pub fn receive(&mut self, owner: u32, sid: u64, request: Request) -> Option<Change> {
        if self.retired || (owner, sid) != (OWNER, SURFACE) {
            return None;
        }
        match request {
            Request::Begin { sequence, geometry } => {
                if sequence == 0 || !geometry.valid() || sequence < self.sequence {
                    return None;
                }
                if sequence == self.sequence {
                    if self.geometry != Some(geometry) {
                        self.poison();
                    }
                    return None;
                }
                if self.reply_pending {
                    return None;
                }
                self.erase_staging();
                self.sequence = sequence;
                self.geometry = Some(geometry);
                self.completed = Completed::None;
                self.poisoned = false;
            }
            Request::Chunk {
                sequence,
                offset,
                count,
                pixels,
            } => {
                if sequence != self.sequence || self.poisoned || self.completed != Completed::None {
                    return None;
                }
                let Some(g) = self.geometry else {
                    return None;
                };
                let len = g.len();
                if offset >= len
                    || offset % 2 != 0
                    || count != (len - offset).min(2)
                    || pixels[count..].iter().any(|&v| v != 0)
                {
                    self.poison();
                    return None;
                }
                let (word, bit) = (offset / 2 / 64, 1u64 << (offset / 2 % 64));
                if self.coverage[word] & bit != 0
                    && self.pixels[offset..offset + count] != pixels[..count]
                {
                    self.poison();
                    return None;
                }
                self.pixels[offset..offset + count].copy_from_slice(&pixels[..count]);
                self.coverage[word] |= bit;
            }
            Request::Commit {
                sequence,
                hash: expected,
            } => {
                if sequence != self.sequence {
                    return None;
                }
                if self.completed == Completed::Bitmap(expected) {
                    self.reply_pending = true;
                    return None;
                }
                if self.poisoned || self.completed != Completed::None {
                    return None;
                }
                let Some(g) = self.geometry else {
                    return None;
                };
                let chunks = (g.len() + 1) / 2;
                for (index, &bits) in self.coverage.iter().enumerate() {
                    let remaining = chunks.saturating_sub(index * 64);
                    let required = if remaining >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << remaining) - 1
                    };
                    if bits != required {
                        self.poison();
                        return None;
                    }
                }
                if hash(&self.pixels[..g.len()]) != expected {
                    self.poison();
                    return None;
                }
                let old = self.published_geometry;
                self.published.copy_from_slice(&self.pixels);
                self.published_geometry = Some(g);
                self.erase_staging();
                self.completed = Completed::Bitmap(expected);
                self.reply_pending = true;
                return Some(Change { old, new: Some(g) });
            }
            Request::Clear { sequence } => {
                if sequence == 0 || sequence < self.sequence {
                    return None;
                }
                if sequence == self.sequence {
                    if self.completed == Completed::Clear {
                        self.reply_pending = true;
                    }
                    return None;
                }
                if self.reply_pending {
                    return None;
                }
                let old = self.published_geometry;
                self.erase_staging();
                erase(&mut self.published);
                self.sequence = sequence;
                self.geometry = None;
                self.published_geometry = None;
                self.completed = Completed::Clear;
                self.poisoned = false;
                self.reply_pending = true;
                return Some(Change { old, new: None });
            }
        }
        None
    }
    /// One attempt per display turn; every nonzero legacy reply status retains
    /// the same receipt. Publication is never replayed to obtain queue space.
    pub fn poll_reply(&mut self, mut reply: impl FnMut(u32, u64) -> u64) {
        if !self.reply_pending || self.retired {
            return;
        }
        let value = Applied {
            surface: SURFACE as u16,
            sequence: self.sequence,
        }
        .encode()
        .unwrap();
        if reply(OWNER, value) == 0 {
            self.reply_pending = false;
        }
    }
    /// Local coordinates only; the compositor independently clips to the actual
    /// winning surface/body and framebuffer before calling this function.
    pub fn sample(&self, owner: u32, sid: u64, x: i64, y: i64) -> Option<u32> {
        if self.retired || (owner, sid) != (OWNER, SURFACE) {
            return None;
        }
        let g = self.published_geometry?;
        let (x, y) = (x.checked_sub(g.x as i64)?, y.checked_sub(g.y as i64)?);
        if x < 0 || y < 0 {
            return None;
        }
        let (x, y) = (x / g.scale as i64, y / g.scale as i64);
        if x >= g.width as i64 || y >= g.height as i64 {
            return None;
        }
        Some(self.published[y as usize * g.width as usize + x as usize])
    }
    #[cfg(test)]
    pub fn pixels_scrubbed(&self) -> bool {
        self.pixels
            .iter()
            .chain(self.published.iter())
            .all(|&v| v == 0)
            && self.coverage.iter().all(|&v| v == 0)
            && self.geometry.is_none()
            && self.published_geometry.is_none()
    }
    #[cfg(test)]
    pub fn scrubbed(&self) -> bool {
        self.pixels_scrubbed() && !self.reply_pending
    }
}
/// Straight alpha over an opaque compositor pixel. RGB uses integer rounding;
/// transparent pixels preserve the original word exactly, including its X byte.
pub fn blend(rgba: u32, base: u32) -> u32 {
    let alpha = rgba >> 24;
    if alpha == 0 {
        return base;
    }
    if alpha == 255 {
        return rgba;
    }
    let mut out = 0xff00_0000;
    for shift in [0, 8, 16] {
        let fg = (rgba >> shift) & 255;
        let bg = (base >> shift) & 255;
        out |= ((fg * alpha + bg * (255 - alpha) + 127) / 255) << shift;
    }
    out
}
