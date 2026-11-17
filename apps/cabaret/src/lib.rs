// SPDX-License-Identifier: GPL-3.0-only
//! **Cabaret**: the SexOS flagship live video broadcast studio, multi-stream router,
//! wide-gamut color/scope suite, and 12-plugin compositor built on `weft`
//! (100% OBS Studio + OmaBroadcast parity).
//!
//! Pure `#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`, driving both the SexOS window
//! event loop and headless OmaTools CLI + integration tests cleanly.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

pub mod control;
pub mod render;
pub mod studio;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use cabaret_core::audio::StripId;
use cabaret_core::compositor::FrameBuffer;
use cabaret_core::outputs::RecordingState;
use cabaret_core::transitions::MultiviewPreset;
use cabaret_plugins::shader_filter::ShaderPreset;
use cabaret_plugins::streamfx_3d::CornerPinQuad;
use sex_color::{ColorSpace, FilmStock};
use weft::menu::{Command, Menu};
use weft::widget::{Column, Id, Layout, Rows, Tab, TextStyle};
use weft::{ButtonStyle, Coach, CoachControls, CoachStep, Response, TabEvent, Ui};

use crate::render::{
    argb_fit, fb_to_weft_argb, render_audio_visualizer_fb, render_multiview_frame,
    render_preview_frame, render_program_frame, render_scope_argb, render_vertical_preview,
    UI_PREVIEW_H, UI_PREVIEW_W, UI_VERTICAL_H, UI_VERTICAL_W,
};
use crate::studio::{ActiveTransitionPreset, CabaretStudio, ScopeKind};

#[cfg(feature = "sexos")]
pub mod sexos_runtime {
    //! Live SexOS PDX / SilkBar tally / HIDE_FROM_CAPTURE / VirtualCam / Shibari bridge.
    use cabaret_core::outputs::RecordingState;
    use sex_pdx::desktop_recording::{
        CapturePixelFmt, CaptureSourceTarget, SessionSequence, StudioSessionSpec,
        WindowCaptureFlagsReq, WindowCapturePolicy,
    };
    use sex_uvc::virtual_cam::{VirtualCamFormat, VirtualCamRing, VirtualCamSource};
    use shibari::{Keeper, Owner};

    use crate::render::{render_program_frame, UI_PREVIEW_H, UI_PREVIEW_W};
    use crate::studio::CabaretStudio;

    /// Holds kernel/service-bound OS objects when compiled for SexOS (`feature = "sexos"`).
    pub struct SexosBroadcastRuntime {
        pub keeper: Keeper,
        pub owner: Owner,
        pub virtual_cam_ring: VirtualCamRing,
        pub window_capture_flags: Option<WindowCaptureFlagsReq>,
        pub session_spec: StudioSessionSpec,
    }

    impl SexosBroadcastRuntime {
        /// Initializes Shibari RAM-only key vault, `Cabaret Virtual Camera` loopback ring,
        /// `WindowCapturePolicy::HIDE_FROM_CAPTURE` flags, and `StudioSessionSpec`.
        pub fn initialize(window_frame_id: u32, studio: &CabaretStudio) -> Self {
            let mut keeper = Keeper::new(0xCA8A_0001_5EED, 32);
            let owner = Owner(1);
            for (slot, secret) in &studio.active_secrets {
                let _ = keeper.tie(owner, &slot.label, secret.as_bytes());
            }

            let virtual_cam_ring = VirtualCamRing::with_max_frame(
                VirtualCamSource::Program,
                1920,
                1080,
                VirtualCamFormat::Rgba32,
            )
            .expect("allocate virtual cam ring");

            let window_capture_flags =
                WindowCaptureFlagsReq::new(window_frame_id.max(1), WindowCapturePolicy::sensitive_overlay());

            let seq = SessionSequence::new(1, 1).expect("initial session sequence");
            let session_spec = StudioSessionSpec::new(
                seq,
                CaptureSourceTarget::FollowFocusedWindow,
                1920,
                1080,
                60,
                CapturePixelFmt::Xrgb8888,
                studio.capture_exclusion.silkbar_tally_active,
                false,
            );

            Self {
                keeper,
                owner,
                virtual_cam_ring,
                window_capture_flags,
                session_spec,
            }
        }

        /// Publishes the latest composited Program frame to `"Cabaret Virtual Camera"` when active
        /// and keeps SilkBar tally enforcement in sync.
        pub fn sync_and_publish(&mut self, studio: &mut CabaretStudio) {
            let tally = studio.stream_active
                || studio.recording.state == RecordingState::Recording
                || studio.virtual_cam.active;
            self.session_spec.tally_active = tally;

            if studio.virtual_cam.active {
                let fb = render_program_frame(studio, UI_PREVIEW_W, UI_PREVIEW_H);
                let rgba = fb.to_srgb_rgba8();
                if let Ok(hdr) = self.virtual_cam_ring.publish_rgba(
                    fb.width,
                    fb.height,
                    60,
                    VirtualCamFormat::Rgba32,
                    tally,
                    &rgba,
                ) {
                    studio.virtual_cam.sequence = hdr.sequence;
                    studio.virtual_cam.epoch = hdr.epoch;
                }
            }
        }
    }
}

/// Top-level dockable/tabbed workspace views in Cabaret.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CabaretTab {
    StudioMixer = 0,
    Multiview = 1,
    PluginsVertical = 2,
    ScopesColor = 3,
    OutputsMultiStream = 4,
    PrompterQc = 5,
}

impl CabaretTab {
    pub fn from_index(idx: usize) -> Self {
        match idx {
            1 => Self::Multiview,
            2 => Self::PluginsVertical,
            3 => Self::ScopesColor,
            4 => Self::OutputsMultiStream,
            5 => Self::PrompterQc,
            _ => Self::StudioMixer,
        }
    }
}

/// Widget handles for one Audio Mixer channel strip card.
#[derive(Clone, Copy, Debug)]
pub struct StripUi {
    pub strip_id: StripId,
    pub mute_btn: Id,
    pub solo_btn: Id,
    pub fader_slider: Id,
    pub meters_label: Id,
}

/// Widget handles for one Downstream Keyer (DSK) channel card.
#[derive(Clone, Copy, Debug)]
pub struct DskChannelUi {
    pub channel_id: u64,
    pub toggle_btn: Id,
    pub tie_check: Id,
    pub status_label: Id,
}

/// Complete retained Weft interface for **Cabaret**.
pub struct Cabaret {
    pub studio: CabaretStudio,
    pub active_tab: CabaretTab,
    pub last_program_frame: FrameBuffer,

    // Chrome: Coach, Tabs, Toolbar, Tally Status
    pub coach: Coach,
    pub coach_controls: CoachControls,
    pub tabs: Id,
    pub guide_btn: Id,
    pub header_status: Id,
    pub tally_bar: Id,

    // Tab 0: Studio & Mixer
    pub preview_image: Id,
    pub preview_caption: Id,
    pub program_image: Id,
    pub program_caption: Id,
    pub take_btn: Id,
    pub undo_btn: Id,
    pub redo_btn: Id,
    pub tbar_slider: Id,
    pub tbar_label: Id,
    pub transition_btns: Vec<(Id, ActiveTransitionPreset)>,
    pub wipe_prev_btn: Id,
    pub wipe_next_btn: Id,
    pub wipe_label: Id,
    pub deck_prev_btn: Id,
    pub deck_next_btn: Id,
    pub deck_status_label: Id,
    pub studios_list: Id,
    pub cue_preview_btn: Id,
    pub take_selected_btn: Id,
    pub sources_list: Id,
    pub toggle_source_vis_btn: Id,
    pub toggle_cam_filter_btn: Id,
    pub audio_pulse_btn: Id,
    pub audio_vis_image: Id,
    pub strip_uis: Vec<StripUi>,

    // Tab 1: Multiview (4/9/16/25 + Standard8)
    pub mv_preset_btns: Vec<(Id, MultiviewPreset)>,
    pub mv_safe_areas_check: Id,
    pub mv_image: Id,
    pub mv_status_label: Id,
    pub mv_slot_btns: Vec<(Id, usize)>,
    pub mv_slot_take_btn: Id,
    pub mv_selected_slot: usize,

    // Tab 2: Plugins, DSK & 9:16 Vertical
    pub dsk_uis: Vec<DskChannelUi>,
    pub vertical_image: Id,
    pub vertical_backtrack_btn: Id,
    pub vertical_summary: Id,
    pub shader_cycle_btn: Id,
    pub shader_clear_btn: Id,
    pub shader_label: Id,
    pub warp_3d_check: Id,
    pub freeze_btn: Id,
    pub time_warp_check: Id,
    pub time_warp_reset_btn: Id,
    pub delay_slowmo_btn: Id,
    pub source_tap_record_btn: Id,
    pub captions_burn_check: Id,
    pub captions_push_btn: Id,
    pub captions_status: Id,
    pub macro_spindle_btn: Id,
    pub macro_main_btn: Id,
    pub macro_status: Id,
    pub link_toggle_btn: Id,
    pub link_status: Id,

    // Tab 3: Scopes & Color
    pub color_space_btns: Vec<(Id, ColorSpace)>,
    pub film_stock_next_btn: Id,
    pub film_stock_clear_btn: Id,
    pub film_stock_label: Id,
    pub scope_kind_btns: Vec<(Id, ScopeKind)>,
    pub scope_image: Id,
    pub scope_ref_image: Id,
    pub scope_metrics_label: Id,

