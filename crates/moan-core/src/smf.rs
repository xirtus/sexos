// SPDX-License-Identifier: GPL-3.0-only
//! Zero-dependency `#![no_std]` Standard MIDI File (`.mid`) Format 0 & 1
//! importer and Format 1 (`960 PPQ`) exporter.
//! Enforces strict memory & note budgets (`<= 4 MiB`, `<= 32` tracks, `<= 4096` notes)
//! and never panics on corrupt or truncated input.

use crate::{
    compose::ExpressiveNote,
    midi,
    pattern::{ChannelId, PatternId, RackChannel, TapestryBank, TapestryPattern, MAX_TRACKS},
    time::{TempoMap, TempoSegment, Time, TimeSignature, DISPLAY_PPQ, MAX_BPM_X100, MIN_BPM_X100},
    Error,
};
use alloc::{string::String, vec::Vec};

pub const MAX_SMF_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SMF_TRACKS: usize = 32;
pub const MAX_SMF_NOTES: usize = 4096;
const MAX_TRACK_NAME_BYTES: usize = 256;
const MAX_PITCH_BENDS_PER_TRACK: usize = 4096;
const MAX_UNKNOWN_CHUNKS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmfError {
    TooLarge,
    InvalidHeader,
    UnsupportedFormat,
    Truncated,
    CorruptTrack,
    TooManyTracks,
    TooManyNotes,
    InvalidTiming,
    Memory,
}

