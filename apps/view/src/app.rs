// SPDX-License-Identifier: GPL-3.0-only
//! The window, the keys and the pictures (see the crate's description for the keys).
use std::collections::HashMap;
use std::time::Instant;

use gaze_codecs::{self as gaze, Exif, Format, Options};
use sex_image::view as look;
use sex_image::{Animation, Image, Limits};
use sex_std::sex_app::{self, files, Event, Window};
use sex_std::sex_pdx;
use sex_std::sex_pdx::app_pointer::Kind;
use sex_std::sex_pdx::pixel_region::{MAX_H, MAX_W, PIXELS};

use crate::{info, thumb};

const BACK: u32 = 0x000D_1420;
/// How long each picture stays up in a slideshow.
const SLIDE_MS: u64 = 2500;
/// The largest file View reads.
const MAX_FILE: u64 = 64 << 20;

/// The file being shown. Whatever borrows it (an animation) is dropped before the
/// next file replaces it.
static mut INPUT: Vec<u8> = Vec::new();
fn input() -> &'static mut Vec<u8> { unsafe { &mut *core::ptr::addr_of_mut!(INPUT) } }

/// The fonts SVG text is drawn with.
fn fonts() -> [&'static [u8]; 1] { [sex_font::GEIST_SANS] }

fn fail(window: &mut Window, why: &str) {
    sex_pdx::serial_record!("[view.failed] {}", why);
    window.show(format!("View. F11 closes.\n\n{why}").as_bytes());
}

/// What a refusal says to the person.
fn message(e: gaze::Error, bytes: &[u8]) -> String {
    match e {
        gaze::Error::UnknownFormat => "That file is not a picture View knows (PNG, JPEG, WebP, GIF, JPEG XL, SVG or camera RAW).".into(),
        gaze::Error::NoDecoder(f) => format!("That is a {} picture, which View cannot show yet.", f.media_type()),
        gaze::Error::NoPreview => "That camera RAW file has no preview View can show.".into(),
        gaze::Error::TooLarge | gaze::Error::InputTooLarge | gaze::Error::Image(sex_image::Error::TooLarge) => "That picture is too large to open.".into(),
        gaze::Error::OutOfMemory | gaze::Error::Image(sex_image::Error::OutOfMemory) => "That picture needs more memory than View has.".into(),
        _ => format!("That {} picture is damaged or uses an unsupported feature.", gaze::sniff(bytes).map_or("", |f| f.media_type())),
    }
}

/// Up to 3200 by 2400 decodes on the heap and is shrunk; the heap, not this
/// limit, is what usually refuses a big photo.
const SHRINKABLE: Limits = Limits { max_width: 4 * MAX_W, max_height: 4 * MAX_H, max_pixels: 16 * PIXELS, max_input_bytes: usize::MAX };

fn options<'a>(limits: Limits, fonts: &'a [&'a [u8]]) -> Options<'a> {
    Options { limits, fonts, ..Options::default() }
}

/// The whole picture, upright, for turning, zooming and thumbnails. An SVG is drawn to
/// fit `fit` (it has no pixels of its own); everything else at its own size.
fn decode_whole(bytes: &[u8], fit: (usize, usize), limits: Limits) -> Result<Image, gaze::Error> {
    let fonts = fonts();
    let limits = if gaze::sniff(bytes) == Some(Format::Svg) {
        Limits { max_width: fit.0.max(1), max_height: fit.1.max(1), max_pixels: fit.0.max(1) * fit.1.max(1), max_input_bytes: usize::MAX }
    } else { limits };
    gaze::decode_with(bytes, &options(limits, &fonts))
}

/// Whether sex-image alone shows the file as it should look: one of its formats, with no
/// EXIF turn to apply.
fn plain(bytes: &[u8]) -> bool {
    matches!(gaze::sniff(bytes), Some(Format::Png | Format::Jpeg | Format::Webp | Format::Gif))
        && Exif::read(bytes).and_then(|e| e.orientation).is_none_or(|o| o <= 1)
}

/// Decodes `bytes` into the window's pixels: straight into them when the picture is
/// plain and fits the window's pixel area, otherwise on the heap (turned upright) and
/// then shrunk to fit it. Returns width, height, shrunk.
fn open(bytes: &[u8], out: &mut [u32], (aw, ah): (usize, usize)) -> Result<(usize, usize, bool), gaze::Error> {
    let (aw, ah) = (aw.max(1), ah.max(1));
    if plain(bytes) {
        let surface = Limits { max_width: aw, max_height: ah, max_pixels: aw * ah, max_input_bytes: usize::MAX };
        match sex_image::decode_into(bytes, &surface, out) {
            Err(sex_image::Error::TooLarge) => {}
            other => return other.map(|(w, h)| (w, h, false)).map_err(gaze::Error::from),
        }
    }
    let image = decode_whole(bytes, (aw, ah), SHRINKABLE)?;
    let shrunk = image.width > aw || image.height > ah;
    let image = image.fit_within(aw, ah)?;
    out[..image.pixels.len()].copy_from_slice(&image.pixels);
    Ok((image.width, image.height, shrunk))
}

/// Lays transparent parts over the window's own colour.
fn flatten(pixels: &mut [u32]) {
    for p in pixels.iter_mut() {
        let a = *p >> 24;
        if a != 255 {
            let mix = |s: u32| ((((*p >> s) & 0xFF) * a + ((BACK >> s) & 0xFF) * (255 - a)) / 255) << s;
            *p = mix(16) | mix(8) | mix(0);
        }
        *p &= 0x00FF_FFFF;
    }
}

const CW: usize = 9;
const CH: usize = 16;
const ROW: usize = 20;
const INK: u32 = 0x00E8E8EA;
const DIM: u32 = 0x009A9AA0;

/// Text drawn with Geist Mono, each character rasterised once.
struct Letters { font: sex_font::Font<'static>, acc: Vec<i32>, cells: HashMap<char, Vec<u8>> }

impl Letters {
    fn new() -> Option<Self> {
        Some(Self { font: sex_font::Font::new(sex_font::GEIST_MONO)?, acc: vec![0; sex_font::raster::scratch_len(CW, CH)], cells: HashMap::new() })
    }
    fn draw(&mut self, out: &mut [u32], stride: usize, (x, y): (usize, usize), right: usize, s: &str, color: u32) {
        for (i, c) in s.chars().enumerate() {
            let cx = x + i * CW;
            if cx + CW > right { break; }
            let (font, acc) = (&self.font, &mut self.acc);
            let cell = self.cells.entry(c).or_insert_with(|| {
                let mut out = vec![0u8; CW * CH];
                if !sex_font::draw_cell(font, c as u32, CW, CH, acc, &mut out) { out.fill(0); }
                out
            });
            for (row, line) in cell.chunks_exact(CW).enumerate() {
                let at = (y + row) * stride + cx;
                let Some(dst) = out.get_mut(at..at + CW) else { return };
                for (p, a) in dst.iter_mut().zip(line) {
                    if *a != 0 { *p = mix(*p, color, *a as u32); }
                }
            }
        }
    }
}

fn mix(under: u32, over: u32, a: u32) -> u32 {
    let ch = |s: u32| ((((under >> s) & 0xFF) * (255 - a) + ((over >> s) & 0xFF) * a) / 255) << s;
    ch(16) | ch(8) | ch(0)
}

/// The I panel: the current picture's details, drawn over it.
struct Panel { on: bool, lines: Vec<info::Line>, letters: Option<Letters> }

impl Panel {
    fn draw(&mut self, out: &mut [u32], aw: usize, ah: usize) {
        let label = self.lines.iter().map(|(l, _)| l.len()).max().unwrap_or(0) + 2;
        let widest = self.lines.iter().map(|(_, v)| v.chars().count()).max().unwrap_or(0) + label;
        let (x, y) = (8, 8);
        let w = (widest * CW + 16).min(aw.saturating_sub(16));
        let h = (self.lines.len() * ROW + 12).min(ah.saturating_sub(16));
        for row in y..y + h {
            for p in &mut out[row * aw + x..row * aw + x + w] { *p = mix(*p, 0x0008_0A10, 215); }
        }
        let Some(letters) = self.letters.as_mut() else { return };
        for (i, (l, v)) in self.lines.iter().enumerate() {
            let ty = y + 6 + i * ROW + (ROW - CH) / 2;
            if ty + CH > y + h { break; }
            letters.draw(out, aw, (x + 8, ty), x + w, l, DIM);
            letters.draw(out, aw, (x + 8 + label * CW, ty), x + w, v, INK);
        }
    }
}

/// Shows the `w` by `h` picture in the window's pixels; with the panel on, the whole
/// window, the picture at its top left and the panel over it.
fn finish(window: &mut Window, (w, h): (usize, usize), panel: &mut Panel) {
    let (aw, ah) = window.size();
    let out = window.pixels();
    if panel.on && aw >= w && ah >= h && aw * ah <= out.len() {
        // Wider rows: move each row out to its new place, the last first.
        for y in (0..h).rev() {
            out.copy_within(y * w..y * w + w, y * aw);
            out[y * aw + w..(y + 1) * aw].fill(BACK);
        }
        out[h * aw..aw * ah].fill(BACK);
        panel.draw(out, aw, ah);
        window.show_pixels(aw, ah);
    } else {
        window.show_pixels(w, h);
    }
}

/// How the person is looking at the picture: turned, zoomed, and where when zoomed.
#[derive(Clone, Copy, Default)]
struct Look {
    /// Quarter turns clockwise.
    turns: u8,
    /// Percent; `None` fits the window (shrinking only), as a picture first opens.
    zoom: Option<u32>,
    /// The scaled picture's pixel at the window's top left.
    pan: (isize, isize),
}

impl Look {
    fn plain(&self) -> bool { self.turns % 4 == 0 && self.zoom.is_none() }
}

/// Shows `src` (`w` by `h`) turned and zoomed as `look` says, in the window. `scratch`
/// holds the turned copy, kept between calls so an animation reuses it. Returns the
/// size shown, or `None` when the heap could not hold a copy.
fn show(window: &mut Window, src: &[u32], (w, h): (usize, usize), look: &mut Look, scratch: &mut Vec<u32>, panel: &mut Panel) -> Option<(usize, usize)> {
    let (aw, ah) = window.size();
    let (aw, ah) = (aw.max(1), ah.max(1));
    let out = window.pixels();
    let (src, (w, h)) = if look.turns % 4 != 0 {
        if scratch.len() != w * h {
            scratch.clear();
            scratch.try_reserve_exact(w * h).ok()?;
            scratch.resize(w * h, 0);
        }
        let size = look::rotate_into(src, (w, h), look.turns, scratch);
        (&scratch[..], size)
    } else {
        (src, (w, h))
    };
    let (ow, oh) = match look.zoom {
        None if w <= aw && h <= ah => { out[..w * h].copy_from_slice(src); (w, h) }
        None => {
            let mut pixels = Vec::new();
            pixels.try_reserve_exact(src.len()).ok()?;
            pixels.extend_from_slice(src);
            let image = (Image { width: w, height: h, pixels }).fit_within(aw, ah).ok()?;
            out[..image.pixels.len()].copy_from_slice(&image.pixels);
            (image.width, image.height)
        }
        Some(pct) => {
            let pan = look::clamp_pan(look.pan, look::scaled((w, h), pct), (aw, ah));
            look.pan = (pan.0 as isize, pan.1 as isize);
            look::view_into(src, (w, h), pct, pan, out, (aw, ah))
        }
    };
    flatten(&mut out[..ow * oh]);
    finish(window, (ow, oh), panel);
    Some((ow, oh))
}

/// A moving picture: its frames, the canvas they draw on, when the next is due.
struct Playing {
    animation: Animation<&'static [u8]>,
    canvas: Vec<u32>,
    due: u64,
    shown: u64,
}

impl Playing {
    /// Some when the file is an animation with more than one frame.
    fn start(bytes: &'static [u8]) -> Option<Self> {
        let animation = Animation::open(bytes, &SHRINKABLE).ok().filter(|a| a.frames() > 1)?;
        let count = animation.width * animation.height;
        let mut canvas = Vec::new();
        canvas.try_reserve_exact(count).ok()?;
        canvas.resize(count, 0);
        sex_pdx::serial_record!("[view.animated] frames={} loops={}", animation.frames(), animation.loops().map_or(-1, |n| n as i64));
        Some(Playing { animation, canvas, due: sex_pdx::get_ticks(), shown: 0 })
    }

    /// Shows the next frame when it is due. False once the animation is over.
    fn tick(&mut self, window: &mut Window, look: &mut Look, scratch: &mut Vec<u32>, panel: &mut Panel) -> bool {
        let now = sex_pdx::get_ticks();
        if now < self.due {
            return true;
        }
        let frames = self.animation.frames() as u64;
        if let Some(loops) = self.animation.loops() {
            if self.shown >= frames * (loops as u64 + 1) {
                return false;
            }
        }
        let Ok(delay) = self.animation.next_into(&mut self.canvas) else { return false };
        if show(window, &self.canvas, (self.animation.width, self.animation.height), look, scratch, panel).is_none() {
            return false;
        }
        self.shown += 1;
        if self.shown <= frames + 1 {
            sex_pdx::serial_record!("[view.frame] n={} delay={}", self.shown, delay);
        }
        // Browsers hold each frame at least 20 ms.
        self.due = now + (delay as u64).max(20);
        true
    }

    fn wait(&self) -> u64 { self.due.saturating_sub(sex_pdx::get_ticks()).clamp(1, 16) }
}

/// Shows the picture at the window's current size. False if it could not be.
fn present(window: &mut Window, bytes: &[u8], panel: &mut Panel) -> bool {
    let area = window.size();
    let started = Instant::now();
    match open(bytes, window.pixels(), area) {
        Ok((w, h, scaled)) => {
            flatten(&mut window.pixels()[..w * h]);
            finish(window, (w, h), panel);
            sex_pdx::serial_record!("[view.shown] w={} h={} bytes={} scaled={} fit={}x{} format={} ms={}", w, h, bytes.len(), scaled as u8, area.0, area.1,
                gaze::sniff(bytes).map_or("?", |f| f.media_type()), started.elapsed().as_millis());
            true
        }
        Err(e) => { fail(window, &message(e, bytes)); false }
    }
}

/// Picture files View lists in a folder.
fn is_picture_name(name: &[u8]) -> bool {
    let Some(dot) = name.iter().rposition(|b| *b == b'.') else { return false };
    let ext = &name[dot + 1..];
    look::is_picture_name(name)
        || [&b"jxl"[..], b"svg", b"dng", b"cr2", b"nef", b"arw", b"orf", b"rw2", b"pef", b"raf"].iter().any(|e| ext.eq_ignore_ascii_case(e))
}

/// The pictures of one folder, in name order, and which is shown.
struct Slides { folder: Vec<u8>, names: Vec<Vec<u8>>, index: usize }

impl Slides {
    fn path(&self, i: usize) -> Vec<u8> {
        let mut p = self.folder.clone();
        p.push(b'/');
        p.extend_from_slice(&self.names[i]);
        p
    }
    /// The folder's pictures, or `None` when it cannot be listed (not granted) or has none.
    fn list(window: &mut Window, folder: &[u8]) -> Option<Slides> {
        let mut names: Vec<Vec<u8>> = files::list(window, folder).ok()?.into_iter()
            .filter(|e| !e.dir && is_picture_name(&e.name)).map(|e| e.name).collect();
        if names.is_empty() { return None; }
        names.sort();
        Some(Slides { folder: folder.to_vec(), names, index: 0 })
    }
}

/// Everything View is showing.
struct State {
    /// Bytes of the current file in [`input`]; 0 when nothing is shown.
    shown: usize,
    playing: Option<Playing>,
    look: Look,
    /// The still picture decoded whole, once it is turned or zoomed.
    still: Option<Image>,
    scratch: Vec<u32>,
    slides: Option<Slides>,
    /// The slideshow: when the next picture is due, if it is running.
    auto: Option<u64>,
    drag: Option<(i16, i16)>,
    panel: Panel,
}

impl State {
    fn bytes(&self) -> &'static [u8] { &input()[..self.shown] }

    /// Reads and shows a file, fresh: no turn, fitted.
    fn load(&mut self, window: &mut Window, path: &[u8]) {
        // The animation borrows the buffer the file is about to be read into.
        self.playing = None;
        self.still = None;
        self.look = Look::default();
        self.shown = 0;
        self.panel.lines.clear();
        let size = match files::stat(window, path) {
            Ok(st) if st.size > MAX_FILE => { fail(window, "That file is larger than 64 MiB."); return; }
            Ok(st) => st.size as usize,
            Err(e) => { fail(window, &String::from_utf8_lossy(files::message(e))); return; }
        };
        let buffer = input();
        buffer.clear();
        if buffer.try_reserve_exact(size + 1).is_err() { fail(window, "That file needs more memory than View has."); return; }
        buffer.resize(size + 1, 0);
        match files::read(window, path, buffer) {
            Err(e) => fail(window, &String::from_utf8_lossy(files::message(e))),
            Ok(len) => {
                buffer.truncate(len);
                if present(window, buffer, &mut self.panel) { self.shown = len }
            }
        }
        if self.shown > 0 {
            self.playing = Playing::start(self.bytes());
            if self.panel.on { self.details(); if self.playing.is_none() { self.redraw(window); } }
        }
    }

    /// The panel's lines for the picture shown, recorded.
    fn details(&mut self) {
        let bytes = self.bytes();
        let format = gaze::sniff(bytes);
        let size = sex_image::dimensions(bytes).ok().map(|(_, w, h)| (w, h)).or_else(|| self.still.as_ref().map(|s| (s.width, s.height)));
        self.panel.lines = info::lines(Exif::read(bytes).as_ref(), format, size);
        sex_pdx::serial_record!("{}", info::record(&self.panel.lines));
    }

    fn slide(&mut self, window: &mut Window, step: isize) {
        let Some(s) = self.slides.as_mut() else { return };
        let n = s.names.len() as isize;
        s.index = (s.index as isize + step).rem_euclid(n) as usize;
        let (path, index, name) = (s.path(s.index), s.index, s.names[s.index].clone());
        self.load(window, &path);
        sex_pdx::serial_record!("[view.slide] index={} of={} name={}", index + 1, n, String::from_utf8_lossy(&name));
    }

    /// The whole picture, upright, decoded once.
    fn still(&mut self, window: &Window) -> Option<&Image> {
        if self.still.is_none() {
            match decode_whole(self.bytes(), window.size(), SHRINKABLE) {
                Ok(image) => self.still = Some(image),
                Err(_) => { sex_pdx::serial_record!("[view.failed] That picture needs more memory than View has."); return None; }
            }
        }
        self.still.as_ref()
    }

    /// Shows the current picture again after the look or the window changed.
    fn redraw(&mut self, window: &mut Window) {
        if self.shown == 0 { return; }
        // An animation shows the new look with its next frame.
        if let Some(p) = self.playing.as_mut() { p.due = 0; return; }
        if self.look.plain() { present(window, self.bytes(), &mut self.panel); return; }
        if self.still(window).is_none() { return; }
        let Some(still) = self.still.as_ref() else { return };
        match show(window, &still.pixels, (still.width, still.height), &mut self.look, &mut self.scratch, &mut self.panel) {
            Some((w, h)) => sex_pdx::serial_record!("[view.drawn] w={} h={} deg={} pct={}", w, h, self.look.turns as u32 * 90,
                self.look.zoom.unwrap_or(0)),
            None => sex_pdx::serial_record!("[view.failed] That picture needs more memory than View has."),
        }
    }

    /// The size of the picture as turned, for zoom steps from "fit".
    fn turned_size(&mut self, window: &Window) -> (usize, usize) {
        let (w, h) = match &self.playing {
            Some(p) => (p.animation.width, p.animation.height),
            None => self.still(window).map_or((1, 1), |s| (s.width, s.height)),
        };
        if self.look.turns % 2 == 1 { (h, w) } else { (w, h) }
    }

    fn zoom(&mut self, window: &mut Window, pct: Option<u32>) {
        self.look.zoom = pct;
        self.look.pan = (0, 0);
        sex_pdx::serial_record!("[view.zoom] pct={}", pct.unwrap_or(0));
        self.redraw(window);
    }

    fn pan(&mut self, window: &mut Window, dx: isize, dy: isize) {
        if self.look.zoom.is_none() { return; }
        self.look.pan = (self.look.pan.0 + dx, self.look.pan.1 + dy);
        self.redraw(window);
    }

    /// A key; returns nothing, as each action draws what it changed.
    fn key(&mut self, window: &mut Window, key: sex_pdx::app_key::Key) {
        let (aw, ah) = window.size();
        let zoomed = self.look.zoom.is_some();
        match (sex_app::nav(key), sex_app::typed(key)) {
            (_, Some(b'r' | b'R')) if self.shown > 0 => {
                self.look.turns = (self.look.turns + if key.shift { 3 } else { 1 }) % 4;
                self.look.pan = (0, 0);
                sex_pdx::serial_record!("[view.rotated] deg={}", self.look.turns as u32 * 90);
                self.redraw(window);
            }
            (_, Some(b'i' | b'I')) if self.shown > 0 => {
                self.panel.on = !self.panel.on;
                if self.panel.on { self.details(); } else { sex_pdx::serial_record!("[view.info] shown=0"); }
                self.redraw(window);
            }
            (_, Some(b'+' | b'=')) if self.shown > 0 => {
                let now = self.look.zoom.unwrap_or_else(|| look::fit_percent(self.turned_size(window), (aw, ah)));
                self.zoom(window, Some(look::zoom_in(now)));
            }
            (_, Some(b'-')) if self.shown > 0 => {
                let now = self.look.zoom.unwrap_or_else(|| look::fit_percent(self.turned_size(window), (aw, ah)));
                self.zoom(window, Some(look::zoom_out(now)));
            }
            (_, Some(b'0')) if self.shown > 0 => self.zoom(window, None),
            (_, Some(b'1')) if self.shown > 0 => self.zoom(window, Some(100)),
            (Some(sex_app::Nav::Left), _) if zoomed => self.pan(window, -(aw as isize / 4), 0),
            (Some(sex_app::Nav::Right), _) if zoomed => self.pan(window, aw as isize / 4, 0),
            (Some(sex_app::Nav::Up), _) if zoomed => self.pan(window, 0, -(ah as isize / 4)),
            (Some(sex_app::Nav::Down), _) if zoomed => self.pan(window, 0, ah as isize / 4),
            (Some(sex_app::Nav::Right), _) | (_, Some(b'n')) => self.slide(window, 1),
            (Some(sex_app::Nav::Left), _) | (_, Some(b'p')) => self.slide(window, -1),
            (_, Some(b' ')) if self.slides.is_some() => {
                self.auto = match self.auto { Some(_) => None, None => Some(sex_pdx::get_ticks() + SLIDE_MS) };
                sex_pdx::serial_record!("[view.slideshow] on={} ms={}", self.auto.is_some() as u8, SLIDE_MS);
            }
            _ => {}
        }
    }

    /// Dragging with the left button moves a zoomed picture under the pointer.
    fn pointer(&mut self, window: &mut Window, p: sex_pdx::app_pointer::Pointer) {
        match p.kind {
            Kind::Down { button: 0 } => self.drag = Some((p.x, p.y)),
            Kind::Move if p.buttons & 1 != 0 => if let Some((x, y)) = self.drag {
                self.drag = Some((p.x, p.y));
                self.pan(window, x as isize - p.x as isize, y as isize - p.y as isize);
            },
            Kind::Up { button: 0 } | Kind::Leave => self.drag = None,
            Kind::Wheel { steps } if self.look.zoom.is_some() => self.pan(window, 0, -(steps as isize) * 48),
            _ => {}
        }
    }
}

/// `--thumb`: no window; the thumbnail file, whole, or none and a failing status.
fn make_thumb(r: &thumb::Request) -> Result<(usize, usize, usize), String> {
    let started = Instant::now();
    let size = std::fs::metadata(&r.input).map_err(|e| format!("cannot read {}: {e}", r.input))?.len();
    if size > MAX_FILE { return Err("the file is larger than 64 MiB".into()); }
    let bytes = std::fs::read(&r.input).map_err(|e| format!("cannot read {}: {e}", r.input))?;
    // An SVG is drawn at the thumbnail's size (never larger than it says it is).
    let fit = (r.size, r.size);
    // Animations: the first frame, which is what sex-image decodes.
    let image = decode_whole(&bytes, fit, Limits::PHOTO).map_err(|e| message(e, &bytes))?;
    let small = thumb::shrink(image, r.size).map_err(|e| format!("cannot shrink: {e:?}"))?;
    let out = thumb::encode(&small).ok_or("cannot encode")?;
    thumb::write_whole(&r.output, &out).map_err(|e| format!("cannot write {}: {e}", r.output))?;
    let _ = files::direct::sync();
    sex_pdx::serial_record!("[view.thumb] status=0 w={} h={} bytes={} ms={} out={}", small.width, small.height, out.len(), started.elapsed().as_millis(), r.output);
    Ok((small.width, small.height, out.len()))
}

pub fn main() {
    let words: Vec<String> = std::env::args().skip(1).collect();
    if let Some(request) = thumb::request(&words) {
        let status = match request.map_err(String::from).and_then(|r| make_thumb(&r)) {
            Ok(_) => 0,
            Err(why) => { sex_pdx::serial_record!("[view.thumb] status=1 {}", why); 1 }
        };
        std::process::exit(status);
    }
    let window = sex_std::window();
    window.title(b"View");
    let mut state = State { shown: 0, playing: None, look: Look::default(), still: None, scratch: Vec::new(), slides: None, auto: None, drag: None,
        panel: Panel { on: false, lines: Vec::new(), letters: Letters::new() } };
    match words.iter().find(|w| !w.is_empty()).map(|f| f.as_bytes().to_vec()) {
        None => fail(window, "Usage: run view @PICTURE (PNG, JPEG, WebP, GIF, JPEG XL, SVG or camera RAW), or run view @FOLDER"),
        Some(file) => {
            let folder = files::stat(window, &file).is_ok_and(|s| s.dir);
            if folder {
                match Slides::list(window, &file) {
                    Some(slides) => { state.slides = Some(slides); state.slide(window, 0); }
                    None => fail(window, "That folder has no pictures View knows."),
                }
            } else {
                state.load(window, &file);
                // Its folder too, when this run may read it: N and P step through it.
                let cut = file.iter().rposition(|b| *b == b'/').unwrap_or(0);
                let name = &file[cut + 1..];
                if state.shown > 0 && cut > 0 {
                    if let Some(mut slides) = Slides::list(window, &file[..cut]) {
                        slides.index = slides.names.iter().position(|n| n == name).unwrap_or(0);
                        sex_pdx::serial_record!("[view.slide] index={} of={} name={}", slides.index + 1, slides.names.len(), String::from_utf8_lossy(name));
                        state.slides = Some(slides);
                    }
                }
            }
        }
    }
    sex_pdx::serial_record!("[view.ready] target={} task={}", window.target, window.task);
    loop {
        let now = sex_pdx::get_ticks();
        if state.auto.is_some_and(|due| now >= due) {
            state.slide(window, 1);
            state.auto = Some(sex_pdx::get_ticks() + SLIDE_MS);
            continue;
        }
        let until_slide = state.auto.map(|due| due.saturating_sub(now));
        let event = if let Some(p) = state.playing.as_mut() {
            if !p.tick(window, &mut state.look, &mut state.scratch, &mut state.panel) {
                state.playing = None;
                continue;
            }
            match window.poll() {
                Some(event) => event,
                None => {
                    let wait = until_slide.map_or(p.wait(), |s| p.wait().min(s.max(1)));
                    let _ = sex_pdx::sys_wait_self_ticks(wait);
                    continue;
                }
            }
        } else if let Some(ms) = until_slide {
            match window.next_within(ms.max(1)) { Some(event) => event, None => continue }
        } else {
            window.next()
        };
        match event {
            Event::Close => { sex_pdx::serial_record!("[view.closed] target={} task={}", window.target, window.task); break; }
            // A new window size: fit the picture to it again (an animation's next frame does).
            Event::Resize { .. } if state.shown > 0 && state.playing.is_none() => state.redraw(window),
            Event::Key(key) => state.key(window, key),
            Event::Pointer(p) => state.pointer(window, p),
            _ => {}
        }
    }
    std::process::exit(0);
}
