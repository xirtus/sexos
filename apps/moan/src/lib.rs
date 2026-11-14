// SPDX-License-Identifier: GPL-3.0-only
//! Moan's editable project and Weft window, shared by guest and host UI tests.
#![no_std]
extern crate alloc;
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
    tracks: Vec<Track<'static>>,
    selected: usize,
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
        let mut tracks = self.tracks.clone();
        for t in &mut tracks {
            t.reset_effects();
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
        let current = self.snapshot();
        self.redo.push(current);
        self.revision = s.revision;
        self.cursor = s.cursor;
        self.from = s.from;
        self.to = s.to;
        self.tracks = s.tracks;
        self.selected = s.selected;
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.undo.push(current);
        self.revision = s.revision;
        self.cursor = s.cursor;
        self.from = s.from;
        self.to = s.to;
        self.tracks = s.tracks;
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
    pub fn tone(&mut self) -> Result<(), &'static str> {
        let period = 120usize.saturating_sub(self.tracks.len() * 8).max(40);
        self.add(
            (0..RATE as usize * 2)
                .map(|i| [if i % period < period / 2 { 4000 } else { -4000 }; 2])
                .collect(),
        )
    }
    pub fn split(&mut self) -> Result<(), &'static str> {
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
pub struct Studio {
    pub export_format: u8,
    pub waveform: Id,
    pub view: waveform::View,
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
                Menu::new("Transport")
                    .command("moan.play")
                    .command("moan.stop"),
            ],
        );
        let scroll = ui.scroll(root);
        ui.set_layout(scroll, Layout::column().fill());
        let body = ui.group(
            scroll,
            Layout::column()
                .gap(8.0)
                .padding(10.0)
                .fill_width()
                .shrink(0.0),
        );
        ui.text(body, "Moan", TextStyle::Title);
        let row = ui.row(body);
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
                    row,
                    if name == "Export WAV" {
                        "Export Audio"
                    } else {
                        name
                    },
                ),
                name,
            ));
        }
        let status = ui.text(
            body,
            "Add a tone, import a 48 kHz PCM WAV, or record a track.",
            TextStyle::Caption,
        );
        let time = ui.row(body);
        ui.text(time, "Playhead (seconds)", TextStyle::Caption);
        let position = ui.field(time, "0", "Seconds");
        ui.text(time, "Range", TextStyle::Caption);
        let from = ui.field(time, "0", "Start seconds");
        let to = ui.field(time, "1", "End seconds");
        for field in [position, from, to] {
            ui.set_layout(field, Layout::default().width(85.0));
        }
        let timeline = ui.list_with(
            body,
            alloc::boxed::Box::new(Rows::default()),
            vec![
                Column::new("Track / clip", 130.0),
                Column::new("Start (s)", 80.0),
                Column::new("End (s)", 80.0),
                Column::new("Gain", 65.0),
            ],
        );
        ui.set_layout(timeline, Layout::default().fill_width().min_height(100.0));
        let edits = ui.row(body);
        for name in ["Split", "Cut range", "Fade edges", "Undo", "Redo"] {
            buttons.push((ui.button(edits, name), name));
        }
        let project_row = ui.row(body);
        ui.set_layout(project_row, Layout::row().gap(6.0).wrap());
        for name in [
            "New Project",
            "Open Project",
            "Save Project",
            "WAV",
            "FLAC",
            "MP3",
        ] {
            buttons.push((ui.button(project_row, name), name));
        }
        let view_row = ui.row(body);
        ui.set_layout(view_row, Layout::row().gap(6.0).wrap());
        for name in ["Zoom in", "Zoom out", "Fit audio", "Earlier", "Later"] {
            buttons.push((ui.button(view_row, name), name));
        }
        let move_mode = ui.checkbox(view_row, "Drag clips", false);
        let scale_label = ui.text(
            body,
            "0.00s — 4.00s. Drag to select time; enable Drag clips to move audio.",
            TextStyle::Caption,
        );
        let (height, pixels) = waveform::render(&Session::default(), waveform::View::default());
        let waveform = ui.image(body, waveform::WIDTH, height, pixels);
        ui.set_label(
            waveform,
            "Audio waveform timeline. Track rows, selected time range and playhead.",
        );
        let mixer = ui.row(body);
        ui.text(mixer, "Volume", TextStyle::Caption);
        let volume = ui.slider(mixer, 0.0, 200.0, 100.0);
        ui.text(mixer, "Pan", TextStyle::Caption);
        let pan = ui.slider(mixer, -100.0, 100.0, 0.0);
        ui.text(mixer, "Clip gain", TextStyle::Caption);
        let gain = ui.slider(mixer, 0.0, 200.0, 100.0);
        let fx = ui.row(body);
        let mute = ui.checkbox(fx, "Mute", false);
        let solo = ui.checkbox(fx, "Solo", false);
        let eq = ui.checkbox(fx, "Warm EQ", false);
        let compressor = ui.checkbox(fx, "Compressor", false);
        let reverb = ui.checkbox(fx, "Reverb", false);
        Self {
            export_format: 0,
            waveform,
            view: waveform::View::default(),
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
        }
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
        for (id, name) in &self.buttons {
            if !["Play", "Pause", "Stop"].contains(name) {
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
        ] {
            ui.commands.set_enabled(command, !busy);
        }
    }
    pub fn message(&self, ui: &mut Ui, text: &str) {
        ui.set_text(self.status, text);
    }
    pub fn refresh(&self, ui: &mut Ui) {
        let (height, pixels) = waveform::render(&self.session, self.view);
        ui.set_image(self.waveform, waveform::WIDTH, height, pixels);
        ui.set_layout(
            self.waveform,
            Layout::default()
                .width(waveform::WIDTH as f32)
                .height(height as f32)
                .shrink(0.0),
        );
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
                rows.push(vec![
                    format!("{} / {}", n + 1, i + 1),
                    format!("{:.3}", c.start() as f64 / RATE as f64),
                    format!("{:.3}", c.end() as f64 / RATE as f64),
                    format!("{}%", c.gain()),
                ]);
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
        if pending_recording && !self.session.is_dirty() && self.pending.is_none() {
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
        ui.handle(event)
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
                .is_some_and(|id| id.index() > self.reverb.index())
        {
            self.drag = None;
            return false;
        }
        if pointer.kind == PointerKind::Leave {
            self.drag = None;
            return false;
        }
        ui.layout();
        let rect = ui.rect(self.waveform);
        let p = Point::new(pointer.x / ui.theme().scale, pointer.y / ui.theme().scale);
        let visible = rect.intersect(&ui.rect(self.viewport));
        let x = ((p.x - rect.x).max(0.0) as usize).min(waveform::WIDTH - 1);
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

        if self.busy {
            let allowed = match r {
                Response::Clicked(id) => self
                    .buttons
                    .iter()
                    .any(|(b, name)| b == id && ["Play", "Pause", "Stop"].contains(name)),
                Response::Command(c) => c == "moan.play" || c == "moan.stop",
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
        ]
        .contains(&command)
        {
            self.fields(ui)?;
        }
        let action = match command {
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
