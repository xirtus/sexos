// SPDX-License-Identifier: GPL-3.0-only
//! Native desktop window backend for macOS (Cocoa / CoreGraphics) and Linux
//! (Wayland / X11) using pure-Rust `winit` + `softbuffer`.
//!
//! Polyamour already renders its entire UI and 3D viewports into
//! `app.painter.pixels` (`0x00RRGGBB`), matching `softbuffer`'s surface format
//! directly. When idle, the event loop sleeps with `ControlFlow::Wait`; while
//! progressive path-tracing, physics, animation playback, or `bpy.app.timers`
//! are active, it ticks on a frame timer.

use crate::app::App;
use crate::doc::Mode;
use crate::input::{Button, Input, Key, Mods};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WinitKey, KeyLocation, ModifiersState, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

struct DesktopApp {
    app: App,
    window: Option<Arc<Window>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    mods: Mods,
    cursor: (i32, i32),
    scale: f64,
    native_hidpi: bool,
    emulated_middle: bool,
    scroll_accum: f64,
    last_anim_step: Instant,
}

impl DesktopApp {
    fn new(app: App) -> Self {
        let native_hidpi = std::env::var("POLYAMOUR_HIDPI")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        Self {
            app,
            window: None,
            context: None,
            surface: None,
            mods: Mods::NONE,
            cursor: (0, 0),
            scale: 1.0,
            native_hidpi,
            emulated_middle: false,
            scroll_accum: 0.0,
            last_anim_step: Instant::now(),
        }
    }

    fn eff_scale(&self) -> f64 {
        if self.native_hidpi {
            1.0
        } else {
            self.scale.max(1.0)
        }
    }

    fn logical_dims(&self, phys: PhysicalSize<u32>) -> (usize, usize) {
        let s = self.eff_scale();
        let w = ((phys.width as f64 / s).round() as usize).max(320);
        let h = ((phys.height as f64 / s).round() as usize).max(240);
        (w, h)
    }

    fn present(&mut self) {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return;
        };
        let phys = window.inner_size();
        let (Some(pw), Some(ph)) = (NonZeroU32::new(phys.width), NonZeroU32::new(phys.height)) else {
            return;
        };
        if surface.resize(pw, ph).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else {
            return;
        };
        let (sw, sh) = (self.app.painter.width, self.app.painter.height);
        let (dw, dh) = (pw.get() as usize, ph.get() as usize);
        let src = &self.app.painter.pixels;
        if sw == dw && sh == dh {
            let n = buf.len().min(src.len());
            buf[..n].copy_from_slice(&src[..n]);
        } else if sw > 0 && sh > 0 && dw > 0 && dh > 0 {
            // Fast nearest-neighbor blit for HiDPI / Retina surfaces so 1x UI metrics stay crisp.
            for dy in 0..dh {
                let sy = (dy * sh / dh).min(sh - 1);
                let src_row = &src[sy * sw..(sy + 1) * sw];
                let dst_row = &mut buf[dy * dw..(dy + 1) * dw];
                if dw == sw * 2 {
                    for (sx, &px) in src_row.iter().enumerate() {
                        dst_row[sx * 2] = px;
                        dst_row[sx * 2 + 1] = px;
                    }
                } else {
                    for (dx, out) in dst_row.iter_mut().enumerate() {
                        *out = src_row[(dx * sw / dw).min(sw - 1)];
                    }
                }
            }
        }
        let _ = buf.present();
    }

    fn dispatch(&mut self, input: Input) {
        self.app.frame(input);
        if let Some(w) = &self.window {
            let dirty = if self.app.doc.dirty { "*" } else { "" };
            let file = self.app.doc.path.as_deref().unwrap_or("Unsaved");
            w.set_title(&format!("Polyamour — {file}{dirty}"));
            w.request_redraw();
        }
    }
}

