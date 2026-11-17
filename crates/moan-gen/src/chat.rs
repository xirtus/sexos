// SPDX-License-Identifier: GPL-3.0-only
//! Muse Natural-Language Chat Bar Parser.
//!
//! Parses natural language prompts, slash commands, and context-sensitive edit intents
//! into structured `StudioChatCommand` actions (Muse generation, MIDI clips, device presets,
//! tempo changes, stem separation, etc.).

use alloc::{string::String, vec::Vec};
use moan_core::RATE;
use crate::job::{MuseMode, StemRole};

/// Parsed studio chat action dispatched by Moan's Muse bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StudioChatCommand {
    /// Execute a full generative Muse job (Create, AddLayer, Replace, Extend, Cover, etc.).
    RunMuseJob(MuseMode),
    /// Generate a MIDI guide clip.
    GenerateMidiClip {
        style: String,
        bars: u16,
        bpm: u16,
    },
    /// Design an effect or synth preset via natural language.
    DesignDevicePreset {
        target: String,
        description: String,
    },
    /// Transcribe selected audio region into MIDI notes.
    ExtractMidiFromSelected,
    /// Split selected audio region into stems (Vocals, Drums, Bass, Chords, FX, Other).
    ExtractStemsFromSelected,
    /// Set the session tempo in BPM.
    SetSessionTempoBpm(u16),
}

/// Parse user input from Muse natural language command bar.
pub fn parse_studio_chat(
    input: &str,
    has_selection: bool,
    selected_is_midi: bool,
) -> StudioChatCommand {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();

    // 1. Session tempo commands: e.g. "/tempo 120", "tempo 140", "128 bpm", "set bpm to 130"
    if let Some(bpm) = parse_tempo_command(&lower) {
        return StudioChatCommand::SetSessionTempoBpm(bpm);
    }

    // 2. Stem separation: e.g. "/stems", "extract stems", "split stems", "separate stems"
    if lower == "/stems"
        || lower.starts_with("extract stem")
        || lower.starts_with("split stem")
        || lower.starts_with("separate stem")
        || lower.contains("into stems")
    {
        return StudioChatCommand::ExtractStemsFromSelected;
    }

    // 3. Audio to MIDI extraction: e.g. "/midi", "audio to midi", "extract midi", "transcribe"
    if lower == "/midi"
        || lower.starts_with("extract midi")
        || lower.starts_with("audio to midi")
        || lower.starts_with("transcribe midi")
        || (has_selection && !selected_is_midi && lower.starts_with("transcribe"))
    {
        return StudioChatCommand::ExtractMidiFromSelected;
    }

    // 4. MIDI Clip Generation: e.g. "generate midi 4 bars tech house", "midi clip 8 bars"
    if lower.starts_with("generate midi")
        || lower.starts_with("midi clip")
        || lower.starts_with("create midi")
        || lower.starts_with("make midi")
    {
        let bars = extract_bars(&lower).unwrap_or(4);
        let bpm = extract_bpm_in_text(&lower).unwrap_or(120);
        return StudioChatCommand::GenerateMidiClip {
            style: String::from(trimmed),
            bars,
            bpm,
        };
    }

    // 5. Device Preset Design: e.g. "preset eq warm tube", "design preset compressor punchy"
    if lower.starts_with("preset ")
        || lower.starts_with("/preset ")
        || lower.starts_with("design preset ")
        || lower.starts_with("sound design ")
    {
        let (target, desc) = parse_preset_target(&lower, trimmed);
        return StudioChatCommand::DesignDevicePreset {
            target,
            description: desc,
        };
    }

    // 6. Context-Aware Muse Modes
    if has_selection && (lower.starts_with("replace") || lower.starts_with("inpaint") || lower.contains("change this section")) {
        return StudioChatCommand::RunMuseJob(MuseMode::ReplaceSection {
            from_frame: 0,
            to_frame: RATE as usize * 4,
        });
    }

    if lower.starts_with("add ") || lower.starts_with("overdub ") || lower.starts_with("layer ") {
        let role = if lower.contains("bass") {
            StemRole::Bass
        } else if lower.contains("drum") || lower.contains("beat") {
            StemRole::Drums
        } else if lower.contains("vocal") || lower.contains("voice") {
            StemRole::Vocals
        } else if lower.contains("chord") || lower.contains("pad") || lower.contains("piano") {
            StemRole::Chords
        } else if lower.contains("fx") || lower.contains("riser") || lower.contains("sweep") {
            StemRole::Fx
        } else {
            StemRole::Other
        };
        return StudioChatCommand::RunMuseJob(MuseMode::AddLayer { role });
    }

    if lower.starts_with("extend") || lower.starts_with("continue") || lower.contains("make it longer") {
        return StudioChatCommand::RunMuseJob(MuseMode::Extend {
            from_frame: 0,
            extra_frames: RATE as usize * 16,
        });
    }

    if lower.contains("mouth beat") || lower.contains("beatbox") || lower.starts_with("cover") || lower.contains("hum to") {
        return StudioChatCommand::RunMuseJob(MuseMode::Cover {
            style_strength_pct: 80,
        });
    }

    if lower.contains("vocal to bgm") || lower.contains("backing track") || lower.contains("bgm for") {
        return StudioChatCommand::RunMuseJob(MuseMode::Vocal2Bgm);
    }

    if lower.starts_with("understand") || lower.starts_with("analyze") || lower.contains("detect key") {
        return StudioChatCommand::RunMuseJob(MuseMode::UnderstandAudio);
    }

    // 7. Full Song Creation (Default)
    let bpm = extract_bpm_in_text(&lower).unwrap_or(120);
    let key_scale = extract_key_in_text(&lower).unwrap_or_else(|| String::from("C Major"));
    let thinking_cot = lower.contains("cot") || lower.contains("think");

    StudioChatCommand::RunMuseJob(MuseMode::CreateSong {
        prompt: String::from(trimmed),
        lyrics: String::new(),
        vocal_language: String::from("en"),
        bpm,
        key_scale,
        time_sig: [4, 4],
        duration_frames: RATE as usize * 30,
        thinking_cot,
        batch_count: 2,
    })
}

