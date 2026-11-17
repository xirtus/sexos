//! Pure-Rust port of **obs-localvocal** / Closed Captions for Cabaret.
//!
//! Provides a live rolling caption buffer (`push_transcript_segment`), an odd-parity
//! **CEA-608** line-21 byte-pair encoder, a **CEA-708** DTVCC packetizer (`cc_data` triplets),
//! `.srt` / `.vtt` timestamped subtitle sidecar exporters, and a styled lower-third burn-in
//! rasterizer on [`cabaret_core::compositor::FrameBuffer`].

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use cabaret_core::compositor::{FrameBuffer, LinearRgba};

use crate::util::{draw_text, fill_rect, fill_rounded_rect, measure_text_width};

/// Single recognized or manual transcript segment pushed into [`CaptionBuffer`].
#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptSegment {
    pub id: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub speaker: Option<String>,
    pub confidence: f32,
    pub is_final: bool,
}

impl TranscriptSegment {
    pub fn new(id: u64, start_ms: u64, end_ms: u64, text: impl Into<String>) -> Self {
        Self {
            id,
            start_ms,
            end_ms: end_ms.max(start_ms),
            text: text.into(),
            speaker: None,
            confidence: 1.0,
            is_final: true,
        }
    }

    pub fn with_speaker(mut self, speaker: impl Into<String>) -> Self {
        self.speaker = Some(speaker.into());
        self
    }

    pub fn with_final(mut self, is_final: bool) -> Self {
        self.is_final = is_final;
        self
    }
}

/// Live rolling closed-caption buffer with word wrapping, interim replacement, and SRT/VTT export.
#[derive(Clone, Debug)]
pub struct CaptionBuffer {
    pub max_lines: usize,
    pub max_chars_per_line: usize,
    pub max_history: usize,
    rolling_lines: Vec<String>,
    segments: Vec<TranscriptSegment>,
    interim: Option<TranscriptSegment>,
}

impl Default for CaptionBuffer {
    fn default() -> Self {
        Self::new(2, 32, 512)
    }
}

impl CaptionBuffer {
    pub fn new(max_lines: usize, max_chars_per_line: usize, max_history: usize) -> Self {
        Self {
            max_lines: max_lines.max(1),
            max_chars_per_line: max_chars_per_line.max(8),
            max_history: max_history.max(1),
            rolling_lines: Vec::new(),
            segments: Vec::new(),
            interim: None,
        }
    }

    /// Pushes a live transcript segment, updating the rolling display lines and sidecar history.
    ///
    /// Non-final segments replace the pending interim text without polluting finalized history;
    /// final segments commit to history (updating an existing matching segment `id` if present).
    pub fn push_transcript_segment(&mut self, seg: TranscriptSegment) {
        if seg.is_final {
            self.interim = None;
            if let Some(existing) = self.segments.iter_mut().find(|s| s.id == seg.id) {
                *existing = seg;
            } else {
                self.segments.push(seg);
                while self.segments.len() > self.max_history {
                    self.segments.remove(0);
                }
            }
            self.rebuild_rolling_lines();
        } else {
            self.interim = Some(seg);
            self.rebuild_rolling_lines();
        }
    }

    fn rebuild_rolling_lines(&mut self) {
        let mut combined = Vec::new();
        for seg in &self.segments {
            let formatted = format_segment_text(seg);
            for line in wrap_text(&formatted, self.max_chars_per_line) {
                combined.push(line);
            }
        }
        if let Some(ref inter) = self.interim {
            let formatted = format_segment_text(inter);
            for line in wrap_text(&formatted, self.max_chars_per_line) {
                combined.push(line);
            }
        }
        let start = combined.len().saturating_sub(self.max_lines);
        self.rolling_lines = combined.split_off(start);
    }

    /// Returns the current live rolling caption lines.
    pub fn current_lines(&self) -> &[String] {
        &self.rolling_lines
    }

    /// Returns finalized transcript segments active at `now_ms`.
    pub fn active_segments_at(&self, now_ms: u64) -> Vec<&TranscriptSegment> {
        self.segments
            .iter()
            .filter(|s| now_ms >= s.start_ms && now_ms <= s.end_ms)
            .collect()
    }

    /// Returns all finalized segments stored in history.
    pub fn segments(&self) -> &[TranscriptSegment] {
        &self.segments
    }

