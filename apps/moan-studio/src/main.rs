// SPDX-License-Identifier: GPL-3.0-only
//! Moan Studio for SexOS — Logic Noir egui DAW with Muse AI (`moan-gen`),
//! Ariel Formant Vocal Synth (`moan-core`), EuropaRack Modular (`moan-modular`),
//! and native Bell 48 kHz stereo audio playback (`sex-app`).

mod extras;

use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, FontId, Key, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind, Ui,
};
use moan_core::vocal::{self, VoiceParams};
use moan_core::{Frame, RATE};
use moan_gen::{parse_studio_chat, MuseJobRequest, MuseMode, RamTier, StemRole, Tier0Engine};
use moan_modular::{FactoryPatch, PatchGraph};
use std::sync::Arc;

// ---------------------------------------------------------------- Logic Noir Palette
const BG: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x1f);
const BAR_BG: Color32 = Color32::from_rgb(0x22, 0x22, 0x26);
const PANEL: Color32 = Color32::from_rgb(0x24, 0x24, 0x28);
const PANEL_DARK: Color32 = Color32::from_rgb(0x19, 0x19, 0x1d);
const INSPECTOR_BG: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x1f);
const MIXER_BG: Color32 = Color32::from_rgb(0x15, 0x15, 0x18);
const STRIP_BG: Color32 = Color32::from_rgb(0x21, 0x21, 0x25);
const STRIP_SEL: Color32 = Color32::from_rgb(0x2d, 0x2e, 0x35);
const LANE_BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x17);
const LANE_SEL: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x20);
const HEADER_BG: Color32 = Color32::from_rgb(0x22, 0x22, 0x26);
const HEADER_SEL: Color32 = Color32::from_rgb(0x32, 0x33, 0x3a);
const RULER_BG: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x20);
const LCD_BG: Color32 = Color32::from_rgb(0x0f, 0x0f, 0x12);
const SLOT_EMPTY: Color32 = Color32::from_rgb(0x18, 0x18, 0x1b);
const HAIRLINE: Color32 = Color32::from_rgb(0x11, 0x11, 0x14);
const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xeb);
const TEXT_DIM: Color32 = Color32::from_rgb(0x94, 0x94, 0x9e);

const ACCENT_BLUE: Color32 = Color32::from_rgb(0x2d, 0x7a, 0xff);
const PLUGIN_BLUE: Color32 = Color32::from_rgb(0x2d, 0x72, 0xe0);
const PLAY_GREEN: Color32 = Color32::from_rgb(0x32, 0xc4, 0x56);
const READ_GREEN: Color32 = Color32::from_rgb(0x4e, 0xd4, 0x66);
const REC_RED: Color32 = Color32::from_rgb(0xe8, 0x36, 0x30);
const CYCLE_AMBER: Color32 = Color32::from_rgb(0xe8, 0xa6, 0x2a);
const SOLO_YELLOW: Color32 = Color32::from_rgb(0xf0, 0xc4, 0x24);
const MUTE_BLUE: Color32 = Color32::from_rgb(0x36, 0x8a, 0xf2);
const COUNT_PURPLE: Color32 = Color32::from_rgb(0x9b, 0x3f, 0xd6);
const OUT_PURPLE: Color32 = Color32::from_rgb(0x8e, 0x44, 0xd8);
const MASTER_PINK: Color32 = Color32::from_rgb(0xc0, 0x42, 0xa8);

const ACCENT_MUSE: Color32 = Color32::from_rgb(0x9b, 0x5d, 0xe5);
const ACCENT_ARIEL: Color32 = Color32::from_rgb(0x00, 0xc2, 0xa8);
const ACCENT_RACK: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x0b);

fn apply_noir_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = LANE_BG;
    v.override_text_color = Some(TEXT);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.inactive.bg_fill = Color32::from_rgb(0x32, 0x32, 0x38);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x32, 0x32, 0x38);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x40, 0x40, 0x48);
    v.widgets.active.weak_bg_fill = ACCENT_BLUE;
    v.selection.bg_fill = ACCENT_BLUE;
    let r = CornerRadius::same(4);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, HAIRLINE);
    v.window_stroke = Stroke::new(1.0, Color32::from_rgb(0x38, 0x38, 0x40));
    extras::install_geist_fonts(ctx);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = vec2(5.0, 4.0);
        s.spacing.button_padding = vec2(6.0, 2.0);
        s.text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(12.0));
        s.text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(11.5));
    });
}

// ---------------------------------------------------------------- Model
#[derive(Clone, Copy, PartialEq, Eq)]
enum TrackKind {
    Instrument,
    Drummer,
    Audio,
}

#[derive(Clone)]
struct MidiNote {
    start_beat: f32,
    len_beats: f32,
    pitch: u8,
    vel: u8,
}

#[derive(Clone)]
struct Track {
    name: String,
    kind: TrackKind,
    color: Color32,
    instrument: String,
    fx: Vec<String>,
    send: Option<String>,
    mute: bool,
    solo: bool,
    rec: bool,
    volume_db: f32,
    pan: f32,
    notes: Vec<MidiNote>,
    pcm: Option<Arc<Vec<Frame>>>,
    peaks: Vec<f32>,
    meter: f32,
    synth_phase: f32,
    synth_env: f32,
}

impl Track {
    fn from_audio(name: &str, color: Color32, fx: Vec<&str>, pcm: Arc<Vec<Frame>>) -> Self {
        let peaks = compute_peaks(&pcm, 160);
        Self {
            name: name.into(),
            kind: TrackKind::Audio,
            color,
            instrument: "Input 1".into(),
            fx: fx.into_iter().map(String::from).collect(),
            send: Some("Bus 3".into()),
            mute: false,
            solo: false,
            rec: false,
            volume_db: -3.5,
            pan: 0.0,
            notes: Vec::new(),
            pcm: Some(pcm),
            peaks,
            meter: 0.0,
            synth_phase: 0.0,
            synth_env: 0.0,
        }
    }
}

fn compute_peaks(pcm: &[Frame], bins: usize) -> Vec<f32> {
    if pcm.is_empty() || bins == 0 {
        return vec![0.15; bins];
    }
    let step = (pcm.len() / bins).max(1);
    (0..bins)
        .map(|i| {
            let start = i * step;
            let end = (start + step).min(pcm.len());
            let mut peak = 0.0f32;
            for f in &pcm[start..end] {
                let l = (f[0] as f32 / 32768.0).abs();
                let r = (f[1] as f32 / 32768.0).abs();
                peak = peak.max(l).max(r);
            }
            peak.clamp(0.05, 1.0)
        })
        .collect()
}

fn synth_demo_bass(frames: usize) -> Arc<Vec<Frame>> {
    let mut out = Vec::with_capacity(frames);
    let freqs = [55.0f32, 55.0, 65.41, 73.42, 65.41, 55.0, 49.0, 55.0];
    let step = (RATE as usize / 2).max(1);
    let mut phase = 0.0f32;
    for i in 0..frames {
        let f = freqs[(i / step) % freqs.len()];
        let env = 1.0 - ((i % step) as f32 / step as f32) * 0.65;
        phase = (phase + f / RATE as f32) % 1.0;
        let saw = (phase * 2.0 - 1.0) * 0.45 + (phase * std::f32::consts::TAU).sin() * 0.55;
        let s = (saw * env * 14000.0) as i16;
        out.push([s, s]);
    }
    Arc::new(out)
}

