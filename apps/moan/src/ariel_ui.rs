// SPDX-License-Identifier: GPL-3.0-only
//! Ariel Vocal & Singing Synth Weft integration.
//!
//! Provides English/Japanese lyric pronunciation, canned vocal presets,
//! formant/breathiness sound-shaping sliders, live karaoke phrase readout,
//! and sample-accurate singing synthesis over MIDI patterns.

extern crate alloc;

use alloc::{
    string::String,
    sync::Arc,
    vec::Vec,
};
use moan_core::{
    phonemes::ARIEL_CANNED_PRESETS,
    pronounce::{english_to_arpabet, japanese_to_phonemes},
    vocal::{self, PhraseSpan, VoiceParams},
    Frame,
};
use weft::{Id, Layout, TextStyle, Ui};

/// Language mode for phonetic conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguageMode {
    English,
    Japanese,
}

impl LanguageMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::English => "Lang: English",
            Self::Japanese => "Lang: Japanese",
        }
    }
}

/// UI widget identifiers and persistent editing state for Ariel.
#[derive(Clone, Debug)]
pub struct ArielUi {
    pub panel: Id,
    pub lyrics_field: Id,
    pub canned_btn: Id,
    pub lang_btn: Id,
    pub f0_slider: Id,
    pub formant_slider: Id,
    pub effort_slider: Id,
    pub breath_slider: Id,
    pub bake_btn: Id,
    pub sing_btn: Id,
    pub karaoke_text: Id,

    pub lyrics_input: String,
    pub canned_index: usize,
    pub lang_mode: LanguageMode,
    pub params: VoiceParams,
}

impl ArielUi {
    pub fn build(ui: &mut Ui, parent: Id) -> Self {
        let panel = ui.panel(
            parent,
            weft::Fill::Control,
            Layout::column().gap(4.0).padding(6.0).fill_width(),
        );

        ui.text(panel, "Ariel · Klatt Formant Vocal Synth", TextStyle::Heading);

        let row1 = ui.group(panel, Layout::row().gap(4.0).wrap().fill_width());
        ui.text(row1, "Lyrics / ARPABET:", TextStyle::Caption);
        let lyrics_field = ui.field(row1, "Hello World", "Singing lyrics or phonetic tokens");
        ui.set_layout(lyrics_field, Layout::default().width(220.0));

        let canned_btn = ui.button(row1, "Canned: choir_chords");
        let lang_btn = ui.button(row1, "Lang: English");

        let row2 = ui.group(panel, Layout::row().gap(6.0).wrap().fill_width());
        ui.text(row2, "F0", TextStyle::Caption);
        let f0_slider = ui.slider(row2, 50.0, 400.0, 120.0);
        ui.set_layout(f0_slider, Layout::default().width(90.0));

        ui.text(row2, "Formant", TextStyle::Caption);
        let formant_slider = ui.slider(row2, 70.0, 150.0, 100.0);
        ui.set_layout(formant_slider, Layout::default().width(90.0));

        ui.text(row2, "Effort", TextStyle::Caption);
        let effort_slider = ui.slider(row2, 0.0, 100.0, 50.0);
        ui.set_layout(effort_slider, Layout::default().width(90.0));

        ui.text(row2, "Breath", TextStyle::Caption);
        let breath_slider = ui.slider(row2, 0.0, 100.0, 0.0);
        ui.set_layout(breath_slider, Layout::default().width(90.0));

        let row3 = ui.group(panel, Layout::row().gap(4.0).wrap().fill_width());
        let bake_btn = ui.button(row3, "Bake Ariel Vocal");
        let sing_btn = ui.button(row3, "Sing on MIDI");
        let karaoke_text = ui.text(row3, "Karaoke: [Ready]", TextStyle::Caption);

        Self {
            panel,
            lyrics_field,
            canned_btn,
            lang_btn,
            f0_slider,
            formant_slider,
            effort_slider,
            breath_slider,
            bake_btn,
            sing_btn,
            karaoke_text,

            lyrics_input: String::from("Hello World"),
            canned_index: 0,
            lang_mode: LanguageMode::English,
            params: VoiceParams::default(),
        }
    }

    /// Advance to next canned phonetic preset.
    pub fn cycle_canned(&mut self, ui: &mut Ui) -> &'static str {
        self.canned_index = (self.canned_index + 1) % ARIEL_CANNED_PRESETS.len();
        let (name, tokens) = ARIEL_CANNED_PRESETS[self.canned_index];
        self.lyrics_input = String::from(tokens);
        ui.set_text(self.lyrics_field, tokens);
        ui.set_text(self.canned_btn, &alloc::format!("Canned: {}", name));
        name
    }

    /// Toggle between English and Japanese pronunciation engines.
    pub fn toggle_language(&mut self, ui: &mut Ui) -> LanguageMode {
        self.lang_mode = match self.lang_mode {
            LanguageMode::English => LanguageMode::Japanese,
            LanguageMode::Japanese => LanguageMode::English,
        };
        ui.set_text(self.lang_btn, self.lang_mode.label());
        self.lang_mode
    }

    /// Convert input lyrics to phonetic ARPABET according to current language mode.
    pub fn resolve_phonetics(&self, input: &str) -> String {
        match self.lang_mode {
            LanguageMode::English => english_to_arpabet(input),
            LanguageMode::Japanese => japanese_to_phonemes(input),
        }
    }

    /// Update live karaoke readout text based on current playhead position.
    pub fn update_karaoke(&self, ui: &mut Ui, spans: &[PhraseSpan], cursor_frame: usize) {
        if spans.is_empty() {
            ui.set_text(self.karaoke_text, "Karaoke: [Silence]");
            return;
        }

        let mut active_idx = None;
        for (i, span) in spans.iter().enumerate() {
            if cursor_frame >= span.start_frame && cursor_frame < span.end_frame {
                active_idx = Some(i);
                break;
            }
        }

        if let Some(idx) = active_idx {
            ui.set_text(
                self.karaoke_text,
                &alloc::format!("Karaoke: >> Phrase {} / {} <<", idx + 1, spans.len()),
            );
        } else if cursor_frame < spans[0].start_frame {
            ui.set_text(self.karaoke_text, "Karaoke: [Pre-roll]");
        } else {
            ui.set_text(self.karaoke_text, "Karaoke: [Outro]");
        }
    }

    /// Synthesize vocal audio frames and phrase spans using current VoiceParams.
    pub fn synthesize(&self, text: &str) -> Result<(Arc<Vec<Frame>>, Vec<PhraseSpan>), vocal::Error> {
        vocal::synthesize(text, &self.params)
    }

    /// Sing current lyrics along a MIDI pattern.
    pub fn sing_midi(
        &self,
        pattern: &moan_core::midi::Pattern,
        text: &str,
    ) -> Result<Arc<Vec<Frame>>, vocal::Error> {
        vocal::sing_midi_pattern(pattern, text, &self.params)
    }
}
