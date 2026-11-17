//! Pure-Rust ports of **tuna** (`NowPlayingCard`) and **input-overlay** (`InputOverlayHud`)
//! for Cabaret.
//!
//! - [`NowPlayingCard`]: decodes `Bell` `LAST_PLAYER` mixer stream words and renders title,
//!   artist, player/Qupid status, optional cover art, and live progress bar to [`FrameBuffer`].
//! - [`InputOverlayHud`]: renders live WASD, modifier keys, mouse buttons, and smoothed
//!   mouse movement vector visualization directly to [`FrameBuffer`].

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use cabaret_core::compositor::{scale_frame, CompositorError, FrameBuffer, LinearRgba, Scaler};

use crate::util::{
    absf, clamp01, cosf, draw_line, draw_text, fill_circle, fill_rounded_rect, lerp, sinf, sqrtf,
    stroke_rect,
};

const BELL_MIXER_TAG: u64 = 0x4D << 56;
const BELL_MIXER_REFUSED: u64 = BELL_MIXER_TAG | (1 << 55);

/// Decoded state from a `Bell` `OP_MIXER` / `LAST_PLAYER` stream word (`sex_pdx::sound_mixer::Stream`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BellLastPlayer {
    pub pd: u32,
    pub volume_pct: u8,
    pub muted: bool,
    pub playing: bool,
}

impl BellLastPlayer {
    /// Decodes a raw 64-bit reply word returned by `Bell` (`LAST_PLAYER` / `STREAM`).
    /// Returns `None` if the word lacks the Bell `0x4D` tag or equals `REFUSED`.
    pub const fn from_bell_word(word: u64) -> Option<Self> {
        if (word >> 56) != (BELL_MIXER_TAG >> 56) || word == BELL_MIXER_REFUSED {
            return None;
        }
        Some(Self {
            volume_pct: (word & 0xFF) as u8,
            muted: (word & (1 << 8)) != 0,
            playing: (word & (1 << 9)) != 0,
            pd: ((word >> 32) & 0xFF_FFFF) as u32,
        })
    }
}

/// Metadata + playback snapshot rendered by [`NowPlayingCard`].
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlayingTrack {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub player_app: String,
    pub player_pd: u32,
    pub duration_ms: u64,
    pub position_ms: u64,
    pub is_playing: bool,
    pub muted: bool,
    pub volume_pct: u8,
    pub cover_art: Option<FrameBuffer>,
}

impl Default for NowPlayingTrack {
    fn default() -> Self {
        Self {
            title: String::from("No Track Playing"),
            artist: String::from("Unknown Artist"),
            album: String::new(),
            player_app: String::from("Qupid"),
            player_pd: 0,
            duration_ms: 0,
            position_ms: 0,
            is_playing: false,
            muted: false,
            volume_pct: 100,
            cover_art: None,
        }
    }
}

/// Visual styling for [`NowPlayingCard`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NowPlayingStyle {
    pub width: u32,
    pub height: u32,
    pub bg_color: LinearRgba,
    pub border_color: LinearRgba,
    pub accent_color: LinearRgba,
    pub track_bg_color: LinearRgba,
    pub text_color: LinearRgba,
    pub subtext_color: LinearRgba,
    pub corner_radius_px: f32,
}

impl Default for NowPlayingStyle {
    fn default() -> Self {
        Self {
            width: 440,
            height: 112,
            bg_color: [0.03, 0.03, 0.05, 0.90],
            border_color: [0.18, 0.20, 0.28, 0.95],
            accent_color: [0.95, 0.22, 0.52, 1.0],
            track_bg_color: [0.12, 0.12, 0.16, 1.0],
            text_color: [0.98, 0.98, 1.0, 1.0],
            subtext_color: [0.62, 0.66, 0.74, 1.0],
            corner_radius_px: 10.0,
        }
    }
}

/// Tuna-compatible Now Playing card synced with `Bell` / `Qupid` `LAST_PLAYER`.
#[derive(Clone, Debug, Default)]
pub struct NowPlayingCard {
    pub track: NowPlayingTrack,
    pub style: NowPlayingStyle,
}

