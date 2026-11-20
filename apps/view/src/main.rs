// SPDX-License-Identifier: GPL-3.0-only
//! Gaze (`apps/view`, `/bin/gaze`): SexOS Image Viewer, Faces-Free Photo Library,
//! Albums, Smart Search by Date & Place, and Non-Destructive Shared-Batik Edit Studio.
//!
//! * `run gaze @photo.png` / `run gaze @folder`: Chromeless pixel-exact Viewer Mode
//!   with quarter-turn rotation (`R` / `Shift+R`), zoom (`+`/`-`/`0`/`1`), pan, animated
//!   GIF/WebP playback, and folder slideshows (`N`, `P`, `Space`).
//! * `run gaze` / `run gaze --library [@folder ...]`: Full Photo Library Mode with
//!   persistent catalog (`/data/gaze/library.gaze`), user albums, offline reverse-geocoded
//!   smart search by date & place (strictly faces-free), and non-destructive crop,
//!   filmic EV exposure, and shared `batik-core` filter presets & stacks (`E`).
//! * `gaze --thumb @IN @OUT SIZE`: no window; writes a thumbnail for Loom (the
//!   `thumb` module), camera RAW previews included.
#![no_std]
#![no_main]

extern crate alloc;

mod canvas;
mod studio;
mod thumb;

use alloc::vec::Vec;
use gaze_core::apply_full_develop;
use sex_app::{files, Event, Window};
use sex_image::view as look;
use sex_image::{Animation, Error, Image, Limits};
use sex_pdx::app_pointer::Kind;
use sex_pdx::pixel_region::{MAX_H, MAX_W, PIXELS};

use studio::{join_path, GazeStudio, InputMode, StudioOutcome, StudioScreen};

/// Static buffer holding the active file in Viewer Mode so `Animation<&'static [u8]>` borrows safely.
static mut INPUT: [u8; 4 << 20] = [0; 4 << 20];
const BACK: u32 = 0x000D_1420;
const SLIDE_MS: u64 = 2500;
const ESC_CODE: u8 = 0x01;
const TAB_CODE: u8 = 0x0F;

fn input() -> &'static mut [u8] {
    unsafe { &mut *core::ptr::addr_of_mut!(INPUT) }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[view.fatal] {}", info);
    sex_pdx::task_finish(1);
}

fn fail(window: &mut Window, why: &[u8]) {
    sex_pdx::serial_record!(
        "[view.failed] {}",
        core::str::from_utf8(why).unwrap_or("?")
    );
    let mut text = [0u8; 160];
    let head = b"View. F11 closes.\n\n";
    let n = (head.len() + why.len()).min(text.len());
    text[..head.len()].copy_from_slice(head);
    text[head.len()..n].copy_from_slice(&why[..n - head.len()]);
    window.show(&text[..n]);
}

const SHRINKABLE: Limits = Limits {
    max_width: 4 * MAX_W,
    max_height: 4 * MAX_H,
    max_pixels: 16 * PIXELS,
    max_input_bytes: usize::MAX,
};

fn open(bytes: &[u8], out: &mut [u32], (aw, ah): (usize, usize)) -> Result<(usize, usize, bool), Error> {
    let (aw, ah) = (aw.max(1), ah.max(1));
    let surface = Limits {
        max_width: aw,
        max_height: ah,
        max_pixels: aw * ah,
        max_input_bytes: usize::MAX,
    };
    match sex_image::decode_into(bytes, &surface, out) {
        Err(Error::TooLarge) => {}
        other => return other.map(|(w, h)| (w, h, false)),
    }
    let image = sex_image::decode(bytes, &SHRINKABLE)?.fit_within(aw, ah)?;
    out[..image.pixels.len()].copy_from_slice(&image.pixels);
    Ok((image.width, image.height, true))
}

fn flatten(pixels: &mut [u32]) {
    for p in pixels.iter_mut() {
        let a = *p >> 24;
        if a != 255 {
            let mix =
                |s: u32| ((((*p >> s) & 0xFF) * a + ((BACK >> s) & 0xFF) * (255 - a)) / 255) << s;
            *p = mix(16) | mix(8) | mix(0);
        }
        *p &= 0x00FF_FFFF;
    }
}

