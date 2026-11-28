// SPDX-License-Identifier: GPL-3.0-only
//! Screen sharing (`sex_pdx::screen_share`): Display draws the Shell's picker hint
//! over the window overview, rings what is shared in red with a "Stop sharing" pill,
//! and copies the shared window's own pixels (or the whole screen) into the bank the
//! program rows read. Only the Shell says what is shared and with whom.
use crate::{alpha_blend_xrgb_over_xrgb, desk_theme, glyphs, scene, ui_label_at_scaled};
use sex_pdx::screen_share::{self as s, View};

const RED: u32 = 0x00E5_3935;
const RED_DARK: u32 = 0x00B0_2020;

pub struct Server {
    pub view: View,
    bank: Option<s::Bank>,
    opened: bool,
    started: bool,
    last: u64,
    frames: u32,
}

impl Server {
    pub const fn new() -> Self { Self { view: View::Hidden, bank: None, opened: false, started: false, last: 0, frames: 0 } }

    /// The Shell's `OP_SHARE_VIEW`. True when the screen must be drawn again.
    pub fn receive(&mut self, caller: u32, w: [u64; 3]) -> bool {
        if caller != 3 { return false; }
        let Some(view) = View::decode(w) else { return false };
        if view == self.view { return false; }
        if !self.opened {
            self.opened = true;
            #[cfg(target_os = "none")]
            { self.bank = s::Bank::open_writer(); }
        }
        let was = self.view;
        if let (View::Sharing { session: a, .. }, View::Sharing { session: b, .. }) = (was, view) { if a != b { self.end(); } }
        if matches!(was, View::Sharing { .. }) && !matches!(view, View::Sharing { .. }) { self.end(); }
        self.view = view;
        sex_pdx::serial_record!("[display.share.view] view={} surface={} target={} bank={}", w[0], w[1], w[2] >> 32, self.bank.is_some() as u8);
        true
    }

    fn end(&mut self) {
        if let Some(b) = self.bank.as_ref() { if self.started { b.stop(); } }
        if self.started { sex_pdx::serial_record!("[display.share.stopped] frames={} scrubbed=1", self.frames); }
        self.started = false;
        self.frames = 0;
        self.last = 0;
    }

    /// The shared rectangle on screen, if anything is shared.
    fn shared_rect(&self, w: usize, h: usize, scene: &scene::SceneSnapshot) -> Option<(usize, usize, usize, usize)> {
        let View::Sharing { kind, surface, .. } = self.view else { return None };
        if kind == s::SCREEN { return Some((0, 0, w, h)); }
        let surf = scene.surfaces.iter().find(|x| x.active && x.surface_id == surface)?;
        let r = scene.groups.rect(surf, w, h);
        (r.2 > 0 && r.3 > 0).then_some(r)
    }

