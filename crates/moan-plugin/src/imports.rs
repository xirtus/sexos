//! C library functions a plugin library may import, implemented in Rust.
//!
//! Plugins are linked with no libc. Their undefined symbols resolve only
//! against this explicit list (plus allocator entry points that the helper
//! supplies), never against an ambient process. Everything here is pure
//! computation or bounded string/memory work: nothing reaches files, clocks,
//! threads or the network.
use core::ffi::{c_char, c_int, c_void};

macro_rules! unary {
    ($($name:ident: $t:ty => $f:path;)*) => {$(
        pub extern "C" fn $name(x: $t) -> $t { $f(x) }
    )*};
}
macro_rules! binary {
    ($($name:ident: $t:ty => $f:path;)*) => {$(
        pub extern "C" fn $name(x: $t, y: $t) -> $t { $f(x, y) }
    )*};
}
pub mod math {
    unary! {
        sinf: f32 => libm::sinf; cosf: f32 => libm::cosf; tanf: f32 => libm::tanf;
        asinf: f32 => libm::asinf; acosf: f32 => libm::acosf; atanf: f32 => libm::atanf;
        sinhf: f32 => libm::sinhf; coshf: f32 => libm::coshf; tanhf: f32 => libm::tanhf;
        expf: f32 => libm::expf; exp2f: f32 => libm::exp2f; expm1f: f32 => libm::expm1f;
        logf: f32 => libm::logf; log2f: f32 => libm::log2f; log10f: f32 => libm::log10f;
        log1pf: f32 => libm::log1pf; sqrtf: f32 => libm::sqrtf; cbrtf: f32 => libm::cbrtf;
        fabsf: f32 => libm::fabsf; floorf: f32 => libm::floorf; ceilf: f32 => libm::ceilf;
        roundf: f32 => libm::roundf; truncf: f32 => libm::truncf; rintf: f32 => libm::rintf;
        sin: f64 => libm::sin; cos: f64 => libm::cos; tan: f64 => libm::tan;
        asin: f64 => libm::asin; acos: f64 => libm::acos; atan: f64 => libm::atan;
        sinh: f64 => libm::sinh; cosh: f64 => libm::cosh; tanh: f64 => libm::tanh;
        exp: f64 => libm::exp; exp2: f64 => libm::exp2; expm1: f64 => libm::expm1;
        log: f64 => libm::log; log2: f64 => libm::log2; log10: f64 => libm::log10;
        log1p: f64 => libm::log1p; sqrt: f64 => libm::sqrt; cbrt: f64 => libm::cbrt;
        fabs: f64 => libm::fabs; floor: f64 => libm::floor; ceil: f64 => libm::ceil;
        round: f64 => libm::round; trunc: f64 => libm::trunc; rint: f64 => libm::rint;
    }
    binary! {
        powf: f32 => libm::powf; atan2f: f32 => libm::atan2f; fmodf: f32 => libm::fmodf;
        hypotf: f32 => libm::hypotf; fminf: f32 => libm::fminf; fmaxf: f32 => libm::fmaxf;
        pow: f64 => libm::pow; atan2: f64 => libm::atan2; fmod: f64 => libm::fmod;
        hypot: f64 => libm::hypot; fmin: f64 => libm::fmin; fmax: f64 => libm::fmax;
    }
    pub extern "C" fn lrintf(x: f32) -> i64 {
        libm::rintf(x) as i64
    }
    pub extern "C" fn lrint(x: f64) -> i64 {
        libm::rint(x) as i64
    }
    pub extern "C" fn sincosf(x: f32, s: *mut f32, c: *mut f32) {
        let (a, b) = libm::sincosf(x);
        unsafe {
            if !s.is_null() {
                *s = a;
            }
            if !c.is_null() {
                *c = b;
            }
        }
    }
}

