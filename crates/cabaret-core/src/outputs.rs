//! Multi-streaming router, encoder groups, service validator, congestion control,
//! reconnect-preserving stream delay, Shibari secret redaction, Hybrid MP4 & 6-track recording,
//! circular replay buffer, QC shuttle player, and teleprompter.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Maximum number of independent stereo audio tracks supported in recordings/streams.
pub const NUM_AUDIO_TRACKS: usize = 6;

/// Unique identifier for a stream destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DestId(pub u64);

/// Errors emitted by output, recording, and replay buffer operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputError {
    EmptyReplayBuffer,
    NoKeyframeInReplayBuffer,
    InvalidTrackIndex(u8),
    NotRecording,
    InvalidRange,
}

/// Volatile Shibari secret slot ID (never holds raw stream keys in serialized config or logs).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShibariSecretSlot {
    pub slot_id: u64,
    pub label: String,
}

impl ShibariSecretSlot {
    pub fn new(slot_id: u64, label: impl Into<String>) -> Self {
        Self {
            slot_id,
            label: label.into(),
        }
    }
}

impl fmt::Display for ShibariSecretSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[SHIBARI_SLOT#{}:{}]", self.slot_id, self.label)
    }
}

/// Automatically redacts any active raw Shibari secret strings found in log output.
pub fn redact_log_line(message: &str, active_secrets: &[(ShibariSecretSlot, &str)]) -> String {
    let mut out = String::from(message);
    for (slot, secret) in active_secrets {
        if !secret.is_empty() && out.contains(secret) {
            let replacement = format!("[SHIBARI:REDACTED:slot#{}]", slot.slot_id);
            out = out.replace(secret, &replacement);
        }
    }
    out
}

/// Video codec selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VideoCodec {
    #[default]
    H264,
    Hevc,
    Av1,
    Vp9,
}

/// Audio codec selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AudioCodec {
    #[default]
    Aac,
    Opus,
    Pcm,
}

/// Rate control mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RateControl {
    #[default]
    Cbr,
    Vbr,
    Cqp,
    Abr,
}

/// Video encoder configuration key used for grouping identical stream outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VideoEncodeProfile {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub codec: VideoCodec,
    pub bitrate_kbps: u32,
    pub keyint_sec: u32,
    pub rate_control: RateControl,
}

impl Default for VideoEncodeProfile {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps_num: 60,
            fps_den: 1,
            codec: VideoCodec::H264,
            bitrate_kbps: 6000,
            keyint_sec: 2,
            rate_control: RateControl::Cbr,
        }
    }
}

/// Audio encoder configuration key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AudioEncodeProfile {
    pub codec: AudioCodec,
    pub bitrate_kbps: u32,
    pub sample_rate_hz: u32,
    /// Selected track index `0..6`.
    pub track_index: u8,
}

impl Default for AudioEncodeProfile {
    fn default() -> Self {
        Self {
            codec: AudioCodec::Aac,
            bitrate_kbps: 160,
            sample_rate_hz: 48_000,
            track_index: 0,
        }
    }
}

/// All supported live stream ingest protocols.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamDest {
    Rtmp {
        url: String,
        secret_slot: ShibariSecretSlot,
    },
    EnhancedRtmp {
        url: String,
        secret_slot: ShibariSecretSlot,
        codec: VideoCodec,
    },
    Srt {
        host: String,
        port: u16,
        passphrase_slot: Option<ShibariSecretSlot>,
        latency_ms: u32,
        stream_id: String,
    },
    Whip {
        endpoint_url: String,
        bearer_slot: Option<ShibariSecretSlot>,
    },
    Whep {
        endpoint_url: String,
        bearer_slot: Option<ShibariSecretSlot>,
    },
    Rist {
        url: String,
        buffer_ms: u32,
        secret_slot: Option<ShibariSecretSlot>,
    },
    Hls {
        manifest_path: String,
        segment_duration_ms: u32,
    },
    NdiLink {
        stream_name: String,
        groups: String,
    },
    CustomMuxer {
        format_name: String,
        target: String,
        options: Vec<(String, String)>,
    },
}

