// SPDX-License-Identifier: GPL-3.0-only
//! Bottom-strip peak-hold stereo VU meter for Splicy.
//!
//! Inspired by Rook's audio level monitoring: tracks per-track and master
//! audio levels, smoothly decays peak-hold meters, detects clipping (>= 248),
//! and paints a segmented meter strip above the status line.

use crate::media::Library;
use crate::view::Rect;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_office::{Canvas, Pen};
use sex_splice::{Micros, Timeline, TrackKind};

pub const VU_STRIP_H: i32 = 28;

pub const VU_BG: u32 = 0x00252529;
pub const VU_LINE: u32 = 0x003a3a42;
pub const VU_TROUGH: u32 = 0x00161619;
pub const VU_GREEN: u32 = 0x002ecc71;
pub const VU_AMBER: u32 = 0x00f39c12;
pub const VU_RED: u32 = 0x00e74c3c;
pub const VU_PEAK: u32 = 0x00ffffff;
pub const VU_TEXT_DIM: u32 = 0x008a8a96;
pub const VU_TEXT_INK: u32 = 0x00e8e8ec;

/// Meter readings for a single track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackMeter {
    pub name: String,
    pub level: u8,
    pub peak_hold: u8,
    pub clipping: bool,
}

/// Horizontal bottom-strip VU meter state.
#[derive(Clone, Debug, Default)]
pub struct VuStrip {
    pub visible: bool,
    pub tracks: Vec<TrackMeter>,
    pub master_l: u8,
    pub master_r: u8,
    pub master_peak: u8,
}

impl VuStrip {
    /// Update track and master meters from timeline audio clips and waveforms.
    pub fn update_from_library(
        &mut self,
        tl: &Timeline,
        lib: &Library,
        playhead: Micros,
        window_us: Micros,
    ) {
        let audio_tracks: Vec<&sex_splice::Track> =
            tl.tracks.iter().filter(|t| t.kind == TrackKind::Audio).collect();

        while self.tracks.len() < audio_tracks.len() {
            let idx = self.tracks.len();
            let name = if audio_tracks[idx].name.is_empty() {
                format!("A{}", idx + 1)
            } else {
                audio_tracks[idx].name.clone()
            };
            self.tracks.push(TrackMeter {
                name,
                level: 0,
                peak_hold: 0,
                clipping: false,
            });
        }
        if self.tracks.len() > audio_tracks.len() {
            self.tracks.truncate(audio_tracks.len());
        }

        for (i, track) in audio_tracks.iter().enumerate() {
            self.tracks[i].name = if track.name.is_empty() {
                format!("A{}", i + 1)
            } else {
                track.name.clone()
            };

            let mut track_level: u8 = 0;
            if !track.muted {
                for clip in &track.clips {
                    if !clip.enabled {
                        continue;
                    }
                    if clip.at <= playhead && playhead < clip.end() {
                        let s0 = clip.source_at(playhead);
                        let s1 = clip.source_at((playhead + window_us.max(1)).min(clip.end()));
                        if let Some(peaks) = lib.peaks.get(&clip.media) {
                            let raw = peaks.over(s0, s1.max(s0 + 1));
                            let scaled = ((raw as u32 * clip.volume as u32) / 1000).min(255) as u8;
                            track_level = track_level.max(scaled);
                        }
                    }
                }
            }

            let prev_hold = self.tracks[i].peak_hold;
            let new_hold = if track_level >= prev_hold {
                track_level
            } else {
                prev_hold.saturating_sub(6)
            };

            self.tracks[i].level = track_level;
            self.tracks[i].peak_hold = new_hold;
            self.tracks[i].clipping = track_level >= 248;
        }

        if self.tracks.is_empty() {
            self.master_l = 0;
            self.master_r = 0;
            self.master_peak = self.master_peak.saturating_sub(6);
        } else if self.tracks.len() == 1 {
            self.master_l = self.tracks[0].level;
            self.master_r = self.tracks[0].level;
        } else {
            self.master_l = self.tracks.iter().step_by(2).map(|t| t.level).max().unwrap_or(0);
            self.master_r = self.tracks.iter().skip(1).step_by(2).map(|t| t.level).max().unwrap_or(0);
        }

        let cur_master = self.master_l.max(self.master_r);
        if cur_master >= self.master_peak {
            self.master_peak = cur_master;
        } else {
            self.master_peak = self.master_peak.saturating_sub(6);
        }
    }

