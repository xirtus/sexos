// SPDX-License-Identifier: GPL-3.0-only
//! Shuttle: macOS AirDrop & Local Handoff Noir GUI on top of `sex_app::launch`.
//! `run shuttle @photo.png` opens the file in the program for its type, handing it that
//! file alone; `run shuttle NAME ARGS` runs a program (`-k` kills, `-a` passes `@/`,
//! `-h` hidden). When launched without CLI args, renders an interactive AirDrop Noir GUI.
#![no_std]
#![no_main]

extern crate alloc;

mod noir;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use noir::{
    draw_card, draw_ring, fill_rect, window_dims, NoirPainter, BG, BORDER, BORDER_ACTIVE, CARD,
    CARD_ALT, DIM, EMERALD, EMERALD_BG, EMERALD_BORDER, HEADER, MUTED, ROSE, ROSE_BG, ROSE_BORDER,
    SKY, SKY_BG, SKY_BORDER, TEXT, WELL,
};
use sex_app::{dnd, files, launch, Event, Window};
use sex_pdx::app_pointer::Kind as PointerKind;
use sex_pdx::fs as w;
use sex_theme::candy_ui::{fill_rounded_rect, ButtonState, ButtonStyle};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[shuttle.fatal] {}", info);
    sex_pdx::task_finish(1);
}

const ENTER: u8 = 0x1C;
const BACKSPACE: u8 = 0x0E;
const ESC: u8 = 0x01;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TransferStatus {
    Launching,
    Active,
    Completed(u32),
    Ended,
    Refused,
}

struct TransferCard {
    cookie: u32,
    label: String,
    target_peer: String,
    row: Option<u32>,
    task: Option<u32>,
    hidden: bool,
    kill_flag: bool,
    progress: u8,
    status: TransferStatus,
    detail: String,
}

#[derive(Clone, Copy)]
struct PeerNode {
    name: &'static str,
    role: &'static str,
    dx: i32,
    dy: i32,
    color: u32,
}

