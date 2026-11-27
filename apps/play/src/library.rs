// SPDX-License-Identifier: GPL-3.0-only
//! Qupid's Apple Music / Brootlite-inspired Media Player & Library UI:
//! `run qupid` (auto-scans `/media/Ventoy`, `/Music`, `/data/music`, `/data/media`),
//! `run qupid --library @/Music` (a folder, read through Files, its subfolders too),
//! or `run qupid --library @a.flac @b.mp3`.
//!
//! Tracks are grouped by artist and album from their tags (FFmpeg reads them inside
//! this program's confinement) and play one after another with no gap between them:
//! the next track is opened while the current one plays, and its first sound follows
//! the last sound of the one before in the same ring.
//!
//! Keys: Up/Down choose, Enter plays from the chosen track, Space or the play key
//! pauses, N or the next key skips, P or the previous key goes back, Q adds the chosen
//! track to the queue (it plays next), Tab cycles sidebar views, F11 closes.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_app::sound::Sound;
use sex_app::{files, Event, Window};
use sex_font::raster::{scratch_len, FX};
use sex_font::Font;
use sex_media::{Decoder, Next, Source};
use sex_theme::candy_ui::{
    draw_candy_badge, draw_candy_button, draw_candy_panel, draw_candy_selection_row,
    draw_candy_tab, draw_candy_toolbar, fill_rounded_rect, ButtonState, ButtonStyle, CandyPalette,
};

const AUDIO: [&[u8]; 9] = [
    b".mp3", b".flac", b".ogg", b".oga", b".opus", b".m4a", b".aac", b".wav", b".mka",
];
fn is_audio(name: &[u8]) -> bool {
    AUDIO
        .iter()
        .any(|e| name.len() > e.len() && name[name.len() - e.len()..].eq_ignore_ascii_case(e))
}
const MAX_TRACKS: usize = 2000;

/// Default media roots scanned when Qupid is launched with no arguments.
const DEFAULT_ROOTS: [&[u8]; 4] = [b"/media/Ventoy", b"/Music", b"/data/music", b"/data/media"];

/// A file read through Files a piece at a time.
struct FileSource {
    window: *mut Window,
    path: Vec<u8>,
    len: u64,
    cache: Vec<u8>,
    cache_at: u64,
}
const CACHE: usize = 128 * 1024;
impl Source for FileSource {
    fn len(&self) -> Option<u64> {
        Some(self.len)
    }
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Option<usize> {
        if at >= self.len {
            return Some(0);
        }
        if !(at >= self.cache_at && at < self.cache_at + self.cache.len() as u64) {
            let want = (CACHE as u64).min(self.len - at) as usize;
            self.cache.resize(want, 0);
            let window = unsafe { &mut *self.window };
            match files::read_at(window, &self.path, at, &mut self.cache) {
                Ok(got) if got > 0 => {
                    self.cache.truncate(got);
                    self.cache_at = at;
                }
                _ => return None,
            }
        }
        let off = (at - self.cache_at) as usize;
        let n = out.len().min(self.cache.len() - off);
        out[..n].copy_from_slice(&self.cache[off..off + n]);
        Some(n)
    }
}

#[derive(Clone)]
struct Track {
    path: Vec<u8>,
    len: u64,
    title: String,
    artist: String,
    album: String,
    number: u32,
    ms: u64,
}

fn open(window: &mut Window, t: &Track) -> Option<Decoder> {
    let wp: *mut Window = window;
    Decoder::open(Box::new(FileSource {
        window: wp,
        path: t.path.clone(),
        len: t.len,
        cache: Vec::new(),
        cache_at: 0,
    }))
    .ok()
}

