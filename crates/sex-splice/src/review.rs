// SPDX-License-Identifier: GPL-3.0-only
//! Evidence-not-vetoes join auditing and unified priority review ordering.
//!
//! Adapted from Scarlet's `fix/evidence-not-vetoes` and review queue `#687`/`#705`/`#706`/`#723`/
//! `#726` (MIT, github.com/Forward-Future/scarlet).
//!
//! An assistant working from a transcript cannot hear raw PCM audio, so a cut edge that slices
//! straight through active mid-word speech is refused (`AuditOutcome::RefuseAcousticClip`). By
//! contrast, lexical or pacing observations that an editor or assistant can weigh deliberately —
//! a doubled word across the join, a leading or interior word dropped by a restart, or a tight
//! breath gap — never veto the edit; they return `AuditOutcome::ApplyWithEvidence` so the edit
//! applies as decided alongside structured evidence.

use crate::edges::{Levels, QUIET_DECIBELS, RUN_ON_GAP, TROUGH_DEPTH};
use crate::rebase::source_to_timeline_spans;
use crate::restarts::restart_key;
use crate::suggest::{RiskTier, Suggestion, Suggestions};
use crate::transcript::{norm, SourceCut, Transcript};
use crate::{Micros, Timeline};
use alloc::string::String;
use alloc::vec::Vec;

/// Structured join observation surfaced alongside an applied edit rather than vetoing it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinFinding {
    /// Kept words immediately before and after the cut normalize to the same spoken word.
    DoubledWord(String),
    /// A word from the removed first attempt was omitted when the speaker restarted.
    DroppedRestartWord(String),
    /// Kept speech on either side of the cut meets with less than [`RUN_ON_GAP`] (`60 ms`) of air.
    TightSpeechEdge { gap_us: Micros },
}

/// Outcome of auditing a proposed [`SourceCut`] against transcript timings and optional [`Levels`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuditOutcome {
    /// Cut edge lands inside active speech on the waveform (`db` in tenths of dBFS); refused
    /// because a transcript-driven pass cannot hear raw PCM mid-word clipping.
    RefuseAcousticClip { at_source: Micros, db: i16 },
    /// Join is acoustically safe to apply; carries any lexical or pacing findings as evidence.
    ApplyWithEvidence(Vec<JoinFinding>),
}

impl AuditOutcome {
    pub fn should_apply(&self) -> bool {
        matches!(self, AuditOutcome::ApplyWithEvidence(_))
    }

    pub fn is_refused(&self) -> bool {
        matches!(self, AuditOutcome::RefuseAcousticClip { .. })
    }

    pub fn findings(&self) -> &[JoinFinding] {
        match self {
            AuditOutcome::RefuseAcousticClip { .. } => &[],
            AuditOutcome::ApplyWithEvidence(f) => f.as_slice(),
        }
    }
}

/// Whether `edge_us` slices inside a word's timing or between two run-on words (`< RUN_ON_GAP`).
fn is_tight_or_midword_transcript_point(transcript: &Transcript, edge_us: Micros) -> bool {
    if transcript
        .words
        .iter()
        .any(|w| w.source.start < edge_us && edge_us < w.source.end)
    {
        return true;
    }
    transcript.words.windows(2).any(|pair| {
        let (l, r) = (&pair[0], &pair[1]);
        r.source.start - l.source.end < RUN_ON_GAP
            && edge_us >= l.source.start
            && edge_us <= r.source.end
    })
}

/// Check whether `edge_us` clips active speech in `levels`, returning `Some(db)` when unsafe.
fn check_acoustic_edge(
    transcript: &Transcript,
    levels: &Levels,
    edge_us: Micros,
) -> Option<i16> {
    if levels.step <= 0 || levels.db.is_empty() || edge_us < levels.from {
        return None;
    }
    let idx = ((edge_us - levels.from) / levels.step) as usize;
    if idx >= levels.db.len() {
        return None;
    }

    let bars = levels.bars();
    let db = levels.db[idx];

    // Room tone or a genuine waveform trough (>= 10 dB below surrounding speech) is always clean.
    if db <= bars.room_tone || (db <= bars.low_point && levels.trough_depth(idx) >= TROUGH_DEPTH) {
        return None;
    }

    // Active speaking level at the razor, or non-room-tone sound inside a word / run-on boundary.
    let speech_floor = bars.low_point.max(QUIET_DECIBELS);
    if db > speech_floor
        || (transcript.media != 0 && is_tight_or_midword_transcript_point(transcript, edge_us))
    {
        return Some(db);
    }

    None
}

