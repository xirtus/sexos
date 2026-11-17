// SPDX-License-Identifier: GPL-3.0-only
//! Rich expressive note model, lossless `midi::Note` conversion via `TempoMap`,
//! and deterministic Piano Roll compositional note transforms.

use crate::{
    midi,
    time::{TempoMap, Time},
    Error,
};
use alloc::vec::Vec;

const MAX_COMPOSE_NOTES: usize = 16_384;

/// Rich expressive note used by Tapestry patterns and Piano Roll tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpressiveNote {
    pub id: u32,
    pub start: Time,
    pub length: Time,
    pub key: u8,
    pub velocity: u8,
    pub pan: i8,
    pub fine_cents: i8,
    pub slide_to_key: Option<u8>,
    pub timbre: u8,
    pub pressure: u8,
}

impl ExpressiveNote {
    pub const fn new(id: u32, start: Time, length: Time, key: u8, velocity: u8) -> Self {
        Self {
            id,
            start,
            length,
            key,
            velocity,
            pan: 0,
            fine_cents: 0,
            slide_to_key: None,
            timbre: 64,
            pressure: 0,
        }
    }

    #[inline]
    pub fn end(&self) -> Option<Time> {
        self.start.checked_add(self.length)
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.start.is_negative()
            || self.length <= Time::ZERO
            || self.key > 127
            || !(1..=127).contains(&self.velocity)
            || !(-100..=100).contains(&self.pan)
            || self.timbre > 127
            || self.pressure > 127
        {
            return Err(Error::Range);
        }
        if let Some(slide) = self.slide_to_key {
            if slide > 127 {
                return Err(Error::Range);
            }
        }
        self.end().ok_or(Error::Range)?;
        Ok(())
    }

    /// Convert to sample-frame `midi::Note` using `TempoMap` and global `swing_pct`.
    pub fn to_midi_note(
        &self,
        tempo: &TempoMap,
        swing_pct: u8,
    ) -> Result<midi::Note, midi::Error> {
        if self.key > 127
            || !(1..=127).contains(&self.velocity)
            || self.start.is_negative()
            || self.length <= Time::ZERO
        {
            return Err(midi::Error::Note);
        }
        let end_beat = self.start.checked_add(self.length).ok_or(midi::Error::Range)?;
        let start_f = tempo.beat_to_frame(self.start, swing_pct);
        let end_f = tempo.beat_to_frame(end_beat, swing_pct);
        let frames = end_f.saturating_sub(start_f).max(1);
        let start = u32::try_from(start_f).map_err(|_| midi::Error::Range)?;
        let frames = u32::try_from(frames).map_err(|_| midi::Error::Range)?;
        Ok(midi::Note {
            start,
            frames,
            key: self.key,
            velocity: self.velocity,
        })
    }

    /// Construct an `ExpressiveNote` from a frame-based `midi::Note` using `TempoMap`.
    pub fn from_midi_note(id: u32, note: midi::Note, tempo: &TempoMap, swing_pct: u8) -> Self {
        let start = tempo.frame_to_beat(note.start as u64, swing_pct);
        let end_frame = note.start as u64 + note.frames as u64;
        let end_beat = tempo.frame_to_beat(end_frame, swing_pct);
        let length = end_beat
            .checked_sub(start)
            .filter(|l| *l > Time::ZERO)
            .unwrap_or(Time::MIN_BEAT_STEP);
        Self::new(
            id,
            start,
            length,
            note.key.min(127),
            note.velocity.clamp(1, 127),
        )
    }
}

