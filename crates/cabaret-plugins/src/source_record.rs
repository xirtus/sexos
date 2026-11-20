//! Pure-Rust port of **source-record** and **freeze-filter** for Cabaret.
//!
//! [`SourceRecordTap`] captures an individual [`SourceId`]'s clean pre-composite video & audio
//! feed into an isolated [`RecordingSession`] and [`ReplayBuffer`], while [`FreezeFilter`]
//! provides instant hold, timed freeze, and rhythmic strobe frame holds.

use alloc::vec::Vec;

use cabaret_core::compositor::FrameBuffer;
use cabaret_core::outputs::{
    AudioTrackMask, ContainerFormat, EncodedPacket, MediaKind, OutputError, RecordingSession,
    RecordingStep, ReplayBuffer,
};
use cabaret_core::scene::SourceId;

/// Clean pre-composite single-source recorder and instant replay tap (`source-record`).
#[derive(Clone, Debug)]
pub struct SourceRecordTap {
    pub source_id: SourceId,
    pub recording: RecordingSession,
    pub replay_buffer: ReplayBuffer,
    pub replay_enabled: bool,
    pub video_frames_captured: u64,
    pub audio_blocks_captured: u64,
    pub last_clean_frame: Option<FrameBuffer>,
}

impl SourceRecordTap {
    pub fn new(
        source_id: SourceId,
        format: ContainerFormat,
        track_mask: AudioTrackMask,
        replay_seconds: u32,
        replay_max_bytes: usize,
    ) -> Self {
        Self {
            source_id,
            recording: RecordingSession::new(format, track_mask),
            replay_buffer: ReplayBuffer::new(replay_seconds, replay_max_bytes),
            replay_enabled: true,
            video_frames_captured: 0,
            audio_blocks_captured: 0,
            last_clean_frame: None,
        }
    }

    pub fn start(&mut self) {
        self.recording.start();
    }

    pub fn pause(&mut self, now_us: i64) {
        self.recording.pause(now_us);
    }

    pub fn resume(&mut self, now_us: i64) {
        self.recording.resume(now_us);
    }

    pub fn stop(&mut self) {
        self.recording.stop();
    }

    /// Taps a clean pre-composite [`FrameBuffer`] for `source_id`, pushing it into both
    /// `replay_buffer` (when enabled) and `recording`.
    pub fn tap_video_frame(
        &mut self,
        pts_us: i64,
        clean_frame: &FrameBuffer,
        is_keyframe: bool,
        available_disk_bytes: u64,
    ) -> RecordingStep {
        self.last_clean_frame = Some(clean_frame.clone());
        self.video_frames_captured += 1;

        let rgba = clean_frame.to_srgb_rgba8();
        let mut payload = Vec::with_capacity(12 + rgba.len());
        payload.extend_from_slice(b"CSRV");
        payload.extend_from_slice(&clean_frame.width.to_le_bytes());
        payload.extend_from_slice(&clean_frame.height.to_le_bytes());
        payload.extend_from_slice(&rgba);

        let pkt = EncodedPacket {
            pts_us,
            dts_us: pts_us,
            is_keyframe,
            kind: MediaKind::Video,
            track_index: 0,
            payload,
        };
        if self.replay_enabled {
            self.replay_buffer.push_packet(pkt.clone());
        }
        self.recording.push_packet(&pkt, available_disk_bytes)
    }

    /// Taps a pre-composite stereo audio block for `source_id`.
    pub fn tap_audio_block(
        &mut self,
        pts_us: i64,
        track_index: u8,
        stereo: &[[f32; 2]],
        available_disk_bytes: u64,
    ) -> RecordingStep {
        self.audio_blocks_captured += 1;
        let mut payload = Vec::with_capacity(stereo.len() * 8);
        for &[l, r] in stereo {
            payload.extend_from_slice(&l.to_le_bytes());
            payload.extend_from_slice(&r.to_le_bytes());
        }
        let pkt = EncodedPacket {
            pts_us,
            dts_us: pts_us,
            is_keyframe: true,
            kind: MediaKind::Audio,
            track_index,
            payload,
        };
        if self.replay_enabled {
            self.replay_buffer.push_packet(pkt.clone());
        }
        self.recording.push_packet(&pkt, available_disk_bytes)
    }

    /// Saves the last `seconds` from this source's isolated replay buffer starting on a video keyframe.
    pub fn save_replay(&self, seconds: f32) -> Result<Vec<EncodedPacket>, OutputError> {
        self.replay_buffer.save_last_seconds(seconds)
    }
}

