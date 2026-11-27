// SPDX-License-Identifier: GPL-3.0-only
//! Suggestions wait for the editor.
//!
//! An assistant (or a rule such as "shorten long pauses") can apply an edit straight to the
//! working timeline, or stage it here. A staged edit is kept as source cuts, not timeline
//! ops, so it still lands on the right words after the editor makes other changes. The
//! editor applies it, skips it, or asks for a revision; nothing is lost either way.
//!
//! Risk tiers (`Safe` vs `NeedsReview`, adapted from Scarlet `#687`/`#705`/`#706`, MIT) let
//! mechanical filler and pause cleanups be accepted in one click (`accept_all_safe`) while
//! semantic assistant or retake proposals wait for human audition.

use crate::transcript::{ripple_ops, SourceCut};
use crate::{EditError, Op, Timeline};
use alloc::string::String;
use alloc::vec::Vec;

/// Who proposed an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Author {
    Editor,
    /// A rule built into Splicy, such as the pause or filler pass.
    Rule,
    /// An assistant, which runs confined and off by default.
    Assistant,
}

/// Review priority tier for a staged suggestion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RiskTier {
    /// Mechanical cleanups (such as rule-based filler or pause passes) safe for 1-click batch keep.
    #[default]
    Safe,
    /// Semantic cuts, retakes, or assistant edits requiring editor review.
    NeedsReview,
}