/// Detect whether `removed` starts with or contains a word dropped by the restart at `after`.
fn detect_dropped_restart_word(
    removed: &[crate::transcript::Word],
    after: &[crate::transcript::Word],
) -> Option<String> {
    if removed.len() < 2 || after.is_empty() {
        return None;
    }

    let r_keys: Vec<String> = removed.iter().map(|w| restart_key(&w.text)).collect();
    let a_keys: Vec<String> = after
        .iter()
        .take(removed.len() + 2)
        .map(|w| restart_key(&w.text))
        .collect();

    // Pattern 1: 1–2 leading connector/starter words in the removed attempt ("So I think..." ->
    // "I think...") that are omitted when the speaker restarts.
    if !r_keys[0].is_empty() && r_keys[0] != a_keys[0] {
        let max_drop = 2.min(removed.len() - 1);
        for drop_len in 1..=max_drop {
            let tail = &r_keys[drop_len..];
            let mut matched = 0usize;
            while matched < tail.len()
                && matched < a_keys.len()
                && !tail[matched].is_empty()
                && tail[matched] == a_keys[matched]
            {
                matched += 1;
            }
            if matched >= 2 || (tail.len() == 1 && matched == 1) {
                let dropped: Vec<String> = removed[..drop_len]
                    .iter()
                    .map(|w| norm(&w.text))
                    .filter(|s| !s.is_empty())
                    .collect();
                if !dropped.is_empty() {
                    return Some(dropped.join(" "));
                }
            }
        }
    }

    // Pattern 2: Interior word present in the first attempt and omitted in the retake
    // ("how a mechanical watch" -> "how mechanical watch").
    if removed.len() >= 3 && a_keys.len() >= removed.len() - 1 && !r_keys[0].is_empty() {
        let mut prefix = 0usize;
        while prefix < r_keys.len()
            && prefix < a_keys.len()
            && !r_keys[prefix].is_empty()
            && r_keys[prefix] == a_keys[prefix]
        {
            prefix += 1;
        }
        if prefix >= 1 && prefix + 1 < r_keys.len() {
            let rem_rest = &r_keys[prefix + 1..];
            if a_keys.len() >= prefix + rem_rest.len()
                && rem_rest
                    .iter()
                    .enumerate()
                    .all(|(k, key)| !key.is_empty() && *key == a_keys[prefix + k])
            {
                let word = norm(&removed[prefix].text);
                if !word.is_empty() {
                    return Some(word);
                }
            }
        }
    }

    None
}