#[derive(Clone, Copy, Default)]
struct Look {
    turns: u8,
    zoom: Option<u32>,
    pan: (isize, isize),
}

impl Look {
    fn plain(&self) -> bool {
        self.turns % 4 == 0 && self.zoom.is_none()
    }
}

fn show(
    window: &mut Window,
    src: &[u32],
    (w, h): (usize, usize),
    look: &mut Look,
    scratch: &mut Vec<u32>,
) -> Option<(usize, usize)> {
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
        None if w <= aw && h <= ah => {
            out[..w * h].copy_from_slice(src);
            (w, h)
        }
        None => {
            let mut pixels = Vec::new();
            pixels.try_reserve_exact(src.len()).ok()?;
            pixels.extend_from_slice(src);
            let image = (Image {
                width: w,
                height: h,
                pixels,
            })
            .fit_within(aw, ah)
            .ok()?;
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
    window.show_pixels(ow, oh);
    Some((ow, oh))
}

struct Playing {
    animation: Animation<&'static [u8]>,
    canvas: Vec<u32>,
    due: u64,
    shown: u64,
}

impl Playing {
    fn start(bytes: &'static [u8]) -> Option<Self> {
        let animation = Animation::open(bytes, &SHRINKABLE)
            .ok()
            .filter(|a| a.frames() > 1)?;
        let count = animation.width * animation.height;
        let mut canvas = Vec::new();
        canvas.try_reserve_exact(count).ok()?;
        canvas.resize(count, 0);
        sex_pdx::serial_record!(
            "[view.animated] frames={} loops={}",
            animation.frames(),
            animation.loops().map_or(-1, |n| n as i64)
        );
        Some(Playing {
            animation,
            canvas,
            due: sex_pdx::get_ticks(),
            shown: 0,
        })
    }

    fn tick(&mut self, window: &mut Window, look: &mut Look, scratch: &mut Vec<u32>) -> bool {
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
        let Ok(delay) = self.animation.next_into(&mut self.canvas) else {
            return false;
        };
        if show(
            window,
            &self.canvas,
            (self.animation.width, self.animation.height),
            look,
            scratch,
        )
        .is_none()
        {
            return false;
        }
        self.shown += 1;
        if self.shown <= frames + 1 {
            sex_pdx::serial_record!("[view.frame] n={} delay={}", self.shown, delay);
        }
        self.due = now + (delay as u64).max(20);
        true
    }

    fn wait(&self) -> u64 {
        self.due.saturating_sub(sex_pdx::get_ticks()).clamp(1, 16)
    }
}

fn present(window: &mut Window, bytes: &[u8]) -> bool {
    let area = window.size();
    let out = window.pixels();
    match open(bytes, out, area) {
        Ok((w, h, scaled)) => {
            flatten(&mut out[..w * h]);
            window.show_pixels(w, h);
            sex_pdx::serial_record!(
                "[view.shown] w={} h={} bytes={} scaled={} fit={}x{}",
                w,
                h,
                bytes.len(),
                scaled as u8,
                area.0,
                area.1
            );
            true
        }
        Err(Error::UnknownFormat) => {
            fail(window, b"That file is not a PNG, JPEG, WebP or GIF picture.");
            false
        }
        Err(Error::TooLarge) => {
            fail(window, b"That picture is too large to open.");
            false
        }
        Err(Error::OutOfMemory) => {
            fail(window, b"That picture needs more memory than View has.");
            false
        }
        Err(_) => {
            fail(
                window,
                b"That picture is damaged or uses an unsupported feature.",
            );
            false
        }
    }
}

struct Slides {
    folder: Vec<u8>,
    names: Vec<Vec<u8>>,
    index: usize,
}

impl Slides {
    fn path(&self, i: usize) -> Vec<u8> {
        join_path(&self.folder, &self.names[i])
    }
    fn list(window: &mut Window, folder: &[u8]) -> Option<Slides> {
        let mut names: Vec<Vec<u8>> = files::list(window, folder)
            .ok()?
            .into_iter()
            .filter(|e| !e.dir && look::is_picture_name(&e.name))
            .map(|e| e.name)
            .collect();
        if names.is_empty() {
            return None;
        }
        names.sort();
        Some(Slides {
            folder: folder.to_vec(),
            names,
            index: 0,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Viewer,
    Studio,
}

struct ViewerState {
    current_path: Vec<u8>,
    shown: usize,
    playing: Option<Playing>,
    look: Look,
    still: Option<Image>,
    scratch: Vec<u32>,
    slides: Option<Slides>,
    auto: Option<u64>,
    drag: Option<(i16, i16)>,
    entered_from_studio: bool,
}

impl ViewerState {
    fn new() -> Self {
        Self {
            current_path: Vec::new(),
            shown: 0,
            playing: None,
            look: Look::default(),
            still: None,
            scratch: Vec::new(),
            slides: None,
            auto: None,
            drag: None,
            entered_from_studio: false,
        }
    }

    fn bytes(&self) -> &'static [u8] {
        &input()[..self.shown]
    }

    fn load(&mut self, window: &mut Window, studio: &GazeStudio, path: &[u8]) {
        self.playing = None;
        self.still = None;
        self.look = Look::default();
        self.shown = 0;
        self.current_path = path.to_vec();

        let buffer = input();
        match files::read(window, path, buffer) {
            Err(e) => fail(window, files::message(e)),
            Ok(len) if len == buffer.len() => fail(window, b"That file is larger than 4 MiB."),
            Ok(len) => {
                // If this photo has a non-destructive edit/develop recipe in library.gaze or sidecar, apply it!
                if let Some((recipe, dev)) = studio
                    .recipes_for_path(path)
                    .filter(|(r, d)| !r.is_identity() || !d.is_identity())
                {
                    if let Ok(raw_img) = sex_image::decode(&buffer[..len], &SHRINKABLE) {
                        if let Ok(edited_img) = apply_full_develop(&raw_img, recipe, dev) {
                            let (ew, eh) = (edited_img.width, edited_img.height);
                            self.still = Some(edited_img);
                            self.shown = len;
                            if let Some(st) = self.still.as_ref() {
                                if let Some((dw, dh)) = show(
                                    window,
                                    &st.pixels,
                                    (ew, eh),
                                    &mut self.look,
                                    &mut self.scratch,
                                ) {
                                    let area = window.size();
                                    let scaled = dw < ew || dh < eh;
                                    sex_pdx::serial_record!(
                                        "[view.shown] w={} h={} bytes={} scaled={} fit={}x{}",
                                        dw,
                                        dh,
                                        len,
                                        scaled as u8,
                                        area.0,
                                        area.1
                                    );
                                }
                            }
                            return;
                        }
                    }
                }

                if present(window, &buffer[..len]) {
                    self.shown = len;
                }
            }
        }
        if self.shown > 0 && self.still.is_none() {
            self.playing = Playing::start(self.bytes());
        }
    }

    fn slide(&mut self, window: &mut Window, studio: &GazeStudio, step: isize) {
        let Some(s) = self.slides.as_mut() else { return };
        let n = s.names.len() as isize;
        s.index = (s.index as isize + step).rem_euclid(n) as usize;
        let (path, index, name) = (s.path(s.index), s.index, s.names[s.index].clone());
        self.load(window, studio, &path);
        sex_pdx::serial_record!(
            "[view.slide] index={} of={} name={}",
            index + 1,
            n,
            core::str::from_utf8(&name).unwrap_or("?")
        );
    }

    fn redraw(&mut self, window: &mut Window, studio: &GazeStudio) {
        if self.shown == 0 {
            return;
        }
        if self.playing.is_some() {
            if let Some(p) = self.playing.as_mut() {
                p.due = 0;
            }
            return;
        }
        let has_recipe = studio
            .recipes_for_path(&self.current_path)
            .map_or(false, |(r, d)| !r.is_identity() || !d.is_identity());
        if self.look.plain() && !has_recipe {
            present(window, self.bytes());
            return;
        }
        if self.still.is_none() {
            match sex_image::decode(self.bytes(), &SHRINKABLE) {
                Ok(image) => self.still = Some(image),
                Err(_) => {
                    sex_pdx::serial_record!(
                        "[view.failed] That picture needs more memory than View has."
                    );
                    return;
                }
            }
        }
        let Some(still) = self.still.as_ref() else {
            return;
        };
        match show(
            window,
            &still.pixels,
            (still.width, still.height),
            &mut self.look,
            &mut self.scratch,
        ) {
            Some((w, h)) => sex_pdx::serial_record!(
                "[view.drawn] w={} h={} deg={} pct={}",
                w,
                h,
                self.look.turns as u32 * 90,
                self.look.zoom.unwrap_or(0)
            ),
            None => {
                sex_pdx::serial_record!("[view.failed] That picture needs more memory than View has.")
            }
        }
    }

    fn turned_size(&self) -> (usize, usize) {
        let (w, h) = match (&self.playing, &self.still) {
            (Some(p), _) => (p.animation.width, p.animation.height),
            (None, Some(s)) => (s.width, s.height),
            _ => sex_image::dimensions(self.bytes())
                .map(|(_, w, h)| (w, h))
                .unwrap_or((1, 1)),
        };
        if self.look.turns % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }

    fn zoom(&mut self, window: &mut Window, studio: &GazeStudio, pct: Option<u32>) {
        self.look.zoom = pct;
        self.look.pan = (0, 0);
        sex_pdx::serial_record!("[view.zoom] pct={}", pct.unwrap_or(0));
        self.redraw(window, studio);
    }

    fn pan(&mut self, window: &mut Window, studio: &GazeStudio, dx: isize, dy: isize) {
        if self.look.zoom.is_none() {
            return;
        }
        self.look.pan = (self.look.pan.0 + dx, self.look.pan.1 + dy);
        self.redraw(window, studio);
    }

    fn pointer(&mut self, window: &mut Window, studio: &GazeStudio, p: sex_pdx::app_pointer::Pointer) {
        match p.kind {
            Kind::Down { button: 0 } => self.drag = Some((p.x, p.y)),
            Kind::Move if p.buttons & 1 != 0 => {
                if let Some((x, y)) = self.drag {
                    self.drag = Some((p.x, p.y));
                    self.pan(window, studio, x as isize - p.x as isize, y as isize - p.y as isize);
                }
            }
            Kind::Up { button: 0 } | Kind::Leave => self.drag = None,
            Kind::Wheel { steps } if self.look.zoom.is_some() => {
                self.pan(window, studio, 0, -(steps as isize) * 48)
            }
            _ => {}
        }
    }
}

fn switch_viewer_to_studio(
    window: &mut Window,
    viewer: &mut ViewerState,
    studio: &mut GazeStudio,
    target_screen: StudioScreen,
    focus_search: bool,
) -> AppMode {
    viewer.auto = None;
    let mut scan_paths = Vec::new();
    if let Some(ref s) = viewer.slides {
        scan_paths.push(s.folder.clone());
    } else if !viewer.current_path.is_empty() {
        scan_paths.push(viewer.current_path.clone());
    }
    if !scan_paths.is_empty() {
        studio.scan_sources(window, &scan_paths);
    }
    if !viewer.current_path.is_empty() {
        studio.select_by_path(&viewer.current_path);
    }
    studio.screen = StudioScreen::Library;
    studio.input_mode = if focus_search {
        InputMode::Search
    } else {
        InputMode::None
    };
    if target_screen == StudioScreen::Edit {
        let _ = studio.open_edit_studio(window);
    }
    studio.render(window);
    AppMode::Studio
}

extern "C" fn app_main() -> ! {
    // `--thumb` runs hidden for Loom: no window, one picture, then the run ends.
    {
        let mut buf = [0u8; sex_pdx::fs::ARGS_MAX];
        let n = files::direct::args(&mut buf);
        let words: Vec<Vec<u8>> = buf[..n].split(|b| *b == b' ').filter(|w| !w.is_empty()).map(|w| w.to_vec()).collect();
        if let Some(status) = thumb::run(&words) {
            sex_app::launch::exit(status);
        }
    }
    let mut window = Window::open();
    window.title(b"Gaze");

    let mut args_buf = [0u8; sex_pdx::fs::ARGS_MAX];
    let given = files::args(&mut window, &mut args_buf);
    let words: Vec<Vec<u8>> = given
        .split(|b| *b == b' ')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_vec())
        .collect();

    let mut force_library = false;
    let mut path_args: Vec<Vec<u8>> = Vec::new();
    for w in &words {
        if w == b"--library" || w == b"-l" {
            force_library = true;
        } else {
            path_args.push(w.clone());
        }
    }

    let mut studio = GazeStudio::new();
    studio.load_catalog(&mut window);

    let mut viewer = ViewerState::new();
    let mut mode = if force_library || path_args.is_empty() {
        studio.scan_sources(&mut window, &path_args);
        studio.persist_catalog(&mut window);
        studio.render(&mut window);
        sex_pdx::serial_record!(
            "[gaze.library.ready] photos={} albums={} mode=library",
            studio.library.photos.len(),
            studio.library.albums.len()
        );
        AppMode::Studio
    } else {
        let file = path_args[0].clone();
        let folder = files::stat(&mut window, &file).is_ok_and(|s| s.dir);
        if folder {
            match Slides::list(&mut window, &file) {
                Some(slides) => {
                    viewer.slides = Some(slides);
                    viewer.slide(&mut window, &studio, 0);
                }
                None => fail(
                    &mut window,
                    b"That folder has no PNG, JPEG, WebP or GIF pictures.",
                ),
            }
        } else {
            viewer.load(&mut window, &studio, &file);
            let cut = file.iter().rposition(|b| *b == b'/').unwrap_or(0);
            let name = &file[cut + 1..];
            if viewer.shown > 0 && cut > 0 {
                if let Some(mut slides) = Slides::list(&mut window, &file[..cut]) {
                    slides.index = slides.names.iter().position(|n| n == name).unwrap_or(0);
                    sex_pdx::serial_record!(
                        "[view.slide] index={} of={} name={}",
                        slides.index + 1,
                        slides.names.len(),
                        core::str::from_utf8(name).unwrap_or("?")
                    );
                    viewer.slides = Some(slides);
                }
            }
        }
        AppMode::Viewer
    };

    sex_pdx::serial_record!("[view.ready] target={} task={}", window.target, window.task);

    loop {
        if mode == AppMode::Studio {
            match window.next() {
                Event::Close => {
                    sex_pdx::serial_record!(
                        "[view.closed] target={} task={}",
                        window.target,
                        window.task
                    );
                    window.close()
                }
                Event::Resize { .. } => {
                    studio.render(&mut window);
                }
                Event::Key(key) => {
                    if studio.handle_key(&mut window, key) == StudioOutcome::OpenViewer {
                        if let Some(path) = studio.selected_path().map(|s| s.as_bytes().to_vec()) {
                            viewer.entered_from_studio = true;
                            viewer.load(&mut window, &studio, &path);
                            mode = AppMode::Viewer;
                            continue;
                        }
                    }
                    studio.render(&mut window);
                }
                Event::Pointer(p) => {
                    if studio.handle_pointer(&mut window, p) == StudioOutcome::OpenViewer {
                        if let Some(path) = studio.selected_path().map(|s| s.as_bytes().to_vec()) {
                            viewer.entered_from_studio = true;
                            viewer.load(&mut window, &studio, &path);
                            mode = AppMode::Viewer;
                            continue;
                        }
                    } else if matches!(p.kind, Kind::Down { button: 0 }) {
                        studio.render(&mut window);
                    }
                }
                _ => {}
            }
            continue;
        }

        // Viewer Mode loop (pixel-exact & animation/slideshow timing preserved)
        let now = sex_pdx::get_ticks();
        if viewer.auto.is_some_and(|due| now >= due) {
            viewer.slide(&mut window, &studio, 1);
            viewer.auto = Some(sex_pdx::get_ticks() + SLIDE_MS);
            continue;
        }
        let until_slide = viewer.auto.map(|due| due.saturating_sub(now));
        let event = if let Some(p) = viewer.playing.as_mut() {
            if !p.tick(&mut window, &mut viewer.look, &mut viewer.scratch) {
                viewer.playing = None;
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
            match window.next_within(ms.max(1)) {
                Some(event) => event,
                None => continue,
            }
        } else {
            window.next()
        };

        match event {
            Event::Close => {
                sex_pdx::serial_record!(
                    "[view.closed] target={} task={}",
                    window.target,
                    window.task
                );
                window.close()
            }
            Event::Resize { .. } if viewer.shown > 0 && viewer.playing.is_none() => {
                viewer.redraw(&mut window, &studio)
            }
            Event::Key(key) => {
                if key.code == TAB_CODE || (key.code == ESC_CODE && viewer.entered_from_studio) {
                    mode = switch_viewer_to_studio(
                        &mut window,
                        &mut viewer,
                        &mut studio,
                        StudioScreen::Library,
                        false,
                    );
                    continue;
                }
                match sex_app::typed(key) {
                    Some(b'l' | b'L') => {
                        mode = switch_viewer_to_studio(
                            &mut window,
                            &mut viewer,
                            &mut studio,
                            StudioScreen::Library,
                            false,
                        );
                        continue;
                    }
                    Some(b'/') => {
                        mode = switch_viewer_to_studio(
                            &mut window,
                            &mut viewer,
                            &mut studio,
                            StudioScreen::Library,
                            true,
                        );
                        continue;
                    }
                    Some(b'e' | b'E') if viewer.shown > 0 => {
                        mode = switch_viewer_to_studio(
                            &mut window,
                            &mut viewer,
                            &mut studio,
                            StudioScreen::Edit,
                            false,
                        );
                        continue;
                    }
                    _ => {}
                }

                let (aw, ah) = window.size();
                let zoomed = viewer.look.zoom.is_some();
                match (sex_app::nav(key), sex_app::typed(key)) {
                    (_, Some(b'r' | b'R')) if viewer.shown > 0 => {
                        viewer.look.turns =
                            (viewer.look.turns + if key.shift { 3 } else { 1 }) % 4;
                        viewer.look.pan = (0, 0);
                        sex_pdx::serial_record!(
                            "[view.rotated] deg={}",
                            viewer.look.turns as u32 * 90
                        );
                        viewer.redraw(&mut window, &studio);
                    }
                    (_, Some(b'+' | b'=')) if viewer.shown > 0 => {
                        let now_pct = viewer
                            .look
                            .zoom
                            .unwrap_or_else(|| look::fit_percent(viewer.turned_size(), (aw, ah)));
                        viewer.zoom(&mut window, &studio, Some(look::zoom_in(now_pct)));
                    }
                    (_, Some(b'-')) if viewer.shown > 0 => {
                        let now_pct = viewer
                            .look
                            .zoom
                            .unwrap_or_else(|| look::fit_percent(viewer.turned_size(), (aw, ah)));
                        viewer.zoom(&mut window, &studio, Some(look::zoom_out(now_pct)));
                    }
                    (_, Some(b'0')) if viewer.shown > 0 => viewer.zoom(&mut window, &studio, None),
                    (_, Some(b'1')) if viewer.shown > 0 => {
                        viewer.zoom(&mut window, &studio, Some(100))
                    }
                    (Some(sex_app::Nav::Left), _) if zoomed => {
                        viewer.pan(&mut window, &studio, -(aw as isize / 4), 0)
                    }
                    (Some(sex_app::Nav::Right), _) if zoomed => {
                        viewer.pan(&mut window, &studio, aw as isize / 4, 0)
                    }
                    (Some(sex_app::Nav::Up), _) if zoomed => {
                        viewer.pan(&mut window, &studio, 0, -(ah as isize / 4))
                    }
                    (Some(sex_app::Nav::Down), _) if zoomed => {
                        viewer.pan(&mut window, &studio, 0, ah as isize / 4)
                    }
                    (Some(sex_app::Nav::Right), _) | (_, Some(b'n')) => {
                        viewer.slide(&mut window, &studio, 1)
                    }
                    (Some(sex_app::Nav::Left), _) | (_, Some(b'p')) => {
                        viewer.slide(&mut window, &studio, -1)
                    }
                    (_, Some(b' ')) if viewer.slides.is_some() => {
                        viewer.auto = match viewer.auto {
                            Some(_) => None,
                            None => Some(sex_pdx::get_ticks() + SLIDE_MS),
                        };
                        sex_pdx::serial_record!(
                            "[view.slideshow] on={} ms={}",
                            viewer.auto.is_some() as u8,
                            SLIDE_MS
                        );
                    }
                    _ => {}
                }
            }
            Event::Pointer(p) => viewer.pointer(&mut window, &studio, p),
            _ => {}
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    sex_app::with_stack(512 * 1024, app_main)
}