/// Service provider presets for compliance validation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ServicePlatform {
    Twitch,
    YouTube,
    Kick,
    Restream,
    #[default]
    Custom,
}

/// Complete stream output target configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamTarget {
    pub id: DestId,
    pub name: String,
    pub platform: ServicePlatform,
    pub dest: StreamDest,
    pub video: VideoEncodeProfile,
    pub audio: AudioEncodeProfile,
    pub enabled: bool,
    pub delay_sec: u32,
}

/// Planned encoder group sharing one encode pass across matching targets or split across distinct profiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncoderGroup {
    pub group_id: usize,
    pub video: VideoEncodeProfile,
    pub audio: AudioEncodeProfile,
    pub dest_ids: Vec<DestId>,
    pub shared: bool,
}

/// Plans shared vs split hardware/software encoders across simultaneous multi-stream targets.
pub struct EncoderGroupPlanner;

impl EncoderGroupPlanner {
    pub fn plan(targets: &[StreamTarget]) -> Vec<EncoderGroup> {
        let mut buckets: BTreeMap<(VideoEncodeProfile, AudioEncodeProfile), Vec<DestId>> =
            BTreeMap::new();
        for t in targets {
            if !t.enabled {
                continue;
            }
            buckets.entry((t.video, t.audio)).or_default().push(t.id);
        }

        buckets
            .into_iter()
            .enumerate()
            .map(|(idx, ((video, audio), dest_ids))| EncoderGroup {
                group_id: idx,
                video,
                audio,
                shared: dest_ids.len() > 1,
                dest_ids,
            })
            .collect()
    }
}

/// Severity of a streaming service compliance check issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationSeverity {
    Warning,
    Error,
}

/// Issue reported by [`validate_service_limits`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceValidationIssue {
    pub dest_id: DestId,
    pub platform: ServicePlatform,
    pub severity: ValidationSeverity,
    pub field: String,
    pub message: String,
}