/// Convert a slice of `ExpressiveNote`s into a frame-baked `midi::Pattern`.
pub fn notes_to_midi_pattern(
    notes: &[ExpressiveNote],
    total_beats: Time,
    instrument: midi::Instrument,
    tempo: &TempoMap,
    swing_pct: u8,
) -> Result<midi::Pattern, midi::Error> {
    if total_beats <= Time::ZERO || notes.len() > midi::MAX_NOTES {
        return Err(midi::Error::Range);
    }
    let total_frames_u64 = tempo.beat_to_frame(total_beats, swing_pct);
    if total_frames_u64 == 0 || total_frames_u64 > midi::MAX_FRAMES as u64 {
        return Err(midi::Error::Range);
    }
    let total_frames = total_frames_u64 as u32;
    let bpm = (tempo.bpm_x100_at(Time::ZERO) / 100).clamp(30, 240) as u16;
    let mut midi_notes = Vec::new();
    midi_notes
        .try_reserve(notes.len())
        .map_err(|_| midi::Error::Memory)?;
    for n in notes {
        let mut mn = n.to_midi_note(tempo, swing_pct)?;
        if mn.start >= total_frames {
            return Err(midi::Error::Note);
        }
        let max_len = total_frames - mn.start;
        if mn.frames > max_len {
            mn.frames = max_len;
        }
        midi_notes.push(mn);
    }
    let pat = midi::Pattern {
        frames: total_frames,
        bpm,
        instrument,
        notes: midi_notes,
    };
    pat.validate()?;
    Ok(pat)
}

