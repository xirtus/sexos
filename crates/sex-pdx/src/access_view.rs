//! Magnifier and colour filters: the Shell chooses, Display draws.
//!
//! Accessibility views that every program gets without knowing about them,
//! the way GNOME's zoom and Windows' colour filters work: Display transforms
//! the finished frame on its way to the screen, after composition and before
//! night light. Screenshots and screen capture keep the plain frame.
//!
//! The Shell sends `OP_ACCESS_VIEW` with the zoom and the filter. The zoom
//! follows the pointer proportionally: the pointer stays where it is on the
//! glass, and moving it to a screen edge shows that edge of the desktop.

use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};

/// Shell -> Display. arg0 = zoom in quarters (4 = 1x .. 32 = 8x), arg1 = filter.
pub const OP_ACCESS_VIEW: u64 = 0x2DD;

/// Zoom is kept in quarter steps.
pub const ZOOM_ONE: u8 = 4;
pub const ZOOM_MAX: u8 = 32;
/// One zoom key press: a quarter below 2x, a half up to 4x, then whole steps.
pub fn zoom_step(zoom: u8, up: bool) -> u8 {
    let step = if zoom < 8 || (zoom == 8 && !up) { 1 } else if zoom < 16 || (zoom == 16 && !up) { 2 } else { 4 };
    if up { zoom.saturating_add(step).min(ZOOM_MAX) } else { zoom.saturating_sub(step).max(ZOOM_ONE) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Filter {
    None = 0,
    /// Stretched contrast and saturation: dim text gets darker, light gets lighter.
    HighContrast = 1,
    /// Inverted lightness keeping hues (Plasma's "invert", macOS "smart invert").
    Inverted = 2,
    Grayscale = 3,
    /// Red/green colour blindness aid (daltonize): shifts lost red/green
    /// difference into lightness and blue.
    RedGreen = 4,
    /// Blue/yellow colour blindness aid.
    BlueYellow = 5,
}
pub const FILTERS: [(&str, Filter); 6] = [
    ("Off", Filter::None),
    ("High contrast", Filter::HighContrast),
    ("Inverted", Filter::Inverted),
    ("Grayscale", Filter::Grayscale),
    ("Red-green aid", Filter::RedGreen),
    ("Blue-yellow aid", Filter::BlueYellow),
];
impl Filter {
    pub fn from_index(index: u64) -> Option<Self> { FILTERS.get(index as usize).map(|f| f.1) }
    pub fn name(self) -> &'static str { FILTERS[self as usize].0 }
}

static ZOOM: AtomicU8 = AtomicU8::new(ZOOM_ONE);
static FILTER: AtomicU8 = AtomicU8::new(0);
/// What the frame on screen was drawn with: zoom, filter, pointer x, y.
static PUBLISHED: AtomicU32 = AtomicU32::new(ZOOM_ONE as u32);
static POINTER: AtomicU32 = AtomicU32::new(0);

/// Takes a new view; false (and no change) when either value is out of range.
pub fn set(zoom: u64, filter: u64) -> bool {
    if !(ZOOM_ONE as u64..=ZOOM_MAX as u64).contains(&zoom) || Filter::from_index(filter).is_none() { return false; }
    ZOOM.store(zoom as u8, Ordering::Relaxed);
    FILTER.store(filter as u8, Ordering::Relaxed);
    true
}
pub fn zoom() -> u8 { ZOOM.load(Ordering::Relaxed) }
pub fn filter() -> Filter { Filter::from_index(FILTER.load(Ordering::Relaxed) as u64).unwrap_or(Filter::None) }
/// True while the output is not a plain copy of the frame.
pub fn active() -> bool { zoom() != ZOOM_ONE || filter() != Filter::None }

/// Display notes where the pointer is each time it draws it.
pub fn set_pointer(x: usize, y: usize) {
    POINTER.store((x.min(0xFFFF) as u32) << 16 | y.min(0xFFFF) as u32, Ordering::Relaxed);
}
pub fn pointer() -> (usize, usize) {
    let p = POINTER.load(Ordering::Relaxed);
    ((p >> 16) as usize, (p & 0xFFFF) as usize)
}

/// True once after anything that moves or recolours every output pixel
/// changed: the publisher then writes the whole frame again instead of only
/// the spans that differ from the previous frame.
pub fn take_change() -> bool {
    let z = zoom();
    // Under zoom the pointer moves the view, so pointer motion is a change too.
    let pointer = if z != ZOOM_ONE { POINTER.load(Ordering::Relaxed) as u64 } else { 0 };
    let now = (z as u32 | (FILTER.load(Ordering::Relaxed) as u32) << 8) ^ (pointer as u32).rotate_left(16);
    PUBLISHED.swap(now, Ordering::Relaxed) != now
}

/// The zoomed view: output pixel (x, y) shows source pixel
/// (`origin + x * step`) in 16.16 fixed point per axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View { pub origin_x: u32, pub origin_y: u32, pub step: u32 }

/// Where the view sits for a `w` x `h` screen with the pointer at `(px, py)`.
/// The pointer's screen position shows the pointer's desktop position, so the
/// point under the pointer never jumps when zoom changes.
pub fn view(zoom: u8, w: usize, h: usize, px: usize, py: usize) -> View {
    let zoom = zoom.clamp(ZOOM_ONE, ZOOM_MAX) as u64;
    let step = ((ZOOM_ONE as u64) << 16) / zoom;
    let axis = |p: usize, len: usize| -> u32 {
        let p = p.min(len.saturating_sub(1)) as u64;
        // origin = p * (1 - 1/z), in 16.16.
        ((p << 16) - p * step) as u32
    };
    View { origin_x: axis(px, w), origin_y: axis(py, h), step: step as u32 }
}
impl View {
    #[inline]
    pub fn source_x(&self, x: usize) -> usize { ((self.origin_x as u64 + x as u64 * self.step as u64) >> 16) as usize }
    #[inline]
    pub fn source_y(&self, y: usize) -> usize { ((self.origin_y as u64 + y as u64 * self.step as u64) >> 16) as usize }
}

#[inline]
fn channels(p: u32) -> (i32, i32, i32) { ((p >> 16 & 0xFF) as i32, (p >> 8 & 0xFF) as i32, (p & 0xFF) as i32) }
#[inline]
fn pack(r: i32, g: i32, b: i32) -> u32 {
    (r.clamp(0, 255) as u32) << 16 | (g.clamp(0, 255) as u32) << 8 | b.clamp(0, 255) as u32
}
/// Rec. 709 luma, 0..=255.
#[inline]
fn luma(r: i32, g: i32, b: i32) -> i32 { (r * 54 + g * 183 + b * 19) >> 8 }

/// One XRGB pixel through `filter`.
#[inline]
pub fn apply(pixel: u32, filter: Filter) -> u32 {
    let (r, g, b) = channels(pixel);
    match filter {
        Filter::None => pixel & 0x00FF_FFFF,
        Filter::HighContrast => {
            // Contrast x2 around mid grey, then saturation x1.5 around luma.
            let s = |c: i32| (c - 128) * 2 + 128;
            let (r, g, b) = (s(r).clamp(0, 255), s(g).clamp(0, 255), s(b).clamp(0, 255));
            let y = luma(r, g, b);
            let t = |c: i32| y + ((c - y) * 3 >> 1);
            pack(t(r), t(g), t(b))
        }
        Filter::Inverted => {
            // Invert lightness, keep hue: invert, then rotate hue 180 degrees
            // by reflecting each channel about the pixel's grey level.
            let (ir, ig, ib) = (255 - r, 255 - g, 255 - b);
            let y = luma(ir, ig, ib);
            pack(2 * y - ir, 2 * y - ig, 2 * y - ib)
        }
        Filter::Grayscale => { let y = luma(r, g, b); pack(y, y, y) }
        Filter::RedGreen => {
            // Simulate deuteranopia (Viénot 1999, LMS-free approximation),
            // then move the lost difference into lightness and blue.
            let sim_r = (r * 160 + g * 96) >> 8;
            let sim_g = (r * 179 + g * 77) >> 8;
            let (er, eg) = (r - sim_r, g - sim_g);
            pack(r, g + (er * 179 >> 8), b + ((er * 179 + eg * 179) >> 8))
        }
        Filter::BlueYellow => {
            // Simulate tritanopia, then move the lost difference into red and green.
            let sim_g = (g * 179 + b * 77) >> 8;
            let sim_b = (g * 120 + b * 136) >> 8;
            let (eg, eb) = (g - sim_g, b - sim_b);
            pack(r + ((eg * 179 + eb * 179) >> 8), g + (eb * 179 >> 8), b)
        }
    }
}

/// Writes one output row `y` of the transformed frame into `out` (len = w).
/// `frame` is the whole composed frame (w * h).
pub fn row(frame: &[u32], w: usize, h: usize, y: usize, view: &View, filter: Filter, out: &mut [u32]) {
    let sy = view.source_y(y).min(h - 1);
    let src = &frame[sy * w..(sy + 1) * w];
    if view.step == 1 << 16 {
        for (o, p) in out.iter_mut().zip(src) { *o = apply(*p, filter); }
        return;
    }
    // Runs of output pixels share one source pixel: transform it once per run.
    let (mut last_sx, mut last) = (usize::MAX, 0u32);
    for (x, o) in out.iter_mut().enumerate().take(w) {
        let sx = view.source_x(x).min(w - 1);
        if sx != last_sx { last_sx = sx; last = apply(src[sx], filter); }
        *o = last;
    }
}