impl NowPlayingCard {
    pub fn new(track: NowPlayingTrack, style: NowPlayingStyle) -> Self {
        Self { track, style }
    }

    /// Updates player status (`pd`, `is_playing`, `muted`, `volume_pct`) from a `Bell`
    /// `LAST_PLAYER` stream word.
    pub fn apply_bell_last_player(&mut self, word: u64, app_name: Option<&str>) -> bool {
        let Some(info) = BellLastPlayer::from_bell_word(word) else {
            return false;
        };
        self.track.player_pd = info.pd;
        self.track.is_playing = info.playing;
        self.track.muted = info.muted;
        self.track.volume_pct = info.volume_pct.min(100);
        if let Some(name) = app_name {
            self.track.player_app = String::from(name);
        }
        true
    }

    /// Advances playback position smoothly when `is_playing` is `true`.
    pub fn advance_time(&mut self, dt_ms: u64) {
        if self.track.is_playing {
            let next = self.track.position_ms.saturating_add(dt_ms);
            self.track.position_ms = if self.track.duration_ms > 0 {
                next.min(self.track.duration_ms)
            } else {
                next
            };
        }
    }

    /// Normalized playback progress `0.0..=1.0`.
    pub fn progress_ratio(&self) -> f32 {
        if self.track.duration_ms == 0 {
            0.0
        } else {
            clamp01(self.track.position_ms as f32 / self.track.duration_ms as f32)
        }
    }

    /// Renders the Now Playing card onto a fresh [`FrameBuffer`].
    pub fn render(&self) -> Result<FrameBuffer, CompositorError> {
        let w = self.style.width.max(120);
        let h = self.style.height.max(64);
        let mut fb = FrameBuffer::new(w, h)?;

        fill_rounded_rect(
            &mut fb,
            0.0,
            0.0,
            w as f32,
            h as f32,
            self.style.corner_radius_px,
            self.style.bg_color,
        );
        stroke_rect(&mut fb, 0, 0, w as i32, h as i32, 1, self.style.border_color);

        // Cover art slot on left.
        let art_pad = 12u32;
        let art_size = h.saturating_sub(art_pad * 2).min(88);
        if let Some(ref art) = self.track.cover_art {
            if let Ok(thumb) = scale_frame(art, art_size, art_size, Scaler::Bilinear) {
                for y in 0..art_size {
                    for x in 0..art_size {
                        if let Some(px) = thumb.get_pixel(x, y) {
                            fb.set_pixel(art_pad + x, art_pad + y, px);
                        }
                    }
                }
            }
        } else {
            // Stylized vinyl/album glyph.
            let cx = art_pad as f32 + art_size as f32 * 0.5;
            let cy = art_pad as f32 + art_size as f32 * 0.5;
            fill_circle(&mut fb, cx, cy, art_size as f32 * 0.48, self.style.track_bg_color);
            fill_circle(&mut fb, cx, cy, art_size as f32 * 0.22, self.style.accent_color);
            fill_circle(&mut fb, cx, cy, art_size as f32 * 0.08, self.style.bg_color);
        }

        let text_x = (art_pad + art_size + 14) as i32;
        draw_text(&mut fb, text_x, 14, &self.track.title, 2, self.style.text_color);

        let sub = if self.track.album.is_empty() {
            self.track.artist.clone()
        } else {
            format!("{} - {}", self.track.artist, self.track.album)
        };
        draw_text(&mut fb, text_x, 36, &sub, 1, self.style.subtext_color);

        let status_label = if self.track.muted {
            format!("{} [MUTED]", self.track.player_app)
        } else if self.track.is_playing {
            format!("{} PLAYING {}%", self.track.player_app, self.track.volume_pct)
        } else {
            format!("{} PAUSED", self.track.player_app)
        };
        draw_text(&mut fb, text_x, 52, &status_label, 1, self.style.accent_color);

        // Progress bar + timestamp.
        let bar_x = text_x;
        let bar_y = (h as i32) - 24;
        let bar_w = (w as i32 - bar_x - 16).max(20);
        let bar_h = 8i32;

        fill_rounded_rect(
            &mut fb,
            bar_x as f32,
            bar_y as f32,
            bar_w as f32,
            bar_h as f32,
            4.0,
            self.style.track_bg_color,
        );
        let fill_w = (bar_w as f32) * self.progress_ratio();
        if fill_w > 0.5 {
            fill_rounded_rect(
                &mut fb,
                bar_x as f32,
                bar_y as f32,
                fill_w,
                bar_h as f32,
                4.0,
                self.style.accent_color,
            );
        }

        let time_str = format!(
            "{} / {}",
            fmt_mm_ss(self.track.position_ms),
            fmt_mm_ss(self.track.duration_ms)
        );
        draw_text(&mut fb, bar_x, bar_y - 12, &time_str, 1, self.style.subtext_color);

        Ok(fb)
    }
}