/// Convert a `midi::Pattern` into `ExpressiveNote`s + constant `TempoMap` + total beat length.
pub fn midi_pattern_to_notes(
    pattern: &midi::Pattern,
) -> Result<(Vec<ExpressiveNote>, TempoMap, Time), midi::Error> {
    pattern.validate()?;
    let tempo = TempoMap::constant(pattern.bpm as u32 * 100).map_err(|_| midi::Error::Range)?;
    let mut out = Vec::new();
    out.try_reserve_exact(pattern.notes.len())
        .map_err(|_| midi::Error::Memory)?;
    for (idx, &n) in pattern.notes.iter().enumerate() {
        out.push(ExpressiveNote::from_midi_note((idx + 1) as u32, n, &tempo, 0));
    }
    let total_beats = tempo
        .frame_to_beat(pattern.frames as u64, 0)
        .max(Time::MIN_BEAT_STEP);
    Ok((out, tempo, total_beats))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArpOrder {
    Up,
    Down,
    UpDown,
    Random,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrumDir {
    Up,
    Down,
    Alternate,
}

/// Pure, deterministic Piano Roll compositional note transforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeTool {
    Legato {
        gap: Time,
    },
    Articulate {
        scale_pct: u16,
    },
    Chop {
        division: Time,
        gate_pct: u8,
    },
    Arpeggiate {
        step: Time,
        order: ArpOrder,
        octaves: u8,
        gate_pct: u8,
        seed: u64,
    },
    Strum {
        spread: Time,
        direction: StrumDir,
        vel_falloff_pct: u8,
    },
    Flam {
        count: u8,
        spacing: Time,
        vel_pct: u8,
    },
    Humanize {
        max_timing_ticks: u16,
        max_vel_delta: u8,
        max_len_pct: u8,
        seed: u64,
    },
    Levels {
        multiply_pct: u16,
        offset: i8,
    },
    Flip {
        reverse_time: bool,
        mirror_pitch: bool,
    },
    Tidy,
}

#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
fn clamp_min_len(t: Time) -> Time {
    if t < Time::MIN_BEAT_STEP {
        Time::MIN_BEAT_STEP
    } else {
        t
    }
}

fn validate_tool_and_notes(notes: &[ExpressiveNote], tool: &ComposeTool) -> Result<(), Error> {
    if notes.len() > MAX_COMPOSE_NOTES {
        return Err(Error::Range);
    }
    for n in notes {
        n.validate()?;
    }
    match *tool {
        ComposeTool::Legato { gap } => {
            if gap.is_negative() {
                return Err(Error::Range);
            }
        }
        ComposeTool::Articulate { .. } => {}
        ComposeTool::Chop { division, .. } => {
            if division <= Time::ZERO {
                return Err(Error::Range);
            }
        }
        ComposeTool::Arpeggiate { step, octaves, .. } => {
            if step <= Time::ZERO || octaves == 0 {
                return Err(Error::Range);
            }
        }
        ComposeTool::Strum { spread, .. } => {
            if spread.is_negative() {
                return Err(Error::Range);
            }
        }
        ComposeTool::Flam { count, spacing, .. } => {
            if count > 0 && spacing <= Time::ZERO {
                return Err(Error::Range);
            }
        }
        ComposeTool::Humanize { .. }
        | ComposeTool::Levels { .. }
        | ComposeTool::Flip { .. }
        | ComposeTool::Tidy => {}
    }
    Ok(())
}

/// Apply a deterministic `ComposeTool` transform to a slice of `ExpressiveNote`s.
pub fn apply_tool(
    notes: &[ExpressiveNote],
    tool: &ComposeTool,
) -> Result<Vec<ExpressiveNote>, Error> {
    validate_tool_and_notes(notes, tool)?;
    if notes.is_empty() {
        return Ok(Vec::new());
    }

    match *tool {
        ComposeTool::Legato { gap } => apply_legato(notes, gap),
        ComposeTool::Articulate { scale_pct } => apply_articulate(notes, scale_pct),
        ComposeTool::Chop { division, gate_pct } => apply_chop(notes, division, gate_pct),
        ComposeTool::Arpeggiate {
            step,
            order,
            octaves,
            gate_pct,
            seed,
        } => apply_arpeggiate(notes, step, order, octaves, gate_pct, seed),
        ComposeTool::Strum {
            spread,
            direction,
            vel_falloff_pct,
        } => apply_strum(notes, spread, direction, vel_falloff_pct),
        ComposeTool::Flam {
            count,
            spacing,
            vel_pct,
        } => apply_flam(notes, count, spacing, vel_pct),
        ComposeTool::Humanize {
            max_timing_ticks,
            max_vel_delta,
            max_len_pct,
            seed,
        } => apply_humanize(notes, max_timing_ticks, max_vel_delta, max_len_pct, seed),
        ComposeTool::Levels {
            multiply_pct,
            offset,
        } => apply_levels(notes, multiply_pct, offset),
        ComposeTool::Flip {
            reverse_time,
            mirror_pitch,
        } => apply_flip(notes, reverse_time, mirror_pitch),
        ComposeTool::Tidy => apply_tidy(notes),
    }
}

/// Apply any `ComposeTool` directly to a `midi::Pattern`, keeping valid frame & note budgets.
pub fn apply_tool_to_midi(
    pattern: &midi::Pattern,
    tool: &ComposeTool,
) -> Result<midi::Pattern, midi::Error> {
    let (expr_notes, tempo, _total_beats) = midi_pattern_to_notes(pattern)?;
    let transformed = apply_tool(&expr_notes, tool).map_err(|e| match e {
        Error::Memory => midi::Error::Memory,
        _ => midi::Error::Range,
    })?;
    if transformed.len() > midi::MAX_NOTES {
        return Err(midi::Error::Range);
    }
    let mut midi_notes = Vec::new();
    midi_notes
        .try_reserve(transformed.len())
        .map_err(|_| midi::Error::Memory)?;
    for n in &transformed {
        let mut mn = n.to_midi_note(&tempo, 0)?;
        if mn.start >= pattern.frames {
            continue;
        }
        let max_len = pattern.frames - mn.start;
        if mn.frames > max_len {
            mn.frames = max_len;
        }
        midi_notes.push(mn);
    }
    let out = midi::Pattern {
        frames: pattern.frames,
        bpm: pattern.bpm,
        instrument: pattern.instrument,
        notes: midi_notes,
    };
    out.validate()?;
    Ok(out)
}

fn apply_legato(notes: &[ExpressiveNote], gap: Time) -> Result<Vec<ExpressiveNote>, Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(notes.len())
        .map_err(|_| Error::Memory)?;
    out.extend_from_slice(notes);
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)).then(a.id.cmp(&b.id)));

    // Collect distinct sorted onset times so chords extend together to the next onset.
    let mut onsets: Vec<Time> = Vec::new();
    onsets
        .try_reserve(out.len())
        .map_err(|_| Error::Memory)?;
    for n in &out {
        if onsets.last().copied() != Some(n.start) {
            onsets.push(n.start);
        }
    }

    for n in &mut out {
        let idx = onsets.partition_point(|&t| t <= n.start);
        if let Some(&next_onset) = onsets.get(idx) {
            let target_end = next_onset
                .checked_sub(gap)
                .unwrap_or(Time::ZERO)
                .max(n.start.saturating_quantized_add(Time::MIN_BEAT_STEP).ok_or(Error::Range)?);
            if let Some(new_len) = target_end.checked_sub(n.start) {
                if new_len > Time::ZERO {
                    n.length = new_len;
                }
            }
        }
    }
    Ok(out)
}

