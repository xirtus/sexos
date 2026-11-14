// SPDX-License-Identifier: GPL-3.0-only
//! Moan's editable project and Weft window, shared by guest and host UI tests.
#![no_std]
extern crate alloc;
pub mod meter_history;
pub mod piano;
pub mod project;
#[cfg(feature = "sexos")]
pub mod project_io;
pub mod waveform;
use alloc::{format, sync::Arc, vec, vec::Vec};
use moan_core::{Clip, Frame, Track, RATE};
use weft::{Column, Command, Id, Layout, Menu, Response, Rows, TextStyle, Ui};
pub const MAX_FRAMES: usize = RATE as usize * 60;
pub const MAX_TRACKS: usize = 8;
#[derive(Clone)]
struct Snapshot {
    revision: u64,
    cursor: usize,
    from: usize,
    to: usize,
    tracks: Vec<ArchivedTrack>,
    selected: usize,
}
#[derive(Clone)]
struct ArchivedTrack {
    skeleton: Track<'static>,
    clips: Vec<ArchivedClip>,
}
#[derive(Clone)]
enum ArchivedClip {
    Audio(Clip<'static>),
    Midi {
        source: usize,
        pattern: Arc<moan_core::midi::Pattern>,
        start: usize,
        offset: usize,
        len: usize,
        gain: u16,
        fades: (usize, usize),
    },
}
impl Snapshot {
    fn materialize(&self) -> Result<Vec<Track<'static>>, &'static str> {
        let mut sources: Vec<(usize, Clip<'static>)> = Vec::new();
        let mut tracks = Vec::new();
        for archived in &self.tracks {
            let mut track = archived.skeleton.clone();
            for clip in &archived.clips {
                let clip = match clip {
                    ArchivedClip::Audio(clip) => clip.clone(),
                    ArchivedClip::Midi {
                        source,
                        pattern,
                        start,
                        offset,
                        len,
                        gain,
                        fades,
                    } => {
                        let mut clip =
                            if let Some((_, clip)) = sources.iter().find(|(id, _)| id == source) {
                                clip.clone()
                            } else {
                                let clip = Clip::midi((**pattern).clone(), 0)
                                    .map_err(|_| "Cannot restore MIDI cache")?;
                                sources.push((*source, clip.clone()));
                                clip
                            };
                        clip.trim(*offset, *offset + *len)
                            .map_err(|_| "MIDI trim")?;
                        clip.move_to(*start).map_err(|_| "MIDI start")?;
                        clip.set_gain(*gain).map_err(|_| "MIDI gain")?;
                        clip.set_fades(fades.0, fades.1).map_err(|_| "MIDI fades")?;
                        clip
                    }
                };
                track.clips.push(clip);
            }
            tracks.push(track);
        }
        Ok(tracks)
    }
}
pub struct Session {
    revision: u64,
    next_revision: u64,
    saved_revision: u64,
    pub tracks: Vec<Track<'static>>,
    pub selected: usize,
    pub cursor: usize,
    pub from: usize,
    pub to: usize,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            revision: 0,
            next_revision: 1,
            saved_revision: 0,
            tracks: Vec::new(),
            selected: 0,
            cursor: 0,
            from: 0,
            to: 0,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }
}
impl Session {
    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }
    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }
    pub fn encode_project(&self) -> Result<Vec<u8>, project::Error> {
        project::encode(self)
    }
    pub fn decode_project(bytes: &[u8]) -> Result<Self, project::Error> {
        project::decode(bytes)
    }
    fn snapshot(&self) -> Snapshot {
        let mut tracks = Vec::new();
        let mut patterns: Vec<(usize, Arc<moan_core::midi::Pattern>)> = Vec::new();
        for track in &self.tracks {
            let mut skeleton = track.clone();
            skeleton.clips.clear();
            skeleton.reset_effects();
            let mut clips = Vec::new();
            for clip in &track.clips {
                if let Some(pattern) = clip.midi_pattern() {
                    let source = clip.source_frames().as_ptr() as usize;
                    let pattern =
                        if let Some((_, p)) = patterns.iter().find(|(id, _)| *id == source) {
                            p.clone()
                        } else {
                            let p = Arc::new(pattern.clone());
                            patterns.push((source, p.clone()));
                            p
                        };
                    clips.push(ArchivedClip::Midi {
                        source,
                        pattern,
                        start: clip.start(),
                        offset: clip.source_offset(),
                        len: clip.len(),
                        gain: clip.gain(),
                        fades: clip.fades(),
                    });
                } else {
                    clips.push(ArchivedClip::Audio(clip.clone()));
                }
            }
            tracks.push(ArchivedTrack { skeleton, clips });
        }
        Snapshot {
            revision: self.revision,
            cursor: self.cursor,
            from: self.from,
            to: self.to,
            tracks,
            selected: self.selected,
        }
    }
    fn remember(&mut self) {
        let s = self.snapshot();
        self.undo.push(s);
        if self.undo.len() > 12 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.revision = self.next_revision;
        self.next_revision = self.next_revision.saturating_add(1);
    }
    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else {
            return false;
        };
        let Ok(tracks) = s.materialize() else {
            self.undo.push(s);
            return false;
        };
        let current = self.snapshot();
        self.redo.push(current);
        self.revision = s.revision;
        self.cursor = s.cursor;
        self.from = s.from;
        self.to = s.to;
        self.tracks = tracks;
        self.selected = s.selected;
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else {
            return false;
        };
        let Ok(tracks) = s.materialize() else {
            self.redo.push(s);
            return false;
        };
        let current = self.snapshot();
        self.undo.push(current);
        self.revision = s.revision;
        self.cursor = s.cursor;
        self.from = s.from;
        self.to = s.to;
        self.tracks = tracks;
        self.selected = s.selected;
        true
    }
    pub fn duration(&self) -> usize {
        self.tracks
            .iter()
            .flat_map(|t| &t.clips)
            .map(Clip::end)
            .max()
            .unwrap_or(0)
    }
    pub fn add(&mut self, pcm: Vec<Frame>) -> Result<(), &'static str> {
        if self.tracks.len() >= MAX_TRACKS {
            return Err("Eight tracks maximum");
        }
        if pcm.is_empty()
            || pcm.len() > MAX_FRAMES
            || self
                .cursor
                .checked_add(pcm.len())
                .is_none_or(|end| end > project::MAX_TIMELINE)
        {
            return Err("Audio must be 1 frame to 60 seconds");
        }
        self.remember();
        let mut t = Track::new();
        t.clips
            .push(Clip::owned(Arc::from(pcm), self.cursor).map_err(|_| "Timeline overflow")?);
        self.tracks.push(t);
        self.selected = self.tracks.len() - 1;
        self.from = self.cursor;
        self.to = self.duration();
        Ok(())
    }
    pub fn add_midi(&mut self, pattern: moan_core::midi::Pattern) -> Result<(), &'static str> {
        if self.tracks.len() >= MAX_TRACKS {
            return Err("Eight tracks maximum");
        }
        let clip = Clip::midi(pattern, self.cursor)
            .map_err(|_| "MIDI pattern exceeds note, voice or memory bounds")?;
        if clip.end() > project::MAX_TIMELINE {
            return Err("Timeline overflow");
        }
        self.remember();
        let mut track = Track::new();
        track.clips.push(clip);
        self.tracks.push(track);
        self.selected = self.tracks.len() - 1;
        self.from = self.cursor;
        self.to = self.duration();
        Ok(())
    }
    pub fn midi_clip(&self) -> Option<&Clip<'static>> {
        let track = self.tracks.get(self.selected)?;
        track
            .clips
            .iter()
            .find(|c| {
                c.midi_pattern().is_some() && self.cursor >= c.start() && self.cursor < c.end()
            })
            .or_else(|| track.clips.iter().find(|c| c.midi_pattern().is_some()))
    }
    pub fn edit_midi(
        &mut self,
        edit: impl FnOnce(&mut moan_core::midi::Pattern),
    ) -> Result<(), &'static str> {
        let source = self.midi_clip().ok_or("Choose a MIDI region")?;
        let old = source.source_frames().as_ptr();
        let mut pattern = source.midi_pattern().unwrap().clone();
        edit(&mut pattern);
        let replacement = Clip::midi(pattern, 0).map_err(|_| "MIDI edit exceeds bounds")?;
        self.remember();
        for clip in &mut self.tracks[self.selected].clips {
            if clip.source_frames().as_ptr() == old {
                clip.replace_midi_source(&replacement)
                    .map_err(|_| "MIDI cache mismatch")?;
            }
        }
        self.tracks[self.selected].reset_effects();
        Ok(())
    }
    pub fn tone(&mut self) -> Result<(), &'static str> {
        let period = 120usize.saturating_sub(self.tracks.len() * 8).max(40);
        self.add(
            (0..RATE as usize * 2)
                .map(|i| [if i % period < period / 2 { 4000 } else { -4000 }; 2])
                .collect(),
        )
    }
    pub fn split(&mut self) -> Result<(), &'static str> {
        if self.tracks.iter().map(|t| t.clips.len()).sum::<usize>() >= project::MAX_CLIPS {
            return Err("Project region limit reached");
        }
        let t = self.tracks.get(self.selected).ok_or("Select a track")?;
        if !t
            .clips
            .iter()
            .any(|c| self.cursor > c.start() && self.cursor < c.end())
        {
            return Err("Place playhead inside a clip");
        }
        self.remember();
        let t = &mut self.tracks[self.selected];
        let i = t
            .clips
            .iter()
            .position(|c| self.cursor > c.start() && self.cursor < c.end())
            .unwrap();
        let offset = self.cursor - t.clips[i].start();
        let right = t.clips[i].split(offset).map_err(|_| "Split failed")?;
        t.clips.insert(i + 1, right);
        Ok(())
    }
    pub fn cut(&mut self) -> Result<(), &'static str> {
        if self.from >= self.to {
            return Err("Select a nonempty time range");
        }
        if self.selected >= self.tracks.len() {
            return Err("Select a track");
        }
        self.remember();
        let mut kept = Vec::new();
        for c in &self.tracks[self.selected].clips {
            if c.end() <= self.from || c.start() >= self.to {
                kept.push(c.clone());
                continue;
            }
            if c.start() < self.from {
                let mut left = c.clone();
                left.trim(0, self.from - c.start())
                    .map_err(|_| "Cut failed")?;
                kept.push(left);
            }
            if c.end() > self.to {
                let mut right = c.clone();
                right
                    .trim(self.to - c.start(), c.len())
                    .map_err(|_| "Cut failed")?;
                kept.push(right);
            }
        }
        self.tracks[self.selected].clips = kept;
        Ok(())
    }
    pub fn fades(&mut self) -> Result<(), &'static str> {
        if self.selected >= self.tracks.len() {
            return Err("Select a track");
        }
        self.remember();
        for c in &mut self.tracks[self.selected].clips {
            let n = (RATE as usize / 20).min(c.len() / 2);
            c.set_fades(n, n).map_err(|_| "Fade failed")?;
        }
        Ok(())
    }
    pub fn gain(&mut self, percent: u16) -> Result<(), &'static str> {
        if percent > 400 {
            return Err("Gain must be 0 to 400%");
        }
        if self.selected >= self.tracks.len() {
            return Err("Select a track");
        }
        self.remember();
        for c in &mut self.tracks[self.selected].clips {
            c.set_gain(percent).map_err(|_| "Gain failed")?;
        }
        Ok(())
    }
}