impl From<Error> for SmfError {
    fn from(e: Error) -> Self {
        match e {
            Error::Memory => SmfError::Memory,
            _ => SmfError::InvalidTiming,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmfTrack {
    pub name: String,
    pub midi_channel: Option<u8>,
    pub program: Option<u8>,
    /// `true` if any note/event on MIDI channel 10 (`ch == 9`) was seen in this track.
    pub is_percussion: bool,
    pub notes: Vec<ExpressiveNote>,
    /// Signed 14-bit pitch bend events `(beat_time, value_in_-8192..=8191)`.
    pub pitch_bends: Vec<(Time, i16)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmfImport {
    pub format: u16,
    pub division_ppq: u16,
    pub tempo_map: TempoMap,
    pub tracks: Vec<SmfTrack>,
}

impl SmfImport {
    /// Materialize imported SMF tracks into a `TapestryBank` with one `TapestryPattern`.
    pub fn to_tapestry_bank(&self) -> Result<TapestryBank, Error> {
        let mut bank = TapestryBank::new();
        bank.tempo_map = self.tempo_map.clone();
        let pat_id = bank.create_pattern("Imported SMF", 1, 4)?;
        let mut max_end = Time::from_beats(4);

        for (idx, trk) in self.tracks.iter().enumerate() {
            if idx >= crate::pattern::MAX_RACK_CHANNELS {
                break;
            }
            if trk.notes.is_empty() && self.tracks.len() > 1 {
                continue;
            }
            let inst = if trk.is_percussion {
                midi::Instrument::VelvetSampler as u8
            } else {
                match trk.program.unwrap_or(0) % 5 {
                    0 => midi::Instrument::SilkTriangle as u8,
                    1 => midi::Instrument::SatinPulse as u8,
                    2 => midi::Instrument::VelvetSampler as u8,
                    3 => midi::Instrument::ArielFormant as u8,
                    _ => midi::Instrument::PrismWavetable as u8,
                }
            };
            let label = if trk.name.is_empty() {
                String::from("SMF Track")
            } else {
                trk.name.clone()
            };
            let ch_id = bank.add_channel(label, inst)?;
            if let Some(ch) = bank.channel_mut(ch_id) {
                ch.target_track = (idx as u8) % MAX_TRACKS;
            }
            if let Some(pat) = bank.pattern_mut(pat_id) {
                for &n in &trk.notes {
                    if let Some(end) = n.end() {
                        if end > max_end {
                            max_end = end;
                        }
                    }
                    pat.add_note(ch_id, n)?;
                }
            }
        }

        let bpb = bank.tempo_map.time_signature_at(Time::ZERO).beats_per_bar();
        let bars_needed = ((max_end.to_f64() / bpb.to_f64()) + 0.999_999) as i64;
        if let Some(pat) = bank.pattern_mut(pat_id) {
            pat.bars = bars_needed.clamp(1, crate::pattern::MAX_BARS as i64) as u8;
        }
        bank.active_pattern = pat_id;
        Ok(bank)
    }
}

#[inline]
fn read_be_u16(bytes: &[u8], offset: usize) -> Result<u16, SmfError> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or(SmfError::Truncated)?;
    Ok(u16::from_be_bytes([slice[0], slice[1]]))
}

#[inline]
fn read_be_u32(bytes: &[u8], offset: usize) -> Result<u32, SmfError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or(SmfError::Truncated)?;
    Ok(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[inline]
fn read_vlq(data: &[u8], cursor: &mut usize) -> Result<u32, SmfError> {
    let mut val: u32 = 0;
    for i in 0..4 {
        let b = *data.get(*cursor).ok_or(SmfError::Truncated)?;
        *cursor += 1;
        val = (val << 7) | ((b & 0x7F) as u32);
        if (b & 0x80) == 0 {
            return Ok(val);
        }
        if i == 3 {
            return Err(SmfError::CorruptTrack);
        }
    }
    Err(SmfError::CorruptTrack)
}

#[inline]
fn read_u8(data: &[u8], cursor: &mut usize) -> Result<u8, SmfError> {
    let b = *data.get(*cursor).ok_or(SmfError::Truncated)?;
    *cursor += 1;
    Ok(b)
}

#[derive(Clone, Copy)]
struct ActiveSmfNote {
    channel: u8,
    key: u8,
    velocity: u8,
    start: Time,
    pressure: u8,
}

fn close_active_note(
    active: &mut Vec<ActiveSmfNote>,
    out_notes: &mut Vec<ExpressiveNote>,
    next_id: &mut u32,
    ch: u8,
    key: u8,
    off_time: Time,
) -> Result<(), SmfError> {
    if let Some(pos) = active
        .iter()
        .position(|a| a.channel == ch && a.key == key)
    {
        let note = active.remove(pos);
        let len = off_time
            .checked_sub(note.start)
            .filter(|l| *l > Time::ZERO)
            .unwrap_or(Time::MIN_BEAT_STEP);
        out_notes.try_reserve(1).map_err(|_| SmfError::Memory)?;
        let mut expr = ExpressiveNote::new(*next_id, note.start, len, note.key, note.velocity);
        expr.pressure = note.pressure;
        *next_id = next_id.saturating_add(1);
        out_notes.push(expr);
    }
    Ok(())
}

/// Parse a Standard MIDI File (`.mid`, Format 0 or Format 1) with strict bounds and `try_reserve`.
pub fn import_smf(bytes: &[u8]) -> Result<SmfImport, SmfError> {
    if bytes.len() > MAX_SMF_BYTES {
        return Err(SmfError::TooLarge);
    }
    if bytes.len() < 14 {
        return Err(SmfError::Truncated);
    }
    if &bytes[0..4] != b"MThd" {
        return Err(SmfError::InvalidHeader);
    }
    let header_len = read_be_u32(bytes, 4)? as usize;
    if header_len < 6 {
        return Err(SmfError::InvalidHeader);
    }
    let header_end = 8usize
        .checked_add(header_len)
        .ok_or(SmfError::InvalidHeader)?;
    if header_end > bytes.len() {
        return Err(SmfError::Truncated);
    }

    let format = read_be_u16(bytes, 8)?;
    if format > 1 {
        return Err(SmfError::UnsupportedFormat);
    }
    let num_tracks = read_be_u16(bytes, 10)? as usize;
    if num_tracks == 0 {
        return Err(SmfError::InvalidHeader);
    }
    if num_tracks > MAX_SMF_TRACKS {
        return Err(SmfError::TooManyTracks);
    }
    let division = read_be_u16(bytes, 12)?;
    if division == 0 || (division & 0x8000) != 0 {
        return Err(SmfError::InvalidTiming);
    }
    let ppq = division;

    let mut pos = header_end;
    let mut tracks = Vec::new();
    tracks
        .try_reserve_exact(num_tracks)
        .map_err(|_| SmfError::Memory)?;
    let mut tempo_segs: Vec<TempoSegment> = Vec::new();
    let mut time_sigs: Vec<(Time, TimeSignature)> = Vec::new();
    let mut total_notes: usize = 0;
    let mut unknown_chunks = 0usize;

    while tracks.len() < num_tracks {
        if pos + 8 > bytes.len() {
            return Err(SmfError::Truncated);
        }
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_len = read_be_u32(bytes, pos + 4)? as usize;
        let data_start = pos + 8;
        let data_end = data_start
            .checked_add(chunk_len)
            .ok_or(SmfError::CorruptTrack)?;
        if data_end > bytes.len() {
            return Err(SmfError::Truncated);
        }
        pos = data_end;

        if chunk_id != b"MTrk" {
            unknown_chunks += 1;
            if unknown_chunks > MAX_UNKNOWN_CHUNKS {
                return Err(SmfError::CorruptTrack);
            }
            continue;
        }

        let trk = &bytes[data_start..data_end];
        let parsed = parse_smf_track(
            trk,
            ppq,
            &mut total_notes,
            &mut tempo_segs,
            &mut time_sigs,
        )?;
        tracks.push(parsed);
    }

    if !tempo_segs.is_empty() && !tempo_segs.iter().any(|s| s.start == Time::ZERO) {
        tempo_segs.try_reserve(1).map_err(|_| SmfError::Memory)?;
        tempo_segs.push(TempoSegment {
            start: Time::ZERO,
            bpm_x100: 12_000,
            ramp_to_bpm_x100: None,
        });
    }
    if !time_sigs.is_empty() && !time_sigs.iter().any(|(at, _)| *at == Time::ZERO) {
        time_sigs.try_reserve(1).map_err(|_| SmfError::Memory)?;
        time_sigs.push((Time::ZERO, TimeSignature::FOUR_FOUR));
    }

    let tempo_map = TempoMap::with_time_signatures(tempo_segs, time_sigs)
        .map_err(SmfError::from)?;

    Ok(SmfImport {
        format,
        division_ppq: ppq,
        tempo_map,
        tracks,
    })
}

fn parse_smf_track(
    trk: &[u8],
    ppq: u16,
    total_notes: &mut usize,
    tempo_segs: &mut Vec<TempoSegment>,
    time_sigs: &mut Vec<(Time, TimeSignature)>,
) -> Result<SmfTrack, SmfError> {
    let mut cursor = 0usize;
    let mut abs_tick: u64 = 0;
    let mut last_time = Time::ZERO;
    let mut running_status: Option<u8> = None;

    let mut name = String::new();
    let mut midi_channel: Option<u8> = None;
    let mut program: Option<u8> = None;
    let mut is_percussion = false;
    let mut notes: Vec<ExpressiveNote> = Vec::new();
    let mut pitch_bends: Vec<(Time, i16)> = Vec::new();
    let mut active: Vec<ActiveSmfNote> = Vec::new();
    let mut next_note_id: u32 = 1;

    while cursor < trk.len() {
        let delta = read_vlq(trk, &mut cursor)?;
        abs_tick = abs_tick
            .checked_add(delta as u64)
            .ok_or(SmfError::CorruptTrack)?;
        let tick_i64 = i64::try_from(abs_tick).map_err(|_| SmfError::CorruptTrack)?;
        let time = Time::from_ticks_nearest(tick_i64, ppq as u32)
            .ok_or(SmfError::InvalidTiming)?;
        last_time = time;

        let peek = *trk.get(cursor).ok_or(SmfError::Truncated)?;
        let status = if peek < 0x80 {
            running_status.ok_or(SmfError::CorruptTrack)?
        } else {
            cursor += 1;
            if peek < 0xF0 {
                running_status = Some(peek);
            } else {
                running_status = None;
            }
            peek
        };

        if status == 0xFF {
            let meta_type = read_u8(trk, &mut cursor)?;
            let meta_len = read_vlq(trk, &mut cursor)? as usize;
            let meta_end = cursor
                .checked_add(meta_len)
                .ok_or(SmfError::CorruptTrack)?;
            let meta_data = trk.get(cursor..meta_end).ok_or(SmfError::Truncated)?;
            cursor = meta_end;

            match meta_type {
                0x03 => {
                    let take = meta_data.len().min(MAX_TRACK_NAME_BYTES);
                    let s = String::from_utf8_lossy(&meta_data[..take]);
                    name.clear();
                    name.try_reserve(s.len()).map_err(|_| SmfError::Memory)?;
                    name.push_str(&s);
                }
                0x51 if meta_len == 3 => {
                    let us = ((meta_data[0] as u32) << 16)
                        | ((meta_data[1] as u32) << 8)
                        | (meta_data[2] as u32);
                    if us > 0 {
                        let bpm_x100 = (6_000_000_000u64 / us as u64)
                            .clamp(MIN_BPM_X100 as u64, MAX_BPM_X100 as u64)
                            as u32;
                        tempo_segs.try_reserve(1).map_err(|_| SmfError::Memory)?;
                        tempo_segs.push(TempoSegment {
                            start: time,
                            bpm_x100,
                            ramp_to_bpm_x100: None,
                        });
                    }
                }
                0x58 if meta_len == 4 => {
                    let num = meta_data[0].max(1);
                    let den_pow = meta_data[1];
                    if den_pow <= 6 {
                        let sig = TimeSignature {
                            numerator: num,
                            denominator: 1u8 << den_pow,
                        };
                        if sig.is_valid() {
                            time_sigs.try_reserve(1).map_err(|_| SmfError::Memory)?;
                            time_sigs.push((time, sig));
                        }
                    }
                }
                0x2F => break,
                _ => {}
            }
            continue;
        }

        if status == 0xF0 || status == 0xF7 {
            let syx_len = read_vlq(trk, &mut cursor)? as usize;
            let syx_end = cursor
                .checked_add(syx_len)
                .ok_or(SmfError::CorruptTrack)?;
            if syx_end > trk.len() {
                return Err(SmfError::Truncated);
            }
            cursor = syx_end;
            continue;
        }

        if status >= 0xF1 {
            match status {
                0xF2 => {
                    read_u8(trk, &mut cursor)?;
                    read_u8(trk, &mut cursor)?;
                }
                0xF1 | 0xF3 => {
                    read_u8(trk, &mut cursor)?;
                }
                _ => {}
            }
            continue;
        }

        let ch = status & 0x0F;
        if midi_channel.is_none() {
            midi_channel = Some(ch);
        }
        if ch == 9 {
            is_percussion = true;
        }

        match status & 0xF0 {
            0x80 => {
                let key = read_u8(trk, &mut cursor)? & 0x7F;
                let _vel = read_u8(trk, &mut cursor)?;
                close_active_note(
                    &mut active,
                    &mut notes,
                    &mut next_note_id,
                    ch,
                    key,
                    time,
                )?;
            }
            0x90 => {
                let key = read_u8(trk, &mut cursor)? & 0x7F;
                let vel = read_u8(trk, &mut cursor)? & 0x7F;
                if vel == 0 {
                    close_active_note(
                        &mut active,
                        &mut notes,
                        &mut next_note_id,
                        ch,
                        key,
                        time,
                    )?;
                } else {
                    // Close any prior unclosed note on the same (ch, key) cleanly before retriggering.
                    close_active_note(
                        &mut active,
                        &mut notes,
                        &mut next_note_id,
                        ch,
                        key,
                        time,
                    )?;
                    if *total_notes >= MAX_SMF_NOTES {
                        return Err(SmfError::TooManyNotes);
                    }
                    *total_notes += 1;
                    active.try_reserve(1).map_err(|_| SmfError::Memory)?;
                    active.push(ActiveSmfNote {
                        channel: ch,
                        key,
                        velocity: vel,
                        start: time,
                        pressure: 0,
                    });
                }
            }
            0xA0 => {
                let key = read_u8(trk, &mut cursor)? & 0x7F;
                let pressure = read_u8(trk, &mut cursor)? & 0x7F;
                if let Some(a) = active
                    .iter_mut()
                    .rev()
                    .find(|a| a.channel == ch && a.key == key)
                {
                    a.pressure = pressure;
                }
            }
            0xB0 => {
                read_u8(trk, &mut cursor)?;
                read_u8(trk, &mut cursor)?;
            }
            0xC0 => {
                let prog = read_u8(trk, &mut cursor)? & 0x7F;
                program = Some(prog);
            }
            0xD0 => {
                read_u8(trk, &mut cursor)?;
            }
            0xE0 => {
                let lsb = read_u8(trk, &mut cursor)? & 0x7F;
                let msb = read_u8(trk, &mut cursor)? & 0x7F;
                let raw = (((msb as i16) << 7) | (lsb as i16)) - 8192;
                if pitch_bends.len() < MAX_PITCH_BENDS_PER_TRACK {
                    pitch_bends.try_reserve(1).map_err(|_| SmfError::Memory)?;
                    pitch_bends.push((time, raw));
                }
            }
            _ => return Err(SmfError::CorruptTrack),
        }
    }

    // Close any dangling NoteOn events at the end of the track.
    let remaining: Vec<ActiveSmfNote> = core::mem::take(&mut active);
    for a in remaining {
        let len = last_time
            .checked_sub(a.start)
            .filter(|l| *l > Time::ZERO)
            .unwrap_or(Time::MIN_BEAT_STEP);
        notes.try_reserve(1).map_err(|_| SmfError::Memory)?;
        let mut expr = ExpressiveNote::new(next_note_id, a.start, len, a.key, a.velocity);
        expr.pressure = a.pressure;
        next_note_id = next_note_id.saturating_add(1);
        notes.push(expr);
    }

    notes.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)).then(a.id.cmp(&b.id)));

    Ok(SmfTrack {
        name,
        midi_channel,
        program,
        is_percussion,
        notes,
        pitch_bends,
    })
}

// ============================================================================
// Standard MIDI File Format 1 (960 PPQ) Exporter
// ============================================================================

fn write_vlq(buf: &mut Vec<u8>, mut val: u32) -> Result<(), SmfError> {
    let mut stack = [0u8; 4];
    let mut len = 0usize;
    stack[0] = (val & 0x7F) as u8;
    len += 1;
    val >>= 7;
    while val > 0 {
        if len >= 4 {
            return Err(SmfError::InvalidTiming);
        }
        stack[len] = ((val & 0x7F) as u8) | 0x80;
        len += 1;
        val >>= 7;
    }
    buf.try_reserve(len).map_err(|_| SmfError::Memory)?;
    for i in (0..len).rev() {
        buf.push(stack[i]);
    }
    Ok(())
}

fn push_chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) -> Result<(), SmfError> {
    if out.len() + 8 + data.len() > MAX_SMF_BYTES {
        return Err(SmfError::TooLarge);
    }
    out.try_reserve(8 + data.len())
        .map_err(|_| SmfError::Memory)?;
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    Ok(())
}

