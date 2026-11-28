// SPDX-License-Identifier: GPL-3.0-only
//! Peek: macOS Noir Quick Look, Network & Binary Inspector.
//! `run +net peek URL`, `run peek @path`, or interactive `run peek`.
#![no_std]
#![no_main]

extern crate alloc;

mod noir;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use noir::{
    draw_card, fill_rect, FaceKind, NoirRenderer, BG_CANVAS, BG_CARD, BG_CONTENT, BG_PILL,
    BG_PILL_HOVER, BG_WELL, BORDER, COL_AMBER, COL_EMERALD, COL_NULL, COL_SKY, ROSE, ROSE_DIM,
    TEXT_DIM, TEXT_MUTED, TEXT_PRIMARY,
};
use sex_app::{files, net, Event, Nav, Window};
use sex_pdx::app_pointer::Kind as PKind;
use sex_theme::candy_ui::fill_rounded_rect;

const PAGE_BYTES: usize = 1024 * 1024;
const TOOLBAR_H: i32 = 48;
const CARDS_H: i32 = 66;
const STATUS_H: i32 = 26;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[peek.fatal] {}", info);
    sex_pdx::task_finish(1);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Text,
    Hex,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    View,
    Path,
    Search,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitId {
    PathBar,
    OpenBtn,
    SearchBtn,
    SearchBar,
    TabText,
    TabHex,
    ScrollTrack,
}

#[derive(Clone, Copy)]
struct HitBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: HitId,
}

struct Inspector {
    renderer: NoirRenderer,
    width: usize,
    height: usize,
    tab: Tab,
    focus: Focus,
    show_search: bool,
    path_input: String,
    search_input: String,
    source_label: String,
    status_badge: String,
    raw: Vec<u8>,
    text_lines: Vec<String>,
    detected_type: String,
    magic_hex: String,
    sha256_short: String,
    scroll: usize,
    hover: Option<HitId>,
    hits: Vec<HitBox>,
    status_msg: String,
}

/// Compact SHA-256 (`no_std`) returning first 16 hex chars + full digest info.
fn sha256_hex_prefix(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(((data.len() + 9 + 63) / 64) * 64);
    padded.extend_from_slice(data);
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, delta) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(delta);
        }
    }
    format!("{:08x}{:08x}", h[0], h[1])
}

fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{}.{} KiB ({bytes} B)", bytes / 1024, (bytes % 1024) * 10 / 1024)
    } else {
        format!("{}.{} MiB", bytes / (1024 * 1024), (bytes % (1024 * 1024)) * 10 / (1024 * 1024))
    }
}

fn detect_type(name: &str, data: &[u8]) -> &'static str {
    if data.starts_with(b"\x7fELF") {
        return "ELF64 Binary";
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "PNG Image";
    }
    if data.starts_with(b"%PDF") {
        return "PDF Document";
    }
    if data.starts_with(b"PK\x03\x04") {
        return "ZIP / Archive";
    }
    let trim = data.iter().position(|b| !b.is_ascii_whitespace()).map_or(&[][..], |i| &data[i..]);
    if trim.len() >= 5 && (trim[..5].eq_ignore_ascii_case(b"<html") || trim[..5].eq_ignore_ascii_case(b"<!doc")) {
        return "HTML Document";
    }
    if trim.starts_with(b"{") || trim.starts_with(b"[") {
        if core::str::from_utf8(data).is_ok() {
            return "JSON Data";
        }
    }
    if name.ends_with(".rs") {
        return "Rust Source";
    }
    if name.ends_with(".md") {
        return "Markdown";
    }
    if name.ends_with(".toml") {
        return "TOML Config";
    }
    if core::str::from_utf8(data).is_ok() {
        "UTF-8 Text"
    } else {
        "Binary Stream"
    }
}