    // Tab 4: Outputs & Multi-Stream
    pub stream_toggle_btn: Id,
    pub record_toggle_btn: Id,
    pub record_pause_btn: Id,
    pub chapter_add_btn: Id,
    pub split_file_btn: Id,
    pub replay_save_btn: Id,
    pub vcam_toggle_btn: Id,
    pub target_checks: Vec<(Id, cabaret_core::outputs::DestId)>,
    pub outputs_summary_label: Id,
    pub encoder_groups_label: Id,
    pub log_tail_label: Id,

    // Tab 5: Prompter & QC Player
    pub prompter_badge: Id,
    pub prompter_wpm_down_btn: Id,
    pub prompter_wpm_up_btn: Id,
    pub prompter_mirror_h_check: Id,
    pub prompter_mirror_v_check: Id,
    pub prompter_voice_field: Id,
    pub prompter_voice_track_btn: Id,
    pub prompter_script_label: Id,
    pub qc_j_btn: Id,
    pub qc_k_btn: Id,
    pub qc_l_btn: Id,
    pub qc_step_back_btn: Id,
    pub qc_step_fwd_btn: Id,
    pub qc_mark_in_btn: Id,
    pub qc_mark_out_btn: Id,
    pub qc_status_label: Id,
    pub open_splicy_btn: Id,
    pub splicy_summary_label: Id,
}

fn walkthrough_steps() -> Vec<CoachStep> {
    vec![
        CoachStep::new(
            "studio_mode",
            "Studio Mode & Transitions",
            "Select a Studio in the Studios list (Preview green tally), pick Cut/Fade/Wipe29/Stinger/Move, drag the T-Bar or click Take to transition live to Program.",
            "Preview / Program & Take",
        ),
        CoachStep::new(
            "audio_mixer",
            "Broadcast Audio Mixer & Sidechain Ducking",
            "Adjust Mic / Serenade and Desktop / Bell App Tap faders. Desktop automatically ducks via cross-channel sidechain keyed from Mic, with live 4x True-Peak dBTP and EBU R128 LUFS meters.",
            "Audio Mixer Panel",
        ),
        CoachStep::new(
            "multiview",
            "Multiview & 1-Click Cue / Take",
            "Switch between Standard (8+2), 2x2, 3x3, 4x4, and 5x5 Multiview grids with green Preview / red Program tally borders and 90%/80% safe-area guides.",
            "Multiview Tab",
        ),
        CoachStep::new(
            "plugins_dsk",
            "All 12 Native Plugins, DSK & 9:16 Vertical",
            "Toggle persistent Downstream Keyers above transitions, monitor the simultaneous 9:16 Vertical canvas, cycle GPU-free Shaders, 3D Corner-Pin, TimeWarp, Local Captions, Macros, and CabaretLink.",
            "Plugins & DSK & 9:16 Tab",
        ),
        CoachStep::new(
            "scopes_color",
            "Wide-Gamut Color & Broadcast Scopes",
            "Switch canvas color spaces (Rec.709, Linear Rec.2020, HDR10 PQ, HLG), apply physical Film Stock LUTs, and inspect Waveform, RGB Parade, Vectorscope, and Histogram.",
            "Scopes & Color Tab",
        ),
        CoachStep::new(
            "outputs_splicy",
            "Multi-Stream, HIDE_FROM_CAPTURE Prompter & Splicy",
            "Go live across Twitch/YouTube AV1/SRT/WHIP with Shibari redacted keys, track spoken teleprompter cues safely excluded from capture, shuttle recordings with J/K/L, and open clips directly in Splicy.",
            "Outputs & Prompter/QC Tabs",
        ),
    ]
}