fn demo_notes(root: u8, pattern: &[u8]) -> Vec<MidiNote> {
    let mut notes = Vec::new();
    for bar in 0..12 {
        for (i, &semi) in pattern.iter().enumerate() {
            let beat = bar as f32 * 4.0 + i as f32 * (4.0 / pattern.len() as f32);
            notes.push(MidiNote {
                start_beat: beat,
                len_beats: 0.45,
                pitch: root.saturating_add(semi),
                vel: 86 + ((i * 11) % 36) as u8,
            });
        }
    }
    notes
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BottomView {
    Mixer,
    PianoRoll,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MuseTab {
    CreateSong,
    Stems,
    StudioChat,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArielPreset {
    Soprano,
    Alto,
    Tenor,
    Bass,
    NoirChoir,
    Whisper,
}

impl ArielPreset {
    const ALL: [Self; 6] = [
        Self::Soprano,
        Self::Alto,
        Self::Tenor,
        Self::Bass,
        Self::NoirChoir,
        Self::Whisper,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Soprano => "Soprano",
            Self::Alto => "Alto",
            Self::Tenor => "Tenor",
            Self::Bass => "Bass",
            Self::NoirChoir => "Noir Choir",
            Self::Whisper => "Whisper",
        }
    }
    fn apply(self, p: &mut VoiceParams) {
        match self {
            Self::Soprano => { p.f0 = 330.0; p.formant_scale = 1.24; p.vibrato_depth = 6.0; p.aspiration = 0.08; p.effort = 0.6; }
            Self::Alto => { p.f0 = 246.9; p.formant_scale = 1.12; p.vibrato_depth = 4.5; p.aspiration = 0.06; p.effort = 0.55; }
            Self::Tenor => { p.f0 = 164.8; p.formant_scale = 0.98; p.vibrato_depth = 4.0; p.aspiration = 0.05; p.effort = 0.6; }
            Self::Bass => { p.f0 = 98.0; p.formant_scale = 0.84; p.vibrato_depth = 2.5; p.aspiration = 0.04; p.effort = 0.7; }
            Self::NoirChoir => { p.f0 = 220.0; p.formant_scale = 1.08; p.vibrato_depth = 8.5; p.aspiration = 0.22; p.effort = 0.45; }
            Self::Whisper => { p.f0 = 180.0; p.formant_scale = 1.05; p.vibrato_depth = 0.0; p.aspiration = 0.85; p.effort = 0.15; }
        }
    }
}

struct StudioApp {
    themed: bool,
    tracks: Vec<Track>,
    selected: usize,
    playing: bool,
    recording: bool,
    cycle: bool,
    metronome: bool,
    count_in: bool,
    tempo: f32,
    playhead_beats: f32,
    play_frame: usize,
    master_db: f32,
    stereo_out_db: f32,
    show_library: bool,
    show_inspector: bool,
    show_bottom: bool,
    show_events: bool,
    bottom: BottomView,
    library_search: String,
    status: String,
    typing: extras::MusicalTypingState,
    sampler: extras::QuickSamplerState,
    undo_stack: Vec<Vec<Track>>,

    // Muse AI Studio
    show_muse: bool,
    muse_tab: MuseTab,
    muse_prompt: String,
    muse_lyrics: String,
    muse_key: String,
    muse_bars: u32,
    muse_chat_input: String,
    muse_chat_log: Vec<String>,
    muse_nonce: u64,

    // Ariel Vocal Synth
    show_ariel: bool,
    ariel_preset: ArielPreset,
    ariel_lyrics: String,
    ariel_params: VoiceParams,

    // EuropaRack Modular
    show_rack: bool,
    rack_patch: FactoryPatch,
    rack_graph: PatchGraph,
    rack_dark_room: bool,
    rack_bars: u32,
    rack_root: u8,

    #[cfg(target_os = "hermit")]
    sound: Option<sex_app::sound::Sound>,
}

impl StudioApp {
    fn new() -> Self {
        let mut ariel_params = VoiceParams::default();
        ArielPreset::NoirChoir.apply(&mut ariel_params);

        let vocal_pcm = vocal::synthesize("midnight neon dreams in moan studio", &ariel_params)
            .map(|(f, _)| f)
            .unwrap_or_else(|_| synth_demo_bass(RATE as usize * 4));

        let tracks = vec![
            Track {
                name: "Classic Electric Piano".into(),
                kind: TrackKind::Instrument,
                color: Color32::from_rgb(0x3a, 0xa8, 0x48),
                instrument: "E-Piano".into(),
                fx: vec!["Tape Delay".into(), "Channel EQ".into()],
                send: Some("Bus 1".into()),
                mute: false,
                solo: false,
                rec: false,
                volume_db: -3.2,
                pan: -0.1,
                notes: demo_notes(60, &[0, 4, 7, 11, 12, 11, 7, 4]),
                pcm: None,
                peaks: Vec::new(),
                meter: 0.42,
                synth_phase: 0.0,
                synth_env: 0.0,
            },
            Track {
                name: "Steinway Grand Piano".into(),
                kind: TrackKind::Instrument,
                color: Color32::from_rgb(0x36, 0x9c, 0x44),
                instrument: "Grand".into(),
                fx: vec!["Channel EQ".into(), "Compressor".into()],
                send: Some("Bus 3".into()),
                mute: false,
                solo: false,
                rec: false,
                volume_db: -2.7,
                pan: 0.15,
                notes: demo_notes(48, &[0, 7, 12, 16]),
                pcm: None,
                peaks: Vec::new(),
                meter: 0.36,
                synth_phase: 0.0,
                synth_env: 0.0,
            },
            Track {
                name: "Analog Pad".into(),
                kind: TrackKind::Instrument,
                color: Color32::from_rgb(0x32, 0x94, 0x4c),
                instrument: "Alchemy".into(),
                fx: vec!["Chorus".into(), "Space D".into()],
                send: Some("Bus 3".into()),
                mute: false,
                solo: false,
                rec: false,
                volume_db: -5.6,
                pan: 0.0,
                notes: demo_notes(55, &[0, 5, 7, 12]),
                pcm: None,
                peaks: Vec::new(),
                meter: 0.28,
                synth_phase: 0.0,
                synth_env: 0.0,
            },
            Track {
                name: "Drummer".into(),
                kind: TrackKind::Drummer,
                color: Color32::from_rgb(0xd4, 0x9e, 0x24),
                instrument: "Drum Kit".into(),
                fx: vec!["Compressor".into()],
                send: Some("Bus 3".into()),
                mute: false,
                solo: false,
                rec: false,
                volume_db: -4.0,
                pan: 0.0,
                notes: demo_notes(36, &[0, 6, 2, 6, 0, 6, 2, 10]),
                pcm: None,
                peaks: Vec::new(),
                meter: 0.55,
                synth_phase: 0.0,
                synth_env: 0.0,
            },
            Track::from_audio(
                "Electric Bass",
                Color32::from_rgb(0x4a, 0x76, 0xd8),
                vec!["Channel EQ"],
                synth_demo_bass(RATE as usize * 8),
            ),
            Track::from_audio(
                "Lead Vocal",
                Color32::from_rgb(0x88, 0x52, 0xd4),
                vec!["Channel EQ", "Compressor", "Space D"],
                vocal_pcm,
            ),
        ];

        let rack_patch = FactoryPatch::EuclideanAcidBassline;
        let rack_graph = rack_patch.build().unwrap_or_else(|_| PatchGraph::new());

        Self {
            themed: false,
            tracks,
            selected: 0,
            playing: false,
            recording: false,
            cycle: false,
            metronome: false,
            count_in: true,
            tempo: 120.0,
            playhead_beats: 16.0,
            play_frame: 0,
            master_db: 0.0,
            stereo_out_db: -1.9,
            show_library: true,
            show_inspector: true,
            show_bottom: true,
            show_events: false,
            bottom: BottomView::Mixer,
            library_search: String::new(),
            status: "Press K for Musical Typing • Q Quick Sampler • Space Play • Shift+M Muse".into(),
            typing: extras::MusicalTypingState::default(),
            sampler: extras::QuickSamplerState::default(),
            undo_stack: Vec::new(),
            show_muse: false,
            muse_tab: MuseTab::CreateSong,
            muse_prompt: "dark noir synthwave, analog bass, shimmering pads, crisp drums".into(),
            muse_lyrics: "[00:00.00] Midnight neon glow\n[00:04.00] Moan Studio in flow".into(),
            muse_key: "C minor".into(),
            muse_bars: 4,
            muse_chat_input: String::new(),
            muse_chat_log: vec![
                "Muse Studio ready (Tier 0 pure-Rust zero-RAM engine).".into(),
                "Try: \"create a 124 bpm dark techno track\"".into(),
            ],
            muse_nonce: 1,
            show_ariel: false,
            ariel_preset: ArielPreset::NoirChoir,
            ariel_lyrics: "midnight neon dreams in moan studio".into(),
            ariel_params,
            show_rack: false,
            rack_patch,
            rack_graph,
            rack_dark_room: true,
            rack_bars: 4,
            rack_root: 48,
            #[cfg(target_os = "hermit")]
            sound: sex_app::sound::Sound::open(),
        }
    }

    fn make_nonce(&mut self) -> [u8; 16] {
        self.muse_nonce = self.muse_nonce.wrapping_add(1);
        let mut n = [0u8; 16];
        n[0..8].copy_from_slice(&self.muse_nonce.to_le_bytes());
        n[8..16].copy_from_slice(&0x4D4F_414E_5345_584Fu64.to_le_bytes());
        n
    }

    fn bars_to_frames(&self, bars: u32) -> usize {
        let secs = (bars as f64 * 4.0) * 60.0 / (self.tempo.max(40.0) as f64);
        ((secs * RATE as f64) as usize).clamp(RATE as usize, RATE as usize * 30)
    }

    fn push_undo(&mut self) {
        self.undo_stack.push(self.tracks.clone());
        if self.undo_stack.len() > 24 {
            self.undo_stack.remove(0);
        }
    }

    fn record_live_note(&mut self, pitch: u8, vel: u8) {
        if self.recording {
            if let Some(t) = self.tracks.get_mut(self.selected) {
                t.notes.push(MidiNote {
                    start_beat: (self.playhead_beats * 4.0).round() / 4.0,
                    len_beats: 0.5,
                    pitch,
                    vel,
                });
            }
        }
    }

    fn handle_hotkeys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (mods, pressed_keys, released_keys) = ctx.input(|i| {
            let mut p = Vec::new();
            let mut r = Vec::new();
            for ev in &i.events {
                if let egui::Event::Key { key, pressed, repeat, .. } = ev {
                    if *pressed && !*repeat {
                        p.push(*key);
                    } else if !*pressed {
                        r.push(*key);
                    }
                }
            }
            (i.modifiers, p, r)
        });

        let cmd = mods.command || mods.ctrl;

        // If Musical Typing is open, route piano keys A..; + Z/X octave + C/V velocity + Tab sustain
        if self.typing.open {
            let root = self.typing.root_midi();
            for &k in &released_keys {
                if let Some(&(_, semi, _, _)) = extras::TYPING_KEYS.iter().find(|x| x.0 == k) {
                    self.typing.release(root + semi);
                }
                if k == Key::Tab {
                    self.typing.sustain = false;
                }
            }
            for &k in &pressed_keys {
                if k == Key::Escape || (cmd && k == Key::K) {
                    self.typing.open = false;
                    self.status = "Musical Typing closed".into();
                    continue;
                }
                if k == Key::Space {
                    self.playing = !self.playing;
                    continue;
                }
                if k == Key::Tab {
                    self.typing.sustain = true;
                    continue;
                }
                if k == Key::Z {
                    self.typing.octave = (self.typing.octave - 1).clamp(1, 6);
                    continue;
                }
                if k == Key::X {
                    self.typing.octave = (self.typing.octave + 1).clamp(1, 6);
                    continue;
                }
                if k == Key::C {
                    self.typing.velocity = self.typing.velocity.saturating_sub(15).max(15);
                    continue;
                }
                if k == Key::V {
                    self.typing.velocity = self.typing.velocity.saturating_add(15).min(127);
                    continue;
                }
                if let Some(&(_, semi, _, _)) = extras::TYPING_KEYS.iter().find(|x| x.0 == k) {
                    let pitch = root + semi;
                    self.typing.trigger(pitch);
                    let vel = self.typing.velocity;
                    self.record_live_note(pitch, vel);
                    self.status = format!("Playing {} (vel {vel})", extras::pitch_name(pitch));
                }
            }
            return;
        }

        for &k in &pressed_keys {
            if cmd {
                match k {
                    Key::K => {
                        self.typing.open = !self.typing.open;
                        self.status = "Musical Typing (A–; plays notes, Z/X octave, Esc closes)".into();
                    }
                    Key::D => {
                        if let Some(t) = self.tracks.get(self.selected).cloned() {
                            self.push_undo();
                            let mut dup = t;
                            dup.name = format!("{} Copy", dup.name);
                            self.tracks.insert(self.selected + 1, dup);
                            self.selected += 1;
                            self.status = "Duplicated selected track".into();
                        }
                    }
                    Key::Z => {
                        if let Some(prev) = self.undo_stack.pop() {
                            self.tracks = prev;
                            self.selected = self.selected.min(self.tracks.len().saturating_sub(1));
                            self.status = "Undo".into();
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if mods.shift {
                match k {
                    Key::M => self.show_muse = !self.show_muse,
                    Key::A => self.show_ariel = !self.show_ariel,
                    Key::R => self.show_rack = !self.show_rack,
                    Key::ArrowUp => self.tempo = (self.tempo + 1.0).clamp(40.0, 240.0),
                    Key::ArrowDown => self.tempo = (self.tempo - 1.0).clamp(40.0, 240.0),
                    _ => {}
                }
                continue;
            }
            match k {
                Key::Space => self.playing = !self.playing,
                Key::Enter => {
                    self.playhead_beats = 0.0;
                    self.play_frame = 0;
                }
                Key::K => {
                    self.typing.open = true;
                    self.status = "Musical Typing open (A–; plays notes, Z/X octave, Esc closes)".into();
                }
                Key::Q => self.sampler.open = !self.sampler.open,
                Key::Y => self.show_library = !self.show_library,
                Key::I => self.show_inspector = !self.show_inspector,
                Key::X => {
                    if self.show_bottom && self.bottom == BottomView::Mixer {
                        self.show_bottom = false;
                    } else {
                        self.show_bottom = true;
                        self.bottom = BottomView::Mixer;
                    }
                }
                Key::P | Key::E => {
                    if self.show_bottom && self.bottom == BottomView::PianoRoll {
                        self.show_bottom = false;
                    } else {
                        self.show_bottom = true;
                        self.bottom = BottomView::PianoRoll;
                    }
                }
                Key::D => self.show_events = !self.show_events,
                Key::C => self.cycle = !self.cycle,
                Key::R => {
                    self.recording = !self.recording;
                    if self.recording {
                        self.playing = true;
                    }
                }
                Key::M => {
                    if let Some(t) = self.tracks.get_mut(self.selected) {
                        t.mute = !t.mute;
                    }
                }
                Key::S => {
                    if let Some(t) = self.tracks.get_mut(self.selected) {
                        t.solo = !t.solo;
                    }
                }
                Key::ArrowUp => {
                    if self.selected > 0 {
                        self.selected -= 1;
                    }
                }
                Key::ArrowDown => {
                    if self.selected + 1 < self.tracks.len() {
                        self.selected += 1;
                    }
                }
                _ => {}
            }
        }
    }

    fn pump_audio(&mut self, ctx: &egui::Context) {
        let live_typing = self.typing.has_sound();
        if !self.playing && !live_typing {
            return;
        }
        let dt = ctx.input(|i| i.stable_dt).clamp(0.008, 0.05);
        if self.playing {
            self.playhead_beats += dt * (self.tempo / 60.0);
            let loop_end = if self.cycle { 16.0 } else { 48.0 };
            if self.playhead_beats >= loop_end {
                self.playhead_beats = 0.0;
                self.play_frame = 0;
            }
        }
        let any_solo = self.tracks.iter().any(|t| t.solo);
        #[cfg(target_os = "hermit")]
        {
            if let Some(sound) = &mut self.sound {
                // Keep a generous 6144-frame (~128 ms) cushion in Bell's ring so UI repaints
                // never underrun or click.
                let room = sound.room();
                let want = if room > 4096 { room.min(6144) } else { 0 };
                if want > 0 {
                    let mut mix = vec![[0i32; 2]; want];
                    let rate_f = RATE as f32;
                    let beats_per_frame = self.tempo / (60.0 * rate_f);
                    let tau = std::f32::consts::TAU;

                    if self.playing {
                        for t in &mut self.tracks {
                            let active = !t.mute && (!any_solo || t.solo);
                            if !active {
                                t.meter *= 0.85;
                                t.synth_env *= 0.9;
                                continue;
                            }
                            let gain = 10.0f32.powf(t.volume_db / 20.0);
                            let pan_l = ((1.0 - t.pan) * 0.5).clamp(0.0, 1.0).sqrt();
                            let pan_r = ((1.0 + t.pan) * 0.5).clamp(0.0, 1.0).sqrt();
                            let mut tr_peak = 0.0f32;
                            if let Some(pcm) = &t.pcm {
                                if !pcm.is_empty() {
                                    let len = pcm.len();
                                    for i in 0..want {
                                        let f = pcm[(self.play_frame + i) % len];
                                        let l = (f[0] as f32 * gain * pan_l) as i32;
                                        let r = (f[1] as f32 * gain * pan_r) as i32;
                                        mix[i][0] += l;
                                        mix[i][1] += r;
                                        tr_peak = tr_peak.max((l as f32 / 32768.0).abs());
                                    }
                                }
                            } else {
                                // Click-free continuous-phase poly-harmonic synth with smooth 10ms envelope
                                let start_beat = self.play_frame as f32 * beats_per_frame;
                                for i in 0..want {
                                    let beat = (start_beat + i as f32 * beats_per_frame) % 48.0;
                                    let active_note = t.notes.iter().find(|n| {
                                        beat >= n.start_beat && beat < n.start_beat + n.len_beats
                                    });
                                    let target_env = if active_note.is_some() { 1.0 } else { 0.0 };
                                    // Smooth ~10 ms attack / ~25 ms release prevents all clicks
                                    let coeff = if target_env > t.synth_env { 0.0022 } else { 0.0009 };
                                    t.synth_env += (target_env - t.synth_env) * coeff;
                                    if t.synth_env > 0.001 {
                                        if let Some(n) = active_note {
                                            let freq = 440.0 * 2.0f32.powf((n.pitch as f32 - 69.0) / 12.0);
                                            t.synth_phase = (t.synth_phase + freq / rate_f) % 1.0;
                                        }
                                        let ph = t.synth_phase * tau;
                                        let sig = ph.sin() * 0.72 + (ph * 2.0).sin() * 0.20 + (ph * 3.0).sin() * 0.08;
                                        let amp = sig * t.synth_env * 3800.0 * gain;
                                        let l = (amp * pan_l) as i32;
                                        let r = (amp * pan_r) as i32;
                                        mix[i][0] += l;
                                        mix[i][1] += r;
                                        tr_peak = tr_peak.max((l as f32 / 32768.0).abs());
                                    }
                                }
                            }
                            t.meter = (t.meter * 0.7 + tr_peak * 0.3).clamp(0.0, 1.0);
                        }
                    }

                    // Mix live Musical Typing voices right into the same click-free Bell stream
                    if live_typing {
                        self.typing.mix_into(&mut mix, rate_f);
                    }

                    let master_gain = 10.0f32.powf((self.master_db + self.stereo_out_db) / 20.0);
                    let frames: Vec<Frame> = mix
                        .into_iter()
                        .map(|[l, r]| {
                            // Soft-knee limiter prevents harsh clipping
                            let soft = |v: i32| -> i16 {
                                let x = (v as f32 * master_gain) / 32767.0;
                                let y = x / (1.0 + x.abs() * 0.35);
                                (y.clamp(-0.98, 0.98) * 32767.0) as i16
                            };
                            [soft(l), soft(r)]
                        })
                        .collect();
                    let written = sound.write(&frames);
                    if self.playing {
                        self.play_frame = self.play_frame.wrapping_add(written);
                    }
                }
            }
        }
        #[cfg(not(target_os = "hermit"))]
        {
            for (idx, t) in self.tracks.iter_mut().enumerate() {
                let active = !t.mute && (!any_solo || t.solo);
                let phase = (self.playhead_beats * 3.1 + idx as f32 * 1.3).sin().abs();
                t.meter = if self.playing && active { (0.25 + phase * 0.6).min(0.95) } else { 0.0 };
            }
        }
        ctx.request_repaint();
    }

    fn draw_menu_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("Moan Studio").strong().color(TEXT).size(11.5));
            ui.separator();
            ui.menu_button("File", |ui| {
                if ui.button("New Project").clicked() {
                    self.push_undo();
                    *self = StudioApp::new();
                    self.themed = true;
                    ui.close();
                }
                if ui.button("Rewind to Bar 1  (Enter)").clicked() {
                    self.playhead_beats = 0.0;
                    self.play_frame = 0;
                    ui.close();
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.button("Undo  (Ctrl+Z)").clicked() {
                    if let Some(prev) = self.undo_stack.pop() {
                        self.tracks = prev;
                    }
                    ui.close();
                }
                if ui.button("Duplicate Track  (Ctrl+D)").clicked() {
                    if let Some(t) = self.tracks.get(self.selected).cloned() {
                        self.push_undo();
                        let mut dup = t;
                        dup.name = format!("{} Copy", dup.name);
                        self.tracks.insert(self.selected + 1, dup);
                        self.selected += 1;
                    }
                    ui.close();
                }
            });
            ui.menu_button("Track", |ui| {
                if ui.button("New Instrument Track").clicked() {
                    self.push_undo();
                    let idx = self.tracks.len() + 1;
                    self.tracks.push(Track {
                        name: format!("Synth {idx}"),
                        kind: TrackKind::Instrument,
                        color: ACCENT_BLUE,
                        instrument: "Synth".into(),
                        fx: vec!["Channel EQ".into()],
                        send: Some("Bus 1".into()),
                        mute: false,
                        solo: false,
                        rec: false,
                        volume_db: -3.0,
                        pan: 0.0,
                        notes: demo_notes(60, &[0, 3, 7, 10]),
                        pcm: None,
                        peaks: Vec::new(),
                        meter: 0.0,
                        synth_phase: 0.0,
                        synth_env: 0.0,
                    });
                    self.selected = self.tracks.len() - 1;
                    ui.close();
                }
                if ui.button("Mute Track  (M)").clicked() {
                    if let Some(t) = self.tracks.get_mut(self.selected) { t.mute = !t.mute; }
                    ui.close();
                }
                if ui.button("Solo Track  (S)").clicked() {
                    if let Some(t) = self.tracks.get_mut(self.selected) { t.solo = !t.solo; }
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.show_library, "Library  (Y)");
                ui.checkbox(&mut self.show_inspector, "Inspector  (I)");
                ui.checkbox(&mut self.show_events, "Event List  (D)");
                if ui.button("Mixer  (X)").clicked() {
                    self.show_bottom = true;
                    self.bottom = BottomView::Mixer;
                    ui.close();
                }
                if ui.button("Piano Roll  (P)").clicked() {
                    self.show_bottom = true;
                    self.bottom = BottomView::PianoRoll;
                    ui.close();
                }
            });
            ui.menu_button("Window", |ui| {
                ui.checkbox(&mut self.typing.open, "Musical Typing  (K)");
                ui.checkbox(&mut self.sampler.open, "Quick Sampler  (Q)");
                ui.separator();
                ui.checkbox(&mut self.show_muse, "✦ Muse AI Studio  (Shift+M)");
                ui.checkbox(&mut self.show_ariel, "◉ Ariel Vocal Synth  (Shift+A)");
                ui.checkbox(&mut self.show_rack, "⚡ EuropaRack Modular  (Shift+R)");
            });
        });
    }

    fn draw_event_list(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("Event List  (D)").strong().color(TEXT));
        });
        ui.separator();
        if let Some(t) = self.tracks.get_mut(self.selected) {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{} • {} events", t.name, t.notes.len())).color(TEXT_DIM).size(11.0));
            });
            ui.add_space(4.0);
            let mut remove_idx = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (idx, n) in t.notes.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_space(6.0);
                        let bar = (n.start_beat / 4.0).floor() as u32 + 1;
                        let beat = (n.start_beat % 4.0).floor() as u32 + 1;
                        ui.label(RichText::new(format!("{bar:02}.{beat}")).monospace().size(10.5).color(TEXT_DIM));
                        ui.label(RichText::new(extras::pitch_name(n.pitch)).monospace().size(10.5).color(ACCENT_ARIEL));
                        ui.label(RichText::new(format!("v{}", n.vel)).monospace().size(10.0).color(TEXT_DIM));
                        if ui.small_button("×").clicked() {
                            remove_idx = Some(idx);
                        }
                    });
                }
            });
            if let Some(idx) = remove_idx {
                t.notes.remove(idx);
            }
        }
    }

    fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if !self.themed {
            apply_noir_theme(&ctx);
            self.themed = true;
        }
        self.handle_hotkeys(&ctx);
        self.pump_audio(&ctx);

        // 0. macOS-style Top Menu Bar
        egui::Panel::top("menu_bar")
            .exact_size(24.0)
            .frame(egui::Frame::new().fill(Color32::from_rgb(0x1b, 0x1b, 0x1e)))
            .show(ui, |ui| self.draw_menu_bar(ui));

        // 1. Top Control Bar
        egui::Panel::top("control_bar")
            .exact_size(54.0)
            .frame(egui::Frame::new().fill(BAR_BG))
            .show(ui, |ui| self.draw_control_bar(ui));

        // 2. Left Library Browser
        if self.show_library {
            egui::Panel::left("library")
                .exact_size(220.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(PANEL_DARK))
                .show(ui, |ui| self.draw_library(ui));
        }

        // 3. Left Inspector (Region Defaults + Dual Channel Strips)
        if self.show_inspector {
            egui::Panel::left("inspector")
                .exact_size(196.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(INSPECTOR_BG))
                .show(ui, |ui| self.draw_inspector(ui));
        }

        // 3b. Right Event List Sidebar (D)
        if self.show_events {
            egui::Panel::right("event_list")
                .exact_size(210.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(PANEL_DARK))
                .show(ui, |ui| self.draw_event_list(ui));
        }

        // 4. Bottom Mixer or Piano Roll
        if self.show_bottom {
            egui::Panel::bottom("bottom_pane")
                .exact_size(340.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(MIXER_BG))
                .show(ui, |ui| match self.bottom {
                    BottomView::Mixer => self.draw_mixer(ui),
                    BottomView::PianoRoll => self.draw_piano_roll(ui),
                });
        }

        // 5. Center Arrangement / Tracks Area
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(LANE_BG))
            .show(ui, |ui| self.draw_tracks_area(ui));

        // 6. Moan Studio Power Suite + Musical Typing + Quick Sampler Floating Windows
        self.draw_muse_window(&ctx);
        self.draw_ariel_window(&ctx);
        self.draw_rack_window(&ctx);

        let mut recorded = Vec::new();
        extras::draw_musical_typing_window(&ctx, &mut self.typing, &mut |pitch, vel| {
            recorded.push((pitch, vel));
        });
        for (pitch, vel) in recorded {
            self.record_live_note(pitch, vel);
        }

        let sel_peaks = self
            .tracks
            .get(self.selected)
            .map(|t| t.peaks.clone())
            .unwrap_or_default();
        extras::draw_quick_sampler_window(&ctx, &mut self.sampler, &sel_peaks);
    }

    fn draw_control_bar(&mut self, ui: &mut Ui) {
        let bar = ui.max_rect();
        let g = ui.painter();
        g.line_segment(
            [pos2(bar.left(), bar.bottom() - 0.5), pos2(bar.right(), bar.bottom() - 0.5)],
            Stroke::new(1.0, HAIRLINE),
        );

        let cy = bar.center().y;
        // Left view toggles
        let mut x = bar.left() + 10.0;
        let views = [
            ("Lib", self.show_library, 0),
            ("Insp", self.show_inspector, 1),
            ("Mix", self.show_bottom && self.bottom == BottomView::Mixer, 2),
            ("Keys", self.show_bottom && self.bottom == BottomView::PianoRoll, 3),
        ];
        for (lbl, on, idx) in views {
            let r = Rect::from_min_size(pos2(x, cy - 12.0), vec2(38.0, 24.0));
            let resp = ui.interact(r, egui::Id::unique(("cb_view", idx)), Sense::click());
            let bg = if on {
                ACCENT_BLUE
            } else if resp.hovered() {
                Color32::from_rgb(0x36, 0x36, 0x3e)
            } else {
                Color32::from_rgb(0x2c, 0x2c, 0x32)
            };
            g.rect(r, CornerRadius::same(4), bg, Stroke::new(1.0, HAIRLINE), StrokeKind::Inside);
            g.text(r.center(), Align2::CENTER_CENTER, lbl, FontId::proportional(11.0), if on { Color32::WHITE } else { TEXT });
            if resp.clicked() {
                match idx {
                    0 => self.show_library = !self.show_library,
                    1 => self.show_inspector = !self.show_inspector,
                    2 => {
                        if self.show_bottom && self.bottom == BottomView::Mixer {
                            self.show_bottom = false;
                        } else {
                            self.show_bottom = true;
                            self.bottom = BottomView::Mixer;
                        }
                    }
                    _ => {
                        if self.show_bottom && self.bottom == BottomView::PianoRoll {
                            self.show_bottom = false;
                        } else {
                            self.show_bottom = true;
                            self.bottom = BottomView::PianoRoll;
                        }
                    }
                }
            }
            x += 42.0;
        }

        // Transport controls left of LCD
        let lcd_w = 340.0f32.min((bar.width() - 560.0).max(220.0));
        let lcd_rect = Rect::from_center_size(pos2(bar.center().x - 20.0, cy), vec2(lcd_w, 38.0));

        let mut tx = lcd_rect.left() - 178.0;
        let tbtns = [
            ("|<", false, TEXT, 0),
            ("[]", false, TEXT, 1),
            (">", self.playing, PLAY_GREEN, 2),
            ("●", self.recording, REC_RED, 3),
            ("↻", self.cycle, CYCLE_AMBER, 4),
        ];
        for (sym, on, col, idx) in tbtns {
            let r = Rect::from_min_size(pos2(tx, cy - 13.0), vec2(30.0, 26.0));
            let resp = ui.interact(r, egui::Id::unique(("cb_tr", idx)), Sense::click());
            let bg = if on {
                col.linear_multiply(0.3)
            } else if resp.hovered() {
                Color32::from_rgb(0x36, 0x36, 0x3e)
            } else {
                Color32::from_rgb(0x2a, 0x2a, 0x30)
            };
            g.rect(r, CornerRadius::same(4), bg, Stroke::new(1.0, if on { col } else { HAIRLINE }), StrokeKind::Inside);
            g.text(r.center(), Align2::CENTER_CENTER, sym, FontId::proportional(13.0), if on { col } else { TEXT });
            if resp.clicked() {
                match idx {
                    0 => { self.playhead_beats = 0.0; self.play_frame = 0; }
                    1 => { self.playing = false; self.recording = false; }
                    2 => { self.playing = !self.playing; }
                    3 => { self.recording = !self.recording; self.playing = self.recording; }
                    _ => { self.cycle = !self.cycle; }
                }
            }
            tx += 34.0;
        }

        // Logic Noir Recessed LCD
        g.rect(
            lcd_rect,
            CornerRadius::same(5),
            LCD_BG,
            Stroke::new(1.0, Color32::from_rgb(0x34, 0x34, 0x3c)),
            StrokeKind::Inside,
        );
        let total_beats = self.playhead_beats.max(0.0);
        let bar_num = (total_beats / 4.0).floor() as u32 + 1;
        let beat_num = (total_beats % 4.0).floor() as u32 + 1;
        let div_num = ((total_beats.fract() * 4.0).floor() as u32) + 1;
        let secs = total_beats * 60.0 / self.tempo.max(40.0);
        let mins = (secs / 60.0).floor() as u32;
        let sec_rem = secs % 60.0;

        g.text(
            pos2(lcd_rect.left() + 10.0, lcd_rect.top() + 11.0),
            Align2::LEFT_CENTER,
            format!("01:{mins:02}:{sec_rem:05.2}"),
            FontId::monospace(12.5),
            TEXT,
        );
        g.text(
            pos2(lcd_rect.left() + 10.0, lcd_rect.bottom() - 10.0),
            Align2::LEFT_CENTER,
            format!("{bar_num:03}  {beat_num}  {div_num}  001"),
            FontId::monospace(11.5),
            TEXT_DIM,
        );
        let bpm_r = Rect::from_center_size(
            pos2(lcd_rect.center().x + 12.0, lcd_rect.top() + 11.0),
            vec2(96.0, 20.0),
        );
        let bpm_resp = ui.interact(bpm_r, egui::Id::unique("lcd_bpm_drag"), Sense::drag());
        if bpm_resp.dragged() {
            self.tempo = (self.tempo - bpm_resp.drag_delta().y * 0.25).clamp(40.0, 240.0);
        }
        g.text(
            bpm_r.center(),
            Align2::CENTER_CENTER,
            format!("{:.2} BPM", self.tempo),
            FontId::monospace(12.5),
            if bpm_resp.hovered() || bpm_resp.dragged() { ACCENT_BLUE } else { TEXT },
        );
        g.text(
            pos2(lcd_rect.center().x + 12.0, lcd_rect.bottom() - 10.0),
            Align2::CENTER_CENTER,
            "Keep Tempo",
            FontId::proportional(10.0),
            TEXT_DIM,
        );
        g.text(
            pos2(lcd_rect.right() - 12.0, lcd_rect.top() + 11.0),
            Align2::RIGHT_CENTER,
            "4/4",
            FontId::monospace(12.5),
            TEXT,
        );
        g.text(
            pos2(lcd_rect.right() - 12.0, lcd_rect.bottom() - 10.0),
            Align2::RIGHT_CENTER,
            "48 kHz",
            FontId::proportional(10.0),
            ACCENT_ARIEL,
        );

        // Count-In & Metronome right of LCD
        let c1234 = Rect::from_min_size(pos2(lcd_rect.right() + 10.0, cy - 12.0), vec2(38.0, 24.0));
        let r1234 = ui.interact(c1234, egui::Id::unique("cb_1234"), Sense::click());
        g.rect(
            c1234,
            CornerRadius::same(4),
            if self.count_in { COUNT_PURPLE } else { Color32::from_rgb(0x2c, 0x2c, 0x32) },
            Stroke::new(1.0, HAIRLINE),
            StrokeKind::Inside,
        );
        g.text(c1234.center(), Align2::CENTER_CENTER, "1234", FontId::proportional(10.5), Color32::WHITE);
        if r1234.clicked() { self.count_in = !self.count_in; }

        let met = Rect::from_min_size(pos2(c1234.right() + 6.0, cy - 12.0), vec2(32.0, 24.0));
        let rmet = ui.interact(met, egui::Id::unique("cb_met"), Sense::click());
        g.rect(
            met,
            CornerRadius::same(4),
            if self.metronome { COUNT_PURPLE } else { Color32::from_rgb(0x2c, 0x2c, 0x32) },
            Stroke::new(1.0, HAIRLINE),
            StrokeKind::Inside,
        );
        g.text(met.center(), Align2::CENTER_CENTER, "Click", FontId::proportional(10.0), Color32::WHITE);
        if rmet.clicked() { self.metronome = !self.metronome; }

        // Moan Studio Power Suite Pills (Muse, Ariel, Rack) on the right
        let pills = [
            ("Muse", ACCENT_MUSE, self.show_muse, 0),
            ("Ariel", ACCENT_ARIEL, self.show_ariel, 1),
            ("Rack", ACCENT_RACK, self.show_rack, 2),
        ];
        let mut px = bar.right() - 166.0;
        for (lbl, accent, active, idx) in pills {
            let r = Rect::from_min_size(pos2(px, cy - 12.0), vec2(48.0, 24.0));
            let resp = ui.interact(r, egui::Id::unique(("suite_pill", idx)), Sense::click());
            let bg = if active {
                accent
            } else if resp.hovered() {
                Color32::from_rgb(0x36, 0x36, 0x3e)
            } else {
                Color32::from_rgb(0x28, 0x28, 0x2e)
            };
            let fg = if active { Color32::from_rgb(0x0f, 0x0f, 0x12) } else { TEXT };
            g.rect(r, CornerRadius::same(4), bg, Stroke::new(1.0, accent.linear_multiply(0.75)), StrokeKind::Inside);
            g.text(r.center(), Align2::CENTER_CENTER, lbl, FontId::proportional(11.5), fg);
            if resp.clicked() {
                match idx {
                    0 => self.show_muse = !self.show_muse,
                    1 => self.show_ariel = !self.show_ariel,
                    _ => self.show_rack = !self.show_rack,
                }
            }
            px += 53.0;
        }
    }

    fn draw_library(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("Library").strong().color(TEXT));
            ui.label(RichText::new("All Sounds").color(TEXT_DIM).size(11.0));
        });
        ui.add_space(6.0);

        let sel_name = self
            .tracks
            .get(self.selected)
            .map(|t| t.name.as_str())
            .unwrap_or("Track");
        let sel_col = self
            .tracks
            .get(self.selected)
            .map(|t| t.color)
            .unwrap_or(PLAY_GREEN);

        ui.vertical_centered(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(8), sel_col.linear_multiply(0.45));
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "♪", FontId::proportional(26.0), sel_col);
            ui.add_space(4.0);
            ui.label(RichText::new(sel_name).color(TEXT).size(11.5));
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.library_search)
                    .hint_text("Search Sounds")
                    .desired_width(200.0),
            );
        });
        ui.add_space(4.0);
        ui.separator();

        let categories = [
            "User Patches",
            "Bass",
            "Drum Kit",
            "Electronic Drum Kit",
            "Guitar",
            "Guitar Amp",
            "Keyboards",
            "Orchestral",
            "Percussion",
            "Piano",
            "Quick Sampler",
            "Studio Horns",
            "Studio Strings",
            "Synthesizer",
            "Vintage B3 Organ",
            "Vintage Electric Piano",
            "Ariel Vocal Choir",
            "EuropaRack Modular",
            "Muse AI Stems",
        ];
        let q = self.library_search.to_ascii_lowercase();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for cat in categories {
                if !q.is_empty() && !cat.to_ascii_lowercase().contains(&q) {
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    if ui.selectable_label(false, format!("{cat}  ›")).clicked() {
                        if let Some(t) = self.tracks.get_mut(self.selected) {
                            t.instrument = cat.into();
                            self.status = format!("Loaded patch \"{cat}\" on {}", t.name);
                        }
                    }
                });
            }
        });
    }

    fn draw_inspector(&mut self, ui: &mut Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("▾ Region Defaults").strong().size(11.5));
        });
        ui.separator();
        if let Some(t) = self.tracks.get_mut(self.selected) {
            egui::Grid::new("insp_region_grid")
                .num_columns(2)
                .spacing([8.0, 3.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Mute").color(TEXT_DIM).size(11.0));
                    ui.checkbox(&mut t.mute, "");
                    ui.end_row();
                    ui.label(RichText::new("Solo").color(TEXT_DIM).size(11.0));
                    ui.checkbox(&mut t.solo, "");
                    ui.end_row();
                    ui.label(RichText::new("Gain").color(TEXT_DIM).size(11.0));
                    ui.add(egui::DragValue::new(&mut t.volume_db).range(-60.0..=6.0).speed(0.2).suffix(" dB"));
                    ui.end_row();
                    ui.label(RichText::new("Pan").color(TEXT_DIM).size(11.0));
                    ui.add(egui::DragValue::new(&mut t.pan).range(-1.0..=1.0).speed(0.02));
                    ui.end_row();
                });
        }
        ui.add_space(6.0);
        ui.separator();
        let sel_title = self
            .tracks
            .get(self.selected)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "Track".into());
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new(format!("▾ Track: {sel_title}")).strong().size(11.0));
        });
        ui.add_space(4.0);

        // Dual Channel Strips (Selected Track + Stereo Out)
        let avail = ui.available_rect_before_wrap();
        let strip_w = (avail.width() - 12.0) * 0.5;
        let left_r = Rect::from_min_size(pos2(avail.left() + 4.0, avail.top()), vec2(strip_w, avail.height() - 4.0));
        let right_r = Rect::from_min_size(pos2(left_r.right() + 4.0, avail.top()), vec2(strip_w, avail.height() - 4.0));

        if let Some(t) = self.tracks.get_mut(self.selected) {
            draw_channel_strip(
                ui,
                left_r,
                &t.name,
                t.color,
                &t.fx,
                t.send.as_deref(),
                &mut t.volume_db,
                &mut t.pan,
                &mut t.mute,
                &mut t.solo,
                t.meter,
                true,
                false,
                1000,
            );
        }
        let mut dummy_m = false;
        let mut dummy_s = false;
        let mut dummy_p = 0.0;
        let out_meter = self.tracks.iter().map(|t| t.meter).fold(0.0f32, f32::max);
        draw_channel_strip(
            ui,
            right_r,
            "Stereo Out",
            OUT_PURPLE,
            &[],
            None,
            &mut self.stereo_out_db,
            &mut dummy_p,
            &mut dummy_m,
            &mut dummy_s,
            out_meter,
            false,
            true,
            1001,
        );
    }

    fn draw_tracks_area(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let g = ui.painter();

        // Top mini toolbar
        let tb = Rect::from_min_size(full.min, vec2(full.width(), 28.0));
        g.rect_filled(tb, 0.0, Color32::from_rgb(0x20, 0x20, 0x24));
        g.line_segment([pos2(tb.left(), tb.bottom()), pos2(tb.right(), tb.bottom())], Stroke::new(1.0, HAIRLINE));

        let add_btn = Rect::from_min_size(pos2(tb.left() + 8.0, tb.top() + 4.0), vec2(22.0, 20.0));
        let add_resp = ui.interact(add_btn, egui::Id::unique("add_track_btn"), Sense::click());
        g.rect(add_btn, CornerRadius::same(3), Color32::from_rgb(0x30, 0x30, 0x38), Stroke::new(1.0, HAIRLINE), StrokeKind::Inside);
        g.text(add_btn.center(), Align2::CENTER_CENTER, "+", FontId::proportional(14.0), TEXT);
        if add_resp.clicked() {
            let idx = self.tracks.len() + 1;
            self.tracks.push(Track {
                name: format!("Synth Track {idx}"),
                kind: TrackKind::Instrument,
                color: ACCENT_BLUE,
                instrument: "Synth".into(),
                fx: vec!["Channel EQ".into()],
                send: Some("Bus 1".into()),
                mute: false,
                solo: false,
                rec: false,
                volume_db: -3.0,
                pan: 0.0,
                notes: demo_notes(60, &[0, 3, 7, 10]),
                pcm: None,
                peaks: Vec::new(),
                meter: 0.0,
                synth_phase: 0.0,
                synth_env: 0.0,
            });
            self.selected = self.tracks.len() - 1;
        }
        g.text(
            pos2(tb.left() + 40.0, tb.center().y),
            Align2::LEFT_CENTER,
            &self.status,
            FontId::proportional(11.0),
            TEXT_DIM,
        );
        g.text(
            pos2(tb.right() - 12.0, tb.center().y),
            Align2::RIGHT_CENTER,
            "Snap: Smart   •   Drag: No Overlap",
            FontId::proportional(11.0),
            TEXT_DIM,
        );

        let header_w = 210.0;
        let ruler_h = 22.0;
        let ruler = Rect::from_min_size(pos2(full.left() + header_w, tb.bottom()), vec2(full.width() - header_w, ruler_h));
        g.rect_filled(Rect::from_min_size(pos2(full.left(), tb.bottom()), vec2(header_w, ruler_h)), 0.0, RULER_BG);
        g.rect_filled(ruler, 0.0, RULER_BG);

        let bars = 12usize;
        let bar_w = (ruler.width() / bars as f32).max(40.0);
        for b in 0..bars {
            let bx = ruler.left() + b as f32 * bar_w;
            g.line_segment([pos2(bx, ruler.top()), pos2(bx, full.bottom())], Stroke::new(1.0, Color32::from_rgb(0x22, 0x22, 0x28)));
            g.text(pos2(bx + 4.0, ruler.center().y), Align2::LEFT_CENTER, format!("{}", b + 1), FontId::proportional(10.5), TEXT_DIM);
        }
        if self.cycle {
            let cyc = Rect::from_min_size(pos2(ruler.left(), ruler.top()), vec2(bar_w * 4.0, 6.0));
            g.rect_filled(cyc, 1.0, CYCLE_AMBER);
        }

        // Click ruler to seek playhead
        let ruler_resp = ui.interact(ruler, egui::Id::unique("ruler_seek"), Sense::click_and_drag());
        if ruler_resp.clicked() || ruler_resp.dragged() {
            if let Some(pos) = ruler_resp.interact_pointer_pos() {
                let rel = ((pos.x - ruler.left()) / bar_w).clamp(0.0, bars as f32);
                self.playhead_beats = rel * 4.0;
            }
        }

        // Track rows
        let row_h = 56.0;
        let mut y = ruler.bottom();
        for i in 0..self.tracks.len() {
            let is_sel = i == self.selected;
            let hdr = Rect::from_min_size(pos2(full.left(), y), vec2(header_w, row_h));
            let lane = Rect::from_min_size(pos2(full.left() + header_w, y), vec2(full.width() - header_w, row_h));

            g.rect_filled(hdr, 0.0, if is_sel { HEADER_SEL } else { HEADER_BG });
            g.rect_filled(lane, 0.0, if is_sel { LANE_SEL } else { LANE_BG });
            g.line_segment([pos2(full.left(), y + row_h), pos2(full.right(), y + row_h)], Stroke::new(1.0, HAIRLINE));
            g.line_segment([pos2(hdr.right(), y), pos2(hdr.right(), y + row_h)], Stroke::new(1.0, HAIRLINE));

            let hresp = ui.interact(hdr, egui::Id::unique(("trk_hdr", i)), Sense::click());
            if hresp.clicked() {
                self.selected = i;
            }

            let t = &mut self.tracks[i];
            // Track number + color bar
            g.rect_filled(Rect::from_min_size(pos2(hdr.left(), y), vec2(4.0, row_h)), 0.0, t.color);
            g.text(pos2(hdr.left() + 14.0, hdr.center().y), Align2::CENTER_CENTER, format!("{}", i + 1), FontId::proportional(11.0), TEXT_DIM);

            // Track icon badge
            let icon_r = Rect::from_min_size(pos2(hdr.left() + 26.0, y + 10.0), vec2(26.0, 26.0));
            g.rect_filled(icon_r, CornerRadius::same(4), t.color.linear_multiply(0.35));
            let sym = match t.kind {
                TrackKind::Instrument => "♪",
                TrackKind::Drummer => "≡",
                TrackKind::Audio => "◉",
            };
            g.text(icon_r.center(), Align2::CENTER_CENTER, sym, FontId::proportional(14.0), t.color);

            // Name
            g.text(pos2(icon_r.right() + 8.0, y + 14.0), Align2::LEFT_CENTER, &t.name, FontId::proportional(11.5), TEXT);

            // M / S / R buttons
            let msr_y = y + 32.0;
            let btns = [("M", t.mute, MUTE_BLUE), ("S", t.solo, SOLO_YELLOW), ("R", t.rec, REC_RED)];
            for (bi, (lbl, on, col)) in btns.iter().enumerate() {
                let br = Rect::from_min_size(pos2(icon_r.right() + 8.0 + bi as f32 * 22.0, msr_y), vec2(18.0, 16.0));
                let resp = ui.interact(br, egui::Id::unique(("trk_msr", i, bi)), Sense::click());
                g.rect(
                    br,
                    CornerRadius::same(3),
                    if *on { *col } else { Color32::from_rgb(0x30, 0x30, 0x38) },
                    Stroke::new(1.0, HAIRLINE),
                    StrokeKind::Inside,
                );
                g.text(br.center(), Align2::CENTER_CENTER, *lbl, FontId::proportional(9.5), if *on { Color32::BLACK } else { TEXT_DIM });
                if resp.clicked() {
                    match bi {
                        0 => t.mute = !t.mute,
                        1 => t.solo = !t.solo,
                        _ => t.rec = !t.rec,
                    }
                }
            }

            // Inline mini meter bar
            let mrect = Rect::from_min_size(pos2(hdr.right() - 48.0, msr_y + 4.0), vec2(40.0, 8.0));
            g.rect_filled(mrect, 2.0, SLOT_EMPTY);
            g.rect_filled(
                Rect::from_min_size(mrect.min, vec2(mrect.width() * t.meter.clamp(0.0, 1.0), mrect.height())),
                2.0,
                PLAY_GREEN,
            );

            // Region block across arrangement lane
            let reg = Rect::from_min_size(pos2(lane.left() + 2.0, y + 3.0), vec2(lane.width() - 6.0, row_h - 6.0));
            g.rect(
                reg,
                CornerRadius::same(4),
                t.color.linear_multiply(0.45),
                Stroke::new(1.0, t.color),
                StrokeKind::Inside,
            );
            // Region header strip
            g.rect_filled(Rect::from_min_size(reg.min, vec2(reg.width(), 14.0)), CornerRadius::same(3), t.color.linear_multiply(0.75));
            g.text(pos2(reg.left() + 6.0, reg.top() + 7.0), Align2::LEFT_CENTER, &t.name, FontId::proportional(10.0), Color32::WHITE);

            let body = Rect::from_min_max(pos2(reg.left() + 2.0, reg.top() + 16.0), pos2(reg.right() - 2.0, reg.bottom() - 2.0));
            if !t.peaks.is_empty() {
                // Draw real audio waveform overview
                let n = t.peaks.len();
                let dx = body.width() / n as f32;
                let mid = body.center().y;
                for (pi, &pk) in t.peaks.iter().enumerate() {
                    let px = body.left() + pi as f32 * dx;
                    let h = pk * body.height() * 0.46;
                    g.line_segment([pos2(px, mid - h), pos2(px, mid + h)], Stroke::new((dx * 0.8).max(1.0), Color32::from_rgb(0xe8, 0xe8, 0xf2)));
                }
            } else {
                // Draw MIDI note preview bars
                for n in &t.notes {
                    let nx = body.left() + (n.start_beat / (bars as f32 * 4.0)) * body.width();
                    let nw = ((n.len_beats / (bars as f32 * 4.0)) * body.width()).max(3.0);
                    let ny_t = 1.0 - ((n.pitch as f32 - 36.0) / 48.0).clamp(0.0, 1.0);
                    let ny = body.top() + ny_t * (body.height() - 3.0);
                    g.rect_filled(Rect::from_min_size(pos2(nx, ny), vec2(nw, 2.5)), 1.0, Color32::from_rgb(0xea, 0xff, 0xea));
                }
            }

            y += row_h;
        }

        // Gold playhead line
        let ph_x = ruler.left() + (self.playhead_beats / 4.0) * bar_w;
        if ph_x >= ruler.left() && ph_x <= ruler.right() {
            g.line_segment([pos2(ph_x, ruler.top()), pos2(ph_x, full.bottom())], Stroke::new(1.5, CYCLE_AMBER));
            g.add(Shape::convex_polygon(
                vec![pos2(ph_x - 5.0, ruler.top()), pos2(ph_x + 5.0, ruler.top()), pos2(ph_x, ruler.top() + 9.0)],
                CYCLE_AMBER,
                Stroke::NONE,
            ));
        }
    }

    fn draw_mixer(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let g = ui.painter();
        let tb = Rect::from_min_size(full.min, vec2(full.width(), 28.0));
        g.rect_filled(tb, 0.0, Color32::from_rgb(0x1f, 0x1f, 0x23));
        g.line_segment([pos2(tb.left(), tb.bottom()), pos2(tb.right(), tb.bottom())], Stroke::new(1.0, HAIRLINE));
        g.text(pos2(tb.left() + 12.0, tb.center().y), Align2::LEFT_CENTER, "Mixer   •   Tracks", FontId::proportional(11.5), TEXT);

        let filters = ["Audio", "Inst", "Aux", "Bus", "Input", "Output", "Master/VCA", "MIDI"];
        let mut fx_x = tb.right() - 420.0;
        for f in filters {
            let r = Rect::from_min_size(pos2(fx_x, tb.top() + 5.0), vec2(48.0, 18.0));
            g.rect_filled(r, CornerRadius::same(3), ACCENT_BLUE);
            g.text(r.center(), Align2::CENTER_CENTER, f, FontId::proportional(10.0), Color32::WHITE);
            fx_x += 51.0;
        }

        let body = Rect::from_min_max(pos2(full.left() + 6.0, tb.bottom() + 4.0), pos2(full.right() - 6.0, full.bottom() - 4.0));
        let strip_w = 88.0;
        let gap = 3.0;
        let out_meter = self.tracks.iter().map(|t| t.meter).fold(0.0f32, f32::max);

        for i in 0..self.tracks.len() {
            let x = body.left() + i as f32 * (strip_w + gap);
            if x + strip_w > body.right() - 185.0 {
                break;
            }
            let r = Rect::from_min_size(pos2(x, body.top()), vec2(strip_w, body.height()));
            let is_sel = i == self.selected;
            let t = &mut self.tracks[i];
            if draw_channel_strip(
                ui,
                r,
                &t.name,
                t.color,
                &t.fx,
                t.send.as_deref(),
                &mut t.volume_db,
                &mut t.pan,
                &mut t.mute,
                &mut t.solo,
                t.meter,
                is_sel,
                false,
                i,
            ) {
                self.selected = i;
            }
        }

        // Stereo Out + Master strips anchored right of tracks
        let tail_x = body.left() + (self.tracks.len().min(10) as f32) * (strip_w + gap) + 4.0;
        let mut dm = false;
        let mut ds = false;
        let mut dp = 0.0;
        let so_r = Rect::from_min_size(pos2(tail_x, body.top()), vec2(strip_w, body.height()));
        draw_channel_strip(
            ui,
            so_r,
            "Stereo Out",
            OUT_PURPLE,
            &[],
            None,
            &mut self.stereo_out_db,
            &mut dp,
            &mut dm,
            &mut ds,
            out_meter,
            false,
            true,
            900,
        );
        let ma_r = Rect::from_min_size(pos2(so_r.right() + gap, body.top()), vec2(strip_w, body.height()));
        draw_channel_strip(
            ui,
            ma_r,
            "Master",
            MASTER_PINK,
            &[],
            None,
            &mut self.master_db,
            &mut dp,
            &mut dm,
            &mut ds,
            out_meter * 0.9,
            false,
            false,
            901,
        );
    }

    fn draw_piano_roll(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let g = ui.painter();
        let tb = Rect::from_min_size(full.min, vec2(full.width(), 26.0));
        g.rect_filled(tb, 0.0, Color32::from_rgb(0x21, 0x21, 0x25));
        let sel_name = self.tracks.get(self.selected).map(|t| t.name.as_str()).unwrap_or("Track");
        g.text(pos2(tb.left() + 12.0, tb.center().y), Align2::LEFT_CENTER, format!("Piano Roll  •  {sel_name}   (Click grid to add/remove notes)"), FontId::proportional(11.5), TEXT);

        let keys_w = 48.0;
        let rows = 24usize;
        let base_pitch = 48u8; // C3..B4
        let grid = Rect::from_min_max(pos2(full.left() + keys_w, tb.bottom()), full.max);
        let row_h = (grid.height() / rows as f32).max(10.0);
        let cols = 32usize;
        let col_w = (grid.width() / cols as f32).max(12.0);

        for r in 0..rows {
            let pitch = base_pitch + (rows - 1 - r) as u8;
            let black = matches!(pitch % 12, 1 | 3 | 6 | 8 | 10);
            let y = grid.top() + r as f32 * row_h;
            let kr = Rect::from_min_size(pos2(full.left(), y), vec2(keys_w, row_h));
            g.rect_filled(kr, 0.0, if black { Color32::from_rgb(0x16, 0x16, 0x1a) } else { Color32::from_rgb(0xd8, 0xd8, 0xde) });
            g.line_segment([pos2(full.left(), y + row_h), pos2(full.right(), y + row_h)], Stroke::new(1.0, Color32::from_rgb(0x22, 0x22, 0x28)));
            if pitch % 12 == 0 {
                g.text(pos2(kr.right() - 4.0, kr.center().y), Align2::RIGHT_CENTER, format!("C{}", pitch / 12 - 1), FontId::proportional(9.0), Color32::from_rgb(0x22, 0x22, 0x26));
            }
            let rr = Rect::from_min_size(pos2(grid.left(), y), vec2(grid.width(), row_h));
            g.rect_filled(rr, 0.0, if black { Color32::from_rgb(0x12, 0x12, 0x15) } else { Color32::from_rgb(0x18, 0x18, 0x1c) });
        }
        for c in 0..cols {
            let x = grid.left() + c as f32 * col_w;
            let strong = c % 4 == 0;
            g.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], Stroke::new(1.0, if strong { Color32::from_rgb(0x2e, 0x2e, 0x36) } else { Color32::from_rgb(0x1f, 0x1f, 0x25) }));
        }

        if let Some(t) = self.tracks.get_mut(self.selected) {
            for n in &t.notes {
                if n.pitch < base_pitch || n.pitch >= base_pitch + rows as u8 {
                    continue;
                }
                let r = (base_pitch + rows as u8 - 1 - n.pitch) as usize;
                let x = grid.left() + (n.start_beat * 2.0) * col_w;
                let w = (n.len_beats * 2.0 * col_w).max(8.0);
                if x > grid.right() {
                    continue;
                }
                let nr = Rect::from_min_size(pos2(x, grid.top() + r as f32 * row_h + 1.0), vec2(w, row_h - 2.0));
                g.rect(nr, CornerRadius::same(2), Color32::from_rgb(0xe8, 0x86, 0x38), Stroke::new(1.0, Color32::from_rgb(0xff, 0xc8, 0x8a)), StrokeKind::Inside);
            }
            let resp = ui.interact(grid, egui::Id::unique("pr_grid_click"), Sense::click());
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let r = ((p.y - grid.top()) / row_h).floor() as usize;
                    let c = ((p.x - grid.left()) / col_w).floor() as usize;
                    if r < rows && c < cols {
                        let pitch = base_pitch + (rows - 1 - r) as u8;
                        let beat = c as f32 * 0.5;
                        if let Some(idx) = t.notes.iter().position(|n| n.pitch == pitch && (n.start_beat - beat).abs() < 0.25) {
                            t.notes.remove(idx);
                        } else {
                            t.notes.push(MidiNote { start_beat: beat, len_beats: 0.5, pitch, vel: 105 });
                        }
                    }
                }
            }
        }
    }

    fn draw_muse_window(&mut self, ctx: &egui::Context) {
        if !self.show_muse {
            return;
        }
        let mut open = self.show_muse;
        egui::Window::new(RichText::new("✦ Muse AI Studio").color(Color32::WHITE).strong())
            .id(egui::Id::unique("sexos_muse_win"))
            .open(&mut open)
            .default_size([460.0, 380.0])
            .frame(egui::Frame::window(&ctx.global_style()).fill(PANEL).stroke(Stroke::new(1.0, ACCENT_MUSE)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.muse_tab, MuseTab::CreateSong, "Create Song");
                    ui.selectable_value(&mut self.muse_tab, MuseTab::Stems, "6-Stem Split");
                    ui.selectable_value(&mut self.muse_tab, MuseTab::StudioChat, "Studio Chat");
                });
                ui.separator();
                match self.muse_tab {
                    MuseTab::CreateSong => {
                        ui.label(RichText::new("Prompt").color(TEXT_DIM));
                        ui.add(egui::TextEdit::multiline(&mut self.muse_prompt).desired_rows(2).desired_width(f32::INFINITY));
                        ui.label(RichText::new("Lyrics / LRC").color(TEXT_DIM));
                        ui.add(egui::TextEdit::multiline(&mut self.muse_lyrics).desired_rows(2).desired_width(f32::INFINITY));
                        ui.horizontal(|ui| {
                            ui.label("Key:");
                            ui.add(egui::TextEdit::singleline(&mut self.muse_key).desired_width(80.0));
                            ui.label("Bars:");
                            ui.add(egui::DragValue::new(&mut self.muse_bars).range(2..=16));
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.add(egui::Button::new(RichText::new("✦ Generate to Timeline").color(Color32::WHITE).strong()).fill(ACCENT_MUSE)).clicked() {
                                let frames = self.bars_to_frames(self.muse_bars);
                                let req = MuseJobRequest {
                                    nonce: self.make_nonce(),
                                    mode: MuseMode::CreateSong {
                                        prompt: self.muse_prompt.clone(),
                                        lyrics: self.muse_lyrics.clone(),
                                        vocal_language: "en".into(),
                                        bpm: self.tempo.round() as u16,
                                        key_scale: self.muse_key.clone(),
                                        time_sig: [4, 4],
                                        duration_frames: frames,
                                        thinking_cot: false,
                                        batch_count: 1,
                                    },
                                    reference_pcm: None,
                                    midi_pattern: None,
                                    target_tier: RamTier::Tier0Featherweight,
                                    created_at_ms: 0,
                                };
                                match Tier0Engine::new().execute(&req) {
                                    Ok(res) => {
                                        let name = format!("Muse: {}", self.muse_key);
                                        self.tracks.push(Track::from_audio(&name, ACCENT_MUSE, vec!["Channel EQ", "Space D"], res.frames));
                                        self.selected = self.tracks.len() - 1;
                                        self.status = format!("Muse generated \"{name}\" onto timeline");
                                    }
                                    Err(e) => self.status = format!("Muse error: {e:?}"),
                                }
                            }
                            if ui.button("Generate + 6 Stems").clicked() {
                                let frames = self.bars_to_frames(self.muse_bars);
                                let req = MuseJobRequest {
                                    nonce: self.make_nonce(),
                                    mode: MuseMode::CreateSong {
                                        prompt: self.muse_prompt.clone(),
                                        lyrics: self.muse_lyrics.clone(),
                                        vocal_language: "en".into(),
                                        bpm: self.tempo.round() as u16,
                                        key_scale: self.muse_key.clone(),
                                        time_sig: [4, 4],
                                        duration_frames: frames,
                                        thinking_cot: false,
                                        batch_count: 1,
                                    },
                                    reference_pcm: None,
                                    midi_pattern: None,
                                    target_tier: RamTier::Tier0Featherweight,
                                    created_at_ms: 0,
                                };
                                let engine = Tier0Engine::new();
                                if let Ok(song) = engine.execute(&req) {
                                    let sreq = MuseJobRequest {
                                        nonce: self.make_nonce(),
                                        mode: MuseMode::ExtractStems,
                                        reference_pcm: Some(song.frames),
                                        midi_pattern: None,
                                        target_tier: RamTier::Tier0Featherweight,
                                        created_at_ms: 0,
                                    };
                                    if let Ok(stems) = engine.execute(&sreq) {
                                        for (role, audio) in stems.stems {
                                            let (n, c) = match role {
                                                StemRole::Vocals => ("Muse Vocals", ACCENT_ARIEL),
                                                StemRole::Drums => ("Muse Drums", REC_RED),
                                                StemRole::Bass => ("Muse Bass", ACCENT_BLUE),
                                                StemRole::Chords => ("Muse Chords", ACCENT_MUSE),
                                                StemRole::Fx => ("Muse FX", ACCENT_RACK),
                                                StemRole::Other => ("Muse Other", PLAY_GREEN),
                                            };
                                            self.tracks.push(Track::from_audio(n, c, vec!["Channel EQ"], audio));
                                        }
                                        self.status = "Muse split 6 stems onto timeline".into();
                                    }
                                }
                            }
                        });
                    }
                    MuseTab::Stems => {
                        ui.label("Add a single generated stem layer directly to the timeline:");
                        ui.horizontal_wrapped(|ui| {
                            for role in [StemRole::Drums, StemRole::Bass, StemRole::Chords, StemRole::Vocals, StemRole::Fx] {
                                if ui.button(format!("+ {}", role.as_str())).clicked() {
                                    let frames = self.bars_to_frames(self.muse_bars);
                                    let req = MuseJobRequest {
                                        nonce: self.make_nonce(),
                                        mode: MuseMode::AddLayer { role },
                                        reference_pcm: Some(Arc::new(vec![[0i16; 2]; frames])),
                                        midi_pattern: None,
                                        target_tier: RamTier::Tier0Featherweight,
                                        created_at_ms: 0,
                                    };
                                    if let Ok(res) = Tier0Engine::new().execute(&req) {
                                        let name = format!("Muse {}", role.as_str());
                                        self.tracks.push(Track::from_audio(&name, ACCENT_MUSE, vec!["Channel EQ"], res.frames));
                                        self.status = format!("Added {name}");
                                    }
                                }
                            }
                        });
                    }
                    MuseTab::StudioChat => {
                        for line in &self.muse_chat_log {
                            ui.label(line);
                        }
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut self.muse_chat_input);
                            if ui.button("Send").clicked() && !self.muse_chat_input.trim().is_empty() {
                                let txt = self.muse_chat_input.trim().to_string();
                                let parsed = parse_studio_chat(&txt, false, false);
                                self.muse_chat_log.push(format!("> {txt}"));
                                self.muse_chat_log.push(format!("Parsed: {parsed:?}"));
                                self.muse_prompt = txt;
                                self.muse_chat_input.clear();
                            }
                        });
                    }
                }
            });
        self.show_muse = open;
    }

    fn draw_ariel_window(&mut self, ctx: &egui::Context) {
        if !self.show_ariel {
            return;
        }
        let mut open = self.show_ariel;
        egui::Window::new(RichText::new("◉ Ariel Formant Vocal Synth").color(Color32::WHITE).strong())
            .id(egui::Id::unique("sexos_ariel_win"))
            .open(&mut open)
            .default_size([400.0, 340.0])
            .frame(egui::Frame::window(&ctx.global_style()).fill(PANEL).stroke(Stroke::new(1.0, ACCENT_ARIEL)))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for p in ArielPreset::ALL {
                        if ui.selectable_label(self.ariel_preset == p, p.name()).clicked() {
                            self.ariel_preset = p;
                            p.apply(&mut self.ariel_params);
                        }
                    }
                });
                ui.separator();
                ui.add(egui::TextEdit::multiline(&mut self.ariel_lyrics).desired_rows(2).desired_width(f32::INFINITY));
                ui.add(egui::Slider::new(&mut self.ariel_params.f0, 65.0..=660.0).text("Pitch Hz"));
                ui.add(egui::Slider::new(&mut self.ariel_params.formant_scale, 0.7..=1.5).text("Formant"));
                ui.add(egui::Slider::new(&mut self.ariel_params.vibrato_depth, 0.0..=15.0).text("Vibrato"));
                ui.add(egui::Slider::new(&mut self.ariel_params.aspiration, 0.0..=1.0).text("Breathiness"));
                if ui.add(egui::Button::new(RichText::new("◉ Render Vocal to Timeline").color(Color32::BLACK).strong()).fill(ACCENT_ARIEL)).clicked() {
                    if let Ok((frames, _)) = vocal::synthesize(&self.ariel_lyrics, &self.ariel_params) {
                        let name = format!("Ariel ({})", self.ariel_preset.name());
                        self.tracks.push(Track::from_audio(&name, ACCENT_ARIEL, vec!["Channel EQ", "Space D"], frames));
                        self.selected = self.tracks.len() - 1;
                        self.status = format!("Rendered {name} onto timeline");
                    }
                }
            });
        self.show_ariel = open;
    }

    fn draw_rack_window(&mut self, ctx: &egui::Context) {
        if !self.show_rack {
            return;
        }
        let mut open = self.show_rack;
        let bg = if self.rack_dark_room { Color32::from_rgb(0x11, 0x11, 0x15) } else { PANEL };
        egui::Window::new(RichText::new("⚡ EuropaRack Modular Patchbay").color(Color32::WHITE).strong())
            .id(egui::Id::unique("sexos_rack_win"))
            .open(&mut open)
            .default_size([520.0, 360.0])
            .frame(egui::Frame::window(&ctx.global_style()).fill(bg).stroke(Stroke::new(1.0, ACCENT_RACK)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_label("Patch")
                        .selected_text(self.rack_patch.name())
                        .show_ui(ui, |ui| {
                            for p in FactoryPatch::ALL {
                                if ui.selectable_label(self.rack_patch == p, p.name()).clicked() {
                                    self.rack_patch = p;
                                    if let Ok(g) = p.build() {
                                        self.rack_graph = g;
                                    }
                                }
                            }
                        });
                    ui.checkbox(&mut self.rack_dark_room, "Dark Room");
                });
                ui.separator();
                egui::ScrollArea::horizontal().max_height(180.0).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for node in &mut self.rack_graph.nodes {
                            egui::Frame::new()
                                .fill(LANE_BG)
                                .stroke(Stroke::new(1.0, Color32::from_rgb(0x34, 0x34, 0x3e)))
                                .corner_radius(CornerRadius::same(4))
                                .inner_margin(6.0)
                                .show(ui, |ui| {
                                    ui.set_min_width(116.0);
                                    ui.label(RichText::new(format!("{:?}", node.kind)).color(ACCENT_RACK).strong().size(11.0));
                                    for idx in 0..node.module.param_count().min(3) {
                                        let mut v = node.module.get_param(idx);
                                        let max_v = if v.abs() > 10.0 { (v.abs() * 2.0).max(2000.0) } else { 1.0 };
                                        if ui.add(egui::DragValue::new(&mut v).range(0.0..=max_v).prefix(format!("P{idx}: "))).changed() {
                                            node.module.set_param(idx, v);
                                        }
                                    }
                                });
                        }
                    });
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Bars:");
                    ui.add(egui::DragValue::new(&mut self.rack_bars).range(1..=12));
                    if ui.add(egui::Button::new(RichText::new("⚡ Render Patch to Timeline").color(Color32::BLACK).strong()).fill(ACCENT_RACK)).clicked() {
                        let total = self.bars_to_frames(self.rack_bars);
                        let step = (total / (self.rack_bars as usize * 4).max(1)).max(4800);
                        let iv = [0u8, 3, 7, 10, 12, 7, 3, 0];
                        let mut notes = Vec::new();
                        let mut pos = 0usize;
                        let mut k = 0usize;
                        while pos < total {
                            notes.push(moan_core::midi::Note {
                                start: pos as u32,
                                frames: (step * 3 / 4) as u32,
                                key: (self.rack_root + iv[k % iv.len()]).min(127),
                                velocity: 108,
                            });
                            pos += step;
                            k += 1;
                        }
                        self.rack_graph.reset();
                        if let Ok(frames) = self.rack_graph.bake_to_frames(total, None, &notes) {
                            let name = format!("Rack: {}", self.rack_patch.name());
                            self.tracks.push(Track::from_audio(&name, ACCENT_RACK, vec!["Channel EQ"], frames));
                            self.selected = self.tracks.len() - 1;
                            self.status = format!("Baked {name} onto timeline");
                        }
                    }
                });
            });
        self.show_rack = open;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_channel_strip(
    ui: &mut Ui,
    r: Rect,
    name: &str,
    color: Color32,
    fx: &[String],
    send: Option<&str>,
    vol_db: &mut f32,
    pan: &mut f32,
    mute: &mut bool,
    solo: &mut bool,
    meter: f32,
    selected: bool,
    outline_purple: bool,
    id: usize,
) -> bool {
    let g = ui.painter();
    let border = if outline_purple {
        OUT_PURPLE
    } else if selected {
        Color32::from_rgb(0x4c, 0x4e, 0x5c)
    } else {
        HAIRLINE
    };
    g.rect(r, CornerRadius::same(3), if selected { STRIP_SEL } else { STRIP_BG }, Stroke::new(1.0, border), StrokeKind::Inside);

    let click_top = ui.interact(Rect::from_min_size(r.min, vec2(r.width(), 22.0)), egui::Id::unique(("strip_sel", id)), Sense::click()).clicked();

    let pad = 5.0;
    let w = r.width() - pad * 2.0;
    let mut y = r.top() + 5.0;

    // Setting slot
    let s_r = Rect::from_min_size(pos2(r.left() + pad, y), vec2(w, 16.0));
    g.rect(s_r, CornerRadius::same(2), Color32::from_rgb(0x2a, 0x2a, 0x30), Stroke::new(1.0, HAIRLINE), StrokeKind::Inside);
    g.text(s_r.center(), Align2::CENTER_CENTER, "Setting", FontId::proportional(9.5), TEXT_DIM);
    y += 20.0;

    // EQ curve thumbnail
    let eq_r = Rect::from_min_size(pos2(r.left() + pad, y), vec2(w, 22.0));
    g.rect_filled(eq_r, CornerRadius::same(2), SLOT_EMPTY);
    g.line_segment([pos2(eq_r.left() + 3.0, eq_r.center().y), pos2(eq_r.right() - 3.0, eq_r.center().y)], Stroke::new(1.2, ACCENT_BLUE));
    y += 26.0;

    // Audio FX blue slots (up to 3)
    for slot in 0..3 {
        let fx_r = Rect::from_min_size(pos2(r.left() + pad, y), vec2(w, 14.0));
        if let Some(fx_name) = fx.get(slot) {
            g.rect_filled(fx_r, CornerRadius::same(2), PLUGIN_BLUE);
            g.text(fx_r.center(), Align2::CENTER_CENTER, fx_name, FontId::proportional(9.0), Color32::WHITE);
        } else {
            g.rect_filled(fx_r, CornerRadius::same(2), SLOT_EMPTY);
        }
        y += 16.0;
    }

    // Send slot
    y += 2.0;
    let snd_r = Rect::from_min_size(pos2(r.left() + pad, y), vec2(w, 14.0));
    if let Some(s) = send {
        g.rect_filled(snd_r, CornerRadius::same(2), PLUGIN_BLUE);
        g.text(snd_r.center(), Align2::CENTER_CENTER, s, FontId::proportional(9.0), Color32::WHITE);
    } else {
        g.rect_filled(snd_r, CornerRadius::same(2), SLOT_EMPTY);
    }
    y += 18.0;

    // Automation Read badge
    let read_r = Rect::from_min_size(pos2(r.left() + pad, y), vec2(w, 14.0));
    g.rect(read_r, CornerRadius::same(2), Color32::from_rgb(0x26, 0x26, 0x2c), Stroke::new(1.0, HAIRLINE), StrokeKind::Inside);
    g.text(read_r.center(), Align2::CENTER_CENTER, "Read", FontId::proportional(9.5), READ_GREEN);
    y += 18.0;

    // Rotary Pan knob
    let knob_c = pos2(r.center().x, y + 11.0);
    let knob_r = Rect::from_center_size(knob_c, vec2(22.0, 22.0));
    let kresp = ui.interact(knob_r, egui::Id::unique(("strip_pan", id)), Sense::drag());
    if kresp.dragged() {
        *pan = (*pan + kresp.drag_delta().x * 0.015).clamp(-1.0, 1.0);
    }
    g.circle_filled(knob_c, 9.5, Color32::from_rgb(0x52, 0x52, 0x5c));
    g.circle_stroke(knob_c, 9.5, Stroke::new(1.0, HAIRLINE));
    let ang = -std::f32::consts::FRAC_PI_2 + *pan * 2.2;
    g.line_segment([knob_c, knob_c + vec2(ang.cos(), ang.sin()) * 8.0], Stroke::new(1.5, Color32::WHITE));
    y += 26.0;

    // dB readout
    g.text(pos2(r.center().x, y + 5.0), Align2::CENTER_CENTER, format!("{vol_db:.1}"), FontId::monospace(9.5), TEXT_DIM);
    y += 14.0;

    // Bottom footer: M / S + vibrant color label
    let footer_h = 44.0;
    let fader_bottom = (r.bottom() - footer_h).max(y + 36.0);

    // Fader track + thumb (left) & stereo LED meter (right)
    let fader_rect = Rect::from_min_max(pos2(r.left() + 14.0, y), pos2(r.left() + 36.0, fader_bottom));
    let fresp = ui.interact(fader_rect, egui::Id::unique(("strip_fader", id)), Sense::click_and_drag());
    if fresp.clicked() || fresp.dragged() {
        if let Some(p) = fresp.interact_pointer_pos() {
            let t = 1.0 - ((p.y - fader_rect.top()) / fader_rect.height()).clamp(0.0, 1.0);
            *vol_db = -60.0 + t * 66.0;
        }
    }
    let track_x = fader_rect.center().x;
    g.line_segment([pos2(track_x, fader_rect.top()), pos2(track_x, fader_rect.bottom())], Stroke::new(3.0, SLOT_EMPTY));
    let norm = ((*vol_db + 60.0) / 66.0).clamp(0.0, 1.0);
    let thumb_y = fader_rect.bottom() - norm * fader_rect.height();
    let thumb = Rect::from_center_size(pos2(track_x, thumb_y), vec2(16.0, 12.0));
    g.rect(thumb, CornerRadius::same(2), Color32::from_rgb(0xb8, 0xb8, 0xc4), Stroke::new(1.0, HAIRLINE), StrokeKind::Inside);

    // Stereo LED meter
    let m_rect = Rect::from_min_max(pos2(r.right() - 24.0, y), pos2(r.right() - 10.0, fader_bottom));
    g.rect_filled(m_rect, 1.0, SLOT_EMPTY);
    let mh = m_rect.height() * meter.clamp(0.0, 1.0);
    if mh > 1.0 {
        let fill_r = Rect::from_min_max(pos2(m_rect.left() + 1.0, m_rect.bottom() - mh), pos2(m_rect.right() - 1.0, m_rect.bottom()));
        let mc = if meter > 0.85 { REC_RED } else if meter > 0.65 { SOLO_YELLOW } else { PLAY_GREEN };
        g.rect_filled(fill_r, 1.0, mc);
    }

    // M / S buttons
    let ms_y = r.bottom() - 40.0;
    let m_btn = Rect::from_min_size(pos2(r.left() + pad, ms_y), vec2(w * 0.48, 16.0));
    let s_btn = Rect::from_min_size(pos2(m_btn.right() + 2.0, ms_y), vec2(w * 0.48, 16.0));
    let mr = ui.interact(m_btn, egui::Id::unique(("strip_m", id)), Sense::click());
    let sr = ui.interact(s_btn, egui::Id::unique(("strip_s", id)), Sense::click());
    if mr.clicked() { *mute = !*mute; }
    if sr.clicked() { *solo = !*solo; }
    g.rect_filled(m_btn, CornerRadius::same(2), if *mute { MUTE_BLUE } else { Color32::from_rgb(0x32, 0x32, 0x3a) });
    g.text(m_btn.center(), Align2::CENTER_CENTER, "M", FontId::proportional(9.5), if *mute { Color32::WHITE } else { TEXT_DIM });
    g.rect_filled(s_btn, CornerRadius::same(2), if *solo { SOLO_YELLOW } else { Color32::from_rgb(0x32, 0x32, 0x3a) });
    g.text(s_btn.center(), Align2::CENTER_CENTER, "S", FontId::proportional(9.5), if *solo { Color32::BLACK } else { TEXT_DIM });

    // Vibrant Logic Noir bottom track color bar
    let lbl_r = Rect::from_min_size(pos2(r.left() + 2.0, r.bottom() - 20.0), vec2(r.width() - 4.0, 18.0));
    g.rect_filled(lbl_r, CornerRadius::same(2), color);
    let short: String = name.chars().take(12).collect();
    g.text(lbl_r.center(), Align2::CENTER_CENTER, short, FontId::proportional(10.0), Color32::from_rgb(0x0c, 0x0c, 0x10));

    click_top
}

fn main() {
    #[cfg(target_os = "hermit")]
    {
        let mut app = StudioApp::new();
        let options = sex_egui::Options::new("Moan Studio");
        sex_egui::run_ui(options, move |ui| {
            app.ui(ui);
        });
    }
    #[cfg(not(target_os = "hermit"))]
    {
        // Host smoke test: paint one frame via sex_egui::Painter to verify layout cleanly.
        let ctx = egui::Context::default();
        let mut app = StudioApp::new();
        let mut painter = sex_egui::Painter::new();
        let mut buf = vec![0u32; 1280 * 720];
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1280.0, 720.0))),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| app.ui(ui));
        let mut target = sex_egui::raster::Target { pixels: &mut buf, width: 1280, height: 720 };
        let stats = painter.paint(&ctx, out, &mut target);
        println!("Moan Studio SexOS host smoke OK: {} tris", stats.triangles);
    }
}