#[derive(Clone, Debug)]
struct RawMidiEvent {
    tick: u32,
    order: u8,
    bytes: Vec<u8>,
}

fn build_conductor_track(tempo_map: &TempoMap, track_label: &str) -> Result<Vec<u8>, SmfError> {
    let mut events: Vec<RawMidiEvent> = Vec::new();

    // Track Name (FF 03)
    {
        let label = track_label.as_bytes();
        let take = label.len().min(MAX_TRACK_NAME_BYTES);
        let mut msg = Vec::new();
        msg.try_reserve(2 + take + 4).map_err(|_| SmfError::Memory)?;
        msg.push(0xFF);
        msg.push(0x03);
        write_vlq(&mut msg, take as u32)?;
        msg.extend_from_slice(&label[..take]);
        events.try_reserve(1).map_err(|_| SmfError::Memory)?;
        events.push(RawMidiEvent {
            tick: 0,
            order: 0,
            bytes: msg,
        });
    }

    // Time signatures (FF 58 04)
    for &(at, sig) in tempo_map.time_signatures() {
        let tick_i64 = at.to_display_ticks().ok_or(SmfError::InvalidTiming)?;
        let tick = u32::try_from(tick_i64.max(0)).map_err(|_| SmfError::InvalidTiming)?;
        let mut msg = Vec::new();
        msg.try_reserve_exact(7).map_err(|_| SmfError::Memory)?;
        msg.extend_from_slice(&[
            0xFF,
            0x58,
            0x04,
            sig.numerator.max(1),
            sig.denominator_log2().min(6),
            24,
            8,
        ]);
        events.try_reserve(1).map_err(|_| SmfError::Memory)?;
        events.push(RawMidiEvent {
            tick,
            order: 1,
            bytes: msg,
        });
    }

    // Tempo segments (FF 51 03); linear ramps are discretized at half-beat resolution.
    let segs = tempo_map.segments();
    for (idx, seg) in segs.iter().enumerate() {
        let push_tempo = |events: &mut Vec<RawMidiEvent>,
                          beat: Time,
                          bpm_x100: u32|
         -> Result<(), SmfError> {
            let tick_i64 = beat.to_display_ticks().ok_or(SmfError::InvalidTiming)?;
            let tick = u32::try_from(tick_i64.max(0)).map_err(|_| SmfError::InvalidTiming)?;
            let us = (6_000_000_000u64 / bpm_x100.max(1) as u64).clamp(1, 0x00FF_FFFF) as u32;
            let mut msg = Vec::new();
            msg.try_reserve_exact(6).map_err(|_| SmfError::Memory)?;
            msg.extend_from_slice(&[
                0xFF,
                0x51,
                0x03,
                ((us >> 16) & 0xFF) as u8,
                ((us >> 8) & 0xFF) as u8,
                (us & 0xFF) as u8,
            ]);
            events.try_reserve(1).map_err(|_| SmfError::Memory)?;
            events.push(RawMidiEvent {
                tick,
                order: 2,
                bytes: msg,
            });
            Ok(())
        };

        push_tempo(&mut events, seg.start, seg.bpm_x100)?;
        if let (Some(target), Some(next)) = (seg.ramp_to_bpm_x100, segs.get(idx + 1)) {
            let span = next.start.checked_sub(seg.start).unwrap_or(Time::ZERO);
            let step = Time::EIGHTH;
            if span > step {
                let mut cur = seg.start.checked_add(step).unwrap_or(next.start);
                let mut steps_emitted = 0usize;
                while cur < next.start && steps_emitted < 256 {
                    let frac = (cur.to_f64() - seg.start.to_f64()) / span.to_f64();
                    let bpm = (seg.bpm_x100 as f64
                        + (target as f64 - seg.bpm_x100 as f64) * frac)
                        as u32;
                    push_tempo(&mut events, cur, bpm)?;
                    steps_emitted += 1;
                    cur = match cur.checked_add(step) {
                        Some(t) => t,
                        None => break,
                    };
                }
            }
        }
    }

    serialize_track_events(events)
}