impl Cabaret {
    /// Builds the full retained Weft Cabaret workspace and renders initial frames/scopes.
    pub fn build(ui: &mut Ui) -> Self {
        let mut studio = CabaretStudio::new_seeded();
        Self::register_commands(ui);

        let root = ui.root();
        ui.set_layout(root, Layout::column());
        Self::build_menubar(ui, root);

        // Top toolbar
        let toolbar = ui.group(
            root,
            Layout::row()
                .gap(8.0)
                .padding_xy(10.0, 4.0)
                .fill_width()
                .shrink(0.0),
        );
        let brand = ui.text(toolbar, "Cabaret Studio", TextStyle::Heading);
        ui.set_color(brand, Some(ui.theme().palette.accent));
        let header_status = ui.text(toolbar, "", TextStyle::Caption);
        ui.set_layout(header_status, Layout::default().grow(1.0));
        let guide_btn = ui.icon_button(toolbar, "question", "Guide (F1)");
        ui.set_tooltip(guide_btn, "Toggle interactive 6-step F1 Coach walkthrough");

        let coach = Coach::new(walkthrough_steps());
        let coach_host = ui.group(
            root,
            Layout::column().padding_xy(10.0, 2.0).fill_width().shrink(0.0),
        );
        let coach_controls = coach.render(ui, coach_host);

        let tabs = ui.tabs(root);
        ui.set_layout(tabs, Layout::default().fill());

        let p_studio = ui.tab_page(tabs, Tab::new("Studio & Mixer").icon("video"));
        let p_mv = ui.tab_page(tabs, Tab::new("Multiview").icon("grid"));
        let p_plugins = ui.tab_page(tabs, Tab::new("Plugins & DSK & 9:16").icon("settings"));
        let p_scopes = ui.tab_page(tabs, Tab::new("Scopes & Color").icon("palette"));
        let p_outputs = ui.tab_page(tabs, Tab::new("Outputs & Multi-Stream").icon("upload"));
        let p_qc = ui.tab_page(tabs, Tab::new("Prompter & QC Player").icon("play"));

        let blank_preview = vec![0xFF0E_1014u32; (UI_PREVIEW_W * UI_PREVIEW_H) as usize];

        // ---- Tab 0: Studio & Mixer -------------------------------------------------
        let sm_scroll = ui.scroll(p_studio);
        ui.set_layout(sm_scroll, Layout::column().gap(6.0).fill());

        let monitors_row = ui.group(sm_scroll, Layout::row().gap(8.0).fill_width().shrink(0.0));

        let prv_card = ui.card(monitors_row, Layout::column().gap(4.0).grow(1.0));
        let preview_caption = ui.text(prv_card, "PREVIEW (GREEN TALLY)", TextStyle::Heading);
        let preview_image = ui.image(
            prv_card,
            UI_PREVIEW_W as usize,
            UI_PREVIEW_H as usize,
            blank_preview.clone(),
        );
        ui.set_label(preview_image, "Preview Canvas");

        let rack_card = ui.card(
            monitors_row,
            Layout::column().gap(4.0).min_width(210.0).shrink(0.0),
        );
        ui.text(rack_card, "Transitions & Deck", TextStyle::Heading);
        let take_btn = ui.icon_button(rack_card, "arrow-right", "TAKE -> Program");
        ui.set_button_style(take_btn, ButtonStyle::Primary);

        let tbar_label = ui.text(rack_card, "T-Bar: 0%", TextStyle::Caption);
        let tbar_slider = ui.slider(rack_card, 0.0, 1.0, 0.0);
        ui.set_step(tbar_slider, 0.05);

        let tr_row = ui.group(rack_card, Layout::row().gap(4.0).wrap().fill_width());
        let mut transition_btns = Vec::new();
        for preset in ActiveTransitionPreset::ALL {
            let b = ui.button(tr_row, preset.label());
            transition_btns.push((b, preset));
        }

        let wipe_row = ui.group(rack_card, Layout::row().gap(4.0).fill_width());
        let wipe_prev_btn = ui.button(wipe_row, "< Wipe");
        let wipe_next_btn = ui.button(wipe_row, "Wipe >");
        let wipe_label = ui.text(rack_card, "", TextStyle::Caption);

        let deck_row = ui.group(rack_card, Layout::row().gap(4.0).fill_width());
        let deck_prev_btn = ui.button(deck_row, "Prev Slide");
        let deck_next_btn = ui.button(deck_row, "Next Slide");
        let deck_status_label = ui.paragraph(rack_card, "");

        let undo_row = ui.group(rack_card, Layout::row().gap(4.0).fill_width());
        let undo_btn = ui.button(undo_row, "Undo");
        let redo_btn = ui.button(undo_row, "Redo");

        let pgm_card = ui.card(monitors_row, Layout::column().gap(4.0).grow(1.0));
        let program_caption = ui.text(pgm_card, "PROGRAM (RED TALLY)", TextStyle::Heading);
        let program_image = ui.image(
            pgm_card,
            UI_PREVIEW_W as usize,
            UI_PREVIEW_H as usize,
            blank_preview.clone(),
        );
        ui.set_label(program_image, "Program Canvas");

        let dock_row = ui.group(sm_scroll, Layout::row().gap(8.0).fill_width().shrink(0.0));

        // Studios list card
        let studios_card = ui.card(dock_row, Layout::column().gap(4.0).min_width(210.0).grow(1.0));
        ui.text(studios_card, "Studios (Scenes)", TextStyle::Heading);
        let studios_list = ui.list_with(
            studios_card,
            Box::new(Rows::default()),
            vec![
                Column::new("Studio", 150.0),
                Column::new("Tally", 54.0),
            ],
        );
        ui.set_layout(studios_list, Layout::default().min_height(120.0).fill_width());
        let st_btns = ui.group(studios_card, Layout::row().gap(4.0).fill_width());
        let cue_preview_btn = ui.button(st_btns, "Cue Preview");
        let take_selected_btn = ui.button(st_btns, "Cut Live");

        // Sources list card
        let sources_card = ui.card(dock_row, Layout::column().gap(4.0).min_width(220.0).grow(1.0));
        ui.text(sources_card, "Studio Sources & Filters", TextStyle::Heading);
        let sources_list = ui.list_with(
            sources_card,
            Box::new(Rows::default()),
            vec![
                Column::new("Source Item", 160.0),
                Column::new("Vis", 48.0),
            ],
        );
        ui.set_layout(sources_list, Layout::default().min_height(120.0).fill_width());
        let src_btns = ui.group(sources_card, Layout::row().gap(4.0).fill_width());
        let toggle_source_vis_btn = ui.button(src_btns, "Toggle Visible");
        let toggle_cam_filter_btn = ui.button(src_btns, "Cam Warm LUT");

        // Audio Mixer card
        let mixer_card = ui.card(dock_row, Layout::column().gap(4.0).min_width(360.0).grow(2.0));
        let mixer_head = ui.group(mixer_card, Layout::row().gap(6.0).fill_width());
        let mx_t = ui.text(mixer_head, "Audio Mixer (R128 / 4x dBTP / Sidechain)", TextStyle::Heading);
        ui.set_layout(mx_t, Layout::default().grow(1.0));
        let audio_pulse_btn = ui.icon_button(mixer_head, "volume", "Sample Audio");

        let mut strip_uis = Vec::new();
        for strip in &studio.audio.strips {
            let row = ui.card(mixer_card, Layout::column().gap(2.0).fill_width().shrink(0.0));
            let top = ui.group(row, Layout::row().gap(6.0).fill_width());
            let mut label_text = strip.name.clone();
            if strip.id == studio.strips.mic_serenade {
                label_text.push_str(" [CLAP]");
            } else if strip.id == studio.strips.desktop_bell {
                label_text.push_str(" [Duck<-Mic]");
            }
            let lbl = ui.text(top, &label_text, TextStyle::Body);
            ui.set_layout(lbl, Layout::default().grow(1.0));
            let mute_btn = ui.button(top, if strip.muted { "Unmute" } else { "Mute" });
            let solo_btn = ui.button(top, "Solo");
            let fader_slider = ui.slider(row, -60.0, 12.0, strip.fader_db);
            ui.set_step(fader_slider, 1.0);
            let meters_label = ui.text(row, "", TextStyle::Mono);
            strip_uis.push(StripUi {
                strip_id: strip.id,
                mute_btn,
                solo_btn,
                fader_slider,
                meters_label,
            });
        }
        let audio_vis_image = ui.image(
            mixer_card,
            300,
            64,
            vec![0xFF06_080Du32; 300 * 64],
        );
        ui.set_label(audio_vis_image, "Audio Spectrum & Goniometer");

        // ---- Tab 1: Multiview ------------------------------------------------------
        let mv_scroll = ui.scroll(p_mv);
        ui.set_layout(mv_scroll, Layout::column().gap(6.0).fill());
        let mv_bar = ui.group(mv_scroll, Layout::row().gap(6.0).wrap().fill_width().shrink(0.0));
        let mut mv_preset_btns = Vec::new();
        for (preset, name) in [
            (MultiviewPreset::Standard8, "Standard (8+2)"),
            (MultiviewPreset::Grid4, "2x2 (4)"),
            (MultiviewPreset::Grid9, "3x3 (9)"),
            (MultiviewPreset::Grid16, "4x4 (16)"),
            (MultiviewPreset::Grid25, "5x5 (25)"),
        ] {
            let b = ui.button(mv_bar, name);
            mv_preset_btns.push((b, preset));
        }
        let mv_safe_areas_check = ui.checkbox(mv_bar, "Safe Areas (90%/80%)", true);
        let mv_status_label = ui.text(mv_scroll, "", TextStyle::Caption);
        let mv_image = ui.image(mv_scroll, 480, 270, vec![0xFF0A_0B10u32; 480 * 270]);
        ui.set_label(mv_image, "Multiview Composite");
        let mv_slot_row = ui.group(mv_scroll, Layout::row().gap(6.0).wrap().fill_width().shrink(0.0));
        let mut mv_slot_btns = Vec::new();
        for idx in 0..4 {
            let b = ui.button(mv_slot_row, &format!("Slot {}", idx + 1));
            mv_slot_btns.push((b, idx));
        }
        let mv_slot_take_btn = ui.button(mv_slot_row, "Take Active Slot");
        ui.set_button_style(mv_slot_take_btn, ButtonStyle::Primary);

        // ---- Tab 2: Plugins, DSK & 9:16 Vertical -----------------------------------
        let pl_scroll = ui.scroll(p_plugins);
        ui.set_layout(pl_scroll, Layout::column().gap(8.0).fill());
        let pl_top = ui.group(pl_scroll, Layout::row().gap(8.0).fill_width().shrink(0.0));

        let dsk_card = ui.card(pl_top, Layout::column().gap(4.0).grow(1.0));
        ui.text(dsk_card, "Downstream Keyer (Pinned Above Transitions)", TextStyle::Heading);
        let mut dsk_uis = Vec::new();
        for ch in &studio.dsk.channels {
            let row = ui.card(dsk_card, Layout::column().gap(2.0).fill_width());
            let head = ui.group(row, Layout::row().gap(6.0).fill_width());
            let lbl = ui.text(head, &ch.name, TextStyle::Body);
            ui.set_layout(lbl, Layout::default().grow(1.0));
            let toggle_btn = ui.button(head, if ch.active { "ON AIR" } else { "OFF" });
            let tie_check = ui.checkbox(row, "Tie to Studio Transition", ch.tie_to_transition);
            let status_label = ui.text(row, "", TextStyle::Caption);
            dsk_uis.push(DskChannelUi {
                channel_id: ch.id,
                toggle_btn,
                tie_check,
                status_label,
            });
        }

        let vert_card = ui.card(pl_top, Layout::column().gap(4.0).shrink(0.0));
        ui.text(vert_card, "9:16 Vertical Canvas (Shorts / TikTok)", TextStyle::Heading);
        let vertical_image = ui.image(
            vert_card,
            UI_VERTICAL_W as usize,
            UI_VERTICAL_H as usize,
            vec![0xFF0A_0A12u32; (UI_VERTICAL_W * UI_VERTICAL_H) as usize],
        );
        ui.set_label(vertical_image, "Vertical 9:16 Preview");
        let vertical_backtrack_btn = ui.button(vert_card, "Save Vertical Backtrack");
        let vertical_summary = ui.paragraph(vert_card, "");

        let fx_card = ui.card(pl_scroll, Layout::column().gap(6.0).fill_width().shrink(0.0));
        ui.text(fx_card, "OBS Essential Plugins Rack", TextStyle::Heading);
        let fx_row1 = ui.group(fx_card, Layout::row().gap(6.0).wrap().fill_width());
        let shader_cycle_btn = ui.button(fx_row1, "Cycle Shader Preset");
        let shader_clear_btn = ui.button(fx_row1, "Shader Off");
        let shader_label = ui.text(fx_row1, "Shader: Off", TextStyle::Caption);
        let warp_3d_check = ui.checkbox(fx_row1, "StreamFX 3D Corner-Pin", false);

        let fx_row2 = ui.group(fx_card, Layout::row().gap(6.0).wrap().fill_width());
        let freeze_btn = ui.button(fx_row2, "Freeze Filter Hold");
        let time_warp_check = ui.checkbox(fx_row2, "TimeWarp Scan", false);
        let time_warp_reset_btn = ui.button(fx_row2, "Reset TimeWarp");
        let delay_slowmo_btn = ui.button(fx_row2, "Delay 0.5x Slow-Mo");
        let source_tap_record_btn = ui.button(fx_row2, "Start Source-Record Tap");

        let cap_card = ui.card(pl_scroll, Layout::column().gap(4.0).fill_width().shrink(0.0));
        ui.text(cap_card, "Closed Captions (CEA-608/708 + SRT/VTT), Macros & CabaretLink", TextStyle::Heading);
        let cap_row = ui.group(cap_card, Layout::row().gap(6.0).wrap().fill_width());
        let captions_burn_check = ui.checkbox(cap_row, "Burn-In Captions", false);
        let captions_push_btn = ui.button(cap_row, "Push Caption Line");
        let macro_spindle_btn = ui.button(cap_row, "Macro: Focus Spindle");
        let macro_main_btn = ui.button(cap_row, "Macro: Focus Cabaret");
        let link_toggle_btn = ui.button(cap_row, "Toggle CabaretLink LAN");
        let captions_status = ui.paragraph(cap_card, "");
        let macro_status = ui.paragraph(cap_card, "");
        let link_status = ui.paragraph(cap_card, "");

        // ---- Tab 3: Scopes & Color -------------------------------------------------
        let sc_scroll = ui.scroll(p_scopes);
        ui.set_layout(sc_scroll, Layout::column().gap(6.0).fill());
        let cs_row = ui.group(sc_scroll, Layout::row().gap(6.0).wrap().fill_width().shrink(0.0));
        ui.label(cs_row, "Canvas Color Space:");
        let mut color_space_btns = Vec::new();
        for space in crate::studio::CanvasColorPipeline::SPACES {
            let b = ui.button(cs_row, crate::studio::CanvasColorPipeline::space_label(space));
            color_space_btns.push((b, space));
        }

        let film_row = ui.group(sc_scroll, Layout::row().gap(6.0).wrap().fill_width().shrink(0.0));
        let film_stock_next_btn = ui.button(film_row, "Next Film Stock LUT");
        let film_stock_clear_btn = ui.button(film_row, "Film LUT: Off");
        let film_stock_label = ui.text(film_row, "Film Stock: None", TextStyle::Caption);

        let sk_row = ui.group(sc_scroll, Layout::row().gap(6.0).wrap().fill_width().shrink(0.0));
        ui.label(sk_row, "Broadcast Scope:");
        let mut scope_kind_btns = Vec::new();
        for kind in ScopeKind::ALL {
            let b = ui.button(sk_row, kind.label());
            scope_kind_btns.push((b, kind));
        }

        let sc_imgs = ui.group(sc_scroll, Layout::row().gap(8.0).fill_width().shrink(0.0));
        let scope_image = ui.image(
            sc_imgs,
            UI_PREVIEW_W as usize,
            UI_PREVIEW_H as usize,
            blank_preview.clone(),
        );
        ui.set_label(scope_image, "Broadcast Video Scope");
        let scope_ref_image = ui.image(
            sc_imgs,
            UI_PREVIEW_W as usize,
            UI_PREVIEW_H as usize,
            blank_preview,
        );
        ui.set_label(scope_ref_image, "Graded Program Reference");
        let scope_metrics_label = ui.paragraph(sc_scroll, "");

        // ---- Tab 4: Outputs & Multi-Stream -----------------------------------------
        let out_scroll = ui.scroll(p_outputs);
        ui.set_layout(out_scroll, Layout::column().gap(6.0).fill());
        let tx_row = ui.group(out_scroll, Layout::row().gap(6.0).wrap().fill_width().shrink(0.0));
        let stream_toggle_btn = ui.icon_button(tx_row, "upload", "Start Streaming");
        ui.set_button_style(stream_toggle_btn, ButtonStyle::Primary);
        let record_toggle_btn = ui.icon_button(tx_row, "save", "Start Recording");
        let record_pause_btn = ui.button(tx_row, "Pause Record");
        let chapter_add_btn = ui.button(tx_row, "Add Chapter");
        let split_file_btn = ui.button(tx_row, "Split MP4");
        let replay_save_btn = ui.button(tx_row, "Save 15s Replay");
        let vcam_toggle_btn = ui.button(tx_row, "Start Virtual Cam");

        let dest_card = ui.card(out_scroll, Layout::column().gap(4.0).fill_width().shrink(0.0));
        ui.text(dest_card, "Multi-Stream Destinations (Shibari Key Isolation)", TextStyle::Heading);
        let mut target_checks = Vec::new();
        for t in &studio.router.targets {
            let c = ui.checkbox(dest_card, &t.name, t.enabled);
            target_checks.push((c, t.id));
        }

        let grp_card = ui.card(out_scroll, Layout::column().gap(4.0).fill_width().shrink(0.0));
        ui.text(grp_card, "Outputs, Encoder Groups & Compliance", TextStyle::Heading);
        let outputs_summary_label = ui.paragraph(grp_card, "");
        let encoder_groups_label = ui.paragraph(grp_card, "");
        let log_tail_label = ui.text(grp_card, "", TextStyle::Mono);

        // ---- Tab 5: Prompter & QC Player -------------------------------------------
        let qc_scroll = ui.scroll(p_qc);
        ui.set_layout(qc_scroll, Layout::column().gap(8.0).fill());

        let prm_card = ui.card(qc_scroll, Layout::column().gap(4.0).fill_width().shrink(0.0));
        ui.text(prm_card, "Teleprompter & Voice Tracking", TextStyle::Heading);
        let prompter_badge = ui.text(
            prm_card,
            "CAPTURE POLICY: HIDE_FROM_CAPTURE (Excluded from Screen & Silk Capture)",
            TextStyle::Caption,
        );
        ui.set_color(prompter_badge, Some(ui.theme().palette.accent));
        let prm_row = ui.group(prm_card, Layout::row().gap(6.0).wrap().fill_width());
        let prompter_wpm_down_btn = ui.button(prm_row, "WPM -20");
        let prompter_wpm_up_btn = ui.button(prm_row, "WPM +20");
        let prompter_mirror_h_check = ui.checkbox(prm_row, "Mirror H", false);
        let prompter_mirror_v_check = ui.checkbox(prm_row, "Mirror V", false);
        let voice_row = ui.group(prm_card, Layout::row().gap(6.0).fill_width());
        let prompter_voice_field = ui.field(
            voice_row,
            "Welcome to Cabaret on SexOS",
            "Spoken words for voice alignment...",
        );
        ui.set_layout(prompter_voice_field, Layout::default().grow(1.0));
        let prompter_voice_track_btn = ui.button(voice_row, "Track Spoken Words");
        let prompter_script_label = ui.paragraph(prm_card, "");

        let qc_card = ui.card(qc_scroll, Layout::column().gap(4.0).fill_width().shrink(0.0));
        ui.text(qc_card, "QC Shuttle Player (-2x..+4x) & Splicy NLE Handoff", TextStyle::Heading);
        let qc_row = ui.group(qc_card, Layout::row().gap(6.0).wrap().fill_width());
        let qc_j_btn = ui.button(qc_row, "J (-Reverse)");
        let qc_k_btn = ui.button(qc_row, "K (Pause)");
        let qc_l_btn = ui.button(qc_row, "L (+Forward)");
        let qc_step_back_btn = ui.button(qc_row, "-1 Frame");
        let qc_step_fwd_btn = ui.button(qc_row, "+1 Frame");
        let qc_mark_in_btn = ui.button(qc_row, "Mark In");
        let qc_mark_out_btn = ui.button(qc_row, "Mark Out");
        let qc_status_label = ui.paragraph(qc_card, "");
        let open_splicy_btn = ui.icon_button(qc_card, "cut", "Open Last Recording in Splicy");
        ui.set_button_style(open_splicy_btn, ButtonStyle::Primary);
        let splicy_summary_label = ui.paragraph(qc_card, "");

        // Persistent bottom Tally Status bar
        let tally_bar = ui.text(root, "", TextStyle::Mono);
        ui.set_layout(
            tally_bar,
            Layout::default().padding_xy(10.0, 4.0).fill_width().shrink(0.0),
        );

        let initial_program = render_program_frame(&mut studio, UI_PREVIEW_W, UI_PREVIEW_H);

        let mut app = Self {
            studio,
            active_tab: CabaretTab::StudioMixer,
            last_program_frame: initial_program,
            coach,
            coach_controls,
            tabs,
            guide_btn,
            header_status,
            tally_bar,
            preview_image,
            preview_caption,
            program_image,
            program_caption,
            take_btn,
            undo_btn,
            redo_btn,
            tbar_slider,
            tbar_label,
            transition_btns,
            wipe_prev_btn,
            wipe_next_btn,
            wipe_label,
            deck_prev_btn,
            deck_next_btn,
            deck_status_label,
            studios_list,
            cue_preview_btn,
            take_selected_btn,
            sources_list,
            toggle_source_vis_btn,
            toggle_cam_filter_btn,
            audio_pulse_btn,
            audio_vis_image,
            strip_uis,
            mv_preset_btns,
            mv_safe_areas_check,
            mv_image,
            mv_status_label,
            mv_slot_btns,
            mv_slot_take_btn,
            mv_selected_slot: 0,
            dsk_uis,
            vertical_image,
            vertical_backtrack_btn,
            vertical_summary,
            shader_cycle_btn,
            shader_clear_btn,
            shader_label,
            warp_3d_check,
            freeze_btn,
            time_warp_check,
            time_warp_reset_btn,
            delay_slowmo_btn,
            source_tap_record_btn,
            captions_burn_check,
            captions_push_btn,
            captions_status,
            macro_spindle_btn,
            macro_main_btn,
            macro_status,
            link_toggle_btn,
            link_status,
            color_space_btns,
            film_stock_next_btn,
            film_stock_clear_btn,
            film_stock_label,
            scope_kind_btns,
            scope_image,
            scope_ref_image,
            scope_metrics_label,
            stream_toggle_btn,
            record_toggle_btn,
            record_pause_btn,
            chapter_add_btn,
            split_file_btn,
            replay_save_btn,
            vcam_toggle_btn,
            target_checks,
            outputs_summary_label,
            encoder_groups_label,
            log_tail_label,
            prompter_badge,
            prompter_wpm_down_btn,
            prompter_wpm_up_btn,
            prompter_mirror_h_check,
            prompter_mirror_v_check,
            prompter_voice_field,
            prompter_voice_track_btn,
            prompter_script_label,
            qc_j_btn,
            qc_k_btn,
            qc_l_btn,
            qc_step_back_btn,
            qc_step_fwd_btn,
            qc_mark_in_btn,
            qc_mark_out_btn,
            qc_status_label,
            open_splicy_btn,
            splicy_summary_label,
        };

        app.refresh_all(ui);
        app
    }