/// Validates encoder parameters against Twitch, YouTube, Kick, and Restream service limits.
pub fn validate_service_limits(target: &StreamTarget) -> Vec<ServiceValidationIssue> {
    let mut issues = Vec::new();
    let fps = target.video.fps_num as f32 / target.video.fps_den.max(1) as f32;

    let mut push = |severity: ValidationSeverity, field: &str, msg: &str| {
        issues.push(ServiceValidationIssue {
            dest_id: target.id,
            platform: target.platform,
            severity,
            field: String::from(field),
            message: String::from(msg),
        });
    };

    match target.platform {
        ServicePlatform::Twitch => {
            if target.video.bitrate_kbps > 8500 {
                push(
                    ValidationSeverity::Error,
                    "video.bitrate_kbps",
                    "Twitch rejects bitrates above 8500 kbps (recommended max 6000 kbps)",
                );
            } else if target.video.bitrate_kbps > 6000 {
                push(
                    ValidationSeverity::Warning,
                    "video.bitrate_kbps",
                    "Bitrate exceeds Twitch recommended 6000 kbps cap",
                );
            }
            if target.video.keyint_sec != 2 {
                push(
                    ValidationSeverity::Error,
                    "video.keyint_sec",
                    "Twitch requires a 2-second keyframe interval",
                );
            }
            if target.audio.bitrate_kbps > 320 || target.audio.codec != AudioCodec::Aac {
                push(
                    ValidationSeverity::Error,
                    "audio",
                    "Twitch requires AAC audio <= 320 kbps",
                );
            }
            if target.video.width > 1920 || target.video.height > 1080 || fps > 60.5 {
                push(
                    ValidationSeverity::Warning,
                    "video.resolution",
                    "Resolution/FPS exceeds standard 1080p60 Twitch ingest guidance",
                );
            }
        }
        ServicePlatform::YouTube => {
            if target.video.bitrate_kbps > 51_000 {
                push(
                    ValidationSeverity::Error,
                    "video.bitrate_kbps",
                    "Bitrate exceeds YouTube 51000 kbps maximum",
                );
            }
            if target.video.keyint_sec == 0 || target.video.keyint_sec > 4 {
                push(
                    ValidationSeverity::Error,
                    "video.keyint_sec",
                    "YouTube requires keyframe interval <= 4 seconds (2s recommended)",
                );
            }
            if target.audio.bitrate_kbps > 512 {
                push(
                    ValidationSeverity::Warning,
                    "audio.bitrate_kbps",
                    "YouTube audio bitrate recommended <= 512 kbps",
                );
            }
        }
        ServicePlatform::Kick => {
            if target.video.bitrate_kbps > 8000 {
                push(
                    ValidationSeverity::Error,
                    "video.bitrate_kbps",
                    "Kick caps live ingest bitrate at 8000 kbps",
                );
            }
            if target.video.keyint_sec != 2 {
                push(
                    ValidationSeverity::Error,
                    "video.keyint_sec",
                    "Kick requires a 2-second keyframe interval",
                );
            }
            if target.video.width > 1920 || target.video.height > 1080 || fps > 60.5 {
                push(
                    ValidationSeverity::Error,
                    "video.resolution",
                    "Kick caps stream resolution/rate at 1080p60",
                );
            }
        }
        ServicePlatform::Restream => {
            if target.video.bitrate_kbps > 15_000 {
                push(
                    ValidationSeverity::Error,
                    "video.bitrate_kbps",
                    "Restream recommends/caps ingest <= 15000 kbps",
                );
            }
            if target.video.keyint_sec != 2 {
                push(
                    ValidationSeverity::Warning,
                    "video.keyint_sec",
                    "Restream recommends 2-second keyframes for multi-CDN relay",
                );
            }
        }
        ServicePlatform::Custom => {}
    }

    if target.audio.track_index as usize >= NUM_AUDIO_TRACKS {
        push(
            ValidationSeverity::Error,
            "audio.track_index",
            "Audio track index must be in 0..6",
        );
    }

    issues
}

/// Congestion controller decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CongestionAction {
    Hold { bitrate_kbps: u32 },
    StepDown { new_bitrate_kbps: u32 },
    RecoverUp { new_bitrate_kbps: u32 },
}

/// Dynamic network congestion bitrate step-down and recovery controller.
#[derive(Clone, Debug, PartialEq)]
pub struct CongestionController {
    pub nominal_kbps: u32,
    pub min_kbps: u32,
    pub current_kbps: u32,
    healthy_streak: u32,
    congested_streak: u32,
}

impl CongestionController {
    pub fn new(nominal_kbps: u32, min_kbps: u32) -> Self {
        let min_b = min_kbps.max(300).min(nominal_kbps.max(300));
        let nom = nominal_kbps.max(min_b);
        Self {
            nominal_kbps: nom,
            min_kbps: min_b,
            current_kbps: nom,
            healthy_streak: 0,
            congested_streak: 0,
        }
    }

    pub fn observe(
        &mut self,
        dropped_frames: u32,
        total_frames: u32,
        rtt_ms: u32,
        queue_fill_ratio: f32,
    ) -> CongestionAction {
        let drop_ratio = if total_frames > 0 {
            dropped_frames as f32 / total_frames as f32
        } else {
            0.0
        };
        let congested = drop_ratio > 0.05 || queue_fill_ratio > 0.75 || rtt_ms > 400;
        let healthy = drop_ratio < 0.01 && queue_fill_ratio < 0.30 && rtt_ms < 200;

        if congested {
            self.healthy_streak = 0;
            self.congested_streak += 1;
            let stepped = ((self.current_kbps as f32 * 0.75) as u32).max(self.min_kbps);
            if stepped < self.current_kbps {
                self.current_kbps = stepped;
                return CongestionAction::StepDown {
                    new_bitrate_kbps: self.current_kbps,
                };
            }
        } else if healthy {
            self.congested_streak = 0;
            self.healthy_streak += 1;
            if self.healthy_streak >= 3 && self.current_kbps < self.nominal_kbps {
                self.healthy_streak = 0;
                let up = ((self.current_kbps as f32 * 1.15) as u32)
                    .max(self.current_kbps + 200)
                    .min(self.nominal_kbps);
                self.current_kbps = up;
                return CongestionAction::RecoverUp {
                    new_bitrate_kbps: self.current_kbps,
                };
            }
        } else {
            self.healthy_streak = 0;
        }

        CongestionAction::Hold {
            bitrate_kbps: self.current_kbps,
        }
    }
}

