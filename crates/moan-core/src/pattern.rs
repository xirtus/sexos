// SPDX-License-Identifier: GPL-3.0-only
//! Tapestry shared Channel Rack, multi-channel patterns, copy-on-write linked
//! clip instances, Cut/Choke groups, bar-quantized SceneMatrix, and
//! RetrospectiveNoteBuffer.

use crate::{
    compose::ExpressiveNote,
    time::{TempoMap, Time, TimeSignature},
    Error,
};
use alloc::{collections::VecDeque, string::String, sync::Arc, vec::Vec};

pub const MAX_TRACKS: u8 = 8;
pub const MAX_RACK_CHANNELS: usize = 32;
pub const MAX_PATTERNS: usize = 256;
pub const MAX_INSTANCES: usize = 1024;
pub const MAX_BARS: u8 = 16;
pub const SCENE_LANES: usize = 8;
pub const SCENE_COUNT: usize = 8;
pub const MAX_RETROSPECTIVE_NOTES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChannelId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PatternId(pub u16);

/// Shared channel strip in Tapestry's Channel Rack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RackChannel {
    pub id: ChannelId,
    pub name: String,
    pub instrument: u8,
    pub root_key: u8,
    pub volume_pct: u16,
    pub pan: i8,
    pub muted: bool,
    pub solo: bool,
    /// `0` = none; nonzero group chokes earlier active notes across channels.
    pub cut_group: u8,
    /// Mixer track route (`0..MAX_TRACKS`).
    pub target_track: u8,
}

impl RackChannel {
    pub fn new(id: ChannelId, name: impl Into<String>, instrument: u8) -> Self {
        Self {
            id,
            name: name.into(),
            instrument,
            root_key: 60,
            volume_pct: 100,
            pan: 0,
            muted: false,
            solo: false,
            cut_group: 0,
            target_track: (id.0 as u8) % MAX_TRACKS,
        }
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.root_key > 127
            || self.volume_pct > 400
            || !(-100..=100).contains(&self.pan)
            || self.target_track >= MAX_TRACKS
        {
            return Err(Error::Range);
        }
        Ok(())
    }
}

/// Step-grid projection of `ExpressiveNote`s within one quantized step window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepCell {
    pub active: bool,
    pub key: u8,
    pub velocity: u8,
    pub pan: i8,
    pub fine_cents: i8,
    /// `1` = normal hit, `2..=4` = ratchet roll subdivisions within the step.
    pub repeat_subdiv: u8,
    pub slide: bool,
}