    fn register_commands(ui: &mut Ui) {
        let cmds = [
            Command::new("cabaret.take", "Take (Preview -> Program)")
                .shortcut("Ctrl+Enter")
                .icon("arrow-right"),
            Command::new("cabaret.record", "Start / Stop Recording")
                .shortcut("Ctrl+R")
                .icon("save"),
            Command::new("cabaret.stream", "Start / Stop Streaming")
                .shortcut("Ctrl+Shift+L")
                .icon("upload"),
            Command::new("cabaret.replay", "Save Replay Buffer")
                .shortcut("Ctrl+Shift+B")
                .icon("refresh"),
            Command::new("cabaret.vcam", "Toggle Cabaret Virtual Camera")
                .icon("video"),
            Command::new("cabaret.undo", "Undo Scene Action")
                .shortcut("Ctrl+U")
                .icon("undo"),
            Command::new("cabaret.redo", "Redo Scene Action")
                .shortcut("Ctrl+Y")
                .icon("redo"),
            Command::new("cabaret.tab.studio", "Studio & Mixer Tab").shortcut("Ctrl+1"),
            Command::new("cabaret.tab.multiview", "Multiview Tab").shortcut("Ctrl+2"),
            Command::new("cabaret.tab.plugins", "Plugins, DSK & 9:16 Tab").shortcut("Ctrl+3"),
            Command::new("cabaret.tab.scopes", "Scopes & Color Tab").shortcut("Ctrl+4"),
            Command::new("cabaret.tab.outputs", "Outputs & Multi-Stream Tab").shortcut("Ctrl+5"),
            Command::new("cabaret.tab.prompter", "Prompter & QC Player Tab").shortcut("Ctrl+6"),
            Command::new("cabaret.guide", "Interactive Studio Guide")
                .shortcut("F1")
                .icon("question"),
        ];
        for c in cmds {
            ui.commands.add(c);
        }
    }