const PEERS: [PeerNode; 4] = [
    PeerNode { name: "Silk Shell", role: "Workspace Portal", dx: -56, dy: -46, color: EMERALD },
    PeerNode { name: "Files Vault", role: "Capability Broker", dx: 58, dy: -42, color: SKY },
    PeerNode { name: "Gaze Viewer", role: "Media Handoff", dx: -52, dy: 52, color: ROSE },
    PeerNode { name: "Spindle Node", role: "Terminal Host", dx: 54, dy: 48, color: EMERALD },
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum HitAction {
    ToggleReceive,
    SendFile,
    SelectPeer(usize),
    KillTransfer(u32),
    RemoveTransfer(usize),
    SubmitInput,
}

#[derive(Clone, Copy)]
struct HitBox {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    action: HitAction,
}

struct App {
    receive_mode: bool,
    selected_peer: usize,
    next_cookie: u32,
    radar_phase: u32,
    queue: Vec<TransferCard>,
    scroll: usize,
    input: String,
    status_line: String,
    hits: Vec<HitBox>,
    mouse: (i32, i32),
    mouse_down: bool,
    hovered: Option<usize>,
    painter: NoirPainter,
}

impl App {
    fn new() -> Self {
        Self {
            receive_mode: true,
            selected_peer: 0,
            next_cookie: 2,
            radar_phase: 0,
            queue: Vec::new(),
            scroll: 0,
            input: String::new(),
            status_line: "Drop a file onto the radar or click 'Send File…' to hand off via AirDrop / Shuttle.".into(),
            hits: Vec::new(),
            mouse: (-1, -1),
            mouse_down: false,
            hovered: None,
            painter: NoirPainter::new(),
        }
    }

    fn btn_state(&self, x: i32, y: i32, w: i32, h: i32) -> ButtonState {
        let (mx, my) = self.mouse;
        if mx >= x && mx < x + w && my >= y && my < y + h {
            if self.mouse_down { ButtonState::Pressed } else { ButtonState::Hover }
        } else {
            ButtonState::Rest
        }
    }

    fn push_button(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        label: &str,
        style: ButtonStyle,
        action: HitAction,
    ) {
        let st = if matches!(style, ButtonStyle::Disabled) {
            ButtonState::Rest
        } else {
            self.btn_state(x, y, w, h)
        };
        self.painter.button(buf, bw, bh, x, y, w, h, label, style, st);
        if !matches!(style, ButtonStyle::Disabled) {
            self.hits.push(HitBox { x, y, w, h, action });
        }
    }

    fn launch_line(&mut self, line: &[u8], cookie: u32, kill: bool, all: bool, hidden: bool) -> bool {
        let (first, rest) = match line.iter().position(|b| *b == b' ') {
            Some(i) => (&line[..i], &line[i + 1..]),
            None => (line, &b""[..]),
        };
        if first.is_empty() {
            return false;
        }
        let mut joined = [0u8; sex_pdx::fs::ARGS_MAX];
        let rest = if all && rest.len() + 3 <= joined.len() {
            joined[..rest.len()].copy_from_slice(rest);
            let at = if rest.is_empty() { 0 } else { joined[rest.len()] = b' '; rest.len() + 1 };
            joined[at..at + 2].copy_from_slice(b"@/");
            &joined[..at + 2]
        } else {
            rest
        };
        let label = String::from_utf8_lossy(line).into_owned();
        let peer_name = PEERS[self.selected_peer].name.to_string();
        let asked = match first {
            [b'@', path @ ..] if rest.is_empty() && path.first() == Some(&b'/') => launch::open(path, cookie),
            name if hidden => launch::run_hidden(name, rest, cookie),
            name => launch::run(name, rest, false, cookie),
        };
        match asked {
            Err(e) => {
                sex_pdx::serial_record!("[shuttle.refused] by=files error={:?}", e);
                let why: &[u8] = match e {
                    launch::Error::NoProgram => b"No program opens files of that type.",
                    launch::Error::Shell => b"The desktop could not be asked.",
                    launch::Error::Files(c) => files::message(c),
                };
                let why_str = String::from_utf8_lossy(why).into_owned();
                self.status_line = format!("Not started: {}", why_str);
                self.queue.insert(
                    0,
                    TransferCard {
                        cookie,
                        label,
                        target_peer: peer_name,
                        row: None,
                        task: None,
                        hidden,
                        kill_flag: kill,
                        progress: 100,
                        status: TransferStatus::Refused,
                        detail: why_str,
                    },
                );
                false
            }
            Ok(()) => {
                self.queue.insert(
                    0,
                    TransferCard {
                        cookie,
                        label,
                        target_peer: peer_name,
                        row: None,
                        task: None,
                        hidden,
                        kill_flag: kill,
                        progress: 35,
                        status: TransferStatus::Launching,
                        detail: "Handing off via Shell capability portal…".into(),
                    },
                );
                true
            }
        }
    }

    fn apply_launch_event(&mut self, ev: launch::Event) -> bool {
        let cookie = ev.cookie();
        let Some(card) = self.queue.iter_mut().find(|c| c.cookie == cookie) else {
            return false;
        };
        match ev {
            launch::Event::Started { row, task, .. } => {
                sex_pdx::serial_record!(
                    "[shuttle.started] row={} task={} kill={} hidden={}",
                    row,
                    task,
                    card.kill_flag as u8,
                    card.hidden as u8
                );
                card.row = Some(row);
                card.task = Some(task);
                card.progress = 72;
                card.status = TransferStatus::Active;
                card.detail = format!("Active on row {} (task {})", row, task);
                self.status_line = format!("Started {} on row {}.", card.label, row);
                if card.kill_flag {
                    launch::kill(cookie);
                }
                false
            }
            launch::Event::Refused { code, .. } => {
                sex_pdx::serial_record!("[shuttle.refused] by=shell code={}", code);
                let why = String::from_utf8_lossy(sex_pdx::program_store::refusal(code as u64)).into_owned();
                card.progress = 100;
                card.status = TransferStatus::Refused;
                card.detail = why.clone();
                self.status_line = format!("Not started: {}", why);
                true
            }
            launch::Event::Done { status, .. } => {
                sex_pdx::serial_record!("[shuttle.done] status={:?}", status);
                card.progress = 100;
                match status {
                    Some(s) => {
                        card.status = TransferStatus::Completed(s);
                        card.detail = format!("Handoff complete (exit {})", s);
                        self.status_line = format!("{} ended with status {}.", card.label, s);
                    }
                    None => {
                        card.status = TransferStatus::Ended;
                        card.detail = "Terminated cleanly".into();
                        self.status_line = format!("{} was ended.", card.label);
                    }
                }
                true
            }
        }
    }

    fn send_file_picker(&mut self, window: &mut Window) {
        let mut out = [0u8; w::PATH_BYTES];
        match files::pick(window, files::Pick::Open, b"", &mut out) {
            Ok(p) => {
                let mut spec = Vec::new();
                if !p.starts_with(b"@") {
                    spec.push(b'@');
                }
                spec.extend_from_slice(p);
                let cookie = self.next_cookie;
                self.next_cookie = self.next_cookie.wrapping_add(1).max(2);
                self.launch_line(&spec, cookie, false, false, false);
            }
            Err(_) => {
                self.status_line = "File picker cancelled.".into();
            }
        }
    }

    fn hit_at(&self, x: i32, y: i32) -> Option<usize> {
        self.hits.iter().rposition(|h| x >= h.x && x < h.x + h.w && y >= h.y && y < h.y + h.h)
    }

    fn pointer(&mut self, window: &mut Window, ptr: sex_pdx::app_pointer::Pointer) -> bool {
        self.mouse = (ptr.x as i32, ptr.y as i32);
        match ptr.kind {
            PointerKind::Move => {
                let h = self.hit_at(self.mouse.0, self.mouse.1);
                if h != self.hovered {
                    self.hovered = h;
                    return true;
                }
                false
            }
            PointerKind::Leave => {
                self.mouse_down = false;
                if self.hovered.is_some() {
                    self.hovered = None;
                    return true;
                }
                false
            }
            PointerKind::Down { button: 0 } => {
                self.mouse_down = true;
                self.hovered = self.hit_at(self.mouse.0, self.mouse.1);
                true
            }
            PointerKind::Up { button: 0 } => {
                self.mouse_down = false;
                if let Some(idx) = self.hit_at(self.mouse.0, self.mouse.1) {
                    let action = self.hits[idx].action;
                    self.dispatch_hit(window, action);
                }
                true
            }
            PointerKind::Wheel { steps } => {
                let delta = -(steps as isize);
                let max_scroll = self.queue.len().saturating_sub(1);
                self.scroll = (self.scroll as isize + delta).clamp(0, max_scroll as isize) as usize;
                true
            }
            _ => false,
        }
    }

    fn dispatch_hit(&mut self, window: &mut Window, action: HitAction) {
        match action {
            HitAction::ToggleReceive => {
                self.receive_mode = !self.receive_mode;
                self.status_line = if self.receive_mode {
                    "Receive Mode enabled: drop files onto Shuttle or pick a target.".into()
                } else {
                    "Receive Mode paused.".into()
                };
            }
            HitAction::SendFile => self.send_file_picker(window),
            HitAction::SelectPeer(i) => {
                self.selected_peer = i.min(PEERS.len() - 1);
                self.status_line = format!("Selected peer: {} ({})", PEERS[self.selected_peer].name, PEERS[self.selected_peer].role);
            }
            HitAction::KillTransfer(cookie) => {
                launch::kill(cookie);
                self.status_line = "Sent stop signal to active handoff.".into();
            }
            HitAction::RemoveTransfer(idx) => {
                if idx < self.queue.len() {
                    self.queue.remove(idx);
                }
            }
            HitAction::SubmitInput => self.submit_input(),
        }
    }

    fn submit_input(&mut self) {
        let trimmed = self.input.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        let bytes = trimmed.into_bytes();
        self.input.clear();
        let cookie = self.next_cookie;
        self.next_cookie = self.next_cookie.wrapping_add(1).max(2);
        self.launch_line(&bytes, cookie, false, false, false);
    }

    fn key(&mut self, key: sex_pdx::app_key::Key) -> bool {
        match key.code {
            ENTER => {
                self.submit_input();
                true
            }
            ESC => {
                if !self.input.is_empty() {
                    self.input.clear();
                    return true;
                }
                false
            }
            BACKSPACE => {
                if self.input.pop().is_some() {
                    return true;
                }
                false
            }
            _ => {
                if let Some(c) = sex_app::text(key).filter(|c| !c.is_control()) {
                    if self.input.len() < 120 {
                        self.input.push(c);
                        return true;
                    }
                }
                false
            }
        }
    }

    fn render(&mut self, window: &mut Window) {
        self.hits.clear();
        let (w, h) = window_dims(window);
        let iw = w as i32;
        let ih = h as i32;
        let buf = window.frame(w, h);
        buf.fill(BG);

        // Top Header Bar (48px)
        fill_rect(buf, w, h, 0, 0, iw, 48, HEADER);
        fill_rect(buf, w, h, 0, 47, iw, 1, BORDER);
        self.painter.text(buf, w, h, false, 18, 14, 120, 15, "SHUTTLE", TEXT);
        if iw >= 520 {
            self.painter.text(buf, w, h, false, 98, 16, 190, 12, "AirDrop & Handoff", MUTED);
        }

        // Right Toolbar Toggle & Primary Rose CTA
        let recv_lbl = if self.receive_mode { "Receive Mode: On" } else { "Receive Mode: Off" };
        let recv_style = if self.receive_mode { ButtonStyle::Active } else { ButtonStyle::Normal };
        self.push_button(buf, w, h, iw - 266, 10, 134, 28, recv_lbl, recv_style, HitAction::ToggleReceive);
        self.push_button(buf, w, h, iw - 124, 10, 108, 28, "Send File…", ButtonStyle::Primary, HitAction::SendFile);

        let body_y = 58;
        let bottom_bar_y = ih - 52;
        let body_h = (bottom_bar_y - body_y - 10).max(140);

        let split = iw >= 660;
        let radar_w = if split { (iw - 48) * 42 / 100 } else { iw - 36 };
        let radar_h = if split { body_h } else { (body_h * 45 / 100).max(120) };

        // Left Card: Peer Discovery Radar
        draw_card(buf, w, h, 18, body_y, radar_w, radar_h, 12, CARD, BORDER);
        self.painter.text(buf, w, h, false, 34, body_y + 12, radar_w - 130, 13, "Peer Discovery Radar", TEXT);
        let (rfg, rbg, rbrd, rtxt) = if self.receive_mode {
            (EMERALD, EMERALD_BG, EMERALD_BORDER, "Discoverable")
        } else {
            (DIM, WELL, BORDER, "Paused")
        };
        self.painter.pill(buf, w, h, 18 + radar_w - 108, body_y + 10, 20, false, rtxt, rfg, rbg, rbrd);

        let rcx = 18 + radar_w / 2;
        let rcy = body_y + 38 + (radar_h - 68) / 2;
        let max_r = ((radar_w.min(radar_h - 68)) / 2 - 14).max(32);
        draw_ring(buf, w, h, rcx, rcy, max_r, BORDER);
        draw_ring(buf, w, h, rcx, rcy, max_r * 2 / 3, BORDER);
        draw_ring(buf, w, h, rcx, rcy, max_r / 3, BORDER_ACTIVE);
        if self.receive_mode {
            let pulse_r = 14 + ((self.radar_phase as i32 * 4) % (max_r - 12).max(10));
            draw_ring(buf, w, h, rcx, rcy, pulse_r, ROSE);
        }
        // Crosshairs
        fill_rect(buf, w, h, rcx - max_r, rcy, max_r * 2, 1, 0x001C_2230);
        fill_rect(buf, w, h, rcx, rcy - max_r, 1, max_r * 2, 0x001C_2230);
        // Center Beacon
        fill_rounded_rect(buf, w, h, rcx - 8, rcy - 8, 16, 16, 8, ROSE);

        // Discovered Local Peers
        for (i, peer) in PEERS.iter().enumerate() {
            let px = rcx + (peer.dx * max_r / 90);
            let py = rcy + (peer.dy * max_r / 90);
            let sel = i == self.selected_peer;
            let nw = 74;
            let nh = 26;
            let bx = (px - nw / 2).clamp(24, 18 + radar_w - nw - 6);
            let by = (py - nh / 2).clamp(body_y + 38, body_y + radar_h - nh - 24);
            draw_card(
                buf,
                w,
                h,
                bx,
                by,
                nw,
                nh,
                8,
                if sel { CARD_ALT } else { WELL },
                if sel { ROSE } else { BORDER_ACTIVE },
            );
            fill_rounded_rect(buf, w, h, bx + 6, by + 9, 8, 8, 4, peer.color);
            self.painter.text(buf, w, h, false, bx + 18, by + 7, nw - 22, 10, peer.name, if sel { TEXT } else { MUTED });
            self.hits.push(HitBox { x: bx, y: by, w: nw, h: nh, action: HitAction::SelectPeer(i) });
        }
        self.painter.text(
            buf,
            w,
            h,
            false,
            34,
            body_y + radar_h - 24,
            radar_w - 68,
            11,
            &format!("Target: {} · {}", PEERS[self.selected_peer].name, PEERS[self.selected_peer].role),
            MUTED,
        );

        // Right Card: Transfer & Handoff Queue
        let (qx, qy, qw, qh) = if split {
            (18 + radar_w + 12, body_y, iw - 36 - radar_w - 12, body_h)
        } else {
            (18, body_y + radar_h + 10, iw - 36, (body_h - radar_h - 10).max(90))
        };
        draw_card(buf, w, h, qx, qy, qw, qh, 12, CARD, BORDER);
        self.painter.text(buf, w, h, false, qx + 16, qy + 12, qw - 110, 13, "Transfer & Handoff Queue", TEXT);
        self.painter.pill(
            buf,
            w,
            h,
            qx + qw - 88,
            qy + 10,
            20,
            true,
            &format!("{} item{}", self.queue.len(), if self.queue.len() == 1 { "" } else { "s" }),
            SKY,
            SKY_BG,
            SKY_BORDER,
        );
        fill_rect(buf, w, h, qx + 12, qy + 38, qw - 24, 1, BORDER);

        let mut cy = qy + 46;
        if self.queue.is_empty() {
            self.painter.text(buf, w, h, false, qx + 18, cy + 8, qw - 36, 12, "No active transfers or launches.", MUTED);
            self.painter.text(buf, w, h, false, qx + 18, cy + 30, qw - 36, 11, "Use 'Send File…', drag-and-drop a file, or enter @/path below.", DIM);
        } else {
            let card_h = 74;
            let gap = 8;
            let max_cards = ((qh - 54) / (card_h + gap)).max(1) as usize;
            let start = self.scroll.min(self.queue.len().saturating_sub(1));
            for idx in (start..self.queue.len()).take(max_cards) {
                let c = &self.queue[idx];
                let (s_lbl, fg, sbg, brd, bar_col) = match c.status {
                    TransferStatus::Launching => ("Launching", ROSE, ROSE_BG, ROSE_BORDER, ROSE),
                    TransferStatus::Active => ("Active", EMERALD, EMERALD_BG, EMERALD_BORDER, EMERALD),
                    TransferStatus::Completed(0) => ("Done (0)", EMERALD, EMERALD_BG, EMERALD_BORDER, EMERALD),
                    TransferStatus::Completed(_) => ("Done", SKY, SKY_BG, SKY_BORDER, SKY),
                    TransferStatus::Ended => ("Ended", MUTED, WELL, BORDER, MUTED),
                    TransferStatus::Refused => ("Refused", ROSE, ROSE_BG, ROSE_BORDER, ROSE),
                };
                draw_card(buf, w, h, qx + 12, cy, qw - 24, card_h, 8, CARD_ALT, BORDER_ACTIVE);
                self.painter.text(buf, w, h, true, qx + 24, cy + 10, qw - 180, 12, &c.label, TEXT);
                self.painter.pill(buf, w, h, qx + qw - 112, cy + 8, 20, false, s_lbl, fg, sbg, brd);
                self.painter.text(buf, w, h, false, qx + 24, cy + 30, qw - 130, 11, &format!("{} · {}", c.target_peer, c.detail), MUTED);

                // Progress bar
                let bar_w = (qw - 134).max(60);
                fill_rounded_rect(buf, w, h, qx + 24, cy + 52, bar_w, 8, 4, WELL);
                let fw = ((bar_w as u32 * c.progress as u32) / 100).max(8) as i32;
                fill_rounded_rect(buf, w, h, qx + 24, cy + 52, fw.min(bar_w), 8, 4, bar_col);

                let cookie = c.cookie;
                let active = matches!(c.status, TransferStatus::Active | TransferStatus::Launching);
                if active {
                    self.push_button(buf, w, h, qx + qw - 98, cy + 44, 72, 22, "End", ButtonStyle::Danger, HitAction::KillTransfer(cookie));
                } else {
                    self.push_button(buf, w, h, qx + qw - 98, cy + 44, 72, 22, "Clear", ButtonStyle::Subtle, HitAction::RemoveTransfer(idx));
                }
                cy += card_h + gap;
            }
        }

        // Bottom Command / Path Bar
        let inp_w = (iw - 144).max(120);
        draw_card(buf, w, h, 18, bottom_bar_y, inp_w, 34, 8, WELL, BORDER_ACTIVE);
        if self.input.is_empty() {
            self.painter.text(buf, w, h, true, 30, bottom_bar_y + 10, inp_w - 24, 12, "Type @/path/to/file or program args…", DIM);
        } else {
            self.painter.text(buf, w, h, true, 30, bottom_bar_y + 10, inp_w - 24, 12, &format!("{}_", self.input), TEXT);
        }
        self.push_button(buf, w, h, 18 + inp_w + 10, bottom_bar_y + 2, 98, 30, "Send (Ret)", ButtonStyle::Primary, HitAction::SubmitInput);
        let _ = &self.status_line;

        window.show_pixels(w, h);
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"Shuttle");
    let mut args = [0u8; sex_pdx::fs::ARGS_MAX];
    let raw_line = files::args(&mut window, &mut args).to_vec();
    let line = raw_line.as_slice();

    let (kill, line) = match line.strip_prefix(b"-k ") { Some(rest) => (true, rest), None => (false, line) };
    let (all, line) = match line.strip_prefix(b"-a ") { Some(rest) => (true, rest), None => (false, line) };
    let (hidden, line) = match line.strip_prefix(b"-h ") { Some(rest) => (true, rest), None => (false, line) };

    let mut app = App::new();
    const COOKIE: u32 = 1;
    if !line.is_empty() {
        let ok = app.launch_line(line, COOKIE, kill, all, hidden);
        app.render(&mut window);
        if ok {
            loop {
                let ev = launch::wait(COOKIE);
                let done = app.apply_launch_event(ev);
                app.render(&mut window);
                if done {
                    break;
                }
            }
        }
    } else {
        app.render(&mut window);
    }

    loop {
        while let Some(ev) = launch::poll_where(|_| true) {
            app.apply_launch_event(ev);
            app.render(&mut window);
        }
        match window.next_within(140) {
            Some(Event::Close) => window.close(),
            Some(Event::Resize { .. }) => app.render(&mut window),
            Some(Event::Key(k)) => {
                if app.key(k) {
                    app.render(&mut window);
                }
            }
            Some(Event::Pointer(ptr)) => {
                if app.pointer(&mut window, ptr) {
                    app.render(&mut window);
                }
            }
            Some(Event::Drop { kind, .. }) => {
                if app.receive_mode && kind == sex_pdx::dnd::FILE {
                    let dropped = dnd::dropped().to_vec();
                    if !dropped.is_empty() {
                        let mut spec = Vec::new();
                        if !dropped.starts_with(b"@") {
                            spec.push(b'@');
                        }
                        spec.extend_from_slice(&dropped);
                        let cookie = app.next_cookie;
                        app.next_cookie = app.next_cookie.wrapping_add(1).max(2);
                        app.launch_line(&spec, cookie, false, false, false);
                        app.render(&mut window);
                    }
                }
            }
            None => {
                if app.receive_mode {
                    app.radar_phase = app.radar_phase.wrapping_add(1);
                    app.render(&mut window);
                }
            }
            _ => {}
        }
    }
}