/// Media stream packet kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
}

/// Encoded AV packet used across stream delay, recording, and replay buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedPacket {
    pub pts_us: i64,
    pub dts_us: i64,
    pub is_keyframe: bool,
    pub kind: MediaKind,
    pub track_index: u8,
    pub payload: Vec<u8>,
}

/// Reconnect connection state for [`StreamDelayQueue`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Disconnected { since_us: i64 },
    Reconnecting,
}

/// Reconnect-preserving broadcast delay queue holding encoded packets without dropping
/// queued delayed data when upstream sockets momentarily disconnect and reconnect.
#[derive(Clone, Debug)]
pub struct StreamDelayQueue {
    pub delay_us: i64,
    pub state: ConnectionState,
    pub preserve_on_reconnect: bool,
    queue: VecDeque<EncodedPacket>,
}

impl StreamDelayQueue {
    pub fn new(delay_seconds: u32, preserve_on_reconnect: bool) -> Self {
        Self {
            delay_us: (delay_seconds as i64) * 1_000_000,
            state: ConnectionState::Connected,
            preserve_on_reconnect,
            queue: VecDeque::new(),
        }
    }

    pub fn push(&mut self, pkt: EncodedPacket) {
        self.queue.push_back(pkt);
    }

    pub fn on_disconnect(&mut self, now_us: i64) {
        self.state = ConnectionState::Disconnected { since_us: now_us };
        if !self.preserve_on_reconnect {
            self.queue.clear();
        }
    }

    pub fn on_reconnect(&mut self) {
        self.state = ConnectionState::Connected;
    }

    /// Drains packets whose `pts_us + delay_us <= now_us` when connected.
    pub fn pop_ready(&mut self, now_us: i64) -> Vec<EncodedPacket> {
        if self.state != ConnectionState::Connected {
            return Vec::new();
        }
        let mut ready = Vec::new();
        while let Some(front) = self.queue.front() {
            if front.pts_us + self.delay_us <= now_us {
                if let Some(pkt) = self.queue.pop_front() {
                    ready.push(pkt);
                }
            } else {
                break;
            }
        }
        ready
    }

    pub fn queued_len(&self) -> usize {
        self.queue.len()
    }
}

/// Top-level multi-streaming router coordinating encoder groups, service validation,
/// congestion controllers, and reconnect-preserving delay queues.
#[derive(Clone, Debug, Default)]
pub struct MultiStreamRouter {
    pub targets: Vec<StreamTarget>,
    pub delay_queues: BTreeMap<DestId, StreamDelayQueue>,
    pub congestion: BTreeMap<DestId, CongestionController>,
}

impl MultiStreamRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_target(&mut self, target: StreamTarget) {
        let id = target.id;
        let delay = target.delay_sec;
        let nom = target.video.bitrate_kbps;
        self.delay_queues
            .insert(id, StreamDelayQueue::new(delay, true));
        self.congestion
            .insert(id, CongestionController::new(nom, (nom / 4).max(500)));
        self.targets.push(target);
    }

    pub fn plan_encoder_groups(&self) -> Vec<EncoderGroup> {
        EncoderGroupPlanner::plan(&self.targets)
    }

    pub fn validate_all(&self) -> Vec<ServiceValidationIssue> {
        let mut issues = Vec::new();
        for t in &self.targets {
            issues.extend(validate_service_limits(t));
        }
        issues
    }
}