fn fmt_mm_ss(ms: u64) -> String {
    let total_s = ms / 1000;
    format!("{:02}:{:02}", total_s / 60, total_s % 60)
}

// =========================================================================
// InputOverlayHud (`input-overlay`)
// =========================================================================

/// Live keyboard + mouse input snapshot for [`InputOverlayHud`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputStateSnapshot {
    pub w: bool,
    pub a: bool,
    pub s: bool,
    pub d: bool,
    pub space: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_key: bool,
    pub extra_keys: Vec<String>,
    pub mouse_left: bool,
    pub mouse_right: bool,
    pub mouse_middle: bool,
    pub mouse_dx: f32,
    pub mouse_dy: f32,
    pub scroll_delta: f32,
}

/// Visual styling for [`InputOverlayHud`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputOverlayStyle {
    pub width: u32,
    pub height: u32,
    pub bg_color: LinearRgba,
    pub key_idle_color: LinearRgba,
    pub key_active_color: LinearRgba,
    pub key_border_color: LinearRgba,
    pub text_color: LinearRgba,
    pub vector_color: LinearRgba,
    pub max_mouse_speed_px: f32,
}

impl Default for InputOverlayStyle {
    fn default() -> Self {
        Self {
            width: 360,
            height: 150,
            bg_color: [0.02, 0.02, 0.04, 0.78],
            key_idle_color: [0.10, 0.11, 0.15, 0.90],
            key_active_color: [0.10, 0.75, 0.98, 1.0],
            key_border_color: [0.35, 0.38, 0.45, 0.95],
            text_color: [0.98, 0.98, 1.0, 1.0],
            vector_color: [1.0, 0.32, 0.58, 1.0],
            max_mouse_speed_px: 50.0,
        }
    }
}

/// Renders live WASD/modifier keys + mouse buttons & movement vector (`input-overlay`).
#[derive(Clone, Debug, Default)]
pub struct InputOverlayHud {
    pub state: InputStateSnapshot,
    pub style: InputOverlayStyle,
    pub smoothed_vec: [f32; 2],
}

impl InputOverlayHud {
    pub fn new(style: InputOverlayStyle) -> Self {
        Self {
            state: InputStateSnapshot::default(),
            style,
            smoothed_vec: [0.0, 0.0],
        }
    }

    /// Updates current input state and applies exponential smoothing to the mouse velocity vector.
    pub fn update(&mut self, state: InputStateSnapshot, dt_ms: f32) {
        let alpha = clamp01(dt_ms / 40.0).max(0.25);
        self.smoothed_vec[0] = lerp(self.smoothed_vec[0], state.mouse_dx, alpha);
        self.smoothed_vec[1] = lerp(self.smoothed_vec[1], state.mouse_dy, alpha);
        self.state = state;
    }

