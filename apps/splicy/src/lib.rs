// SPDX-License-Identifier: GPL-3.0-only
//! Splicy, the SexOS video editor: what the window edits, apart from the window.
//!
//! [`Editor`] holds one clip's edit: its timeline history, its transcript and the
//! suggestions waiting for the editor. Words are picked with a caret and a selection, and
//! deleting them cuts their pictures and sound from every track at once (sex-splice's
//! ripple delete), so the transcript is the way to edit, as in Scarlet. Nothing here
//! touches a file or the screen: [`words`] reads and writes the transcript beside the
//! clip, [`view`] paints, and the program in `main.rs` does the rest. So all of it is
//! tested on the host.
#![cfg_attr(feature = "sexos", no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod cache;
pub mod compositor;
pub mod media;
pub mod view;
pub mod vu;
pub mod words;

pub use cache::{CacheConfig, CachedFrame, FrameCache};
pub use compositor::{composite_cpu, BlendMode, CompositeOutput, LayerInput, LayerTransform, QualityMode};
pub use vu::{TrackMeter, VuStrip, VU_STRIP_H};

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_splice::history::History;
use sex_splice::suggest::{Author, Status, Suggestions};
use sex_splice::transcript::{ripple_ops, Transcript};
use sex_splice::{EditError, MediaKind, Micros, Op, Range, Timeline, TrackKind, SECOND};

/// How far either side of a join `\` plays, as in Scarlet's "hear the cut".
pub const JOIN_CONTEXT: Micros = 2 * SECOND;
/// The clean-up pass shortens pauses longer than this...
pub const LONG_PAUSE: Micros = SECOND;
/// ...down to this.
pub const KEEP_PAUSE: Micros = 300_000;

/// Audition mode for `\`: hear the cut itself or hear what was removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditionMode {
    HearJoin,
    HearRemoved,
}

pub struct Editor {
    pub history: History,
    pub transcript: Transcript,
    pub suggestions: Suggestions,
    /// The word the caret is on.
    pub caret: usize,
    /// The other end of the selection, when there is one.
    pub anchor: Option<usize>,
    /// Which pending suggestion card has the focus.
    pub card: usize,
    pub audition: AuditionMode,
}

impl Editor {
    /// One clip on a picture track and its sound on a sound track, both playing all of it.
    /// `transcript.media` is set to the clip.
    pub fn open(path: &str, duration: Micros, fps: (u32, u32), sound: bool, mut transcript: Transcript) -> Editor {
        let (num, den) = if fps.0 == 0 || fps.1 == 0 { (30, 1) } else { fps };
        let mut tl = Timeline::new(num, den);
        let media = tl.add_media(MediaKind::Video, path, duration.max(1));
        let mut ops = Vec::new();
        let source = Range::new(0, duration.max(1));
        let v = tl.add_track(TrackKind::Video, "Picture");
        ops.push(Op::AddClip { track: v, media, at: 0, source });
        if sound {
            let a = tl.add_track(TrackKind::Audio, "Sound");
            ops.push(Op::AddClip { track: a, media, at: 0, source });
        }
        let tl = tl.apply_all(&ops).unwrap_or(tl);
        transcript.media = media;
        Editor {
            history: History::new(tl),
            transcript,
            suggestions: Suggestions::default(),
            caret: 0,
            anchor: None,
            card: 0,
            audition: AuditionMode::HearJoin,
        }
    }

    pub fn timeline(&self) -> &Timeline {
        self.history.current()
    }

    /// For each word, whether it still plays.
    pub fn playing(&self) -> Vec<bool> {
        let mut out = alloc::vec![false; self.transcript.words.len()];
        for (i, _) in self.transcript.playing(self.timeline()) {
            out[i] = true;
        }
        out
    }