pub unsafe extern "C" fn memcpy(d: *mut c_void, s: *const c_void, n: usize) -> *mut c_void {
    core::ptr::copy_nonoverlapping(s.cast::<u8>(), d.cast::<u8>(), n);
    d
}
pub unsafe extern "C" fn memmove(d: *mut c_void, s: *const c_void, n: usize) -> *mut c_void {
    core::ptr::copy(s.cast::<u8>(), d.cast::<u8>(), n);
    d
}
pub unsafe extern "C" fn memset(d: *mut c_void, c: c_int, n: usize) -> *mut c_void {
    core::ptr::write_bytes(d.cast::<u8>(), c as u8, n);
    d
}
pub unsafe extern "C" fn memcmp(a: *const c_void, b: *const c_void, n: usize) -> c_int {
    let (a, b) = (
        core::slice::from_raw_parts(a.cast::<u8>(), n),
        core::slice::from_raw_parts(b.cast::<u8>(), n),
    );
    for (x, y) in a.iter().zip(b) {
        if x != y {
            return *x as c_int - *y as c_int;
        }
    }
    0
}
pub unsafe extern "C" fn strlen(s: *const c_char) -> usize {
    core::ffi::CStr::from_ptr(s).to_bytes().len()
}
pub unsafe extern "C" fn strcmp(a: *const c_char, b: *const c_char) -> c_int {
    strncmp(a, b, usize::MAX)
}
pub unsafe extern "C" fn strncmp(a: *const c_char, b: *const c_char, n: usize) -> c_int {
    for i in 0..n {
        let (x, y) = (*a.add(i) as u8, *b.add(i) as u8);
        if x != y || x == 0 {
            return x as c_int - y as c_int;
        }
    }
    0
}

/// Imports the helper supplies itself (allocator, `strdup`, `snprintf`); see
/// `mapped.rs`. Together with [`lookup`] this is a plugin's whole C surface.
pub const HELPER: [&[u8]; 8] = [
    b"moan_alloc",
    b"moan_free",
    b"malloc",
    b"calloc",
    b"realloc",
    b"free",
    b"strdup",
    b"snprintf",
];

/// Whether a plugin library may import `name`.
pub fn allowed(name: &[u8]) -> bool {
    lookup(name).is_some() || HELPER.contains(&name)
}

/// Names of the pure functions above, with their addresses.
pub fn lookup(name: &[u8]) -> Option<usize> {
    use math::*;
    macro_rules! table {
        ($($f:ident),* $(,)?) => {
            match name {
                $(n if n == stringify!($f).as_bytes() => $f as *const () as usize,)*
                _ => return None,
            }
        };
    }
    Some(table!(
        sinf, cosf, tanf, asinf, acosf, atanf, sinhf, coshf, tanhf, expf, exp2f, expm1f, logf,
        log2f, log10f, log1pf, sqrtf, cbrtf, fabsf, floorf, ceilf, roundf, truncf, rintf, sin,
        cos, tan, asin, acos, atan, sinh, cosh, tanh, exp, exp2, expm1, log, log2, log10, log1p,
        sqrt, cbrt, fabs, floor, ceil, round, trunc, rint, powf, atan2f, fmodf, hypotf, fminf,
        fmaxf, pow, atan2, fmod, hypot, fmin, fmax, lrintf, lrint, sincosf, memcpy, memmove,
        memset, memcmp, strlen, strcmp, strncmp,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_resolves_math_and_strings() {
        assert!(lookup(b"powf").is_some());
        assert!(lookup(b"memset").is_some());
        assert!(lookup(b"open").is_none());
        assert!(lookup(b"mmap").is_none());
        let f: extern "C" fn(f32, f32) -> f32 =
            unsafe { core::mem::transmute(lookup(b"powf").unwrap()) };
        assert_eq!(f(10.0, 2.0), 100.0);
        unsafe {
            assert_eq!(strcmp(c"abc".as_ptr(), c"abd".as_ptr()), -1);
            assert_eq!(strlen(c"lv2".as_ptr()), 3);
        }
    }
}
