// SPDX-License-Identifier: GPL-3.0-only
//! Opt-In Muse Weft integration (built-in Tier 0 generation).
//!
//! Provides the dual-mode architecture (Pure Classic DAW default vs Opt-In Muse),
//! the Enable Muse setup card with RAM probe and pack download options,
//! the bottom floating Studio 2.0 Chat bar, left Takes strip, right Inspector
//! stem extraction & LRC provenance cards, and dedicated Workspace::Muse.

extern crate alloc;

use alloc::{format, string::String};
use moan_gen::{
    job::StemRole,
    pack::{ModelPackKind, ModelPackState},
    tier::select_tier,
};
use weft::{Id, Layout, TextStyle, Ui};

/// UI widget handles and view state for Muse.
pub struct MuseUi {
    pub setup_card: Id,
    pub setup_ram_label: Id,
    pub tier0_synth_btn: Id,
    pub cancel_setup_btn: Id,
    pub disable_muse_btn: Id,

    pub chat_bar: Id,
    pub prompt_field: Id,
    pub role_btn: Id,
    pub primary_action_btn: Id,
    pub cover_btn: Id,
    pub vocal2bgm_btn: Id,
    pub run_chat_btn: Id,

    pub takes_strip: Id,
    pub takes_label: Id,
    pub next_version_btn: Id,
    pub insert_take_btn: Id,
    pub insert_all_stems_btn: Id,

    pub inspector_muse: Id,
    pub remix_clip_btn: Id,
    pub extract_stems_btn: Id,
    pub inspector_insert_all_btn: Id,
    pub provenance_text: Id,

    pub muse_workspace: Id,
    pub ws_tier_badge: Id,
    pub ws_prompt_field: Id,
    pub ws_lyrics_field: Id,
    pub ws_create_btn: Id,
    pub ws_vocal2bgm_btn: Id,
    pub ws_extract_btn: Id,

    pub current_role: StemRole,
    pub role_index: usize,
    pub prompt_input: String,
    pub lyrics_input: String,
    pub active_take_idx: usize,
    pub active_version_idx: usize,
}