fn serialize_track_events(mut events: Vec<RawMidiEvent>) -> Result<Vec<u8>, SmfError> {
    events.sort_by(|a, b| a.tick.cmp(&b.tick).then(a.order.cmp(&b.order)));
    let mut track = Vec::new();
    let mut prev_tick = 0u32;
    for ev in events {
        let delta = ev.tick.saturating_sub(prev_tick);
        prev_tick = ev.tick;
        write_vlq(&mut track, delta)?;
        track
            .try_reserve(ev.bytes.len())
            .map_err(|_| SmfError::Memory)?;
        track.extend_from_slice(&ev.bytes);
    }
    // End of Track (FF 2F 00)
    track.try_reserve(4).map_err(|_| SmfError::Memory)?;
    track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
    Ok(track)
}

fn build_note_track(
    name: &str,
    midi_ch: u8,
    program: u8,
    notes: &[(Time, Time, u8, u8)], // (start, end, key, velocity)
) -> Result<Vec<u8>, SmfError> {
    let ch = midi_ch & 0x0F;
    let mut events: Vec<RawMidiEvent> = Vec::new();

    // Track Name (FF 03)
    {
        let label = name.as_bytes();
        let take = label.len().min(MAX_TRACK_NAME_BYTES);
        let mut msg = Vec::new();
        msg.try_reserve(2 + take + 4).map_err(|_| SmfError::Memory)?;
        msg.push(0xFF);
        msg.push(0x03);
        write_vlq(&mut msg, take as u32)?;
        msg.extend_from_slice(&label[..take]);
        events.try_reserve(1).map_err(|_| SmfError::Memory)?;
        events.push(RawMidiEvent {
            tick: 0,
            order: 0,
            bytes: msg,
        });
    }

    // Program Change (Cn prog)
    {
        let mut msg = Vec::new();
        msg.try_reserve_exact(2).map_err(|_| SmfError::Memory)?;
        msg.extend_from_slice(&[0xC0 | ch, program & 0x7F]);
        events.try_reserve(1).map_err(|_| SmfError::Memory)?;
        events.push(RawMidiEvent {
            tick: 0,
            order: 1,
            bytes: msg,
        });
    }

    events
        .try_reserve(notes.len().saturating_mul(2))
        .map_err(|_| SmfError::Memory)?;
    for &(start, end, key, vel) in notes {
        let on_tick_i64 = start.to_display_ticks().ok_or(SmfError::InvalidTiming)?;
        let off_tick_i64 = end.to_display_ticks().ok_or(SmfError::InvalidTiming)?;
        let on_tick = u32::try_from(on_tick_i64.max(0)).map_err(|_| SmfError::InvalidTiming)?;
        let off_tick = u32::try_from(off_tick_i64.max(on_tick_i64 + 1))
            .map_err(|_| SmfError::InvalidTiming)?;

        let mut on_bytes = Vec::new();
        on_bytes.try_reserve_exact(3).map_err(|_| SmfError::Memory)?;
        on_bytes.extend_from_slice(&[0x90 | ch, key & 0x7F, vel.clamp(1, 127)]);
        events.push(RawMidiEvent {
            tick: on_tick,
            order: 3, // NoteOn after any NoteOff at the same tick
            bytes: on_bytes,
        });

        let mut off_bytes = Vec::new();
        off_bytes.try_reserve_exact(3).map_err(|_| SmfError::Memory)?;
        off_bytes.extend_from_slice(&[0x80 | ch, key & 0x7F, 0]);
        events.push(RawMidiEvent {
            tick: off_tick,
            order: 2, // NoteOff precedes NoteOn at identical ticks
            bytes: off_bytes,
        });
    }

    serialize_track_events(events)
}