/// Container formats for recording and replay saving.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ContainerFormat {
    #[default]
    HybridMp4,
    FragmentedMp4,
    Mkv,
    Mov,
    MpegTs,
}

/// 6-bit track routing bitmask (`tracks 1..=6` mapped to bits `0..=5`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioTrackMask(pub u8);

impl Default for AudioTrackMask {
    fn default() -> Self {
        Self(0b00_0001)
    }
}

impl AudioTrackMask {
    pub const ALL_6: Self = Self(0b11_1111);

    pub const fn from_tracks(tracks_0_based: &[u8]) -> Self {
        let mut mask = 0u8;
        let mut i = 0;
        while i < tracks_0_based.len() {
            let t = tracks_0_based[i];
            if (t as usize) < NUM_AUDIO_TRACKS {
                mask |= 1 << t;
            }
            i += 1;
        }
        Self(mask)
    }

    pub const fn contains(self, track_index: u8) -> bool {
        (track_index as usize) < NUM_AUDIO_TRACKS && (self.0 & (1 << track_index)) != 0
    }

    pub fn active_tracks(self) -> Vec<u8> {
        (0..NUM_AUDIO_TRACKS as u8)
            .filter(|&t| self.contains(t))
            .collect()
    }
}

/// Auto-split rule aligned on incoming video keyframes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SplitRule {
    #[default]
    Never,
    TimeSeconds(u64),
    SizeBytes(u64),
    Manual,
}

/// Recording chapter marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChapterMarker {
    pub timestamp_us: i64,
    pub title: String,
}

/// Recording session lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingState {
    Stopped,
    Recording,
    Paused,
    StoppedLowDisk,
}

/// Event returned when feeding a packet into [`RecordingSession`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingStep {
    Written { segment_index: u32, adjusted_pts_us: i64 },
    IgnoredPausedOrTrack,
    SplitToSegment { new_segment_index: u32 },
    StoppedLowDiskSpace,
}

/// Recording session supporting Hybrid MP4, 6 audio tracks, Pause/Resume, chapters,
/// keyframe-aligned auto-split, and low-disk auto-stop.
#[derive(Clone, Debug)]
pub struct RecordingSession {
    pub format: ContainerFormat,
    pub track_mask: AudioTrackMask,
    pub split_rule: SplitRule,
    pub min_free_disk_bytes: u64,
    pub state: RecordingState,
    pub segment_index: u32,
    pub segment_bytes: u64,
    pub segment_start_pts_us: Option<i64>,
    pub last_active_pts_us: i64,
    pub pause_started_us: Option<i64>,
    pub total_paused_us: i64,
    pub manual_split_armed: bool,
    pub chapters: Vec<ChapterMarker>,
    pub hybrid_mp4_moov_finalized: bool,
    pub written_packets: u64,
}

impl RecordingSession {
    pub fn new(format: ContainerFormat, track_mask: AudioTrackMask) -> Self {
        Self {
            format,
            track_mask,
            split_rule: SplitRule::Never,
            min_free_disk_bytes: 512 * 1024 * 1024,
            state: RecordingState::Stopped,
            segment_index: 1,
            segment_bytes: 0,
            segment_start_pts_us: None,
            last_active_pts_us: 0,
            pause_started_us: None,
            total_paused_us: 0,
            manual_split_armed: false,
            chapters: Vec::new(),
            hybrid_mp4_moov_finalized: false,
            written_packets: 0,
        }
    }

    pub fn start(&mut self) {
        self.state = RecordingState::Recording;
        self.segment_index = 1;
        self.segment_bytes = 0;
        self.segment_start_pts_us = None;
        self.total_paused_us = 0;
        self.pause_started_us = None;
        self.hybrid_mp4_moov_finalized = false;
        self.written_packets = 0;
    }

    pub fn pause(&mut self, now_us: i64) {
        if self.state == RecordingState::Recording {
            self.state = RecordingState::Paused;
            self.pause_started_us = Some(now_us);
        }
    }

