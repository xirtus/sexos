// SPDX-License-Identifier: GPL-3.0-only
//! Parts of a video worth skipping, and when to offer the next one: Qupid's "Skip
//! intro", "Skip credits" and "Up next" cards. Adapted from brootlite, which gets
//! these from Jellyfin's media segments; here they come from the file's own chapter
//! names ("Opening", "Intro", "Recap", "Ending", "Credits", "Preview"), and a server
//! that knows them can add [`Segment`]s directly. Times only: no drawing.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Recap, Intro, Credits, Preview }

impl Kind {
    /// What the skip card says.
    pub fn prompt(self) -> &'static str {
        match self { Kind::Recap => "Skip recap", Kind::Intro => "Skip intro", Kind::Credits => "Skip credits", Kind::Preview => "Skip preview" }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment { pub kind: Kind, pub start_ms: u64, pub end_ms: u64 }

/// One chapter as the file lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter { pub title: String, pub start_ms: u64 }

fn has_word(title: &str, word: &str) -> bool {
    title.split(|c: char| !c.is_alphanumeric()).any(|w| w.eq_ignore_ascii_case(word))
}

/// What a chapter's title says it is.
pub fn classify(title: &str) -> Option<Kind> {
    let t = title.to_ascii_lowercase();
    if t.contains("recap") || t.contains("previously") { return Some(Kind::Recap); }
    if t.contains("preview") || t.contains("next episode") || t.contains("next time") { return Some(Kind::Preview); }
    if t.contains("credit") || t.contains("ending") || t.contains("outro") || has_word(&t, "ed") || has_word(&t, "end") { return Some(Kind::Credits); }
    if t.contains("intro") || t.contains("opening") || has_word(&t, "op") || t.contains("theme song") || t.contains("title sequence") { return Some(Kind::Intro); }
    None
}

/// Segments from chapters: each named chapter runs until the next chapter starts, the
/// last until `duration_ms`. Chapters need not be in order.
pub fn from_chapters(chapters: &[Chapter], duration_ms: u64) -> Vec<Segment> {
    let mut sorted: Vec<&Chapter> = chapters.iter().collect();
    sorted.sort_by_key(|c| c.start_ms);
    let mut out = Vec::new();
    for (i, c) in sorted.iter().enumerate() {
        let Some(kind) = classify(&c.title) else { continue };
        let end = sorted.get(i + 1).map_or(duration_ms, |n| n.start_ms);
        if end > c.start_ms {
            // Two chapters in a row of one kind ("Ending", "Ending (cont.)") make one.
            match out.last_mut() {
                Some(Segment { kind: k, end_ms, .. }) if *k == kind && *end_ms == c.start_ms => *end_ms = end,
                _ => out.push(Segment { kind, start_ms: c.start_ms, end_ms: end }),
            }
        }
    }
    out
}

/// A skip card is not offered in a segment's last moments: skipping would save nothing.
pub const SKIP_TAIL_MS: u64 = 1500;

/// The segment to offer skipping at `ms`.
pub fn skip_at(segments: &[Segment], ms: u64) -> Option<&Segment> {
    segments.iter().find(|s| ms >= s.start_ms && ms + SKIP_TAIL_MS < s.end_ms)
}

/// How long the "Up next" card counts down before the next item starts.
pub const COUNTDOWN_MS: u64 = 10_000;
/// Without credits chapters, the card shows this long before the end.
pub const UP_NEXT_LEAD_MS: u64 = 30_000;
/// Credits that start earlier than this share of the video are not the end credits.
const CREDITS_FROM_PERCENT: u64 = 75;

/// When the "Up next" card appears: the end credits' start when the chapters name
/// them, else a little before the end. None for a video too short to need it.
pub fn up_next_from(segments: &[Segment], duration_ms: u64) -> Option<u64> {
    if duration_ms < 2 * UP_NEXT_LEAD_MS { return None; }
    let credits = segments.iter().rev()
        .find(|s| s.kind == Kind::Credits && s.start_ms * 100 >= duration_ms * CREDITS_FROM_PERCENT)
        .map(|s| s.start_ms);
    Some(credits.unwrap_or(duration_ms - UP_NEXT_LEAD_MS))
}

/// The "Up next" card's state at `ms`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpNext {
    /// Not yet (or dismissed).
    Hidden,
    /// Showing; the next item starts in this many milliseconds.
    Counting(u64),
    /// The countdown ran out: start the next item now.
    Go,
}

/// Follows one item's "Up next" card: it appears at [`up_next_from`], counts down
/// [`COUNTDOWN_MS`] of played time, and can be dismissed ("Watch credits") until the
/// viewer seeks back before it.
#[derive(Clone, Debug)]
pub struct UpNextCard { pub from_ms: Option<u64>, dismissed: bool }

impl UpNextCard {
    pub fn new(segments: &[Segment], duration_ms: u64) -> UpNextCard {
        UpNextCard { from_ms: up_next_from(segments, duration_ms), dismissed: false }
    }
    pub fn dismiss(&mut self) { self.dismissed = true; }
    pub fn at(&mut self, ms: u64) -> UpNext {
        let Some(from) = self.from_ms else { return UpNext::Hidden };
        if ms < from { self.dismissed = false; return UpNext::Hidden; }
        if self.dismissed { return UpNext::Hidden; }
        let into = ms - from;
        if into >= COUNTDOWN_MS { UpNext::Go } else { UpNext::Counting(COUNTDOWN_MS - into) }
    }
}
