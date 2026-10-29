//! Actual retained publisher, shell admission function and snapshot state; only
//! transport and the immutable kernel LIVE query are mocked.
#![allow(dead_code, static_mut_refs)]
extern crate self as sex_pdx;
pub const ERR_SERVICE_NOT_READY: u64 = u64::MAX - 1;
#[macro_export]
macro_rules! serial_record {
    ($($a:tt)*) => {};
}
#[path = "../servers/silk-shell/src/app_custody.rs"]
mod app_custody;
#[path = "../crates/sex-pdx/src/desktop_style.rs"]
pub mod desktop_style;
#[path = "../crates/sex-pdx/src/document_view.rs"]
pub mod document_view;
#[path = "../servers/silk-shell/src/linen_async.rs"]
mod linen_async;
pub struct PdxMessage {
    pub type_id: u64,
    pub caller_pd: u32,
    pub arg0: u64,
    pub arg1: u64,
    pub arg2: u64,
}
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static LIVE_STATUS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(5);
pub mod app_lifecycle {
    pub const SLOTS: [usize; 5] = [38, 39, 40, 45, 50];
    pub const RETRY: u64 = 2;
    pub const LIVE: u64 = 5;
    pub fn info(_: usize) -> Result<(), u64> {
        match crate::LIVE_STATUS.load(std::sync::atomic::Ordering::Relaxed) {
            0 => Ok(()),
            s => Err(s),
        }
    }
}
static mut APP_CUSTODY: [Option<app_custody::Entry>; 5] = [None; 5];
static mut WORKSPACE_REFRESH_PENDING: [bool; 5] = [false; 5];
static mut LINEN_ASYNC: linen_async::Snapshot = linen_async::Snapshot::new();
include!(env!("WORKSPACE_REFRESH_SOURCE"));
#[test]
fn publisher_coalesces_full_uses_exact_boot_task_and_close_fences() {
    let mut p = document_view::WorkspaceRefresh::new(9, 90);
    for _ in 0..128 {
        p.changed();
    }
    for _ in 0..8 {
        let mut attempts = 0;
        p.poll(|a| {
            attempts += 1;
            assert_eq!(a, [9, 90, 0]);
            Err(ERR_SERVICE_NOT_READY)
        });
        assert_eq!(attempts, 1);
        assert!(p.pending());
    }
    p.poll(|_| Ok(()));
    assert!(!p.pending());
    p.poll(|_| panic!());
    p.changed();
    p.close();
    p.changed();
    p.poll(|_| panic!());
    assert!(!p.pending());
    for target in [0, 12, 15, 17, u32::MAX] {
        let mut p = document_view::WorkspaceRefresh::new(target, 90);
        p.changed();
        assert!(!p.pending());
    }
    let mut p = document_view::WorkspaceRefresh::new(16, 160);
    p.changed();
    p.poll(|a| {
        assert_eq!(a, [16, 160, 0]);
        Err(3)
    });
    p.changed();
    assert!(!p.pending());
}
unsafe fn setup() {
    WORKSPACE_REFRESH_PENDING = [false; 5];
    APP_CUSTODY = [None; 5];
    APP_CUSTODY[0] = Some(app_custody::Entry::new(38, 9, 90, 156));
    LINEN_ASYNC = linen_async::Snapshot::new();
    LIVE_STATUS.store(5, std::sync::atomic::Ordering::Relaxed);
}
fn notice(caller: u32, args: [u64; 3]) -> PdxMessage {
    PdxMessage {
        type_id: document_view::WORKSPACE_CHANGED,
        caller_pd: caller,
        arg0: args[0],
        arg1: args[1],
        arg2: args[2],
    }
}
#[test]
fn actual_shell_requires_exact_live_subject_task_and_zero_payload() {
    let _l = LOCK.lock().unwrap();
    unsafe {
        for (caller, args) in [
            (12, [9, 90, 0]),
            (9, [16, 90, 0]),
            (9, [9, 9, 0]),
            (9, [9, 90, 1]),
            (9, [(1 << 32) | 9, 90, 0]),
        ] {
            setup();
            assert!(workspace_refresh_receive(&notice(caller, args)));
            assert!(!LINEN_ASYNC.can_send());
        }
        for status in [0, 1, 2, 3, u64::MAX] {
            setup();
            LIVE_STATUS.store(status, std::sync::atomic::Ordering::Relaxed);
            workspace_refresh_receive(&notice(9, [9, 90, 0]));
            assert!(!LINEN_ASYNC.can_send());
        }
        for dead in [false, true] {
            setup();
            let e = APP_CUSTODY[0].as_mut().unwrap();
            if dead {
                e.dead = true
            } else {
                e.close_pending = true
            };
            workspace_refresh_receive(&notice(9, [9, 90, 0]));
            assert!(!LINEN_ASYNC.can_send());
        }
        setup();
        assert!(workspace_refresh_receive(&notice(9, [9, 90, 0])));
        assert_eq!(LINEN_ASYNC.request().unwrap().opcode, 0x49);
    }
}
#[test]
fn in_flight_snapshot_finishes_then_burst_invalidations_collapse_into_one_rescan() {
    let _l = LOCK.lock().unwrap();
    unsafe {
        setup();
        LINEN_ASYNC.refresh(false);
        assert_eq!(LINEN_ASYNC.request().unwrap().opcode, 0x44);
        LINEN_ASYNC.sent();
        for _ in 0..128 {
            workspace_refresh_receive(&notice(9, [9, 90, 0]));
        }
        for slot in 0..16 {
            assert_eq!(
                LINEN_ASYNC.reply(7, 1, 0),
                if slot == 15 {
                    linen_async::Reply::Published
                } else {
                    linen_async::Reply::Progress
                }
            );
            if slot < 15 {
                assert_eq!(LINEN_ASYNC.request().unwrap().opcode, 0x44);
                LINEN_ASYNC.sent();
            }
        }
        assert_eq!(LINEN_ASYNC.request().unwrap().opcode, 0x49);
        LINEN_ASYNC.sent();
        assert_eq!(LINEN_ASYNC.reply(7, 1, 0), linen_async::Reply::Progress);
        for _ in 0..16 {
            assert_eq!(LINEN_ASYNC.request().unwrap().opcode, 0x44);
            LINEN_ASYNC.sent();
            LINEN_ASYNC.reply(7, 1, 0);
        }
        assert_eq!(LINEN_ASYNC.request(), None);
        assert!(!LINEN_ASYNC.busy());
    }
}
#[test]
fn custody_retry_is_retained_without_effect_and_close_cancels_before_retry() {
    let _l = LOCK.lock().unwrap();
    unsafe {
        setup();
        LIVE_STATUS.store(2, std::sync::atomic::Ordering::Relaxed);
        workspace_refresh_receive(&notice(9, [9, 90, 0]));
        for _ in 0..8 {
            workspace_refresh_poll();
            assert!(WORKSPACE_REFRESH_PENDING[0]);
            assert!(!LINEN_ASYNC.can_send());
        }
        LIVE_STATUS.store(5, std::sync::atomic::Ordering::Relaxed);
        workspace_refresh_poll();
        assert!(!WORKSPACE_REFRESH_PENDING[0]);
        assert_eq!(LINEN_ASYNC.request().unwrap().opcode, 0x49);
        setup();
        LIVE_STATUS.store(2, std::sync::atomic::Ordering::Relaxed);
        workspace_refresh_receive(&notice(9, [9, 90, 0]));
        APP_CUSTODY[0].as_mut().unwrap().close_pending = true;
        LIVE_STATUS.store(5, std::sync::atomic::Ordering::Relaxed);
        workspace_refresh_poll();
        assert!(!WORKSPACE_REFRESH_PENDING[0]);
        assert!(!LINEN_ASYNC.can_send());
    }
}
