// SPDX-License-Identifier: GPL-3.0-only
//! Core Cabaret broadcast studio state integrating `cabaret-core`, all 12 plugins
//! from `cabaret-plugins`, `sex-color`, `burlesque-core`, and `sex-splice`.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use burlesque_core::LivePresentationSource;
use cabaret_core::audio::{
    AudioFilter, AudioFilterEntry, AudioMeters, AudioMixer, AudioStrip, ChannelLayout,
    MixedAudioOutput, MoanPlugFormat, MoanPlugSlot, MonitorMode, StripId,
};
use cabaret_core::compositor::{
    BlendMode, BoundsMode, FrameBuffer, MixColorSpace, Scaler, Transform2D,
};
use cabaret_core::filters::{ColorWheel, FilterContext, FilterEntry, VideoFilter};
use cabaret_core::outputs::{
    redact_log_line, AudioEncodeProfile, AudioTrackMask, ContainerFormat, DestId, EncodedPacket,
    EncoderGroup, EncoderGroupPlanner, MediaKind, MultiStreamRouter, OutputError, QcPlayerState,
    RecordingSession, RecordingState, RecordingStep, ReplayBuffer, ServicePlatform,
    ServiceValidationIssue, ShibariSecretSlot, SplitRule, SplicyClipSpec, StreamDest,
    StreamTarget, TeleprompterState, VideoCodec, VideoEncodeProfile,
};
use cabaret_core::scene::{
    ItemId, PhoneProtocol, Profile, SceneCollection, SourceId, StudioId, StudioLayoutMode,
    UndoStack, UvcPixelFormat,
};
use cabaret_core::transitions::{
    MultiviewLayout, MultiviewPreset, StingerConfig, StudioMode, TransitionKind, TransitionSpec,
    TransitionTick, WipeDirection, WipePattern, ALL_WIPE_PATTERNS,
};
use cabaret_plugins::adv_switcher::{
    execute_macro_actions, Condition, MacroAction, MacroEngine, MacroOutputCommand, MacroRule,
    MacroTriggerEvent, MacroWorldState, StreamActiveState,
};
use cabaret_plugins::downstream_keyer::{DownstreamKeyerBank, DskChannel, DskTransition};
use cabaret_plugins::local_captions::{
    encode_cea608_pairs, packetize_cea708, CaptionBuffer, Cea608Mode, Cea608Pair, Cea708Packet,
    TranscriptSegment,
};
use cabaret_plugins::move_transition::{EasingCurve, MoveTransitionConfig};
use cabaret_plugins::multi_canvas::{
    CanvasId, CanvasPreset, SecondaryCanvasItem, SecondaryCanvasManager,
};
use cabaret_plugins::overlays::{
    InputOverlayHud, InputStateSnapshot, NowPlayingCard, NowPlayingTrack,
};
use cabaret_plugins::shader_filter::ShaderPreset;
use cabaret_plugins::source_record::{FreezeFilter, SourceRecordTap};
use cabaret_plugins::spectralizer::{
    compute_stereo_correlation, SpectrumAnalyzer, SpectrumConfig,
};
use cabaret_plugins::streamfx_3d::{CornerPinQuad, PitchYawRollFov};
use cabaret_plugins::studio_link::{
    LinkPacket, LinkPayload, LinkPixelFormat, LinkTally, LinkVideoFrame, ServiceAnnouncement,
};
use cabaret_plugins::time_warp::{DynamicDelayBuffer, TimeWarpConfig, TimeWarpScan};
use cabaret_plugins::wasm_fx::WasmFxBridge;
use cabaret_proto::browser_bridge::{generate_init_js, ControlLevel};
use cabaret_proto::streamdeck::StreamDeckModel;
use sex_color::{
    ColorSpace, FilmStock, ScopeStandard, ToneMapConfig, ToneMapMode, WaveformMode,
    BT2408_SDR_WHITE_NITS, REF_WHITE_NITS,
};
use sex_splice::handoff::{build_handoff_bundle, HandoffBundle};
use sex_splice::{otio, MediaKind as SpliceMediaKind, Op, Range, Timeline, TrackKind, SECOND};

/// Built-in 4 seeded broadcast studios.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeededStudios {
    pub main_camera_slides: StudioId,
    pub screen_pip: StudioId,
    pub coding_terminal: StudioId,
    pub brb_starting_soon: StudioId,
}

/// Built-in seeded reusable source IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeededSources {
    pub silk_window: SourceId,
    pub uvc_camera: SourceId,
    pub presentation_deck: SourceId,
    pub browser_alerts: SourceId,
    pub phone_srt: SourceId,
    pub now_playing_card: SourceId,
    pub input_overlay: SourceId,
    pub backdrop_solid: SourceId,
}

/// Built-in seeded audio mixer strip IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeededStrips {
    pub mic_serenade: StripId,
    pub desktop_bell: StripId,
    pub browser_alerts: StripId,
    pub media_deck: StripId,
}

/// Transition selector exposed cleanly to UI and control plane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActiveTransitionPreset {
    Cut,
    #[default]
    Fade,
    Wipe29,
    Stinger,
    Move,
}

impl ActiveTransitionPreset {
    pub const ALL: [Self; 5] = [
        Self::Cut,
        Self::Fade,
        Self::Wipe29,
        Self::Stinger,
        Self::Move,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Cut => "Cut",
            Self::Fade => "Fade",
            Self::Wipe29 => "Wipe29",
            Self::Stinger => "Stinger",
            Self::Move => "Move",
        }
    }
}

/// Active color & broadcast scope display configuration (`sex-color`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScopeKind {
    Waveform,
    RgbParade,
    Vectorscope,
    Histogram,
}

impl ScopeKind {
    pub const ALL: [Self; 4] = [
        Self::Waveform,
        Self::RgbParade,
        Self::Vectorscope,
        Self::Histogram,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Waveform => "Waveform",
            Self::RgbParade => "RGB Parade",
            Self::Vectorscope => "Vectorscope",
            Self::Histogram => "Histogram",
        }
    }
}

/// Canvas color pipeline powered by `sex-color`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasColorPipeline {
    pub canvas_space: ColorSpace,
    pub tone_map: ToneMapConfig,
    pub film_stock: Option<FilmStock>,
    pub sdr_white_nits: f32,
    pub scope_kind: ScopeKind,
    pub scope_standard: ScopeStandard,
    pub waveform_mode: WaveformMode,
}

impl Default for CanvasColorPipeline {
    fn default() -> Self {
        Self {
            canvas_space: ColorSpace::Rec709,
            tone_map: ToneMapConfig {
                mode: ToneMapMode::Bt2390,
                source_peak_nits: 1_000.0,
                target_peak_nits: REF_WHITE_NITS,
            },
            film_stock: None,
            sdr_white_nits: BT2408_SDR_WHITE_NITS,
            scope_kind: ScopeKind::Waveform,
            scope_standard: ScopeStandard::Auto,
            waveform_mode: WaveformMode::RgbOverlay,
        }
    }
}

impl CanvasColorPipeline {
    /// Selectable broadcast canvas color spaces.
    pub const SPACES: [ColorSpace; 4] = [
        ColorSpace::Rec709,
        ColorSpace::LinearRec2020,
        ColorSpace::Hdr10Pq,
        ColorSpace::Hlg,
    ];

    pub const fn space_label(space: ColorSpace) -> &'static str {
        match space {
            ColorSpace::Rec709 => "Rec.709 (SDR)",
            ColorSpace::LinearRec2020 => "Linear Rec.2020 (Wide)",
            ColorSpace::Hdr10Pq => "HDR10 PQ (BT.2100)",
            ColorSpace::Hlg => "HLG (BT.2100)",
            ColorSpace::Srgb => "sRGB",
            ColorSpace::Rec601 => "Rec.601",
            ColorSpace::Rec2020 => "Rec.2020",
            ColorSpace::LinearRec709 => "Linear Rec.709",
            ColorSpace::Oklab => "Oklab",
        }
    }
}

/// Self-exclusion / SexOS window capture policy status (mirrors `sex_pdx::WindowCapturePolicy`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureExclusionStatus {
    pub cabaret_ui_hidden: bool,
    pub teleprompter_hidden: bool,
    pub silkbar_tally_active: bool,
    pub capture_pixel_format: &'static str,
}