impl RiskTier {
    /// Default risk tier inferred from who proposed the edit.
    pub const fn for_author(by: Author) -> Self {
        match by {
            Author::Editor | Author::Rule => RiskTier::Safe,
            Author::Assistant => RiskTier::NeedsReview,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending,
    Applied,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub id: u32,
    pub by: Author,
    pub tier: RiskTier,
    /// One sentence the editor reads on the suggestion's card.
    pub summary: String,
    pub cuts: Vec<SourceCut>,
    pub status: Status,
}

#[derive(Clone, Debug, Default)]
pub struct Suggestions {
    pub items: Vec<Suggestion>,
    next: u32,
}

impl Suggestions {
    pub fn stage(&mut self, by: Author, summary: &str, cuts: Vec<SourceCut>) -> u32 {
        self.stage_with_tier(by, RiskTier::for_author(by), summary, cuts)
    }

    pub fn stage_with_tier(
        &mut self,
        by: Author,
        tier: RiskTier,
        summary: &str,
        cuts: Vec<SourceCut>,
    ) -> u32 {
        self.next += 1;
        self.items.push(Suggestion {
            id: self.next,
            by,
            tier,
            summary: summary.into(),
            cuts,
            status: Status::Pending,
        });
        self.next
    }

    pub fn get(&self, id: u32) -> Option<&Suggestion> {
        self.items.iter().find(|s| s.id == id)
    }

    pub fn pending(&self) -> impl Iterator<Item = &Suggestion> {
        self.items.iter().filter(|s| s.status == Status::Pending)
    }

    pub fn safe_pending_count(&self) -> usize {
        self.items
            .iter()
            .filter(|s| s.status == Status::Pending && s.tier == RiskTier::Safe)
            .count()
    }

    pub fn review_pending_count(&self) -> usize {
        self.items
            .iter()
            .filter(|s| s.status == Status::Pending && s.tier == RiskTier::NeedsReview)
            .count()
    }

    /// The ops a suggestion would make on `tl` now, for preview before applying.
    pub fn ops_for(&self, id: u32, tl: &Timeline) -> Option<Vec<Op>> {
        self.get(id).map(|s| ripple_ops(tl, &s.cuts))
    }

    /// Apply a pending suggestion to `tl`, returning the new timeline.
    pub fn apply(&mut self, id: u32, tl: &Timeline) -> Result<Option<Timeline>, EditError> {
        let Some(s) = self.items.iter_mut().find(|s| s.id == id && s.status == Status::Pending) else {
            return Ok(None);
        };
        let next = tl.apply_all(&ripple_ops(tl, &s.cuts))?;
        s.status = Status::Applied;
        Ok(Some(next))
    }

    /// Apply all pending [`RiskTier::Safe`] suggestions atomically ("Keep N safe edits"),
    /// leaving [`RiskTier::NeedsReview`] suggestions pending and returning `(new_timeline, N)`.
    pub fn accept_all_safe(&mut self, tl: &Timeline) -> Result<(Timeline, usize), EditError> {
        let mut next = tl.clone();
        let mut count = 0usize;
        for s in self
            .items
            .iter()
            .filter(|s| s.status == Status::Pending && s.tier == RiskTier::Safe)
        {
            next = next.apply_all(&ripple_ops(&next, &s.cuts))?;
            count += 1;
        }
        if count > 0 {
            for s in self
                .items
                .iter_mut()
                .filter(|s| s.status == Status::Pending && s.tier == RiskTier::Safe)
            {
                s.status = Status::Applied;
            }
        }
        Ok((next, count))
    }

    pub fn skip(&mut self, id: u32) -> bool {
        match self.items.iter_mut().find(|s| s.id == id && s.status == Status::Pending) {
            Some(s) => {
                s.status = Status::Skipped;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MediaKind, Range, TrackKind, SECOND};
    use alloc::vec;

    #[test]
    fn risk_tiers_counts_and_one_click_accept_all_safe() {
        let mut tl = Timeline::new(30, 1);
        let m = tl.add_media(MediaKind::Video, "/clips/talk.mp4", 20 * SECOND);
        let v = tl.add_track(TrackKind::Video, "V1");
        let a = tl.add_track(TrackKind::Audio, "A1");
        tl.apply(&Op::AddClip { track: v, media: m, at: 0, source: Range::new(0, 20 * SECOND) }).unwrap();
        tl.apply(&Op::AddClip { track: a, media: m, at: 0, source: Range::new(0, 20 * SECOND) }).unwrap();

        let mut s = Suggestions::default();
        assert_eq!(RiskTier::default(), RiskTier::Safe);

        // Rule defaults to Safe; Assistant defaults to NeedsReview; stage_with_tier overrides either.
        let s_filler = s.stage(Author::Rule, "Remove filler", vec![SourceCut { media: m, range: Range::new(2 * SECOND, 3 * SECOND) }]);
        let s_retake = s.stage(Author::Assistant, "Remove retake", vec![SourceCut { media: m, range: Range::new(5 * SECOND, 8 * SECOND) }]);
        let s_pause = s.stage_with_tier(Author::Assistant, RiskTier::Safe, "Shorten long silence", vec![SourceCut { media: m, range: Range::new(12 * SECOND, 15 * SECOND) }]);
        let s_risky_rule = s.stage_with_tier(Author::Rule, RiskTier::NeedsReview, "Tight run-on word cut", vec![SourceCut { media: m, range: Range::new(17 * SECOND, 18 * SECOND) }]);

        assert_eq!(s.get(s_filler).unwrap().tier, RiskTier::Safe);
        assert_eq!(s.get(s_retake).unwrap().tier, RiskTier::NeedsReview);
        assert_eq!(s.safe_pending_count(), 2);
        assert_eq!(s.review_pending_count(), 2);

        // 1-click "Keep 2 safe edits": applies s_filler (1s) + s_pause (3s), leaves NeedsReview untouched!
        let (tl2, accepted) = s.accept_all_safe(&tl).unwrap();
        assert_eq!(accepted, 2);
        assert_eq!(tl2.duration(), 16 * SECOND);
        assert_eq!(s.safe_pending_count(), 0);
        assert_eq!(s.review_pending_count(), 2);
        assert_eq!(s.get(s_filler).unwrap().status, Status::Applied);
        assert_eq!(s.get(s_pause).unwrap().status, Status::Applied);
        assert_eq!(s.get(s_retake).unwrap().status, Status::Pending);
        assert_eq!(s.get(s_risky_rule).unwrap().status, Status::Pending);

        // Calling accept_all_safe again is an idempotent no-op.
        let (tl3, again) = s.accept_all_safe(&tl2).unwrap();
        assert_eq!(again, 0);
        assert_eq!(tl3, tl2);
    }
}

