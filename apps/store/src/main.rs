// SPDX-License-Identifier: GPL-3.0-only
//! sexshop (formerly Store): macOS App Store Dark-Mode Noir GUI + CLI package client.
//!
//! Starts as `run +net +pkg sexshop` in Spindle for network downloads and package
//! installs. Files verifies every catalog and package Ed25519 signature + SHA-256
//! before installing anything.
//!
//! Headless CLI mode (`!words.is_empty()`) handles:
//! `store refresh`, `store search WORDS`, `store get [REPO/]NAME`, `store upgrade`,
//! `store remove NAME`, `store page [REPO/]NAME`, `store repos`, `store repo remove NAME`.
#![no_std]
#![no_main]

mod noir_ui;

extern crate alloc;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use noir_ui::{
    draw_icon, draw_monogram_tile, draw_noir_card, draw_noir_well, draw_pill_button,
    draw_sans, draw_sans_bold, draw_sans_sm, draw_mono, draw_sidebar_badge, fill_solid,
    noir_palette, Glyphs, StoreIcon, NOIR_AMBER, NOIR_INDIGO, NOIR_MINT, NOIR_PURPLE, NOIR_ROSE,
};
use sex_app::{clipboard, files, nav, net, typed, Event, Nav, Window};
use sex_pdx::app_pointer::Kind as PointerKind;
use sex_pdx::fs as w;
use sex_pdx::package as p;
use sex_pkg::index::{Asset, Listing};
use sex_theme::candy_ui::{
    draw_candy_badge, fill_rounded_rect, mix_rgb, ButtonState, ButtonStyle, CandyPalette,
};
use sha2::{Digest, Sha256};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[store.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const ENTER: u8 = 0x1C;
const ESC: u8 = 0x01;
const BACKSPACE: u8 = 0x0E;
const TAB: u8 = 0x0F;
const KEY_A: u8 = 0x1E;
const KEY_V: u8 = 0x2F;

const STATUS_LINES: usize = 2;
const CATALOG_BYTES: usize = 256 * 1024;
const SIG_BYTES: usize = 4096;
const PACKAGE_BYTES: usize = 16 * 1024 * 1024;
const PICTURE_BYTES: usize = 4 * 1024 * 1024;

const SIDEBAR_W: i32 = 228;
const HEADER_H: i32 = 50;
const FOOTER_H: i32 = 44;
const ICON_DETAIL: usize = 72;
const ICON_CARD: usize = 48;
const SHOT_W: usize = 240;
const SHOT_H: usize = 148;

fn shown(b: &[u8]) -> &str {
    core::str::from_utf8(b).unwrap_or("?")
}
fn word(status: u8) -> &'static str {
    match status {
        w::OK => "ok",
        w::DENIED => "denied",
        w::NOT_FOUND => "not_found",
        w::INVALID => "invalid",
        _ => "failed",
    }
}

/// What Files said: its status, the lines for the person, and the lines for the program.
struct Said {
    status: u8,
    lines: Vec<Vec<u8>>,
    urls: Vec<Vec<u8>>,
}

fn ask(window: &mut Window, op: u64, arg: &[u8]) -> Said {
    let (status, report) = files::package(window, op, arg, 0);
    let mut said = Said {
        status,
        lines: Vec::new(),
        urls: Vec::new(),
    };
    for line in report.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        match line.strip_prefix(&[p::MACHINE]) {
            Some(url) => said.urls.push(url.to_vec()),
            None => {
                sex_pdx::serial_record!("[store.line] {}", shown(line));
                said.lines.push(line.to_vec());
            }
        }
    }
    if status == w::DENIED && said.lines.is_empty() {
        said.lines.push(
            b"Not allowed: start sexshop with `sexshop` (run +net +pkg sexshop).".to_vec(),
        );
        sex_pdx::serial_record!("[store.line] not allowed op={}", op);
    }
    said
}

/// Downloads `url` into the file `to` in this program's folder.
fn download(window: &mut Window, url: &[u8], to: &[u8], max: usize) -> Result<(), Vec<u8>> {
    let mut buf = vec![0u8; max];
    match net::get(window, url, &mut buf) {
        Ok((200, n)) => files::write(window, to, &buf[..n]).map_err(|e| {
            format!("Could not save the download: {}", shown(w::message(e))).into_bytes()
        }),
        Ok((status, _)) => Err(format!("The store answered {} for {}.", status, shown(url)).into_bytes()),
        Err(e) => Err(net::message(e).to_vec()),
    }
}

/// The result of one job: the last status and every line said on the way.
struct Done {
    status: u8,
    lines: Vec<Vec<u8>>,
}
impl Done {
    fn new() -> Self {
        Done {
            status: w::OK,
            lines: Vec::new(),
        }
    }
    fn failed(&mut self, line: Vec<u8>) {
        sex_pdx::serial_record!("[store.line] {}", shown(&line));
        self.lines.push(line);
        self.status = w::IO;
    }
    fn take(&mut self, said: Said) -> Said {
        self.status = said.status;
        self.lines.extend(said.lines.iter().cloned());
        said
    }
}

/// Every catalog again: the main store's and each repository's, from what Files keeps.
fn refresh(window: &mut Window) -> Done {
    let mut done = Done::new();
    let source = done.take(ask(window, p::SOURCE, b""));
    if source.status != w::OK {
        return done;
    }
    let mut worst = w::OK;
    for line in &source.urls {
        let url = line.split(|b| *b == b' ').next().unwrap_or(line);
        let mut ok = true;
        for (leaf, to, max) in [
            (&b"sexos-store.txt"[..], p::CATALOG_DOWNLOAD, CATALOG_BYTES),
            (b"sexos-store.sig", p::SIG_DOWNLOAD, SIG_BYTES),
        ] {
            let mut at = url.to_vec();
            at.extend_from_slice(leaf);
            if let Err(why) = download(window, &at, to, max) {
                done.failed(why);
                ok = false;
                break;
            }
        }
        if ok {
            done.take(ask(window, p::CATALOG, line));
        }
        if done.status != w::OK {
            worst = done.status;
        }
    }
    done.status = worst;
    let (all, _) = catalog(window, b"");
    sex_pdx::serial_record!("[store.updates] count={}", all.iter().filter(|l| l.newer).count());
    done
}

/// Downloads and installs one package (`[REPO/]NAME`), or every newer one (empty).
fn get(window: &mut Window, name: &[u8]) -> Done {
    let mut done = Done::new();
    let found = done.take(ask(window, p::LOCATE, name));
    if found.status != w::OK {
        return done;
    }
    let sizes: Vec<usize> = found
        .lines
        .iter()
        .filter(|l| l.starts_with(b"Getting "))
        .map(|l| kib(l).map_or(PACKAGE_BYTES, |k| k * 1024 + 64 * 1024))
        .collect();
    for (i, url) in found.urls.iter().enumerate() {
        if let Err(why) = download(
            window,
            url,
            p::DOWNLOAD,
            sizes.get(i).copied().unwrap_or(PACKAGE_BYTES),
        ) {
            done.failed(why);
            return done;
        }
        let installed = done.take(ask(window, p::INSTALL, p::DOWNLOAD));
        if installed.status != w::OK {
            return done;
        }
    }
    done
}
fn kib(line: &[u8]) -> Option<usize> {
    let open = line.iter().rposition(|b| *b == b'(')?;
    let digits = line[open + 1..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .fold(0usize, |v, b| v * 10 + (*b - b'0') as usize);
    (digits > 0).then_some(digits)
}
fn just(said: Said) -> Done {
    Done {
        status: said.status,
        lines: said.lines,
    }
}

/// Every catalog entry matching `words`, as Files describes them.
fn catalog(window: &mut Window, words: &[u8]) -> (Vec<Listing>, Done) {
    let said = ask(window, p::DETAILS, words);
    if said.status != w::OK {
        return (Vec::new(), just(said));
    }
    let items = said
        .urls
        .iter()
        .filter_map(|l| Listing::parse(shown(l)))
        .collect();
    (items, Done::new())
}

/// A checked, decoded picture holding packed `0xAARRGGBB` pixels.
struct Pic {
    w: usize,
    h: usize,
    px: Vec<u32>,
}

/// Downloads a catalog picture, verifies its SHA-256 digest against the signed catalog,
/// decodes it, and fits it within `max_w` x `max_h`.
fn picture(
    window: &mut Window,
    base: &str,
    asset: &Asset,
    max_w: usize,
    max_h: usize,
) -> Result<Pic, &'static str> {
    let url = format!("{}{}", base, asset.file);
    let mut buf = Vec::new();
    buf.try_reserve_exact(PICTURE_BYTES + 16 * 1024)
        .map_err(|_| "no memory")?;
    buf.resize(PICTURE_BYTES + 16 * 1024, 0);
    let n = match net::get(window, url.as_bytes(), &mut buf) {
        Ok((200, n)) => n,
        Ok(_) => return Err("not served"),
        Err(_) => return Err("no network"),
    };
    buf.truncate(n);
    if Sha256::digest(&buf).as_slice() != asset.sha256 {
        return Err("hash differs from the signed catalog");
    }
    let limits = sex_image::Limits {
        max_width: 4096,
        max_height: 4096,
        max_pixels: 16_000_000,
        max_input_bytes: PICTURE_BYTES,
    };
    let image = sex_image::decode(&buf, &limits).map_err(|_| "not a picture")?;
    drop(buf);
    let small = image.fit_within(max_w, max_h).map_err(|_| "too large")?;
    Ok(Pic {
        w: small.width,
        h: small.height,
        px: small.pixels,
    })
}

fn format_hex_prefix(bytes: &[u8; 32], chars: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(chars);
    for &b in bytes {
        if out.len() >= chars {
            break;
        }
        out.push(HEX[(b >> 4) as usize] as char);
        if out.len() >= chars {
            break;
        }
        out.push(HEX[(b & 0xF) as usize] as char);
    }
    out
}