fn assign_midi_channel(instrument: u8, melodic_slot: &mut u8) -> u8 {
    if instrument == midi::Instrument::VelvetSampler as u8 {
        return 9; // Standard MIDI Percussion Channel 10 (0-indexed 9)
    }
    let ch = *melodic_slot;
    *melodic_slot = if ch == 8 { 10 } else { (ch + 1) % 16 };
    if *melodic_slot == 9 {
        *melodic_slot = 10;
    }
    ch
}

fn encode_format1(
    conductor_track: Vec<u8>,
    note_tracks: Vec<Vec<u8>>,
) -> Result<Vec<u8>, SmfError> {
    let total_tracks = 1usize + note_tracks.len();
    if total_tracks > MAX_SMF_TRACKS {
        return Err(SmfError::TooManyTracks);
    }
    let mut out = Vec::new();
    out.try_reserve(14 + conductor_track.len() + 8)
        .map_err(|_| SmfError::Memory)?;
    // MThd header: Format 1, N tracks, 960 PPQ
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(total_tracks as u16).to_be_bytes());
    out.extend_from_slice(&(DISPLAY_PPQ as u16).to_be_bytes());

    push_chunk(&mut out, b"MTrk", &conductor_track)?;
    for trk in note_tracks {
        push_chunk(&mut out, b"MTrk", &trk)?;
    }
    Ok(out)
}

