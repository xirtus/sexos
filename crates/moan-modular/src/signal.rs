// SPDX-License-Identifier: GPL-3.0-only
//! Polyphonic signal, port, and cable model for EuropaRack.

use crate::math::{clamp, soft_clip};

pub const BLOCK_FRAMES: usize = 64;
pub const MAX_POLY: usize = 16;
pub const RATE: u32 = 48_000;
pub const MAX_MIDI_EVENTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PortKind {
    /// Stereo audio: `[f32; 2]` normalized `-1.0..=1.0` or Eurorack `±5.0V`.
    Audio = 0,
    /// 1V/oct pitch CV across 1..=16 channels (`0.0V = C4 = 261.6256 Hz`).
    PitchCv = 1,
    /// Modulation CV `±10.0V` across 1..=16 channels.
    ModCv = 2,
    /// Gate/Trigger `0.0V` or `10.0V` across 1..=16 channels.
    Gate = 3,
    /// Bounded MIDI note events (up to 32 events per block).
    Midi = 4,
}

impl PortKind {
    pub const fn default_color(self) -> CableColor {
        match self {
            Self::Audio => CableColor::CrimsonAudio,
            Self::PitchCv => CableColor::GoldPitchCv,
            Self::ModCv => CableColor::CyanModCv,
            Self::Gate => CableColor::EmeraldGate,
            Self::Midi => CableColor::VioletMidi,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CableColor {
    CrimsonAudio = 0,
    GoldPitchCv = 1,
    CyanModCv = 2,
    EmeraldGate = 3,
    VioletMidi = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RackTheme {
    /// VCV Dark Room brightness mode: `0..=100` (0 = dark night glow, 100 = day).
    pub dark_room_brightness: u8,
}

impl Default for RackTheme {
    fn default() -> Self {
        Self {
            dark_room_brightness: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModularMidiEvent {
    pub frame_offset: u8,
    pub channel: u8,
    pub key: u8,
    pub velocity: u8,
    pub is_note_on: bool,
}

impl ModularMidiEvent {
    pub const fn note_on(frame_offset: u8, channel: u8, key: u8, velocity: u8) -> Self {
        Self {
            frame_offset,
            channel,
            key,
            velocity,
            is_note_on: true,
        }
    }

    pub const fn note_off(frame_offset: u8, channel: u8, key: u8) -> Self {
        Self {
            frame_offset,
            channel,
            key,
            velocity: 0,
            is_note_on: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MidiBuffer {
    pub events: [ModularMidiEvent; MAX_MIDI_EVENTS],
    pub len: u8,
}

impl Default for MidiBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl MidiBuffer {
    pub const fn new() -> Self {
        Self {
            events: [ModularMidiEvent {
                frame_offset: 0,
                channel: 0,
                key: 0,
                velocity: 0,
                is_note_on: false,
            }; MAX_MIDI_EVENTS],
            len: 0,
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    #[inline]
    pub fn push(&mut self, event: ModularMidiEvent) -> bool {
        if (self.len as usize) < MAX_MIDI_EVENTS {
            self.events[self.len as usize] = event;
            self.len += 1;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn events(&self) -> &[ModularMidiEvent] {
        &self.events[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SignalBuffer {
    pub kind: PortKind,
    pub audio: [[f32; 2]; BLOCK_FRAMES],
    pub poly_channels: u8,
    pub poly: [[f32; BLOCK_FRAMES]; MAX_POLY],
    pub midi: MidiBuffer,
}

impl SignalBuffer {
    pub const fn new(kind: PortKind) -> Self {
        Self {
            kind,
            audio: [[0.0; 2]; BLOCK_FRAMES],
            poly_channels: 1,
            poly: [[0.0; BLOCK_FRAMES]; MAX_POLY],
            midi: MidiBuffer::new(),
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        for frame in &mut self.audio {
            *frame = [0.0, 0.0];
        }
        for ch in &mut self.poly {
            for sample in ch {
                *sample = 0.0;
            }
        }
        self.poly_channels = 1;
        self.midi.clear();
    }

    #[inline]
    pub fn get_audio(&self, frame: usize) -> [f32; 2] {
        if frame < BLOCK_FRAMES {
            self.audio[frame]
        } else {
            [0.0, 0.0]
        }
    }

    #[inline]
    pub fn set_audio(&mut self, frame: usize, sample: [f32; 2]) {
        if frame < BLOCK_FRAMES {
            self.audio[frame] = sample;
        }
    }

    #[inline]
    pub fn set_audio_mono(&mut self, frame: usize, sample: f32) {
        if frame < BLOCK_FRAMES {
            self.audio[frame] = [sample, sample];
        }
    }

    /// Read CV for a given channel. If channel exceeds active poly channels,
    /// broadcast channel 0 (Eurorack monophonic normalled broadcast).
    #[inline]
    pub fn get_cv(&self, ch: usize, frame: usize) -> f32 {
        if frame >= BLOCK_FRAMES {
            return 0.0;
        }
        let actual_ch = if self.poly_channels > 1 && ch < (self.poly_channels as usize) {
            ch
        } else {
            0
        };
        self.poly[actual_ch][frame]
    }

    #[inline]
    pub fn set_cv(&mut self, ch: usize, frame: usize, value: f32) {
        if ch < MAX_POLY && frame < BLOCK_FRAMES {
            self.poly[ch][frame] = value;
            if (ch as u8 + 1) > self.poly_channels {
                self.poly_channels = ch as u8 + 1;
            }
        }
    }

    /// Mix incoming signal from source into self with given attenuation factor.
    #[inline]
    pub fn mix_from(&mut self, src: &SignalBuffer, attenuation: f32) {
        match self.kind {
            PortKind::Audio => match src.kind {
                PortKind::Audio => {
                    for f in 0..BLOCK_FRAMES {
                        self.audio[f][0] += src.audio[f][0] * attenuation;
                        self.audio[f][1] += src.audio[f][1] * attenuation;
                    }
                }
                PortKind::PitchCv | PortKind::ModCv | PortKind::Gate => {
                    // CV to Audio conversion: 5V Eurorack maps to 1.0 full-scale
                    for f in 0..BLOCK_FRAMES {
                        let val = src.get_cv(0, f) * (1.0 / 5.0) * attenuation;
                        self.audio[f][0] += val;
                        self.audio[f][1] += val;
                    }
                }
                PortKind::Midi => {}
            },
            PortKind::PitchCv | PortKind::ModCv | PortKind::Gate => match src.kind {
                PortKind::PitchCv | PortKind::ModCv | PortKind::Gate => {
                    let max_ch = (self.poly_channels as usize).max(src.poly_channels as usize);
                    self.poly_channels = clamp(max_ch as f32, 1.0, MAX_POLY as f32) as u8;
                    for ch in 0..max_ch {
                        for f in 0..BLOCK_FRAMES {
                            let val = src.get_cv(ch, f) * attenuation;
                            self.poly[ch][f] += val;
                        }
                    }
                }
                PortKind::Audio => {
                    // Audio to CV: 1.0 full-scale maps to 5V Eurorack
                    for f in 0..BLOCK_FRAMES {
                        let mono = (src.audio[f][0] + src.audio[f][1]) * 0.5 * 5.0 * attenuation;
                        self.poly[0][f] += mono;
                    }
                }
                PortKind::Midi => {}
            },
            PortKind::Midi => {
                if src.kind == PortKind::Midi {
                    for event in src.midi.events() {
                        self.midi.push(*event);
                    }
                }
            }
        }
    }

    /// Soft limit buffers to prevent NaN and explosive feedback runaway.
    #[inline]
    pub fn soft_limit(&mut self) {
        for frame in &mut self.audio {
            frame[0] = soft_clip(frame[0]);
            frame[1] = soft_clip(frame[1]);
        }
        let ch_count = (self.poly_channels as usize).min(MAX_POLY);
        for ch in 0..ch_count {
            for f in 0..BLOCK_FRAMES {
                let v = self.poly[ch][f];
                if v.is_nan() || v.is_infinite() {
                    self.poly[ch][f] = 0.0;
                } else {
                    self.poly[ch][f] = clamp(v, -12.0, 12.0);
                }
            }
        }
    }
}