fn format_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        let mib = bytes / (1024 * 1024);
        let tenths = (bytes % (1024 * 1024)) * 10 / (1024 * 1024);
        format!("{mib}.{tenths} MiB")
    } else {
        format!("{} KiB", bytes.div_ceil(1024))
    }
}

struct Canvas {
    px: &'static mut [u32],
    w: usize,
    h: usize,
    pal: CandyPalette,
    glyphs: Glyphs,
}

impl Canvas {
    fn new(window: &Window) -> Self {
        let px = window.pixels();
        Canvas {
            px,
            w: 0,
            h: 0,
            pal: noir_palette(),
            glyphs: Glyphs::new(),
        }
    }

    fn size(&mut self, window: &Window) {
        let (w, h) = window.size();
        let w = w.clamp(320, 4096);
        let h = h.max(240).min(self.px.len() / w);
        self.w = w;
        self.h = h;
    }

    fn blit_pic(&mut self, x: i32, y: i32, pic: &Pic, radius: i32, clip_top: i32, clip_bot: i32) {
        if pic.w == 0 || pic.h == 0 || radius < 0 {
            return;
        }
        let w_i = pic.w as i32;
        let h_i = pic.h as i32;
        let r2 = radius * radius;
        for py in 0..h_i {
            let dy = y + py;
            if dy < clip_top || dy >= clip_bot || dy < 0 || dy >= self.h as i32 {
                continue;
            }
            for px in 0..w_i {
                let dx = x + px;
                if dx < 0 || dx >= self.w as i32 {
                    continue;
                }
                if radius > 0 {
                    let cx = if px < radius {
                        radius - px - 1
                    } else if px >= w_i - radius {
                        px - (w_i - radius)
                    } else {
                        0
                    };
                    let cy = if py < radius {
                        radius - py - 1
                    } else if py >= h_i - radius {
                        py - (h_i - radius)
                    } else {
                        0
                    };
                    if cx > 0 && cy > 0 && cx * cx + cy * cy > r2 {
                        continue;
                    }
                }
                let src = pic.px[py as usize * pic.w + px as usize];
                let a = (src >> 24) & 0xFF;
                if a == 0 {
                    continue;
                }
                let dst_idx = dy as usize * self.w + dx as usize;
                self.px[dst_idx] = if a == 255 {
                    src & 0x00FF_FFFF
                } else {
                    mix_rgb(self.px[dst_idx], src & 0x00FF_FFFF, a as u8)
                };
            }
        }
    }

    fn wrapped(
        &mut self,
        x: i32,
        mut y: i32,
        max_w: i32,
        s: &str,
        color: u32,
        bottom: i32,
    ) -> i32 {
        for para in s.split('\n') {
            let mut line = String::new();
            for word in para.split(' ') {
                let trial = if line.is_empty() {
                    word.to_string()
                } else {
                    format!("{line} {word}")
                };
                if self.glyphs.measure(0, 12, &trial) > max_w && !line.is_empty() {
                    if y + 18 > bottom {
                        return y;
                    }
                    self.glyphs.draw_text(self.px, self.w, self.h, x, y, max_w, 12, 0, &line, color);
                    y += 18;
                    line = word.to_string();
                } else {
                    line = trial;
                }
            }
            if y + 18 > bottom {
                return y;
            }
            self.glyphs.draw_text(self.px, self.w, self.h, x, y, max_w, 12, 0, &line, color);
            y += 18;
        }
        y
    }
}

// ---- Offline Curated SexOS Directory Preview ----

struct BuiltinApp {
    name: &'static str,
    category: &'static str,
    tab: SidebarTab,
    summary: &'static str,
}

const BUILTIN_PREVIEW: [BuiltinApp; 9] = [
    BuiltinApp { name: "Spindle", category: "System", tab: SidebarTab::System, summary: "Capability terminal, package & system CLI" },
    BuiltinApp { name: "Tatami", category: "System", tab: SidebarTab::System, summary: "macOS Noir System Settings & partition installer" },
    BuiltinApp { name: "Loom", category: "Productivity", tab: SidebarTab::Productivity, summary: "Hardware-accelerated SexOS web browser" },
    BuiltinApp { name: "Jot", category: "Productivity", tab: SidebarTab::Productivity, summary: "Rich text writer & collaborative markdown pad" },
    BuiltinApp { name: "Tailor", category: "Productivity", tab: SidebarTab::Productivity, summary: "Syntax-aware code editor & workspace tool" },
    BuiltinApp { name: "Tether", category: "Productivity", tab: SidebarTab::Productivity, summary: "Encrypted notes, tasks & system bridge" },
    BuiltinApp { name: "Kaleidoscope", category: "Creative", tab: SidebarTab::Creative, summary: "Image viewer, color inspector & canvas" },
    BuiltinApp { name: "Qupid", category: "Creative", tab: SidebarTab::Creative, summary: "Lossless music player & waveform visualizer" },
    BuiltinApp { name: "BrootLite", category: "System", tab: SidebarTab::System, summary: "Columnar file browser & volume inspector" },
];

// ---- Sidebar Navigation & Hits ----

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarTab {
    Discover,
    AllApps,
    Creative,
    Productivity,
    System,
    Updates,
    Repositories,
}

impl SidebarTab {
    const ALL: [Self; 7] = [
        Self::Discover,
        Self::AllApps,
        Self::Creative,
        Self::Productivity,
        Self::System,
        Self::Updates,
        Self::Repositories,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Discover => "Discover",
            Self::AllApps => "All Apps",
            Self::Creative => "Creative",
            Self::Productivity => "Productivity",
            Self::System => "System",
            Self::Updates => "Updates",
            Self::Repositories => "Repositories",
        }
    }

    const fn icon(self) -> StoreIcon {
        match self {
            Self::Discover => StoreIcon::Discover,
            Self::AllApps => StoreIcon::AllApps,
            Self::Creative => StoreIcon::Creative,
            Self::Productivity => StoreIcon::Productivity,
            Self::System => StoreIcon::System,
            Self::Updates => StoreIcon::Updates,
            Self::Repositories => StoreIcon::Repos,
        }
    }

    const fn color(self) -> u32 {
        match self {
            Self::Discover => NOIR_ROSE,
            Self::AllApps => NOIR_INDIGO,
            Self::Creative => NOIR_PURPLE,
            Self::Productivity => 0x003B_82F6,
            Self::System => NOIR_AMBER,
            Self::Updates => NOIR_MINT,
            Self::Repositories => 0x0064_748B,
        }
    }

    fn matches_category(self, cat: &str) -> bool {
        match self {
            Self::Discover | Self::AllApps => true,
            Self::Creative => matches!(
                cat,
                "creative" | "art" | "media" | "graphics" | "music" | "audio" | "video" | "photo" | "paint" | "games" | "shader"
            ),
            Self::Productivity => matches!(
                cat,
                "productivity" | "office" | "tools" | "dev" | "devel" | "editor" | "text" | "mail" | "browser" | "notes" | "utilities"
            ),
            Self::System => matches!(
                cat,
                "system" | "network" | "terminal" | "files" | "security" | "utils" | "settings"
            ),
            Self::Updates | Self::Repositories => true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    List,
    Page(usize),
    Repos,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    Nav(SidebarTab),
    CatPill(usize),
    FocusSearch,
    ClearFilter,
    Open(usize),
    CardInstall(usize),
    CardRemove(usize),
    Install,
    Remove,
    Back,
    Refresh,
    UpgradeAll,
    RepoRow(usize),
    RemoveRepo,
}

struct HitBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    hit: Hit,
}

impl HitBox {
    fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
}

struct View {
    all: Vec<Listing>,
    shown: Vec<usize>,
    categories: Vec<String>,
    category: usize,
    tab: SidebarTab,
    selected: usize,
    scroll_y: i32,
    scroll_to_selected: bool,
    mode: Mode,
    filter: Vec<u8>,
    typing: bool,
    search_selected: bool,
    status: Vec<Vec<u8>>,
    renders: u32,
    hits: Vec<HitBox>,
    hovered: Option<Hit>,
    pressed: Option<Hit>,
    icons: Vec<(String, Option<Pic>)>,
    shots: Vec<Pic>,
    repos: Vec<Vec<u8>>,
    repo_selected: usize,
    canvas: Canvas,
}

impl View {
    fn say(&mut self, lines: Vec<Vec<u8>>) {
        self.status.extend(lines);
        let extra = self.status.len().saturating_sub(STATUS_LINES);
        self.status.drain(..extra);
    }

    fn updates(&self) -> usize {
        self.all.iter().filter(|l| l.newer).count()
    }

    fn push_hit(&mut self, x: i32, y: i32, w: i32, h: i32, hit: Hit) {
        if w > 0 && h > 0 {
            self.hits.push(HitBox { x, y, w, h, hit });
        }
    }

    fn hit_at(&self, px: i32, py: i32) -> Option<Hit> {
        self.hits.iter().rev().find(|h| h.contains(px, py)).map(|h| h.hit)
    }