fn apply_articulate(
    notes: &[ExpressiveNote],
    scale_pct: u16,
) -> Result<Vec<ExpressiveNote>, Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(notes.len())
        .map_err(|_| Error::Memory)?;
    for &n in notes {
        let mut copy = n;
        let scaled = n
            .length
            .mul_frac_nearest(scale_pct as i64, 100)
            .ok_or(Error::Range)?;
        copy.length = clamp_min_len(scaled);
        out.push(copy);
    }
    Ok(out)
}

fn apply_chop(
    notes: &[ExpressiveNote],
    division: Time,
    gate_pct: u8,
) -> Result<Vec<ExpressiveNote>, Error> {
    let gate = gate_pct.clamp(1, 100) as i64;
    let mut next_id = notes.iter().map(|n| n.id).max().unwrap_or(0).saturating_add(1);
    let mut out = Vec::new();
    out.try_reserve(notes.len())
        .map_err(|_| Error::Memory)?;

    for &n in notes {
        let note_end = n.end().ok_or(Error::Range)?;
        let mut k: i64 = 0;
        loop {
            let step_offset = division.mul_int(k).ok_or(Error::Range)?;
            let slice_start = n
                .start
                .saturating_quantized_add(step_offset)
                .ok_or(Error::Range)?;
            if slice_start >= note_end {
                break;
            }
            if out.len() >= MAX_COMPOSE_NOTES {
                return Err(Error::Range);
            }
            let remaining = note_end.checked_sub(slice_start).ok_or(Error::Range)?;
            let avail = remaining.min(division);
            let raw_len = avail
                .mul_frac_nearest(gate, 100)
                .ok_or(Error::Range)?;
            let min_allowed = Time::MIN_BEAT_STEP.min(avail);
            let slice_len = raw_len.max(min_allowed);
            let next_slice_start = slice_start
                .saturating_quantized_add(division)
                .ok_or(Error::Range)?;
            let is_last = next_slice_start >= note_end;
            let id = if k == 0 {
                n.id
            } else {
                let assigned = next_id;
                next_id = next_id.saturating_add(1);
                assigned
            };
            out.try_reserve(1).map_err(|_| Error::Memory)?;
            out.push(ExpressiveNote {
                id,
                start: slice_start,
                length: slice_len,
                key: n.key,
                velocity: n.velocity,
                pan: n.pan,
                fine_cents: n.fine_cents,
                slide_to_key: if is_last { n.slide_to_key } else { None },
                timbre: n.timbre,
                pressure: n.pressure,
            });
            k += 1;
        }
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)).then(a.id.cmp(&b.id)));
    Ok(out)
}