/// Decodes a tapped video packet emitted by [`SourceRecordTap::tap_video_frame`] back into a [`FrameBuffer`].
pub fn decode_tapped_video_packet(pkt: &EncodedPacket) -> Option<FrameBuffer> {
    if pkt.kind != MediaKind::Video || pkt.payload.len() < 12 || &pkt.payload[..4] != b"CSRV" {
        return None;
    }
    let w = u32::from_le_bytes(pkt.payload[4..8].try_into().ok()?);
    let h = u32::from_le_bytes(pkt.payload[8..12].try_into().ok()?);
    FrameBuffer::from_srgb_rgba8(w, h, &pkt.payload[12..]).ok()
}

/// Operating mode for [`FreezeFilter`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum FreezeMode {
    #[default]
    Live,
    InstantHold,
    TimedFreeze {
        duration_ms: f32,
    },
    Strobe {
        hold_ms: f32,
        live_ms: f32,
    },
}

/// Frame-hold / strobe filter (`freeze-filter`).
#[derive(Clone, Debug, Default)]
pub struct FreezeFilter {
    pub mode: FreezeMode,
    held_frame: Option<FrameBuffer>,
    elapsed_ms: f32,
}

impl FreezeFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trigger_instant(&mut self) {
        self.mode = FreezeMode::InstantHold;
        self.elapsed_ms = 0.0;
        self.held_frame = None;
    }

    pub fn trigger_timed(&mut self, duration_ms: f32) {
        self.mode = FreezeMode::TimedFreeze {
            duration_ms: duration_ms.max(0.0),
        };
        self.elapsed_ms = 0.0;
        self.held_frame = None;
    }

    pub fn set_strobe(&mut self, hold_ms: f32, live_ms: f32) {
        self.mode = FreezeMode::Strobe {
            hold_ms: hold_ms.max(1.0),
            live_ms: live_ms.max(1.0),
        };
        self.elapsed_ms = 0.0;
        self.held_frame = None;
    }

    pub fn unfreeze(&mut self) {
        self.mode = FreezeMode::Live;
        self.elapsed_ms = 0.0;
        self.held_frame = None;
    }

    pub fn is_frozen(&self) -> bool {
        match self.mode {
            FreezeMode::Live => false,
            FreezeMode::InstantHold => self.held_frame.is_some(),
            FreezeMode::TimedFreeze { duration_ms } => self.elapsed_ms < duration_ms,
            FreezeMode::Strobe { hold_ms, live_ms } => {
                let period = (hold_ms + live_ms).max(2.0);
                let phase = self.elapsed_ms % period;
                phase < hold_ms
            }
        }
    }

    /// Processes `live_frame` advancing internal freeze/strobe timers by `dt_ms`.
    pub fn process(&mut self, live_frame: &FrameBuffer, dt_ms: f32) -> FrameBuffer {
        let dt = dt_ms.max(0.0);
        match self.mode {
            FreezeMode::Live => {
                self.held_frame = None;
                live_frame.clone()
            }
            FreezeMode::InstantHold => {
                if let Some(ref held) = self.held_frame {
                    held.clone()
                } else {
                    self.held_frame = Some(live_frame.clone());
                    live_frame.clone()
                }
            }
            FreezeMode::TimedFreeze { duration_ms } => {
                if self.held_frame.is_none() {
                    self.held_frame = Some(live_frame.clone());
                    self.elapsed_ms = 0.0;
                    return live_frame.clone();
                }
                self.elapsed_ms += dt;
                if self.elapsed_ms >= duration_ms {
                    self.unfreeze();
                    live_frame.clone()
                } else {
                    self.held_frame.clone().unwrap_or_else(|| live_frame.clone())
                }
            }
            FreezeMode::Strobe { hold_ms, live_ms } => {
                let h_ms = hold_ms.max(1.0);
                let l_ms = live_ms.max(1.0);
                let period = h_ms + l_ms;
                let phase = self.elapsed_ms % period;
                self.elapsed_ms += dt;
                if phase < h_ms {
                    if let Some(ref held) = self.held_frame {
                        held.clone()
                    } else {
                        self.held_frame = Some(live_frame.clone());
                        live_frame.clone()
                    }
                } else {
                    self.held_frame = None;
                    live_frame.clone()
                }
            }
        }
    }
}