    fn btn_state(&self, hit: Hit) -> ButtonState {
        if self.pressed == Some(hit) {
            ButtonState::Pressed
        } else if self.hovered == Some(hit) {
            ButtonState::Hover
        } else {
            ButtonState::Rest
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_button_hit(
        &mut self,
        x: i32,
        y: i32,
        bw: i32,
        bh: i32,
        label: &str,
        style: ButtonStyle,
        hit: Option<Hit>,
        push: bool,
    ) {
        let st = match hit {
            Some(h) if style != ButtonStyle::Disabled => self.btn_state(h),
            _ => ButtonState::Rest,
        };
        let (w, h) = (self.canvas.w, self.canvas.h);
        let pal = self.canvas.pal;
        draw_pill_button(
            self.canvas.px,
            w,
            h,
            &mut self.canvas.glyphs,
            x,
            y,
            bw,
            bh,
            label,
            style,
            st,
            &pal,
        );
        if push {
            if let Some(h) = hit {
                self.push_hit(x, y, bw, bh, h);
            }
        }
    }

    fn reload(&mut self, window: &mut Window) {
        let (all, done) = catalog(window, &self.filter);
        self.all = all;
        if done.status != w::OK {
            self.say(done.lines);
        }
        let mut cats: Vec<String> = Vec::new();
        for l in &self.all {
            let c = l.item.category().to_string();
            if !cats.contains(&c) {
                cats.push(c);
            }
        }
        let current = self.categories.get(self.category.wrapping_sub(1)).cloned();
        self.categories = cats;
        self.category = current
            .and_then(|c| self.categories.iter().position(|x| *x == c))
            .map_or(0, |i| i + 1);
        self.pick_category();
    }

    fn pick_category(&mut self) {
        let cat_filter = if self.tab == SidebarTab::AllApps {
            self.categories.get(self.category.wrapping_sub(1)).cloned()
        } else {
            None
        };
        self.shown = (0..self.all.len())
            .filter(|i| {
                let l = &self.all[*i];
                match self.tab {
                    SidebarTab::Updates => l.newer,
                    SidebarTab::AllApps => cat_filter
                        .as_deref()
                        .is_none_or(|c| l.item.category() == c),
                    tab => tab.matches_category(l.item.category()),
                }
            })
            .collect();
        self.selected = self.selected.min(self.shown.len().saturating_sub(1));
        self.scroll_y = self.scroll_y.max(0);
        if let Mode::Page(i) = self.mode {
            if i >= self.all.len() {
                self.mode = Mode::List;
            }
        }
    }

    fn icon(&mut self, window: &mut Window, i: usize, side: usize) -> Option<usize> {
        let l = &self.all[i];
        let key = format!("{}@{}", l.full_name(), side);
        if let Some(k) = self.icons.iter().position(|(n, _)| *n == key) {
            return self.icons[k].1.as_ref().map(|_| k);
        }
        let pic = l
            .item
            .icon
            .clone()
            .and_then(|a| picture(window, &l.base.clone(), &a, side, side).ok());
        self.icons.push((key, pic));
        let k = self.icons.len() - 1;
        self.icons[k].1.as_ref().map(|_| k)
    }

    fn draw_pkg_icon(
        &mut self,
        window: &mut Window,
        i: usize,
        x: i32,
        y: i32,
        side: i32,
        clip_top: i32,
        clip_bot: i32,
    ) {
        let pal = self.canvas.pal;
        let name = self.all[i].item.name.clone();
        let cat = self.all[i].item.category().to_string();
        if let Some(ic) = self.icon(window, i, side as usize) {
            let pic = self.icons[ic].1.take().unwrap();
            let rad = (side / 4).clamp(6, 14);
            if y >= clip_top - 4 && y + side <= clip_bot + 4 {
                fill_rounded_rect(
                    self.canvas.px,
                    self.canvas.w,
                    self.canvas.h,
                    x,
                    y,
                    side,
                    side,
                    rad,
                    pal.well,
                );
            }
            let ox = x + (side - pic.w as i32) / 2;
            let oy = y + (side - pic.h as i32) / 2;
            self.canvas.blit_pic(ox, oy, &pic, rad, clip_top, clip_bot);
            self.icons[ic].1 = Some(pic);
        } else if y + side >= clip_top && y <= clip_bot {
            draw_monogram_tile(
                self.canvas.px,
                self.canvas.w,
                self.canvas.h,
                &mut self.canvas.glyphs,
                x,
                y,
                side,
                &name,
                &cat,
                &pal,
            );
        }
    }

    fn render(&mut self, window: &mut Window, working: Option<&str>) {
        self.canvas.size(window);
        let (w, h) = (self.canvas.w, self.canvas.h);
        let (wi, hi) = (w as i32, h as i32);
        let pal = self.canvas.pal;
        self.hits.clear();

        // Main `#11141D` canvas and `#0B0D12` left sidebar.
        fill_solid(self.canvas.px, w, h, 0, 0, wi, hi, pal.bg);
        fill_solid(self.canvas.px, w, h, 0, 0, SIDEBAR_W, hi, pal.panel);
        fill_solid(self.canvas.px, w, h, SIDEBAR_W, 0, 1, hi, pal.specular);

        self.render_sidebar(wi, hi);

        let body_top = HEADER_H + 10;
        let body_bot = hi - FOOTER_H;
        match self.mode {
            Mode::List => self.render_list(window, body_top, body_bot),
            Mode::Page(i) => self.render_page(window, i, body_top, body_bot),
            Mode::Repos => self.render_repos(body_top, body_bot),
        }

        // Re-mask top canvas header over any scrolled content so scrolled cards never bleed.
        self.render_canvas_header(wi);
        self.render_footer(wi, hi, working);

        window.show_pixels(w, h);
        self.renders += 1;
        if self.mode == Mode::List {
            let installed = self
                .shown
                .iter()
                .filter(|i| self.all[**i].installed.is_some())
                .count();
            let newer = self.shown.iter().filter(|i| self.all[**i].newer).count();
            sex_pdx::serial_record!(
                "[store.list] items={} installed={} newer={} selected={} render={}",
                self.shown.len(),
                installed,
                newer,
                self.selected,
                self.renders
            );
        }
    }

    fn render_sidebar(&mut self, _wi: i32, hi: i32) {
        let (w, h) = (self.canvas.w, self.canvas.h);
        let pal = self.canvas.pal;
        let sx = 12;
        let sw = SIDEBAR_W - 24;

        // App Store Brand Header.
        draw_sidebar_badge(self.canvas.px, w, h, &mut self.canvas.glyphs, sx, 13, NOIR_ROSE, StoreIcon::Discover);
        draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, sx + 28, 12, sw - 32, 15, "sexshop", pal.text);
        draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, sx + 28, 30, sw - 32, "SexOS App Store", pal.text_dim);

        // Search Input Pill (`/` or click to type).
        let sby = 52;
        let sbh = 30;
        let border = if self.typing || !self.filter.is_empty() { pal.accent } else { pal.specular };
        draw_noir_well(self.canvas.px, w, h, sx, sby, sw, sbh, 8, border, &pal);
        draw_icon(self.canvas.px, w, h, &mut self.canvas.glyphs, sx + 8, sby + 8, StoreIcon::Search, pal.text_dim);

        if self.filter.is_empty() {
            draw_sans(self.canvas.px, w, h, &mut self.canvas.glyphs, sx + 27, sby + 7, sw - 36, "Search apps (/)", pal.text_dim);
            if self.typing {
                fill_solid(self.canvas.px, w, h, sx + 26, sby + 7, 1, 16, pal.accent);
            }
            self.push_hit(sx, sby, sw, sbh, Hit::FocusSearch);
        } else {
            let q = shown(&self.filter);
            let qw = self.canvas.glyphs.measure(0, 13, q).min(sw - 52);
            if self.search_selected {
                fill_rounded_rect(self.canvas.px, w, h, sx + 25, sby + 5, qw + 4, 20, 4, pal.selection);
            }
            draw_sans(self.canvas.px, w, h, &mut self.canvas.glyphs, sx + 27, sby + 7, sw - 52, q, pal.text);
            if self.typing && !self.search_selected {
                fill_solid(self.canvas.px, w, h, sx + 28 + qw, sby + 7, 1, 16, pal.accent);
            }
            let cx = sx + sw - 22;
            fill_rounded_rect(self.canvas.px, w, h, cx, sby + 7, 16, 16, 8, pal.control);
            draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 5, sby + 7, 10, "x", pal.text_dim);
            self.push_hit(sx, sby, sw - 24, sbh, Hit::FocusSearch);
            self.push_hit(cx - 2, sby + 4, 22, 22, Hit::ClearFilter);
        }