    fn build_menubar(ui: &mut Ui, root: Id) {
        let file = Menu::new("File")
            .command("cabaret.record")
            .command("cabaret.replay")
            .separator()
            .command("cabaret.undo")
            .command("cabaret.redo");
        let studio = Menu::new("Studio")
            .command("cabaret.take")
            .command("cabaret.tab.studio")
            .command("cabaret.tab.multiview");
        let sources = Menu::new("Sources")
            .command("cabaret.tab.plugins")
            .command("cabaret.tab.scopes");
        let plugins = Menu::new("Plugins")
            .command("cabaret.tab.plugins")
            .command("cabaret.tab.prompter");
        let outputs = Menu::new("Outputs")
            .command("cabaret.stream")
            .command("cabaret.record")
            .command("cabaret.vcam")
            .command("cabaret.tab.outputs");
        let help = Menu::new("Help").command("cabaret.guide");
        ui.menubar(root, vec![file, studio, sources, plugins, outputs, help]);
    }

    pub fn set_tab(&mut self, ui: &mut Ui, tab: CabaretTab) {
        self.active_tab = tab;
        ui.set_active_tab(self.tabs, tab as usize);
        self.refresh_active_tab_visuals(ui);
        self.sync_chrome_labels(ui);
    }

    pub fn toggle_coach(&mut self, ui: &mut Ui) {
        self.coach.toggle();
        self.coach.sync(ui, &self.coach_controls);
    }

    /// Processes a Weft UI [`Response`] and keeps Cabaret studio state & visuals synced.
    pub fn on(&mut self, ui: &mut Ui, r: &Response) {
        if self.coach.on(ui, &self.coach_controls, r) {
            return;
        }
        match r {
            Response::Clicked(id) => self.on_clicked(ui, *id),
            Response::Toggled(id, on) => self.on_toggled(ui, *id, *on),
            Response::Changed(id) => self.on_changed(ui, *id),
            Response::Selected(id) if *id == self.studios_list => {
                if let Some(row) = ui.selected_row(self.studios_list) {
                    if let Some(st) = self.studio.collection.studios.get(row) {
                        self.studio.set_preview_studio(st.id);
                        self.refresh_all(ui);
                    }
                }
            }
            Response::Activated(id, row) if *id == self.studios_list => {
                if let Some(st) = self.studio.collection.studios.get(*row) {
                    self.studio.set_preview_studio(st.id);
                    self.studio.trigger_take(true);
                    self.refresh_all(ui);
                }
            }
            Response::Tab(id, TabEvent::Selected(idx)) if *id == self.tabs => {
                self.active_tab = CabaretTab::from_index(*idx);
                self.refresh_active_tab_visuals(ui);
                self.sync_chrome_labels(ui);
            }
            Response::Command(cmd) => self.on_command(ui, cmd.as_str()),
            _ => {}
        }
    }

    fn on_command(&mut self, ui: &mut Ui, cmd: &str) {
        match cmd {
            "cabaret.take" => {
                self.studio.trigger_take(true);
                ui.set_value(self.tbar_slider, 0.0);
                self.refresh_all(ui);
            }
            "cabaret.record" => {
                if self.studio.recording.state == RecordingState::Recording {
                    self.studio.stop_recording();
                } else {
                    self.studio.start_recording();
                }
                self.refresh_all(ui);
            }
            "cabaret.stream" => {
                if self.studio.stream_active {
                    self.studio.stop_streaming();
                } else {
                    self.studio.start_streaming();
                }
                self.refresh_all(ui);
            }
            "cabaret.replay" => {
                let _ = self.studio.save_replay(15.0);
                self.refresh_all(ui);
            }
            "cabaret.vcam" => {
                self.studio.toggle_virtual_cam();
                self.refresh_all(ui);
            }
            "cabaret.undo" => {
                let _ = self.studio.undo.undo(&mut self.studio.collection);
                self.refresh_all(ui);
            }
            "cabaret.redo" => {
                let _ = self.studio.undo.redo(&mut self.studio.collection);
                self.refresh_all(ui);
            }
            "cabaret.tab.studio" => self.set_tab(ui, CabaretTab::StudioMixer),
            "cabaret.tab.multiview" => self.set_tab(ui, CabaretTab::Multiview),
            "cabaret.tab.plugins" => self.set_tab(ui, CabaretTab::PluginsVertical),
            "cabaret.tab.scopes" => self.set_tab(ui, CabaretTab::ScopesColor),
            "cabaret.tab.outputs" => self.set_tab(ui, CabaretTab::OutputsMultiStream),
            "cabaret.tab.prompter" => self.set_tab(ui, CabaretTab::PrompterQc),
            "cabaret.guide" => self.toggle_coach(ui),
            _ => {}
        }
    }

