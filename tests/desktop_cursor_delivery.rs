//! Execute production cursor send/flush code with only PDX transport mocked.
#![allow(dead_code, unused_unsafe, static_mut_refs)]
extern crate self as sex_pdx;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
#[macro_export]
macro_rules! serial_record{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
macro_rules! serial_println{($($a:tt)*)=>{{let _=format_args!($($a)*);}};}
include!(env!("CURSOR_STATUS_SOURCE"));
static LOCK: Mutex<()> = Mutex::new(());
static STATUS: AtomicU64 = AtomicU64::new(0);
static CALLS: Mutex<Vec<(u64, u64, u64, u64, u64)>> = Mutex::new(Vec::new());
const SLOT_DISPLAY: u64 = 5;
const OP_SURFACE_UPDATE: u64 = 0xEB;
const SURFACE_ID_CURSOR: u64 = 0xC0;
static mut CURSOR_SEND_PENDING: bool = false;
static mut CURSOR_APPLIES_SINCE_SEND: u32 = 0;
static mut POINTER_X: i32 = 0;
static mut POINTER_Y: i32 = 0;
static mut INPUT_TRACE_SEQ: u32 = 8;
static mut SHELL_APPLY_TICK: u32 = 7;
static mut CURSOR_SEND_COUNT: u32 = 0;
static mut INPUT_TRACE_SENDS: u32 = 0;
static mut ABS_SAMPLE_COUNT: u32 = 0;
fn desktop_width() -> i32 {
    1280
}
fn desktop_height() -> i32 {
    800
}
fn pdx_call(slot: u64, op: u64, sid: u64, a: u64, b: u64) -> (u64, u64) {
    CALLS.lock().unwrap().push((slot, op, sid, a, b));
    (STATUS.load(Ordering::Relaxed), 0)
}
include!(env!("CURSOR_DELIVERY_SOURCE"));
#[test]
fn full_retains_latest_and_only_admission_advances_send_counters() {
    let _guard = LOCK.lock().unwrap();
    CALLS.lock().unwrap().clear();
    STATUS.store(ERR_SERVICE_NOT_READY, Ordering::Relaxed);
    unsafe {
        CURSOR_SEND_PENDING = false;
        CURSOR_SEND_COUNT = 0;
        INPUT_TRACE_SENDS = 0;
        send_cursor_checked(10, 20, "abs");
        assert!(CURSOR_SEND_PENDING);
        assert_eq!((POINTER_X, POINTER_Y), (10, 20));
        send_cursor_checked(30, 40, "abs");
        assert!(CURSOR_SEND_PENDING);
        assert_eq!((POINTER_X, POINTER_Y), (30, 40));
        assert_eq!((CURSOR_SEND_COUNT, INPUT_TRACE_SENDS), (0, 0));
        STATUS.store(0, Ordering::Relaxed);
        flush_pending_cursor_send();
        assert!(!CURSOR_SEND_PENDING);
        assert_eq!((CURSOR_SEND_COUNT, INPUT_TRACE_SENDS), (1, 1));
        assert_eq!(
            CALLS.lock().unwrap().last().copied(),
            Some((5, 0xEB, 0xC0, (8 << 32) | 30, (7 << 32) | 40))
        );
        let count = CALLS.lock().unwrap().len();
        flush_pending_cursor_send();
        assert_eq!(CALLS.lock().unwrap().len(), count);
    }
}
#[test]
fn hard_rejection_does_not_spin_and_coordinates_remain_bounded() {
    let _guard = LOCK.lock().unwrap();
    CALLS.lock().unwrap().clear();
    STATUS.store(99, Ordering::Relaxed);
    unsafe {
        CURSOR_SEND_PENDING = true;
        CURSOR_SEND_COUNT = 0;
        INPUT_TRACE_SENDS = 0;
        send_cursor_checked(-500, 2000, "abs");
        assert!(!CURSOR_SEND_PENDING);
        assert_eq!((POINTER_X, POINTER_Y), (0, 799));
        assert_eq!((CURSOR_SEND_COUNT, INPUT_TRACE_SENDS), (0, 0));
        let count = CALLS.lock().unwrap().len();
        flush_pending_cursor_send();
        assert_eq!(CALLS.lock().unwrap().len(), count);
    }
}
