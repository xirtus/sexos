// SPDX-License-Identifier: GPL-3.0-only
//! Actual pointer-free journal and image owner machine; finite crash cuts, not
//! native service/physical persistence proof (that remains a guest gate).
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/desktop_recording.rs"]
pub mod desktop_recording;
#[path = "../servers/sexfiles/src/recording_image.rs"]
mod recording_image;
#[path = "../servers/sexfiles/src/recording_journal.rs"]
mod recording_journal;
use desktop_recording::*;
use recording_image::{Action, OwnerState, Service};
use recording_journal::{Error, Fence, Journal};
fn owner() -> ImageRequest {
    ImageRequest::new(ImageKind::Read, RoleSequence::new(1, 7).unwrap(), 1, 1).unwrap()
}
fn released(mut request: ImageRequest) -> ImageRequest {
    request.kind = ImageKind::Release;
    request
}
fn initialized() -> Box<Journal> {
    let journal = Box::new(Journal::new());
    assert_eq!(journal.load(), Err(Error::Uninitialized));
    journal.initialize_cold().unwrap();
    journal
}
#[test]
fn cold_zero_and_dirty_uncommitted_custody_are_distinct() {
    let journal = initialized();
    assert_eq!(journal.load(), Ok(Fence::Free));
    assert_eq!(journal.initialize_cold(), Err(Error::Ambiguous));
    journal.fault_word(16 + 1, 1); // zero commit with a dirty body is not proof of Free.
    assert_eq!(journal.load(), Err(Error::Ambiguous));
    journal.quarantine(1).unwrap();
    assert_eq!(journal.load(), Ok(Fence::Quarantined(1)));
    let mut service = Service::restore(journal.load().unwrap());
    assert!(service.busy());
    service.observe_quarantine(1, false);
    assert!(service.busy());
    service.observe_quarantine(2, false);
    assert!(!service.busy());
}
#[test]
fn every_acquire_commit_cut_requires_durable_held_before_io_can_begin() {
    for cut in 0..=17 {
        let journal = initialized();
        let result = journal.checkpoint_cut(Fence::Held(owner()), cut);
        let io_began = result.is_ok();
        let restored = journal.load();
        if io_began {
            assert_eq!(restored, Ok(Fence::Held(owner())));
        } else {
            assert!(matches!(restored, Ok(Fence::Free) | Err(Error::Ambiguous)));
        }
        // Ambiguous is quarantined, never used as an empty resource grant.
        let fence = restored.unwrap_or(Fence::Quarantined(1));
        let recovered = Service::restore(fence);
        assert!(!io_began || recovered.busy(), "cut={cut}");
    }
}
#[test]
fn before_or_after_positive_delivery_restart_never_reissues_read_permission() {
    for delivered in [false, true] {
        let journal = initialized();
        let request = owner();
        let mut service = Service::new();
        assert!(matches!(
            service.request(request, OwnerState::Live),
            Action::Read
        ));
        journal.checkpoint(service.fence()).unwrap();
        assert!(service.read_finished(request, BindStatus::Ok, 4096));
        service.poll(OwnerState::Live, |reply| {
            assert_eq!(reply.kind, ImageKind::Read);
            delivered
        });
        let mut restored = Service::restore(journal.load().unwrap());
        restored.poll(OwnerState::Closing, |_| {
            panic!("old Files epoch must not reissue a read permission")
        });
        assert!(restored.busy());
        match restored.request(request, OwnerState::Closing) {
            Action::Refused(reply) => {
                assert_ne!(reply.status, BindStatus::Ok);
                assert_eq!(reply.bytes, 0)
            }
            _ => panic!(),
        }
        assert!(restored.busy());
        assert!(matches!(
            restored.request(released(request), OwnerState::Closing),
            Action::Retained
        ));
        journal.checkpoint(restored.fence()).unwrap();
        restored.poll(OwnerState::Closing, |reply| {
            assert_eq!(reply.kind, ImageKind::Release);
            true
        });
        journal.checkpoint(restored.fence()).unwrap();
        assert_eq!(journal.load(), Ok(Fence::Free));
    }
}
#[test]
fn every_release_commit_cut_preserves_a_resource_obligation_until_exact_handoff() {
    for cut in 0..=17 {
        let journal = initialized();
        journal.checkpoint(Fence::Held(owner())).unwrap();
        let result = journal.checkpoint_cut(Fence::Releasing(owner()), cut);
        let restored = journal.load().unwrap_or(Fence::Quarantined(1));
        assert_ne!(restored, Fence::Free);
        if result.is_ok() {
            assert_eq!(restored, Fence::Releasing(owner()));
        }
    }
}
#[test]
fn release_ack_delivery_then_interrupted_clear_restores_cleanup_only() {
    for cut in 0..=17 {
        let journal = initialized();
        journal.checkpoint(Fence::Held(owner())).unwrap();
        journal.checkpoint(Fence::Releasing(owner())).unwrap();
        // Exact owner already handed off the kernel copy; a delivered cleanup ACK
        // cannot turn into a future positive image read after Files restarts.
        let result = journal.checkpoint_cut(Fence::Free, cut);
        let fence = journal.load().unwrap_or(Fence::Quarantined(1));
        if result.is_ok() {
            assert_eq!(fence, Fence::Free);
        } else {
            assert_ne!(fence, Fence::Free);
        }
        let mut restored = Service::restore(fence);
        restored.poll(OwnerState::Closing, |reply| {
            assert_eq!(reply.kind, ImageKind::Release);
            assert_eq!(reply.bytes, 0);
            false
        });
    }
}
#[test]
fn malformed_committed_or_equal_generation_state_cannot_fall_back_to_free() {
    let journal = initialized();
    journal.checkpoint(Fence::Held(owner())).unwrap();
    journal.fault_word(16 + 2, 0);
    assert_eq!(journal.load(), Err(Error::Ambiguous));
    journal.quarantine(1).unwrap();
    assert_eq!(journal.load(), Ok(Fence::Quarantined(1)));
    let journal = initialized();
    journal.duplicate_slot();
    assert_eq!(journal.load(), Err(Error::Ambiguous));
    let journal = initialized();
    journal.checkpoint(Fence::Held(owner())).unwrap();
    journal.fault_word(16, 0);
    assert_eq!(
        journal.load(),
        Err(Error::Ambiguous),
        "a zeroed newer commit with retained payload cannot expose old Free"
    );
}
#[test]
fn generation_exhaustion_does_not_wrap_or_release_held_custody() {
    let journal = Box::new(Journal::new());
    journal
        .seed_record(0, u64::MAX, Fence::Held(owner()))
        .unwrap();
    for _ in 0..4 {
        assert_eq!(journal.checkpoint(Fence::Free), Err(Error::Exhausted));
        assert_eq!(journal.quarantine(1), Err(Error::Exhausted));
        assert_eq!(journal.load(), Ok(Fence::Held(owner())));
    }
}