    /// Renders the complete keyboard + mouse vector HUD to a [`FrameBuffer`].
    pub fn render(&self) -> Result<FrameBuffer, CompositorError> {
        let w = self.style.width.max(240);
        let h = self.style.height.max(120);
        let mut fb = FrameBuffer::filled(w, h, self.style.bg_color)?;

        let draw_key = |fb: &mut FrameBuffer, x: i32, y: i32, kw: i32, kh: i32, label: &str, pressed: bool| {
            let fill = if pressed {
                self.style.key_active_color
            } else {
                self.style.key_idle_color
            };
            fill_rounded_rect(fb, x as f32, y as f32, kw as f32, kh as f32, 4.0, fill);
            stroke_rect(fb, x, y, kw, kh, 1, self.style.key_border_color);
            let tx = x + (kw - (label.len() as i32 * 6)) / 2;
            let ty = y + (kh - 7) / 2;
            draw_text(fb, tx.max(x + 2), ty.max(y + 2), label, 1, self.style.text_color);
        };

        // WASD cluster
        let key_sz = 30;
        let gap = 4;
        let base_x = 16;
        let base_y = 14;
        draw_key(
            &mut fb,
            base_x + key_sz + gap,
            base_y,
            key_sz,
            key_sz,
            "W",
            self.state.w,
        );
        draw_key(
            &mut fb,
            base_x,
            base_y + key_sz + gap,
            key_sz,
            key_sz,
            "A",
            self.state.a,
        );
        draw_key(
            &mut fb,
            base_x + key_sz + gap,
            base_y + key_sz + gap,
            key_sz,
            key_sz,
            "S",
            self.state.s,
        );
        draw_key(
            &mut fb,
            base_x + (key_sz + gap) * 2,
            base_y + key_sz + gap,
            key_sz,
            key_sz,
            "D",
            self.state.d,
        );

        // Modifier + Space row
        let mod_y = base_y + (key_sz + gap) * 2 + 6;
        draw_key(&mut fb, base_x, mod_y, 38, 22, "CTRL", self.state.ctrl);
        draw_key(&mut fb, base_x + 42, mod_y, 44, 22, "SHIFT", self.state.shift);
        draw_key(&mut fb, base_x + 90, mod_y, 32, 22, "ALT", self.state.alt);
        draw_key(&mut fb, base_x + 126, mod_y, 64, 22, "SPACE", self.state.space);

        // Mouse buttons + movement vector reticle on the right side
        let mouse_x = (w as i32) - 135;
        let mouse_y = 14;
        draw_key(&mut fb, mouse_x, mouse_y, 34, 34, "LMB", self.state.mouse_left);
        draw_key(&mut fb, mouse_x + 38, mouse_y, 24, 34, "M", self.state.mouse_middle);
        draw_key(&mut fb, mouse_x + 66, mouse_y, 34, 34, "RMB", self.state.mouse_right);

        // Movement vector compass circle below mouse buttons
        let rcx = (mouse_x + 50) as f32;
        let rcy = (mouse_y + 76) as f32;
        let r_rad = 32.0_f32;
        fill_circle(&mut fb, rcx, rcy, r_rad, self.style.key_idle_color);
        draw_line(&mut fb, rcx - r_rad, rcy, rcx + r_rad, rcy, 1.0, self.style.key_border_color);
        draw_line(&mut fb, rcx, rcy - r_rad, rcx, rcy + r_rad, 1.0, self.style.key_border_color);

        let max_sp = self.style.max_mouse_speed_px.max(1.0);
        let vx = self.smoothed_vec[0];
        let vy = self.smoothed_vec[1];
        let mag = sqrtf(vx * vx + vy * vy);
        if mag > 0.25 {
            let norm_len = (mag / max_sp).min(1.0) * (r_rad - 3.0);
            let ux = vx / mag;
            let uy = vy / mag;
            let ex = rcx + ux * norm_len;
            let ey = rcy + uy * norm_len;
            draw_line(&mut fb, rcx, rcy, ex, ey, 2.5, self.style.vector_color);
            fill_circle(&mut fb, ex, ey, 3.5, self.style.vector_color);
            let _ = (cosf(0.0), sinf(0.0));
        } else {
            fill_circle(&mut fb, rcx, rcy, 3.0, self.style.text_color);
        }

        // Scroll indicator
        if absf(self.state.scroll_delta) > 0.01 {
            let tag = if self.state.scroll_delta > 0.0 { "SCR+" } else { "SCR-" };
            draw_text(&mut fb, mouse_x + 92, mouse_y + 70, tag, 1, self.style.vector_color);
        }

        Ok(fb)
    }
}