fn apply_arpeggiate(
    notes: &[ExpressiveNote],
    step: Time,
    order: ArpOrder,
    octaves: u8,
    gate_pct: u8,
    seed: u64,
) -> Result<Vec<ExpressiveNote>, Error> {
    let octaves = octaves.clamp(1, 8);
    let gate = gate_pct.clamp(1, 100) as i64;

    // Slice timeline across all note boundaries so chords and progressions expand cleanly.
    let mut bounds = Vec::new();
    bounds
        .try_reserve(notes.len() * 2)
        .map_err(|_| Error::Memory)?;
    for n in notes {
        bounds.push(n.start);
        bounds.push(n.end().ok_or(Error::Range)?);
    }
    bounds.sort();
    bounds.dedup();

    let mut next_id = notes.iter().map(|n| n.id).max().unwrap_or(0).saturating_add(1);
    let mut rng = seed ^ 0xA5A5_5A5A_1234_5678;
    let mut out = Vec::new();

    for w in bounds.windows(2) {
        let seg_start = w[0];
        let seg_end = w[1];
        if seg_end <= seg_start {
            continue;
        }

        // Collect active notes spanning [seg_start, seg_end).
        let mut pool: Vec<ExpressiveNote> = Vec::new();
        for &n in notes {
            let n_end = n.end().ok_or(Error::Range)?;
            if n.start <= seg_start && n_end >= seg_end {
                pool.try_reserve(1).map_err(|_| Error::Memory)?;
                pool.push(n);
            }
        }
        if pool.is_empty() {
            continue;
        }

        let mut expanded: Vec<ExpressiveNote> = Vec::new();
        expanded
            .try_reserve(pool.len() * octaves as usize)
            .map_err(|_| Error::Memory)?;
        for oct in 0..octaves {
            for &src in &pool {
                let k = src.key as u16 + (oct as u16) * 12;
                if k <= 127 {
                    let mut entry = src;
                    entry.key = k as u8;
                    expanded.push(entry);
                }
            }
        }
        if expanded.is_empty() {
            continue;
        }
        expanded.sort_by(|a, b| a.key.cmp(&b.key).then(a.id.cmp(&b.id)));
        expanded.dedup_by_key(|n| n.key);

        let n_keys = expanded.len();
        let mut k: i64 = 0;
        loop {
            let step_off = step.mul_int(k).ok_or(Error::Range)?;
            let note_start = seg_start
                .saturating_quantized_add(step_off)
                .ok_or(Error::Range)?;
            if note_start >= seg_end {
                break;
            }
            if out.len() >= MAX_COMPOSE_NOTES {
                return Err(Error::Range);
            }
            let rem = seg_end.checked_sub(note_start).ok_or(Error::Range)?;
            let avail = rem.min(step);
            let raw_len = avail
                .mul_frac_nearest(gate, 100)
                .ok_or(Error::Range)?;
            let note_len = raw_len.max(Time::MIN_BEAT_STEP.min(avail));

            let idx = match order {
                ArpOrder::Up => (k as usize) % n_keys,
                ArpOrder::Down => n_keys - 1 - ((k as usize) % n_keys),
                ArpOrder::UpDown => {
                    if n_keys <= 1 {
                        0
                    } else {
                        let period = 2 * n_keys - 2;
                        let phase = (k as usize) % period;
                        if phase < n_keys {
                            phase
                        } else {
                            period - phase
                        }
                    }
                }
                ArpOrder::Random => (splitmix64(&mut rng) as usize) % n_keys,
            };

            let src = expanded[idx];
            out.try_reserve(1).map_err(|_| Error::Memory)?;
            out.push(ExpressiveNote {
                id: next_id,
                start: note_start,
                length: note_len,
                key: src.key,
                velocity: src.velocity,
                pan: src.pan,
                fine_cents: src.fine_cents,
                slide_to_key: None,
                timbre: src.timbre,
                pressure: src.pressure,
            });
            next_id = next_id.saturating_add(1);
            k += 1;
        }
    }

    Ok(out)
}

fn apply_strum(
    notes: &[ExpressiveNote],
    spread: Time,
    direction: StrumDir,
    vel_falloff_pct: u8,
) -> Result<Vec<ExpressiveNote>, Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(notes.len())
        .map_err(|_| Error::Memory)?;
    out.extend_from_slice(notes);
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)).then(a.id.cmp(&b.id)));

    let falloff_cap = vel_falloff_pct.min(95) as u32;
    let mut i = 0usize;
    let mut chord_idx = 0usize;
    while i < out.len() {
        let onset = out[i].start;
        let mut j = i + 1;
        while j < out.len() && out[j].start == onset {
            j += 1;
        }
        let count = j - i;
        if count >= 2 {
            let up = match direction {
                StrumDir::Up => true,
                StrumDir::Down => false,
                StrumDir::Alternate => chord_idx % 2 == 0,
            };
            if !up {
                out[i..j].reverse();
            }
            let denom = (count - 1) as u32;
            for (rank, note) in out[i..j].iter_mut().enumerate() {
                let orig_end = note.end().ok_or(Error::Range)?;
                let offset = spread
                    .mul_frac_nearest(rank as i64, denom)
                    .ok_or(Error::Range)?;
                let desired_start = onset
                    .saturating_quantized_add(offset)
                    .ok_or(Error::Range)?;
                // Preserve note end intact while clamping start so length stays >= MIN_BEAT_STEP (or orig length).
                let min_len = Time::MIN_BEAT_STEP.min(note.length);
                let max_start = orig_end.checked_sub(min_len).unwrap_or(onset);
                let new_start = desired_start.min(max_start);
                let new_len = orig_end
                    .checked_sub(new_start)
                    .unwrap_or(min_len)
                    .max(min_len);
                note.start = new_start;
                note.length = new_len;

                let vel_pct = 100u32.saturating_sub(falloff_cap * (rank as u32) / denom);
                note.velocity = ((note.velocity as u32 * vel_pct) / 100).clamp(1, 127) as u8;
            }
        }
        chord_idx += 1;
        i = j;
    }

    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)).then(a.id.cmp(&b.id)));
    Ok(out)
}

