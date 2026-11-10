// SPDX-License-Identifier: GPL-3.0-only
//! The window, the keys and the rendered page.
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{render, PixmapSettings, RenderCache, RenderSettings};
use sex_std::sex_app::{self, Event, Nav};
use std::time::Instant;

use crate::sample;

const BACKGROUND: u32 = 0x003a3a3e;
const MARGIN: usize = 12;

struct Viewer {
    pdf: Pdf,
    name: String,
    page: usize,
    /// 1.0 fits the page to the window's width.
    zoom: f32,
    scroll: usize,
    /// The rendered page: pixels 0x00RRGGBB, width, height.
    shown: Option<(Vec<u32>, usize, usize, usize, f32)>,
}

impl Viewer {
    fn pages(&self) -> usize { self.pdf.pages().len() }

    /// Renders the current page for a window `width` wide, unless it is already.
    fn render(&mut self, width: usize) {
        if self.shown.as_ref().is_some_and(|s| s.3 == self.page && s.4 == self.zoom && s.1.abs_diff(self.fit_width(width)) <= 1) { return; }
        let pages = self.pdf.pages();
        let Some(page) = pages.get(self.page) else { return };
        let (pw, _) = page.render_dimensions();
        let scale = (self.fit_width(width) as f32 / pw.max(1.0)).max(0.05);
        let started = Instant::now();
        let cache = RenderCache::new();
        let pixmap = render(page, &cache, &InterpreterSettings::default(), &RenderSettings::default(),
            &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE });
        let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
        let rgba = pixmap.data_as_u8_slice();
        let pixels: Vec<u32> = rgba.chunks_exact(4).map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect();
        let ink = pixels.iter().filter(|p| **p != 0x00ffffff).count();
        sex_std::record(&format!("[pdf.page] page={} pages={} width={w} height={h} ink={ink} ms={}",
            self.page + 1, pages.len(), started.elapsed().as_millis()));
        self.shown = Some((pixels, w, h, self.page, self.zoom));
    }

    fn fit_width(&self, width: usize) -> usize {
        ((width.saturating_sub(2 * MARGIN)) as f32 * self.zoom).clamp(64.0, 6000.0) as usize
    }

    fn draw(&mut self, window: &mut sex_app::Window) {
        let (width, height) = window.size();
        self.render(width);
        let Some((page, pw, ph, _, _)) = &self.shown else { return };
        let out = window.pixels();
        let (width, height) = (width.min(out.len() / height.max(1)), height);
        let max_scroll = (ph + 2 * MARGIN).saturating_sub(height);
        self.scroll = self.scroll.min(max_scroll);
        let left = width.saturating_sub(*pw) / 2;
        for y in 0..height {
            let row = &mut out[y * width..(y + 1) * width];
            row.fill(BACKGROUND);
            let py = (y + self.scroll).wrapping_sub(MARGIN);
            if py < *ph {
                let from = &page[py * pw..py * pw + pw];
                let n = (*pw).min(width - left);
                row[left..left + n].copy_from_slice(&from[..n]);
            }
        }
        window.show_pixels(width, height);
        let short: String = self.name.chars().take(9).collect();
        window.title(format!("{short} {}/{}", self.page + 1, self.pages()).as_bytes());
    }
}

fn open(name: &str) -> Result<Pdf, String> {
    let bytes = std::fs::read(name).map_err(|e| format!("Could not read {name}: {e}"))?;
    Pdf::new(bytes).map_err(|e| format!("{name} is not a PDF this viewer can read ({e:?})"))
}

pub fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| {
        if std::fs::metadata("welcome.pdf").is_err() {
            if let Err(e) = std::fs::write("welcome.pdf", sample::document()) { eprintln!("Could not write welcome.pdf: {e}"); }
        }
        "welcome.pdf".to_string()
    });
    let pdf = match open(&name) {
        Ok(pdf) => pdf,
        Err(message) => {
            // Said in the window as text; closing it ends the program.
            println!("{message}\n\nPut a PDF in this program's folder and run: pdf NAME.pdf");
            std::process::exit(1);
        }
    };
    let (target, task) = sex_std::identity();
    sex_std::record(&format!("[pdf.open] target={target} task={task} name={name} pages={}", pdf.pages().len()));
    let window = sex_std::window();
    let mut viewer = Viewer { pdf, name, page: 0, zoom: 1.0, scroll: 0, shown: None };
    viewer.draw(window);
    loop {
        let before = (viewer.page, viewer.zoom, viewer.scroll);
        match window.next() {
            Event::Close => break,
            Event::Resize { .. } => { viewer.draw(window); continue; }
            Event::Key(key) => {
                let last = viewer.pages().saturating_sub(1);
                let step = window.size().1 * 3 / 4;
                match (sex_app::nav(key), sex_app::typed(key), key.code) {
                    (Some(Nav::Right), ..) | (_, Some(b' ' | b'n'), _) | (_, _, 0x51 | 0x6D) => viewer.page = (viewer.page + 1).min(last),
                    (Some(Nav::Left), ..) | (_, Some(b'p'), _) | (_, _, 0x49 | 0x68) => viewer.page = viewer.page.saturating_sub(1),
                    (Some(Nav::Home), ..) => viewer.page = 0,
                    (Some(Nav::End), ..) => viewer.page = last,
                    (Some(Nav::Down), ..) => viewer.scroll += step,
                    (Some(Nav::Up), ..) => viewer.scroll = viewer.scroll.saturating_sub(step),
                    (_, Some(b'+' | b'='), _) => viewer.zoom = (viewer.zoom * 1.25).min(4.0),
                    (_, Some(b'-'), _) => viewer.zoom = (viewer.zoom / 1.25).max(0.25),
                    (_, Some(b'0'), _) => viewer.zoom = 1.0,
                    _ => {}
                }
                if viewer.page != before.0 { viewer.scroll = 0; }
            }
            _ => continue,
        }
        if (viewer.page, viewer.zoom, viewer.scroll) != before { viewer.draw(window); }
    }
    sex_std::record("[pdf.closed]");
    std::process::exit(0);
}
