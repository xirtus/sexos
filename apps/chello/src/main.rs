// SPDX-License-Identifier: GPL-3.0-only
//! Starts the ordinary C program in chello.c: its `main(argc, argv)` runs on SexOS's
//! POSIX guest library (sex-libc) over Rust std (sex-std), inside this program.
use sex_std as _;
use sex_libc as _;

#[cfg(c_program)]
extern "C" {
    fn sex_c_main(argc: core::ffi::c_int, argv: *mut *mut core::ffi::c_char) -> core::ffi::c_int;
}

fn main() {
    #[cfg(c_program)]
    {
        let args: Vec<std::ffi::CString> = std::env::args().map(|a| std::ffi::CString::new(a).unwrap_or_default()).collect();
        let mut argv: Vec<*mut core::ffi::c_char> = args.iter().map(|a| a.as_ptr() as *mut _).collect();
        argv.push(core::ptr::null_mut());
        sex_libc::posix::run(sex_c_main, &mut argv);
    }
    #[cfg(not(c_program))]
    println!("This build has no C compiler output (musl headers were missing).");
}
