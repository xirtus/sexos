// SPDX-License-Identifier: GPL-3.0-only
//! Glue to a SexOS program window: Shell's keys and pointer in, pixels out.
//!
//! ```ignore
//! let mut app = weft::sexos::App::open("Abacus", Theme::light(), Strings::new("en"));
//! let ok = app.ui.button(app.ui.root(), "OK");
//! loop { for r in app.next() { if r == Response::Clicked(ok) { /* … */ } } }
//! ```
use alloc::vec::Vec;

use sex_app::{Event as AppEvent, Window};
use sex_pdx::app_pointer::Kind as PKind;

use crate::i18n::Strings;
use crate::input::{Button, Event, Key, Pointer, PointerKind};
use crate::theme::Theme;
use crate::ui::Ui;
use crate::Response;

/// Milliseconds since boot.
pub fn now() -> u64 { sex_pdx::get_ticks() }

/// Shell's event as Weft's; `None` for what Weft has no event for yet (drops and the
/// end of a drag this window started, read with `sex_app::dnd`).
pub fn event(e: AppEvent) -> Option<Event> {
    Some(match e {
        AppEvent::Drop { .. } | AppEvent::DragEnded { .. } => return None,
        AppEvent::Key(k) => Event::Key(Key { code: k.code, shift: k.shift, control: k.control, text: k.char() }),
        AppEvent::Close => Event::Close,
        AppEvent::Resize { width, height } => Event::Resize { width: width as u32, height: height as u32 },
        AppEvent::Drop { kind, x, y } => Event::Drop { kind, x, y },
        AppEvent::DragEnded { target } => Event::DragEnded { target },
        AppEvent::Pointer(p) => {
            let button = |b: u8| match b { 1 => Button::Middle, 2 => Button::Right, _ => Button::Left };
            let kind = match p.kind {
                PKind::Move => PointerKind::Move,
                PKind::Down { button: b } => PointerKind::Down(button(b)),
                PKind::Up { button: b } => PointerKind::Up(button(b)),
                PKind::Wheel { steps } => PointerKind::Wheel(steps),
                PKind::Leave => PointerKind::Leave,
            };
            Event::Pointer(Pointer { x: p.x as f32, y: p.y as f32, kind })
        }
    })
}

/// A window with a Weft interface.
pub struct App {
    pub window: Window,
    pub ui: Ui,
    /// Frames painted, for gates and logs.
    pub frames: u64,
}

impl App {
    pub fn open(title: &str, theme: Theme, strings: Strings) -> App {
        let mut window = Window::open();
        window.title(title.as_bytes());
        // The screen reader gets the tree only if the person turned one on.
        sex_app::a11y::ask();
        let (w, h) = window.size();
        let ui = Ui::new(w as u32, h as u32, theme, strings);
        App { window, ui, frames: 0 }
    }

    /// Paints if anything changed and shows the pixels.
    pub fn present(&mut self) {
        if !self.ui.needs_paint() { return; }
        let (w, h) = self.ui.size();
        let pixels = self.window.pixels();
        if pixels.len() < (w * h) as usize { return; }
        if self.ui.paint(&mut pixels[..(w * h) as usize]).is_some() {
            self.window.show_pixels(w as usize, h as usize);
            self.frames += 1;
        }
    }

    /// Paints, waits for input or a timer, and returns what the person did (possibly
    /// nothing, after a timer).
    pub fn next(&mut self) -> Vec<Response> {
        self.present();
        self.a11y_send();
        let mut wait = self.ui.tick(now());
        // Shell's answer to "is a reader on" arrives with no event of its own: look
        // again now and then while none is, so the tree goes out soon after one starts.
        if sex_app::a11y::asked() && !sex_app::a11y::on() { wait = Some(wait.map_or(A11Y_LOOK_MS, |w| w.min(A11Y_LOOK_MS))); }
        let e = match wait { Some(ms) => self.window.next_within(ms.max(1)), None => Some(self.window.next()) };
        self.ui.tick(now());
        let mut out = Vec::new();
        if let Some(e) = e {
            if let Some(e) = event(e) { out.extend(self.ui.handle(e)); }
            // Take whatever else is already waiting before painting again.
            for _ in 0..64 {
                let Some(e) = self.window.poll() else { break };
                if let Some(e) = event(e) { out.extend(self.ui.handle(e)); }
            }
        }
        self.present();
        self.a11y_send();
        out
    }

    /// While a screen reader is on: the whole tree when it starts, then what changed
    /// (`sex_pdx::a11y`). Nothing is built or sent while none is.
    pub fn a11y_send(&mut self) {
        let changed = sex_app::a11y::changed();
        if !sex_app::a11y::on() { return; }
        let update = if changed { Some(self.ui.a11y_tree()) } else { self.ui.a11y_update() };
        if let Some(u) = update {
            let bytes = crate::a11y::encode(&u);
            if !sex_app::a11y::send_update(&bytes) { sex_pdx::serial_record!("[weft.a11y.send.failed] bytes={}", bytes.len()); }
        }
    }
}

/// How often a window looks for Shell's "a reader is on" while none is (ms).
const A11Y_LOOK_MS: u64 = 500;