    /// The selected words, first to last; the caret's word when nothing is selected.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let n = self.transcript.words.len();
        if n == 0 {
            return None;
        }
        let c = self.caret.min(n - 1);
        let a = self.anchor.unwrap_or(c).min(n - 1);
        Some((a.min(c), a.max(c)))
    }

    /// Move the caret by `delta` words; `extend` grows the selection instead of dropping it.
    pub fn move_caret(&mut self, delta: isize, extend: bool) {
        let n = self.transcript.words.len();
        if n == 0 {
            return;
        }
        if extend && self.anchor.is_none() {
            self.anchor = Some(self.caret);
        }
        if !extend {
            self.anchor = None;
        }
        self.caret = (self.caret as isize + delta).clamp(0, n as isize - 1) as usize;
    }

    /// Put the caret on word `i`, as a click does; `extend` selects up to it.
    pub fn place(&mut self, i: usize, extend: bool) {
        if i >= self.transcript.words.len() {
            return;
        }
        if extend {
            self.anchor.get_or_insert(self.caret);
        } else {
            self.anchor = None;
        }
        self.caret = i;
    }

    /// Select the sentence the caret is in: back to the word after the last one ending in
    /// `.`, `?` or `!`, and on to the next one that does.
    pub fn select_sentence(&mut self) {
        let w = &self.transcript.words;
        if w.is_empty() {
            return;
        }
        let ends = |i: usize| w[i].text.trim_end_matches(['"', '\'', ')']).ends_with(['.', '?', '!']);
        let c = self.caret.min(w.len() - 1);
        let mut first = c;
        while first > 0 && !ends(first - 1) {
            first -= 1;
        }
        let mut last = c;
        while last + 1 < w.len() && !ends(last) {
            last += 1;
        }
        self.anchor = Some(first);
        self.caret = last;
    }

    /// Cut the selected words (or the caret's word) from the edit. False when they no
    /// longer play, so there is nothing to cut.
    pub fn delete(&mut self) -> Result<bool, EditError> {
        let Some((first, last)) = self.selection() else { return Ok(false) };
        let Some(cut) = self.transcript.cut_words(first, last) else { return Ok(false) };
        let ops = ripple_ops(self.timeline(), &[cut]);
        if ops.is_empty() {
            return Ok(false);
        }
        let next = self.timeline().apply_all(&ops)?;
        let summary = format!("Cut \"{}\"", self.quote(first, last));
        self.history.commit(&summary, next);
        self.anchor = None;
        self.caret = (last + 1).min(self.transcript.words.len() - 1);
        Ok(true)
    }

    /// Words `first..=last`, shortened to about 40 characters for a summary.
    pub fn quote(&self, first: usize, last: usize) -> String {
        let mut s = String::new();
        for w in &self.transcript.words[first..=last] {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(&w.text);
            if s.chars().count() > 40 {
                let mut t: String = s.chars().take(40).collect();
                t.push_str("...");
                return t;
            }
        }
        s
    }

    pub fn undo(&mut self) -> bool {
        self.history.undo()
    }

    pub fn redo(&mut self) -> bool {
        self.history.redo()
    }

    /// Stage the clean-up pass as suggestions for the editor to take or skip: earlier
    /// takes of a repeated phrase, filler words, and long pauses. Returns how many cards
    /// were added. Nothing changes until one is accepted.
    pub fn suggest_cleanup(&mut self) -> usize {
        let before = self.suggestions.items.len();
        let t = &self.transcript;
        for (first, last) in t.repeated_takes(3) {
            if let Some(cut) = t.cut_words(first, last) {
                let summary = format!("Remove the first take of \"{}\"", self.quote(first, last));
                self.suggestions.stage(Author::Rule, &summary, alloc::vec![cut]);
            }
        }
        let fillers = t.filler_cuts();
        if !fillers.is_empty() {
            let summary = format!("Remove {} filler word{}", fillers.len(), if fillers.len() == 1 { "" } else { "s" });
            self.suggestions.stage(Author::Rule, &summary, fillers);
        }
        let pauses = t.pause_cuts(LONG_PAUSE, KEEP_PAUSE);
        if !pauses.is_empty() {
            let summary = format!("Shorten {} long pause{}", pauses.len(), if pauses.len() == 1 { "" } else { "s" });
            self.suggestions.stage(Author::Rule, &summary, pauses);
        }
        self.card = 0;
        self.suggestions.items.len() - before
    }

    /// The pending suggestions, in the order their cards show.
    pub fn pending(&self) -> Vec<u32> {
        self.suggestions.pending().map(|s| s.id).collect()
    }

    pub fn next_card(&mut self) {
        let n = self.pending().len();
        self.card = if n == 0 { 0 } else { (self.card + 1) % n };
    }

    /// Apply the focused card's cuts to the edit as it is now.
    pub fn accept(&mut self) -> Result<bool, EditError> {
        let Some(&id) = self.pending().get(self.card) else { return Ok(false) };
        let tl = self.timeline().clone();
        let summary = self.suggestions.get(id).map(|s| s.summary.clone()).unwrap_or_default();
        match self.suggestions.apply(id, &tl)? {
            Some(next) => {
                self.history.commit(&summary, next);
                self.card = self.card.min(self.pending().len().saturating_sub(1));
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn skip(&mut self) -> bool {
        let Some(&id) = self.pending().get(self.card) else { return false };
        let done = self.suggestions.skip(id);
        self.card = self.card.min(self.pending().len().saturating_sub(1));
        done
    }

    /// Where the caret's word plays now; for a cut word, where the next playing word does.
    pub fn caret_time(&self) -> Option<Micros> {
        let playing = self.transcript.playing(self.timeline());
        playing.iter().find(|(i, _)| *i >= self.caret).or(playing.last()).map(|(_, t)| *t)
    }

    /// The word playing at timeline time `t`, or the last one that started before it.
    pub fn word_at_time(&self, t: Micros) -> Option<usize> {
        let playing = self.transcript.playing(self.timeline());
        let i = playing.partition_point(|(_, at)| *at <= t);
        playing.get(i.checked_sub(1)?).map(|(w, _)| *w)
    }

    /// The timeline times where two pieces of the source meet because something between
    /// them was cut.
    pub fn joins(&self) -> Vec<Micros> {
        let tl = self.timeline();
        let Some(track) = tl.tracks.iter().find(|t| t.kind == TrackKind::Video) else { return Vec::new() };
        track
            .clips
            .windows(2)
            .filter(|w| w[0].end() == w[1].at && (w[0].media != w[1].media || w[0].source.end != w[1].source.start))
            .map(|w| w[1].at)
            .collect()
    }

    /// What `\` plays: two seconds either side of the join nearest the caret.
    pub fn join_preview(&self) -> Option<Range> {
        let at = self.caret_time().unwrap_or(0);
        let join = self.joins().into_iter().min_by_key(|j| (j - at).abs())?;
        Some(Range::new((join - JOIN_CONTEXT).max(0), (join + JOIN_CONTEXT).min(self.timeline().duration())))
    }

    /// Finds the nearest join on the video track where `w[0].media == w[1].media && w[0].source.end < w[1].source.start`
    /// and returns `Some((w[0].media, Range::new(w[0].source.end, w[1].source.start)))`.
    pub fn removed_at_nearest_join(&self) -> Option<(u32, Range)> {
        let tl = self.timeline();
        let track = tl.tracks.iter().find(|t| t.kind == TrackKind::Video)?;
        let at = self.caret_time().unwrap_or(0);
        let candidate = track
            .clips
            .windows(2)
            .filter(|w| w[0].end() == w[1].at && w[0].media == w[1].media && w[0].source.end < w[1].source.start)
            .min_by_key(|w| (w[1].at - at).abs())?;
        Some((candidate[0].media, Range::new(candidate[0].source.end, candidate[1].source.start)))
    }

    /// Toggles audition mode between `HearJoin` and `HearRemoved`.
    pub fn cycle_audition(&mut self) -> AuditionMode {
        self.audition = match self.audition {
            AuditionMode::HearJoin => AuditionMode::HearRemoved,
            AuditionMode::HearRemoved => AuditionMode::HearJoin,
        };
        self.audition
    }

    /// How many suggestions were applied and skipped, for the status line.
    pub fn decided(&self) -> (usize, usize) {
        let s = &self.suggestions.items;
        (s.iter().filter(|s| s.status == Status::Applied).count(), s.iter().filter(|s| s.status == Status::Skipped).count())
    }
}

#[cfg(test)]
mod tests;