    /// Exports all finalized transcript segments as a standard SubRip (`.srt`) sidecar string.
    pub fn export_srt(&self) -> String {
        let mut out = String::new();
        for (idx, seg) in self.segments.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            let text = format_segment_text(seg);
            let wrapped = wrap_text(&text, self.max_chars_per_line).join("\n");
            out.push_str(&format!(
                "{}\n{} --> {}\n{}\n",
                idx + 1,
                format_ts_srt(seg.start_ms),
                format_ts_srt(seg.end_ms),
                wrapped
            ));
        }
        out
    }

    /// Exports all finalized transcript segments as a standard WebVTT (`.vtt`) sidecar string.
    pub fn export_vtt(&self) -> String {
        let mut out = String::from("WEBVTT\n\n");
        for (idx, seg) in self.segments.iter().enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            let text = format_segment_text(seg);
            let wrapped = wrap_text(&text, self.max_chars_per_line).join("\n");
            out.push_str(&format!(
                "{} --> {}\n{}\n",
                format_ts_vtt(seg.start_ms),
                format_ts_vtt(seg.end_ms),
                wrapped
            ));
        }
        out
    }
}

fn format_segment_text(seg: &TranscriptSegment) -> String {
    match &seg.speaker {
        Some(sp) if !sp.is_empty() => format!("{sp}: {}", seg.text.trim()),
        _ => String::from(seg.text.trim()),
    }
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if current.is_empty() {
            if word_len <= max_chars {
                current.push_str(word);
            } else {
                // Split overlong token cleanly.
                let mut chunk = String::new();
                for ch in word.chars() {
                    if chunk.chars().count() >= max_chars {
                        lines.push(chunk);
                        chunk = String::new();
                    }
                    chunk.push(ch);
                }
                current = chunk;
            }
        } else if current.chars().count() + 1 + word_len <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = String::from(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn format_ts_srt(ms: u64) -> String {
    let total_s = ms / 1000;
    let millis = ms % 1000;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{h:02}:{m:02}:{s:02},{millis:03}")
}

fn format_ts_vtt(ms: u64) -> String {
    let total_s = ms / 1000;
    let millis = ms % 1000;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    format!("{h:02}:{m:02}:{s:02}.{millis:03}")
}

// =========================================================================
// CEA-608 Line-21 Byte-Pair Encoder
// =========================================================================

/// Sets bit 7 of a 7-bit byte so total 1-bits is odd (CEA-608 standard parity).
#[inline]
pub const fn odd_parity_byte(b7: u8) -> u8 {
    let v = b7 & 0x7F;
    if v.count_ones() % 2 == 0 {
        v | 0x80
    } else {
        v
    }
}

/// Returns `true` if `byte` satisfies CEA-608 odd parity.
#[inline]
pub const fn has_odd_parity(byte: u8) -> bool {
    byte.count_ones() % 2 == 1
}

/// Single CEA-608 Line-21 2-byte frame payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cea608Pair(pub [u8; 2]);

impl Cea608Pair {
    pub const fn control(c1: u8, c2: u8) -> Self {
        Self([odd_parity_byte(c1), odd_parity_byte(c2)])
    }

    pub const fn chars(c1: u8, c2: u8) -> Self {
        Self([odd_parity_byte(c1), odd_parity_byte(c2)])
    }

    pub const fn all_odd_parity(self) -> bool {
        has_odd_parity(self.0[0]) && has_odd_parity(self.0[1])
    }
}

/// CEA-608 caption presentation mode (Channel 1, Field 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Cea608Mode {
    #[default]
    PopOn,
    RollUp2,
    RollUp3,
    RollUp4,
}

/// Encodes `text` into odd-parity CEA-608 Line-21 byte pairs (CC1 Field 1).
pub fn encode_cea608_pairs(text: &str, mode: Cea608Mode) -> Vec<Cea608Pair> {
    let mut pairs = Vec::new();
    match mode {
        Cea608Mode::PopOn => {
            pairs.push(Cea608Pair::control(0x14, 0x20)); // RCL: Resume Caption Loading
            pairs.push(Cea608Pair::control(0x14, 0x2E)); // ENM: Erase Non-Displayed Memory
            pairs.push(Cea608Pair::control(0x14, 0x70)); // PAC: Row 15, indent 0
        }
        Cea608Mode::RollUp2 => {
            pairs.push(Cea608Pair::control(0x14, 0x25)); // RU2
            pairs.push(Cea608Pair::control(0x14, 0x70)); // PAC Row 15
        }
        Cea608Mode::RollUp3 => {
            pairs.push(Cea608Pair::control(0x14, 0x26)); // RU3
            pairs.push(Cea608Pair::control(0x14, 0x70));
        }
        Cea608Mode::RollUp4 => {
            pairs.push(Cea608Pair::control(0x14, 0x27)); // RU4
            pairs.push(Cea608Pair::control(0x14, 0x70));
        }
    }

    let ascii: Vec<u8> = text
        .chars()
        .map(|ch| {
            let u = ch as u32;
            if (0x20..=0x7E).contains(&u) {
                u as u8
            } else {
                b'?'
            }
        })
        .collect();

    let mut i = 0;
    while i < ascii.len() {
        let a = ascii[i];
        let b = ascii.get(i + 1).copied().unwrap_or(0x00);
        pairs.push(Cea608Pair::chars(a, b));
        i += 2;
    }

    match mode {
        Cea608Mode::PopOn => {
            pairs.push(Cea608Pair::control(0x14, 0x2F)); // EOC: End of Caption
        }
        Cea608Mode::RollUp2 | Cea608Mode::RollUp3 | Cea608Mode::RollUp4 => {
            pairs.push(Cea608Pair::control(0x14, 0x2D)); // CR: Carriage Return
        }
    }
    pairs
}

/// Decodes printable text from CEA-608 odd-parity byte pairs (ignoring non-printable control codes).
pub fn decode_cea608_pairs(pairs: &[Cea608Pair]) -> Option<String> {
    let mut out = String::new();
    for pair in pairs {
        if !pair.all_odd_parity() {
            return None;
        }
        let d0 = pair.0[0] & 0x7F;
        let d1 = pair.0[1] & 0x7F;
        if d0 < 0x20 {
            continue;
        }
        out.push(d0 as char);
        if d1 >= 0x20 {
            out.push(d1 as char);
        }
    }
    Some(out)
}

// =========================================================================
// CEA-708 DTVCC Packetizer
// =========================================================================

/// Encoded CTA-708 DTVCC caption packet + ATSC A/53 `cc_data` 3-byte triplets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cea708Packet {
    pub sequence_num: u8,
    pub service_num: u8,
    pub packet_bytes: Vec<u8>,
    /// ATSC A/53 `cc_data` triplets: `[0xFF, b0, b1]` for packet start, `[0xFE, b0, b1]` for data.
    pub cc_triplets: Vec<[u8; 3]>,
}

