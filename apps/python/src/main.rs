// SPDX-License-Identifier: GPL-3.0-only
//! Starts CPython 3.13 (vendor/cpython-3.13.18, PSF license): its `main` runs on
//! SexOS's POSIX guest library (sex-libc) over Rust std (sex-std), inside this program,
//! so `run python -c CODE` or `run python script.py` work like `python` anywhere
//! else, with the program's own folder as its file system and the standard library
//! frozen into the binary.
use sex_std as _;
use sex_libc as _;

#[cfg(c_program)]
extern "C" {
    fn sex_python_main(argc: core::ffi::c_int, argv: *mut *mut core::ffi::c_char) -> core::ffi::c_int;
}

fn main() {
    #[cfg(c_program)]
    {
        let args: Vec<std::ffi::CString> = std::env::args().map(|a| std::ffi::CString::new(a).unwrap_or_default()).collect();
        let mut argv: Vec<*mut core::ffi::c_char> = args.iter().map(|a| a.as_ptr() as *mut _).collect();
        argv.push(core::ptr::null_mut());
        sex_libc::posix::run(sex_python_main, &mut argv);
    }
    #[cfg(not(c_program))]
    println!("This build has no C compiler output (musl headers were missing).");
}