/// Audit a proposed [`SourceCut`] using the evidence-not-vetoes policy:
/// - Refuses acoustic mid-word speech cuts (`AuditOutcome::RefuseAcousticClip`) when `levels`
///   shows active speech across a played cut boundary.
/// - Otherwise approves (`AuditOutcome::ApplyWithEvidence`), reporting any `DoubledWord`,
///   `DroppedRestartWord`, or `TightSpeechEdge` observations as structured evidence.
pub fn audit_cut_join(
    transcript: &Transcript,
    cut: SourceCut,
    levels: Option<&Levels>,
) -> AuditOutcome {
    if cut.range.is_empty() {
        return AuditOutcome::ApplyWithEvidence(Vec::new());
    }

    if let Some(lv) = levels {
        let levels_end = lv.from + (lv.db.len() as Micros) * lv.step.max(0);
        let has_kept_before = cut.range.start > lv.from.max(0)
            || (cut.media == transcript.media
                && transcript.words.iter().any(|w| w.source.start < cut.range.start));
        let has_kept_after = cut.range.end < levels_end
            || (cut.media == transcript.media
                && transcript.words.iter().any(|w| w.source.end > cut.range.end));

        if has_kept_before {
            if let Some(db) = check_acoustic_edge(transcript, lv, cut.range.start) {
                return AuditOutcome::RefuseAcousticClip {
                    at_source: cut.range.start,
                    db,
                };
            }
        }
        if has_kept_after {
            if let Some(db) = check_acoustic_edge(transcript, lv, cut.range.end) {
                return AuditOutcome::RefuseAcousticClip {
                    at_source: cut.range.end,
                    db,
                };
            }
        }
    }

    if cut.media != transcript.media || transcript.words.is_empty() {
        return AuditOutcome::ApplyWithEvidence(Vec::new());
    }

    let mut last_before: Option<usize> = None;
    let mut removed_first: Option<usize> = None;
    let mut removed_last: Option<usize> = None;
    let mut first_after: Option<usize> = None;

    for (i, w) in transcript.words.iter().enumerate() {
        let mid = w.source.start + (w.source.end - w.source.start) / 2;
        if mid < cut.range.start {
            last_before = Some(i);
        } else if mid < cut.range.end {
            if removed_first.is_none() {
                removed_first = Some(i);
            }
            removed_last = Some(i);
        } else if first_after.is_none() {
            first_after = Some(i);
        }
    }

    let mut findings = Vec::new();

    if let (Some(bi), Some(ai)) = (last_before, first_after) {
        let left_plain = norm(&transcript.words[bi].text).replace('\'', "");
        let right_plain = norm(&transcript.words[ai].text).replace('\'', "");
        if !left_plain.is_empty() && left_plain == right_plain {
            findings.push(JoinFinding::DoubledWord(norm(&transcript.words[ai].text)));
        }
    }

    if let (Some(rf), Some(rl), Some(ai)) = (removed_first, removed_last, first_after) {
        let removed_slice = &transcript.words[rf..=rl];
        let after_slice = &transcript.words[ai..];
        if let Some(dropped) = detect_dropped_restart_word(removed_slice, after_slice) {
            findings.push(JoinFinding::DroppedRestartWord(dropped));
        }
    }

    if let (Some(bi), Some(ai)) = (last_before, first_after) {
        let air_before = (cut.range.start - transcript.words[bi].source.end).max(0);
        let air_after = (transcript.words[ai].source.start - cut.range.end).max(0);
        let gap_us = air_before + air_after;
        if gap_us < RUN_ON_GAP {
            findings.push(JoinFinding::TightSpeechEdge { gap_us });
        }
    }

    AuditOutcome::ApplyWithEvidence(findings)
}

fn suggestion_timeline_at(s: &Suggestion, tl: &Timeline) -> Micros {
    s.cuts
        .iter()
        .filter_map(|cut| {
            source_to_timeline_spans(tl, cut.media, cut.range)
                .first()
                .map(|r| r.start)
                .or_else(|| tl.place_of(cut.media, cut.range.start))
        })
        .min()
        .unwrap_or(Micros::MAX)
}