/// Packetizes `text` into a CTA-708 DTVCC caption channel packet (`service_num` 1..=6)
/// and ATSC A/53 `cc_data` triplets.
pub fn packetize_cea708(text: &str, service_num: u8, sequence_num: u8) -> Cea708Packet {
    let svc = service_num.clamp(1, 6);
    let seq = sequence_num & 0x03;

    // Build Service Block payload: SetCurrentWindow0 (0x80), ClearWindows (0x88, 0x01), text, DisplayWindows (0x89, 0x01).
    let mut svc_data = Vec::new();
    svc_data.push(0x80); // SetCurrentWindow 0
    svc_data.push(0x88); // ClearWindows
    svc_data.push(0x01); // Window 0 bitmap
    let text_bytes: Vec<u8> = text
        .bytes()
        .filter(|&b| (0x20..=0x7E).contains(&b) || b == b'\n')
        .take(96)
        .collect();
    svc_data.extend_from_slice(&text_bytes);
    svc_data.push(0x89); // DisplayWindows
    svc_data.push(0x01);

    let block_size = svc_data.len().min(31) as u8;
    let svc_header = (svc << 5) | block_size;

    // DTVCC Packet size must be even (2..=128 bytes) including 1-byte packet header.
    let raw_len = 1 + 1 + block_size as usize;
    let padded_len = if raw_len % 2 == 1 { raw_len + 1 } else { raw_len }.clamp(2, 126);
    let size_code = (padded_len / 2) as u8;
    let pkt_header = (seq << 6) | (size_code & 0x3F);

    let mut packet_bytes = Vec::with_capacity(padded_len);
    packet_bytes.push(pkt_header);
    packet_bytes.push(svc_header);
    packet_bytes.extend_from_slice(&svc_data[..block_size as usize]);
    while packet_bytes.len() < padded_len {
        packet_bytes.push(0x00);
    }

    let mut cc_triplets = Vec::with_capacity(packet_bytes.len() / 2);
    for (idx, pair) in packet_bytes.chunks_exact(2).enumerate() {
        // ATSC A/53: bit2=1 (valid), bits1..0 = 0b11 (DTVCC Packet Start) or 0b10 (DTVCC Packet Data).
        let header_byte = if idx == 0 { 0xFF } else { 0xFE };
        cc_triplets.push([header_byte, pair[0], pair[1]]);
    }

    Cea708Packet {
        sequence_num: seq,
        service_num: svc,
        packet_bytes,
        cc_triplets,
    }
}