    pub fn resume(&mut self, now_us: i64) {
        if self.state == RecordingState::Paused {
            if let Some(ps) = self.pause_started_us.take() {
                self.total_paused_us += (now_us - ps).max(0);
            }
            self.state = RecordingState::Recording;
        }
    }

    pub fn stop(&mut self) {
        self.state = RecordingState::Stopped;
        if self.format == ContainerFormat::HybridMp4 {
            self.hybrid_mp4_moov_finalized = true;
        }
    }

    pub fn add_chapter(&mut self, title: impl Into<String>) {
        self.chapters.push(ChapterMarker {
            timestamp_us: self.last_active_pts_us,
            title: title.into(),
        });
    }

    pub fn request_manual_split(&mut self) {
        self.manual_split_armed = true;
    }

    pub fn push_packet(
        &mut self,
        pkt: &EncodedPacket,
        available_disk_bytes: u64,
    ) -> RecordingStep {
        if self.state != RecordingState::Recording {
            return RecordingStep::IgnoredPausedOrTrack;
        }
        if available_disk_bytes < self.min_free_disk_bytes {
            self.state = RecordingState::StoppedLowDisk;
            if self.format == ContainerFormat::HybridMp4 {
                self.hybrid_mp4_moov_finalized = true;
            }
            return RecordingStep::StoppedLowDiskSpace;
        }
        if pkt.kind == MediaKind::Audio && !self.track_mask.contains(pkt.track_index) {
            return RecordingStep::IgnoredPausedOrTrack;
        }

        let adj_pts = (pkt.pts_us - self.total_paused_us).max(0);
        let seg_start = *self.segment_start_pts_us.get_or_insert(adj_pts);

        let should_split = pkt.kind == MediaKind::Video
            && pkt.is_keyframe
            && match self.split_rule {
                SplitRule::Never => self.manual_split_armed,
                SplitRule::Manual => self.manual_split_armed,
                SplitRule::TimeSeconds(sec) => {
                    self.manual_split_armed || (adj_pts - seg_start) >= (sec as i64) * 1_000_000
                }
                SplitRule::SizeBytes(bytes) => {
                    self.manual_split_armed || self.segment_bytes >= bytes
                }
            };

        if should_split && self.written_packets > 0 {
            self.segment_index += 1;
            self.segment_bytes = pkt.payload.len() as u64;
            self.segment_start_pts_us = Some(adj_pts);
            self.manual_split_armed = false;
            self.last_active_pts_us = adj_pts;
            self.written_packets += 1;
            return RecordingStep::SplitToSegment {
                new_segment_index: self.segment_index,
            };
        }

        self.segment_bytes += pkt.payload.len() as u64;
        self.last_active_pts_us = adj_pts;
        self.written_packets += 1;
        RecordingStep::Written {
            segment_index: self.segment_index,
            adjusted_pts_us: adj_pts,
        }
    }
}

/// Circular Replay Buffer storing bounded encoded AV packets for instant keyframe-aligned saving.
#[derive(Clone, Debug)]
pub struct ReplayBuffer {
    pub max_duration_us: i64,
    pub max_bytes: usize,
    total_bytes: usize,
    ring: VecDeque<EncodedPacket>,
}

impl ReplayBuffer {
    pub fn new(max_seconds: u32, max_bytes: usize) -> Self {
        Self {
            max_duration_us: (max_seconds.max(1) as i64) * 1_000_000,
            max_bytes: max_bytes.max(1024),
            total_bytes: 0,
            ring: VecDeque::new(),
        }
    }