/// Return pending suggestion IDs in unified review priority order:
/// all [`RiskTier::NeedsReview`] suggestions ordered by current timeline position first,
/// followed by all [`RiskTier::Safe`] suggestions ordered by current timeline position.
pub fn priority_review_order(suggestions: &Suggestions, tl: &Timeline) -> Vec<u32> {
    let mut keyed: Vec<(u8, Micros, u32)> = suggestions
        .pending()
        .map(|s| {
            let tier_rank = match s.tier {
                RiskTier::NeedsReview => 0,
                RiskTier::Safe => 1,
            };
            (tier_rank, suggestion_timeline_at(s, tl), s.id)
        })
        .collect();

    keyed.sort_by_key(|&(rank, at, id)| (rank, at, id));
    keyed.into_iter().map(|(_, _, id)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suggest::Author;
    use crate::transcript::Word;
    use crate::{MediaKind, Op, Range, TrackKind, SECOND};
    use alloc::vec;

    const STEP: Micros = 20_000;
    const MS: Micros = 1_000;

    fn levels(stretches: &[(Micros, i16)]) -> Levels {
        let mut db = Vec::new();
        for &(ms, level) in stretches {
            db.extend(core::iter::repeat(level).take((ms * 1000 / STEP) as usize));
        }
        Levels { from: 0, step: STEP, db }
    }

    fn word(text: &str, start_ms: Micros, end_ms: Micros) -> Word {
        Word {
            text: String::from(text),
            source: Range::new(start_ms * MS, end_ms * MS),
        }
    }

    #[test]
    fn refuses_acoustic_mid_word_speech_clip_when_razor_hits_voice() {
        // "welcome" 100..500ms loud (-20 dBFS), "um" 600..800ms, "everyone" 900..1400ms loud (-18 dBFS).
        let lv = levels(&[
            (100, -650),
            (400, -200),
            (100, -650),
            (200, -240),
            (100, -650),
            (500, -180),
            (200, -650),
        ]);
        let tr = Transcript {
            media: 1,
            words: vec![
                word("welcome", 100, 500),
                word("um", 600, 800),
                word("everyone", 900, 1400),
            ],
        };

        // Bad start edge cutting straight through "welcome" at 300ms (-200 tenths of dBFS).
        let bad_start = SourceCut { media: 1, range: Range::new(300 * MS, 850 * MS) };
        let out1 = audit_cut_join(&tr, bad_start, Some(&lv));
        assert!(out1.is_refused());
        assert_eq!(out1, AuditOutcome::RefuseAcousticClip { at_source: 300 * MS, db: -200 });

        // Bad end edge landing inside "everyone" at 1000ms (-180 tenths of dBFS).
        let bad_end = SourceCut { media: 1, range: Range::new(550 * MS, 1_000 * MS) };
        assert_eq!(
            audit_cut_join(&tr, bad_end, Some(&lv)),
            AuditOutcome::RefuseAcousticClip { at_source: 1_000 * MS, db: -180 }
        );

        // Clean placed cut in room tone (550ms..850ms => 50ms + 50ms = 100ms air) passes cleanly!
        let clean = SourceCut { media: 1, range: Range::new(550 * MS, 850 * MS) };
        let out_clean = audit_cut_join(&tr, clean, Some(&lv));
        assert!(out_clean.should_apply());
        assert_eq!(out_clean, AuditOutcome::ApplyWithEvidence(vec![]));
    }

    #[test]
    fn lexical_doubled_word_dropped_restart_and_tight_edge_apply_with_evidence() {
        // 1. Cutting only "um" inside "went to the, um, the store" leaves "the" doubled across join.
        let tr_double = Transcript {
            media: 1,
            words: vec![
                word("to", 0, 200),
                word("the,", 250, 450),
                word("um,", 600, 800),
                word("the", 950, 1150),
                word("store", 1200, 1600),
            ],
        };
        let cut_um = tr_double.cut_words(2, 2).unwrap();
        assert_eq!(
            audit_cut_join(&tr_double, cut_um, None),
            AuditOutcome::ApplyWithEvidence(vec![JoinFinding::DoubledWord(String::from("the"))])
        );

        // 2a. Leading starter word dropped before restart: "So I think that, I think that works"
        let tr_drop_lead = Transcript {
            media: 1,
            words: vec![
                word("Okay.", 0, 300),
                word("So", 500, 650),
                word("I", 700, 800),
                word("think", 850, 1100),
                word("that,", 1150, 1400),
                word("I", 1600, 1700),
                word("think", 1750, 2000),
                word("that", 2050, 2250),
                word("works.", 2300, 2700),
            ],
        };
        let cut_restart = tr_drop_lead.cut_words(1, 4).unwrap();
        assert_eq!(
            audit_cut_join(&tr_drop_lead, cut_restart, None),
            AuditOutcome::ApplyWithEvidence(vec![JoinFinding::DroppedRestartWord(String::from("so"))])
        );

        // 2b. Interior word dropped in retake: "how a mechanical watch, how mechanical watch"
        //     with a tight 20ms gap from the preceding word!
        let tr_drop_mid = Transcript {
            media: 1,
            words: vec![
                word("see", 0, 200),
                word("how", 220, 400),
                word("a", 420, 500),
                word("mechanical", 520, 900),
                word("watch,", 920, 1200),
                word("how", 1400, 1600),
                word("mechanical", 1620, 2000),
                word("watch", 2020, 2300),
            ],
        };
        let cut_mid = tr_drop_mid.cut_words(1, 4).unwrap();
        assert_eq!(
            audit_cut_join(&tr_drop_mid, cut_mid, None),
            AuditOutcome::ApplyWithEvidence(vec![
                JoinFinding::DroppedRestartWord(String::from("a")),
                JoinFinding::TightSpeechEdge { gap_us: 20 * MS },
            ])
        );
    }

    #[test]
    fn tight_speech_edge_on_acoustic_dip_applies_with_evidence() {
        // Run-on words with a real 40ms room-tone dip between them (20ms air on each side = 40ms < 60ms).
        let lv = levels(&[(200, -200), (40, -620), (200, -220), (40, -620), (200, -200)]);
        let tr = Transcript {
            media: 1,
            words: vec![
                word("fast", 0, 200),
                word("uh", 240, 440),
                word("speech", 480, 680),
            ],
        };
        let placed = tr.placed_cut(1, 1, &lv).unwrap();
        let outcome = audit_cut_join(&tr, placed.cut, Some(&lv));
        assert!(outcome.should_apply(), "{outcome:?}");
        assert_eq!(
            outcome.findings(),
            &[JoinFinding::TightSpeechEdge { gap_us: 40 * MS }]
        );
    }

    #[test]
    fn priority_review_order_puts_needs_review_first_by_timeline_time_then_safe() {
        let mut tl = Timeline::new(30, 1);
        let m = tl.add_media(MediaKind::Video, "/clips/ep.mp4", 30 * SECOND);
        let v = tl.add_track(TrackKind::Video, "V1");
        tl.apply(&Op::AddClip { track: v, media: m, at: 0, source: Range::new(0, 30 * SECOND) }).unwrap();

        // Ripple delete 0..4s first to verify ordering uses live timeline positions.
        tl.apply(&Op::RippleDelete { range: Range::new(0, 4 * SECOND) }).unwrap();

        let mut s = Suggestions::default();
        let safe_late = s.stage(Author::Rule, "Late pause", vec![SourceCut { media: m, range: Range::new(22 * SECOND, 24 * SECOND) }]);
        let rev_late = s.stage(Author::Assistant, "Late retake", vec![SourceCut { media: m, range: Range::new(18 * SECOND, 20 * SECOND) }]);
        let safe_early = s.stage(Author::Rule, "Early um", vec![SourceCut { media: m, range: Range::new(5 * SECOND, 6 * SECOND) }]);
        let rev_early = s.stage(Author::Assistant, "Early retake", vec![SourceCut { media: m, range: Range::new(9 * SECOND, 11 * SECOND) }]);
        let skipped = s.stage(Author::Assistant, "Skipped item", vec![SourceCut { media: m, range: Range::new(4 * SECOND, 5 * SECOND) }]);
        assert!(s.skip(skipped));

        assert_eq!(
            priority_review_order(&s, &tl),
            vec![rev_early, rev_late, safe_early, safe_late]
        );
    }

    #[test]
    fn noisy_room_and_leading_slate_acoustic_audit() {
        // 2s recording in a noisy room (-44 dBFS ambient, -26 dBFS speech):
        // cutting inside ambient room tone at 750ms passes, while mid-speech at 350ms refuses!
        let noisy = levels(&[(700, -260), (100, -440), (700, -260), (500, -440)]);
        let tr = Transcript {
            media: 1,
            words: vec![
                word("slate", 0, 680),
                word("real", 820, 1500),
            ],
        };

        // Trimming leading slate 0..750ms (starts at 0 so nothing kept before, ends in noisy room tone) passes!
        let trim_slate = SourceCut { media: 1, range: Range::new(0, 750 * MS) };
        assert!(audit_cut_join(&tr, trim_slate, Some(&noisy)).should_apply());

        // Slicing mid-speech at 350ms..750ms is refused even in a noisy room.
        let mid_slate = SourceCut { media: 1, range: Range::new(350 * MS, 750 * MS) };
        assert_eq!(
            audit_cut_join(&tr, mid_slate, Some(&noisy)),
            AuditOutcome::RefuseAcousticClip { at_source: 350 * MS, db: -260 }
        );
    }
}