impl Default for StepCell {
    fn default() -> Self {
        Self {
            active: false,
            key: 60,
            velocity: 100,
            pan: 0,
            fine_cents: 0,
            repeat_subdiv: 1,
            slide: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelPreviewMode {
    StepGrid,
    MiniRoll,
}

/// Multi-channel Tapestry pattern. Step-grid edits and Piano Roll edits operate
/// directly on the exact same underlying `ExpressiveNote` vectors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapestryPattern {
    pub id: PatternId,
    pub name: String,
    pub color_rgb: u32,
    pub bars: u8,
    /// `4` = 1/16, `3` = 1/12T, `8` = 1/32, `16` = 1/64.
    pub steps_per_beat: u8,
    pub channel_notes: Vec<(ChannelId, Vec<ExpressiveNote>)>,
}

impl TapestryPattern {
    pub fn new(
        id: PatternId,
        name: impl Into<String>,
        bars: u8,
        steps_per_beat: u8,
    ) -> Result<Self, Error> {
        if !(1..=MAX_BARS).contains(&bars) || !(1..=16).contains(&steps_per_beat) {
            return Err(Error::Range);
        }
        Ok(Self {
            id,
            name: name.into(),
            color_rgb: 0x388d74,
            bars,
            steps_per_beat,
            channel_notes: Vec::new(),
        })
    }

    #[inline]
    pub fn step_duration(&self) -> Time {
        Time::new(1, self.steps_per_beat.max(1) as u32).unwrap_or(Time::SIXTEENTH)
    }

    #[inline]
    pub fn total_beats(&self, sig: TimeSignature) -> Time {
        sig.beats_per_bar()
            .mul_int(self.bars.max(1) as i64)
            .unwrap_or_else(|| Time::from_beats(4))
    }

    pub fn total_steps(&self, sig: TimeSignature) -> usize {
        let total = self.total_beats(sig);
        let spb = self.steps_per_beat.max(1) as i128;
        let steps = (total.num as i128 * spb) / (total.den as i128);
        steps.clamp(1, 4096) as usize
    }

    fn next_note_id(&self) -> u32 {
        self.channel_notes
            .iter()
            .flat_map(|(_, notes)| notes.iter().map(|n| n.id))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub fn channel_notes(&self, channel: ChannelId) -> &[ExpressiveNote] {
        self.channel_notes
            .iter()
            .find(|(ch, _)| *ch == channel)
            .map_or(&[], |(_, notes)| notes.as_slice())
    }

    pub fn channel_notes_mut(
        &mut self,
        channel: ChannelId,
    ) -> Result<&mut Vec<ExpressiveNote>, Error> {
        if let Some(pos) = self.channel_notes.iter().position(|(ch, _)| *ch == channel) {
            return Ok(&mut self.channel_notes[pos].1);
        }
        if self.channel_notes.len() >= MAX_RACK_CHANNELS {
            return Err(Error::Range);
        }
        self.channel_notes
            .try_reserve(1)
            .map_err(|_| Error::Memory)?;
        self.channel_notes.push((channel, Vec::new()));
        let last = self.channel_notes.len() - 1;
        Ok(&mut self.channel_notes[last].1)
    }

    /// Add an `ExpressiveNote` to `channel`, validating bounds and keeping chronological sort.
    pub fn add_note(&mut self, channel: ChannelId, mut note: ExpressiveNote) -> Result<u32, Error> {
        note.validate()?;
        if note.id == 0 {
            note.id = self.next_note_id();
        }
        let notes = self.channel_notes_mut(channel)?;
        notes.try_reserve(1).map_err(|_| Error::Memory)?;
        let idx = notes.partition_point(|n| n.start <= note.start);
        notes.insert(idx, note);
        Ok(note.id)
    }

    /// Remove a note by `id` from `channel`.
    pub fn remove_note(&mut self, channel: ChannelId, note_id: u32) -> bool {
        if let Some((_, notes)) = self.channel_notes.iter_mut().find(|(ch, _)| *ch == channel) {
            if let Some(pos) = notes.iter().position(|n| n.id == note_id) {
                notes.remove(pos);
                return true;
            }
        }
        false
    }

    /// Read step-grid view directly from underlying `ExpressiveNote`s for `channel`.
    pub fn step_grid(
        &self,
        channel: ChannelId,
        root_key: u8,
        sig: TimeSignature,
    ) -> Result<Vec<StepCell>, Error> {
        let count = self.total_steps(sig);
        let step_len = self.step_duration();
        let notes = self.channel_notes(channel);
        let mut cells = Vec::new();
        cells
            .try_reserve_exact(count)
            .map_err(|_| Error::Memory)?;

        for step_idx in 0..count {
            let s_start = step_len.mul_int(step_idx as i64).ok_or(Error::Range)?;
            let s_end = s_start.checked_add(step_len).ok_or(Error::Range)?;
            let mut first: Option<&ExpressiveNote> = None;
            let mut hits = 0u8;
            for n in notes {
                if n.start >= s_start && n.start < s_end {
                    if first.is_none() {
                        first = Some(n);
                    }
                    hits = hits.saturating_add(1);
                }
            }
            if let Some(n) = first {
                cells.push(StepCell {
                    active: true,
                    key: n.key,
                    velocity: n.velocity,
                    pan: n.pan,
                    fine_cents: n.fine_cents,
                    repeat_subdiv: hits.clamp(1, 4),
                    slide: n.slide_to_key.is_some(),
                });
            } else {
                cells.push(StepCell {
                    active: false,
                    key: root_key,
                    ..StepCell::default()
                });
            }
        }
        Ok(cells)
    }

    /// Toggle a step directly on the underlying `ExpressiveNote` vector.
    /// Returns `Ok(true)` if toggled ON, `Ok(false)` if toggled OFF.
    pub fn toggle_step(
        &mut self,
        channel: ChannelId,
        step_idx: usize,
        key: u8,
        velocity: u8,
    ) -> Result<bool, Error> {
        if key > 127 || !(1..=127).contains(&velocity) {
            return Err(Error::Range);
        }
        let step_len = self.step_duration();
        let s_start = step_len.mul_int(step_idx as i64).ok_or(Error::Range)?;
        let s_end = s_start.checked_add(step_len).ok_or(Error::Range)?;
        let next_id = self.next_note_id();
        let notes = self.channel_notes_mut(channel)?;

        let had_any = notes.iter().any(|n| n.start >= s_start && n.start < s_end);
        if had_any {
            notes.retain(|n| n.start < s_start || n.start >= s_end);
            return Ok(false);
        }

        notes.try_reserve(1).map_err(|_| Error::Memory)?;
        let note = ExpressiveNote::new(next_id, s_start, step_len, key, velocity);
        let idx = notes.partition_point(|n| n.start <= note.start);
        notes.insert(idx, note);
        Ok(true)
    }

    /// Write an explicit `StepCell` (including ratchet `repeat_subdiv` 2..=4 and slide)
    /// directly into the underlying `ExpressiveNote` vector.
    pub fn set_step(
        &mut self,
        channel: ChannelId,
        step_idx: usize,
        cell: StepCell,
    ) -> Result<(), Error> {
        if cell.key > 127 || !(1..=127).contains(&cell.velocity) || !(-100..=100).contains(&cell.pan)
        {
            return Err(Error::Range);
        }
        let step_len = self.step_duration();
        let s_start = step_len.mul_int(step_idx as i64).ok_or(Error::Range)?;
        let s_end = s_start.checked_add(step_len).ok_or(Error::Range)?;
        let mut next_id = self.next_note_id();
        let notes = self.channel_notes_mut(channel)?;
        notes.retain(|n| n.start < s_start || n.start >= s_end);

        if !cell.active {
            return Ok(());
        }

        let subdiv = cell.repeat_subdiv.clamp(1, 4);
        let sub_len = step_len.div_int(subdiv as i64).ok_or(Error::Range)?;
        notes.try_reserve(subdiv as usize).map_err(|_| Error::Memory)?;
        for r in 0..subdiv {
            let off = sub_len.mul_int(r as i64).ok_or(Error::Range)?;
            let start = s_start.checked_add(off).ok_or(Error::Range)?;
            let note = ExpressiveNote {
                id: next_id,
                start,
                length: sub_len,
                key: cell.key,
                velocity: cell.velocity,
                pan: cell.pan,
                fine_cents: cell.fine_cents,
                slide_to_key: if cell.slide { Some(cell.key) } else { None },
                timbre: 64,
                pressure: 0,
            };
            next_id = next_id.saturating_add(1);
            let idx = notes.partition_point(|n| n.start <= note.start);
            notes.insert(idx, note);
        }
        Ok(())
    }

    /// Returns `true` if `channel` contains chords, overlapping notes, varied pitches,
    /// slides, or sustained notes longer than one step (suitable for mini-roll preview).
    pub fn is_polyphonic_melodic(&self, channel: ChannelId, root_key: u8) -> bool {
        let notes = self.channel_notes(channel);
        if notes.is_empty() {
            return false;
        }
        let step_len = self.step_duration();
        for (i, n) in notes.iter().enumerate() {
            if n.key != root_key || n.slide_to_key.is_some() || n.length > step_len {
                return true;
            }
            let n_end = match n.end() {
                Some(e) => e,
                None => return true,
            };
            for other in &notes[i + 1..] {
                if other.start < n_end {
                    return true;
                }
            }
        }
        false
    }

    pub fn preview_mode(&self, channel: ChannelId, root_key: u8) -> ChannelPreviewMode {
        if self.is_polyphonic_melodic(channel, root_key) {
            ChannelPreviewMode::MiniRoll
        } else {
            ChannelPreviewMode::StepGrid
        }
    }
}

/// Linked pattern clip placed on the timeline / playlist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatternInstance {
    pub pattern: PatternId,
    pub start_beat: Time,
    pub length_beats: Time,
    pub offset_beats: Time,
    pub lane: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportContext {
    PatternMode,
    SongMode,
}

/// Realized playback event with exact musical beats, 48 kHz frames, and choke/mute applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RealizedChannelNote {
    pub channel: ChannelId,
    pub target_track: u8,
    pub instrument: u8,
    pub cut_group: u8,
    pub start_beat: Time,
    pub end_beat: Time,
    pub start_frame: u64,
    pub end_frame: u64,
    pub key: u8,
    pub velocity: u8,
    pub pan: i8,
    pub fine_cents: i8,
    pub slide_to_key: Option<u8>,
    pub timbre: u8,
    pub pressure: u8,
}

/// Central Tapestry bank owning shared `RackChannel`s, `Arc`-shared `TapestryPattern`s,
/// linked `PatternInstance`s, transport context, `TempoMap`, and global swing.
#[derive(Clone, Debug)]
pub struct TapestryBank {
    pub channels: Vec<RackChannel>,
    pub patterns: Vec<Arc<TapestryPattern>>,
    pub instances: Vec<PatternInstance>,
    pub active_pattern: PatternId,
    pub transport_mode: TransportContext,
    pub tempo_map: TempoMap,
    pub swing_pct: u8,
    next_channel_id: u16,
    next_pattern_id: u16,
}

impl Default for TapestryBank {
    fn default() -> Self {
        Self::new()
    }
}

impl TapestryBank {
    pub fn new() -> Self {
        Self {
            channels: Vec::new(),
            patterns: Vec::new(),
            instances: Vec::new(),
            active_pattern: PatternId(0),
            transport_mode: TransportContext::PatternMode,
            tempo_map: TempoMap::default(),
            swing_pct: 0,
            next_channel_id: 0,
            next_pattern_id: 0,
        }
    }

    pub fn add_channel(
        &mut self,
        name: impl Into<String>,
        instrument: u8,
    ) -> Result<ChannelId, Error> {
        let id = ChannelId(self.next_channel_id);
        let ch = RackChannel::new(id, name, instrument);
        self.push_channel(ch)
    }

    pub fn push_channel(&mut self, ch: RackChannel) -> Result<ChannelId, Error> {
        ch.validate()?;
        if self.channels.len() >= MAX_RACK_CHANNELS
            || self.channels.iter().any(|c| c.id == ch.id)
        {
            return Err(Error::Range);
        }
        self.channels.try_reserve(1).map_err(|_| Error::Memory)?;
        let id = ch.id;
        self.next_channel_id = self.next_channel_id.max(id.0.saturating_add(1));
        self.channels.push(ch);
        Ok(id)
    }

    pub fn channel(&self, id: ChannelId) -> Option<&RackChannel> {
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn channel_mut(&mut self, id: ChannelId) -> Option<&mut RackChannel> {
        self.channels.iter_mut().find(|c| c.id == id)
    }

    pub fn create_pattern(
        &mut self,
        name: impl Into<String>,
        bars: u8,
        steps_per_beat: u8,
    ) -> Result<PatternId, Error> {
        let id = PatternId(self.next_pattern_id);
        let pat = TapestryPattern::new(id, name, bars, steps_per_beat)?;
        self.push_pattern(pat)
    }

    pub fn push_pattern(&mut self, pat: TapestryPattern) -> Result<PatternId, Error> {
        if self.patterns.len() >= MAX_PATTERNS || self.patterns.iter().any(|p| p.id == pat.id) {
            return Err(Error::Range);
        }
        self.patterns.try_reserve(1).map_err(|_| Error::Memory)?;
        let id = pat.id;
        self.next_pattern_id = self.next_pattern_id.max(id.0.saturating_add(1));
        if self.patterns.is_empty() {
            self.active_pattern = id;
        }
        self.patterns.push(Arc::new(pat));
        Ok(id)
    }

    pub fn pattern(&self, id: PatternId) -> Option<&TapestryPattern> {
        self.patterns
            .iter()
            .find(|p| p.id == id)
            .map(|arc| arc.as_ref())
    }

    /// Copy-on-write mutable access to a pattern via `Arc::make_mut`.
    pub fn pattern_mut(&mut self, id: PatternId) -> Option<&mut TapestryPattern> {
        self.patterns
            .iter_mut()
            .find(|p| p.id == id)
            .map(Arc::make_mut)
    }

    pub fn place_instance(&mut self, instance: PatternInstance) -> Result<usize, Error> {
        if self.instances.len() >= MAX_INSTANCES
            || instance.start_beat.is_negative()
            || instance.length_beats <= Time::ZERO
            || instance.offset_beats.is_negative()
            || instance.lane >= SCENE_LANES as u8
            || self.pattern(instance.pattern).is_none()
        {
            return Err(Error::Range);
        }
        self.instances.try_reserve(1).map_err(|_| Error::Memory)?;
        let idx = self.instances.len();
        self.instances.push(instance);
        Ok(idx)
    }

    /// Deep-clone an existing pattern into a fresh `PatternId`.
    pub fn clone_pattern(&mut self, id: PatternId) -> Result<PatternId, Error> {
        let src = self.pattern(id).ok_or(Error::Range)?.clone();
        let new_id = PatternId(self.next_pattern_id);
        let mut cloned = src;
        cloned.id = new_id;
        cloned.name.push_str(" (Copy)");
        self.push_pattern(cloned)
    }

    /// Clone the pattern referenced by `instances[instance_idx]` into a new `PatternId`
    /// and rebind ONLY that instance, leaving all sibling instances linked to the original.
    pub fn make_instance_unique(&mut self, instance_idx: usize) -> Result<PatternId, Error> {
        let old_id = self
            .instances
            .get(instance_idx)
            .ok_or(Error::Range)?
            .pattern;
        let new_id = self.clone_pattern(old_id)?;
        self.instances[instance_idx].pattern = new_id;
        Ok(new_id)
    }

    /// Split a multi-channel pattern into separate single-channel patterns (one per non-empty channel).
    pub fn split_pattern_by_channel(&mut self, id: PatternId) -> Result<Vec<PatternId>, Error> {
        let src = self.pattern(id).ok_or(Error::Range)?.clone();
        let mut created_ids = Vec::new();
        for (ch_id, notes) in src.channel_notes {
            if notes.is_empty() {
                continue;
            }
            let ch_label = self
                .channel(ch_id)
                .map(|c| c.name.as_str())
                .unwrap_or("Channel");
            let mut pat_name = src.name.clone();
            pat_name.push_str(" - ");
            pat_name.push_str(ch_label);

            let new_id = PatternId(self.next_pattern_id);
            let mut single = TapestryPattern::new(new_id, pat_name, src.bars, src.steps_per_beat)?;
            single.color_rgb = src.color_rgb;
            single
                .channel_notes
                .try_reserve_exact(1)
                .map_err(|_| Error::Memory)?;
            single.channel_notes.push((ch_id, notes));
            self.push_pattern(single)?;
            created_ids.try_reserve(1).map_err(|_| Error::Memory)?;
            created_ids.push(new_id);
        }
        Ok(created_ids)
    }

    /// Realize all audible events for `mode`, honoring Mute/Solo, `TempoMap`, global swing,
    /// and cross-channel / intra-channel `cut_group` choking at the exact choke onset frame.
    pub fn realize_events(
        &self,
        mode: TransportContext,
    ) -> Result<Vec<RealizedChannelNote>, Error> {
        let any_solo = self.channels.iter().any(|c| c.solo && !c.muted);
        let sig = self.tempo_map.time_signature_at(Time::ZERO);
        let mut out = Vec::new();

        match mode {
            TransportContext::PatternMode => {
                if let Some(pat) = self.pattern(self.active_pattern) {
                    let pat_end = pat.total_beats(sig);
                    self.emit_pattern_slice(
                        pat,
                        Time::ZERO,
                        Time::ZERO,
                        pat_end,
                        any_solo,
                        &mut out,
                    )?;
                }
            }
            TransportContext::SongMode => {
                for inst in &self.instances {
                    let Some(pat) = self.pattern(inst.pattern) else {
                        continue;
                    };
                    let src_end = inst
                        .offset_beats
                        .checked_add(inst.length_beats)
                        .ok_or(Error::Range)?;
                    self.emit_pattern_slice(
                        pat,
                        inst.start_beat,
                        inst.offset_beats,
                        src_end,
                        any_solo,
                        &mut out,
                    )?;
                }
            }
        }

        // Sort chronologically so cut groups truncate earlier sounding notes cleanly.
        out.sort_by(|a, b| {
            a.start_frame
                .cmp(&b.start_frame)
                .then(a.start_beat.cmp(&b.start_beat))
                .then(a.channel.cmp(&b.channel))
                .then(a.key.cmp(&b.key))
        });

        // Apply Cut/Choke groups: any note in nonzero `cut_group` chokes earlier active notes
        // in the same `cut_group` at its exact `start_frame`.
        for i in 0..out.len() {
            let group = out[i].cut_group;
            if group == 0 {
                continue;
            }
            let choke_frame = out[i].start_frame;
            let choke_beat = out[i].start_beat;
            for prev in &mut out[..i] {
                if prev.cut_group == group
                    && prev.start_frame < choke_frame
                    && prev.end_frame > choke_frame
                {
                    prev.end_frame = choke_frame;
                    if prev.end_beat > choke_beat {
                        prev.end_beat = choke_beat;
                    }
                }
            }
        }

        Ok(out)
    }

    fn emit_pattern_slice(
        &self,
        pat: &TapestryPattern,
        dest_start: Time,
        src_offset: Time,
        src_end: Time,
        any_solo: bool,
        out: &mut Vec<RealizedChannelNote>,
    ) -> Result<(), Error> {
        for (ch_id, notes) in &pat.channel_notes {
            let Some(ch) = self.channel(*ch_id) else {
                continue;
            };
            if ch.muted || (any_solo && !ch.solo) || ch.volume_pct == 0 {
                continue;
            }
            out.try_reserve(notes.len()).map_err(|_| Error::Memory)?;
            for note in notes {
                if note.start < src_offset || note.start >= src_end {
                    continue;
                }
                let raw_end = note.end().ok_or(Error::Range)?.min(src_end);
                if raw_end <= note.start {
                    continue;
                }
                let rel_start = note.start.checked_sub(src_offset).ok_or(Error::Range)?;
                let rel_end = raw_end.checked_sub(src_offset).ok_or(Error::Range)?;
                let abs_start = dest_start
                    .saturating_quantized_add(rel_start)
                    .ok_or(Error::Range)?;
                let abs_end = dest_start
                    .saturating_quantized_add(rel_end)
                    .ok_or(Error::Range)?;
                let start_frame = self.tempo_map.beat_to_frame(abs_start, self.swing_pct);
                let end_frame = self
                    .tempo_map
                    .beat_to_frame(abs_end, self.swing_pct)
                    .max(start_frame.saturating_add(1));

                let vel = ((note.velocity as u32 * ch.volume_pct as u32) / 100).clamp(1, 127) as u8;
                let pan = (note.pan as i16 + ch.pan as i16).clamp(-100, 100) as i8;

                out.push(RealizedChannelNote {
                    channel: ch.id,
                    target_track: ch.target_track,
                    instrument: ch.instrument,
                    cut_group: ch.cut_group,
                    start_beat: abs_start,
                    end_beat: abs_end,
                    start_frame,
                    end_frame,
                    key: note.key,
                    velocity: vel,
                    pan,
                    fine_cents: note.fine_cents,
                    slide_to_key: note.slide_to_key,
                    timbre: note.timbre,
                    pressure: note.pressure,
                });
            }
        }
        Ok(())
    }
}

/// One row of 8 pattern slots in Tapestry's SceneMatrix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneRow {
    pub name: String,
    pub slots: [Option<PatternId>; SCENE_LANES],
}

impl SceneRow {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            slots: [None; SCENE_LANES],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueuedScene {
    pub scene_idx: usize,
    pub launch_beat: Time,
    pub launch_frame: u64,
}

/// 8-lane x 8-scene live performance launcher with bar-quantized scheduling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneMatrix {
    pub scenes: Vec<SceneRow>,
    pub active_scene: Option<usize>,
    pub queued: Option<QueuedScene>,
}

impl Default for SceneMatrix {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneMatrix {
    pub fn new() -> Self {
        let mut scenes = Vec::new();
        let _ = scenes.try_reserve_exact(SCENE_COUNT);
        for idx in 0..SCENE_COUNT {
            let mut label = String::from("Scene ");
            label.push((b'1' + idx as u8) as char);
            scenes.push(SceneRow::new(label));
        }
        Self {
            scenes,
            active_scene: None,
            queued: None,
        }
    }

    pub fn set_slot(
        &mut self,
        scene_idx: usize,
        lane: usize,
        pattern: Option<PatternId>,
    ) -> Result<(), Error> {
        let row = self.scenes.get_mut(scene_idx).ok_or(Error::Range)?;
        let slot = row.slots.get_mut(lane).ok_or(Error::Range)?;
        *slot = pattern;
        Ok(())
    }

    /// Queue a scene launch quantized to the exact next bar boundary in `tempo_map`.
    pub fn queue_scene(
        &mut self,
        scene_idx: usize,
        current_frame: u64,
        tempo_map: &TempoMap,
        swing_pct: u8,
    ) -> Result<QueuedScene, Error> {
        if scene_idx >= self.scenes.len() {
            return Err(Error::Range);
        }
        let (launch_beat, launch_frame) = tempo_map.next_bar_at_frame(current_frame, swing_pct);
        let q = QueuedScene {
            scene_idx,
            launch_beat,
            launch_frame,
        };
        self.queued = Some(q);
        Ok(q)
    }

    pub fn queue_scene_for_bank(
        &mut self,
        scene_idx: usize,
        current_frame: u64,
        bank: &TapestryBank,
    ) -> Result<QueuedScene, Error> {
        self.queue_scene(scene_idx, current_frame, &bank.tempo_map, bank.swing_pct)
    }

    /// Poll/advance launch state against `current_frame`. When `current_frame >= launch_frame`,
    /// activates the scene and records its launched `PatternInstance`s into `bank.instances`.
    /// Returns the number of `PatternInstance`s launched (0 if nothing triggered yet).
    pub fn advance_to(
        &mut self,
        current_frame: u64,
        bank: &mut TapestryBank,
    ) -> Result<usize, Error> {
        let Some(q) = self.queued else {
            return Ok(0);
        };
        if current_frame < q.launch_frame {
            return Ok(0);
        }
        let slots = self.scenes.get(q.scene_idx).ok_or(Error::Range)?.slots;
        let sig = bank.tempo_map.time_signature_at(q.launch_beat);
        let mut launched = 0usize;
        for (lane, slot) in slots.iter().enumerate() {
            let Some(pat_id) = *slot else {
                continue;
            };
            let Some(pat) = bank.pattern(pat_id) else {
                continue;
            };
            let len_beats = pat.total_beats(sig);
            bank.place_instance(PatternInstance {
                pattern: pat_id,
                start_beat: q.launch_beat,
                length_beats: len_beats,
                offset_beats: Time::ZERO,
                lane: lane as u8,
            })?;
            launched += 1;
        }
        self.active_scene = Some(q.scene_idx);
        self.queued = None;
        Ok(launched)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetrospectiveNote {
    pub channel: ChannelId,
    pub start_frame: u64,
    pub length_frames: u64,
    pub key: u8,
    pub velocity: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActiveCapture {
    channel: ChannelId,
    start_frame: u64,
    key: u8,
    velocity: u8,
}

/// Bounded ring buffer (`<= 512` notes) capturing unrecorded live performances
/// so riffs can always be recovered into a `TapestryPattern`.
#[derive(Clone, Debug)]
pub struct RetrospectiveNoteBuffer {
    ring: VecDeque<RetrospectiveNote>,
    active: Vec<ActiveCapture>,
}

impl Default for RetrospectiveNoteBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl RetrospectiveNoteBuffer {
    pub fn new() -> Self {
        Self {
            ring: VecDeque::new(),
            active: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    pub fn clear(&mut self) {
        self.ring.clear();
        self.active.clear();
    }

    /// Push a completed live note into the bounded 512-entry ring buffer.
    pub fn push_note(&mut self, note: RetrospectiveNote) -> Result<(), Error> {
        if note.key > 127 || !(1..=127).contains(&note.velocity) || note.length_frames == 0 {
            return Err(Error::Range);
        }
        if self.ring.len() >= MAX_RETROSPECTIVE_NOTES {
            self.ring.pop_front();
        } else {
            self.ring.try_reserve(1).map_err(|_| Error::Memory)?;
        }
        self.ring.push_back(note);
        Ok(())
    }

    pub fn note_on(
        &mut self,
        channel: ChannelId,
        frame: u64,
        key: u8,
        velocity: u8,
    ) -> Result<(), Error> {
        if key > 127 || !(1..=127).contains(&velocity) {
            return Err(Error::Range);
        }
        if self.active.len() >= 64 {
            self.active.remove(0);
        } else {
            self.active.try_reserve(1).map_err(|_| Error::Memory)?;
        }
        self.active.push(ActiveCapture {
            channel,
            start_frame: frame,
            key,
            velocity,
        });
        Ok(())
    }

    pub fn note_off(&mut self, channel: ChannelId, frame: u64, key: u8) -> Result<bool, Error> {
        if let Some(pos) = self
            .active
            .iter()
            .position(|a| a.channel == channel && a.key == key)
        {
            let cap = self.active.remove(pos);
            let length_frames = frame.saturating_sub(cap.start_frame).max(1);
            self.push_note(RetrospectiveNote {
                channel: cap.channel,
                start_frame: cap.start_frame,
                length_frames,
                key: cap.key,
                velocity: cap.velocity,
            })?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &RetrospectiveNote> {
        self.ring.iter()
    }

    /// Recover all captured notes into a bar-aligned `TapestryPattern`, optionally quantizing
    /// note onsets to `quantize_grid` (e.g. `Some(Time::SIXTEENTH)`).
    pub fn recover_to_pattern(
        &self,
        id: PatternId,
        name: impl Into<String>,
        tempo_map: &TempoMap,
        swing_pct: u8,
        quantize_grid: Option<Time>,
    ) -> Result<TapestryPattern, Error> {
        if let Some(grid) = quantize_grid {
            if grid <= Time::ZERO {
                return Err(Error::Range);
            }
        }
        if self.ring.is_empty() {
            return TapestryPattern::new(id, name, 1, 4);
        }

        let min_frame = self
            .ring
            .iter()
            .map(|n| n.start_frame)
            .min()
            .unwrap_or(0);
        let min_beat = tempo_map.frame_to_beat(min_frame, swing_pct);
        let (bar_origin, _) = tempo_map.bar_bounds_at_beat(min_beat);

        let mut max_rel_end = Time::ZERO;
        let mut staged: Vec<(ChannelId, ExpressiveNote)> = Vec::new();
        staged
            .try_reserve(self.ring.len())
            .map_err(|_| Error::Memory)?;

        for (idx, item) in self.ring.iter().enumerate() {
            let s_beat = tempo_map.frame_to_beat(item.start_frame, swing_pct);
            let e_beat = tempo_map.frame_to_beat(
                item.start_frame.saturating_add(item.length_frames),
                swing_pct,
            );
            let mut rel_start = s_beat.checked_sub(bar_origin).unwrap_or(Time::ZERO);
            if let Some(grid) = quantize_grid {
                // Snap rel_start to nearest multiple of `grid`.
                let num = rel_start.num as i128 * grid.den as i128;
                let den = rel_start.den as i128 * grid.num as i128;
                let idx_q = (num + den / 2) / den;
                rel_start = grid.mul_int(idx_q as i64).unwrap_or(rel_start);
            }
            let len = e_beat
                .checked_sub(s_beat)
                .filter(|l| *l > Time::ZERO)
                .unwrap_or(Time::MIN_BEAT_STEP);
            let rel_end = rel_start
                .saturating_quantized_add(len)
                .ok_or(Error::Range)?;
            if rel_end > max_rel_end {
                max_rel_end = rel_end;
            }
            staged.push((
                item.channel,
                ExpressiveNote::new(
                    (idx + 1) as u32,
                    rel_start,
                    len,
                    item.key,
                    item.velocity,
                ),
            ));
        }

        let sig = tempo_map.time_signature_at(bar_origin);
        let bpb = sig.beats_per_bar();
        let bars_needed = ((max_rel_end.to_f64() / bpb.to_f64()) + 0.999_999) as i64;
        let bars = bars_needed.clamp(1, MAX_BARS as i64) as u8;

        let mut pat = TapestryPattern::new(id, name, bars, 4)?;
        for (ch, note) in staged {
            pat.add_note(ch, note)?;
        }
        Ok(pat)
    }
}