impl MuseUi {
    pub fn build(ui: &mut Ui, root: Id, arrange: Id, smart: Id) -> Self {
        // 1. Enable Muse Setup Card (docked in root, hidden by default)
        let setup_card = ui.panel(
            root,
            weft::Fill::Control,
            Layout::column().gap(6.0).padding(10.0).fill_width(),
        );
        ui.text(
            setup_card,
            "✨ Enable Muse · Generative Music Studio",
            TextStyle::Heading,
        );
        let setup_ram_label = ui.text(
            setup_card,
            "Detected RAM: probing... Tier recommendation: Lite",
            TextStyle::Body,
        );

        let setup_btns = ui.group(setup_card, Layout::row().gap(6.0).wrap().fill_width());
        let tier0_synth_btn = ui.button(setup_btns, "Use Built-in Synth (0 MB)");
        let cancel_setup_btn = ui.button(setup_btns, "Cancel Muse Setup");
        let disable_muse_btn = ui.button(setup_btns, "Disable Muse");

        // 2. Bottom Floating Context & Studio 2.0 Chat Bar (inside arrange, below waveforms)
        let chat_bar = ui.panel(
            arrange,
            weft::Fill::Control,
            Layout::column().gap(4.0).padding(6.0).fill_width(),
        );
        let chat_row1 = ui.group(chat_bar, Layout::row().gap(4.0).wrap().fill_width());
        ui.text(chat_row1, "✨ Muse Chat:", TextStyle::Caption);
        let prompt_field = ui.field(
            chat_row1,
            "gritty asymmetric distortion",
            "Prompt, styles, or natural chat (e.g. 128 bpm, extract stems)",
        );
        ui.set_layout(prompt_field, Layout::default().width(240.0));

        let role_btn = ui.button(chat_row1, "Role: Full Song");
        let primary_action_btn = ui.button(chat_row1, "Create Song");
        let cover_btn = ui.button(chat_row1, "Cover / Mouth Beat");
        let vocal2bgm_btn = ui.button(chat_row1, "Vocal2BGM");
        let run_chat_btn = ui.button(chat_row1, "Run Chat");

        // 3. Left Takes Strip
        let takes_strip = ui.panel(
            arrange,
            weft::Fill::Surface,
            Layout::column().gap(4.0).padding(6.0).width(160.0).shrink(0.0),
        );
        let takes_label = ui.text(takes_strip, "Takes History (0)", TextStyle::Heading);
        let next_version_btn = ui.button(takes_strip, "< Version 1 / 1 >");
        let insert_take_btn = ui.button(takes_strip, "Insert Take");
        let insert_all_stems_btn = ui.button(takes_strip, "Insert All Stems");

        // 4. Right Inspector Muse Cards (inside smart panel)
        let inspector_muse = ui.panel(
            smart,
            weft::Fill::Control,
            Layout::column().gap(4.0).padding(6.0).fill_width(),
        );
        ui.text(inspector_muse, "Muse Stem & Provenance", TextStyle::Heading);
        let remix_clip_btn = ui.button(inspector_muse, "Remix Clip");
        let extract_stems_btn = ui.button(inspector_muse, "Extract Stems");
        let inspector_insert_all_btn = ui.button(inspector_muse, "Insert All Stems");
        let provenance_text = ui.text(
            inspector_muse,
            "Provenance: Prompt: none · LRC: none",
            TextStyle::Caption,
        );

        // 5. Dedicated Workspace::Muse Canvas
        let muse_workspace = ui.panel(
            root,
            weft::Fill::Surface,
            Layout::column().gap(6.0).padding(8.0).fill_width(),
        );
        ui.text(
            muse_workspace,
            "✨ Moan Muse · Songwriting Canvas",
            TextStyle::Heading,
        );
        let ws_tier_badge = ui.text(
            muse_workspace,
            "Hardware Tier: 🪶 Tier 0 Featherweight (Pure Rust)",
            TextStyle::Body,
        );
        let ws_prompt_field = ui.field(
            muse_workspace,
            "Cyberpunk synthwave with heavy analog bass",
            "Song style, genres, instrumentation",
        );
        let ws_lyrics_field = ui.field(
            muse_workspace,
            "[Verse]\nNeon reflections on rain-slicked asphalt\n[Chorus]\nMoan into the electric night",
            "Structured lyrics ([Verse], [Chorus])",
        );
        let ws_actions = ui.group(muse_workspace, Layout::row().gap(6.0).wrap().fill_width());
        let ws_create_btn = ui.button(ws_actions, "Create Song");
        let ws_vocal2bgm_btn = ui.button(ws_actions, "Vocal2BGM");
        let ws_extract_btn = ui.button(ws_actions, "Extract Stems");

        Self {
            setup_card,
            setup_ram_label,
            tier0_synth_btn,
            cancel_setup_btn,
            disable_muse_btn,

            chat_bar,
            prompt_field,
            role_btn,
            primary_action_btn,
            cover_btn,
            vocal2bgm_btn,
            run_chat_btn,

            takes_strip,
            takes_label,
            next_version_btn,
            insert_take_btn,
            insert_all_stems_btn,

            inspector_muse,
            remix_clip_btn,
            extract_stems_btn,
            inspector_insert_all_btn,
            provenance_text,

            muse_workspace,
            ws_tier_badge,
            ws_prompt_field,
            ws_lyrics_field,
            ws_create_btn,
            ws_vocal2bgm_btn,
            ws_extract_btn,

            current_role: StemRole::Other,
            role_index: 0,
            prompt_input: String::from("gritty asymmetric distortion"),
            lyrics_input: String::new(),
            active_take_idx: 0,
            active_version_idx: 0,
        }
    }

    /// Update RAM and Tier recommendation text on the Setup Card.
    pub fn update_setup_ram(&self, ui: &mut Ui, ram_mb: u64, pack_state: &ModelPackState) {
        let rec = ModelPackKind::recommended_for_ram(ram_mb);
        let selection = select_tier(ram_mb, pack_state, false, None);
        ui.set_text(
            self.setup_ram_label,
            &format!(
                "Detected RAM: {} MB · Recommended Pack: {} · Status: {}",
                ram_mb,
                rec.label(),
                selection.badge_text
            ),
        );
        ui.set_text(
            self.ws_tier_badge,
            &format!("Hardware Tier: {}", selection.badge_text),
        );
    }

    /// Cycle through stem roles: Full Song -> Drums -> Bass -> Vocals -> Chords -> FX.
    pub fn cycle_role(&mut self, ui: &mut Ui) -> StemRole {
        let roles = [
            StemRole::Other,
            StemRole::Drums,
            StemRole::Bass,
            StemRole::Vocals,
            StemRole::Chords,
            StemRole::Fx,
        ];
        self.role_index = (self.role_index + 1) % roles.len();
        self.current_role = roles[self.role_index];
        let label = match self.current_role {
            StemRole::Other => "Role: Full Song",
            StemRole::Drums => "Role: Drums",
            StemRole::Bass => "Role: Bass",
            StemRole::Vocals => "Role: Vocals",
            StemRole::Chords => "Role: Chords",
            StemRole::Fx => "Role: FX",
        };
        ui.set_text(self.role_btn, label);
        self.current_role
    }

    /// Update the primary action button label depending on time selection range.
    pub fn update_contextual_action(&self, ui: &mut Ui, from: usize, to: usize, tracks_count: usize) {
        let label = if from < to {
            "Replace Section"
        } else if tracks_count == 0 {
            "Create Song"
        } else {
            "Add Layer"
        };
        ui.set_text(self.primary_action_btn, label);
    }
}