    fn on_clicked(&mut self, ui: &mut Ui, id: Id) {
        if id == self.guide_btn {
            self.toggle_coach(ui);
            return;
        }
        if id == self.take_btn {
            self.studio.trigger_take(true);
            ui.set_value(self.tbar_slider, 0.0);
            self.refresh_all(ui);
            return;
        }
        if id == self.undo_btn {
            let _ = self.studio.undo.undo(&mut self.studio.collection);
            self.refresh_all(ui);
            return;
        }
        if id == self.redo_btn {
            let _ = self.studio.undo.redo(&mut self.studio.collection);
            self.refresh_all(ui);
            return;
        }
        if let Some(&(_, preset)) = self.transition_btns.iter().find(|(b, _)| *b == id) {
            self.studio.select_transition_preset(preset);
            self.refresh_all(ui);
            return;
        }
        if id == self.wipe_prev_btn {
            self.studio.cycle_wipe_pattern(-1);
            self.studio.select_transition_preset(ActiveTransitionPreset::Wipe29);
            self.refresh_all(ui);
            return;
        }
        if id == self.wipe_next_btn {
            self.studio.cycle_wipe_pattern(1);
            self.studio.select_transition_preset(ActiveTransitionPreset::Wipe29);
            self.refresh_all(ui);
            return;
        }
        if id == self.deck_prev_btn {
            self.studio.presentation_prev_step();
            self.refresh_all(ui);
            return;
        }
        if id == self.deck_next_btn {
            self.studio.presentation_next_step();
            self.refresh_all(ui);
            return;
        }
        if id == self.cue_preview_btn {
            if let Some(row) = ui.selected_row(self.studios_list) {
                if let Some(st) = self.studio.collection.studios.get(row) {
                    self.studio.set_preview_studio(st.id);
                    self.refresh_all(ui);
                }
            }
            return;
        }
        if id == self.take_selected_btn {
            if let Some(row) = ui.selected_row(self.studios_list) {
                if let Some(st) = self.studio.collection.studios.get(row) {
                    self.studio.set_preview_studio(st.id);
                    self.studio.trigger_take(true);
                    self.refresh_all(ui);
                }
            }
            return;
        }
        if id == self.toggle_source_vis_btn {
            let preview_id = self.studio.studio_mode.preview_studio;
            if let Some(row) = ui.selected_row(self.sources_list) {
                if let Some(item) = self
                    .studio
                    .collection
                    .studio(preview_id)
                    .and_then(|s| s.items.get(row))
                    .cloned()
                {
                    let _ = self.studio.undo.execute(
                        &mut self.studio.collection,
                        cabaret_core::scene::UndoCommand::SetItemVisibility {
                            studio_id: preview_id,
                            item_id: item.id,
                            old_visible: item.visible,
                            new_visible: !item.visible,
                        },
                    );
                    self.refresh_all(ui);
                }
            }
            return;
        }
        if id == self.toggle_cam_filter_btn {
            if let Some(cam) = self.studio.collection.source_mut(self.studio.sources.uvc_camera) {
                if let Some(entry) = cam.filters.entries.get_mut(1) {
                    entry.enabled = !entry.enabled;
                }
            }
            self.refresh_all(ui);
            return;
        }
        if id == self.audio_pulse_btn {
            self.studio.tick_audio(0.28, 0.25, 4800);
            self.refresh_all(ui);
            return;
        }

        for su in self.strip_uis.clone() {
            if id == su.mute_btn {
                if let Some(strip) = self.studio.audio.strip_mut(su.strip_id) {
                    strip.muted = !strip.muted;
                }
                self.studio.tick_audio(0.18, 0.25, 2400);
                self.refresh_all(ui);
                return;
            }
            if id == su.solo_btn {
                if let Some(strip) = self.studio.audio.strip_mut(su.strip_id) {
                    strip.solo = !strip.solo;
                }
                self.studio.tick_audio(0.18, 0.25, 2400);
                self.refresh_all(ui);
                return;
            }
        }

        if let Some(&(_, preset)) = self.mv_preset_btns.iter().find(|(b, _)| *b == id) {
            self.studio.multiview_preset = preset;
            self.refresh_all(ui);
            return;
        }
        if let Some(&(_, idx)) = self.mv_slot_btns.iter().find(|(b, _)| *b == id) {
            self.mv_selected_slot = idx;
            if let Some(st) = self.studio.collection.studios.get(idx) {
                self.studio.set_preview_studio(st.id);
            }
            self.refresh_all(ui);
            return;
        }
        if id == self.mv_slot_take_btn {
            if let Some(st) = self.studio.collection.studios.get(self.mv_selected_slot) {
                self.studio.set_preview_studio(st.id);
                self.studio.trigger_take(true);
                self.refresh_all(ui);
            }
            return;
        }

        for du in self.dsk_uis.clone() {
            if id == du.toggle_btn {
                let next = !self
                    .studio
                    .dsk
                    .channel(du.channel_id)
                    .is_some_and(|c| c.active);
                self.studio.dsk.set_active(du.channel_id, next);
                // Finish fade/slide transition visibly for UI clicks
                self.studio.dsk.tick(500.0);
                self.refresh_all(ui);
                return;
            }
        }

        if id == self.vertical_backtrack_btn {
            if let Some(sec) = self
                .studio
                .secondary_canvases
                .canvas_mut(self.studio.vertical_canvas_id)
            {
                let pts = (self.studio.clock_ms as i64) * 1000;
                sec.push_encoded_packet(cabaret_core::outputs::EncodedPacket {
                    pts_us: pts,
                    dts_us: pts,
                    is_keyframe: true,
                    kind: cabaret_core::outputs::MediaKind::Video,
                    track_index: 0,
                    payload: vec![0x77; 64],
                });
                let _ = sec.save_backtrack(15.0);
            }
            self.refresh_all(ui);
            return;
        }

        if id == self.shader_cycle_btn {
            let n = ShaderPreset::all_defaults().len();
            self.studio.active_shader_index = Some(
                self.studio
                    .active_shader_index
                    .map_or(0, |i| (i + 1) % n),
            );
            self.refresh_all(ui);
            return;
        }
        if id == self.shader_clear_btn {
            self.studio.active_shader_index = None;
            self.refresh_all(ui);
            return;
        }
        if id == self.freeze_btn {
            match self.studio.freeze_filter.mode {
                cabaret_plugins::source_record::FreezeMode::Live => {
                    self.studio.freeze_filter.trigger_instant();
                }
                _ => self.studio.freeze_filter.unfreeze(),
            }
            self.refresh_all(ui);
            return;
        }
        if id == self.time_warp_reset_btn {
            self.studio.time_warp.reset();
            self.refresh_all(ui);
            return;
        }
        if id == self.delay_slowmo_btn {
            if self.studio.delay_buffer.live_follow {
                self.studio.delay_buffer.set_speed(0.5);
            } else {
                self.studio.delay_buffer.go_live();
            }
            self.refresh_all(ui);
            return;
        }
        if id == self.source_tap_record_btn {
            if self.studio.source_tap.recording.state == RecordingState::Recording {
                self.studio.source_tap.stop();
            } else {
                self.studio.source_tap.start();
            }
            self.refresh_all(ui);
            return;
        }
        if id == self.captions_push_btn {
            let count = self.studio.captions.segments().len() + 1;
            self.studio.push_caption_line(
                Some("Guest"),
                &format!("Segment #{count}: Live captions encoded to CEA-608/708 + SRT/VTT."),
            );
            self.refresh_all(ui);
            return;
        }
        if id == self.macro_spindle_btn {
            self.studio.evaluate_macros_for_window("Spindle Terminal - SexOS");
            self.refresh_all(ui);
            return;
        }
        if id == self.macro_main_btn {
            self.studio.evaluate_macros_for_window("Cabaret Main Control");
            self.refresh_all(ui);
            return;
        }
        if id == self.link_toggle_btn {
            self.studio.link_enabled = !self.studio.link_enabled;
            self.refresh_all(ui);
            return;
        }

        if let Some(&(_, cs)) = self.color_space_btns.iter().find(|(b, _)| *b == id) {
            self.studio.color_pipeline.canvas_space = cs;
            self.refresh_all(ui);
            return;
        }
        if id == self.film_stock_next_btn {
            let all = FilmStock::ALL;
            let next = match self.studio.color_pipeline.film_stock {
                None => 0,
                Some(cur) => {
                    let pos = all.iter().position(|s| *s == cur).unwrap_or(0);
                    (pos + 1) % all.len()
                }
            };
            self.studio.color_pipeline.film_stock = Some(all[next]);
            self.refresh_all(ui);
            return;
        }
        if id == self.film_stock_clear_btn {
            self.studio.color_pipeline.film_stock = None;
            self.refresh_all(ui);
            return;
        }
        if let Some(&(_, sk)) = self.scope_kind_btns.iter().find(|(b, _)| *b == id) {
            self.studio.color_pipeline.scope_kind = sk;
            self.refresh_all(ui);
            return;
        }

        if id == self.stream_toggle_btn {
            if self.studio.stream_active {
                self.studio.stop_streaming();
            } else {
                self.studio.start_streaming();
            }
            self.refresh_all(ui);
            return;
        }
        if id == self.record_toggle_btn {
            if self.studio.recording.state == RecordingState::Recording {
                self.studio.stop_recording();
            } else {
                self.studio.start_recording();
            }
            self.refresh_all(ui);
            return;
        }
        if id == self.record_pause_btn {
            self.studio.toggle_record_pause();
            self.refresh_all(ui);
            return;
        }
        if id == self.chapter_add_btn {
            let n = self.studio.recording.chapters.len() + 1;
            self.studio.add_recording_chapter(&format!("Chapter {n}"));
            self.refresh_all(ui);
            return;
        }
        if id == self.split_file_btn {
            self.studio.split_recording_file();
            self.refresh_all(ui);
            return;
        }
        if id == self.replay_save_btn {
            let _ = self.studio.save_replay(15.0);
            self.refresh_all(ui);
            return;
        }
        if id == self.vcam_toggle_btn {
            self.studio.toggle_virtual_cam();
            self.refresh_all(ui);
            return;
        }

        if id == self.prompter_wpm_down_btn {
            self.studio.teleprompter.scroll_wpm =
                (self.studio.teleprompter.scroll_wpm - 20.0).max(40.0);
            self.refresh_all(ui);
            return;
        }
        if id == self.prompter_wpm_up_btn {
            self.studio.teleprompter.scroll_wpm =
                (self.studio.teleprompter.scroll_wpm + 20.0).min(300.0);
            self.refresh_all(ui);
            return;
        }
        if id == self.prompter_voice_track_btn {
            let spoken = String::from(ui.text_of(self.prompter_voice_field));
            let words: Vec<&str> = spoken.split_whitespace().collect();
            self.studio.teleprompter.update_voice_match(&words);
            self.refresh_all(ui);
            return;
        }
        if id == self.qc_j_btn {
            self.studio.qc_player.shuttle_j();
            self.studio.qc_player.advance(500_000);
            self.refresh_all(ui);
            return;
        }
        if id == self.qc_k_btn {
            self.studio.qc_player.shuttle_k();
            self.refresh_all(ui);
            return;
        }
        if id == self.qc_l_btn {
            self.studio.qc_player.shuttle_l();
            self.studio.qc_player.advance(500_000);
            self.refresh_all(ui);
            return;
        }
        if id == self.qc_step_back_btn {
            self.studio.qc_player.frame_step(-1);
            self.refresh_all(ui);
            return;
        }
        if id == self.qc_step_fwd_btn {
            self.studio.qc_player.frame_step(1);
            self.refresh_all(ui);
            return;
        }
        if id == self.qc_mark_in_btn {
            self.studio.qc_player.mark_in();
            self.refresh_all(ui);
            return;
        }
        if id == self.qc_mark_out_btn {
            self.studio.qc_player.mark_out();
            self.refresh_all(ui);
            return;
        }
        if id == self.open_splicy_btn {
            let _ = self.studio.open_last_recording_in_splicy();
            self.refresh_all(ui);
        }
    }