/// Export a single multi-channel `TapestryPattern` as a Format 1 (`960 PPQ`) `.mid` byte vector.
pub fn export_smf_pattern(
    pattern: &TapestryPattern,
    channels: &[RackChannel],
    tempo_map: &TempoMap,
) -> Result<Vec<u8>, SmfError> {
    let conductor = build_conductor_track(tempo_map, &pattern.name)?;
    let mut note_tracks = Vec::new();
    let mut total_notes = 0usize;
    let mut melodic_slot = 0u8;

    for (ch_id, notes) in &pattern.channel_notes {
        if notes.is_empty() {
            continue;
        }
        total_notes = total_notes
            .checked_add(notes.len())
            .ok_or(SmfError::TooManyNotes)?;
        if total_notes > MAX_SMF_NOTES {
            return Err(SmfError::TooManyNotes);
        }
        if note_tracks.len() + 1 >= MAX_SMF_TRACKS {
            return Err(SmfError::TooManyTracks);
        }
        let ch = channels.iter().find(|c| c.id == *ch_id);
        let name = ch.map_or("Channel", |c| c.name.as_str());
        let inst = ch.map_or(0, |c| c.instrument);
        let midi_ch = assign_midi_channel(inst, &mut melodic_slot);

        let mut spans = Vec::new();
        spans
            .try_reserve_exact(notes.len())
            .map_err(|_| SmfError::Memory)?;
        for n in notes {
            let end = n.end().ok_or(SmfError::InvalidTiming)?;
            spans.push((n.start, end, n.key, n.velocity));
        }
        let trk = build_note_track(name, midi_ch, inst, &spans)?;
        note_tracks.try_reserve(1).map_err(|_| SmfError::Memory)?;
        note_tracks.push(trk);
    }

    encode_format1(conductor, note_tracks)
}