        // Sidebar Navigation Tabs.
        let updates = self.updates();
        let mut ny = 96;
        for tab in SidebarTab::ALL {
            if ny + 30 > hi - 54 {
                break;
            }
            let active = match self.mode {
                Mode::Repos => tab == SidebarTab::Repositories,
                _ => self.tab == tab,
            };
            let hit = Hit::Nav(tab);
            let hovered = self.hovered == Some(hit);
            if active {
                fill_rounded_rect(self.canvas.px, w, h, sx, ny, sw, 28, 7, pal.selection);
                fill_rounded_rect(self.canvas.px, w, h, sx + 2, ny + 5, 3, 18, 2, pal.accent);
            } else if hovered {
                fill_rounded_rect(self.canvas.px, w, h, sx, ny, sw, 28, 7, pal.control);
            }
            draw_sidebar_badge(self.canvas.px, w, h, &mut self.canvas.glyphs, sx + 8, ny + 4, tab.color(), tab.icon());
            let label_w = if tab == SidebarTab::Updates && updates > 0 { sw - 74 } else { sw - 40 };
            draw_sans(
                self.canvas.px,
                w,
                h,
                &mut self.canvas.glyphs,
                sx + 34,
                ny + 6,
                label_w,
                tab.label(),
                if active { pal.text } else { pal.text_dim },
            );
            if tab == SidebarTab::Updates && updates > 0 {
                let badge = format!("{updates}");
                let bw = self.canvas.glyphs.measure(1, 11, &badge).max(12) + 12;
                let bx = sx + sw - bw - 6;
                draw_candy_badge(self.canvas.px, w, h, bx, ny + 5, bw, 18, pal.accent, &pal);
                draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, bx + 6, ny + 6, bw, 11, &badge, pal.text_on_accent);
            }
            self.push_hit(sx, ny, sw, 28, hit);
            ny += 32;
        }

        // Bottom Sidebar Refresh CTA.
        let rby = hi - 40;
        self.draw_button_hit(sx, rby, sw, 28, "  Refresh Catalogs", ButtonStyle::Normal, Some(Hit::Refresh), true);
        draw_icon(self.canvas.px, w, h, &mut self.canvas.glyphs, sx + 16, rby + 7, StoreIcon::Refresh, pal.text_dim);
    }

    fn render_canvas_header(&mut self, wi: i32) {
        let (w, h) = (self.canvas.w, self.canvas.h);
        let pal = self.canvas.pal;
        let cx = SIDEBAR_W + 22;
        let cw = (wi - SIDEBAR_W - 44).max(200);

        fill_solid(self.canvas.px, w, h, SIDEBAR_W + 1, 0, wi - SIDEBAR_W - 1, HEADER_H, pal.toolbar_top);
        fill_solid(self.canvas.px, w, h, SIDEBAR_W + 1, HEADER_H - 1, wi - SIDEBAR_W - 1, 1, pal.hairline);

        match self.mode {
            Mode::Page(i) => {
                self.draw_button_hit(cx, 11, 76, 28, "  Back", ButtonStyle::Normal, Some(Hit::Back), true);
                draw_icon(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 10, 18, StoreIcon::Back, pal.text);
                if i < self.all.len() {
                    let crumb = format!("{}  /  {}", self.tab.label(), self.all[i].full_name());
                    draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 88, 16, cw - 190, 15, &crumb, pal.text);
                }
            }
            Mode::Repos => {
                draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx, 15, cw - 190, 16, "Repositories & Trust Roots", pal.text);
            }
            Mode::List => {
                let heading = if !self.filter.is_empty() {
                    format!("Search: \"{}\"  ({} found)", shown(&self.filter), self.shown.len())
                } else if self.all.is_empty() {
                    format!("{}  •  Offline Preview", self.tab.label())
                } else {
                    let n = self.shown.len();
                    format!("{}  •  {} package{}", self.tab.label(), n, if n == 1 { "" } else { "s" })
                };
                draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx, 15, cw - 200, 16, &heading, pal.text);
            }
        }

        // Right header quick buttons: Refresh & Update All.
        let mut rx = wi - 18;
        let ref_w = 78;
        rx -= ref_w;
        self.draw_button_hit(rx, 11, ref_w, 28, "Refresh", ButtonStyle::Normal, Some(Hit::Refresh), true);

        let updates = self.updates();
        if updates > 0 && self.mode == Mode::List {
            let uw = 96;
            rx -= uw + 8;
            let lbl = format!("Update ({updates})");
            self.draw_button_hit(rx, 11, uw, 28, &lbl, ButtonStyle::Primary, Some(Hit::UpgradeAll), true);
        }
    }

    fn render_footer(&mut self, wi: i32, hi: i32, working: Option<&str>) {
        let (w, h) = (self.canvas.w, self.canvas.h);
        let pal = self.canvas.pal;
        let fy = hi - FOOTER_H;
        let cx = SIDEBAR_W + 22;
        let cw = (wi - SIDEBAR_W - 44).max(200);

        fill_solid(self.canvas.px, w, h, SIDEBAR_W + 1, fy, wi - SIDEBAR_W - 1, FOOTER_H, pal.panel);
        fill_solid(self.canvas.px, w, h, SIDEBAR_W + 1, fy, wi - SIDEBAR_W - 1, 1, pal.hairline);

        let status_line: String = match working {
            Some(s) => s.to_string(),
            None => self
                .status
                .last()
                .map(|l| shown(l).to_string())
                .unwrap_or_else(|| "Verified Ed25519 + SHA-256 package storefront".to_string()),
        };
        let scol = if working.is_some() { pal.accent } else { pal.text };
        draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, cx, fy + 6, cw, &status_line, scol);

        let hints = match self.mode {
            Mode::List => "Enter Open   •   R Refresh   •   U Update All   •   P Repos   •   / Search   •   Wheel Scroll",
            Mode::Page(_) => "Enter Install / Update   •   Delete Uninstall   •   Esc Back",
            Mode::Repos => "Delete Remove Repository   •   Esc Back",
        };
        draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, cx, fy + 24, cw, hints, pal.text_dim);
    }

    fn render_list(&mut self, window: &mut Window, body_top: i32, body_bot: i32) {
        let (w, h) = (self.canvas.w, self.canvas.h);
        let pal = self.canvas.pal;
        let cx = SIDEBAR_W + 22;
        let cw = (w as i32 - SIDEBAR_W - 44).max(240);
        let start_y = body_top + 6 - self.scroll_y;
        let mut y = start_y;

        // 1. Dynamic category pills when on AllApps and catalog has categories.
        if self.tab == SidebarTab::AllApps && !self.categories.is_empty() {
            let mut px = cx;
            let names: Vec<String> = core::iter::once("All".to_string())
                .chain(self.categories.iter().cloned())
                .collect();
            for (idx, name) in names.iter().enumerate() {
                let pw = self.canvas.glyphs.measure(1, 11, name) + 22;
                if px + pw > cx + cw {
                    break;
                }
                if y + 26 >= body_top && y <= body_bot {
                    let on = idx == self.category;
                    let hit = Hit::CatPill(idx);
                    self.draw_button_hit(
                        px,
                        y,
                        pw,
                        24,
                        name,
                        if on { ButtonStyle::Primary } else { ButtonStyle::Normal },
                        Some(hit),
                        true,
                    );
                }
                px += pw + 8;
            }
            y += 34;
        }

        // 2. Updates header banner when in Updates tab.
        if self.tab == SidebarTab::Updates {
            let uh = 66;
            if y + uh >= body_top && y <= body_bot {
                draw_noir_card(self.canvas.px, w, h, cx, y, cw, uh, 10, &pal, None);
                let updates = self.updates();
                let title = if updates == 0 {
                    "Everything Is Up to Date".to_string()
                } else {
                    format!("{updates} Pending Package Update{}", if updates == 1 { "" } else { "s" })
                };
                draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 16, y + 12, cw - 230, 15, &title, pal.text);
                draw_sans_sm(
                    self.canvas.px,
                    w,
                    h,
                    &mut self.canvas.glyphs,
                    cx + 16,
                    y + 36,
                    cw - 230,
                    "Upgrades verify package signatures cleanly and keep your previous version for rollback.",
                    pal.text_dim,
                );
                if updates > 0 {
                    let push = y + 18 >= body_top && y + 48 <= body_bot;
                    self.draw_button_hit(cx + cw - 118, y + 18, 102, 30, "Update All", ButtonStyle::Primary, Some(Hit::UpgradeAll), push);
                } else {
                    let push = y + 18 >= body_top && y + 48 <= body_bot;
                    self.draw_button_hit(cx + cw - 128, y + 18, 112, 30, "Check Again", ButtonStyle::Normal, Some(Hit::Refresh), push);
                }
            }
            y += uh + 12;
        }

        // 3. Offline / Unrefreshed Catalog State vs Live Storefront Grid.
        if self.all.is_empty() && self.filter.is_empty() && self.tab != SidebarTab::Updates {
            y += self.render_offline_empty(cx, y, cw, body_top, body_bot);
        } else if self.shown.is_empty() {
            if self.tab != SidebarTab::Updates {
                let eh = 96;
                if y + eh >= body_top && y <= body_bot {
                    draw_noir_card(self.canvas.px, w, h, cx, y, cw, eh, 10, &pal, None);
                    if !self.filter.is_empty() {
                        let msg = format!("No packages match \"{}\"", shown(&self.filter));
                        draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 18, y + 16, cw - 36, 15, &msg, pal.text);
                        draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 18, y + 40, cw - 36, "Try another search word or clear the filter to browse all packages.", pal.text_dim);
                        self.draw_button_hit(cx + 18, y + 60, 104, 24, "Clear Search", ButtonStyle::Primary, Some(Hit::ClearFilter), true);
                    } else {
                        draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 18, y + 16, cw - 36, 15, "No packages in this category", pal.text);
                        draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 18, y + 40, cw - 36, "Browse All Apps or refresh catalogs from your trusted repositories.", pal.text_dim);
                        self.draw_button_hit(cx + 18, y + 60, 108, 24, "Browse All Apps", ButtonStyle::Normal, Some(Hit::Nav(SidebarTab::AllApps)), true);
                    }
                }
                y += eh + 12;
            }
        } else {
            // Featured Hero Banner on Discover when not filtering.
            if self.tab == SidebarTab::Discover && self.filter.is_empty() {
                let hero_idx = self
                    .shown
                    .iter()
                    .copied()
                    .find(|&idx| self.all[idx].item.featured)
                    .unwrap_or(self.shown[0]);
                y += self.render_featured_hero(window, hero_idx, cx, y, cw, body_top, body_bot) + 14;
            }

            let grid_title = if self.tab == SidebarTab::Discover && self.filter.is_empty() {
                "Catalog Applications"
            } else {
                "Packages"
            };
            if y + 18 >= body_top && y <= body_bot {
                draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 2, y, cw, 12, grid_title, pal.text_dim);
            }
            y += 22;

            // 2-Column or 3-Column Noir App Cards Grid.
            let cols: i32 = if cw >= 860 { 3 } else if cw >= 460 { 2 } else { 1 };
            let gap = 12;
            let card_w = (cw - gap * (cols - 1)) / cols;
            let card_h = 124;

            if self.scroll_to_selected {
                self.scroll_to_selected = false;
                let sel_row = (self.selected as i32) / cols;
                let sel_top = y + sel_row * (card_h + gap);
                if sel_top < body_top {
                    let delta = body_top - sel_top;
                    self.scroll_y = (self.scroll_y - delta).max(0);
                    y += delta;
                } else if sel_top + card_h > body_bot {
                    let delta = sel_top + card_h - body_bot;
                    self.scroll_y += delta;
                    y -= delta;
                }
            }

            for (pos, idx) in self.shown.clone().into_iter().enumerate() {
                let col = (pos as i32) % cols;
                let row = (pos as i32) / cols;
                let card_x = cx + col * (card_w + gap);
                let card_y = y + row * (card_h + gap);
                if card_y + card_h >= body_top && card_y <= body_bot {
                    self.render_app_card(window, idx, pos == self.selected, card_x, card_y, card_w, card_h, body_top, body_bot);
                }
            }
            let total_rows = ((self.shown.len() as i32) + cols - 1) / cols;
            y += total_rows * (card_h + gap);
        }

        let total_h = (y - start_y).max(0);
        let view_h = (body_bot - body_top).max(80);
        let max_scroll = (total_h - view_h).max(0);
        if self.scroll_y > max_scroll {
            self.scroll_y = max_scroll;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_featured_hero(
        &mut self,
        window: &mut Window,
        i: usize,
        x: i32,
        y: i32,
        cw: i32,
        body_top: i32,
        body_bot: i32,
    ) -> i32 {
        let hero_h = 132;
        if y + hero_h < body_top || y > body_bot {
            return hero_h;
        }
        let (w, h) = (self.canvas.w, self.canvas.h);
        let pal = self.canvas.pal;
        let l = self.all[i].clone();

        let ohit = Hit::Open(i);
        let border = if self.hovered == Some(ohit) { Some(pal.accent) } else { None };
        draw_noir_card(self.canvas.px, w, h, x, y, cw, hero_h, 12, &pal, border);
        fill_rounded_rect(self.canvas.px, w, h, x + 2, y + 10, 4, hero_h - 20, 2, NOIR_ROSE);

        // Icon tile (68x68).
        self.draw_pkg_icon(window, i, x + 20, y + 32, 68, body_top, body_bot);

        // Featured pill + Verified badge.
        let tx = x + 104;
        let tw = (cw - 240).max(120);
        draw_candy_badge(self.canvas.px, w, h, tx, y + 14, 78, 18, NOIR_ROSE, &pal);
        draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, tx + 10, y + 16, 64, 10, "FEATURED", 0x00FF_FFFF);

        let repo_lbl = if l.repo == sex_pkg::repo::MAIN { "Main Store".to_string() } else { l.repo.clone() };
        let meta = format!("{}  •  {}  •  SHA-256 Verified", l.item.category(), repo_lbl);
        draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, tx + 88, y + 16, (tw - 88).max(40), &meta, NOIR_MINT);

        let title = format!("{}  v{}", l.full_name(), l.item.version);
        draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, tx, y + 39, tw, 17, &title, pal.text);
        draw_sans(self.canvas.px, w, h, &mut self.canvas.glyphs, tx, y + 66, tw, &l.item.summary, pal.text_dim);

        let sub = format!("Package Size: {}   •   Grants: {}", format_size(l.item.bytes), sex_pkg::index::grant_words(l.item.grants));
        draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, tx, y + 92, tw, &sub, pal.text_dim);

        if y >= body_top && y + hero_h <= body_bot {
            self.push_hit(x, y, cw, hero_h, ohit);
        }

        // Right CTA button (`GET` / `UPDATE` / `REMOVE`).
        let (btn_lbl, bstyle, bhit) = match (l.installed.is_some(), l.newer) {
            (_, true) => ("UPDATE", ButtonStyle::Primary, Hit::CardInstall(i)),
            (false, false) => ("GET", ButtonStyle::Primary, Hit::CardInstall(i)),
            (true, false) => ("REMOVE", ButtonStyle::Danger, Hit::CardRemove(i)),
        };
        let bx = x + cw - 110;
        let by = y + (hero_h - 30) / 2;
        let push = by >= body_top && by + 30 <= body_bot;
        self.draw_button_hit(bx, by, 92, 30, btn_lbl, bstyle, Some(bhit), push);
        hero_h
    }

    #[allow(clippy::too_many_arguments)]
    fn render_app_card(
        &mut self,
        window: &mut Window,
        i: usize,
        selected: bool,
        x: i32,
        y: i32,
        cw: i32,
        ch: i32,
        body_top: i32,
        body_bot: i32,
    ) {
        let (w, h) = (self.canvas.w, self.canvas.h);
        let pal = self.canvas.pal;
        let l = self.all[i].clone();

        let ohit = Hit::Open(i);
        let border = if selected {
            Some(pal.accent)
        } else if self.hovered == Some(ohit) {
            Some(pal.specular)
        } else {
            None
        };
        draw_noir_card(self.canvas.px, w, h, x, y, cw, ch, 10, &pal, border);
        if selected {
            fill_rounded_rect(self.canvas.px, w, h, x + 2, y + 12, 3, ch - 24, 2, pal.accent);
        }

        self.draw_pkg_icon(window, i, x + 14, y + 14, ICON_CARD as i32, body_top, body_bot);

        let tx = x + 72;
        let tw = (cw - 86).max(60);
        draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, tx, y + 14, tw, 14, &l.full_name(), pal.text);

        let sub = format!("v{}  •  {}", l.item.version, l.item.category());
        draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, tx, y + 34, tw, &sub, pal.text_dim);

        draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, x + 14, y + 70, cw - 28, &l.item.summary, pal.text_dim);

        // Bottom row: Verified size badge on left + crisp GET / UPDATE / REMOVE pill on right.
        let mut meta = format_size(l.item.bytes);
        if l.item.grants != 0 {
            meta.push_str("  •  ");
            meta.push_str(&sex_pkg::index::grant_words(l.item.grants));
        }
        draw_icon(self.canvas.px, w, h, &mut self.canvas.glyphs, x + 14, y + ch - 28, StoreIcon::Shield, NOIR_MINT);
        draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, x + 31, y + ch - 27, (cw - 128).max(40), &meta, NOIR_MINT);

        if y + ch > body_top && y < body_bot {
            self.push_hit(x, y, cw, ch, ohit);
        }

        let (btn_lbl, bstyle, bhit) = match (l.installed.is_some(), l.newer) {
            (_, true) => ("UPDATE", ButtonStyle::Primary, Hit::CardInstall(i)),
            (false, false) => ("GET", ButtonStyle::Primary, Hit::CardInstall(i)),
            (true, false) => ("REMOVE", ButtonStyle::Danger, Hit::CardRemove(i)),
        };
        let bw = 74;
        let bx = x + cw - bw - 12;
        let by = y + ch - 34;
        let push = by >= body_top && by + 24 <= body_bot;
        self.draw_button_hit(bx, by, bw, 24, btn_lbl, bstyle, Some(bhit), push);
    }

    fn render_offline_empty(&mut self, x: i32, mut y: i32, cw: i32, body_top: i32, body_bot: i32) -> i32 {
        let start_y = y;
        let (w, h) = (self.canvas.w, self.canvas.h);
        let pal = self.canvas.pal;

        // 1. Sleek Empty-State Catalog Card with `[Refresh Catalogs]` CTA.
        let card_h = 104;
        if y + card_h >= body_top && y <= body_bot {
            draw_noir_card(self.canvas.px, w, h, x, y, cw, card_h, 12, &pal, None);
            fill_rounded_rect(self.canvas.px, w, h, x + 2, y + 10, 4, card_h - 20, 2, NOIR_ROSE);
            draw_sidebar_badge(self.canvas.px, w, h, &mut self.canvas.glyphs, x + 18, y + 18, NOIR_ROSE, StoreIcon::Discover);
            draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, x + 48, y + 16, cw - 200, 15, "No Catalogs Cached Yet", pal.text);
            draw_sans_sm(
                self.canvas.px,
                w,
                h,
                &mut self.canvas.glyphs,
                x + 48,
                y + 40,
                cw - 190,
                "Refresh signed catalogs once connected to a network, or configure a source in Spindle (`pkg refresh URL`).",
                pal.text_dim,
            );
            draw_sans_sm(
                self.canvas.px,
                w,
                h,
                &mut self.canvas.glyphs,
                x + 48,
                y + 58,
                cw - 190,
                "All pre-installed desktop applications below remain ready to launch offline.",
                NOIR_MINT,
            );
            let push = y + 36 >= body_top && y + 68 <= body_bot;
            self.draw_button_hit(x + cw - 146, y + 36, 130, 32, "Refresh Catalogs", ButtonStyle::Primary, Some(Hit::Refresh), push);
        }
        y += card_h + 16;

        // 2. Curated Built-in SexOS App Directory Preview Cards.
        if y + 18 >= body_top && y <= body_bot {
            draw_sans_bold(
                self.canvas.px,
                w,
                h,
                &mut self.canvas.glyphs,
                x + 2,
                y,
                cw,
                12,
                "Built-In SexOS App Directory (Pre-Installed on System)",
                pal.text_dim,
            );
        }
        y += 22;

        let cols: i32 = if cw >= 860 { 3 } else if cw >= 460 { 2 } else { 1 };
        let gap = 12;
        let card_w = (cw - gap * (cols - 1)) / cols;
        let ch = 96;

        let items: Vec<&BuiltinApp> = BUILTIN_PREVIEW
            .iter()
            .filter(|a| matches!(self.tab, SidebarTab::Discover | SidebarTab::AllApps) || a.tab == self.tab)
            .collect();

        for (pos, app) in items.iter().enumerate() {
            let col = (pos as i32) % cols;
            let row = (pos as i32) / cols;
            let cx = x + col * (card_w + gap);
            let cy = y + row * (ch + gap);
            if cy + ch >= body_top && cy <= body_bot {
                draw_noir_card(self.canvas.px, w, h, cx, cy, card_w, ch, 10, &pal, None);
                draw_monogram_tile(
                    self.canvas.px,
                    w,
                    h,
                    &mut self.canvas.glyphs,
                    cx + 14,
                    cy + 14,
                    44,
                    app.name,
                    &app.category.to_ascii_lowercase(),
                    &pal,
                );
                draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 68, cy + 14, card_w - 150, 14, app.name, pal.text);
                draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 68, cy + 33, card_w - 82, app.category, pal.text_dim);
                draw_candy_badge(self.canvas.px, w, h, cx + card_w - 72, cy + 12, 58, 18, pal.control, &pal);
                draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + card_w - 64, cy + 14, 48, 10, "BUILT-IN", NOIR_MINT);
                draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 14, cy + 66, card_w - 28, app.summary, pal.text_dim);
            }
        }
        let rows = ((items.len() as i32) + cols - 1) / cols;
        y += rows * (ch + gap);
        y - start_y
    }

    fn render_page(&mut self, window: &mut Window, i: usize, body_top: i32, body_bot: i32) {
        if i >= self.all.len() {
            self.mode = Mode::List;
            return;
        }
        let (w, h) = (self.canvas.w, self.canvas.h);
        let pal = self.canvas.pal;
        let cx = SIDEBAR_W + 22;
        let cw = (w as i32 - SIDEBAR_W - 44).max(240);
        let l = self.all[i].clone();
        let start_y = body_top + 6 - self.scroll_y;
        let mut y = start_y;

        // 1. Package Hero Card.
        let hero_h = 148;
        if y + hero_h >= body_top && y <= body_bot {
            draw_noir_card(self.canvas.px, w, h, cx, y, cw, hero_h, 12, &pal, None);
            self.draw_pkg_icon(window, i, cx + 18, y + 18, ICON_DETAIL as i32, body_top, body_bot);

            let tx = cx + 104;
            let tw = (cw - 120).max(120);
            let title = format!("{}  v{}", l.full_name(), l.item.version);
            draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, tx, y + 16, tw, 18, &title, pal.text);
            draw_sans(self.canvas.px, w, h, &mut self.canvas.glyphs, tx, y + 40, tw, &l.item.summary, pal.text_dim);

            let from = if l.repo == sex_pkg::repo::MAIN { "Main Store".to_string() } else { l.repo.clone() };
            let meta = format!("{}  •  {}  •  Repository: {}", l.item.category(), format_size(l.item.bytes), from);
            draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, tx, y + 62, tw, &meta, pal.text_dim);

            // Verified SHA-256 Pill Badge.
            let sha_short = format_hex_prefix(&l.item.sha256, 12);
            let sha_lbl = format!("SHA-256 Verified  •  {sha_short}");
            let sbw = self.canvas.glyphs.measure(2, 11, &sha_lbl) + 30;
            draw_candy_badge(self.canvas.px, w, h, tx, y + 82, sbw, 20, mix_rgb(pal.well, NOIR_MINT, 38), &pal);
            draw_icon(self.canvas.px, w, h, &mut self.canvas.glyphs, tx + 6, y + 85, StoreIcon::Shield, NOIR_MINT);
            draw_mono(self.canvas.px, w, h, &mut self.canvas.glyphs, tx + 24, y + 85, sbw - 28, &sha_lbl, NOIR_MINT);

            // Install / Update / Uninstall buttons.
            let mut bx = tx;
            let by = y + 110;
            let (lbl, pstyle, can_install) = match (l.installed, l.newer) {
                (None, _) => ("Install", ButtonStyle::Primary, true),
                (Some(_), true) => ("Update", ButtonStyle::Primary, true),
                (Some(_), false) => ("Installed", ButtonStyle::Disabled, false),
            };
            let bw = 96;
            let push_inst = can_install && by >= body_top && by + 28 <= body_bot;
            self.draw_button_hit(bx, by, bw, 28, lbl, pstyle, can_install.then_some(Hit::Install), push_inst);
            bx += bw + 10;

            if l.installed.is_some() {
                let rw = 96;
                let push_rm = by >= body_top && by + 28 <= body_bot;
                self.draw_button_hit(bx, by, rw, 28, "Uninstall", ButtonStyle::Danger, Some(Hit::Remove), push_rm);
                bx += rw + 10;
            }
            if let Some(prev) = l.previous {
                let rb = format!("Previous {prev} kept  •  pkg rollback {} in Spindle", l.item.name);
                draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, bx, by + 7, (cx + cw - bx - 12).max(40), &rb, pal.text_dim);
            }
        }
        y += hero_h + 12;

        // 2. Permissions & Grants Card.
        let perm_h = 64;
        if y + perm_h >= body_top && y <= body_bot {
            draw_noir_card(self.canvas.px, w, h, cx, y, cw, perm_h, 10, &pal, None);
            draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 16, y + 10, cw - 32, 12, "Permissions & Privacy Grants", pal.text);
            let mut gx = cx + 16;
            if l.item.grants == 0 {
                draw_candy_badge(self.canvas.px, w, h, gx, y + 32, 152, 20, mix_rgb(pal.well, NOIR_MINT, 34), &pal);
                draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, gx + 10, y + 35, 136, 11, "No Extra Grants", NOIR_MINT);
            } else {
                for &(bit, name) in &sex_pkg::GRANTS {
                    if l.item.grants & bit != 0 {
                        let pw = self.canvas.glyphs.measure(1, 11, name) + 24;
                        draw_candy_badge(self.canvas.px, w, h, gx, y + 32, pw, 20, mix_rgb(pal.well, NOIR_AMBER, 44), &pal);
                        draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, gx + 12, y + 35, pw - 16, 11, name, NOIR_AMBER);
                        gx += pw + 8;
                    }
                }
                draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, gx + 6, y + 36, (cx + cw - gx - 18).max(40), "Confirmed by you on first run & manageable in Tatami Privacy", pal.text_dim);
            }
        }
        y += perm_h + 12;

        // 3. Screenshots Gallery Card.
        let shot_card_h = if self.shots.is_empty() { 70 } else { SHOT_H as i32 + 46 };
        if y + shot_card_h >= body_top && y <= body_bot {
            draw_noir_card(self.canvas.px, w, h, cx, y, cw, shot_card_h, 10, &pal, None);
            draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 16, y + 10, cw - 32, 12, "Screenshots Preview", pal.text);
            if self.shots.is_empty() {
                let msg = if l.item.screenshots.is_empty() {
                    "This package does not include catalog preview images."
                } else {
                    "Preview images could not be downloaded or failed SHA-256 verification."
                };
                draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 16, y + 36, cw - 32, msg, pal.text_dim);
            } else {
                let mut sx = cx + 16;
                for pic in core::mem::take(&mut self.shots).into_iter() {
                    if sx + pic.w as i32 <= cx + cw - 12 {
                        fill_rounded_rect(self.canvas.px, w, h, sx - 1, y + 33, pic.w as i32 + 2, pic.h as i32 + 2, 8, pal.specular);
                        self.canvas.blit_pic(sx, y + 34, &pic, 7, body_top, body_bot);
                    }
                    sx += pic.w as i32 + 12;
                    self.shots.push(pic);
                }
            }
        }
        y += shot_card_h + 12;

        // 4. Full Description & Integrity Card.
        let desc_text = if l.item.description.is_empty() {
            l.item.summary.as_str()
        } else {
            l.item.description.as_str()
        };
        let desc_lines = desc_text.lines().count().max(2) as i32 + 2;
        let desc_h = (desc_lines * 18 + 54).clamp(90, 260);
        if y + desc_h >= body_top && y <= body_bot {
            draw_noir_card(self.canvas.px, w, h, cx, y, cw, desc_h, 10, &pal, None);
            draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 16, y + 10, cw - 32, 12, "About & Cryptographic Digest", pal.text);
            let next_y = self.canvas.wrapped(cx + 16, y + 32, cw - 32, desc_text, pal.text, y + desc_h - 26);
            let full_sha = format!("SHA-256: {}", format_hex_prefix(&l.item.sha256, 64));
            draw_mono(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 16, (next_y + 4).min(y + desc_h - 22), cw - 32, &full_sha, pal.text_dim);
        }
        y += desc_h + 12;

        let total_h = (y - start_y).max(0);
        let view_h = (body_bot - body_top).max(80);
        let max_scroll = (total_h - view_h).max(0);
        if self.scroll_y > max_scroll {
            self.scroll_y = max_scroll;
        }
    }

    fn render_repos(&mut self, body_top: i32, body_bot: i32) {
        let (w, h) = (self.canvas.w, self.canvas.h);
        let pal = self.canvas.pal;
        let cx = SIDEBAR_W + 22;
        let cw = (w as i32 - SIDEBAR_W - 44).max(240);
        let start_y = body_top + 6 - self.scroll_y;
        let mut y = start_y;

        // Guidance card.
        let gh = 78;
        if y + gh >= body_top && y <= body_bot {
            draw_noir_card(self.canvas.px, w, h, cx, y, cw, gh, 10, &pal, None);
            draw_sans_bold(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 16, y + 12, cw - 32, 13, "Adding Third-Party Repositories", pal.text);
            draw_sans_sm(
                self.canvas.px,
                w,
                h,
                &mut self.canvas.glyphs,
                cx + 16,
                y + 34,
                cw - 32,
                "Repository publisher trust is configured explicitly in Spindle:",
                pal.text_dim,
            );
            draw_mono(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 16, y + 52, cw - 32, "pkg repo add NAME URL ED25519_PUBLIC_KEY", pal.accent);
        }
        y += gh + 12;

        let repos = self.repos.clone();
        if repos.is_empty() {
            draw_noir_card(self.canvas.px, w, h, cx, y, cw, 64, 10, &pal, None);
            draw_sans_sm(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 16, y + 22, cw - 32, "No repositories listed yet.", pal.text_dim);
            return;
        }

        for (k, line) in repos.iter().enumerate() {
            let rh = 44;
            if y + rh >= body_top && y <= body_bot {
                let sel = k == self.repo_selected;
                let rhit = Hit::RepoRow(k);
                let border = if sel { Some(pal.accent) } else { None };
                draw_noir_card(self.canvas.px, w, h, cx, y, cw, rh, 9, &pal, border);
                if sel {
                    fill_rounded_rect(self.canvas.px, w, h, cx + 1, y + 1, cw - 2, rh - 2, 8, pal.selection);
                    fill_rounded_rect(self.canvas.px, w, h, cx + 2, y + 8, 3, rh - 16, 2, pal.accent);
                }
                draw_sans(self.canvas.px, w, h, &mut self.canvas.glyphs, cx + 16, y + 13, cw - 150, shown(line), pal.text);
                self.push_hit(cx, y, cw - 130, rh, rhit);

                let name = line.split(|b| *b == b' ').next().unwrap_or(b"");
                if name != b"main" && !name.is_empty() && sel {
                    self.draw_button_hit(cx + cw - 110, y + 10, 96, 24, "Remove", ButtonStyle::Danger, Some(Hit::RemoveRepo), true);
                }
            }
            y += rh + 8;
        }
    }

    fn job(&mut self, window: &mut Window, what: &str, run: impl FnOnce(&mut Window) -> Done) {
        self.render(window, Some(what));
        let done = run(window);
        self.say(done.lines);
        self.reload(window);
        self.render(window, None);
    }

    fn open_page(&mut self, window: &mut Window, i: usize) {
        self.mode = Mode::Page(i);
        self.scroll_y = 0;
        self.render(window, Some("Verifying package pictures..."));
        let l = self.all[i].clone();
        self.shots.clear();
        let mut bad = 0;
        for a in &l.item.screenshots {
            match picture(window, &l.base, a, SHOT_W, SHOT_H) {
                Ok(pic) => self.shots.push(pic),
                Err(why) => {
                    bad += 1;
                    self.say(vec![format!("{}: {}", a.file, why).into_bytes()]);
                }
            }
        }
        sex_pdx::serial_record!(
            "[store.page] name={} repo={} shots={}/{} bad={}",
            l.item.name,
            l.repo,
            self.shots.len(),
            l.item.screenshots.len(),
            bad
        );
        self.render(window, None);
    }

    fn load_repos(&mut self, window: &mut Window) {
        let said = ask(window, p::REPOS, b"");
        self.repos = said.lines;
        self.repo_selected = self.repo_selected.min(self.repos.len().saturating_sub(1));
    }

    fn install(&mut self, window: &mut Window, i: usize) {
        if i >= self.all.len() {
            return;
        }
        let name = self.all[i].full_name();
        self.job(window, &format!("Downloading {name}..."), |win| get(win, name.as_bytes()));
        self.repage(&name);
    }

    fn remove(&mut self, window: &mut Window, i: usize) {
        if i >= self.all.len() {
            return;
        }
        let name = self.all[i].item.name.clone();
        let full = self.all[i].full_name();
        self.job(window, &format!("Removing {name}..."), |win| {
            just(ask(win, p::REMOVE, name.as_bytes()))
        });
        self.repage(&full);
    }

    fn repage(&mut self, full: &str) {
        if let Mode::Page(_) = self.mode {
            self.mode = self
                .all
                .iter()
                .position(|l| l.full_name() == full)
                .map_or(Mode::List, Mode::Page);
        }
    }

    fn act(&mut self, window: &mut Window, hit: Hit) {
        match hit {
            Hit::FocusSearch => {
                if self.typing && !self.filter.is_empty() {
                    self.search_selected = !self.search_selected;
                } else {
                    self.typing = true;
                    self.search_selected = false;
                }
            }
            Hit::ClearFilter => {
                self.filter.clear();
                self.search_selected = false;
                self.typing = true;
                self.selected = 0;
                self.scroll_y = 0;
                self.reload(window);
            }
            Hit::Nav(tab) => {
                self.tab = tab;
                self.selected = 0;
                self.scroll_y = 0;
                if tab == SidebarTab::Repositories {
                    self.load_repos(window);
                    self.mode = Mode::Repos;
                } else {
                    self.mode = Mode::List;
                    self.pick_category();
                }
            }
            Hit::CatPill(k) => {
                self.category = k;
                self.selected = 0;
                self.scroll_y = 0;
                self.pick_category();
            }
            Hit::Open(i) => {
                if let Some(k) = self.shown.iter().position(|x| *x == i) {
                    self.selected = k;
                }
                self.open_page(window, i);
                return;
            }
            Hit::CardInstall(i) => {
                self.install(window, i);
                return;
            }
            Hit::CardRemove(i) => {
                self.remove(window, i);
                return;
            }
            Hit::Install => {
                if let Mode::Page(i) = self.mode {
                    self.install(window, i);
                    return;
                }
            }
            Hit::Remove => {
                if let Mode::Page(i) = self.mode {
                    self.remove(window, i);
                    return;
                }
            }
            Hit::Back => {
                self.mode = Mode::List;
                self.scroll_y = 0;
            }
            Hit::Refresh => {
                self.job(window, "Downloading catalogs...", refresh);
                return;
            }
            Hit::UpgradeAll => {
                self.job(window, "Upgrading packages...", |win| get(win, b""));
                return;
            }
            Hit::RepoRow(k) => {
                self.repo_selected = k;
            }
            Hit::RemoveRepo => {
                self.remove_repo(window);
                return;
            }
        }
        self.render(window, None);
    }

    fn remove_repo(&mut self, window: &mut Window) {
        let Some(line) = self.repos.get(self.repo_selected) else { return };
        let name = line.split(|b| *b == b' ').next().unwrap_or(b"").to_vec();
        if name == b"main" || name.is_empty() {
            self.say(vec![
                b"The main store is set in Spindle: pkg refresh URL".to_vec(),
            ]);
            self.render(window, None);
            return;
        }
        self.job(
            window,
            &format!("Removing repository {}...", shown(&name)),
            |win| just(ask(win, p::REPO_REMOVE, &name)),
        );
        self.load_repos(window);
        self.mode = Mode::Repos;
        self.render(window, None);
    }
}

