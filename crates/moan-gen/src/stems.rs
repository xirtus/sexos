// SPDX-License-Identifier: GPL-3.0-only
//! Stem Model, Take History, Version Switcher (< Version 1 / 2 >), and Suno Studio TakeCrate.
//!
//! Stores generated takes, variations, separated multi-track stems, and timed lyrics,
//! supporting Suno Studio's left Takes strip, version switching, Remix, and Insert All.

use alloc::{string::String, sync::Arc, vec::Vec};
use moan_core::{Frame, RATE};
use crate::job::{MuseError, MuseMode, StemRole};
use crate::lrc::LrcLine;

/// Generated audio for a take version.
#[derive(Clone, Debug)]
pub struct TakeAudio {
    pub id: u64,
    pub frames: Arc<Vec<Frame>>,
    pub sample_rate: u32,
}

impl TakeAudio {
    pub fn new(id: u64, frames: Arc<Vec<Frame>>) -> Self {
        Self {
            id,
            frames,
            sample_rate: RATE,
        }
    }

    pub fn len_frames(&self) -> usize {
        self.frames.len()
    }
}

/// A specific version of a generated take (supporting `< Version 1 / 2 >` switcher).
#[derive(Clone, Debug)]
pub struct TakeVersion {
    pub version_index: usize,
    pub audio: TakeAudio,
    pub stems: Vec<(StemRole, Arc<Vec<Frame>>)>,
    pub lrc: Vec<LrcLine>,
    pub created_at_ms: u64,
}

/// A take container in Suno Studio's left takes strip.
#[derive(Clone, Debug)]
pub struct TakeCrate {
    pub id: u64,
    pub title: String,
    pub prompt: String,
    pub lyrics: String,
    pub lrc: Vec<LrcLine>,
    pub bpm: u16,
    pub key_scale: String,
    pub role: StemRole,
    pub versions: Vec<TakeVersion>,
    pub active_version: usize,
    pub stems: Vec<(StemRole, Arc<Vec<Frame>>)>,
}

impl TakeCrate {
    pub fn new(
        id: u64,
        title: &str,
        prompt: &str,
        lyrics: &str,
        bpm: u16,
        key_scale: &str,
        role: StemRole,
    ) -> Self {
        Self {
            id,
            title: String::from(title),
            prompt: String::from(prompt),
            lyrics: String::from(lyrics),
            lrc: Vec::new(),
            bpm,
            key_scale: String::from(key_scale),
            role,
            versions: Vec::new(),
            active_version: 0,
            stems: Vec::new(),
        }
    }

    /// Add a new generated audio version to this take crate.
    pub fn add_version(
        &mut self,
        audio_frames: Arc<Vec<Frame>>,
        stems: Vec<(StemRole, Arc<Vec<Frame>>)>,
        lrc: Vec<LrcLine>,
        created_at_ms: u64,
    ) -> Result<usize, MuseError> {
        let version_index = self.versions.len();
        let audio = TakeAudio::new(self.id ^ ((version_index as u64 + 1) << 32), audio_frames);
        let version = TakeVersion {
            version_index,
            audio,
            stems: stems.clone(),
            lrc: lrc.clone(),
            created_at_ms,
        };
        self.versions.try_reserve(1).map_err(|_| MuseError::Memory)?;
        self.versions.push(version);

        // Update active version if this is the first version
        if version_index == 0 {
            self.active_version = 0;
            self.lrc = lrc;
            self.stems = stems;
        }

        Ok(version_index)
    }

    /// Switch active version (Suno Studio `< Version 1 / 2 >` switcher).
    pub fn switch_version(&mut self, version_idx: usize) -> Result<(), MuseError> {
        if version_idx >= self.versions.len() {
            return Err(MuseError::InvalidParameters("Version index out of bounds"));
        }
        self.active_version = version_idx;
        let v = &self.versions[version_idx];
        self.lrc = v.lrc.clone();
        self.stems = v.stems.clone();
        Ok(())
    }

    /// Return reference to the active version's audio frames.
    pub fn active_audio(&self) -> Option<&Arc<Vec<Frame>>> {
        self.versions.get(self.active_version).map(|v| &v.audio.frames)
    }

    /// Return reference to active stems.
    pub fn active_stems(&self) -> &[(StemRole, Arc<Vec<Frame>>)] {
        if let Some(v) = self.versions.get(self.active_version) {
            &v.stems
        } else {
            &self.stems
        }
    }

    /// Suno Studio `↻ Remix`: construct a fresh generation request from active parameters.
    pub fn remix(&self) -> MuseMode {
        let duration_frames = self.active_audio().map(|a| a.len()).unwrap_or(48000 * 30);
        MuseMode::CreateSong {
            prompt: self.prompt.clone(),
            lyrics: self.lyrics.clone(),
            vocal_language: String::from("en"),
            bpm: self.bpm,
            key_scale: self.key_scale.clone(),
            time_sig: [4, 4],
            duration_frames,
            thinking_cot: false,
            batch_count: 2,
        }
    }

    /// Suno Studio `← Insert All`: returns all active stems ready to be placed on Moan tracks.
    pub fn insert_all(&self) -> Vec<(StemRole, Arc<Vec<Frame>>)> {
        self.active_stems().to_vec()
    }
}