    /// Over the overview while picking; the red ring and the stop pill while sharing.
    pub fn draw(&self, fb: *mut u32, w: usize, h: usize, total: usize, scene: &scene::SceneSnapshot) {
        let scale = scene.metrics.scale();
        let put = |x: usize, y: usize, c: u32| { let i = y * w + x; if x < w && y < h && i < total { unsafe { core::ptr::write_volatile(fb.add(i), c); } } };
        let text = |t: &[u8], x0: usize, y0: usize, max: usize, bg: u32| {
            let line = glyphs::UI_H * scale as usize;
            let tw = glyphs::ui_width(t, scale).unwrap_or(0).min(max);
            for y in 0..line { for x in 0..tw { if let Some(c) = ui_label_at_scaled(t, x, y, bg, scale) { put(x0 + x, y0 + y, c); } } }
            tw
        };
        let fill = |r: (usize, usize, usize, usize), c: u32| { for y in r.1..r.1 + r.3 { for x in r.0..r.0 + r.2 { put(x, y, c); } } };
        let line = glyphs::UI_H * scale as usize;
        match self.view {
            View::Hidden => {}
            View::Picking { asker } => {
                let Some(p) = s::Picker::new(w, scene.metrics.bar() as usize, scale) else { return };
                let d = desk_theme::current();
                fill(p.pill, d.menu);
                let who = crate::switcher_title(asker);
                let mut msg = [0u8; 160];
                let mut n = 0;
                for part in [who, b" wants to share your screen: click a window, or".as_slice()] {
                    for &b in part { if n < msg.len() { msg[n] = b; n += 1; } }
                }
                let pad = 8 * scale as usize;
                text(&msg[..n], p.pill.0 + pad, p.pill.1 + (p.pill.3.saturating_sub(line)) / 2, p.button.0.saturating_sub(p.pill.0 + 2 * pad), d.menu);
                fill(p.button, d.focus);
                const LABEL: &[u8] = b"Entire screen";
                let tw = glyphs::ui_width(LABEL, scale).unwrap_or(0).min(p.button.2);
                text(LABEL, p.button.0 + (p.button.2 - tw) / 2, p.button.1 + p.button.3.saturating_sub(line) / 2, p.button.2, d.focus);
                unsafe {
                    static mut PICK_LOG: u32 = 8;
                    if PICK_LOG > 0 { PICK_LOG -= 1; sex_pdx::serial_record!("[display.share.picker] pill={}x{}+{}+{} button={}x{}+{}+{}", p.pill.2, p.pill.3, p.pill.0, p.pill.1, p.button.2, p.button.3, p.button.0, p.button.1); }
                }
            }
            View::Sharing { .. } => {
                if let Some((rx, ry, rw, rh)) = self.shared_rect(w, h, scene) {
                    // Inside the shared rectangle, so it moves with the window.
                    let t = (3 * scale as usize).min(rw / 2).min(rh / 2);
                    fill((rx, ry, rw, t), RED); fill((rx, ry + rh - t, rw, t), RED);
                    fill((rx, ry, t, rh), RED); fill((rx + rw - t, ry, t, rh), RED);
                }
                let pill = s::stop_pill(w, h, scale);
                for y in pill.1..pill.1 + pill.3 { for x in pill.0..pill.0 + pill.2 {
                    let i = y * w + x;
                    if i < total { unsafe { let under = core::ptr::read_volatile(fb.add(i)); core::ptr::write_volatile(fb.add(i), alpha_blend_xrgb_over_xrgb(RED_DARK, under, 235)); } }
                } }
                const STOP: &[u8] = b"Sharing your screen - Stop sharing";
                let tw = glyphs::ui_width(STOP, scale).unwrap_or(0).min(pill.2);
                text(STOP, pill.0 + (pill.2 - tw) / 2, pill.1 + pill.3.saturating_sub(line) / 2, pill.2, RED_DARK);
            }
        }
    }

    /// After a CPU publication: copies the shared pixels (fitted) into the bank, at
    /// most every `INTERVAL_MS`.
    pub fn published(&mut self, w: usize, h: usize, front: &[u32], scene: &scene::SceneSnapshot, now: u64) {
        let View::Sharing { kind, surface, target, session } = self.view else { return };
        let Some(bank) = self.bank else { return };
        if self.started && now.saturating_sub(self.last) < s::INTERVAL_MS { return; }
        let Some((rx, ry, rw, rh)) = self.shared_rect(w, h, scene) else { return };
        if !self.started {
            let title: &[u8] = if kind == s::SCREEN { b"Screen" } else { crate::switcher_title(surface) };
            bank.start(target, kind, session, title);
            self.started = true;
            sex_pdx::serial_record!("[display.share.started] kind={} surface={} target={} session={} w={} h={}", kind, surface, target, session, rw, rh);
        }
        self.last = now;
        let (fw, fh) = s::fit(rw, rh);
        if fw == 0 || front.len() < w * h { return; }
        let held = unsafe { crate::APP_PRESENT_BITS };
        let seq = bank.publish(fw, fh, |px| {
            for y in 0..fh {
                let sy = ry + y * rh / fh;
                for x in 0..fw {
                    let sx = rx + x * rw / fw;
                    px[y * fw + x] = if kind == s::SCREEN { front[sy * w + sx] } else {
                        // The window's own pixels, even where another window covers it.
                        crate::composite_pixel_selected(sx, sy, w, h, 0, surface, Some(surface), scene)
                    };
                }
            }
        });
        unsafe { crate::APP_PRESENT_BITS = held; }
        if seq.is_some() {
            self.frames += 1;
            if self.frames <= 3 || self.frames.is_power_of_two() { sex_pdx::serial_record!("[display.share.frame] n={} w={} h={}", self.frames, fw, fh); }
        }
    }
}