fn apply_flam(
    notes: &[ExpressiveNote],
    count: u8,
    spacing: Time,
    vel_pct: u8,
) -> Result<Vec<ExpressiveNote>, Error> {
    if count == 0 {
        let mut copy = Vec::new();
        copy.try_reserve_exact(notes.len())
            .map_err(|_| Error::Memory)?;
        copy.extend_from_slice(notes);
        return Ok(copy);
    }
    let total_needed = notes
        .len()
        .checked_mul(count as usize + 1)
        .ok_or(Error::Range)?;
    if total_needed > MAX_COMPOSE_NOTES {
        return Err(Error::Range);
    }

    let mut next_id = notes.iter().map(|n| n.id).max().unwrap_or(0).saturating_add(1);
    let mut out = Vec::new();
    out.try_reserve_exact(total_needed)
        .map_err(|_| Error::Memory)?;

    let lead = spacing.mul_int(count as i64).ok_or(Error::Range)?;
    let vel_pct_u32 = vel_pct.clamp(1, 100) as u32;

    for &n in notes {
        // Keep main note at exact start when there is room; otherwise shift just enough to avoid < 0.
        let main_start = if n.start >= lead { n.start } else { lead };
        let mut cur_vel = n.velocity as u32;
        for step_back in 1..=count {
            cur_vel = ((cur_vel * vel_pct_u32) / 100).clamp(1, 127);
            let off = spacing.mul_int(step_back as i64).ok_or(Error::Range)?;
            let grace_start = main_start.checked_sub(off).ok_or(Error::Range)?;
            out.push(ExpressiveNote {
                id: next_id,
                start: grace_start,
                length: spacing,
                key: n.key,
                velocity: cur_vel as u8,
                pan: n.pan,
                fine_cents: n.fine_cents,
                slide_to_key: None,
                timbre: n.timbre,
                pressure: n.pressure,
            });
            next_id = next_id.saturating_add(1);
        }
        let mut main_note = n;
        main_note.start = main_start;
        out.push(main_note);
    }

    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)).then(a.id.cmp(&b.id)));
    Ok(out)
}

fn apply_humanize(
    notes: &[ExpressiveNote],
    max_timing_ticks: u16,
    max_vel_delta: u8,
    max_len_pct: u8,
    seed: u64,
) -> Result<Vec<ExpressiveNote>, Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(notes.len())
        .map_err(|_| Error::Memory)?;
    let mut rng = seed ^ 0x51A7_BA55_DEAD_BEEF;

    for &n in notes {
        let mut copy = n;
        if max_timing_ticks > 0 {
            let span = (max_timing_ticks as i64) * 2 + 1;
            let r = (splitmix64(&mut rng) % (span as u64)) as i64;
            let tick_delta = r - max_timing_ticks as i64;
            let delta = Time::from_display_ticks(tick_delta);
            let shifted = n
                .start
                .saturating_quantized_add(delta)
                .ok_or(Error::Range)?;
            copy.start = shifted.max(Time::ZERO);
        }
        if max_vel_delta > 0 {
            let span = (max_vel_delta as u64) * 2 + 1;
            let r = (splitmix64(&mut rng) % span) as i16;
            let dv = r - max_vel_delta as i16;
            copy.velocity = (n.velocity as i16 + dv).clamp(1, 127) as u8;
        }
        if max_len_pct > 0 {
            let cap = max_len_pct.min(90) as i64;
            let span = (cap * 2 + 1) as u64;
            let r = (splitmix64(&mut rng) % span) as i64;
            let dpct = r - cap;
            let factor = (100 + dpct).max(10);
            let scaled = n
                .length
                .mul_frac_nearest(factor, 100)
                .ok_or(Error::Range)?;
            copy.length = clamp_min_len(scaled);
        }
        out.push(copy);
    }

    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)).then(a.id.cmp(&b.id)));
    Ok(out)
}