fn base_name(path: &[u8]) -> String {
    let name = path.rsplit(|b| *b == b'/').next().unwrap_or(path);
    let stem = match name.iter().rposition(|b| *b == b'.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    };
    String::from_utf8_lossy(stem).into_owned()
}

/// Every audio file under `path` (a file, or a folder three levels down).
fn collect(window: &mut Window, path: &[u8], depth: u32, out: &mut Vec<(Vec<u8>, u64)>) {
    if out.len() >= MAX_TRACKS {
        return;
    }
    match files::stat(window, path) {
        Ok(st) if st.dir => {
            if depth > 3 {
                return;
            }
            let Ok(mut entries) = files::list(window, path) else {
                return;
            };
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            for e in entries {
                let mut p = path.to_vec();
                if !p.ends_with(b"/") {
                    p.push(b'/');
                }
                p.extend_from_slice(&e.name);
                if e.dir {
                    collect(window, &p, depth + 1, out);
                } else if is_audio(&e.name) && out.len() < MAX_TRACKS {
                    out.push((p, e.size));
                }
            }
        }
        Ok(st) if is_audio(path) => out.push((path.to_vec(), st.size)),
        _ => {}
    }
}

/// Reads each file's tags; files FFmpeg cannot open are left out.
fn library(window: &mut Window, args: &[&[u8]]) -> Vec<Track> {
    let mut found = Vec::new();
    for a in args {
        collect(window, a, 0, &mut found);
    }
    let mut tracks = Vec::new();
    for (k, (path, len)) in found.into_iter().enumerate() {
        if k % 25 == 0 && k > 0 {
            window.show(format!("Qupid Library\n\nReading tags: {} files...", k).as_bytes());
        }
        let t = Track {
            path,
            len,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            number: 0,
            ms: 0,
        };
        let Some(d) = open(window, &t) else {
            continue;
        };
        if d.info.sample_rate == 0 {
            continue;
        }
        let number = d
            .tag("track")
            .and_then(|v| v.split('/').next().and_then(|n| n.trim().parse().ok()))
            .unwrap_or(0);
        tracks.push(Track {
            title: d.tag("title").unwrap_or_else(|| base_name(&t.path)),
            artist: d
                .tag("album_artist")
                .or_else(|| d.tag("artist"))
                .unwrap_or_else(|| String::from("Unknown artist")),
            album: d
                .tag("album")
                .unwrap_or_else(|| String::from("Unknown album")),
            number,
            ms: d.info.duration_ms,
            ..t
        });
    }
    tracks.sort_by(|a, b| {
        (a.artist.to_lowercase(), a.album.to_lowercase(), a.number, &a.path).cmp(&(
            b.artist.to_lowercase(),
            b.album.to_lowercase(),
            b.number,
            &b.path,
        ))
    });
    tracks
}

fn clock(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}

/// Deterministic accent swatch color for an album/artist tile.
fn art_color(seed: &str) -> u32 {
    const SWATCHES: [u32; 6] = [
        0x00D6_336C, // Candy Berry
        0x009B_59B6, // Orchid
        0x005B_6EE1, // Periwinkle
        0x00E0_6C75, // Coral Rose
        0x0038_A3A5, // Lagoon Teal
        0x00E5_9866, // Peach Amber
    ];
    let mut h: u32 = 2166136261;
    for &b in seed.as_bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    SWATCHES[(h as usize) % SWATCHES.len()]
}

/// Compact Geist Sans rasterizer for Qupid's Candy UI.
struct TextPainter {
    font: Option<Font<'static>>,
    acc: Vec<i32>,
    glyph: Vec<u8>,
}

impl TextPainter {
    fn new() -> Self {
        Self {
            font: Font::new(sex_font::GEIST_SANS),
            acc: Vec::new(),
            glyph: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        max_w: i32,
        px_h: i32,
        text: &str,
        color: u32,
    ) {
        let Some(font) = self.font else {
            return;
        };
        let px_size = px_h.max(10) as usize;
        let size = (px_size * 64) as u32;
        let asc = font.ascender(size);
        let line_h = ((font.ascender(size) - font.descender(size)) / FX + 2).max(px_size as i32) as usize;
        let limit = (x + max_w).min(bw as i32);
        let cr = ((color >> 16) & 0xFF) as u32;
        let cg = ((color >> 8) & 0xFF) as u32;
        let cb = (color & 0xFF) as u32;
        let pad = 2usize;
        let mut pen = x * FX;
        let mut prev = None;
        for ch in text.chars() {
            if pen / FX >= limit - 6 {
                break;
            }
            let Some(id) = font.glyph(ch as u32) else {
                pen += (px_h / 2) * FX;
                prev = None;
                continue;
            };
            if let Some(p) = prev {
                pen += font.kerning(p, id, size);
            }
            let adv = font.advance(id, size);
            let gx0 = (pen / FX) - pad as i32;
            let gw = ((adv / FX).max(0) as usize + 2 * pad + 2).min(64);
            if gw > 0 && line_h > 0 && line_h <= 64 {
                let slen = scratch_len(gw, line_h);
                if self.acc.len() < slen {
                    self.acc.resize(slen, 0);
                }
                if self.glyph.len() < gw * line_h {
                    self.glyph.resize(gw * line_h, 0);
                }
                for b in &mut self.glyph[..gw * line_h] {
                    *b = 0;
                }
                let sub = pen - (pen / FX) * FX + pad as i32 * FX;
                font.draw(
                    id,
                    size,
                    sub,
                    asc + FX,
                    gw,
                    line_h,
                    &mut self.acc[..slen],
                    &mut self.glyph[..gw * line_h],
                );
                for row in 0..line_h {
                    let py = y + row as i32;
                    if py < 0 || py >= bh as i32 {
                        continue;
                    }
                    let row_off = py as usize * bw;
                    for col in 0..gw {
                        let px = gx0 + col as i32;
                        if px < 0 || px >= limit {
                            continue;
                        }
                        let a = self.glyph[row * gw + col] as u32;
                        if a == 0 {
                            continue;
                        }
                        let dst = buf[row_off + px as usize];
                        let dr = (dst >> 16) & 0xFF;
                        let dg = (dst >> 8) & 0xFF;
                        let db = dst & 0xFF;
                        let inv = 255 - a;
                        let nr = (cr * a + dr * inv) / 255;
                        let ng = (cg * a + dg * inv) / 255;
                        let nb = (cb * a + db * inv) / 255;
                        buf[row_off + px as usize] = (nr << 16) | (ng << 8) | nb;
                    }
                }
            }
            pen += adv;
            prev = Some(id);
        }
    }
}

/// Plays tracks back to back through one sound ring.
struct Engine {
    sound: Sound,
    /// The track playing (being decoded) and its decoder.
    cur: Option<(usize, Decoder)>,
    /// The one after it, opened ahead so its first sound follows at once.
    next: Option<(usize, Decoder)>,
    pcm: Vec<[i16; 2]>,
    left: (usize, usize),
    pic: Vec<u32>,
    paused: bool,
    /// Ring position where each track's sound began: (frame, track).
    starts: Vec<(u64, usize)>,
}

impl Engine {
    /// The track the listener hears now: the last one whose first frame has played.
    fn hearing(&self) -> Option<usize> {
        let played = self.sound.played_total();
        self.starts
            .iter()
            .rev()
            .find(|(f, _)| *f <= played)
            .or(self.starts.first())
            .map(|s| s.1)
    }
    /// Elapsed milliseconds in the currently heard track.
    fn elapsed_ms(&self) -> u64 {
        let played = self.sound.played_total();
        let start = self
            .starts
            .iter()
            .rev()
            .find(|(f, _)| *f <= played)
            .or(self.starts.first())
            .map(|s| s.0)
            .unwrap_or(played);
        played.saturating_sub(start) * 1000 / (sex_app::sound::RATE as u64).max(1)
    }
    fn stop(&mut self) {
        self.cur = None;
        self.next = None;
        self.left = (0, 0);
        self.starts.clear();
        self.sound.skip();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarTab {
    ListenNow,
    Songs,
    Albums,
    Queue,
}

impl SidebarTab {
    const ALL: [Self; 4] = [Self::ListenNow, Self::Songs, Self::Albums, Self::Queue];
    fn label(self) -> &'static str {
        match self {
            Self::ListenNow => "Listen Now",
            Self::Songs => "Songs",
            Self::Albums => "Albums",
            Self::Queue => "Up Next",
        }
    }
    fn next(self) -> Self {
        match self {
            Self::ListenNow => Self::Songs,
            Self::Songs => Self::Albums,
            Self::Albums => Self::Queue,
            Self::Queue => Self::ListenNow,
        }
    }
}

#[derive(Clone, Copy)]
enum HitAction {
    Tab(SidebarTab),
    Track(usize),
    Prev,
    PlayPause,
    Next,
    QueueSelected,
}

#[derive(Clone, Copy)]
struct HitBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    action: HitAction,
}

impl HitBox {
    fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

struct App {
    tracks: Vec<Track>,
    chosen: usize,
    queue: Vec<usize>,
    tab: SidebarTab,
    painter: TextPainter,
    hits: Vec<HitBox>,
    dirty: bool,
    last_render_ms: u64,
    last_click_track: Option<(usize, u64)>,
}

impl App {
    fn new(tracks: Vec<Track>) -> Self {
        Self {
            tracks,
            chosen: 0,
            queue: Vec::new(),
            tab: SidebarTab::Songs,
            painter: TextPainter::new(),
            hits: Vec::new(),
            dirty: true,
            last_render_ms: 0,
            last_click_track: None,
        }
    }

    /// The track after `i`: the queue first, then the library's order. The queue
    /// keeps it until [`App::commit`], so a track queued later can still go first.
    fn after(&self, i: usize) -> Option<usize> {
        if let Some(&q) = self.queue.first() {
            return Some(q);
        }
        (i + 1 < self.tracks.len()).then_some(i + 1)
    }

    /// Track `n` starts now: off the queue if it came from there.
    fn commit(&mut self, n: usize) {
        if self.queue.first() == Some(&n) {
            self.queue.remove(0);
        }
    }

    fn visible_indices(&self) -> Vec<usize> {
        match self.tab {
            SidebarTab::Queue => self.queue.clone(),
            _ => (0..self.tracks.len()).collect(),
        }
    }

    fn render(&mut self, window: &mut Window, e: &Engine) {
        let now = sex_pdx::get_ticks();
        if !self.dirty && now.saturating_sub(self.last_render_ms) < 250 {
            return;
        }
        if !window.ready() {
            return;
        }
        let (w, h) = window.size();
        if w < 120 || h < 120 {
            return;
        }
        self.dirty = false;
        self.last_render_ms = now;
        self.hits.clear();

        let pal = CandyPalette::dark();
        let buf = window.frame(w, h);
        buf.fill(pal.bg);

        let sidebar_w = if w >= 520 { 168usize } else { 0usize };
        let header_h = 46usize;
        let bar_h = 68usize;
        let content_x = sidebar_w;
        let content_w = w.saturating_sub(sidebar_w);
        let list_y = header_h;
        let list_h = h.saturating_sub(header_h + bar_h);

        // 1. Left Apple-Music / Brootlite sidebar
        if sidebar_w > 0 {
            draw_candy_panel(buf, w, h, 8, 8, (sidebar_w - 14) as i32, (h.saturating_sub(bar_h + 14)) as i32, 12, &pal);
            draw_candy_badge(buf, w, h, 18, 18, 86, 24, 0x00D6_336C, &pal);
            self.painter.draw(buf, w, h, 28, 22, 74, 14, "QUPID", 0x00FF_FFFF);

            let mut ty = 56i32;
            for tab in SidebarTab::ALL {
                let active = self.tab == tab;
                let bx = 16i32;
                let bw = (sidebar_w - 30) as i32;
                let bh = 30i32;
                draw_candy_tab(buf, w, h, bx, ty, bw, bh, active, ButtonState::Rest, &pal);
                let label = if tab == SidebarTab::Queue && !self.queue.is_empty() {
                    format!("{} ({})", tab.label(), self.queue.len())
                } else {
                    String::from(tab.label())
                };
                self.painter.draw(
                    buf,
                    w,
                    h,
                    bx + 12,
                    ty + 6,
                    bw - 20,
                    14,
                    &label,
                    if active { pal.text } else { pal.text_dim },
                );
                self.hits.push(HitBox {
                    x: bx,
                    y: ty,
                    w: bw,
                    h: bh,
                    action: HitAction::Tab(tab),
                });
                ty += 36;
            }

            // Keyboard shortcut legend at bottom of sidebar
            let hy = h.saturating_sub(bar_h + 56) as i32;
            if hy > ty + 16 {
                self.painter.draw(buf, w, h, 20, hy, (sidebar_w - 32) as i32, 11, "Enter: Play", pal.text_dim);
                self.painter.draw(buf, w, h, 20, hy + 15, (sidebar_w - 32) as i32, 11, "Space: Pause  Q: Queue", pal.text_dim);
                self.painter.draw(buf, w, h, 20, hy + 30, (sidebar_w - 32) as i32, 11, "N/P: Next/Prev", pal.text_dim);
            }
        }

        // 2. Top Header Toolbar
        draw_candy_toolbar(
            buf,
            w,
            h,
            content_x as i32 + 4,
            8,
            (content_w.saturating_sub(12)) as i32,
            (header_h - 10) as i32,
            &pal,
        );
        self.painter.draw(
            buf,
            w,
            h,
            content_x as i32 + 18,
            16,
            240,
            18,
            self.tab.label(),
            pal.text,
        );

        let albums = {
            let mut a: Vec<(&str, &str)> = self
                .tracks
                .iter()
                .map(|t| (t.artist.as_str(), t.album.as_str()))
                .collect();
            a.dedup();
            a.len()
        };
        let stats = format!("{} tracks  |  {} albums", self.tracks.len(), albums);
        self.painter.draw(
            buf,
            w,
            h,
            (w.saturating_sub(210)) as i32,
            19,
            195,
            13,
            &stats,
            pal.text_dim,
        );

        // 3. Main Library List Panel
        let panel_x = content_x as i32 + 4;
        let panel_y = list_y as i32 + 4;
        let panel_w = (content_w.saturating_sub(12)) as i32;
        let panel_h = (list_h.saturating_sub(10)) as i32;
        draw_candy_panel(buf, w, h, panel_x, panel_y, panel_w, panel_h, 12, &pal);

        let visible = self.visible_indices();
        let hearing = e.hearing();

        if visible.is_empty() {
            let cy = panel_y + (panel_h / 2).saturating_sub(42);
            let cx = panel_x + 28;
            let heading = if self.tab == SidebarTab::Queue {
                "Your Up Next queue is empty"
            } else {
                "Qupid Music & Media Library"
            };
            let hint1 = if self.tab == SidebarTab::Queue {
                "Press Q on any track in Songs to add it to Up Next."
            } else {
                "Drop .mp3 / .wav / .flac / .ogg files on Ventoy or /Music"
            };
            let hint2 = "Or run:  run qupid --library @/media/Ventoy   (or @/Music)";
            self.painter.draw(buf, w, h, cx, cy, panel_w - 56, 18, heading, pal.accent);
            self.painter.draw(buf, w, h, cx, cy + 28, panel_w - 56, 13, hint1, pal.text);
            self.painter.draw(buf, w, h, cx, cy + 50, panel_w - 56, 13, hint2, pal.text_dim);
        } else {
            let row_h = 32usize;
            let max_rows = (panel_h.max(0) as usize).saturating_sub(16) / row_h;
            let top = self
                .chosen
                .saturating_sub(max_rows / 2)
                .min(visible.len().saturating_sub(max_rows));
            let mut ry = panel_y + 8;

            for &idx in visible.iter().skip(top).take(max_rows) {
                let t = &self.tracks[idx];
                let is_sel = idx == self.chosen;
                let is_playing = hearing == Some(idx);
                let rx = panel_x + 8;
                let rw = (panel_w - 16).max(60);

                if is_sel {
                    draw_candy_selection_row(buf, w, h, rx, ry, rw, (row_h - 4) as i32, 8, &pal);
                }

                // Mini album swatch
                fill_rounded_rect(buf, w, h, rx + 10, ry + 5, 18, 18, 4, art_color(&t.album));
                let badge = if is_playing {
                    if e.paused { "||" } else { ">" }
                } else {
                    ""
                };
                if !badge.is_empty() {
                    self.painter.draw(buf, w, h, rx + 14, ry + 7, 14, 12, badge, 0x00FF_FFFF);
                }

                // Track title + artist / album
                let title_w = (rw * 45 / 100).max(100);
                let title_col = if is_playing { pal.accent } else { pal.text };
                let label = if t.number > 0 {
                    format!("{:2}. {}", t.number, t.title)
                } else {
                    t.title.clone()
                };
                self.painter.draw(buf, w, h, rx + 38, ry + 6, title_w, 14, &label, title_col);

                let meta_x = rx + 44 + title_w;
                let meta_w = (rw - title_w - 115).max(40);
                let meta = format!("{} - {}", t.artist, t.album);
                self.painter.draw(buf, w, h, meta_x, ry + 7, meta_w, 12, &meta, pal.text_dim);

                let dur = clock(t.ms);
                self.painter.draw(buf, w, h, rx + rw - 54, ry + 7, 48, 12, &dur, pal.text_dim);

                self.hits.push(HitBox {
                    x: rx,
                    y: ry,
                    w: rw,
                    h: (row_h - 4) as i32,
                    action: HitAction::Track(idx),
                });
                ry += row_h as i32;
            }
        }

        // 4. Bottom Now-Playing Transport Bar
        let ty = (h - bar_h) as i32 + 4;
        let th = (bar_h - 10) as i32;
        draw_candy_panel(buf, w, h, 8, ty, (w - 16) as i32, th, 12, &pal);

        if let Some(h_idx) = hearing.filter(|&i| i < self.tracks.len()) {
            let t = &self.tracks[h_idx];
            fill_rounded_rect(buf, w, h, 18, ty + 10, 38, 38, 8, art_color(&t.album));
            self.painter.draw(buf, w, h, 66, ty + 10, 220, 14, &t.title, pal.text);
            self.painter.draw(buf, w, h, 66, ty + 29, 220, 12, &t.artist, pal.text_dim);

            // Progress bar
            let prog_x = (w as i32 / 2).saturating_sub(110).max(296);
            let prog_w = (w as i32 - prog_x - 24).clamp(80, 260);
            let elapsed = e.elapsed_ms().min(t.ms);
            fill_rounded_rect(buf, w, h, prog_x, ty + 42, prog_w, 6, 3, pal.well);
            if t.ms > 0 {
                let fill_w = ((elapsed as u128 * prog_w as u128) / t.ms as u128) as i32;
                if fill_w > 0 {
                    fill_rounded_rect(buf, w, h, prog_x, ty + 42, fill_w.min(prog_w), 6, 3, pal.accent);
                }
            }
            let time_str = format!("{} / {}", clock(elapsed), clock(t.ms));
            self.painter.draw(buf, w, h, prog_x + prog_w - 86, ty + 24, 86, 11, &time_str, pal.text_dim);
        } else {
            fill_rounded_rect(buf, w, h, 18, ty + 10, 38, 38, 8, pal.well);
            self.painter.draw(buf, w, h, 66, ty + 18, 220, 13, "Not Playing", pal.text_dim);
        }

        // Transport Buttons (Prev / Play-Pause / Next / +Queue)
        let cx = (w as i32 / 2).saturating_sub(96).max(296);
        let btns: [(&str, i32, ButtonStyle, HitAction); 4] = [
            ("<<", 38, ButtonStyle::Subtle, HitAction::Prev),
            (
                if e.paused || hearing.is_none() { "Play" } else { "Pause" },
                64,
                ButtonStyle::Primary,
                HitAction::PlayPause,
            ),
            (">>", 38, ButtonStyle::Subtle, HitAction::Next),
            ("+Q", 38, ButtonStyle::Subtle, HitAction::QueueSelected),
        ];
        let mut bx = cx;
        for (lbl, bw, style, act) in btns {
            draw_candy_button(buf, w, h, bx, ty + 10, bw, 26, 8, style, ButtonState::Rest, &pal);
            let col = if matches!(style, ButtonStyle::Primary) {
                0x00FF_FFFF
            } else {
                pal.text
            };
            self.painter.draw(buf, w, h, bx + 10, ty + 15, bw - 12, 13, lbl, col);
            self.hits.push(HitBox {
                x: bx,
                y: ty + 10,
                w: bw,
                h: 26,
                action: act,
            });
            bx += bw + 8;
        }

        window.show_pixels(w, h);
    }
}

fn start_track(e: &mut Engine, app: &mut App, i: usize, wp: *mut Window) {
    e.stop();
    e.paused = false;
    e.sound.pause(false);
    let w = unsafe { &mut *wp };
    e.cur = open(w, &app.tracks[i]).map(|d| (i, d));
    e.starts.push((e.sound.written_total(), i));
    app.dirty = true;
    sex_pdx::serial_record!("[qupid.library.play] n={} title={}", i, app.tracks[i].title);
}

/// Runs the library view over `refs` (folders and songs); never returns.
pub fn run(window: &mut Window, refs: &[&[u8]]) -> ! {
    window.title(b"Qupid");
    let scan_refs: &[&[u8]] = if refs.is_empty() { &DEFAULT_ROOTS } else { refs };
    let tracks = library(window, scan_refs);
    sex_pdx::serial_record!("[qupid.library] tracks={}", tracks.len());
    for (i, t) in tracks.iter().enumerate().take(64) {
        sex_pdx::serial_record!(
            "[qupid.library.track] n={} artist={} album={} number={} title={} ms={}",
            i,
            t.artist,
            t.album,
            t.number,
            t.title,
            t.ms
        );
    }
    let Some(sound) = Sound::open() else {
        window.show(b"Qupid Library\n\nNo sound for this program.");
        loop {
            if let Event::Close = window.next() {
                sex_pdx::task_finish(0)
            }
        }
    };
    let mut app = App::new(tracks);
    let mut e = Engine {
        sound,
        cur: None,
        next: None,
        pcm: vec![[0i16; 2]; 4096],
        left: (0, 0),
        pic: vec![0u32; 16 * 16],
        paused: false,
        starts: Vec::new(),
    };
    let wp: *mut Window = window;
    let window = unsafe { &mut *wp };
    let start = move |e: &mut Engine, app: &mut App, i: usize| start_track(e, app, i, wp);
    if !app.tracks.is_empty() && !refs.is_empty() {
        start(&mut e, &mut app, 0);
    }
    let mut last_heard = None;
    loop {
        while let Some(ev) = window.poll() {
            match ev {
                Event::Close => {
                    e.stop();
                    e.sound.close();
                    sex_pdx::task_finish(0)
                }
                Event::Resize { .. } => {
                    app.dirty = true;
                }
                Event::Pointer(p) => match p.kind {
                    sex_pdx::app_pointer::Kind::Down { button: 0 } => {
                        let px = p.x as i32;
                        let py = p.y as i32;
                        if let Some(hit) = app.hits.iter().find(|h| h.contains(px, py)).copied() {
                            match hit.action {
                                HitAction::Tab(t) => {
                                    app.tab = t;
                                    app.dirty = true;
                                }
                                HitAction::Track(idx) => {
                                    let now = sex_pdx::get_ticks();
                                    let dbl = app
                                        .last_click_track
                                        .is_some_and(|(prev, t)| prev == idx && now.saturating_sub(t) < 500);
                                    app.chosen = idx;
                                    app.last_click_track = Some((idx, now));
                                    if dbl || e.hearing().is_none() {
                                        start(&mut e, &mut app, idx);
                                    }
                                    app.dirty = true;
                                }
                                HitAction::Prev if !app.tracks.is_empty() => {
                                    let now = e.hearing().unwrap_or(0);
                                    start(&mut e, &mut app, now.saturating_sub(1));
                                }
                                HitAction::PlayPause => {
                                    if e.cur.is_none() && !app.tracks.is_empty() {
                                        let i = app.chosen.min(app.tracks.len() - 1);
                                        start(&mut e, &mut app, i);
                                    } else {
                                        e.paused = !e.paused;
                                        e.sound.pause(e.paused);
                                        sex_pdx::serial_record!("[qupid.library.paused] {}", e.paused as u8);
                                    }
                                    app.dirty = true;
                                }
                                HitAction::Next => {
                                    let now = e.hearing().unwrap_or(0);
                                    if let Some(n) = app.after(now) {
                                        app.commit(n);
                                        start(&mut e, &mut app, n);
                                    }
                                }
                                HitAction::QueueSelected if !app.tracks.is_empty() => {
                                    app.queue.push(app.chosen);
                                    e.next = None;
                                    app.dirty = true;
                                    sex_pdx::serial_record!(
                                        "[qupid.library.queued] n={} queue={}",
                                        app.chosen,
                                        app.queue.len()
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                    sex_pdx::app_pointer::Kind::Wheel { steps } => {
                        if steps > 0 {
                            app.chosen = app.chosen.saturating_sub(1);
                            app.dirty = true;
                        } else if steps < 0 && app.chosen + 1 < app.tracks.len() {
                            app.chosen += 1;
                            app.dirty = true;
                        }
                    }
                    _ => {}
                },
                Event::Key(k) => match k.code {
                    0x0F => {
                        app.tab = app.tab.next();
                        app.dirty = true;
                    }
                    0x48 => {
                        app.chosen = app.chosen.saturating_sub(1);
                        app.dirty = true;
                    }
                    0x50 => {
                        if app.chosen + 1 < app.tracks.len() {
                            app.chosen += 1;
                            app.dirty = true;
                        }
                    }
                    0x1C if !app.tracks.is_empty() => {
                        let i = app.chosen;
                        start(&mut e, &mut app, i)
                    }
                    57 | sex_pdx::app_key::KEY_PLAY_PAUSE => {
                        if e.cur.is_none() && !app.tracks.is_empty() {
                            let i = app.chosen.min(app.tracks.len() - 1);
                            start(&mut e, &mut app, i);
                        } else {
                            e.paused = !e.paused;
                            e.sound.pause(e.paused);
                            sex_pdx::serial_record!("[qupid.library.paused] {}", e.paused as u8);
                        }
                        app.dirty = true;
                    }
                    0x31 | sex_pdx::app_key::KEY_NEXT => {
                        let now = e.hearing().unwrap_or(0);
                        if let Some(n) = app.after(now) {
                            app.commit(n);
                            start(&mut e, &mut app, n)
                        }
                    }
                    0x19 | sex_pdx::app_key::KEY_PREVIOUS if !app.tracks.is_empty() => {
                        let now = e.hearing().unwrap_or(0);
                        start(&mut e, &mut app, now.saturating_sub(1))
                    }
                    0x10 if !app.tracks.is_empty() => {
                        app.queue.push(app.chosen);
                        e.next = None;
                        app.dirty = true;
                        sex_pdx::serial_record!(
                            "[qupid.library.queued] n={} queue={}",
                            app.chosen,
                            app.queue.len()
                        );
                    }
                    sex_pdx::app_key::KEY_STOP => {
                        e.stop();
                        e.paused = true;
                        app.dirty = true;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        // Keep the ring fed, track after track.
        let mut wrote = false;
        while !e.paused && e.cur.is_some() {
            if e.left.0 == e.left.1 {
                let (i, d) = e.cur.as_mut().unwrap();
                let i = *i;
                if e.next.is_none() {
                    if let Some(n) = app.after(i) {
                        e.next = open(unsafe { &mut *wp }, &app.tracks[n]).map(|d| (n, d));
                        if e.next.is_none() {
                            sex_pdx::serial_record!("[qupid.library.skip] n={} reason=open", n);
                        }
                    }
                }
                match d.next(&mut e.pic, 16, 16, &mut e.pcm) {
                    Ok(Next::Sound { frames, .. }) => e.left = (0, frames),
                    Ok(Next::Picture { .. }) => continue,
                    Ok(Next::Later) => break,
                    Ok(Next::End) | Err(_) => {
                        e.cur = e.next.take();
                        if let Some((n, _)) = e.cur {
                            app.commit(n);
                            e.starts.push((e.sound.written_total(), n));
                            app.dirty = true;
                            sex_pdx::serial_record!(
                                "[qupid.library.next] n={} at_frame={} title={}",
                                n,
                                e.sound.written_total(),
                                app.tracks[n].title
                            );
                        } else {
                            app.dirty = true;
                            sex_pdx::serial_record!(
                                "[qupid.library.end] at_frame={}",
                                e.sound.written_total()
                            );
                        }
                        continue;
                    }
                }
            }
            let took = e.sound.write(&e.pcm[e.left.0..e.left.1]);
            e.left.0 += took;
            wrote |= took > 0;
            if took == 0 {
                break;
            }
        }
        let heard = e.hearing();
        if heard != last_heard {
            if let Some(h) = heard {
                sex_pdx::serial_record!(
                    "[qupid.library.hearing] n={} played={}",
                    h,
                    e.sound.played_total()
                );
            }
            last_heard = heard;
            app.dirty = true;
        }
        app.render(window, &e);
        if !wrote {
            let _ = sex_pdx::sys_wait_self_ticks(15);
        }
    }
}