    fn on_toggled(&mut self, ui: &mut Ui, id: Id, on: bool) {
        if id == self.mv_safe_areas_check {
            self.studio.multiview_show_safe_areas = on;
            self.refresh_all(ui);
            return;
        }
        for du in &self.dsk_uis {
            if id == du.tie_check {
                if let Some(ch) = self.studio.dsk.channel_mut(du.channel_id) {
                    ch.tie_to_transition = on;
                    if on {
                        ch.tied_armed = Some(!ch.active);
                    } else {
                        ch.tied_armed = None;
                    }
                }
                self.refresh_all(ui);
                return;
            }
        }
        if id == self.warp_3d_check {
            self.studio.warp_3d_enabled = on;
            if on {
                self.studio.corner_pin = CornerPinQuad {
                    top_left: [0.06, 0.04],
                    top_right: [0.94, 0.08],
                    bottom_right: [0.98, 0.96],
                    bottom_left: [0.02, 0.92],
                };
            }
            self.refresh_all(ui);
            return;
        }
        if id == self.time_warp_check {
            self.studio.time_warp_enabled = on;
            self.refresh_all(ui);
            return;
        }
        if id == self.captions_burn_check {
            self.studio.burn_in_captions = on;
            self.refresh_all(ui);
            return;
        }
        for &(cid, dest_id) in &self.target_checks {
            if cid == id {
                if let Some(t) = self.studio.router.targets.iter_mut().find(|t| t.id == dest_id) {
                    t.enabled = on;
                }
                self.refresh_all(ui);
                return;
            }
        }
        if id == self.prompter_mirror_h_check {
            self.studio.teleprompter.mirror_h = on;
            self.refresh_all(ui);
            return;
        }
        if id == self.prompter_mirror_v_check {
            self.studio.teleprompter.mirror_v = on;
            self.refresh_all(ui);
        }
    }

    fn on_changed(&mut self, ui: &mut Ui, id: Id) {
        if id == self.tbar_slider {
            let v = ui.value(self.tbar_slider);
            if self.studio.set_t_bar(v).is_some() {
                ui.set_value(self.tbar_slider, 0.0);
            }
            self.refresh_all(ui);
            return;
        }
        for su in &self.strip_uis {
            if id == su.fader_slider {
                let db = ui.value(su.fader_slider);
                if let Some(strip) = self.studio.audio.strip_mut(su.strip_id) {
                    strip.fader_db = db;
                }
                self.studio.tick_audio(0.18, 0.25, 2400);
                self.refresh_all(ui);
                return;
            }
        }
    }

    /// Re-renders active tab visual surfaces and syncs all status widgets cleanly.
    pub fn refresh_all(&mut self, ui: &mut Ui) {
        self.last_program_frame = render_program_frame(&mut self.studio, UI_PREVIEW_W, UI_PREVIEW_H);
        self.sync_chrome_labels(ui);
        self.sync_studio_tab(ui);
        self.sync_plugins_controls(ui);
        self.sync_color_controls(ui);
        self.sync_outputs_controls(ui);
        self.sync_prompter_controls(ui);
        self.refresh_active_tab_visuals(ui);
    }

    fn refresh_active_tab_visuals(&mut self, ui: &mut Ui) {
        match self.active_tab {
            CabaretTab::StudioMixer => {
                let prv = render_preview_frame(&mut self.studio, UI_PREVIEW_W, UI_PREVIEW_H);
                ui.set_image(
                    self.preview_image,
                    UI_PREVIEW_W as usize,
                    UI_PREVIEW_H as usize,
                    fb_to_weft_argb(&prv),
                );
                ui.set_image(
                    self.program_image,
                    UI_PREVIEW_W as usize,
                    UI_PREVIEW_H as usize,
                    fb_to_weft_argb(&self.last_program_frame),
                );
                let vis = render_audio_visualizer_fb(&mut self.studio, 300, 64);
                ui.set_image(self.audio_vis_image, 300, 64, fb_to_weft_argb(&vis));
            }
            CabaretTab::Multiview => {
                let mv = render_multiview_frame(&mut self.studio, 480, 270);
                ui.set_image(self.mv_image, 480, 270, fb_to_weft_argb(&mv));
            }
            CabaretTab::PluginsVertical => {
                let vert = render_vertical_preview(&mut self.studio, UI_VERTICAL_W, UI_VERTICAL_H);
                ui.set_image(
                    self.vertical_image,
                    UI_VERTICAL_W as usize,
                    UI_VERTICAL_H as usize,
                    fb_to_weft_argb(&vert),
                );
            }
            CabaretTab::ScopesColor => {
                let scope_argb =
                    render_scope_argb(&self.studio, &self.last_program_frame, UI_PREVIEW_W, UI_PREVIEW_H);
                ui.set_image(
                    self.scope_image,
                    UI_PREVIEW_W as usize,
                    UI_PREVIEW_H as usize,
                    argb_fit(
                        &scope_argb,
                        UI_PREVIEW_W as usize,
                        UI_PREVIEW_H as usize,
                        UI_PREVIEW_W as usize,
                        UI_PREVIEW_H as usize,
                    ),
                );
                ui.set_image(
                    self.scope_ref_image,
                    UI_PREVIEW_W as usize,
                    UI_PREVIEW_H as usize,
                    fb_to_weft_argb(&self.last_program_frame),
                );
            }
            CabaretTab::OutputsMultiStream | CabaretTab::PrompterQc => return,
        }
        ui.invalidate();
    }

    fn sync_chrome_labels(&self, ui: &mut Ui) {
        let pgm_name = self.studio.studio_name(self.studio.studio_mode.program_studio);
        let prv_name = self.studio.studio_name(self.studio.studio_mode.preview_studio);
        let stream_txt = if self.studio.stream_active { "LIVE" } else { "OFFLINE" };
        let rec_txt = match self.studio.recording.state {
            RecordingState::Recording => "REC",
            RecordingState::Paused => "PAUSED",
            _ => "IDLE",
        };
        let vcam_txt = if self.studio.virtual_cam.active { "ON" } else { "OFF" };

        ui.set_text(
            self.header_status,
            &format!(
                "{} | {} | {} | StreamDeck: {} attached",
                pgm_name,
                self.studio.profile.name,
                if self.studio.capture_exclusion.cabaret_ui_hidden {
                    "HIDE_FROM_CAPTURE"
                } else {
                    "CAPTURED"
                },
                self.studio.attached_streamdecks.len()
            ),
        );
        ui.set_text(
            self.tally_bar,
            &format!(
                "PROGRAM: {pgm_name} | PREVIEW: {prv_name} | STREAM: {stream_txt} | REC: {rec_txt} (6-ch HybridMP4) | VCAM: {} [{vcam_txt}] | CAPTURE: HIDE_FROM_CAPTURE",
                self.studio.virtual_cam.device_name
            ),
        );
    }

    fn sync_studio_tab(&self, ui: &mut Ui) {
        let prv = self.studio.studio_mode.preview_studio;
        let pgm = self.studio.studio_mode.program_studio;
        ui.set_text(
            self.preview_caption,
            &format!("PREVIEW (GREEN TALLY): {}", self.studio.studio_name(prv)),
        );
        ui.set_text(
            self.program_caption,
            &format!("PROGRAM (RED TALLY): {}", self.studio.studio_name(pgm)),
        );
        ui.set_text(
            self.tbar_label,
            &format!("T-Bar: {:.0}%", self.studio.studio_mode.t_bar * 100.0),
        );
        for &(btn, preset) in &self.transition_btns {
            ui.set_button_style(
                btn,
                if preset == self.studio.transition_preset {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                },
            );
        }
        ui.set_text(
            self.wipe_label,
            &format!(
                "Wipe #{}/{}: {:?}",
                self.studio.wipe_pattern_index + 1,
                29,
                self.studio.current_wipe_pattern()
            ),
        );

        let (si, stot, bi, btot) = self.studio.presentation.state();
        ui.set_text(
            self.deck_status_label,
            &format!(
                "Deck Slide {}/{} Step {}/{}\nTitle: {}\nNotes: {}",
                si + 1,
                stot,
                bi,
                btot,
                self.studio.presentation.slide_title(),
                self.studio.presentation.presenter_notes()
            ),
        );

        ui.set_enabled(self.undo_btn, self.studio.undo.can_undo());
        ui.set_enabled(self.redo_btn, self.studio.undo.can_redo());

        // Populate Studios list
        let mut srows = Rows::default();
        for st in &self.studio.collection.studios {
            let tally = if st.id == pgm {
                "PGM"
            } else if st.id == prv {
                "PRV"
            } else {
                ""
            };
            srows.0.push(vec![st.name.clone(), String::from(tally)]);
        }
        let prev_sel = ui.selected_row(self.studios_list);
        ui.set_rows(self.studios_list, srows);
        if let Some(sel) = prev_sel {
            ui.select_row(self.studios_list, Some(sel));
        }

        // Populate Sources list for Preview studio
        let mut src_rows = Rows::default();
        if let Some(st) = self.studio.collection.studio(prv) {
            for it in &st.items {
                src_rows.0.push(vec![
                    it.name.clone(),
                    String::from(if it.visible { "ON" } else { "OFF" }),
                ]);
            }
        }
        let prev_src = ui.selected_row(self.sources_list);
        ui.set_rows(self.sources_list, src_rows);
        if let Some(sel) = prev_src {
            ui.select_row(self.sources_list, Some(sel));
        }

        // Sync audio strip controls and R128/True-Peak readouts
        for su in &self.strip_uis {
            if let Some(strip) = self.studio.audio.strip(su.strip_id) {
                ui.set_text(su.mute_btn, if strip.muted { "Unmute" } else { "Mute" });
                ui.set_button_style(
                    su.solo_btn,
                    if strip.solo {
                        ButtonStyle::Primary
                    } else {
                        ButtonStyle::Normal
                    },
                );
                let m = strip.meter.last_meters;
                ui.set_text(
                    su.meters_label,
                    &format!(
                        "{:+.1}dB | Peak:{:+.1} | TP:{:+.1}dBTP | I:{:+.1}LUFS",
                        strip.fader_db, m.sample_peak_db, m.true_peak_dbtp, m.integrated_lufs
                    ),
                );
            }
        }
    }

