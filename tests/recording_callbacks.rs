//! Actual Rust encoder callbacks with a declared scripted C-ABI boundary.
//! Real FFmpeg encoding is qualified separately by recording_guest_gate.py.
extern crate alloc;
#[path = "../crates/sex-media/src/recording/mod.rs"]
mod recording;
fn init_libc() {}

use core::ffi::{c_int, c_void};

#[no_mangle]
extern "C" fn sx_record_open(
    _: c_int,
    _: c_int,
    _: c_int,
    _: *mut c_void,
    _: unsafe extern "C" fn(*mut c_void, *const u8, c_int) -> c_int,
    _: unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64,
    error: *mut c_int,
) -> *mut c_void {
    unsafe {
        *error = 0;
    }
    alloc::boxed::Box::into_raw(alloc::boxed::Box::new(0u64)).cast()
}
#[no_mangle]
extern "C" fn sx_record_free(raw: *mut c_void) {
    if !raw.is_null() {
        unsafe {
            drop(alloc::boxed::Box::from_raw(raw.cast::<u64>()));
        }
    }
}
#[no_mangle]
extern "C" fn sx_record_frame(_: *mut c_void, _: *const u32, _: i64) -> c_int {
    0
}
#[no_mangle]
extern "C" fn sx_record_finish(_: *mut c_void) -> c_int {
    0
}