/// The run's own words: what Spindle added (`+packages`) and given paths are not commands.
fn command_words(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for word in raw
        .split(|b| *b == b' ')
        .filter(|w| !w.is_empty() && *w != p::GRANT_TOKEN && !w.starts_with(b"@"))
    {
        if !out.is_empty() {
            out.push(b' ');
        }
        out.extend_from_slice(word);
    }
    out
}

/// `store page NAME`: the page, its pictures checked and drawn, for scripts and gates.
fn page(window: &mut Window, name: &[u8]) -> Done {
    let (all, done) = catalog(window, b"");
    if done.status != w::OK {
        return done;
    }
    let name = shown(name);
    let Some(i) = all
        .iter()
        .position(|l| l.full_name() == name)
        .or_else(|| all.iter().position(|l| l.item.name == name))
    else {
        let mut d = Done::new();
        d.status = w::NOT_FOUND;
        d.lines.push(format!("No package {name} in the catalogs.").into_bytes());
        return d;
    };
    let l = all[i].clone();
    let icon = match &l.item.icon {
        None => "none",
        Some(a) => match picture(window, &l.base, a, 64, 64) {
            Ok(_) => "ok",
            Err(why) => why,
        },
    };
    let mut ok = 0;
    for a in &l.item.screenshots {
        if picture(window, &l.base, a, SHOT_W, SHOT_H).is_ok() {
            ok += 1;
        }
    }
    sex_pdx::serial_record!(
        "[store.page] name={} repo={} version={} bytes={} grants={} category={} featured={} icon={} shots={}/{} description={} installed={} previous={}",
        l.item.name,
        l.repo,
        l.item.version,
        l.item.bytes,
        sex_pkg::index::grant_words(l.item.grants),
        l.item.category(),
        l.item.featured as u8,
        icon,
        ok,
        l.item.screenshots.len(),
        l.item.description.len(),
        l.installed.map(|v| v.to_string()).unwrap_or_else(|| "-".into()),
        l.previous.map(|v| v.to_string()).unwrap_or_else(|| "-".into())
    );
    let mut d = Done::new();
    d.lines.push(
        format!(
            "{} {} from {}: {} KiB, asks for {}",
            l.full_name(),
            l.item.version,
            l.repo,
            l.item.bytes.div_ceil(1024),
            sex_pkg::index::grant_words(l.item.grants)
        )
        .into_bytes(),
    );
    if !l.item.description.is_empty() {
        d.lines.push(l.item.description.replace('\n', " ").into_bytes());
    }
    if icon != "ok" && icon != "none" {
        d.status = w::INVALID;
    }
    if ok != l.item.screenshots.len() {
        d.status = w::INVALID;
    }
    d
}