/// Extracts printable caption text back from a sequence of CEA-708 DTVCC `cc_data` triplets.
pub fn depacketize_cea708(cc_triplets: &[[u8; 3]]) -> Option<String> {
    let mut pkt = Vec::new();
    for &[hdr, b0, b1] in cc_triplets {
        let cc_valid = (hdr & 0x04) != 0;
        let cc_type = hdr & 0x03;
        if cc_valid && (cc_type == 0b10 || cc_type == 0b11) {
            pkt.push(b0);
            pkt.push(b1);
        }
    }
    if pkt.len() < 2 {
        return None;
    }
    let svc_hdr = pkt[1];
    let block_size = (svc_hdr & 0x1F) as usize;
    if pkt.len() < 2 + block_size {
        return None;
    }
    let block = &pkt[2..2 + block_size];
    let mut text = String::new();
    let mut i = 0;
    while i < block.len() {
        let b = block[i];
        match b {
            0x80 => i += 1,          // SetCurrentWindow
            0x88 | 0x89 => i += 2,   // ClearWindows / DisplayWindows + 1-byte mask
            0x20..=0x7E | b'\n' => {
                text.push(b as char);
                i += 1;
            }
            _ => i += 1,
        }
    }
    Some(text)
}

// =========================================================================
// Styled Lower-Third Burn-In Rasterizer
// =========================================================================

/// Visual styling for burned-in closed-caption lower thirds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptionBurnInStyle {
    pub bg_color: LinearRgba,
    pub text_color: LinearRgba,
    pub accent_color: LinearRgba,
    pub font_scale: u32,
    pub padding_x: u32,
    pub padding_y: u32,
    pub margin_bottom_px: u32,
    pub corner_radius_px: f32,
    pub line_spacing_px: u32,
}

impl Default for CaptionBurnInStyle {
    fn default() -> Self {
        Self {
            bg_color: [0.02, 0.02, 0.04, 0.84],
            text_color: [0.98, 0.98, 1.0, 1.0],
            accent_color: [0.95, 0.25, 0.55, 1.0],
            font_scale: 2,
            padding_x: 16,
            padding_y: 10,
            margin_bottom_px: 24,
            corner_radius_px: 6.0,
            line_spacing_px: 6,
        }
    }
}

/// Rasterizes styled lower-third closed captions directly onto `frame`.
pub fn render_lower_third_captions(
    frame: &mut FrameBuffer,
    lines: &[&str],
    style: &CaptionBurnInStyle,
) {
    if lines.is_empty() {
        return;
    }
    let scale = style.font_scale.max(1);
    let char_h = 7 * scale;
    let max_text_w = lines
        .iter()
        .map(|l| measure_text_width(l, scale))
        .max()
        .unwrap_or(0);
    if max_text_w == 0 {
        return;
    }

    let accent_w = 4 * scale;
    let box_w = (max_text_w + style.padding_x * 2 + accent_w + 6).min(frame.width);
    let total_text_h =
        lines.len() as u32 * char_h + (lines.len().saturating_sub(1)) as u32 * style.line_spacing_px;
    let box_h = (total_text_h + style.padding_y * 2).min(frame.height);

    let bx = (frame.width.saturating_sub(box_w)) / 2;
    let by = frame
        .height
        .saturating_sub(box_h + style.margin_bottom_px);

    fill_rounded_rect(
        frame,
        bx as f32,
        by as f32,
        box_w as f32,
        box_h as f32,
        style.corner_radius_px,
        style.bg_color,
    );

    // Left accent bar inside the lower-third pill.
    fill_rect(
        frame,
        (bx + 4) as i32,
        (by + 4) as i32,
        accent_w as i32,
        box_h.saturating_sub(8) as i32,
        style.accent_color,
    );

    let text_x = (bx + 4 + accent_w + style.padding_x / 2 + 4) as i32;
    for (idx, line) in lines.iter().enumerate() {
        let ty = by + style.padding_y + idx as u32 * (char_h + style.line_spacing_px);
        draw_text(frame, text_x, ty as i32, line, scale, style.text_color);
    }
}