fn parse_tempo_command(lower: &str) -> Option<u16> {
    if let Some(rest) = lower.strip_prefix("/tempo ") {
        return rest.trim().parse().ok();
    }
    if let Some(rest) = lower.strip_prefix("tempo ") {
        return rest.trim().parse().ok();
    }
    if let Some(rest) = lower.strip_prefix("set bpm to ") {
        return rest.trim().parse().ok();
    }
    if let Some(rest) = lower.strip_prefix("set tempo to ") {
        return rest.trim().parse().ok();
    }
    if let Some(rest) = lower.strip_suffix(" bpm") {
        if let Ok(v) = rest.trim().parse::<u16>() {
            if (30..=300).contains(&v) {
                return Some(v);
            }
        }
    }
    None
}

fn extract_bars(lower: &str) -> Option<u16> {
    let words: Vec<&str> = lower.split_whitespace().collect();
    for i in 0..words.len() {
        if (words[i] == "bar" || words[i] == "bars") && i > 0 {
            if let Ok(b) = words[i - 1].parse::<u16>() {
                return Some(b);
            }
        }
    }
    None
}

fn extract_bpm_in_text(lower: &str) -> Option<u16> {
    let words: Vec<&str> = lower.split_whitespace().collect();
    for i in 0..words.len() {
        if words[i] == "bpm" && i > 0 {
            if let Ok(b) = words[i - 1].parse::<u16>() {
                if (40..=240).contains(&b) {
                    return Some(b);
                }
            }
        }
    }
    None
}

fn extract_key_in_text(lower: &str) -> Option<String> {
    let keys = [
        "c major", "c minor", "c# minor", "db major", "d major", "d minor", "eb major",
        "e major", "e minor", "f major", "f minor", "f# minor", "g major", "g minor",
        "ab major", "a major", "a minor", "bb major", "b major", "b minor",
    ];
    for &k in &keys {
        if lower.contains(k) {
            let mut capitalized = String::new();
            for word in k.split_whitespace() {
                if !capitalized.is_empty() {
                    capitalized.push(' ');
                }
                let mut chars = word.chars();
                if let Some(first) = chars.next() {
                    capitalized.push(first.to_ascii_uppercase());
                    capitalized.push_str(chars.as_str());
                }
            }
            return Some(capitalized);
        }
    }
    None
}

fn parse_preset_target(lower: &str, orig: &str) -> (String, String) {
    let targets = ["eq", "compressor", "reverb", "delay", "synth", "limiter", "filter"];
    for &t in &targets {
        if lower.contains(t) {
            return (String::from(t), String::from(orig));
        }
    }
    (String::from("device"), String::from(orig))
}
