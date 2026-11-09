// SPDX-License-Identifier: GPL-3.0-only
//! `run cpp`: C++ in a SexOS program. The C++ half (src/demo.cpp) uses libc++'s
//! containers, strings, streams, virtual calls, std::thread, std::mutex and
//! thread_local; this Rust half opens the window, runs the C++ static constructors,
//! and shows the report. C++ belongs to ported engines in their own organs: the
//! system around them stays Rust.
#![no_std]
#![no_main]

extern crate alloc;
use sex_app::{Event, Window};

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    sex_pdx::serial_record!("[cpp.fatal] {}", info.message());
    sex_pdx::task_finish(1);
}

fn log(bytes: &[u8]) {
    sex_pdx::serial_record!("[cpp.log] {}", core::str::from_utf8(bytes).unwrap_or("?"));
}
fn exit(code: core::ffi::c_int) -> ! { sex_pdx::task_finish(code as u64) }
fn clock() -> (i64, u64) { let ns = sex_pdx::get_ticks() * 1_000_000; (ns as i64, ns) }

#[cfg(libcxx)]
extern "C" { fn cpp_demo(out: *mut u8, size: usize) -> core::ffi::c_int; }

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut window = Window::open();
    window.title(b"C++");
    let (target, task) = (window.target, window.task);
    sex_libc::set_log(log);
    sex_libc::set_exit(exit);
    sex_libc::time::set_clock(clock);
    let mut text = [0u8; 2048];
    #[cfg(libcxx)]
    let (ok, len) = unsafe {
        sex_libc::cxx::run_constructors();
        let status = cpp_demo(text.as_mut_ptr(), text.len());
        (status == 0, text.iter().position(|&b| b == 0).unwrap_or(text.len()))
    };
    #[cfg(not(libcxx))]
    let (ok, len) = {
        let why = b"0/0\nlibc++ was not built for this image (scripts/build_libcxx.sh)\n";
        text[..why.len()].copy_from_slice(why);
        (false, why.len())
    };
    let report = &text[..len];
    let summary = report.split(|&b| b == b'\n').next().unwrap_or(b"");
    let summary = core::str::from_utf8(summary).unwrap_or("?");
    if ok {
        sex_pdx::serial_record!("[cpp.ok] target={} task={} passed={}", target, task, summary);
    } else {
        sex_pdx::serial_record!("[cpp.fail] target={} task={} passed={}", target, task, summary);
        for line in report.split(|&b| b == b'\n') {
            if !line.is_empty() { log(line); }
        }
    }
    window.show(report);
    loop {
        if let Event::Close = window.next() {
            sex_pdx::serial_record!("[cpp.closed] target={} task={}", target, task);
            window.close()
        }
    }
}