/// Strict PCM WAV import: 48 kHz stereo signed 16-bit, bounded to one minute.
/// Unknown RIFF chunks are skipped with their alignment byte; malformed input
/// fails before allocation. Other formats require a decoder, not reinterpretation.
pub fn read_wav(data: &[u8]) -> Result<Vec<Frame>, &'static str> {
    fn u32at(d: &[u8], i: usize) -> Result<u32, &'static str> {
        Ok(u32::from_le_bytes(
            d.get(i..i + 4).ok_or("Truncated WAV")?.try_into().unwrap(),
        ))
    }
    if data.get(..4) != Some(b"RIFF") || data.get(8..12) != Some(b"WAVE") {
        return Err("Choose a PCM WAV file");
    }
    let end = 8usize
        .checked_add(u32at(data, 4)? as usize)
        .ok_or("Invalid RIFF size")?;
    if end > data.len() || end < 12 {
        return Err("Invalid RIFF size");
    }
    let mut at = 12;
    let mut fmt = false;
    let mut audio = None;
    while at < end {
        if end - at < 8 {
            return Err("Truncated chunk");
        }
        let n = u32at(data, at + 4)? as usize;
        let start = at + 8;
        let finish = start
            .checked_add(n)
            .filter(|x| *x <= end)
            .ok_or("Invalid chunk length")?;
        match &data[at..at + 4] {
            b"fmt " => {
                if fmt
                    || n < 16
                    || data[start..start + 4] != [1, 0, 2, 0]
                    || u32at(data, start + 4)? != RATE
                    || u32at(data, start + 8)? != RATE * 4
                    || data[start + 12..start + 16] != [4, 0, 16, 0]
                {
                    return Err("WAV must be 48 kHz stereo 16-bit PCM");
                }
                fmt = true;
            }
            b"data" => {
                if audio.is_some() || n == 0 || n % 4 != 0 || n / 4 > MAX_FRAMES {
                    return Err("Invalid or oversized WAV audio");
                }
                audio = Some(&data[start..finish]);
            }
            _ => {}
        }
        at = finish
            .checked_add(n % 2)
            .filter(|x| *x <= end)
            .ok_or("Missing chunk padding")?;
    }
    if !fmt {
        return Err("Missing WAV format");
    }
    let bytes = audio.ok_or("Missing WAV audio")?;
    Ok(bytes
        .chunks_exact(4)
        .map(|b| {
            [
                i16::from_le_bytes([b[0], b[1]]),
                i16::from_le_bytes([b[2], b[3]]),
            ]
        })
        .collect())
}
pub fn wav_header(frames: usize) -> Result<[u8; 44], &'static str> {
    let n = u32::try_from(frames.checked_mul(4).ok_or("WAV too large")?)
        .map_err(|_| "WAV too large")?;
    let size = n.checked_add(36).ok_or("WAV too large")?;
    let mut h = [0; 44];
    h[..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&size.to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..24].copy_from_slice(&[1, 0, 2, 0]);
    h[24..28].copy_from_slice(&RATE.to_le_bytes());
    h[28..32].copy_from_slice(&(RATE * 4).to_le_bytes());
    h[32..36].copy_from_slice(&[4, 0, 16, 0]);
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&n.to_le_bytes());
    Ok(h)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Play,
    Pause,
    Stop,
    Record,
    Import,
    Export,
    OpenProject,
    SaveProject,
    NewProject,
    Quit,
}
#[derive(Clone, Copy)]
enum Drag {
    Select {
        track: usize,
        anchor: usize,
    },
    Move {
        track: usize,
        clip: usize,
        anchor: usize,
        start: usize,
        remembered: bool,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Workspace {
    Arrange,
    Mix,
    PianoRoll,
    Instruments,
}
pub struct Studio {
    pub export_format: u8,
    pub waveform: Id,
    pub piano: Id,
    arrange: Id,
    navigation: Id,
    library: Id,
    smart: Id,
    console: Id,
    piano_panel: Id,
    track_buttons: Vec<Id>,
    mixer_strips: Vec<Id>,
    workspace: Workspace,
    library_open: bool,
    piano_label: Id,
    velocity: Id,
    note_velocity: u8,
    app_end: Id,
    meter_names: Vec<Id>,
    meter_values: Vec<Id>,
    mixer_volume: Vec<Id>,
    mixer_pan: Vec<Id>,
    pub view: waveform::View,
    transport_cursor: Option<usize>,
    viewport: Id,
    scale_label: Id,
    move_mode: Id,
    move_clips: bool,
    drag: Option<Drag>,
    pending: Option<(weft::DialogId, Action)>,
    busy: bool,
    pub session: Session,
    pub timeline: Id,
    pub status: Id,
    pub position: Id,
    pub from: Id,
    pub to: Id,
    pub buttons: Vec<(Id, &'static str)>,
    volume: Id,
    pan: Id,
    gain: Id,
    eq: Id,
    compressor: Id,
    reverb: Id,
    mute: Id,
    solo: Id,
}
impl Studio {
    pub fn build(ui: &mut Ui) -> Self {
        for (id, label, key) in [
            ("moan.library.keys", "Silk Keys MIDI", "Ctrl+Shift+I"),
            ("moan.library.bass", "Satin Bass MIDI", "Ctrl+Shift+B"),
            ("moan.library.kit", "Velvet Kit MIDI", "Ctrl+Shift+D"),
            ("moan.piano", "Show Piano Roll", "Ctrl+P"),
            ("moan.tracks", "Focus Track List", "Ctrl+Shift+R"),
            ("moan.mixer", "Show Mixer", "Ctrl+M"),
            ("moan.octave", "Raise MIDI Octave", "Ctrl+Shift+Up"),
            ("moan.lengthen", "Lengthen MIDI Notes", "Ctrl+Shift+G"),
            ("moan.velocity-half", "Soften MIDI Notes", "Ctrl+Shift+V"),
            (
                "moan.clear-notes",
                "Clear MIDI Notes",
                "Ctrl+Shift+Backspace",
            ),
            ("moan.play", "Play", "Ctrl+Space"),
            ("moan.stop", "Stop", "Escape"),
            ("moan.undo", "Undo Edit", "Ctrl+Z"),
            ("moan.redo", "Redo Edit", "Ctrl+Shift+Z"),
            ("moan.import", "Import WAV", "Ctrl+O"),
            ("moan.export", "Export Audio", "Ctrl+Shift+E"),
            ("moan.open-project", "Open Project", "Ctrl+Shift+O"),
            ("moan.save-project", "Save Project", "Ctrl+S"),
            ("moan.new-project", "New Project", "Ctrl+N"),
            ("moan.format.wav", "WAV Export", "Ctrl+Shift+1"),
            ("moan.format.flac", "FLAC Export", "Ctrl+Shift+2"),
            ("moan.format.mp3", "MP3 Export", "Ctrl+Shift+3"),
            ("moan.timeline", "Show Waveform Timeline", "Ctrl+T"),
            ("moan.metrics", "Refresh View", "Ctrl+Shift+L"),
            ("moan.tone", "Add Tone Track", "Ctrl+Shift+T"),
            ("moan.split", "Split Clip", "Ctrl+E"),
            ("moan.cut", "Cut Selected Time", "Ctrl+Shift+K"),
            ("moan.move-clips", "Drag Clips Mode", "Ctrl+Shift+M"),
        ] {
            ui.commands.add(Command::new(id, label).shortcut(key));
        }
        let root = ui.root();
        ui.set_layout(root, Layout::column());
        ui.menubar(
            root,
            vec![
                Menu::new("File")
                    .command("moan.new-project")
                    .command("moan.open-project")
                    .command("moan.save-project")
                    .separator()
                    .command("moan.import")
                    .command("moan.export"),
                Menu::new("Edit").command("moan.undo").command("moan.redo"),
                Menu::new("MIDI")
                    .command("moan.library.keys")
                    .command("moan.library.bass")
                    .command("moan.library.kit")
                    .separator()
                    .command("moan.piano")
                    .command("moan.clear-notes")
                    .command("moan.octave")
                    .command("moan.velocity-half"),
                Menu::new("Transport")
                    .command("moan.play")
                    .command("moan.stop"),
            ],
        );
        // Transport and workspace navigation never scroll out of reach.
        let transport = ui.group(root, Layout::row().gap(4.0).padding(4.0).wrap().shrink(0.0));
        ui.text(transport, "MOAN", TextStyle::Heading);
        let mut buttons = Vec::new();
        for name in [
            "Play",
            "Pause",
            "Stop",
            "Record",
            "Tone",
            "Import WAV",
            "Export WAV",
        ] {
            buttons.push((
                ui.button(
                    transport,
                    if name == "Export WAV" {
                        "Export Audio"
                    } else {
                        name
                    },
                ),
                name,
            ));
        }
        let navigation = ui.group(
            root,
            Layout::row()
                .gap(4.0)
                .padding_xy(4.0, 0.0)
                .wrap()
                .shrink(0.0),
        );
        for name in ["Arrange", "Mix", "Piano Roll", "Instruments", "Library"] {
            buttons.push((ui.button(navigation, name), name));
        }
        for name in [
            "New Project",
            "Open Project",
            "Save Project",
            "WAV",
            "FLAC",
            "MP3",
        ] {
            buttons.push((ui.button(navigation, name), name));
        }
        let status = ui.text(
            root,
            "Original instruments, audio tracks and editable MIDI regions",
            TextStyle::Caption,
        );
        let scroll = ui.scroll(root);
        ui.set_layout(scroll, Layout::column().fill());
        let body = ui.group(
            scroll,
            Layout::column()
                .gap(4.0)
                .padding(4.0)
                .fill_width()
                .shrink(0.0),
        );
        let workspace = ui.group(
            body,
            Layout::row()
                .gap(6.0)
                .fill_width()
                .align_items(taffy::AlignItems::START),
        );
        let library = ui.panel(
            workspace,
            weft::Fill::Surface,
            Layout::column()
                .gap(4.0)
                .padding(6.0)
                .width(150.0)
                .shrink(0.0),
        );
        ui.text(library, "Library", TextStyle::Heading);
        for name in ["Silk Keys", "Satin Bass", "Velvet Kit"] {
            buttons.push((ui.button(library, name), name));
        }
        ui.text(
            library,
            "Original patches\n+ two-bar loops",
            TextStyle::Caption,
        );
        let arrange = ui.panel(
            workspace,
            weft::Fill::Surface,
            Layout::column().gap(4.0).padding(4.0).fill(),
        );
        let view_row = ui.group(arrange, Layout::row().gap(4.0).wrap().fill_width());
        for name in ["Zoom in", "Zoom out", "Fit audio", "Earlier", "Later"] {
            buttons.push((ui.button(view_row, name), name));
        }
        let move_mode = ui.checkbox(view_row, "Drag clips", false);
        let scale_label = ui.text(arrange, "Timeline · 0.00s — 4.00s", TextStyle::Caption);
        let lanes = ui.group(arrange, Layout::row().gap(0.0).fill_width());
        let track_headers = ui.group(lanes, Layout::column().gap(0.0).width(142.0).shrink(0.0));
        let ruler = ui.text(track_headers, "TRACK / REGION", TextStyle::Caption);
        ui.set_layout(ruler, Layout::default().height(waveform::HEADER as f32));
        let mut track_buttons = Vec::new();
        for _ in 0..MAX_TRACKS {
            let id = ui.button(track_headers, "");
            ui.set_layout(
                id,
                Layout::default()
                    .fill_width()
                    .height(waveform::ROW as f32)
                    .shrink(0.0),
            );
            ui.set_visible(id, false);
            track_buttons.push(id);
        }
        let (height, pixels) = waveform::render(&Session::default(), waveform::View::default());
        let waveform = ui.image(lanes, waveform::WIDTH, height, pixels);
        ui.set_label(
            waveform,
            "Timeline: aligned track lanes, real audio waveforms and MIDI regions",
        );
        let smart = ui.panel(
            workspace,
            weft::Fill::Surface,
            Layout::column()
                .gap(4.0)
                .padding(6.0)
                .width(196.0)
                .min_width(0.0)
                .shrink(0.0),
        );
        ui.text(smart, "Smart Controls", TextStyle::Heading);
        let time = ui.group(smart, Layout::column().gap(2.0));
        ui.text(time, "Position / range (seconds)", TextStyle::Caption);
        let position = ui.field(time, "0", "Playhead seconds");
        let from = ui.field(time, "0", "Start seconds");
        let to = ui.field(time, "1", "End seconds");
        let edits = ui.group(smart, Layout::row().gap(4.0).wrap().fill_width());
        for name in ["Split", "Cut range", "Fade edges", "Undo", "Redo"] {
            buttons.push((ui.button(edits, name), name));
        }
        ui.text(smart, "Regions", TextStyle::Caption);
        let timeline = ui.list_with(
            smart,
            alloc::boxed::Box::new(Rows::default()),
            vec![Column::new("Region", 160.0)],
        );
        ui.set_layout(
            timeline,
            Layout::default().fill_width().height(90.0).shrink(0.0),
        );
        ui.text(smart, "Volume / pan / region gain", TextStyle::Caption);
        let volume = ui.slider(smart, 0.0, 200.0, 100.0);
        let pan = ui.slider(smart, -100.0, 100.0, 0.0);
        let gain = ui.slider(smart, 0.0, 200.0, 100.0);
        let flags = ui.group(smart, Layout::row().gap(4.0).wrap().fill_width());
        let mute = ui.checkbox(flags, "Mute", false);
        let solo = ui.checkbox(flags, "Solo", false);
        let eq = ui.checkbox(flags, "Warm EQ", false);
        let compressor = ui.checkbox(flags, "Compressor", false);
        let reverb = ui.checkbox(flags, "Reverb", false);
        let console = ui.panel(
            root,
            weft::Fill::Surface,
            Layout::column().gap(4.0).padding(4.0).fill_width(),
        );
        ui.text(
            console,
            "Mixer · consumed post-fader Peak / RMS",
            TextStyle::Caption,
        );
        let strips = ui.group(console, Layout::row().gap(4.0).wrap().fill_width());
        let mut meter_names = Vec::new();
        let mut meter_values = Vec::new();
        let mut mixer_volume = Vec::new();
        let mut mixer_pan = Vec::new();
        let mut mixer_strips = Vec::new();
        for _ in 0..MAX_TRACKS {
            let strip = ui.panel(
                strips,
                weft::Fill::Control,
                Layout::column()
                    .gap(3.0)
                    .padding(4.0)
                    .width(126.0)
                    .shrink(0.0),
            );
            mixer_strips.push(strip);
            meter_names.push(ui.text(strip, "", TextStyle::Caption));
            meter_values.push(ui.progress(strip, Some(0.0)));
            ui.text(strip, "Level / Pan", TextStyle::Caption);
            mixer_volume.push(ui.slider(strip, 0.0, 200.0, 100.0));
            mixer_pan.push(ui.slider(strip, -100.0, 100.0, 0.0));
            ui.set_visible(strip, false);
        }
        let piano_panel = ui.panel(
            root,
            weft::Fill::Surface,
            Layout::column().gap(4.0).padding(4.0).fill_width(),
        );
        let piano_label = ui.text(
            piano_panel,
            "Piano Roll · C3–B4 · 16 steps",
            TextStyle::Caption,
        );
        let piano = ui.image(
            piano_panel,
            piano::WIDTH,
            piano::HEIGHT,
            piano::render(None),
        );
        ui.set_label(piano, "Piano roll with pitch ruler: C3–B4, 16 steps");
        let midi_edits = ui.group(piano_panel, Layout::row().gap(4.0).wrap().fill_width());
        for name in [
            "Clear notes",
            "Quantize",
            "Transpose -",
            "Transpose +",
            "Set velocity",
            "Length +",
        ] {
            buttons.push((ui.button(midi_edits, name), name));
        }
        ui.text(midi_edits, "Velocity", TextStyle::Caption);
        let velocity = ui.slider(midi_edits, 1.0, 127.0, 96.0);
        ui.set_layout(velocity, Layout::default().width(110.0));
        let app_end = ui.text(
            body,
            "48 kHz stereo · Stop to edit · bounded immutable instrument cache",
            TextStyle::Caption,
        );
        ui.set_visible(smart, false);
        ui.set_visible(piano_panel, false);
        let studio = Self {
            export_format: 0,
            waveform,
            piano,
            arrange,
            navigation,
            library,
            smart,
            console,
            piano_panel,
            track_buttons,
            mixer_strips,
            workspace: Workspace::Arrange,
            library_open: true,
            piano_label,
            velocity,
            note_velocity: 96,
            app_end,
            meter_names,
            meter_values,
            mixer_volume,
            mixer_pan,
            view: waveform::View::default(),
            transport_cursor: None,
            viewport: scroll,
            scale_label,
            move_mode,
            move_clips: false,
            drag: None,
            pending: None,
            busy: false,
            session: Session::default(),
            timeline,
            status,
            position,
            from,
            to,
            buttons,
            volume,
            pan,
            gain,
            eq,
            compressor,
            reverb,
            mute,
            solo,
        };
        studio.refresh(ui);
        studio
    }
    pub fn set_busy(&mut self, ui: &mut Ui, busy: bool) {
        if self.busy == busy {
            return;
        }
        self.busy = busy;
        if busy {
            self.drag = None;
        }
        for id in [
            self.waveform,
            self.piano,
            self.velocity,
            self.move_mode,
            self.position,
            self.from,
            self.to,
            self.timeline,
            self.volume,
            self.pan,
            self.gain,
            self.eq,
            self.compressor,
            self.reverb,
            self.mute,
            self.solo,
        ] {
            ui.set_enabled(id, !busy);
        }
        for (index, id) in self
            .mixer_volume
            .iter()
            .chain(self.mixer_pan.iter())
            .enumerate()
        {
            ui.set_enabled(*id, !busy && index % MAX_TRACKS < self.session.tracks.len());
        }
        for (id, name) in &self.buttons {
            if ![
                "Play",
                "Pause",
                "Stop",
                "Arrange",
                "Mix",
                "Piano Roll",
                "Instruments",
                "Library",
            ]
            .contains(name)
            {
                ui.set_enabled(*id, !busy);
            }
        }
        for command in [
            "moan.undo",
            "moan.redo",
            "moan.import",
            "moan.export",
            "moan.open-project",
            "moan.save-project",
            "moan.new-project",
            "moan.library.keys",
            "moan.library.bass",
            "moan.library.kit",
            "moan.clear-notes",
        ] {
            ui.commands.set_enabled(command, !busy);
        }
    }
    pub fn publish_meters(
        &self,
        ui: &mut Ui,
        meters: &mut [serenade_core::LevelMeter; MAX_TRACKS],
    ) {
        for (i, meter) in meters.iter_mut().enumerate() {
            let values = meter.snapshot();
            let peak = values[0].peak.max(values[1].peak);
            let rms = values[0].rms.max(values[1].rms);
            ui.set_value(
                self.meter_values[i],
                ((serenade_core::linear_to_dbfs(peak) + 60.0) / 60.0).clamp(0.0, 1.0),
            );
            if let Some(track) = self.session.tracks.get(i) {
                let name = track
                    .clips
                    .iter()
                    .find_map(Clip::midi_pattern)
                    .map_or("Audio", |p| p.instrument.name());
                ui.set_text(
                    self.meter_names[i],
                    &format!(
                        "{} {} · P{} R{} dB",
                        i + 1,
                        name,
                        serenade_core::linear_to_dbfs_approx(peak),
                        serenade_core::linear_to_dbfs_approx(rms)
                    ),
                );
            }
        }
    }
    pub fn message(&self, ui: &mut Ui, text: &str) {
        ui.set_text(self.status, text);
    }
    fn visible_layout(ui: &mut Ui, id: Id, visible: bool, mut layout: Layout) {
        ui.set_visible(id, visible);
        if !visible {
            layout.style().display = taffy::Display::None;
        }
        ui.set_layout(id, layout);
    }
    fn layout_document(&self, ui: &mut Ui) {
        let available = ui.size().0 as f32 / ui.theme().scale;
        let short = ui.size().1 as f32 / ui.theme().scale < 300.0;
        Self::visible_layout(
            ui,
            self.navigation,
            !short,
            Layout::row()
                .gap(4.0)
                .padding_xy(4.0, 0.0)
                .wrap()
                .fill_width()
                .shrink(0.0),
        );
        Self::visible_layout(ui, self.status, !short, Layout::default());
        for (id, name) in &self.buttons {
            if ["Tone", "Import WAV", "Export WAV"].contains(name) {
                Self::visible_layout(ui, *id, !short, Layout::default());
            }
        }
        let wide = available >= 1000.0;
        let show_library = self.library_open && available >= 800.0;
        Self::visible_layout(
            ui,
            self.library,
            show_library,
            Layout::column()
                .gap(4.0)
                .padding(6.0)
                .width(150.0)
                .shrink(0.0),
        );
        Self::visible_layout(
            ui,
            self.smart,
            self.workspace == Workspace::Instruments,
            Layout::column()
                .gap(4.0)
                .padding(6.0)
                .width(196.0)
                .min_width(0.0)
                .shrink(0.0),
        );
        Self::visible_layout(
            ui,
            self.console,
            !short && matches!(self.workspace, Workspace::Arrange | Workspace::Mix),
            Layout::column().gap(4.0).padding(4.0).fill_width(),
        );
        Self::visible_layout(
            ui,
            self.piano_panel,
            self.workspace == Workspace::PianoRoll,
            Layout::column().gap(4.0).padding(4.0).fill_width(),
        );
        let sidebar = if show_library { 156.0 } else { 0.0 }
            + if self.workspace == Workspace::Instruments {
                202.0
            } else {
                0.0
            };
        let width = (available - 26.0 - sidebar).max(230.0);
        ui.set_layout(
            self.arrange,
            Layout::column()
                .gap(4.0)
                .padding(4.0)
                .width(width)
                .min_width(0.0)
                .shrink(0.0),
        );
        let content = (width - 150.0).max(80.0);
        let height =
            waveform::HEADER + waveform::ROW * self.session.tracks.len().min(MAX_TRACKS).max(1);
        ui.set_layout(
            self.waveform,
            Layout::default()
                .width(content)
                .height(height as f32)
                .shrink(0.0),
        );
        ui.set_layout(
            self.piano,
            Layout::default()
                .width((available - 24.0).max(100.0))
                .height(piano::HEIGHT as f32)
                .shrink(0.0),
        );
        for strip in &self.mixer_strips {
            Self::visible_layout(
                ui,
                *strip,
                self.mixer_strips
                    .iter()
                    .position(|item| item == strip)
                    .unwrap()
                    < self.session.tracks.len(),
                Layout::column()
                    .gap(3.0)
                    .padding(4.0)
                    .width(if wide { 126.0 } else { 116.0 })
                    .shrink(0.0),
            );
        }
    }
    fn show_workspace(&mut self, ui: &mut Ui, workspace: Workspace) {
        self.workspace = workspace;
        ui.set_visible(
            self.console,
            matches!(workspace, Workspace::Arrange | Workspace::Mix),
        );
        ui.set_visible(self.piano_panel, workspace == Workspace::PianoRoll);
        self.layout_document(ui);
        ui.invalidate();
    }
    pub fn transport_cursor(&mut self, ui: &mut Ui, cursor: Option<usize>) {
        if self.transport_cursor != cursor {
            self.transport_cursor = cursor;
            let (height, pixels) = waveform::render_at(
                &self.session,
                self.view,
                cursor.unwrap_or(self.session.cursor),
            );
            ui.set_image(self.waveform, waveform::WIDTH, height, pixels);
        }
    }
    pub fn refresh(&self, ui: &mut Ui) {
        let (height, pixels) = waveform::render_at(
            &self.session,
            self.view,
            self.transport_cursor.unwrap_or(self.session.cursor),
        );
        ui.set_image(self.waveform, waveform::WIDTH, height, pixels);
        self.layout_document(ui);
        ui.set_text(
            self.scale_label,
            &format!(
                "{:.2}s — {:.2}s. Drag to select time; enable Drag clips to move audio.",
                self.view.start as f64 / RATE as f64,
                (self.view.start + self.view.span) as f64 / RATE as f64
            ),
        );
        for (id, value) in [
            (self.position, self.session.cursor),
            (self.from, self.session.from),
            (self.to, self.session.to),
        ] {
            ui.set_text(id, &format!("{:.6}", value as f64 / RATE as f64));
        }
        for (id, name) in &self.buttons {
            if let Some(format) = ["WAV", "FLAC", "MP3"].iter().position(|item| item == name) {
                ui.set_button_style(
                    *id,
                    if format == self.export_format as usize {
                        weft::ButtonStyle::Primary
                    } else {
                        weft::ButtonStyle::Normal
                    },
                );
            }
        }

        ui.set_image(
            self.piano,
            piano::WIDTH,
            piano::HEIGHT,
            piano::render(self.session.midi_clip().and_then(Clip::midi_pattern)),
        );
        ui.set_text(
            self.piano_label,
            &self
                .session
                .midi_clip()
                .and_then(Clip::midi_pattern)
                .map_or_else(
                    || "Choose an instrument from Library. Piano roll: C3–B4, 16 steps.".into(),
                    |p| {
                        format!(
                            "{} · {} notes · {} BPM · {} · velocity {}",
                            p.instrument.name(),
                            p.notes.len(),
                            p.bpm,
                            p.notes
                                .last()
                                .map_or_else(|| "C3–B4".into(), |note| piano::note_name(note.key)),
                            self.note_velocity
                        )
                    },
                ),
        );
        for i in 0..MAX_TRACKS {
            Self::visible_layout(
                ui,
                self.track_buttons[i],
                i < self.session.tracks.len(),
                Layout::default()
                    .fill_width()
                    .height(waveform::ROW as f32)
                    .shrink(0.0),
            );
            ui.set_visible(self.mixer_strips[i], i < self.session.tracks.len());
            if let Some(track) = self.session.tracks.get(i) {
                let name = track
                    .clips
                    .iter()
                    .find_map(Clip::midi_pattern)
                    .map_or("Audio", |p| p.instrument.name());
                ui.set_text(self.track_buttons[i], &format!("{} {}", i + 1, name));
                ui.set_button_style(
                    self.track_buttons[i],
                    if i == self.session.selected {
                        weft::ButtonStyle::Primary
                    } else {
                        weft::ButtonStyle::Normal
                    },
                );
                ui.set_text(self.meter_names[i], &format!("{} {}", i + 1, name));
                ui.set_color(
                    self.meter_names[i],
                    Some(weft::Color::hex(
                        [0x72d6b5, 0xb29ae3, 0xe4b579, 0x82c5eb][i % 4],
                    )),
                );
                ui.set_value(self.mixer_volume[i], track.volume() as f32);
                ui.set_value(self.mixer_pan[i], track.pan() as f32);
            } else {
                ui.set_text(self.meter_names[i], "Empty");
                ui.set_value(self.meter_values[i], 0.0);
            }
            ui.set_enabled(
                self.mixer_volume[i],
                !self.busy && i < self.session.tracks.len(),
            );
            ui.set_enabled(
                self.mixer_pan[i],
                !self.busy && i < self.session.tracks.len(),
            );
        }
        let mut rows = Vec::new();
        for (n, t) in self.session.tracks.iter().enumerate() {
            if t.clips.is_empty() {
                rows.push(vec![
                    format!("{} / empty", n + 1),
                    "—".into(),
                    "—".into(),
                    "—".into(),
                ]);
            }
            for (i, c) in t.clips.iter().enumerate() {
                rows.push(vec![format!(
                    "{} {} / {}",
                    n + 1,
                    if c.midi_pattern().is_some() {
                        "MIDI"
                    } else {
                        "Audio"
                    },
                    i + 1
                )]);
            }
        }
        ui.set_rows(self.timeline, Rows(rows));
        let selected = self
            .session
            .tracks
            .iter()
            .take(self.session.selected)
            .map(|t| t.clips.len().max(1))
            .sum();
        ui.select_row(self.timeline, Some(selected));
        if let Some(t) = self.session.tracks.get(self.session.selected) {
            ui.set_value(self.volume, t.volume() as f32);
            ui.set_value(self.pan, t.pan() as f32);
            ui.set_checked(self.mute, t.muted);
            ui.set_checked(self.solo, t.solo);
            ui.set_checked(self.eq, t.effects.eq.is_some());
            ui.set_checked(self.compressor, t.effects.compressor.is_some());
            ui.set_checked(self.reverb, t.effects.reverb.is_some());
            ui.set_value(self.gain, t.clips.first().map_or(100, Clip::gain) as f32);
        }
    }
    fn guard_action(&mut self, ui: &mut Ui, action: Action) -> Option<Action> {
        if self.pending.is_some() {
            return None;
        }
        if self.session.is_dirty() {
            let id = ui.message(
                "Unsaved Moan project",
                "Discard unsaved edits? Save Project keeps editable tracks and audio.",
                &["Cancel", "Discard"],
                0,
                0,
            );
            self.pending = Some((id, action));
            None
        } else {
            Some(action)
        }
    }
    pub fn request_close(&mut self, ui: &mut Ui) -> Option<Action> {
        self.request_close_with_pending(ui, false)
    }
    pub fn request_close_with_pending(
        &mut self,
        ui: &mut Ui,
        pending_recording: bool,
    ) -> Option<Action> {
        if let Some((_, action)) = &mut self.pending {
            *action = Action::Quit;
            return None;
        }
        if pending_recording && !self.session.is_dirty() {
            let id = ui.message(
                "Unsaved microphone recording",
                "Discard the recording and close? Stop keeps it as an editable track.",
                &["Cancel", "Discard"],
                0,
                0,
            );
            self.pending = Some((id, Action::Quit));
            None
        } else {
            self.guard_action(ui, Action::Quit)
        }
    }
    pub fn load_project(&mut self, ui: &mut Ui, bytes: &[u8]) -> Result<(), project::Error> {
        let session = Session::decode_project(bytes)?;
        self.session = session;
        self.view = waveform::View {
            start: 0,
            span: self.session.duration().max(RATE as usize * 2),
        };
        self.drag = None;
        self.refresh(ui);
        Ok(())
    }
    pub fn new_project(&mut self, ui: &mut Ui) {
        self.session = Session::default();
        self.view = waveform::View::default();
        self.drag = None;
        self.refresh(ui);
    }
    fn change_view(&mut self, command: &str) {
        match command {
            "Fit audio" => {
                self.view.start = 0;
                self.view.span = self.session.duration().max(RATE as usize * 2);
            }
            "Earlier" => self.view.start = self.view.start.saturating_sub(self.view.span / 2),
            "Later" => {
                self.view.start = self
                    .view
                    .start
                    .saturating_add(self.view.span / 2)
                    .min(project::MAX_TIMELINE.saturating_sub(self.view.span))
            }
            _ => {
                self.view.span = if command == "Zoom in" {
                    self.view.span / 2
                } else {
                    self.view.span.saturating_mul(2)
                }
                .clamp(RATE as usize / 10, project::MAX_TIMELINE);
                self.view.start = self
                    .session
                    .cursor
                    .saturating_sub(self.view.span / 2)
                    .min(project::MAX_TIMELINE - self.view.span);
            }
        }
    }
    /// Intercept only document content interactions; all widgets still belong to Weft.
    pub fn handle_event(&mut self, ui: &mut Ui, event: weft::Event) -> Vec<Response> {
        if let weft::Event::Pointer(pointer) = event {
            if self.pointer(ui, pointer) {
                return vec![Response::Command("moan.selection".into())];
            }
        }
        let resized = matches!(event, weft::Event::Resize { .. });
        let mut responses = ui.handle(event);
        if resized {
            self.layout_document(ui);
            ui.layout();
            responses.push(Response::Command("moan.layout".into()));
        }
        responses
    }
    fn pointer(&mut self, ui: &mut Ui, pointer: weft::Pointer) -> bool {
        use weft::{Button, Point, PointerKind};
        // Weft appends popup/dialog/palette controls after the retained app tree.
        // Their focused widget keeps ownership of pointer input above the canvas.
        if self.busy
            || !ui.find_kind("menurow").is_empty()
            || self.pending.is_some()
            || ui
                .focused()
                .is_some_and(|id| id.index() > self.app_end.index())
        {
            self.drag = None;
            return false;
        }
        if pointer.kind == PointerKind::Leave {
            self.drag = None;
            return false;
        }
        ui.layout();
        let piano_rect = ui.rect(self.piano);
        if pointer.kind == PointerKind::Down(Button::Left) {
            let p = Point::new(pointer.x / ui.theme().scale, pointer.y / ui.theme().scale);
            if self.workspace == Workspace::PianoRoll
                && piano_rect
                    .intersect(&weft::Rect::new(
                        0.0,
                        0.0,
                        ui.size().0 as f32 / ui.theme().scale,
                        ui.size().1 as f32 / ui.theme().scale,
                    ))
                    .contains(p)
                && self.session.midi_clip().is_some()
            {
                let x =
                    ((p.x - piano_rect.x) / piano_rect.w * piano::WIDTH as f32).max(0.0) as usize;
                let y =
                    ((p.y - piano_rect.y) / piano_rect.h * piano::HEIGHT as f32).max(0.0) as usize;
                if x < piano::KEYBOARD {
                    return false;
                }
                let step = piano::step(x);
                let key = piano::key(y);
                let velocity = self.note_velocity;
                ui.focus(None);
                if let Err(message) = self.session.edit_midi(|p| {
                    let grid = (p.frames / piano::STEPS as u32).max(1);
                    let start = (step as u32 * grid).min(p.frames - 1);
                    if let Some(i) = p
                        .notes
                        .iter()
                        .position(|n| n.key == key && n.start / grid == step as u32)
                    {
                        p.notes.remove(i);
                    } else {
                        p.notes.push(moan_core::midi::Note {
                            start,
                            frames: grid.min(p.frames - start),
                            key,
                            velocity,
                        });
                    }
                }) {
                    self.message(ui, message);
                } else {
                    self.refresh(ui);
                }
                return true;
            }
        }
        let rect = ui.rect(self.waveform);
        let p = Point::new(pointer.x / ui.theme().scale, pointer.y / ui.theme().scale);
        let visible = rect.intersect(&ui.rect(self.viewport));
        let x = (((p.x - rect.x).max(0.0) / (rect.w - 1.0).max(1.0) * (waveform::WIDTH - 1) as f32
            + 0.5) as usize)
            .min(waveform::WIDTH - 1);
        let frame = self.view.frame(x);
        if pointer.kind == PointerKind::Down(Button::Left)
            && visible.contains(p)
            && p.y >= rect.y + waveform::HEADER as f32
        {
            let track = ((p.y - rect.y - waveform::HEADER as f32) / waveform::ROW as f32) as usize;
            if track >= self.session.tracks.len() {
                return false;
            }
            ui.focus(None);
            self.session.selected = track;
            self.session.cursor = frame;
            self.session.from = frame;
            self.session.to = frame;
            self.drag = if self.move_clips {
                self.session.tracks[track]
                    .clips
                    .iter()
                    .enumerate()
                    .find(|(_, c)| frame >= c.start() && frame < c.end())
                    .map(|(clip, c)| Drag::Move {
                        track,
                        clip,
                        anchor: frame,
                        start: c.start(),
                        remembered: false,
                    })
            } else {
                Some(Drag::Select {
                    track,
                    anchor: frame,
                })
            };
            self.refresh(ui);
            return true;
        }
        if matches!(
            pointer.kind,
            PointerKind::Move | PointerKind::Up(Button::Left)
        ) {
            if let Some(mut drag) = self.drag {
                match &mut drag {
                    Drag::Select { track, anchor } => {
                        self.session.selected = *track;
                        self.session.from = (*anchor).min(frame);
                        self.session.to = (*anchor).max(frame);
                        self.session.cursor = frame;
                    }
                    Drag::Move {
                        track,
                        clip,
                        anchor,
                        start,
                        remembered,
                    } => {
                        let len = self.session.tracks[*track].clips[*clip].len();
                        let new = if frame >= *anchor {
                            start.saturating_add(frame - *anchor)
                        } else {
                            start.saturating_sub(*anchor - frame)
                        }
                        .min(project::MAX_TIMELINE - len);
                        if new != self.session.tracks[*track].clips[*clip].start() {
                            if !*remembered {
                                self.session.remember();
                                *remembered = true;
                            }
                            let _ = self.session.tracks[*track].clips[*clip].move_to(new);
                        }
                        self.session.cursor = frame;
                    }
                }
                self.drag = if pointer.kind == PointerKind::Up(Button::Left) {
                    None
                } else {
                    Some(drag)
                };
                self.refresh(ui);
                return true;
            }
        }
        false
    }
    /// Same native backend lifecycle as Weft App::next, with document pointer routing.
    #[cfg(feature = "sexos")]
    pub fn next(&mut self, app: &mut weft::sexos::App) -> Vec<Response> {
        app.present();
        let wait = app.ui.tick(weft::sexos::now());
        let event = match wait {
            Some(ms) => app.window.next_within(ms.max(1)),
            None => Some(app.window.next()),
        };
        app.ui.tick(weft::sexos::now());
        let mut out = Vec::new();
        if let Some(event) = event {
            out.extend(self.handle_event(&mut app.ui, weft::sexos::event(event)));
            for _ in 0..64 {
                let Some(event) = app.window.poll() else {
                    break;
                };
                out.extend(self.handle_event(&mut app.ui, weft::sexos::event(event)));
            }
        }
        app.present();
        out
    }
    fn fields(&mut self, ui: &Ui) -> Result<(), &'static str> {
        fn frames(text: &str) -> Result<usize, &'static str> {
            let v = text
                .parse::<f64>()
                .map_err(|_| "Enter seconds as a number")?;
            if !v.is_finite() || v < 0.0 || v > project::MAX_TIMELINE as f64 / RATE as f64 {
                return Err("Seconds must be 0 to 3660");
            }
            Ok((v * RATE as f64 + 0.5) as usize)
        }
        self.session.cursor = frames(ui.text_of(self.position))?;
        self.session.from = frames(ui.text_of(self.from))?;
        self.session.to = frames(ui.text_of(self.to))?;
        Ok(())
    }
    pub fn on(&mut self, ui: &mut Ui, r: &Response) -> Result<Option<Action>, &'static str> {
        if let Response::Dialog(id, value) = r {
            if let Some((pending, action)) = self.pending {
                if *id == pending {
                    self.pending = None;
                    return Ok(if *value == weft::DialogValue::Button(1) {
                        Some(action)
                    } else {
                        None
                    });
                }
            }
        }
        if *r == Response::Toggled(self.move_mode, true)
            || *r == Response::Toggled(self.move_mode, false)
        {
            self.move_clips = ui.is_checked(self.move_mode);
            return Ok(None);
        }

        if let Response::Clicked(id) = r {
            if let Some(track) = self.track_buttons.iter().position(|item| item == id) {
                if !self.busy && track < self.session.tracks.len() {
                    self.session.selected = track;
                    self.refresh(ui);
                }
                return Ok(None);
            }
        }
        if self.busy {
            let allowed = match r {
                Response::Clicked(id) => self.buttons.iter().any(|(b, name)| {
                    b == id
                        && [
                            "Play",
                            "Pause",
                            "Stop",
                            "Arrange",
                            "Mix",
                            "Piano Roll",
                            "Instruments",
                            "Library",
                        ]
                        .contains(name)
                }),
                Response::Command(c) => [
                    "moan.play",
                    "moan.stop",
                    "moan.mixer",
                    "moan.piano",
                    "moan.timeline",
                ]
                .contains(&c.as_str()),
                _ => false,
            };
            if !allowed {
                return Ok(None);
            }
        }
        if let Response::Selected(id) = r {
            if *id == self.timeline {
                if let Some(row) = ui.selected_row(*id) {
                    let mut at = 0;
                    for (n, t) in self.session.tracks.iter().enumerate() {
                        if row < at + t.clips.len().max(1) {
                            self.session.selected = n;
                            break;
                        }
                        at += t.clips.len().max(1);
                    }
                    self.refresh(ui);
                }
                return Ok(None);
            }
        }
        if let Response::Changed(id) = r {
            if *id == self.velocity {
                self.note_velocity = ui.value(*id) as u8;
                self.refresh(ui);
                return Ok(None);
            }
            if let Some(index) = self.mixer_volume.iter().position(|v| v == id) {
                if index < self.session.tracks.len() {
                    self.session.remember();
                    self.session.tracks[index]
                        .set_volume(ui.value(*id) as u16)
                        .map_err(|_| "Volume")?;
                    self.refresh(ui);
                }
                return Ok(None);
            }
            if let Some(index) = self.mixer_pan.iter().position(|v| v == id) {
                if index < self.session.tracks.len() {
                    self.session.remember();
                    self.session.tracks[index]
                        .set_pan(ui.value(*id) as i16)
                        .map_err(|_| "Pan")?;
                    self.refresh(ui);
                }
                return Ok(None);
            }
            if *id == self.gain {
                self.session.gain(ui.value(*id) as u16)?;
                self.refresh(ui);
            } else if *id == self.volume || *id == self.pan {
                let volume = ui.value(self.volume) as u16;
                let pan = ui.value(self.pan) as i16;
                if self.session.selected < self.session.tracks.len() {
                    self.session.remember();
                    let t = &mut self.session.tracks[self.session.selected];
                    t.set_volume(volume).map_err(|_| "Volume")?;
                    t.set_pan(pan).map_err(|_| "Pan")?;
                }
            }
            return Ok(None);
        }
        if let Response::Toggled(id, on) = r {
            if self.session.selected >= self.session.tracks.len() {
                return Err("Select a track");
            }
            self.session.remember();
            let t = &mut self.session.tracks[self.session.selected];
            if *id == self.mute {
                t.muted = *on;
            }
            if *id == self.solo {
                t.solo = *on;
            }
            if *id == self.eq {
                t.effects.eq = if *on {
                    Some(moan_core::effects::Equalizer::new(125, 100, 75).unwrap())
                } else {
                    None
                };
            }
            if *id == self.compressor {
                t.effects.compressor = if *on {
                    Some(moan_core::effects::Compressor::new(6000, 4, 48, 4800).unwrap())
                } else {
                    None
                };
            }
            if *id == self.reverb {
                t.effects.reverb = if *on {
                    Some(moan_core::effects::Reverb::new(65, 30).unwrap())
                } else {
                    None
                };
            }
            return Ok(None);
        }
        let command = match r {
            Response::Clicked(id) => self.buttons.iter().find(|(b, _)| b == id).map(|(_, s)| *s),
            Response::Command(c) => Some(match c.as_str() {
                "moan.library.keys" => "Silk Keys",
                "moan.library.bass" => "Satin Bass",
                "moan.library.kit" => "Velvet Kit",
                "moan.clear-notes" => "Clear notes",
                "moan.piano" => "Show piano",
                "moan.tracks" => "Show tracks",
                "moan.mixer" => "Show mixer",
                "moan.octave" => "Octave up",
                "moan.lengthen" => "Length +",
                "moan.velocity-half" => "Half velocity",
                "moan.play" => "Play",
                "moan.stop" => "Stop",
                "moan.undo" => "Undo",
                "moan.redo" => "Redo",
                "moan.import" => "Import WAV",
                "moan.export" => "Export WAV",
                "moan.open-project" => "Open Project",
                "moan.save-project" => "Save Project",
                "moan.new-project" => "New Project",
                "moan.format.wav" => "WAV",
                "moan.format.flac" => "FLAC",
                "moan.format.mp3" => "MP3",
                "moan.timeline" => "Show timeline",
                "moan.tone" => "Tone",
                "moan.split" => "Split",
                "moan.cut" => "Cut range",
                "moan.move-clips" => "Drag clips",
                _ => return Ok(None),
            }),
            _ => None,
        };
        let Some(command) = command else {
            return Ok(None);
        };
        if ![
            "Stop",
            "Pause",
            "Undo",
            "Redo",
            "New Project",
            "Open Project",
            "WAV",
            "FLAC",
            "MP3",
            "Zoom in",
            "Zoom out",
            "Fit audio",
            "Earlier",
            "Later",
            "Show timeline",
            "Show piano",
            "Show mixer",
            "Show tracks",
        ]
        .contains(&command)
        {
            self.fields(ui)?;
        }
        let action = match command {
            "Silk Keys" | "Satin Bass" | "Velvet Kit" => {
                let instrument = match command {
                    "Satin Bass" => moan_core::midi::Instrument::SatinPulse,
                    "Velvet Kit" => moan_core::midi::Instrument::VelvetSampler,
                    _ => moan_core::midi::Instrument::SilkTriangle,
                };
                self.session
                    .add_midi(moan_core::midi::Pattern::preset(instrument))?;
                None
            }
            "Show tracks" => {
                self.show_workspace(ui, Workspace::Instruments);
                ui.invalidate();
                ui.focus(Some(self.timeline));
                None
            }
            "Show piano" | "Piano Roll" => {
                self.show_workspace(ui, Workspace::PianoRoll);
                ui.invalidate();
                ui.focus(Some(self.piano));
                None
            }
            "Show mixer" | "Mix" => {
                self.show_workspace(ui, Workspace::Mix);
                ui.invalidate();
                ui.focus(Some(self.meter_values[0]));
                None
            }
            "Arrange" => {
                self.show_workspace(ui, Workspace::Arrange);
                None
            }
            "Instruments" => {
                self.show_workspace(ui, Workspace::Instruments);
                None
            }
            "Library" => {
                self.library_open = !self.library_open;
                self.layout_document(ui);
                ui.invalidate();
                None
            }
            "Clear notes" => {
                self.session.edit_midi(|p| p.notes.clear())?;
                None
            }
            "Octave up" => {
                self.session.edit_midi(|p| {
                    for n in &mut p.notes {
                        n.key = n.key.saturating_add(12).min(127);
                    }
                })?;
                None
            }
            "Half velocity" => {
                self.note_velocity = (self.note_velocity / 2).max(1);
                let velocity = self.note_velocity;
                ui.set_value(self.velocity, velocity as f32);
                self.session.edit_midi(|p| {
                    for n in &mut p.notes {
                        n.velocity = velocity;
                    }
                })?;
                None
            }
            "Length +" => {
                self.session.edit_midi(|p| {
                    for n in &mut p.notes {
                        n.frames = n.frames.saturating_mul(2).min(p.frames - n.start);
                    }
                })?;
                None
            }
            "Set velocity" => {
                let velocity = self.note_velocity;
                self.session.edit_midi(|p| {
                    for n in &mut p.notes {
                        n.velocity = velocity;
                    }
                })?;
                None
            }
            "Quantize" => {
                self.session.edit_midi(|p| {
                    let grid = (p.frames / piano::STEPS as u32).max(1);
                    for n in &mut p.notes {
                        n.start = ((n.start + grid / 2) / grid * grid).min(p.frames - 1);
                        n.frames = n.frames.min(p.frames - n.start);
                    }
                })?;
                None
            }
            "Transpose -" | "Transpose +" => {
                self.session.edit_midi(|p| {
                    for n in &mut p.notes {
                        n.key = if command == "Transpose -" {
                            n.key.saturating_sub(1)
                        } else {
                            (n.key + 1).min(127)
                        };
                    }
                })?;
                None
            }
            "Play" => Some(Action::Play),
            "Pause" => Some(Action::Pause),
            "Stop" => Some(Action::Stop),
            "Record" => Some(Action::Record),
            "Import WAV" => Some(Action::Import),
            "Export WAV" => Some(Action::Export),
            "Open Project" => self.guard_action(ui, Action::OpenProject),
            "New Project" => self.guard_action(ui, Action::NewProject),
            "Save Project" => Some(Action::SaveProject),
            "Drag clips" => {
                self.move_clips = !self.move_clips;
                ui.set_checked(self.move_mode, self.move_clips);
                None
            }
            "Show timeline" => {
                self.show_workspace(ui, Workspace::Arrange);
                ui.focus(Some(self.waveform));
                None
            }
            "WAV" | "FLAC" | "MP3" => {
                self.export_format = match command {
                    "FLAC" => 1,
                    "MP3" => 2,
                    _ => 0,
                };
                None
            }
            "Zoom in" | "Zoom out" | "Fit audio" | "Earlier" | "Later" => {
                self.change_view(command);
                None
            }
            "Tone" => {
                self.session.tone()?;
                None
            }
            "Split" => {
                self.session.split()?;
                None
            }
            "Cut range" => {
                self.session.cut()?;
                None
            }
            "Fade edges" => {
                self.session.fades()?;
                None
            }
            "Undo" => {
                self.session.undo();
                None
            }
            "Redo" => {
                self.session.redo();
                None
            }
            _ => None,
        };
        self.refresh(ui);
        if action.is_none() {
            self.message(
                ui,
                &format!(
                    "{} tracks, {:.2} seconds",
                    self.session.tracks.len(),
                    self.session.duration() as f64 / RATE as f64
                ),
            );
        }
        Ok(action)
    }
}