/// Export a single-channel frame-based `midi::Pattern` as Format 1 (`960 PPQ`) `.mid` bytes.
pub fn export_smf_midi_pattern(pattern: &midi::Pattern) -> Result<Vec<u8>, SmfError> {
    pattern.validate().map_err(|_| SmfError::InvalidTiming)?;
    let (expr_notes, tempo_map, _) = crate::compose::midi_pattern_to_notes(pattern)
        .map_err(|_| SmfError::InvalidTiming)?;
    let mut pat = TapestryPattern::new(PatternId(0), pattern.instrument.name(), 1, 4)
        .map_err(SmfError::from)?;
    let ch_id = ChannelId(0);
    for n in expr_notes {
        pat.add_note(ch_id, n).map_err(SmfError::from)?;
    }
    let ch = RackChannel::new(ch_id, pattern.instrument.name(), pattern.instrument as u8);
    export_smf_pattern(&pat, &[ch], &tempo_map)
}

/// Export a complete `TapestryBank` (honoring `transport_mode`, `TempoMap`, mute/solo,
/// and cut-group note durations) as standards-compliant `MThd` Format 1 (`960 PPQ`) `.mid` bytes.
pub fn export_smf_bank(bank: &TapestryBank) -> Result<Vec<u8>, SmfError> {
    let realized = bank
        .realize_events(bank.transport_mode)
        .map_err(SmfError::from)?;
    if realized.len() > MAX_SMF_NOTES {
        return Err(SmfError::TooManyNotes);
    }
    let conductor = build_conductor_track(&bank.tempo_map, "Moan Tapestry")?;

    let mut note_tracks = Vec::new();
    let mut melodic_slot = 0u8;

    for ch in &bank.channels {
        let mut spans = Vec::new();
        for ev in &realized {
            if ev.channel == ch.id {
                spans.try_reserve(1).map_err(|_| SmfError::Memory)?;
                spans.push((ev.start_beat, ev.end_beat, ev.key, ev.velocity));
            }
        }
        if spans.is_empty() {
            continue;
        }
        if note_tracks.len() + 1 >= MAX_SMF_TRACKS {
            return Err(SmfError::TooManyTracks);
        }
        let midi_ch = assign_midi_channel(ch.instrument, &mut melodic_slot);
        let trk = build_note_track(&ch.name, midi_ch, ch.instrument, &spans)?;
        note_tracks.try_reserve(1).map_err(|_| SmfError::Memory)?;
        note_tracks.push(trk);
    }

    encode_format1(conductor, note_tracks)
}
