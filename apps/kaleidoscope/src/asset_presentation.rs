// SPDX-License-Identifier: GPL-3.0-only
//! Current image-presentation correlation, not display or source authority.
//! The owning outbox retains accepted COMMIT/CLEAR custody independently. A
//! generation change forgets current readiness without replaying that mutation.
use crate::{asset_view::Mode, bitmap_delivery::Receipt};
use sex_pdx::bitmap_batch::Geometry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub generation: u64,
    pub geometry: Geometry,
    pub viewport: u64,
    pub ticket: u32,
    pub sequence: u32,
    pub hash: u64,
}

#[derive(Clone, Copy)]
struct Clear { generation:u64, viewport:u64, sequence:u32 }

pub struct Presentation {
    mode: Mode,
    generation: u64,
    text: Option<(u64, u64)>,
    layout: Option<Layout>,
    applied: bool,
    clear: Option<Clear>,
    clear_applied: bool,
    // Outbox sequences remain monotonic across mode/source/viewport changes.
    // Never bind its coalesced historical sequence to a fresh generation.
    sequence_floor: u32,
    exhausted: bool,
}
impl Presentation {
    pub const fn new() -> Self {
        Self { mode: Mode::Fit, generation: 1, text: None, layout: None,
            applied: false, clear: None, clear_applied: false, sequence_floor: 0, exhausted: false }
    }
    pub fn mode(&self) -> Mode { self.mode }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn layout(&self) -> Option<Layout> { self.layout }
    pub fn has_layout(&self) -> bool { self.layout.is_some() || self.clear.is_some() }
    pub fn ready(&self) -> bool { self.applied && !self.exhausted }

    /// Invalidates readiness only. It never changes the delivery outbox, whose
    /// accepted historical receipt must still be reconciled by its own owner.
    pub fn invalidate(&mut self) -> Result<(), ()> {
        self.text = None;
        self.layout = None;
        self.applied = false;
        self.clear = None;
        self.clear_applied = false;
        let Some(next) = self.generation.checked_add(1) else {
            self.exhausted = true;
            return Err(());
        };
        if self.exhausted { return Err(()); }
        self.generation = next;
        Ok(())
    }
    pub fn select(&mut self, mode: Mode) -> Result<bool, ()> {
        if self.mode == mode { return Ok(false); }
        self.invalidate()?;
        self.mode = mode;
        Ok(true)
    }
    /// Called only for an already authenticated positive text Applied receipt.
    /// Its metadata carries this presentation generation and viewport.
    pub fn text_applied(&mut self, generation: u64, viewport: u64, current_viewport: u64) -> bool {
        if self.exhausted || generation != self.generation || viewport == 0
            || viewport != current_viewport || self.text == Some((generation, viewport)) {
            return false;
        }
        self.text = Some((generation, viewport));
        true
    }
    pub fn can_queue(&self, viewport: u64) -> bool {
        !self.exhausted && viewport != 0 && !self.has_layout()
            && self.text == Some((self.generation, viewport))
    }
    /// Stores only a description returned by successful outbox queueing. A
    /// false result changes no text/layout/sequence state and grants no retry.
    pub fn expect(&mut self, geometry: Geometry, viewport: u64, ticket: u32,
        sequence: u32, hash: u64) -> bool {
        if !self.can_queue(viewport) || !geometry.valid() || ticket == 0
            || sequence <= self.sequence_floor {
            return false;
        }
        self.layout = Some(Layout { generation: self.generation, geometry,
            viewport, ticket, sequence, hash });
        self.sequence_floor = sequence;
        self.applied = false;
        true
    }
    /// A no-fit erasure is a presentation barrier too. Remember it until the
    /// next generation, so a drained clear is never queued again automatically.
    pub fn expect_clear(&mut self, viewport:u64, sequence:u32)->bool {
        if !self.can_queue(viewport) || sequence<=self.sequence_floor { return false; }
        self.clear=Some(Clear { generation:self.generation, viewport, sequence });
        self.sequence_floor=sequence;
        self.clear_applied=false;
        true
    }
    pub fn cleared(&mut self, receipt:&Receipt, current_viewport:u64)->bool {
        let Some(clear)=self.clear else { return false; };
        if self.exhausted || self.clear_applied || clear.generation!=self.generation
            || clear.viewport!=current_viewport || receipt.sequence!=clear.sequence
            || receipt.geometry.is_some() || receipt.hash!=0
            || receipt.metadata.viewport!=clear.viewport || receipt.metadata.source!=0 {
            return false;
        }
        self.clear_applied=true;
        true
    }
    /// Receipt authority has already been checked by the owning outbox. This
    /// join establishes current readiness once; it does not prove pixels.
    pub fn applied(&mut self, receipt: &Receipt, current_viewport: u64,
        current_ticket: u32, current_hash: u64) -> bool {
        let Some(layout) = self.layout else { return false; };
        if self.exhausted || self.applied || layout.generation != self.generation
            || layout.viewport != current_viewport || layout.ticket != current_ticket
            || layout.hash != current_hash || receipt.sequence != layout.sequence
            || receipt.geometry != Some(layout.geometry) || receipt.hash != layout.hash
            || receipt.metadata.viewport != layout.viewport
            || receipt.metadata.source != layout.ticket as u64 {
            return false;
        }
        self.applied = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maximum_outbox_sequence_cannot_be_rebound_after_invalidation() {
        let mut p=Presentation::new();let geometry=Geometry{x:8,y:80,width:1,height:1,scale:1};
        assert!(p.text_applied(1,1,1));assert!(p.expect(geometry,1,7,u32::MAX,0));
        p.invalidate().unwrap();assert!(p.text_applied(2,1,1));
        assert!(!p.expect(geometry,1,7,u32::MAX,0));assert!(!p.expect_clear(1,u32::MAX));
        assert!(!p.has_layout());assert!(p.can_queue(1));
    }
    #[test]
    fn generation_exhaustion_erases_current_readiness_and_cannot_rebind_old_receipts() {
        let mut p = Presentation::new();p.generation = u64::MAX;
        assert!(p.text_applied(u64::MAX, 1, 1));
        let geometry = Geometry { x: 8, y: 80, width: 1, height: 1, scale: 1 };
        assert!(p.expect(geometry, 1, 7, 1, 0));
        assert_eq!(p.select(Mode::Actual), Err(()));
        assert_eq!(p.mode(), Mode::Fit);assert_eq!(p.generation(), u64::MAX);
        assert!(!p.has_layout());assert!(!p.ready());assert!(!p.can_queue(1));
        assert!(!p.text_applied(u64::MAX, 1, 1));assert_eq!(p.invalidate(), Err(()));
    }
}
