// SPDX-License-Identifier: GPL-3.0-only
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/desktop_recording.rs"]
pub mod desktop_recording;
#[path = "../servers/sexfiles/src/recording_role.rs"]
mod recording_role;
use desktop_recording::*;
use recording_role::*;
fn request(epoch: u64, count: u32) -> BindRequest {
    BindRequest::new(RoleSequence::new(epoch, count).unwrap(), 107, epoch, 3).unwrap()
}
#[test]
fn provider_backpressure_preserves_exact_receipt_until_authenticated_delivery() {
    let mut role = Role::new();
    let req = request(2, 1);
    assert!(role.retain(req, BindStatus::Ok));
    assert!(role.repeated(req));
    assert!(role.busy(request(2, 2)));
    assert!(!role.retain(request(2, 2), BindStatus::Ok));
    for _ in 0..4 {
        role.poll(
            |_| Check::Live,
            |reply| {
                assert_eq!(reply, BindReply::new(req, BindStatus::Ok));
                false
            },
        );
        assert!(role.pending());
        assert_eq!(role.bound(), None);
    }
    role.poll(
        |_| Check::Wait,
        |_| panic!("unknown lifecycle cannot deliver"),
    );
    assert!(role.pending());
    role.poll(|_| Check::Live, |_| true);
    assert!(!role.pending());
    assert_eq!(role.bound(), Some(Scope::request(req)));
}
#[test]
fn old_shell_or_files_lifecycle_retires_undelivered_positive_without_emission() {
    for invalid_field in 0..3 {
        let mut role = Role::new();
        let req = request(2, 1);
        role.retain(req, BindStatus::Ok);
        role.poll(|_| Check::Live, |_| false);
        let actual = Scope {
            task: if invalid_field == 0 { 108 } else { 107 },
            shell_epoch: if invalid_field == 1 { 3 } else { 2 },
            files_epoch: if invalid_field == 2 { 4 } else { 3 },
        };
        role.poll(
            |scope| {
                if scope == actual {
                    Check::Live
                } else {
                    Check::Invalid
                }
            },
            |_| panic!("stale positive escaped"),
        );
        assert!(!role.pending());
        assert_eq!(role.bound(), None);
    }
}
#[test]
fn installed_binding_is_revoked_by_conclusive_retirement_but_not_contention() {
    let mut role = Role::new();
    role.retain(request(2, 1), BindStatus::Ok);
    role.poll(|_| Check::Live, |_| true);
    role.reconcile(|_| Check::Wait);
    assert!(role.bound().is_some());
    role.reconcile(|_| Check::Invalid);
    assert_eq!(role.bound(), None);
}

#[test]
fn file_write_receipt_retains_the_initiating_scope_until_delivery_or_retirement() {
    let scope = Scope::request(request(2, 1));
    let mut reply = ScopedReply::new();
    assert!(reply.retain(scope, 0x1234));
    assert!(!reply.retain(Scope { task: 108, ..scope }, 0x5678));
    reply.poll(
        |got| {
            assert_eq!(got, scope);
            Check::Live
        },
        |_| panic!("full live scope includes recipient"),
        |_| panic!("no cancellation"),
        |word| {
            assert_eq!(word, 0x1234);
            false
        },
    );
    assert!(reply.pending());
    reply.poll(
        |_| Check::Wait,
        |_| panic!(),
        |_| panic!(),
        |_| panic!("contention cannot authorize delivery"),
    );
    assert!(reply.pending());
    reply.poll(
        |_| Check::Invalid,
        |_| Check::Invalid,
        |_| 0xdead,
        |_| panic!("retired task cannot receive"),
    );
    assert!(!reply.pending());
    let next = Scope { task: 108, ..scope };
    assert!(reply.retain(next, 0x5678));
    reply.poll(
        |got| {
            assert_eq!(got, next);
            Check::Live
        },
        |_| panic!(),
        |_| panic!(),
        |word| {
            assert_eq!(word, 0x5678);
            true
        },
    );
    assert!(!reply.pending());
}
#[test]
fn shell_retirement_returns_a_negative_fence_to_the_same_live_task_only() {
    let scope = Scope::request(request(2, 1));
    let mut reply = ScopedReply::new();
    assert!(reply.retain(scope, 0x1234));
    reply.poll(
        |_| Check::Invalid,
        |got| {
            assert_eq!(got, scope);
            Check::Live
        },
        |_| 0xdead,
        |word| {
            assert_eq!(word, 0xdead);
            false
        },
    );
    assert!(reply.pending());
    reply.poll(|_| Check::Wait, |_| panic!(), |_| panic!(), |_| panic!());
    reply.poll(
        |_| Check::Live,
        |_| panic!(),
        |_| panic!(),
        |word| {
            assert_eq!(word, 0xdead);
            true
        },
    );
    assert!(!reply.pending());
    assert!(reply.retain(scope, 0x1234));
    reply.retire(108);
    assert!(reply.pending());
    reply.retire(107);
    assert!(!reply.pending());
    reply.poll(
        |_| panic!(),
        |_| panic!(),
        |_| panic!(),
        |_| panic!("cleanup must clear before Files ACK"),
    );
}

#[test]
fn provider_retirement_clears_only_the_exact_old_task_before_ack() {
    let mut role = Role::new();
    let req = request(2, 1);
    role.retain(req, BindStatus::Ok);
    role.poll(|_| Check::Live, |_| true);
    role.retain(request(2, 2), BindStatus::Ok);
    role.retire(108);
    assert!(role.pending());
    assert!(role.bound().is_some());
    role.retire(107);
    assert!(!role.pending());
    assert_eq!(role.bound(), None);
    role.poll(
        |_| panic!(),
        |_| panic!("old positive must be gone before cleanup ACK"),
    );
}