/// Virtual camera status synchronized with `sex_uvc::virtual_cam::VIRTUAL_CAM_NAME`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtualCamState {
    pub active: bool,
    pub device_name: String,
    pub source_label: String,
    pub sequence: u64,
    pub epoch: u64,
}

/// Packaged handoff result for 1-click "Open Last Recording in Splicy".
#[derive(Clone, Debug)]
pub struct SplicyHandoff {
    pub clip: SplicyClipSpec,
    pub timeline: Timeline,
    pub otio_json: String,
    pub bundle: HandoffBundle,
}

/// Complete Cabaret Studio state combining core mixing, compositing, outputs,
/// and all 12 OBS plugins.
pub struct CabaretStudio {
    pub profile: Profile,
    pub collection: SceneCollection,
    pub undo: UndoStack,
    pub studios: SeededStudios,
    pub sources: SeededSources,
    pub strips: SeededStrips,
    pub studio_mode: StudioMode,
    pub multiview_preset: MultiviewPreset,
    pub multiview_show_safe_areas: bool,

    // Transition presets & stinger config
    pub transition_preset: ActiveTransitionPreset,
    pub transition_duration_ms: u32,
    pub wipe_pattern_index: usize,
    pub stinger: StingerConfig,
    pub move_config: MoveTransitionConfig,

    // Audio mixer & visualizer
    pub audio: AudioMixer,
    pub spectrum: SpectrumAnalyzer,
    pub stereo_correlation: f32,
    pub last_mixed_audio: Option<MixedAudioOutput>,
    pub last_goniometer_frame: Option<FrameBuffer>,

    // Embedded live presentation deck (Burlesque) & confined Browser bridge (Kaleidoscope)
    pub presentation: LivePresentationSource,
    pub browser_init_js: String,
    pub browser_events_log: Vec<String>,

    // All 12 Native Plugins + Overlays
    pub dsk: DownstreamKeyerBank,
    pub secondary_canvases: SecondaryCanvasManager,
    pub vertical_canvas_id: CanvasId,
    pub macros: MacroEngine,
    pub active_macro_events: Vec<MacroTriggerEvent>,
    pub focused_silk_window: String,
    pub active_shader_index: Option<usize>,
    pub warp_3d_enabled: bool,
    pub corner_pin: CornerPinQuad,
    pub perspective_3d: PitchYawRollFov,
    pub wasm_fx: WasmFxBridge,
    pub source_tap: SourceRecordTap,
    pub freeze_filter: FreezeFilter,
    pub time_warp: TimeWarpScan,
    pub time_warp_enabled: bool,
    pub delay_buffer: DynamicDelayBuffer,
    pub captions: CaptionBuffer,
    pub burn_in_captions: bool,
    pub last_cea608: Vec<Cea608Pair>,
    pub last_cea708: Option<Cea708Packet>,
    pub link_enabled: bool,
    pub link_announcement: ServiceAnnouncement,
    pub link_last_packet_bytes: usize,
    pub now_playing: NowPlayingCard,
    pub input_overlay: InputOverlayHud,

    // Wide-gamut color + broadcast scopes
    pub color_pipeline: CanvasColorPipeline,

    // Multi-streaming, Recording, Replay Buffer, QC Player, Teleprompter
    pub router: MultiStreamRouter,
    pub active_secrets: Vec<(ShibariSecretSlot, String)>,
    pub stream_active: bool,
    pub stream_reconnecting: bool,
    pub stream_duration_ms: u64,
    pub stream_bytes_sent: u64,
    pub stream_frames_sent: u64,
    pub stream_frames_dropped: u64,

    pub recording: RecordingSession,
    pub last_recording_path: String,
    pub available_disk_bytes: u64,
    pub replay_buffer: ReplayBuffer,
    pub saved_replays_count: u32,
    pub last_replay_packets: usize,

    pub virtual_cam: VirtualCamState,
    pub capture_exclusion: CaptureExclusionStatus,

    pub qc_player: QcPlayerState,
    pub last_splicy_handoff: Option<SplicyHandoff>,
    pub teleprompter: TeleprompterState,

    // Attached hardware / runtime state
    pub attached_streamdecks: Vec<StreamDeckModel>,
    pub filter_contexts: BTreeMap<SourceId, FilterContext>,
    pub clock_ms: u64,
    pub frame_counter: u64,
    pub log_lines: Vec<String>,
}

impl Clone for CabaretStudio {
    fn clone(&self) -> Self {
        Self {
            profile: self.profile.clone(),
            collection: self.collection.clone(),
            undo: self.undo.clone(),
            studios: self.studios,
            sources: self.sources,
            strips: self.strips,
            studio_mode: self.studio_mode.clone(),
            multiview_preset: self.multiview_preset,
            multiview_show_safe_areas: self.multiview_show_safe_areas,
            transition_preset: self.transition_preset,
            transition_duration_ms: self.transition_duration_ms,
            wipe_pattern_index: self.wipe_pattern_index,
            stinger: self.stinger.clone(),
            move_config: self.move_config.clone(),
            audio: self.audio.clone(),
            spectrum: self.spectrum.clone(),
            stereo_correlation: self.stereo_correlation,
            last_mixed_audio: self.last_mixed_audio.clone(),
            last_goniometer_frame: self.last_goniometer_frame.clone(),
            presentation: self.presentation.clone(),
            browser_init_js: self.browser_init_js.clone(),
            browser_events_log: self.browser_events_log.clone(),
            dsk: self.dsk.clone(),
            secondary_canvases: self.secondary_canvases.clone(),
            vertical_canvas_id: self.vertical_canvas_id,
            macros: self.macros.clone(),
            active_macro_events: self.active_macro_events.clone(),
            focused_silk_window: self.focused_silk_window.clone(),
            active_shader_index: self.active_shader_index,
            warp_3d_enabled: self.warp_3d_enabled,
            corner_pin: self.corner_pin,
            perspective_3d: self.perspective_3d,
            wasm_fx: WasmFxBridge::with_limits(*self.wasm_fx.limits()),
            source_tap: self.source_tap.clone(),
            freeze_filter: self.freeze_filter.clone(),
            time_warp: self.time_warp.clone(),
            time_warp_enabled: self.time_warp_enabled,
            delay_buffer: self.delay_buffer.clone(),
            captions: self.captions.clone(),
            burn_in_captions: self.burn_in_captions,
            last_cea608: self.last_cea608.clone(),
            last_cea708: self.last_cea708.clone(),
            link_enabled: self.link_enabled,
            link_announcement: self.link_announcement.clone(),
            link_last_packet_bytes: self.link_last_packet_bytes,
            now_playing: self.now_playing.clone(),
            input_overlay: self.input_overlay.clone(),
            color_pipeline: self.color_pipeline,
            router: self.router.clone(),
            active_secrets: self.active_secrets.clone(),
            stream_active: self.stream_active,
            stream_reconnecting: self.stream_reconnecting,
            stream_duration_ms: self.stream_duration_ms,
            stream_bytes_sent: self.stream_bytes_sent,
            stream_frames_sent: self.stream_frames_sent,
            stream_frames_dropped: self.stream_frames_dropped,
            recording: self.recording.clone(),
            last_recording_path: self.last_recording_path.clone(),
            available_disk_bytes: self.available_disk_bytes,
            replay_buffer: self.replay_buffer.clone(),
            saved_replays_count: self.saved_replays_count,
            last_replay_packets: self.last_replay_packets,
            virtual_cam: self.virtual_cam.clone(),
            capture_exclusion: self.capture_exclusion,
            qc_player: self.qc_player.clone(),
            last_splicy_handoff: self.last_splicy_handoff.clone(),
            teleprompter: self.teleprompter.clone(),
            attached_streamdecks: self.attached_streamdecks.clone(),
            filter_contexts: self.filter_contexts.clone(),
            clock_ms: self.clock_ms,
            frame_counter: self.frame_counter,
            log_lines: self.log_lines.clone(),
        }
    }
}