fn apply_levels(
    notes: &[ExpressiveNote],
    multiply_pct: u16,
    offset: i8,
) -> Result<Vec<ExpressiveNote>, Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(notes.len())
        .map_err(|_| Error::Memory)?;
    for &n in notes {
        let mut copy = n;
        let scaled = (n.velocity as i32 * multiply_pct as i32) / 100 + offset as i32;
        copy.velocity = scaled.clamp(1, 127) as u8;
        out.push(copy);
    }
    Ok(out)
}

fn apply_flip(
    notes: &[ExpressiveNote],
    reverse_time: bool,
    mirror_pitch: bool,
) -> Result<Vec<ExpressiveNote>, Error> {
    let mut out = Vec::new();
    out.try_reserve_exact(notes.len())
        .map_err(|_| Error::Memory)?;

    let mut min_start = notes[0].start;
    let mut max_end = notes[0].end().ok_or(Error::Range)?;
    let mut min_key = notes[0].key;
    let mut max_key = notes[0].key;

    for n in notes {
        if n.start < min_start {
            min_start = n.start;
        }
        let end = n.end().ok_or(Error::Range)?;
        if end > max_end {
            max_end = end;
        }
        min_key = min_key.min(n.key);
        max_key = max_key.max(n.key);
    }

    let sum_key = min_key as u16 + max_key as u16;

    for &n in notes {
        let mut copy = n;
        if reverse_time {
            let n_end = n.end().ok_or(Error::Range)?;
            let tail = max_end.checked_sub(n_end).ok_or(Error::Range)?;
            copy.start = min_start
                .saturating_quantized_add(tail)
                .ok_or(Error::Range)?;
        }
        if mirror_pitch {
            copy.key = (sum_key.saturating_sub(n.key as u16)).clamp(0, 127) as u8;
            if let Some(sk) = n.slide_to_key {
                copy.slide_to_key = Some((sum_key.saturating_sub(sk as u16)).clamp(0, 127) as u8);
            }
        }
        out.push(copy);
    }

    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)).then(a.id.cmp(&b.id)));
    Ok(out)
}

fn apply_tidy(notes: &[ExpressiveNote]) -> Result<Vec<ExpressiveNote>, Error> {
    let mut sorted = Vec::new();
    sorted
        .try_reserve_exact(notes.len())
        .map_err(|_| Error::Memory)?;
    sorted.extend_from_slice(notes);
    // For identical (start, key), keep longer then higher velocity note first.
    sorted.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(a.key.cmp(&b.key))
            .then(b.length.cmp(&a.length))
            .then(b.velocity.cmp(&a.velocity))
            .then(a.id.cmp(&b.id))
    });

    // Deduplicate identical (start, key) duplicates, then trim same-pitch overlaps.
    // Deliberate cross-pitch overlaps (different keys) are preserved for legato slides.
    let mut out: Vec<ExpressiveNote> = Vec::new();
    out.try_reserve(sorted.len()).map_err(|_| Error::Memory)?;

    for note in sorted {
        // Check against the most recent note with the same pitch.
        if let Some(prev) = out.iter_mut().rev().find(|p| p.key == note.key) {
            if prev.start == note.start {
                // Exact onset + pitch duplicate: drop duplicate.
                continue;
            }
            let prev_end = prev.end().ok_or(Error::Range)?;
            if prev_end > note.start {
                let trimmed = note.start.checked_sub(prev.start).ok_or(Error::Range)?;
                if trimmed > Time::ZERO {
                    prev.length = trimmed;
                }
            }
        }
        out.push(note);
    }

    // Ensure any very rare sub-step overlap isn't zero-length and keep chronological order.
    out.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)).then(a.id.cmp(&b.id)));
    Ok(out)
}
