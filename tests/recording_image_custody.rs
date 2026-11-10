// SPDX-License-Identifier: GPL-3.0-only
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/desktop_recording.rs"]
pub mod desktop_recording;
#[path = "../servers/sexfiles/src/recording_image.rs"]
mod production;
#[path = "../servers/sexfiles/src/recording_journal.rs"]
mod recording_journal;
use desktop_recording::*;
use production::*;
fn read(epoch: u64, count: u32) -> ImageRequest {
    ImageRequest::new(
        ImageKind::Read,
        RoleSequence::new(epoch, count).unwrap(),
        epoch,
        3,
    )
    .unwrap()
}
fn release(mut request: ImageRequest) -> ImageRequest {
    request.kind = ImageKind::Release;
    request
}
fn loaded() -> (Service, ImageRequest) {
    let mut state = Service::new();
    let request = read(2, 1);
    assert!(matches!(
        state.request(request, OwnerState::Live),
        Action::Read
    ));
    assert!(state.read_finished(request, BindStatus::Ok, 4096));
    (state, request)
}
#[test]
fn retained_read_and_release_keep_the_image_exclusive_through_reply_backpressure() {
    let (mut state, request) = loaded();
    for _ in 0..4 {
        state.poll(OwnerState::Live, |reply| {
            assert_eq!(reply.kind, ImageKind::Read);
            assert_eq!(reply.bytes, 4096);
            false
        });
        assert!(state.busy());
        assert!(state.pending());
    }
    state.poll(OwnerState::Unknown, |_| {
        panic!("unknown scope cannot deliver")
    });
    state.poll(OwnerState::Live, |_| true);
    assert!(state.busy());
    assert!(!state.pending());
    assert!(matches!(
        state.request(release(request), OwnerState::Live),
        Action::Retained
    ));
    state.poll(OwnerState::Live, |reply| {
        assert_eq!(reply.kind, ImageKind::Release);
        assert_eq!(reply.bytes, 0);
        false
    });
    assert!(state.busy());
    state.poll(OwnerState::Live, |_| true);
    assert!(!state.busy());
    assert!(!state.pending());
}
#[test]
fn concurrent_or_wrong_transaction_cannot_replace_the_owned_image() {
    let (mut state, owner) = loaded();
    for request in [
        read(2, 2),
        release(read(2, 2)),
        release(read(3, 1)),
        ImageRequest {
            files_epoch: 4,
            ..release(owner)
        },
    ] {
        assert!(matches!(
            state.request(request, OwnerState::Live),
            Action::Refused(_)
        ));
        assert_eq!(state.owner(), Some(owner));
    }
    state.poll(OwnerState::Live, |reply| {
        assert_eq!(reply.sequence, owner.sequence);
        assert_eq!(reply.bytes, 4096);
        true
    });
    assert!(state.busy());
}
#[test]
fn closing_is_not_quiescence_after_a_positive_read_was_delivered() {
    let (mut state, _) = loaded();
    state.poll(OwnerState::Live, |_| true);
    state.poll(OwnerState::Closing, |_| panic!());
    assert!(state.busy());
    state.poll(OwnerState::Unknown, |_| panic!());
    assert!(state.busy());
    state.poll(OwnerState::Quiescent, |_| {
        panic!("old epoch receipt cannot escape")
    });
    assert!(!state.busy());
    assert!(matches!(
        state.request(read(3, 1), OwnerState::Live),
        Action::Read
    ));
}
#[test]
fn closure_before_read_delivery_revokes_its_positive_even_after_later_live_snapshot() {
    let (mut state, _) = loaded();
    state.poll(OwnerState::Closing, |reply| {
        assert_eq!(reply.status, BindStatus::Denied);
        false
    });
    assert!(state.busy());
    state.poll(OwnerState::Live, |reply| {
        assert_eq!(reply.status, BindStatus::Denied);
        true
    });
    assert!(!state.busy());
    let mut state = Service::new();
    let request = read(2, 1);
    assert!(matches!(
        state.request(request, OwnerState::Live),
        Action::Read
    ));
    state.poll(OwnerState::Closing, |_| panic!());
    assert!(state.read_finished(request, BindStatus::Ok, 4096));
    state.poll(OwnerState::Live, |reply| {
        assert_eq!(reply.status, BindStatus::Denied);
        true
    });
    assert!(!state.busy());
}
#[test]
fn completion_and_release_cannot_advance_a_different_or_unfinished_read() {
    let mut state = Service::new();
    let request = read(2, 1);
    assert!(matches!(
        state.request(request, OwnerState::Live),
        Action::Read
    ));
    assert!(!state.read_finished(read(2, 2), BindStatus::Ok, 4096));
    assert!(!state.read_finished(request, BindStatus::Ok, MAX_IMAGE_BYTES + 1));
    assert!(matches!(
        state.request(release(request), OwnerState::Live),
        Action::Refused(_)
    ));
    assert!(state.busy());
    assert!(state.read_finished(request, BindStatus::Io, 0));
    state.poll(OwnerState::Live, |reply| {
        assert_eq!(reply.status, BindStatus::Io);
        true
    });
    assert!(!state.busy());
    for owner in [
        OwnerState::Unknown,
        OwnerState::Closing,
        OwnerState::Quiescent,
    ] {
        assert!(matches!(state.request(request, owner), Action::Refused(_)));
        assert!(!state.busy());
    }
}
#[test]
fn exact_cleanup_release_is_allowed_while_closing_without_new_read_authority() {
    let (mut state, request) = loaded();
    state.poll(OwnerState::Live, |_| true);
    assert!(matches!(
        state.request(release(request), OwnerState::Unknown),
        Action::Retained
    ));
    assert!(state.busy());
    state.poll(OwnerState::Unknown, |_| panic!());
    state.poll(OwnerState::Closing, |reply| {
        assert_eq!(reply.kind, ImageKind::Release);
        true
    });
    assert!(!state.busy());
    assert!(matches!(
        state.request(read(2, 2), OwnerState::Closing),
        Action::Refused(_)
    ));
}