const SEEDED_PRESENTATION_MD: &str = r#"# Cabaret Studio Live
- Dual-Canvas Studio Mode (16:9 + 9:16 Vertical)
- All 12 Essential Broadcast Plugins Native in Rust
- Zero-Copy SexOS Silk Capture + HIDE_FROM_CAPTURE
<!-- transition: morph -->
<!-- build: 1 DissolveIn 350 -->
<!-- build: 2 DissolveIn 350 -->
<!-- notes: Welcome to Cabaret on SexOS! Press Take or Spacebar to cue next slide build. -->
---
# Wide-Gamut Color & Audio Mixing
- Linear FP32 Compositor + Rec.2020 / HDR10 PQ / HLG
- EBU R128 LUFS, 4x True-Peak dBTP & Sidechain Ducking
- Shibari Sealed Stream Keys + Multi-CDN RTMP / AV1 / SRT / WHIP
<!-- transition: dissolve -->
<!-- build: 1 DissolveIn 300 -->
<!-- notes: Notice Desktop audio ducks automatically when talking into Mic / Serenade. -->
---
# Instant Replay & Splicy NLE Handoff
- Hybrid MP4 + 6 Independent Audio Tracks + Chapters
- Built-in J/K/L QC Player -> 1-Click Splicy Timeline Export
<!-- notes: Wrap up the segment, mark Out, and open directly in Splicy. -->
"#;

const SEEDED_TELEPROMPTER_SCRIPT: &str = "\
Welcome to Cabaret on SexOS. Today we are showcasing live dual-canvas broadcast \
production with zero-copy Silk window capture, automatic microphone sidechain \
ducking, hardware-free software scopes, and instant one-click Splicy NLE handoff.";