    pub fn push_packet(&mut self, pkt: EncodedPacket) {
        let newest_pts = pkt.pts_us;
        self.total_bytes += pkt.payload.len();
        self.ring.push_back(pkt);

        while self.total_bytes > self.max_bytes
            || self
                .ring
                .front()
                .is_some_and(|f| newest_pts - f.pts_us > self.max_duration_us)
        {
            if let Some(old) = self.ring.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(old.payload.len());
            } else {
                break;
            }
        }
    }

    /// Saves the last `seconds` ending at the newest buffered packet, starting on a valid video keyframe.
    pub fn save_last_seconds(&self, seconds: f32) -> Result<Vec<EncodedPacket>, OutputError> {
        let Some(latest) = self.ring.back() else {
            return Err(OutputError::EmptyReplayBuffer);
        };
        let target_start = latest.pts_us - ((seconds.max(0.1) * 1_000_000.0) as i64);

        // Prefer the last video keyframe at or before target_start so the saved clip covers
        // at least `seconds`; otherwise fall back to the earliest video keyframe in the ring.
        let mut best_kf_idx: Option<usize> = None;
        for (i, p) in self.ring.iter().enumerate() {
            if p.kind == MediaKind::Video && p.is_keyframe {
                if p.pts_us <= target_start {
                    best_kf_idx = Some(i);
                } else if best_kf_idx.is_none() {
                    best_kf_idx = Some(i);
                    break;
                }
            }
        }

        let start_idx = best_kf_idx.ok_or(OutputError::NoKeyframeInReplayBuffer)?;
        let kf_pts = self.ring[start_idx].pts_us;

        // Include video from keyframe onward and audio synchronized >= kf_pts.
        Ok(self
            .ring
            .iter()
            .skip(start_idx)
            .filter(|p| p.pts_us >= kf_pts)
            .cloned()
            .collect())
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    pub fn buffered_bytes(&self) -> usize {
        self.total_bytes
    }
}

/// Export range handed off from QC Player to **Splicy** (`crates/sex-splice`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplicyClipSpec {
    pub source_path: String,
    pub mark_in_us: i64,
    pub mark_out_us: i64,
}

/// Instant QC review player state supporting J/K/L shuttle (`-2x..+4x`), frame stepping,
/// and Splicy in/out clip marking.
#[derive(Clone, Debug, PartialEq)]
pub struct QcPlayerState {
    pub source_path: String,
    pub duration_us: i64,
    pub fps_num: u32,
    pub fps_den: u32,
    pub position_us: i64,
    pub shuttle_speed: f32,
    pub playing: bool,
    pub mark_in_us: Option<i64>,
    pub mark_out_us: Option<i64>,
}

impl QcPlayerState {
    pub fn new(
        source_path: impl Into<String>,
        duration_us: i64,
        fps_num: u32,
        fps_den: u32,
    ) -> Self {
        Self {
            source_path: source_path.into(),
            duration_us: duration_us.max(0),
            fps_num: fps_num.max(1),
            fps_den: fps_den.max(1),
            position_us: 0,
            shuttle_speed: 0.0,
            playing: false,
            mark_in_us: None,
            mark_out_us: None,
        }
    }

    pub fn set_shuttle_speed(&mut self, speed: f32) {
        self.shuttle_speed = speed.clamp(-2.0, 4.0);
        self.playing = self.shuttle_speed != 0.0;
    }

    /// J key: step reverse shuttle (`0 -> -1x -> -2x`).
    pub fn shuttle_j(&mut self) {
        let next = if self.shuttle_speed > 0.0 {
            -1.0
        } else if self.shuttle_speed == 0.0 {
            -1.0
        } else {
            -2.0
        };
        self.set_shuttle_speed(next);
    }

    /// K key: pause shuttle (`0x`).
    pub fn shuttle_k(&mut self) {
        self.set_shuttle_speed(0.0);
    }

    /// L key: step forward shuttle (`<=0 -> 1x -> 2x -> 4x`).
    pub fn shuttle_l(&mut self) {
        let next = if self.shuttle_speed <= 0.0 {
            1.0
        } else if self.shuttle_speed < 1.5 {
            2.0
        } else {
            4.0
        };
        self.set_shuttle_speed(next);
    }

    /// Advances playback position by `dt_us * shuttle_speed`.
    pub fn advance(&mut self, dt_us: i64) {
        if !self.playing || self.shuttle_speed == 0.0 {
            return;
        }
        let delta = (dt_us as f64 * self.shuttle_speed as f64) as i64;
        self.position_us = (self.position_us + delta).clamp(0, self.duration_us);
    }