    /// Paint horizontal VU meter strip into canvas rectangle `r`.
    pub fn paint(&self, c: &mut Canvas, r: Rect, small: &mut Pen) {
        if r.w <= 0 || r.h <= 0 {
            return;
        }

        // Background and top border
        c.fill(r.x, r.y, r.w, r.h, VU_BG);
        c.fill(r.x, r.y, r.w, 1, VU_LINE);

        let master_w = 140.min(r.w / 3).max(40);
        let avail_w = (r.w - master_w - 16).max(0);

        // Draw track meters
        if !self.tracks.is_empty() && avail_w > 20 {
            let slot_w = (avail_w / self.tracks.len() as i32).clamp(40, 200);
            let mut cur_x = r.x + 8;
            for tm in &self.tracks {
                if cur_x + slot_w > r.x + avail_w {
                    break;
                }
                let label = if tm.clipping {
                    format!("{}:CLIP", tm.name)
                } else {
                    tm.name.clone()
                };
                let lw = small.measure(&label).min(slot_w / 2);
                let label_color = if tm.clipping { VU_RED } else { VU_TEXT_INK };
                let ty = r.y + (r.h - small.line_height()) / 2;
                small.text(c, cur_x, ty, &label, label_color, Some((cur_x, lw + 4)));

                let bar_x = cur_x + lw + 6;
                let bar_w = (cur_x + slot_w - bar_x - 6).max(10);
                let bar_h = 8;
                let bar_y = r.y + (r.h - bar_h) / 2;

                draw_meter_bar(c, bar_x, bar_y, bar_w, bar_h, tm.level, tm.peak_hold, tm.clipping);
                cur_x += slot_w;
            }
        } else if self.tracks.is_empty() {
            let msg = "No audio";
            let ty = r.y + (r.h - small.line_height()) / 2;
            small.text(c, r.x + 10, ty, msg, VU_TEXT_DIM, None);
        }

        // Master meter on the right
        let mx = r.x + r.w - master_w;
        c.fill(mx - 4, r.y + 4, 1, r.h - 8, VU_LINE);

        let m_clipping = self.master_l >= 248 || self.master_r >= 248 || self.master_peak >= 248;
        let m_label = if m_clipping { "M:CLIP" } else { "MASTER" };
        let mlw = small.measure(m_label).min(master_w / 3);
        let m_color = if m_clipping { VU_RED } else { VU_TEXT_INK };
        let ty = r.y + (r.h - small.line_height()) / 2;
        small.text(c, mx + 4, ty, m_label, m_color, Some((mx + 4, mlw + 4)));

        let mbar_x = mx + mlw + 8;
        let mbar_w = (mx + master_w - mbar_x - 8).max(10);
        let mbar_h = 4;
        let mbar_y1 = r.y + 7;
        let mbar_y2 = r.y + 16;

        draw_meter_bar(
            c,
            mbar_x,
            mbar_y1,
            mbar_w,
            mbar_h,
            self.master_l,
            self.master_peak,
            self.master_l >= 248,
        );
        draw_meter_bar(
            c,
            mbar_x,
            mbar_y2,
            mbar_w,
            mbar_h,
            self.master_r,
            self.master_peak,
            self.master_r >= 248,
        );
    }
}

fn draw_meter_bar(
    c: &mut Canvas,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    level: u8,
    peak_hold: u8,
    clipping: bool,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    // Background trough
    c.fill(x, y, w, h, VU_TROUGH);

    let active_w = (level as i32 * w / 255).min(w);
    for bx in 0..active_w {
        let val = (bx * 255 / w) as u8;
        let col = if val >= 240 {
            VU_RED
        } else if val >= 192 {
            VU_AMBER
        } else {
            VU_GREEN
        };
        c.fill(x + bx, y, 1, h, col);
    }

    // Peak hold tick
    if peak_hold > 0 {
        let px = (peak_hold as i32 * w / 255).clamp(0, w - 1);
        c.fill(x + px, y, 1, h, VU_PEAK);
    }

    // Clipping end pip
    if clipping {
        c.fill(x + w - 2, y, 2, h, VU_RED);
    }
}