impl CabaretStudio {
    /// Creates a fully seeded Cabaret Studio ready for broadcast, recording, and QC.
    pub fn new_seeded() -> Self {
        let mut col = SceneCollection::new("SexOS Live Broadcast", 1920, 1080);

        let main_camera_slides = col.add_studio("Main Camera & Slides");
        let screen_pip = col.add_studio("Screen + Picture-in-Picture");
        let coding_terminal = col.add_studio("Coding & Terminal");
        let brb_starting_soon = col.add_studio("BRB / Starting Soon");
        if let Some(st) = col.studio_mut(coding_terminal) {
            st.layout = StudioLayoutMode::Grid { columns: 2 };
        }
        let studios = SeededStudios {
            main_camera_slides,
            screen_pip,
            coding_terminal,
            brb_starting_soon,
        };

        // Reusable Broadcast Sources
        let backdrop_solid = col
            .add_source(
                "Studio Slate Backdrop",
                cabaret_core::scene::SourceKind::ColorSolid {
                    rgba: [0.04, 0.05, 0.08, 1.0],
                },
            )
            .expect("backdrop solid");

        let silk_window = col
            .add_source(
                "Silk Window (Focused)",
                cabaret_core::scene::SourceKind::SilkWindow {
                    frame_id: 1,
                    follow_focused: true,
                    hide_self: true,
                },
            )
            .expect("silk window");

        let uvc_camera = col
            .add_source(
                "UVC Camera (1080p60)",
                cabaret_core::scene::SourceKind::UvcCamera {
                    device_id: String::from("sexuvc:0"),
                    pixel_fmt: UvcPixelFormat::Nv12,
                    range_auto: true,
                },
            )
            .expect("uvc camera");
        if let Some(cam) = col.source_mut(uvc_camera) {
            let _ = cam.filters.add(FilterEntry {
                id: 1,
                name: String::from("Sharpen (Camera)"),
                enabled: true,
                filter: VideoFilter::Sharpen { amount: 0.22 },
            });
            let _ = cam.filters.add(FilterEntry {
                id: 2,
                name: String::from("Color Wheels (Warm Key)"),
                enabled: false,
                filter: VideoFilter::ColorWheels4Way {
                    lift: ColorWheel::ZERO,
                    gamma: ColorWheel {
                        r: 0.02,
                        g: 0.01,
                        b: -0.01,
                        master: 0.0,
                    },
                    gain: ColorWheel::ONE,
                    offset: ColorWheel::ZERO,
                    saturation: 1.08,
                },
            });
        }

        let presentation_deck = col
            .add_source(
                "Burlesque Live Deck",
                cabaret_core::scene::SourceKind::PresentationDeck {
                    path: String::from("/decks/cabaret-keynote.md"),
                    slide_idx: 0,
                    build_step: 0,
                },
            )
            .expect("presentation deck");

        let browser_alerts = col
            .add_source(
                "Kaleidoscope Alerts",
                cabaret_core::scene::SourceKind::Browser {
                    url: String::from("cabaret://alerts/overlay.html"),
                    width: 640,
                    height: 360,
                    custom_css: String::from("body { background: transparent; margin: 0; }"),
                    shutdown_when_hidden: false,
                    private_audio: true,
                },
            )
            .expect("browser alerts");

        let phone_srt = col
            .add_source(
                "Phone SRT Camera",
                cabaret_core::scene::SourceKind::PhoneStream {
                    protocol: PhoneProtocol::Srt,
                    port: 9000,
                    passphrase_or_key: String::from("[SHIBARI_SLOT#5:phone-srt-key]"),
                    hookup_device: String::from("Hookup-Cam-Alpha"),
                },
            )
            .expect("phone srt");

        let now_playing_card = col
            .add_source(
                "Now Playing (Bell/Qupid)",
                cabaret_core::scene::SourceKind::RichText {
                    content: String::from("Qupid: Midnight Broadcast - Synthwave FM"),
                    font_px: 20.0,
                    color: [0.95, 0.96, 1.0, 1.0],
                    outline: None,
                    shadow: None,
                    tail_lines: Some(2),
                },
            )
            .expect("now playing");

        let input_overlay = col
            .add_source(
                "Input Overlay HUD",
                cabaret_core::scene::SourceKind::RichText {
                    content: String::from("WASD + Mouse HUD"),
                    font_px: 16.0,
                    color: [0.15, 0.85, 1.0, 1.0],
                    outline: None,
                    shadow: None,
                    tail_lines: Some(2),
                },
            )
            .expect("input overlay");

        let sources = SeededSources {
            silk_window,
            uvc_camera,
            presentation_deck,
            browser_alerts,
            phone_srt,
            now_playing_card,
            input_overlay,
            backdrop_solid,
        };

        // Populate Studio 1: "Main Camera & Slides"
        let _ = col.add_item(main_camera_slides, backdrop_solid, "Studio Backdrop");
        if let Ok(item_id) = col.add_item(main_camera_slides, uvc_camera, "Main Camera (Left)") {
            configure_item(&mut col, main_camera_slides, item_id, [16.0, 18.0], [0.58, 0.92]);
        }
        if let Ok(item_id) =
            col.add_item(main_camera_slides, presentation_deck, "Burlesque Slides (PIP)")
        {
            configure_item(
                &mut col,
                main_camera_slides,
                item_id,
                [172.0, 22.0],
                [0.42, 0.48],
            );
        }
        if let Ok(item_id) =
            col.add_item(main_camera_slides, browser_alerts, "Kaleidoscope Alerts")
        {
            configure_item(
                &mut col,
                main_camera_slides,
                item_id,
                [176.0, 118.0],
                [0.38, 0.28],
            );
        }

        // Populate Studio 2: "Screen + Picture-in-Picture"
        let _ = col.add_item(screen_pip, backdrop_solid, "Screen Canvas Backdrop");
        if let Ok(item_id) = col.add_item(screen_pip, silk_window, "Focused Silk Window") {
            configure_item(&mut col, screen_pip, item_id, [10.0, 10.0], [0.92, 0.90]);
        }
        if let Ok(item_id) = col.add_item(screen_pip, uvc_camera, "Camera PIP (Bottom Right)") {
            configure_item(&mut col, screen_pip, item_id, [212.0, 104.0], [0.32, 0.36]);
        }

        // Populate Studio 3: "Coding & Terminal"
        let _ = col.add_item(coding_terminal, backdrop_solid, "Coding Backdrop");
        if let Ok(item_id) = col.add_item(coding_terminal, silk_window, "Spindle / Editor Window") {
            configure_item(&mut col, coding_terminal, item_id, [12.0, 12.0], [0.85, 0.85]);
        }
        if let Ok(item_id) = col.add_item(coding_terminal, input_overlay, "Keystroke HUD") {
            configure_item(&mut col, coding_terminal, item_id, [18.0, 124.0], [0.45, 0.25]);
        }
        if let Ok(item_id) = col.add_item(coding_terminal, phone_srt, "Phone Sidecam (SRT)") {
            configure_item(&mut col, coding_terminal, item_id, [216.0, 14.0], [0.28, 0.32]);
        }

        // Populate Studio 4: "BRB / Starting Soon"
        let _ = col.add_item(brb_starting_soon, backdrop_solid, "BRB Backdrop");
        if let Ok(item_id) =
            col.add_item(brb_starting_soon, presentation_deck, "Hold Slate Deck")
        {
            configure_item(
                &mut col,
                brb_starting_soon,
                item_id,
                [24.0, 20.0],
                [0.85, 0.65],
            );
        }
        if let Ok(item_id) =
            col.add_item(brb_starting_soon, now_playing_card, "Now Playing Card")
        {
            configure_item(
                &mut col,
                brb_starting_soon,
                item_id,
                [24.0, 134.0],
                [0.65, 0.22],
            );
        }

        // Studio Mode defaults: Program = Main Camera & Slides, Preview = Screen + PIP
        let mut studio_mode = StudioMode::new(screen_pip, main_camera_slides);
        studio_mode.active_transition = TransitionSpec::fade(300);

        // Seed 4-strip Audio Mixer (Mic BEFORE Desktop so sidechain ducking keys cleanly)
        let mut audio = AudioMixer::new(48_000, "Bell Main Out");
        let mic_id = StripId(1);
        let desktop_id = StripId(2);
        let browser_id = StripId(3);
        let media_id = StripId(4);

        let mut mic = AudioStrip::new(mic_id, "Mic / Serenade", ChannelLayout::Mono, 48_000);
        mic.track_mask = 0b00_0011; // Track 1 + 2
        mic.monitor_mode = MonitorMode::MonitorAndOutput;
        let _ = mic.add_filter(AudioFilterEntry {
            id: 1,
            name: String::from("Voice Denoise"),
            enabled: true,
            filter: AudioFilter::VoiceDenoise {
                strength: 0.55,
                floor_db: -48.0,
            },
        });
        let _ = mic.add_filter(AudioFilterEntry {
            id: 2,
            name: String::from("De-Esser 6.5kHz"),
            enabled: true,
            filter: AudioFilter::DeEsser {
                freq_hz: 6500.0,
                bandwidth_q: 2.0,
                threshold_db: -18.0,
                ratio: 3.0,
            },
        });
        let _ = mic.add_filter(AudioFilterEntry {
            id: 3,
            name: String::from("Vocal 3-Band EQ"),
            enabled: true,
            filter: AudioFilter::Eq3Band {
                low_db: -2.0,
                low_hz: 110.0,
                mid_db: 2.5,
                mid_hz: 2400.0,
                mid_q: 1.1,
                high_db: 1.5,
                high_hz: 8000.0,
            },
        });
        let _ = mic.add_filter(AudioFilterEntry {
            id: 4,
            name: String::from("Safety Limiter"),
            enabled: true,
            filter: AudioFilter::Limiter {
                threshold_db: -1.0,
                release_ms: 80.0,
            },
        });
        mic.add_moan_plugin(MoanPlugSlot {
            slot_id: 1,
            plugin_id: String::from("org.cabaret.loudmax"),
            format: MoanPlugFormat::Clap,
            bypass: false,
            params: vec![
                (String::from("threshold_db"), -1.5),
                (String::from("ceiling_db"), -0.3),
            ],
        });
        audio.add_strip(mic);

        let mut desktop =
            AudioStrip::new(desktop_id, "Desktop / Bell App Tap", ChannelLayout::Stereo, 48_000);
        desktop.track_mask = 0b00_0101; // Track 1 + 3
        desktop.captures_monitor_device_id = Some(String::from("Bell Main Out"));
        desktop.monitor_mode = MonitorMode::MonitorOff;
        let _ = desktop.add_filter(AudioFilterEntry {
            id: 10,
            name: String::from("Sidechain Duck (Mic)"),
            enabled: true,
            filter: AudioFilter::Compressor {
                ratio: 5.0,
                threshold_db: -24.0,
                attack_ms: 4.0,
                release_ms: 140.0,
                output_gain_db: 0.0,
                sidechain_source: Some(mic_id),
            },
        });
        audio.add_strip(desktop);

        let mut browser_strip =
            AudioStrip::new(browser_id, "Browser Alerts", ChannelLayout::Stereo, 48_000);
        browser_strip.track_mask = 0b00_1000; // Private Track 4
        browser_strip.monitor_mode = MonitorMode::MonitorAndOutput;
        let _ = browser_strip.add_filter(AudioFilterEntry {
            id: 20,
            name: String::from("Alert Trim -3 dB"),
            enabled: true,
            filter: AudioFilter::Gain { gain_db: -3.0 },
        });
        audio.add_strip(browser_strip);

        let mut media_strip =
            AudioStrip::new(media_id, "Media / Deck", ChannelLayout::Stereo, 48_000);
        media_strip.track_mask = 0b01_0001; // Track 1 + 5
        media_strip.fader_db = -6.0;
        let _ = media_strip.add_filter(AudioFilterEntry {
            id: 30,
            name: String::from("Program Gentle EQ"),
            enabled: false,
            filter: AudioFilter::Eq3Band {
                low_db: 0.0,
                low_hz: 120.0,
                mid_db: -1.5,
                mid_hz: 1800.0,
                mid_q: 0.9,
                high_db: 0.5,
                high_hz: 9000.0,
            },
        });
        audio.add_strip(media_strip);

        let strips = SeededStrips {
            mic_serenade: mic_id,
            desktop_bell: desktop_id,
            browser_alerts: browser_id,
            media_deck: media_id,
        };

        // Burlesque Live Deck
        let presentation = LivePresentationSource::from_markdown(SEEDED_PRESENTATION_MD)
            .expect("seeded burlesque markdown deck");

        // Downstream Keyer Bank (Logo Bug persistent on air, Lower-Third toggleable/tied)
        let mut dsk = DownstreamKeyerBank::new();
        let mut logo_bug = DskChannel::new(1, "Logo Bug (CABARET LIVE)", main_camera_slides)
            .with_transition(DskTransition::Fade { duration_ms: 200 })
            .with_z_order(10);
        logo_bug.active = true;
        logo_bug.visibility = 1.0;
        dsk.add_channel(logo_bug);

        let lower_third = DskChannel::new(2, "Lower-Third Speaker", main_camera_slides)
            .with_transition(DskTransition::Slide {
                direction: WipeDirection::Right,
                duration_ms: 250,
            })
            .with_tie_to_transition(true)
            .with_z_order(20);
        dsk.add_channel(lower_third);

        // Secondary 9:16 Vertical Canvas (Shorts / TikTok)
        let mut secondary_canvases = SecondaryCanvasManager::new();
        let vertical_canvas_id = secondary_canvases.create_canvas(
            "Vertical 9:16 (Shorts / TikTok)",
            CanvasPreset::Vertical9x16,
            60,
            32 * 1024 * 1024,
        );
        if let Some(vert) = secondary_canvases.canvas_mut(vertical_canvas_id) {
            vert.active_studio_id = Some(main_camera_slides);
            vert.add_default_item(SecondaryCanvasItem::new(
                uvc_camera,
                "Vertical Cam Crop",
                Transform2D {
                    pos: [0.0, 0.0],
                    scale: [1.0, 0.60],
                    ..Transform2D::default()
                },
            ));
            vert.add_default_item(SecondaryCanvasItem::new(
                presentation_deck,
                "Vertical Slide Strip",
                Transform2D {
                    pos: [0.0, 1120.0],
                    scale: [1.0, 0.42],
                    ..Transform2D::default()
                },
            ));
            vert.add_stream_dest(
                DestId(101),
                "YouTube Shorts Vertical",
                ServicePlatform::YouTube,
                StreamDest::Rtmp {
                    url: String::from("rtmp://a.rtmp.youtube.com/live2"),
                    secret_slot: ShibariSecretSlot::new(10, "yt-vertical-key"),
                },
                VideoEncodeProfile {
                    width: 1080,
                    height: 1920,
                    fps_num: 60,
                    fps_den: 1,
                    codec: VideoCodec::H264,
                    bitrate_kbps: 8000,
                    keyint_sec: 2,
                    ..VideoEncodeProfile::default()
                },
                AudioEncodeProfile::default(),
            );
        }

        // Macro Engine (Advanced Scene Switcher rules)
        let mut macros = MacroEngine::new();
        macros.add_rule(
            MacroRule::new(
                1,
                "Auto-Switch to Coding on Spindle Focus",
                10,
                Condition::FocusedSilkWindow {
                    frame_id: None,
                    title_contains: Some(String::from("Spindle")),
                },
                vec![MacroAction::SwitchStudio {
                    studio_id: coding_terminal,
                    preview_only: false,
                }],
            )
            .with_hold_ms(0)
            .with_cooldown_ms(100),
        );
        macros.add_rule(
            MacroRule::new(
                2,
                "Highlight Lower-Third When Mic Hot",
                5,
                Condition::AudioThresholdDb {
                    strip_id: mic_id,
                    threshold_db: -22.0,
                    above: true,
                    hysteresis_db: 2.0,
                },
                vec![MacroAction::SetDskActive {
                    channel_id: 2,
                    active: true,
                }],
            )
            .with_hold_ms(0)
            .with_cooldown_ms(250),
        );

        // Multi-stream targets + Shibari secrets
        let mut router = MultiStreamRouter::new();
        let active_secrets = vec![
            (
                ShibariSecretSlot::new(1, "twitch-stream-key"),
                String::from("live_998877_secret_twitch_tok"),
            ),
            (
                ShibariSecretSlot::new(2, "youtube-stream-key"),
                String::from("yt_445566_secret_av1_tok"),
            ),
            (
                ShibariSecretSlot::new(3, "srt-passphrase"),
                String::from("CabaretSuperSecretSrtPassphrase"),
            ),
            (
                ShibariSecretSlot::new(4, "whip-bearer"),
                String::from("whip_bearer_99112233"),
            ),
            (
                ShibariSecretSlot::new(5, "phone-srt-key"),
                String::from("phone_hookup_srt_pass"),
            ),
        ];

        router.add_target(StreamTarget {
            id: DestId(1),
            name: String::from("Twitch Primary (H.264 6Mbps)"),
            platform: ServicePlatform::Twitch,
            dest: StreamDest::Rtmp {
                url: String::from("rtmp://live.twitch.tv/app"),
                secret_slot: active_secrets[0].0.clone(),
            },
            video: VideoEncodeProfile {
                width: 1920,
                height: 1080,
                fps_num: 60,
                fps_den: 1,
                codec: VideoCodec::H264,
                bitrate_kbps: 6000,
                keyint_sec: 2,
                ..VideoEncodeProfile::default()
            },
            audio: AudioEncodeProfile::default(),
            enabled: true,
            delay_sec: 0,
        });

        router.add_target(StreamTarget {
            id: DestId(2),
            name: String::from("YouTube Enhanced-RTMP (AV1 12Mbps)"),
            platform: ServicePlatform::YouTube,
            dest: StreamDest::EnhancedRtmp {
                url: String::from("rtmp://a.rtmp.youtube.com/live2"),
                secret_slot: active_secrets[1].0.clone(),
                codec: VideoCodec::Av1,
            },
            video: VideoEncodeProfile {
                width: 1920,
                height: 1080,
                fps_num: 60,
                fps_den: 1,
                codec: VideoCodec::Av1,
                bitrate_kbps: 12000,
                keyint_sec: 2,
                ..VideoEncodeProfile::default()
            },
            audio: AudioEncodeProfile::default(),
            enabled: true,
            delay_sec: 2,
        });

        router.add_target(StreamTarget {
            id: DestId(3),
            name: String::from("SRT Studio Relay (Shared H.264)"),
            platform: ServicePlatform::Custom,
            dest: StreamDest::Srt {
                host: String::from("relay.cabaret.local"),
                port: 9001,
                passphrase_slot: Some(active_secrets[2].0.clone()),
                latency_ms: 120,
                stream_id: String::from("cabaret-main"),
            },
            video: VideoEncodeProfile {
                width: 1920,
                height: 1080,
                fps_num: 60,
                fps_den: 1,
                codec: VideoCodec::H264,
                bitrate_kbps: 6000,
                keyint_sec: 2,
                ..VideoEncodeProfile::default()
            },
            audio: AudioEncodeProfile::default(),
            enabled: true,
            delay_sec: 0,
        });

        router.add_target(StreamTarget {
            id: DestId(4),
            name: String::from("WebRTC WHIP Edge"),
            platform: ServicePlatform::Custom,
            dest: StreamDest::Whip {
                endpoint_url: String::from("https://edge.cabaret.local/whip"),
                bearer_slot: Some(active_secrets[3].0.clone()),
            },
            video: VideoEncodeProfile {
                width: 1920,
                height: 1080,
                fps_num: 60,
                fps_den: 1,
                codec: VideoCodec::H264,
                bitrate_kbps: 6000,
                keyint_sec: 2,
                ..VideoEncodeProfile::default()
            },
            audio: AudioEncodeProfile::default(),
            enabled: true,
            delay_sec: 0,
        });

        router.add_target(StreamTarget {
            id: DestId(5),
            name: String::from("RIST Backup Trunk"),
            platform: ServicePlatform::Custom,
            dest: StreamDest::Rist {
                url: String::from("rist://backup.cabaret.local:9900"),
                buffer_ms: 250,
                secret_slot: None,
            },
            video: VideoEncodeProfile::default(),
            audio: AudioEncodeProfile::default(),
            enabled: false,
            delay_sec: 0,
        });

        // Captions seeded with opening line
        let mut captions = CaptionBuffer::default();
        captions.push_transcript_segment(
            TranscriptSegment::new(1, 0, 3500, "Welcome to Cabaret Live Studio on SexOS.")
                .with_speaker("Host"),
        );

        // Now Playing & Input Overlay
        let now_playing = NowPlayingCard::new(
            NowPlayingTrack {
                title: String::from("Neon Studio Pulse"),
                artist: String::from("Cabaret Ensemble"),
                album: String::from("Live Sessions"),
                player_app: String::from("Qupid"),
                player_pd: 12,
                duration_ms: 215_000,
                position_ms: 42_000,
                is_playing: true,
                muted: false,
                volume_pct: 84,
                cover_art: None,
            },
            Default::default(),
        );
        let mut input_overlay = InputOverlayHud::new(Default::default());
        input_overlay.update(
            InputStateSnapshot {
                ctrl: true,
                w: true,
                mouse_left: true,
                mouse_dx: 14.0,
                mouse_dy: -6.0,
                ..InputStateSnapshot::default()
            },
            16.0,
        );

        let mut st = Self {
            profile: Profile {
                name: String::from("Cabaret 1080p60 Broadcast"),
                monitoring_device: String::from("Bell Main Out"),
                ..Profile::default()
            },
            collection: col,
            undo: UndoStack::new(256),
            studios,
            sources,
            strips,
            studio_mode,
            multiview_preset: MultiviewPreset::Standard8,
            multiview_show_safe_areas: true,

            transition_preset: ActiveTransitionPreset::Fade,
            transition_duration_ms: 300,
            wipe_pattern_index: 4, // IrisCircle
            stinger: StingerConfig {
                asset_path: String::from("/stingers/cabaret-swipe.webm"),
                duration_ms: 600,
                ..StingerConfig::default()
            },
            move_config: MoveTransitionConfig {
                duration_ms: 450,
                easing: EasingCurve::CubicInOut,
                enter_slide: [-80.0, 0.0],
                exit_slide: [80.0, 0.0],
                ..MoveTransitionConfig::default()
            },

            audio,
            spectrum: SpectrumAnalyzer::new(SpectrumConfig::default()),
            stereo_correlation: 0.92,
            last_mixed_audio: None,
            last_goniometer_frame: None,

            presentation,
            browser_init_js: generate_init_js(ControlLevel::Advanced),
            browser_events_log: vec![String::from("window.obsstudio v30.0.0-cabaret initialized")],

            dsk,
            secondary_canvases,
            vertical_canvas_id,
            macros,
            active_macro_events: Vec::new(),
            focused_silk_window: String::from("Cabaret Studio"),
            active_shader_index: None,
            warp_3d_enabled: false,
            corner_pin: CornerPinQuad::unit(),
            perspective_3d: PitchYawRollFov::default(),
            wasm_fx: WasmFxBridge::new(),
            source_tap: SourceRecordTap::new(
                uvc_camera,
                ContainerFormat::HybridMp4,
                AudioTrackMask::ALL_6,
                60,
                16 * 1024 * 1024,
            ),
            freeze_filter: FreezeFilter::new(),
            time_warp: TimeWarpScan::new(TimeWarpConfig::default()),
            time_warp_enabled: false,
            delay_buffer: DynamicDelayBuffer::new(120),
            captions,
            burn_in_captions: false,
            last_cea608: Vec::new(),
            last_cea708: None,
            link_enabled: true,
            link_announcement: ServiceAnnouncement::default(),
            link_last_packet_bytes: 0,
            now_playing,
            input_overlay,

            color_pipeline: CanvasColorPipeline::default(),

            router,
            active_secrets,
            stream_active: false,
            stream_reconnecting: false,
            stream_duration_ms: 0,
            stream_bytes_sent: 0,
            stream_frames_sent: 0,
            stream_frames_dropped: 0,

            recording: RecordingSession::new(ContainerFormat::HybridMp4, AudioTrackMask::ALL_6),
            last_recording_path: String::from("/recordings/cabaret-session-001.mp4"),
            available_disk_bytes: 64 * 1024 * 1024 * 1024,
            replay_buffer: ReplayBuffer::new(60, 16 * 1024 * 1024),
            saved_replays_count: 0,
            last_replay_packets: 0,

            virtual_cam: VirtualCamState {
                active: false,
                device_name: String::from("Cabaret Virtual Camera"),
                source_label: String::from("Program"),
                sequence: 0,
                epoch: 1,
            },
            capture_exclusion: CaptureExclusionStatus {
                cabaret_ui_hidden: true,
                teleprompter_hidden: true,
                silkbar_tally_active: false,
                capture_pixel_format: "XRGB8888",
            },

            qc_player: QcPlayerState::new(
                "/recordings/cabaret-session-001.mp4",
                30 * SECOND,
                60,
                1,
            ),
            last_splicy_handoff: None,
            teleprompter: TeleprompterState::new(SEEDED_TELEPROMPTER_SCRIPT),

            attached_streamdecks: vec![StreamDeckModel::Mk2, StreamDeckModel::Plus],
            filter_contexts: BTreeMap::new(),
            clock_ms: 0,
            frame_counter: 0,
            log_lines: Vec::new(),
        };

        // Seed initial replay buffer GOP + audio meters + CEA-608/708 packets
        st.seed_initial_packets();
        st.tick_audio(0.18, 0.25, 4800);
        st.update_caption_packets();
        st.log_event("Cabaret studio initialized (4 studios, 4 strips, 12 plugins)");
        st
    }