impl ApplicationHandler for DesktopApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title("Polyamour")
            .with_inner_size(LogicalSize::new(
                self.app.painter.width as f64,
                self.app.painter.height as f64,
            ))
            .with_min_inner_size(LogicalSize::new(640.0, 400.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("polyamour: failed to create window: {e}");
                event_loop.exit();
                return;
            }
        };
        self.scale = window.scale_factor();
        let (lw, lh) = self.logical_dims(window.inner_size());
        if (lw, lh) != (self.app.painter.width, self.app.painter.height) {
            self.app.frame(Input::Resize { width: lw, height: lh });
        } else {
            self.app.frame(Input::Tick);
        }
        let context = match softbuffer::Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("polyamour: failed to create display context: {e}");
                event_loop.exit();
                return;
            }
        };
        let surface = match softbuffer::Surface::new(&context, window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("polyamour: failed to create window surface: {e}");
                event_loop.exit();
                return;
            }
        };
        window.request_redraw();
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.app.quit = true;
                event_loop.exit();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor;
                if let Some(w) = &self.window {
                    let (lw, lh) = self.logical_dims(w.inner_size());
                    self.dispatch(Input::Resize { width: lw, height: lh });
                }
            }
            WindowEvent::Resized(phys) => {
                if phys.width > 0 && phys.height > 0 {
                    let (lw, lh) = self.logical_dims(phys);
                    self.dispatch(Input::Resize { width: lw, height: lh });
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = map_mods(m.state());
            }
            WindowEvent::CursorMoved { position: PhysicalPosition { x, y }, .. } => {
                let s = self.eff_scale();
                let pos = ((x / s).round() as i32, (y / s).round() as i32);
                self.cursor = pos;
                self.dispatch(Input::Move { x: pos.0, y: pos.1 });
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let (x, y) = self.cursor;
                match state {
                    ElementState::Pressed => {
                        // Emulate 3-button mouse on trackpads (Alt/Option + Left -> Middle),
                        // except in Sculpt / WeightPaint where Alt + Left is subtract brush.
                        let allow_emulate = !matches!(self.app.doc.mode, Mode::Sculpt | Mode::WeightPaint);
                        self.emulated_middle = button == MouseButton::Left && self.mods.alt && allow_emulate;
                        let btn = if self.emulated_middle {
                            Button::Middle
                        } else {
                            match button {
                                MouseButton::Left => Button::Left,
                                MouseButton::Middle => Button::Middle,
                                MouseButton::Right => Button::Right,
                                _ => return,
                            }
                        };
                        let mods = if self.emulated_middle {
                            Mods { alt: false, ..self.mods }
                        } else {
                            self.mods
                        };
                        self.dispatch(Input::Down { x, y, button: btn, mods });
                    }
                    ElementState::Released => {
                        let btn = if button == MouseButton::Left && self.emulated_middle {
                            self.emulated_middle = false;
                            Button::Middle
                        } else {
                            match button {
                                MouseButton::Left => Button::Left,
                                MouseButton::Middle => Button::Middle,
                                MouseButton::Right => Button::Right,
                                _ => return,
                            }
                        };
                        self.dispatch(Input::Up { x, y, button: btn });
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let steps = match delta {
                    MouseScrollDelta::LineDelta(_, dy) => {
                        if dy == 0.0 {
                            0
                        } else if dy.abs() < 1.0 {
                            dy.signum() as i32
                        } else {
                            dy.round() as i32
                        }
                    }
                    MouseScrollDelta::PixelDelta(pos) => {
                        self.scroll_accum += pos.y / (24.0 * self.eff_scale());
                        let s = self.scroll_accum.trunc() as i32;
                        self.scroll_accum -= s as f64;
                        s
                    }
                };
                if steps != 0 {
                    let (x, y) = self.cursor;
                    self.dispatch(Input::Wheel { x, y, steps, mods: self.mods });
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if let Some(key) = map_key(&event.logical_key, event.location) {
                    let text = if !self.mods.ctrl && !self.mods.alt {
                        event
                            .text
                            .as_ref()
                            .and_then(|s| s.chars().next())
                            .filter(|c| !c.is_control())
                    } else {
                        None
                    };
                    self.dispatch(Input::Key { key, mods: self.mods, text });
                }
            }
            WindowEvent::RedrawRequested => {
                self.present();
            }
            _ => {}
        }
        if self.app.quit {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.quit {
            event_loop.exit();
            return;
        }
        if self.app.doc.playing {
            let step = Duration::from_millis(1000 / 24);
            let now = Instant::now();
            if now.duration_since(self.last_anim_step) >= step {
                self.last_anim_step = now;
                let d = &mut self.app.doc;
                d.frame = if d.frame >= d.scene.frame_end {
                    d.scene.frame_start
                } else {
                    d.frame + 1
                };
                self.dispatch(Input::Tick);
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.last_anim_step + step));
        } else if self.app.wants_tick() {
            self.dispatch(Input::Tick);
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(12)));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn map_mods(s: ModifiersState) -> Mods {
    #[cfg(target_os = "macos")]
    let ctrl = s.control_key() || s.super_key();
    #[cfg(not(target_os = "macos"))]
    let ctrl = s.control_key();
    Mods {
        shift: s.shift_key(),
        ctrl,
        alt: s.alt_key(),
    }
}

fn map_key(logical: &WinitKey, location: KeyLocation) -> Option<Key> {
    if location == KeyLocation::Numpad {
        match logical {
            WinitKey::Named(NamedKey::Enter) => return Some(Key::NumEnter),
            WinitKey::Character(s) => match s.as_str() {
                "0" => return Some(Key::Num(0)),
                "1" => return Some(Key::Num(1)),
                "2" => return Some(Key::Num(2)),
                "3" => return Some(Key::Num(3)),
                "4" => return Some(Key::Num(4)),
                "5" => return Some(Key::Num(5)),
                "6" => return Some(Key::Num(6)),
                "7" => return Some(Key::Num(7)),
                "8" => return Some(Key::Num(8)),
                "9" => return Some(Key::Num(9)),
                "." | "," => return Some(Key::NumDot),
                "+" => return Some(Key::NumPlus),
                "-" => return Some(Key::NumMinus),
                "*" => return Some(Key::NumStar),
                "/" => return Some(Key::NumSlash),
                _ => {}
            },
            _ => {}
        }
    }
    match logical {
        WinitKey::Named(n) => match n {
            NamedKey::Escape => Some(Key::Esc),
            NamedKey::Tab => Some(Key::Tab),
            NamedKey::Enter => Some(Key::Enter),
            NamedKey::Space => Some(Key::Space),
            NamedKey::Backspace => Some(Key::Backspace),
            NamedKey::Delete => Some(Key::Delete),
            NamedKey::Home => Some(Key::Home),
            NamedKey::End => Some(Key::End),
            NamedKey::PageUp => Some(Key::PageUp),
            NamedKey::PageDown => Some(Key::PageDown),
            NamedKey::ArrowUp => Some(Key::Up),
            NamedKey::ArrowDown => Some(Key::Down),
            NamedKey::ArrowLeft => Some(Key::Left),
            NamedKey::ArrowRight => Some(Key::Right),
            NamedKey::F1 => Some(Key::F(1)),
            NamedKey::F2 => Some(Key::F(2)),
            NamedKey::F3 => Some(Key::F(3)),
            NamedKey::F4 => Some(Key::F(4)),
            NamedKey::F5 => Some(Key::F(5)),
            NamedKey::F6 => Some(Key::F(6)),
            NamedKey::F7 => Some(Key::F(7)),
            NamedKey::F8 => Some(Key::F(8)),
            NamedKey::F9 => Some(Key::F(9)),
            NamedKey::F10 => Some(Key::F(10)),
            NamedKey::F11 => Some(Key::F(11)),
            NamedKey::F12 => Some(Key::F(12)),
            _ => None,
        },
        WinitKey::Character(s) => {
            let c = s.chars().next()?;
            Some(Key::Char(c.to_ascii_lowercase()))
        }
        _ => None,
    }
}

/// Open a native interactive desktop window for `app` on macOS or Linux and run
/// until the user closes the window or triggers `Quit` (`Ctrl+Q` / `Cmd+Q`).
pub fn run(app: App) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| format!("cannot start event loop: {e}"))?;
    let mut state = DesktopApp::new(app);
    event_loop.run_app(&mut state).map_err(|e| format!("window loop error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_mapping_distinguishes_numpad_and_named_keys() {
        assert_eq!(map_key(&WinitKey::Character("1".into()), KeyLocation::Numpad), Some(Key::Num(1)));
        assert_eq!(map_key(&WinitKey::Character("1".into()), KeyLocation::Standard), Some(Key::Char('1')));
        assert_eq!(map_key(&WinitKey::Character(".".into()), KeyLocation::Numpad), Some(Key::NumDot));
        assert_eq!(map_key(&WinitKey::Named(NamedKey::F12), KeyLocation::Standard), Some(Key::F(12)));
        assert_eq!(map_key(&WinitKey::Character("G".into()), KeyLocation::Standard), Some(Key::Char('g')));
    }
}