fn command(mut window: Window, line: &[u8]) -> ! {
    let window = &mut window;
    let mut it = line.splitn(2, |b| *b == b' ');
    let (op, rest) = (it.next().unwrap_or(b""), it.next().unwrap_or(b""));
    let done = match op {
        b"refresh" => refresh(window),
        b"search" => just(ask(window, p::SEARCH, rest)),
        b"get" | b"install" if !rest.is_empty() => get(window, rest),
        b"upgrade" => get(window, b""),
        b"remove" if !rest.is_empty() => just(ask(window, p::REMOVE, rest)),
        b"page" if !rest.is_empty() => page(window, rest),
        b"repos" => just(ask(window, p::REPOS, b"")),
        b"repo" if rest.starts_with(b"remove ") => {
            just(ask(window, p::REPO_REMOVE, rest[7..].trim_ascii()))
        }
        b"repo" | b"rollback" => {
            let line = if op == b"repo" {
                "Adding a repository is your trust decision: in Spindle, pkg repo add NAME URL KEY"
            } else {
                "Going back a version is done in Spindle: pkg rollback NAME"
            };
            sex_pdx::serial_record!("[store.line] {}", line);
            Done {
                status: w::DENIED,
                lines: vec![line.as_bytes().to_vec()],
            }
        }
        _ => {
            sex_pdx::serial_record!(
                "[store.line] Usage: store [refresh | search WORDS | get [REPO/]NAME | upgrade | remove NAME | page NAME | repos | repo remove NAME]"
            );
            Done {
                status: w::INVALID,
                lines: Vec::new(),
            }
        }
    };
    let mut t = String::new();
    for l in &done.lines {
        t.push_str(shown(l));
        t.push('\n');
    }
    window.show(t.as_bytes());
    sex_pdx::serial_record!(
        "[store.done] op={} status={} lines={}",
        shown(op),
        word(done.status),
        done.lines.len()
    );
    sex_pdx::task_finish(0)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"sexshop");
    let mut raw = [0u8; w::ARGS_MAX];
    let args = files::args(&mut window, &mut raw);
    let packages = args.split(|b| *b == b' ').any(|t| t == p::GRANT_TOKEN);
    let words = command_words(args);
    sex_pdx::serial_record!(
        "[store.ready] target={} task={} packages={} command={}",
        window.target,
        window.task,
        packages as u8,
        shown(&words)
    );
    if !words.is_empty() {
        command(window, &words);
    }

    let canvas = Canvas::new(&window);
    let mut view = View {
        all: Vec::new(),
        shown: Vec::new(),
        categories: Vec::new(),
        category: 0,
        tab: SidebarTab::Discover,
        selected: 0,
        scroll_y: 0,
        scroll_to_selected: false,
        mode: Mode::List,
        filter: Vec::new(),
        typing: false,
        search_selected: false,
        status: Vec::new(),
        renders: 0,
        hits: Vec::new(),
        hovered: None,
        pressed: None,
        icons: Vec::new(),
        shots: Vec::new(),
        repos: Vec::new(),
        repo_selected: 0,
        canvas,
    };
    if !packages {
        view.say(vec![
            b"This run cannot install packages. Launch with `sexshop` for package grants.".to_vec(),
        ]);
    }
    view.reload(&mut window);
    sex_pdx::serial_record!("[store.updates] count={}", view.updates());
    view.render(&mut window, None);

    loop {
        let key = match window.next() {
            Event::Key(key) => key,
            Event::Close => window.close(),
            Event::Resize { .. } => {
                view.render(&mut window, None);
                continue;
            }
            Event::Pointer(pt) => {
                let hit = view.hit_at(pt.x as i32, pt.y as i32);
                match pt.kind {
                    PointerKind::Move => {
                        if view.hovered != hit {
                            view.hovered = hit;
                            view.render(&mut window, None);
                        }
                    }
                    PointerKind::Up { button: 0 } => {
                        if view.pressed.is_some() || view.hovered != hit {
                            view.pressed = None;
                            view.hovered = hit;
                            view.render(&mut window, None);
                        }
                    }
                    PointerKind::Down { button: 0 } => {
                        view.hovered = hit;
                        view.pressed = hit;
                        if !matches!(hit, Some(Hit::FocusSearch | Hit::ClearFilter)) && view.typing {
                            view.typing = false;
                            view.search_selected = false;
                        }
                        if let Some(h) = hit {
                            view.act(&mut window, h);
                        } else {
                            view.render(&mut window, None);
                        }
                    }
                    PointerKind::Wheel { steps } => {
                        view.scroll_y = (view.scroll_y - (steps as i32) * 40).max(0);
                        view.render(&mut window, None);
                    }
                    _ => {}
                }
                continue;
            }
            _ => continue,
        };

        if key.control && key.code == KEY_A && !view.filter.is_empty() {
            view.typing = true;
            view.search_selected = true;
            view.render(&mut window, None);
            continue;
        }
        if key.control && key.code == KEY_V {
            if let Ok(clip) = clipboard::paste(&mut window) {
                if view.search_selected {
                    view.filter.clear();
                    view.search_selected = false;
                }
                view.typing = true;
                for b in clip.bytes() {
                    if b == b'\n' || b == b'\r' {
                        break;
                    }
                    if b >= b' ' && view.filter.len() < 64 {
                        view.filter.push(b);
                    }
                }
                view.selected = 0;
                view.scroll_y = 0;
                view.mode = Mode::List;
                view.reload(&mut window);
                view.render(&mut window, None);
            }
            continue;
        }

        if view.typing {
            match key.code {
                ENTER | ESC => {
                    view.typing = false;
                    view.search_selected = false;
                }
                BACKSPACE => {
                    if view.search_selected {
                        view.filter.clear();
                        view.search_selected = false;
                    } else {
                        view.filter.pop();
                    }
                }
                _ => {
                    if let Some(c) = typed(key).filter(|c| *c >= b' ') {
                        if view.search_selected {
                            view.filter.clear();
                            view.search_selected = false;
                        }
                        if view.filter.len() < 64 {
                            view.filter.push(c);
                        }
                    }
                }
            }
            view.selected = 0;
            view.scroll_y = 0;
            view.reload(&mut window);
            view.render(&mut window, None);
            continue;
        }

        match view.mode {
            Mode::Page(i) => match (nav(key), key.code) {
                (_, ESC | BACKSPACE) => {
                    view.mode = Mode::List;
                    view.scroll_y = 0;
                }
                (Some(Nav::Delete), _) if i < view.all.len() && view.all[i].installed.is_some() => {
                    view.remove(&mut window, i);
                    continue;
                }
                (_, ENTER) if i < view.all.len() => {
                    if view.all[i].installed.is_some() && !view.all[i].newer {
                        view.say(vec![
                            format!("{} is installed and up to date.", view.all[i].full_name())
                                .into_bytes(),
                        ]);
                    } else {
                        view.install(&mut window, i);
                        continue;
                    }
                }
                _ => continue,
            },
            Mode::Repos => match (nav(key), key.code) {
                (_, ESC | BACKSPACE) => {
                    view.mode = Mode::List;
                    view.tab = SidebarTab::Discover;
                    view.scroll_y = 0;
                }
                (Some(Nav::Up), _) => view.repo_selected = view.repo_selected.saturating_sub(1),
                (Some(Nav::Down), _) => {
                    view.repo_selected = (view.repo_selected + 1).min(view.repos.len().saturating_sub(1))
                }
                (Some(Nav::Delete), _) => {
                    view.remove_repo(&mut window);
                    continue;
                }
                _ => continue,
            },
            Mode::List => {
                let chosen = view.shown.get(view.selected).copied();
                match (nav(key), key.code, typed(key)) {
                    (Some(Nav::Up), _, _) => {
                        view.selected = view.selected.saturating_sub(1);
                        view.scroll_to_selected = true;
                    }
                    (Some(Nav::Down), _, _) => {
                        view.selected = (view.selected + 1).min(view.shown.len().saturating_sub(1));
                        view.scroll_to_selected = true;
                    }
                    (Some(Nav::Home), _, _) => {
                        view.selected = 0;
                        view.scroll_y = 0;
                    }
                    (Some(Nav::End), _, _) => {
                        view.selected = view.shown.len().saturating_sub(1);
                        view.scroll_to_selected = true;
                    }
                    (Some(Nav::Left), _, _) => {
                        if view.tab == SidebarTab::AllApps && !view.categories.is_empty() {
                            view.category = view
                                .category
                                .checked_sub(1)
                                .unwrap_or(view.categories.len());
                            view.selected = 0;
                            view.scroll_y = 0;
                            view.pick_category();
                        }
                    }
                    (Some(Nav::Right), _, _) | (_, TAB, _) => {
                        if view.tab == SidebarTab::AllApps && !view.categories.is_empty() {
                            view.category = (view.category + 1) % (view.categories.len() + 1);
                        } else {
                            let idx = SidebarTab::ALL
                                .iter()
                                .position(|t| *t == view.tab)
                                .unwrap_or(0);
                            let next = SidebarTab::ALL[(idx + 1) % SidebarTab::ALL.len()];
                            view.act(&mut window, Hit::Nav(next));
                            continue;
                        }
                        view.selected = 0;
                        view.scroll_y = 0;
                        view.pick_category();
                    }
                    (Some(Nav::Delete), _, _) => {
                        if let Some(i) = chosen.filter(|i| view.all[*i].installed.is_some()) {
                            view.remove(&mut window, i);
                            continue;
                        }
                    }
                    (_, ENTER, _) => {
                        if let Some(i) = chosen {
                            view.open_page(&mut window, i);
                            continue;
                        }
                    }
                    (_, _, Some(b'r' | b'R')) => {
                        view.job(&mut window, "Downloading catalogs...", refresh);
                        continue;
                    }
                    (_, _, Some(b'u' | b'U')) => {
                        view.job(&mut window, "Upgrading packages...", |win| get(win, b""));
                        continue;
                    }
                    (_, _, Some(b'p' | b'P')) => {
                        view.load_repos(&mut window);
                        view.tab = SidebarTab::Repositories;
                        view.mode = Mode::Repos;
                    }
                    (_, _, Some(b'/')) => {
                        view.typing = true;
                        view.search_selected = false;
                    }
                    (_, ESC, _) if !view.filter.is_empty() => {
                        view.filter.clear();
                        view.selected = 0;
                        view.scroll_y = 0;
                        view.reload(&mut window);
                    }
                    _ => continue,
                }
            }
        }
        view.render(&mut window, None);
    }
}