    fn sync_plugins_controls(&self, ui: &mut Ui) {
        // Multiview controls
        for &(btn, preset) in &self.mv_preset_btns {
            ui.set_button_style(
                btn,
                if preset == self.studio.multiview_preset {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                },
            );
        }
        ui.set_text(
            self.mv_status_label,
            &format!(
                "Multiview Preset: {:?} | Safe Areas: {} | Program: {} | Preview: {}",
                self.studio.multiview_preset,
                self.studio.multiview_show_safe_areas,
                self.studio.studio_name(self.studio.studio_mode.program_studio),
                self.studio.studio_name(self.studio.studio_mode.preview_studio),
            ),
        );
        for &(btn, idx) in &self.mv_slot_btns {
            let title = self
                .studio
                .collection
                .studios
                .get(idx)
                .map(|s| s.name.as_str())
                .unwrap_or("Empty");
            ui.set_text(btn, title);
            ui.set_button_style(
                btn,
                if idx == self.mv_selected_slot {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                },
            );
        }

        // DSK channels
        for du in &self.dsk_uis {
            if let Some(ch) = self.studio.dsk.channel(du.channel_id) {
                ui.set_text(du.toggle_btn, if ch.active { "ON AIR" } else { "OFF" });
                ui.set_button_style(
                    du.toggle_btn,
                    if ch.active {
                        ButtonStyle::Primary
                    } else {
                        ButtonStyle::Normal
                    },
                );
                ui.set_text(
                    du.status_label,
                    &format!(
                        "Visibility: {:.0}% | TieArmed: {:?} | Z: {}",
                        ch.visibility * 100.0,
                        ch.tied_armed,
                        ch.z_order
                    ),
                );
            }
        }

        // 9:16 vertical summary
        if let Some(sec) = self
            .studio
            .secondary_canvases
            .canvas(self.studio.vertical_canvas_id)
        {
            ui.set_text(
                self.vertical_summary,
                &format!(
                    "{} ({}x{}) | Targets: {} | Backtrack Pkts: {}",
                    sec.name,
                    sec.width,
                    sec.height,
                    sec.router.targets.len(),
                    sec.replay_buffer.len()
                ),
            );
        }

        // Shader & FX labels
        let shader_txt = match self.studio.active_shader_preset() {
            Some(p) => format!("Shader: {:?}", p)
                .split('{')
                .next()
                .unwrap_or("Active")
                .trim()
                .to_string(),
            None => String::from("Shader: Off"),
        };
        ui.set_text(self.shader_label, &shader_txt);
        ui.set_text(
            self.delay_slowmo_btn,
            if self.studio.delay_buffer.live_follow {
                "Delay 0.5x Slow-Mo"
            } else {
                "Delay Go Live (1.0x)"
            },
        );
        ui.set_text(
            self.source_tap_record_btn,
            if self.studio.source_tap.recording.state == RecordingState::Recording {
                "Stop Source-Record Tap"
            } else {
                "Start Source-Record Tap"
            },
        );

        let lines = self.studio.captions.current_lines().join(" | ");
        ui.set_text(
            self.captions_status,
            &format!(
                "Captions (CEA-608 pairs: {}, 708 pkts: {}): {lines}",
                self.studio.last_cea608.len(),
                self.studio
                    .last_cea708
                    .as_ref()
                    .map(|p| p.cc_triplets.len())
                    .unwrap_or(0)
            ),
        );
        ui.set_text(
            self.macro_status,
            &format!(
                "Macros: {} rules | Focused Silk Window: '{}' | Triggers fired: {}",
                self.studio.macros.rules.len(),
                self.studio.focused_silk_window,
                self.studio.active_macro_events.len()
            ),
        );
        ui.set_text(
            self.link_status,
            &format!(
                "CabaretLink LAN: {} ({}:{} | last frame {} bytes) | WasmFX fuel: {}",
                if self.studio.link_enabled { "ADVERTISING" } else { "DISABLED" },
                self.studio.link_announcement.host_name,
                self.studio.link_announcement.port,
                self.studio.link_last_packet_bytes,
                self.studio.wasm_fx.limits().fuel_per_frame
            ),
        );
    }

    fn sync_color_controls(&self, ui: &mut Ui) {
        for &(btn, cs) in &self.color_space_btns {
            ui.set_button_style(
                btn,
                if cs == self.studio.color_pipeline.canvas_space {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                },
            );
        }
        for &(btn, sk) in &self.scope_kind_btns {
            ui.set_button_style(
                btn,
                if sk == self.studio.color_pipeline.scope_kind {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                },
            );
        }
        let stock_name = self
            .studio
            .color_pipeline
            .film_stock
            .map(FilmStock::name)
            .unwrap_or("None");
        ui.set_text(self.film_stock_label, &format!("Film Stock: {stock_name}"));
        ui.set_text(
            self.scope_metrics_label,
            &format!(
                "Color Space: {:?} | ToneMap: {:?} ({:.0}->{:.0} nits) | LUT: {stock_name} | Scope: {}",
                self.studio.color_pipeline.canvas_space,
                self.studio.color_pipeline.tone_map.mode,
                self.studio.color_pipeline.tone_map.source_peak_nits,
                self.studio.color_pipeline.tone_map.target_peak_nits,
                self.studio.color_pipeline.scope_kind.label()
            ),
        );
    }

    fn sync_outputs_controls(&self, ui: &mut Ui) {
        ui.set_text(
            self.stream_toggle_btn,
            if self.studio.stream_active {
                "Stop Streaming"
            } else {
                "Start Streaming"
            },
        );
        ui.set_text(
            self.record_toggle_btn,
            if self.studio.recording.state == RecordingState::Recording {
                "Stop Recording"
            } else {
                "Start Recording"
            },
        );
        ui.set_text(
            self.record_pause_btn,
            if self.studio.recording.state == RecordingState::Paused {
                "Resume Record"
            } else {
                "Pause Record"
            },
        );
        ui.set_text(
            self.vcam_toggle_btn,
            if self.studio.virtual_cam.active {
                "Stop Virtual Cam"
            } else {
                "Start Virtual Cam"
            },
        );

        let groups = self.studio.encoder_groups();
        let issues = self.studio.validate_streams();
        let group_summary: Vec<String> = groups
            .iter()
            .map(|g| {
                format!(
                    "G#{}({:?} {}k, {} dests, shared={})",
                    g.group_id, g.video.codec, g.video.bitrate_kbps, g.dest_ids.len(), g.shared
                )
            })
            .collect();

        ui.set_text(
            self.outputs_summary_label,
            &format!(
                "Streaming: {} ({} frames) | Recording: {:?} seg #{} (tracks=ALL_6, HybridMP4, chapters={}) | Replay: {} saved ({} buffered) | VCam: {} ({})",
                self.studio.stream_active,
                self.studio.stream_frames_sent,
                self.studio.recording.state,
                self.studio.recording.segment_index,
                self.studio.recording.chapters.len(),
                self.studio.saved_replays_count,
                self.studio.replay_buffer.len(),
                self.studio.virtual_cam.device_name,
                if self.studio.virtual_cam.active { "ON" } else { "OFF" }
            ),
        );
        ui.set_text(
            self.encoder_groups_label,
            &format!(
                "Encoder Groups: {} | Service Issues: {} | Shibari Vault Slots: {}",
                group_summary.join("; "),
                issues.len(),
                self.studio.active_secrets.len()
            ),
        );
        let tail: Vec<&str> = self
            .studio
            .log_lines
            .iter()
            .rev()
            .take(4)
            .rev()
            .map(String::as_str)
            .collect();
        ui.set_text(self.log_tail_label, &tail.join("\n"));
    }

    fn sync_prompter_controls(&self, ui: &mut Ui) {
        let tp = &self.studio.teleprompter;
        ui.set_text(
            self.prompter_script_label,
            &format!(
                "WPM: {:.0} | Mirror H/V: {}/{} | Voice Word Index: {}/{} | Scroll: {:.1}px\n{}",
                tp.scroll_wpm,
                tp.mirror_h,
                tp.mirror_v,
                tp.voice_word_index,
                tp.words.len(),
                tp.scroll_offset_px,
                tp.script_text
            ),
        );

        let qc = &self.studio.qc_player;
        ui.set_text(
            self.qc_status_label,
            &format!(
                "Source: {} | Pos: {:.2}s / {:.2}s | Shuttle: {:+.1}x | In: {:?} | Out: {:?}",
                qc.source_path,
                qc.position_us as f64 / 1_000_000.0,
                qc.duration_us as f64 / 1_000_000.0,
                qc.shuttle_speed,
                qc.mark_in_us,
                qc.mark_out_us
            ),
        );

        if let Some(ref h) = self.studio.last_splicy_handoff {
            ui.set_text(
                self.splicy_summary_label,
                &format!(
                    "Splicy Handoff Ready: {}..{} us | {} tracks | {} markers | OTIO ({} bytes) | FCPXML ({} bytes)",
                    h.clip.mark_in_us,
                    h.clip.mark_out_us,
                    h.timeline.tracks.len(),
                    h.timeline.markers.len(),
                    h.otio_json.len(),
                    h.bundle.fcpxml.len()
                ),
            );
        } else {
            ui.set_text(
                self.splicy_summary_label,
                "Click 'Open Last Recording in Splicy' to export marked timeline, chapters & OTIO.",
            );
        }
    }
}