    fn seed_initial_packets(&mut self) {
        let dur_step_us = 250_000i64;
        for i in 0..8i64 {
            let pts_us = i * dur_step_us;
            let pkt = EncodedPacket {
                pts_us,
                dts_us: pts_us,
                is_keyframe: i % 4 == 0,
                kind: MediaKind::Video,
                track_index: 0,
                payload: vec![0xAA; 64],
            };
            self.replay_buffer.push_packet(pkt);
        }
    }

    /// Appends a log line with automatic Shibari stream-key/passphrase redaction.
    pub fn log_event(&mut self, msg: &str) {
        let pairs: Vec<(ShibariSecretSlot, &str)> = self
            .active_secrets
            .iter()
            .map(|(s, v)| (s.clone(), v.as_str()))
            .collect();
        let safe = redact_log_line(msg, &pairs);
        self.log_lines.push(safe);
        if self.log_lines.len() > 64 {
            self.log_lines.remove(0);
        }
    }

    /// Returns name of a studio ID, if present.
    pub fn studio_name(&self, id: StudioId) -> &str {
        self.collection
            .studio(id)
            .map(|s| s.name.as_str())
            .unwrap_or("Unknown Studio")
    }

    /// Resolves studio by exact name (case-sensitive first, then ASCII case-insensitive).
    pub fn find_studio_by_name(&self, name: &str) -> Option<StudioId> {
        self.collection
            .studios
            .iter()
            .find(|s| s.name == name)
            .or_else(|| {
                self.collection
                    .studios
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(name))
            })
            .map(|s| s.id)
    }

    /// Applies the active transition preset into `self.studio_mode.active_transition`.
    pub fn select_transition_preset(&mut self, preset: ActiveTransitionPreset) {
        self.transition_preset = preset;
        let dur = self.transition_duration_ms;
        self.studio_mode.active_transition = match preset {
            ActiveTransitionPreset::Cut => TransitionSpec::cut(),
            ActiveTransitionPreset::Fade => TransitionSpec::fade(dur),
            ActiveTransitionPreset::Wipe29 => TransitionSpec::wipe(self.current_wipe_pattern(), dur, 0.08),
            ActiveTransitionPreset::Stinger => TransitionSpec {
                name: String::from("Stinger"),
                kind: TransitionKind::Stinger {
                    config: self.stinger.clone(),
                },
                duration_ms: self.stinger.duration_ms,
            },
            ActiveTransitionPreset::Move => TransitionSpec {
                name: String::from("Move"),
                kind: TransitionKind::Fade,
                duration_ms: self.move_config.duration_ms,
            },
        };
    }

    pub fn current_wipe_pattern(&self) -> WipePattern {
        ALL_WIPE_PATTERNS[self.wipe_pattern_index % ALL_WIPE_PATTERNS.len()]
    }

    pub fn cycle_wipe_pattern(&mut self, delta: isize) {
        let n = ALL_WIPE_PATTERNS.len() as isize;
        let next = ((self.wipe_pattern_index as isize + delta).rem_euclid(n)) as usize;
        self.wipe_pattern_index = next;
        if self.transition_preset == ActiveTransitionPreset::Wipe29 {
            self.select_transition_preset(ActiveTransitionPreset::Wipe29);
        }
    }

    /// Sets the Studio Mode Preview studio and keeps linked secondary canvases in sync if desired.
    pub fn set_preview_studio(&mut self, id: StudioId) -> bool {
        if self.collection.studio(id).is_none() {
            return false;
        }
        self.studio_mode.preview_studio = id;
        true
    }

    /// Sets Program directly (when not using Studio Mode or via direct switch).
    pub fn set_program_studio(&mut self, id: StudioId) -> bool {
        if self.collection.studio(id).is_none() {
            return false;
        }
        self.studio_mode.program_studio = id;
        self.secondary_canvases.on_main_studio_switched(id);
        true
    }

    /// Triggers Studio Mode `TAKE`, optionally completing any timed transition synchronously.
    ///
    /// Returns the new Program `StudioId`.
    pub fn trigger_take(&mut self, complete_immediately: bool) -> StudioId {
        let tick = self.studio_mode.take_with_collection(&mut self.collection);
        self.dsk.on_main_transition_triggered();
        if complete_immediately && matches!(tick, TransitionTick::InProgress { .. }) {
            let dur = self.studio_mode.running.as_ref().map(|r| r.spec.duration_ms).unwrap_or(1000) as f32;
            self.studio_mode.tick(dur + 10.0);
        }
        let pgm = self.studio_mode.program_studio;
        self.secondary_canvases.on_main_studio_switched(pgm);
        self.log_event(&format!(
            "TAKE -> Program='{}' ({})",
            self.studio_name(pgm),
            self.transition_preset.label()
        ));
        pgm
    }

    /// Updates interactive T-bar `[0.0 ..= 1.0]`, committing swap when pushed all the way.
    pub fn set_t_bar(&mut self, pos: f32) -> Option<StudioId> {
        self.studio_mode.set_t_bar(pos);
        let switched = self.studio_mode.complete_t_bar(Some(&mut self.collection));
        if let Some(pgm) = switched {
            self.dsk.on_main_transition_triggered();
            self.secondary_canvases.on_main_studio_switched(pgm);
        }
        switched
    }

    /// Computes live Multiview layout for `(width, height)`.
    pub fn multiview_layout(&self, width: f32, height: f32) -> MultiviewLayout {
        let ids: Vec<StudioId> = self.collection.studios.iter().map(|s| s.id).collect();
        let mut layout = MultiviewLayout::compute(
            self.multiview_preset,
            width,
            height,
            6.0,
            &ids,
            &self.studio_mode,
        );
        layout.show_safe_areas = self.multiview_show_safe_areas;
        layout
    }

    /// Handles a Multiview viewport click (`click_count == 1` -> Preview, `>= 2` -> Take).
    pub fn multiview_click(
        &mut self,
        width: f32,
        height: f32,
        px: f32,
        py: f32,
        click_count: u8,
    ) -> Option<cabaret_core::transitions::MultiviewAction> {
        let mv = self.multiview_layout(width, height);
        let action = mv.handle_click(
            px,
            py,
            click_count,
            &mut self.studio_mode,
            Some(&mut self.collection),
        );
        if let Some(cabaret_core::transitions::MultiviewAction::DoubleClickToTake(_)) = action {
            self.dsk.on_main_transition_triggered();
            if self.studio_mode.running.is_some() {
                let dur = self
                    .studio_mode
                    .running
                    .as_ref()
                    .map(|r| r.spec.duration_ms)
                    .unwrap_or(500) as f32;
                self.studio_mode.tick(dur + 1.0);
            }
            self.secondary_canvases
                .on_main_studio_switched(self.studio_mode.program_studio);
        }
        action
    }

    /// Advances the embedded Burlesque presentation deck one build step / slide forward
    /// and synchronizes `SourceKind::PresentationDeck`.
    pub fn presentation_next_step(&mut self) -> bool {
        let ok = self.presentation.next_step();
        self.presentation.flush_animations();
        self.sync_presentation_source_meta();
        ok
    }

    /// Steps the embedded Burlesque presentation deck backward one build step / slide.
    pub fn presentation_prev_step(&mut self) -> bool {
        let ok = self.presentation.prev_step();
        self.presentation.flush_animations();
        self.sync_presentation_source_meta();
        ok
    }

    fn sync_presentation_source_meta(&mut self) {
        let (slide_idx, _, step_idx, _) = self.presentation.state();
        if let Some(src) = self.collection.source_mut(self.sources.presentation_deck) {
            if let cabaret_core::scene::SourceKind::PresentationDeck {
                slide_idx: s,
                build_step: b,
                ..
            } = &mut src.kind
            {
                *s = slide_idx as u32;
                *b = step_idx as u32;
            }
        }
    }

    /// Synthesizes a deterministic audio block, mixes all 4 strips with DSP + Sidechain Ducking
    /// + 4x True-Peak + EBU R128 LUFS metering, and updates FFT Spectrum & Goniometer correlation.
    pub fn tick_audio(
        &mut self,
        mic_amp: f32,
        desktop_amp: f32,
        frame_count: usize,
    ) -> MixedAudioOutput {
        let frames = frame_count.max(64);
        let mut mic_mono = vec![0.0_f32; frames];
        let mut desk_stereo = vec![0.0_f32; frames * 2];
        let mut media_stereo = vec![0.0_f32; frames * 2];
        let sr = 48_000.0_f32;
        let phase_base = self.clock_ms as f32 * 0.001;

        for i in 0..frames {
            let t = phase_base + i as f32 / sr;
            let voice = libm::sinf(2.0 * core::f32::consts::PI * 220.0 * t)
                + 0.35 * libm::sinf(2.0 * core::f32::consts::PI * 660.0 * t);
            mic_mono[i] = mic_amp * voice * 0.72;

            let music_l = libm::sinf(2.0 * core::f32::consts::PI * 440.0 * t);
            let music_r = libm::sinf(2.0 * core::f32::consts::PI * 554.37 * t);
            desk_stereo[i * 2] = desktop_amp * music_l;
            desk_stereo[i * 2 + 1] = desktop_amp * music_r;

            media_stereo[i * 2] = 0.08 * music_l;
            media_stereo[i * 2 + 1] = 0.08 * music_r;
        }

        let mut inputs: BTreeMap<StripId, &[f32]> = BTreeMap::new();
        inputs.insert(self.strips.mic_serenade, &mic_mono);
        inputs.insert(self.strips.desktop_bell, &desk_stereo);
        inputs.insert(self.strips.media_deck, &media_stereo);

        let mixed = self
            .audio
            .mix_block(&inputs, frames)
            .expect("audio mixer block");
        let dt_ms = (frames as f32 / sr) * 1000.0;
        self.spectrum.analyze(&mixed.tracks[0], dt_ms);
        self.stereo_correlation = compute_stereo_correlation(&mixed.tracks[0]);
        self.last_mixed_audio = Some(mixed.clone());
        mixed
    }

    pub fn strip_meters(&self, id: StripId) -> AudioMeters {
        self.last_mixed_audio
            .as_ref()
            .and_then(|m| m.strip_meters.get(&id).copied())
            .unwrap_or_default()
    }

    /// Evaluates Advanced Scene Switcher (`MacroEngine`) rules for a focused Silk window title.
    pub fn evaluate_macros_for_window(&mut self, title: &str) -> Vec<MacroTriggerEvent> {
        self.focused_silk_window = String::from(title);
        let mut strip_levels_db = BTreeMap::new();
        for strip in &self.audio.strips {
            strip_levels_db.insert(strip.id, strip.meter.last_meters.sample_peak_db);
        }
        let world = MacroWorldState {
            now_ms: self.clock_ms,
            focused_window_frame_id: Some(1),
            focused_window_title: self.focused_silk_window.clone(),
            strip_levels_db,
            media: BTreeMap::new(),
            user_idle_ms: 0,
            stream_state: if self.stream_active {
                StreamActiveState::Live
            } else {
                StreamActiveState::Stopped
            },
            recording_state: match self.recording.state {
                RecordingState::Recording => {
                    cabaret_plugins::adv_switcher::RecordingActiveState::Recording
                }
                RecordingState::Paused => {
                    cabaret_plugins::adv_switcher::RecordingActiveState::Paused
                }
                _ => cabaret_plugins::adv_switcher::RecordingActiveState::Stopped,
            },
        };

        let events = self.macros.evaluate(&world);
        if !events.is_empty() {
            let cmds = execute_macro_actions(
                &events,
                &mut self.studio_mode,
                &mut self.collection,
                &mut self.audio,
                &mut self.dsk,
            );
            // Finish any animated Take immediately when triggered by macro rule.
            if self.studio_mode.running.is_some() {
                let dur = self
                    .studio_mode
                    .running
                    .as_ref()
                    .map(|r| r.spec.duration_ms)
                    .unwrap_or(500) as f32;
                self.studio_mode.tick(dur + 1.0);
                self.secondary_canvases
                    .on_main_studio_switched(self.studio_mode.program_studio);
            }
            for cmd in cmds {
                match cmd {
                    MacroOutputCommand::StartRecord => self.start_recording(),
                    MacroOutputCommand::StopRecord => {
                        let _ = self.stop_recording();
                    }
                    MacroOutputCommand::SaveReplay { seconds } => {
                        let _ = self.save_replay(seconds);
                    }
                }
            }
            self.active_macro_events = events.clone();
        }
        events
    }

    pub fn active_shader_preset(&self) -> Option<ShaderPreset> {
        let all = ShaderPreset::all_defaults();
        self.active_shader_index.map(|idx| all[idx % all.len()])
    }

    pub fn push_caption_line(&mut self, speaker: Option<&str>, text: &str) {
        let next_id = self.captions.segments().len() as u64 + 1;
        let start = self.clock_ms;
        let mut seg = TranscriptSegment::new(next_id, start, start + 3500, text);
        if let Some(sp) = speaker {
            seg = seg.with_speaker(sp);
        }
        self.captions.push_transcript_segment(seg);
        self.update_caption_packets();
    }

    pub fn update_caption_packets(&mut self) {
        let line = self
            .captions
            .current_lines()
            .last()
            .cloned()
            .unwrap_or_default();
        self.last_cea608 = encode_cea608_pairs(&line, Cea608Mode::PopOn);
        self.last_cea708 = Some(packetize_cea708(&line, 1, (self.frame_counter & 3) as u8));
    }

    /// Packages `CabaretLink` LAN video frame + Program/Preview tally packet.
    pub fn emit_cabaret_link_packet(&mut self, program_frame: &FrameBuffer) -> Option<Vec<u8>> {
        if !self.link_enabled {
            return None;
        }
        let video = LinkVideoFrame::encode(program_frame, LinkPixelFormat::RleRgba8).ok()?;
        let pkt = LinkPacket {
            seq: self.frame_counter as u32,
            timestamp_us: (self.clock_ms as i64) * 1000,
            payload: LinkPayload::Video(video),
        };
        let bytes = pkt.encode();
        self.link_last_packet_bytes = bytes.len();
        Some(bytes)
    }

    pub fn link_tally_packet(&self) -> Vec<u8> {
        let pkt = LinkPacket {
            seq: self.frame_counter as u32,
            timestamp_us: (self.clock_ms as i64) * 1000,
            payload: LinkPayload::Tally(LinkTally {
                program: true,
                preview: false,
                stream_name: String::from(self.studio_name(self.studio_mode.program_studio)),
            }),
        };
        pkt.encode()
    }

    /// Plans shared vs split encoders across enabled stream targets.
    pub fn encoder_groups(&self) -> Vec<EncoderGroup> {
        EncoderGroupPlanner::plan(&self.router.targets)
    }

    pub fn validate_streams(&self) -> Vec<ServiceValidationIssue> {
        self.router.validate_all()
    }

    pub fn start_streaming(&mut self) {
        self.stream_active = true;
        self.capture_exclusion.silkbar_tally_active = true;
        self.log_event("Streaming started across enabled multi-stream targets");
    }

    pub fn stop_streaming(&mut self) {
        self.stream_active = false;
        self.capture_exclusion.silkbar_tally_active =
            self.recording.state == RecordingState::Recording || self.virtual_cam.active;
        self.log_event("Streaming stopped");
    }

    pub fn start_recording(&mut self) {
        self.recording.format = ContainerFormat::HybridMp4;
        self.recording.track_mask = AudioTrackMask::ALL_6;
        self.recording.start();
        self.capture_exclusion.silkbar_tally_active = true;
        self.log_event("Recording started (HybridMP4, 6 audio tracks)");
    }

    pub fn stop_recording(&mut self) -> String {
        self.recording.stop();
        self.capture_exclusion.silkbar_tally_active = self.stream_active || self.virtual_cam.active;
        self.qc_player.source_path = self.last_recording_path.clone();
        self.qc_player.duration_us = (self.recording.last_active_pts_us).max(10 * SECOND);
        self.log_event(&format!("Recording saved to {}", self.last_recording_path));
        self.last_recording_path.clone()
    }

    pub fn toggle_record_pause(&mut self) {
        let now_us = (self.clock_ms as i64) * 1000;
        match self.recording.state {
            RecordingState::Recording => {
                self.recording.pause(now_us);
                self.log_event("Recording paused");
            }
            RecordingState::Paused => {
                self.recording.resume(now_us);
                self.log_event("Recording resumed");
            }
            _ => {}
        }
    }

    pub fn add_recording_chapter(&mut self, title: &str) {
        self.recording.add_chapter(title);
        self.log_event(&format!("Chapter marker added: {title}"));
    }

    pub fn split_recording_file(&mut self) {
        self.recording.split_rule = SplitRule::Manual;
        self.recording.request_manual_split();
        let now_us = (self.clock_ms as i64) * 1000;
        let _ = self.recording.push_packet(
            &EncodedPacket {
                pts_us: now_us,
                dts_us: now_us,
                is_keyframe: true,
                kind: MediaKind::Video,
                track_index: 0,
                payload: vec![0xEE; 128],
            },
            self.available_disk_bytes,
        );
    }

    pub fn save_replay(&mut self, seconds: f32) -> Result<usize, OutputError> {
        let pkts = self.replay_buffer.save_last_seconds(seconds)?;
        let count = pkts.len();
        self.saved_replays_count += 1;
        self.last_replay_packets = count;
        self.log_event(&format!("Saved {seconds:.0}s replay buffer ({count} packets)"));
        Ok(count)
    }

    pub fn toggle_virtual_cam(&mut self) -> bool {
        self.virtual_cam.active = !self.virtual_cam.active;
        if self.virtual_cam.active {
            self.virtual_cam.epoch += 1;
            self.virtual_cam.sequence += 1;
        }
        self.capture_exclusion.silkbar_tally_active =
            self.stream_active || self.recording.state == RecordingState::Recording || self.virtual_cam.active;
        self.virtual_cam.active
    }

    /// 1-click "Open Last Recording in Splicy": constructs a synchronized `sex_splice::Timeline`,
    /// OpenTimelineIO JSON, and full FCPXML/EDL/SRT handoff bundle.
    pub fn open_last_recording_in_splicy(&mut self) -> Result<SplicyHandoff, OutputError> {
        let clip = self.qc_player.export_splicy_clip()?;
        let mut tl = Timeline::new(
            self.qc_player.fps_num,
            self.qc_player.fps_den,
        );
        let dur = self.qc_player.duration_us.max(SECOND);
        let m_vid = tl.add_media(SpliceMediaKind::Video, &clip.source_path, dur);
        let t_vid = tl.add_track(TrackKind::Video, "V1 Program");
        let t_aud = tl.add_track(TrackKind::Audio, "A1 Mix");

        let range = Range::new(clip.mark_in_us, clip.mark_out_us.min(dur));
        if !range.is_empty() {
            if let Ok(next) = tl.apply_all(&[
                Op::AddClip {
                    track: t_vid,
                    media: m_vid,
                    at: 0,
                    source: range,
                },
                Op::AddClip {
                    track: t_aud,
                    media: m_vid,
                    at: 0,
                    source: range,
                },
            ]) {
                tl = next;
            }
        }
        for ch in &self.recording.chapters {
            let rel_at = (ch.timestamp_us - clip.mark_in_us).clamp(0, range.len());
            let _ = tl.apply(&Op::AddMarker {
                at: rel_at,
                label: ch.title.clone(),
            });
        }

        let otio_json = otio::export(&tl, "Cabaret Broadcast");
        let bundle = build_handoff_bundle(&tl, None, "Cabaret Broadcast");
        let handoff = SplicyHandoff {
            clip,
            timeline: tl,
            otio_json,
            bundle,
        };
        self.last_splicy_handoff = Some(handoff.clone());
        self.log_event("Exported marked clip to Splicy timeline + OTIO bundle");
        Ok(handoff)
    }

    /// Whether something live needs frames even with no input: a stream, a recording
    /// or the virtual camera.
    pub fn wants_frames(&self) -> bool {
        self.stream_active || self.recording.state == RecordingState::Recording || self.virtual_cam.active
    }

    /// Advances clocks, live animations, active outputs, DSK transitions, and delay buffers.
    pub fn step_frame(&mut self, dt_ms: u32) {
        self.clock_ms = self.clock_ms.saturating_add(dt_ms as u64);
        self.frame_counter = self.frame_counter.saturating_add(1);

        // Advance Studio Mode running transition if active
        if let TransitionTick::Completed { new_program } = self.studio_mode.tick(dt_ms as f32) {
            self.secondary_canvases.on_main_studio_switched(new_program);
        }
        self.dsk.tick(dt_ms as f32);
        self.presentation.tick(dt_ms);
        self.now_playing.advance_time(dt_ms as u64);
        self.teleprompter.advance_time(dt_ms as f32 / 1000.0);

        let pts_us = (self.clock_ms as i64) * 1000;
        let is_kf = self.frame_counter % 30 == 0;
        let vid_pkt = EncodedPacket {
            pts_us,
            dts_us: pts_us,
            is_keyframe: is_kf,
            kind: MediaKind::Video,
            track_index: 0,
            payload: vec![0x55; 96],
        };
        self.replay_buffer.push_packet(vid_pkt.clone());
        if self.recording.state == RecordingState::Recording {
            let step = self.recording.push_packet(&vid_pkt, self.available_disk_bytes);
            if matches!(step, RecordingStep::StoppedLowDiskSpace) {
                self.log_event("Recording auto-stopped: low disk space");
            }
        }
        if self.stream_active {
            self.stream_duration_ms += dt_ms as u64;
            self.stream_frames_sent += 1;
            self.stream_bytes_sent += 16_384;
        }
        if self.virtual_cam.active {
            self.virtual_cam.sequence += 1;
        }
    }
}

fn configure_item(
    col: &mut SceneCollection,
    studio_id: StudioId,
    item_id: ItemId,
    pos: [f32; 2],
    scale: [f32; 2],
) {
    if let Some(st) = col.studio_mut(studio_id) {
        if let Some(it) = st.items.iter_mut().find(|i| i.id == item_id) {
            it.transform = Transform2D {
                pos,
                scale,
                bounds_mode: BoundsMode::None,
                ..Transform2D::default()
            };
            it.blend_mode = BlendMode::Normal;
            it.mix_alpha_or_default();
        }
    }
}

trait SceneItemExt {
    fn mix_alpha_or_default(&mut self);
}

impl SceneItemExt for cabaret_core::scene::SceneItem {
    fn mix_alpha_or_default(&mut self) {
        self.opacity = 1.0;
        self.opacity_space = MixColorSpace::Linear;
        self.scaler = Scaler::Bilinear;
    }
}