/// Text between HTML tags, with runs of blank space folded.
fn extract_html_text(html: &[u8]) -> String {
    let mut out = Vec::new();
    let (mut in_tag, mut space) = (false, true);
    let mut i = 0;
    while i < html.len() {
        for (open, close) in [(&b"<style"[..], &b"</style>"[..]), (b"<script", b"</script>")] {
            if html[i..].len() >= open.len() && html[i..i + open.len()].eq_ignore_ascii_case(open) {
                i += html[i..]
                    .windows(close.len())
                    .position(|w| w.eq_ignore_ascii_case(close))
                    .map_or(html.len() - i, |p| p + close.len());
            }
        }
        if i >= html.len() {
            break;
        }
        let b = html[i];
        i += 1;
        match b {
            b'<' => in_tag = true,
            b'>' => {
                in_tag = false;
                if !space {
                    out.push(b' ');
                    space = true;
                }
            }
            _ if in_tag => {}
            b'\n' | b'\r' | b'\t' | b' ' => {
                if !space {
                    out.push(b' ');
                    space = true;
                }
            }
            32..=126 => {
                out.push(b);
                space = false;
            }
            _ => {}
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn build_text_lines(kind: &str, data: &[u8]) -> Vec<String> {
    if data.is_empty() {
        return vec![String::from("(empty payload)")];
    }
    if kind == "HTML Document" {
        let extracted = extract_html_text(data);
        if !extracted.trim().is_empty() {
            return wrap_plain(&extracted, 88);
        }
    }
    if let Ok(s) = core::str::from_utf8(data) {
        let mut lines = Vec::new();
        for l in s.lines() {
            if l.len() <= 100 {
                lines.push(String::from(l));
            } else {
                lines.extend(wrap_plain(l, 100));
            }
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        return lines;
    }
    vec![String::from("Binary payload — switch to Hex + ASCII Inspector tab to inspect raw bytes.")]
}

fn wrap_plain(s: &str, max_cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > max_cols {
            out.push(core::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

impl Inspector {
    fn new(w: usize, h: usize) -> Self {
        let mut me = Self {
            renderer: NoirRenderer::new(),
            width: w.max(320),
            height: h.max(240),
            tab: Tab::Text,
            focus: Focus::View,
            show_search: false,
            path_input: String::new(),
            search_input: String::new(),
            source_label: String::from("Quick Look"),
            status_badge: String::from("Ready"),
            raw: Vec::new(),
            text_lines: Vec::new(),
            detected_type: String::from("UTF-8 Text"),
            magic_hex: String::from("--"),
            sha256_short: String::from("--"),
            scroll: 0,
            hover: None,
            hits: Vec::new(),
            status_msg: String::from("Type a URL or @path and press Enter, or click Open."),
        };
        let welcome = b"Peek - SexOS Noir Quick Look & Binary Inspector\n\n\
            Usage:\n  - run +net peek http://host/page   Fetch and inspect network response\n\
              - run peek @/path/to/file          Inspect local file bytes & metadata\n\
              - Type any path or URL in the top bar and press Enter\n\
              - Click 'Open' or press Ctrl+O to choose a file\n\n\
            Tabs:\n  [1] Text Preview        Clean readable text with search highlighting\n\
              [2] Hex + ASCII         Color-coded 16-byte columns (Null, ASCII, Ctrl, High)";
        me.set_payload("peek://welcome", "Interactive", welcome.to_vec());
        me
    }

    fn set_payload(&mut self, source: &str, badge: &str, data: Vec<u8>) {
        self.source_label = String::from(source);
        self.path_input = String::from(source);
        self.status_badge = String::from(badge);
        let kind = detect_type(source, &data);
        self.detected_type = String::from(kind);
        self.magic_hex = if data.is_empty() {
            String::from("00 00 00 00")
        } else {
            data.iter()
                .take(4)
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        self.sha256_short = sha256_hex_prefix(&data);
        self.text_lines = build_text_lines(kind, &data);
        self.raw = data;
        self.scroll = 0;
        if self.detected_type == "Binary Stream" || self.detected_type == "ELF64 Binary" {
            self.tab = Tab::Hex;
        }
    }

    fn load_target(&mut self, window: &mut Window, target: &str) {
        let target = target.trim();
        if target.is_empty() {
            return;
        }
        if target.starts_with("http://") || target.starts_with("https://") {
            let mut page = vec![0u8; PAGE_BYTES];
            match net::get(window, target.as_bytes(), &mut page) {
                Ok((status, len)) => {
                    sex_pdx::serial_record!(
                        "[peek.got] status={} bytes={} heap={}",
                        status,
                        len,
                        sex_app::heap::held()
                    );
                    page.truncate(len);
                    self.set_payload(target, &format!("HTTP {status}"), page);
                    self.status_msg = format!("Fetched {} bytes (HTTP {status}).", len);
                }
                Err(e) => {
                    sex_pdx::serial_record!("[peek.refused] error={:?}", e);
                    let msg = core::str::from_utf8(net::message(e)).unwrap_or("Network request refused");
                    self.set_payload(target, "Refused", msg.as_bytes().to_vec());
                    self.status_msg = String::from(msg);
                }
            }
            return;
        }
        let path = target.as_bytes();
        match files::stat(window, path) {
            Ok(st) => {
                let cap = (st.size as usize).min(PAGE_BYTES);
                let mut data = vec![0u8; cap];
                let mut at = 0;
                while at < cap {
                    match files::read_at(window, path, at as u64, &mut data[at..]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => at += n,
                    }
                }
                data.truncate(at);
                self.set_payload(target, "File", data);
                self.status_msg = format!("Loaded {} ({} bytes).", target, at);
            }
            Err(e) => {
                let msg = core::str::from_utf8(files::message(e)).unwrap_or("Cannot read file");
                self.set_payload(target, "Error", msg.as_bytes().to_vec());
                self.status_msg = format!("Read error: {msg}");
            }
        }
    }

    fn total_rows(&self) -> usize {
        match self.tab {
            Tab::Text => self.text_lines.len().max(1),
            Tab::Hex => ((self.raw.len() + 15) / 16).max(1),
        }
    }

    fn visible_rows(&self) -> usize {
        let content_y = TOOLBAR_H + CARDS_H + 16 + if self.show_search { 34 } else { 0 };
        let content_h = (self.height as i32 - content_y - STATUS_H - 12).max(48);
        let row_h = self.renderer.line_height(FaceKind::Mono, 13) as i32;
        ((content_h - 32) / row_h).max(1) as usize
    }

    fn clamp_scroll(&mut self) {
        let max_top = self.total_rows().saturating_sub(self.visible_rows());
        if self.scroll > max_top {
            self.scroll = max_top;
        }
    }

    fn draw(&mut self, window: &mut Window) {
        self.clamp_scroll();
        self.hits.clear();
        let (w, h) = (self.width, self.height);
        let wi = w as i32;
        let hi = h as i32;
        let buf = window.frame(w, h);
        buf.fill(BG_CANVAS);

        // Top Toolbar
        fill_rect(buf, w, h, 0, 0, wi, TOOLBAR_H, BG_CARD);
        fill_rect(buf, w, h, 0, TOOLBAR_H - 1, wi, 1, BORDER);

        // PEEK brand badge
        fill_rounded_rect(buf, w, h, 12, 11, 56, 26, 7, ROSE);
        self.renderer.draw_text(buf, w, h, FaceKind::SansBold, 22, 16, 44, 13, "PEEK", TEXT_PRIMARY);

        // Right-aligned buttons & tabs
        let hex_w = 104;
        let txt_w = 96;
        let find_w = 58;
        let open_w = 60;
        let hex_x = wi - 12 - hex_w;
        let txt_x = hex_x - 6 - txt_w;
        let find_x = txt_x - 10 - find_w;
        let open_x = find_x - 6 - open_w;

        // Path Bar
        let path_x = 78;
        let path_w = (open_x - path_x - 10).max(80);
        let path_border = if self.focus == Focus::Path { ROSE } else { BORDER };
        draw_card(buf, w, h, path_x, 9, path_w, 30, 8, BG_WELL, path_border);
        self.hits.push(HitBox { x: path_x, y: 9, w: path_w, h: 30, id: HitId::PathBar });
        let shown_path = if self.focus == Focus::Path {
            format!("{}|", self.path_input)
        } else if self.path_input.is_empty() {
            String::from("Enter URL or @path...")
        } else {
            self.path_input.clone()
        };
        let pcol = if self.path_input.is_empty() && self.focus != Focus::Path { TEXT_MUTED } else { TEXT_PRIMARY };
        self.renderer.draw_text(buf, w, h, FaceKind::Mono, path_x + 10, 16, path_w - 20, 12, &shown_path, pcol);

        // Open button
        let obg = if self.hover == Some(HitId::OpenBtn) { BG_PILL_HOVER } else { BG_PILL };
        draw_card(buf, w, h, open_x, 9, open_w, 30, 8, obg, BORDER);
        self.hits.push(HitBox { x: open_x, y: 9, w: open_w, h: 30, id: HitId::OpenBtn });
        self.renderer.draw_text(buf, w, h, FaceKind::SansBold, open_x + 13, 16, open_w - 16, 12, "Open", TEXT_PRIMARY);

        // Find toggle button
        let fbg = if self.show_search { ROSE_DIM } else if self.hover == Some(HitId::SearchBtn) { BG_PILL_HOVER } else { BG_PILL };
        let fborder = if self.show_search { ROSE } else { BORDER };
        draw_card(buf, w, h, find_x, 9, find_w, 30, 8, fbg, fborder);
        self.hits.push(HitBox { x: find_x, y: 9, w: find_w, h: 30, id: HitId::SearchBtn });
        self.renderer.draw_text(buf, w, h, FaceKind::SansBold, find_x + 15, 16, find_w - 16, 12, "Find", TEXT_PRIMARY);

        // Dual View Tabs
        for (bx, bw, label, is_active, hid) in [
            (txt_x, txt_w, "Text Preview", self.tab == Tab::Text, HitId::TabText),
            (hex_x, hex_w, "Hex + ASCII", self.tab == Tab::Hex, HitId::TabHex),
        ] {
            let fill = if is_active { ROSE } else if self.hover == Some(hid) { BG_PILL_HOVER } else { BG_PILL };
            let bord = if is_active { ROSE } else { BORDER };
            draw_card(buf, w, h, bx, 9, bw, 30, 8, fill, bord);
            self.hits.push(HitBox { x: bx, y: 9, w: bw, h: 30, id: hid });
            self.renderer.draw_text(buf, w, h, FaceKind::SansBold, bx + 11, 16, bw - 16, 12, label, TEXT_PRIMARY);
        }

        // Optional Search Bar
        let mut top_y = TOOLBAR_H + 10;
        if self.show_search {
            draw_card(buf, w, h, 12, top_y, wi - 24, 28, 7, BG_CARD, if self.focus == Focus::Search { ROSE } else { BORDER });
            self.hits.push(HitBox { x: 12, y: top_y, w: wi - 24, h: 28, id: HitId::SearchBar });
            self.renderer.draw_text(buf, w, h, FaceKind::SansBold, 22, top_y + 6, 60, 12, "Search:", TEXT_DIM);
            let s_txt = if self.focus == Focus::Search {
                format!("{}|", self.search_input)
            } else {
                self.search_input.clone()
            };
            self.renderer.draw_text(buf, w, h, FaceKind::Mono, 80, top_y + 6, wi - 110, 12, &s_txt, TEXT_PRIMARY);
            top_y += 34;
        }

        // Metadata Summary Cards (3 equal cards)
        let card_gap = 10;
        let card_w = ((wi - 24 - card_gap * 2) / 3).max(80);
        let cards = [
            ("PAYLOAD SIZE", format_size(self.raw.len()), format!("{} lines", self.text_lines.len())),
            ("DETECTED TYPE", self.detected_type.clone(), self.status_badge.clone()),
            ("MAGIC & SHA-256", self.magic_hex.clone(), format!("sha256: {}", self.sha256_short)),
        ];
        for (idx, (title, main_val, sub_val)) in cards.iter().enumerate() {
            let cx = 12 + idx as i32 * (card_w + card_gap);
            draw_card(buf, w, h, cx, top_y, card_w, CARDS_H - 8, 10, BG_CARD, BORDER);
            self.renderer.draw_text(buf, w, h, FaceKind::SansBold, cx + 12, top_y + 7, card_w - 24, 10, title, TEXT_MUTED);
            self.renderer.draw_text(buf, w, h, FaceKind::SansBold, cx + 12, top_y + 22, card_w - 24, 14, main_val, TEXT_PRIMARY);
            self.renderer.draw_text(buf, w, h, FaceKind::Mono, cx + 12, top_y + 40, card_w - 24, 11, sub_val, if idx == 2 { ROSE } else { TEXT_DIM });
        }

        // Main Inspector Content Card
        let content_y = top_y + CARDS_H;
        let content_h = (hi - content_y - STATUS_H - 10).max(48);
        let content_w = wi - 24;
        draw_card(buf, w, h, 12, content_y, content_w, content_h, 10, BG_CONTENT, BORDER);

        let row_h = self.renderer.line_height(FaceKind::Mono, 13) as i32;
        let adv = self.renderer.mono_advance(13);
        let vis = self.visible_rows();
        let needle_lower = self.search_input.to_ascii_lowercase();

        match self.tab {
            Tab::Text => {
                // Header
                fill_rect(buf, w, h, 13, content_y + 1, content_w - 2, 24, BG_CARD);
                fill_rect(buf, w, h, 13, content_y + 25, content_w - 2, 1, BORDER);
                self.renderer.draw_text(buf, w, h, FaceKind::SansBold, 24, content_y + 5, content_w - 40, 11, &format!("TEXT PREVIEW  ·  {}", self.source_label), TEXT_DIM);

                let body_y = content_y + 30;
                for (i, line) in self.text_lines.iter().skip(self.scroll).take(vis).enumerate() {
                    let line_no = self.scroll + i + 1;
                    let ry = body_y + i as i32 * row_h;
                    if ry + row_h > content_y + content_h - 4 {
                        break;
                    }
                    let num_str = format!("{line_no:>4}");
                    self.renderer.draw_text(buf, w, h, FaceKind::Mono, 20, ry, 44, 12, &num_str, TEXT_MUTED);
                    if !needle_lower.is_empty() && line.to_ascii_lowercase().contains(&needle_lower) {
                        fill_rounded_rect(buf, w, h, 64, ry, content_w - 86, row_h, 4, ROSE_DIM);
                    }
                    self.renderer.draw_text(buf, w, h, FaceKind::Mono, 70, ry, content_w - 92, 13, line, TEXT_PRIMARY);
                }
            }
            Tab::Hex => {
                // Header
                fill_rect(buf, w, h, 13, content_y + 1, content_w - 2, 24, BG_CARD);
                fill_rect(buf, w, h, 13, content_y + 25, content_w - 2, 1, BORDER);
                let ox = 22;
                let hx0 = ox + adv * 10;
                let ax0 = hx0 + adv * 51;
                self.renderer.draw_text(buf, w, h, FaceKind::MonoBold, ox, content_y + 5, adv * 9, 11, "OFFSET", TEXT_MUTED);
                self.renderer.draw_text(buf, w, h, FaceKind::MonoBold, hx0, content_y + 5, adv * 49, 11, "00 01 02 03 04 05 06 07  08 09 0A 0B 0C 0D 0E 0F", TEXT_DIM);
                self.renderer.draw_text(buf, w, h, FaceKind::MonoBold, ax0, content_y + 5, adv * 17, 11, "ASCII", ROSE);

                let body_y = content_y + 30;
                let total_rows = (self.raw.len() + 15) / 16;
                for i in 0..vis {
                    let row_idx = self.scroll + i;
                    if row_idx >= total_rows {
                        break;
                    }
                    let ry = body_y + i as i32 * row_h;
                    if ry + row_h > content_y + content_h - 4 {
                        break;
                    }
                    if row_idx % 2 == 1 {
                        fill_rect(buf, w, h, 14, ry, content_w - 20, row_h, 0x0013_1824);
                    }
                    let base = row_idx * 16;
                    let off_str = format!("{base:08X}");
                    self.renderer.draw_text(buf, w, h, FaceKind::Mono, ox, ry, adv * 9, 13, &off_str, TEXT_MUTED);

                    let chunk = &self.raw[base..(base + 16).min(self.raw.len())];
                    let mut ascii_buf = [0u8; 16];
                    for (j, &b) in chunk.iter().enumerate() {
                        let bx = hx0 + (j as i32 * 3 + if j >= 8 { 1 } else { 0 }) * adv;
                        let col = byte_color(b);
                        let hex = format!("{b:02X}");
                        self.renderer.draw_text(buf, w, h, FaceKind::Mono, bx, ry, adv * 3, 13, &hex, col);
                        ascii_buf[j] = if (0x20..=0x7E).contains(&b) { b } else { b'.' };
                    }
                    for (j, &b) in chunk.iter().enumerate() {
                        let ch_s = [ascii_buf[j]];
                        let s = core::str::from_utf8(&ch_s).unwrap_or(".");
                        self.renderer.draw_text(buf, w, h, FaceKind::Mono, ax0 + j as i32 * adv, ry, adv + 2, 13, s, byte_color(b));
                    }
                }
            }
        }

        // Scrollbar
        let total = self.total_rows();
        if total > vis {
            let track_x = 12 + content_w - 10;
            let track_y = content_y + 28;
            let track_h = (content_h - 32).max(16);
            fill_rounded_rect(buf, w, h, track_x, track_y, 6, track_h, 3, BG_WELL);
            self.hits.push(HitBox { x: track_x - 4, y: track_y, w: 14, h: track_h, id: HitId::ScrollTrack });
            let thumb_h = ((vis * track_h as usize) / total).clamp(20, track_h as usize) as i32;
            let max_top = total - vis;
            let thumb_y = track_y + ((self.scroll * (track_h - thumb_h) as usize) / max_top.max(1)) as i32;
            fill_rounded_rect(buf, w, h, track_x, thumb_y, 6, thumb_h, 3, ROSE);
        }

        // Bottom Status Bar
        let sy = hi - STATUS_H;
        fill_rect(buf, w, h, 0, sy, wi, STATUS_H, BG_CARD);
        fill_rect(buf, w, h, 0, sy, wi, 1, BORDER);
        self.renderer.draw_text(buf, w, h, FaceKind::Sans, 14, sy + 6, wi - 280, 11, &self.status_msg, TEXT_DIM);
        let pos_info = format!("Row {}/{}  ·  1/2 Tabs  ·  F11 Close", (self.scroll + 1).min(total), total);
        let pm = self.renderer.measure(FaceKind::Sans, 11, &pos_info);
        self.renderer.draw_text(buf, w, h, FaceKind::Sans, (wi - 14 - pm).max(14), sy + 6, pm + 8, 11, &pos_info, TEXT_MUTED);

        window.show_frame(w, h);
    }

    fn hit_test(&self, x: i32, y: i32) -> Option<HitId> {
        self.hits
            .iter()
            .rev()
            .find(|h| x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h)
            .map(|h| h.id)
    }
}

fn byte_color(b: u8) -> u32 {
    match b {
        0x00 => COL_NULL,
        b'\n' | b'\r' | b'\t' | 0x01..=0x1F | 0x7F => COL_SKY,
        0x20..=0x7E => COL_EMERALD,
        0x80..=0xFF => COL_AMBER,
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    sex_app::with_stack(1 << 20, main)
}

extern "C" fn main() -> ! {
    let mut window = Window::open();
    window.title(b"Peek");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let raw_arg = files::args(&mut window, &mut args).to_vec();
    let (w, h) = window.size();
    let mut app = Inspector::new(w, h);

    if !raw_arg.is_empty() {
        let arg_str = core::str::from_utf8(&raw_arg).unwrap_or("").trim();
        if !arg_str.is_empty() {
            app.load_target(&mut window, arg_str);
        }
    }

    app.draw(&mut window);
    sex_pdx::serial_record!("[peek.ready] target={} task={}", window.target, window.task);

    loop {
        match window.next() {
            Event::Close => {
                sex_pdx::serial_record!("[peek.closed] target={} task={}", window.target, window.task);
                window.close();
            }
            Event::Resize { width, height } => {
                app.width = width.max(320);
                app.height = height.max(240);
                app.draw(&mut window);
            }
            Event::Pointer(p) => {
                let (x, y) = (p.x as i32, p.y as i32);
                match p.kind {
                    PKind::Move => {
                        let h = app.hit_test(x, y);
                        if h != app.hover {
                            app.hover = h;
                            app.draw(&mut window);
                        }
                    }
                    PKind::Wheel { steps } => {
                        if steps > 0 {
                            app.scroll = app.scroll.saturating_sub(steps as usize * 3);
                        } else if steps < 0 {
                            app.scroll = app.scroll.saturating_add((-steps) as usize * 3);
                        }
                        app.draw(&mut window);
                    }
                    PKind::Down { button: 0 } => {
                        match app.hit_test(x, y) {
                            Some(HitId::PathBar) => app.focus = Focus::Path,
                            Some(HitId::SearchBar) => app.focus = Focus::Search,
                            Some(HitId::TabText) => {
                                app.tab = Tab::Text;
                                app.scroll = 0;
                                app.focus = Focus::View;
                            }
                            Some(HitId::TabHex) => {
                                app.tab = Tab::Hex;
                                app.scroll = 0;
                                app.focus = Focus::View;
                            }
                            Some(HitId::SearchBtn) => {
                                app.show_search = !app.show_search;
                                app.focus = if app.show_search { Focus::Search } else { Focus::View };
                            }
                            Some(HitId::OpenBtn) => {
                                app.focus = Focus::View;
                                let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
                                if let Ok(p) = files::pick(&mut window, files::Pick::Open, b"", &mut out) {
                                    let s = String::from_utf8_lossy(p).into_owned();
                                    app.load_target(&mut window, &s);
                                }
                            }
                            Some(HitId::ScrollTrack) => {
                                if let Some(hb) = app.hits.iter().find(|h| h.id == HitId::ScrollTrack) {
                                    let rel = ((y - hb.y).max(0) as usize).min(hb.h.max(1) as usize);
                                    let max_top = app.total_rows().saturating_sub(app.visible_rows());
                                    app.scroll = rel * max_top / (hb.h.max(1) as usize);
                                }
                            }
                            None => app.focus = Focus::View,
                        }
                        app.draw(&mut window);
                    }
                    _ => {}
                }
            }
            Event::Key(key) => {
                let page = app.visible_rows().saturating_sub(1).max(1);
                let nav = sex_app::nav(key);
                let mut handled = true;
                if key.control && key.code == 0x21 {
                    // Ctrl+F
                    app.show_search = true;
                    app.focus = Focus::Search;
                } else if key.control && key.code == 0x18 {
                    // Ctrl+O
                    let mut out = [0u8; sex_pdx::fs::PATH_BYTES];
                    if let Ok(p) = files::pick(&mut window, files::Pick::Open, b"", &mut out) {
                        let s = String::from_utf8_lossy(p).into_owned();
                        app.load_target(&mut window, &s);
                    }
                } else if key.code == 0x01 {
                    // Esc
                    if app.focus != Focus::View {
                        app.focus = Focus::View;
                    } else if app.show_search {
                        app.show_search = false;
                    }
                } else if app.focus == Focus::Path {
                    match key.code {
                        0x1C => {
                            let t = app.path_input.clone();
                            app.focus = Focus::View;
                            app.load_target(&mut window, &t);
                        }
                        0x0E => {
                            app.path_input.pop();
                        }
                        _ => {
                            if let Some(c) = sex_app::text(key) {
                                app.path_input.push(c);
                            } else {
                                handled = false;
                            }
                        }
                    }
                } else if app.focus == Focus::Search {
                    match key.code {
                        0x1C => {
                            let needle = app.search_input.to_ascii_lowercase();
                            if !needle.is_empty() && app.tab == Tab::Text {
                                if let Some(pos) = app.text_lines.iter().skip(app.scroll + 1).position(|l| l.to_ascii_lowercase().contains(&needle)) {
                                    app.scroll += 1 + pos;
                                } else if let Some(pos) = app.text_lines.iter().position(|l| l.to_ascii_lowercase().contains(&needle)) {
                                    app.scroll = pos;
                                }
                            }
                        }
                        0x0E => {
                            app.search_input.pop();
                        }
                        _ => {
                            if let Some(c) = sex_app::text(key) {
                                app.search_input.push(c);
                            } else {
                                handled = false;
                            }
                        }
                    }
                } else {
                    match (nav, key.code) {
                        (Some(Nav::Up), _) => app.scroll = app.scroll.saturating_sub(1),
                        (Some(Nav::Down), _) => app.scroll = app.scroll.saturating_add(1),
                        (Some(Nav::Home), _) => app.scroll = 0,
                        (Some(Nav::End), _) => app.scroll = app.total_rows(),
                        (_, 0x49 | 0x68) => app.scroll = app.scroll.saturating_sub(page),
                        (_, 0x51 | 0x6D) => app.scroll = app.scroll.saturating_add(page),
                        (_, 0x02) => { app.tab = Tab::Text; app.scroll = 0; }
                        (_, 0x03) => { app.tab = Tab::Hex; app.scroll = 0; }
                        _ => handled = false,
                    }
                }
                if handled {
                    app.draw(&mut window);
                }
            }
            _ => {}
        }
    }
}