    /// Steps exact frames forward (`>0`) or backward (`<0`) and pauses shuttle.
    pub fn frame_step(&mut self, delta_frames: i32) {
        self.shuttle_k();
        let frame_us = ((1_000_000_i64 * self.fps_den as i64) / self.fps_num as i64).max(1);
        self.position_us =
            (self.position_us + frame_us * delta_frames as i64).clamp(0, self.duration_us);
    }

    pub fn mark_in(&mut self) {
        self.mark_in_us = Some(self.position_us);
    }

    pub fn mark_out(&mut self) {
        self.mark_out_us = Some(self.position_us);
    }

    pub fn export_splicy_clip(&self) -> Result<SplicyClipSpec, OutputError> {
        let mi = self.mark_in_us.unwrap_or(0);
        let mo = self.mark_out_us.unwrap_or(self.duration_us);
        if mo <= mi {
            return Err(OutputError::InvalidRange);
        }
        Ok(SplicyClipSpec {
            source_path: self.source_path.clone(),
            mark_in_us: mi,
            mark_out_us: mo,
        })
    }
}

/// Teleprompter overlay state with WPM auto-scroll, mirror flip, capture-hiding,
/// and live voice-word-index alignment tracking.
#[derive(Clone, Debug, PartialEq)]
pub struct TeleprompterState {
    pub script_text: String,
    pub words: Vec<String>,
    pub scroll_wpm: f32,
    pub scroll_offset_px: f32,
    pub line_height_px: f32,
    pub words_per_line: f32,
    pub mirror_h: bool,
    pub mirror_v: bool,
    pub hide_from_capture: bool,
    pub voice_word_index: usize,
}

impl Default for TeleprompterState {
    fn default() -> Self {
        Self {
            script_text: String::new(),
            words: Vec::new(),
            scroll_wpm: 140.0,
            scroll_offset_px: 0.0,
            line_height_px: 48.0,
            words_per_line: 8.0,
            mirror_h: false,
            mirror_v: false,
            hide_from_capture: true,
            voice_word_index: 0,
        }
    }
}

impl TeleprompterState {
    pub fn new(script_text: impl Into<String>) -> Self {
        let mut st = Self::default();
        st.set_script(script_text);
        st
    }

    pub fn set_script(&mut self, text: impl Into<String>) {
        self.script_text = text.into();
        self.words = self
            .script_text
            .split_whitespace()
            .map(normalize_word)
            .filter(|w| !w.is_empty())
            .collect();
        self.voice_word_index = 0;
        self.scroll_offset_px = 0.0;
    }

    /// Advances smooth auto-scroll according to `scroll_wpm`.
    pub fn advance_time(&mut self, dt_seconds: f32) {
        if dt_seconds <= 0.0 || self.scroll_wpm <= 0.0 {
            return;
        }
        let words_per_sec = self.scroll_wpm / 60.0;
        let px_per_word = self.line_height_px / self.words_per_line.max(1.0);
        self.scroll_offset_px += words_per_sec * px_per_word * dt_seconds;
    }

    /// Advances `voice_word_index` by matching recognized spoken words in a forward lookahead window
    /// and syncs `scroll_offset_px` cleanly to the active line.
    pub fn update_voice_match(&mut self, spoken_words: &[&str]) -> usize {
        if self.words.is_empty() || spoken_words.is_empty() {
            return self.voice_word_index;
        }
        let lookahead = 24;
        for &raw in spoken_words {
            let norm = normalize_word(raw);
            if norm.is_empty() {
                continue;
            }
            let end = (self.voice_word_index + lookahead).min(self.words.len());
            if let Some(rel) = self.words[self.voice_word_index..end]
                .iter()
                .position(|w| *w == norm)
            {
                self.voice_word_index = self.voice_word_index + rel + 1;
            }
        }
        let px_per_word = self.line_height_px / self.words_per_line.max(1.0);
        self.scroll_offset_px = self.voice_word_index as f32 * px_per_word;
        self.voice_word_index
    }
}

fn normalize_word(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}
