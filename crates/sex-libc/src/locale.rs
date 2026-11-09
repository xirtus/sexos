// SPDX-License-Identifier: GPL-3.0-only
//! `<locale.h>`, `<ctype.h>` and `<wchar.h>` as C++'s standard library uses them:
//! one locale, "C" with UTF-8 text. Every locale handle is that one locale, so
//! streams and formatting behave the same everywhere, which is what an engine wants.
use crate::{c_char, c_int, c_long, c_void};

pub type wchar_t = i32;
pub type locale_t = *mut c_void;
/// The one locale; its address is what every handle points to.
static mut THE_LOCALE: u64 = 0;
fn the_locale() -> locale_t { core::ptr::addr_of_mut!(THE_LOCALE).cast() }

#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn newlocale(_mask: c_int, _name: *const c_char, _base: locale_t) -> locale_t { the_locale() }
#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn freelocale(_l: locale_t) {}
#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn uselocale(_l: locale_t) -> locale_t { the_locale() }
/// Longest multibyte character: UTF-8.
#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn __ctype_get_mb_cur_max() -> usize { 4 }

#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn tolower(c: c_int) -> c_int { if (b'A' as c_int..=b'Z' as c_int).contains(&c) { c + 32 } else { c } }
#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn toupper(c: c_int) -> c_int { if (b'a' as c_int..=b'z' as c_int).contains(&c) { c - 32 } else { c } }
fn ascii(c: c_int, f: fn(&u8) -> bool) -> c_int { ((0..128).contains(&c) && f(&(c as u8))) as c_int }
#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn isdigit_l(c: c_int, _l: locale_t) -> c_int { ascii(c, u8::is_ascii_digit) }
#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn isxdigit_l(c: c_int, _l: locale_t) -> c_int { ascii(c, u8::is_ascii_hexdigit) }
#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn islower_l(c: c_int, _l: locale_t) -> c_int { ascii(c, u8::is_ascii_lowercase) }
#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn isupper_l(c: c_int, _l: locale_t) -> c_int { ascii(c, u8::is_ascii_uppercase) }
#[cfg_attr(target_os = "none", no_mangle)]
pub extern "C" fn iswlower_l(c: u32, _l: locale_t) -> c_int { char::from_u32(c).is_some_and(|c| c.is_lowercase()) as c_int }

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strtod_l(s: *const c_char, end: *mut *mut c_char, _l: locale_t) -> f64 { unsafe { crate::stdlib::strtod(s, end) } }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strtof_l(s: *const c_char, end: *mut *mut c_char, _l: locale_t) -> f32 { unsafe { crate::stdlib::strtof(s, end) } }

/// `long double strtold_l(...)`: x87's 80-bit result in st(0), parsed at double
/// precision (the program's own formatting never needs more).
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
#[unsafe(naked)]
#[no_mangle]
pub unsafe extern "C" fn strtold_l(_s: *const c_char, _end: *mut *mut c_char, _l: locale_t) {
    core::arch::naked_asm!(
        "sub rsp, 24",
        "call {strtod}",
        "movsd qword ptr [rsp], xmm0",
        "fld qword ptr [rsp]",
        "add rsp, 24",
        "ret",
        strtod = sym crate::stdlib::strtod,
    );
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn strftime_l(out: *mut c_char, cap: usize, fmt: *const c_char, t: *const crate::time::tm, _l: locale_t) -> usize {
    unsafe { crate::time::strftime(out, cap, fmt, t) }
}

// ── UTF-8 multibyte text ────────────────────────────────────────────────────

const ILSEQ: usize = usize::MAX;
const INCOMPLETE: usize = usize::MAX - 1;
pub const EILSEQ: c_int = 84;

/// Decodes one character from at most `n` bytes: (char, bytes) or an error code.
unsafe fn decode(s: *const u8, n: usize) -> Result<(u32, usize), usize> {
    if n == 0 { return Err(INCOMPLETE); }
    let b0 = unsafe { *s };
    let (len, init) = match b0 {
        0x00..=0x7F => return Ok((b0 as u32, 1)),
        0xC2..=0xDF => (2, (b0 & 0x1F) as u32),
        0xE0..=0xEF => (3, (b0 & 0x0F) as u32),
        0xF0..=0xF4 => (4, (b0 & 0x07) as u32),
        _ => return Err(ILSEQ),
    };
    let mut c = init;
    for i in 1..len {
        if i >= n { return Err(INCOMPLETE); }
        let b = unsafe { *s.add(i) };
        if b & 0xC0 != 0x80 { return Err(ILSEQ); }
        c = (c << 6) | (b & 0x3F) as u32;
    }
    if char::from_u32(c).is_none() || (len == 3 && c < 0x800) || (len == 4 && c < 0x10000) { return Err(ILSEQ); }
    Ok((c, len))
}
fn encode(c: u32, out: &mut [u8; 4]) -> Option<usize> {
    let ch = char::from_u32(c)?;
    Some(ch.encode_utf8(out).len())
}

/// `mbstate_t` (8 bytes in musl's headers): byte 0 holds how many bytes of an
/// unfinished character were seen, bytes 1..4 hold them, so a character split across
/// calls still decodes.
type State = [u8; 8];
static mut INTERNAL: State = [0; 8];
unsafe fn state(st: *mut c_void) -> *mut State {
    if st.is_null() { core::ptr::addr_of_mut!(INTERNAL) } else { st.cast() }
}

#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn mbrtowc(wc: *mut wchar_t, s: *const c_char, n: usize, st: *mut c_void) -> usize {
    let st = unsafe { &mut *state(st) };
    if s.is_null() { st[0] = 0; return 0; }
    let have = (st[0] as usize).min(3);
    let mut buf = [0u8; 4];
    buf[..have].copy_from_slice(&st[1..1 + have]);
    if n == 0 { return INCOMPLETE; }
    // Read only the bytes this character needs: never past its end (a string's NUL
    // may be the last readable byte).
    let lead = if have > 0 { buf[0] } else { unsafe { *s.cast::<u8>() } };
    let need = match lead { 0xC2..=0xDF => 2, 0xE0..=0xEF => 3, 0xF0..=0xF4 => 4, _ => 1 };
    let take = n.min(need.max(have + 1) - have);
    unsafe { core::ptr::copy_nonoverlapping(s.cast::<u8>(), buf.as_mut_ptr().add(have), take) };
    match unsafe { decode(buf.as_ptr(), have + take) } {
        Ok((c, len)) => {
            st[0] = 0;
            if !wc.is_null() { unsafe { *wc = c as wchar_t }; }
            if c == 0 { 0 } else { len - have }
        }
        Err(INCOMPLETE) => {
            // Every byte given was used (n < need) and the character is still unfinished.
            let total = have + take;
            st[0] = total as u8;
            st[1..1 + total].copy_from_slice(&buf[..total]);
            INCOMPLETE
        }
        Err(e) => { st[0] = 0; crate::set_errno(EILSEQ); e }
    }
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn mbrlen(s: *const c_char, n: usize, st: *mut c_void) -> usize { unsafe { mbrtowc(core::ptr::null_mut(), s, n, st) } }
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn mbtowc(wc: *mut wchar_t, s: *const c_char, n: usize) -> c_int {
    if s.is_null() { return 0; }
    let mut fresh: State = [0; 8];
    match unsafe { mbrtowc(wc, s, n, fresh.as_mut_ptr().cast()) } { r if r >= INCOMPLETE => -1, r => r as c_int }
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn wcrtomb(s: *mut c_char, wc: wchar_t, _st: *mut c_void) -> usize {
    let mut buf = [0u8; 4];
    let Some(n) = encode(wc as u32, &mut buf) else { crate::set_errno(EILSEQ); return ILSEQ };
    if !s.is_null() { unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), s.cast(), n) }; }
    n
}
/// Converts up to `nms` bytes of `*src` into at most `len` wide characters, resuming
/// any character `st` holds. With no destination it only counts, and leaves `*src`
/// and the state as they were.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn mbsnrtowcs(dst: *mut wchar_t, src: *mut *const c_char, nms: usize, len: usize, st: *mut c_void) -> usize {
    let mut copy: State = unsafe { *state(st) };
    let st: *mut c_void = if dst.is_null() { copy.as_mut_ptr().cast() } else { unsafe { state(st) }.cast() };
    let mut p = unsafe { *src };
    let mut left = nms;
    let mut count = 0;
    while dst.is_null() || count < len {
        let mut c: wchar_t = 0;
        match unsafe { mbrtowc(&mut c, p, left, st) } {
            0 => {
                if !dst.is_null() { unsafe { *dst.add(count) = 0; *src = core::ptr::null() }; }
                return count;
            }
            INCOMPLETE => { p = unsafe { p.add(left) }; break; }
            ILSEQ => { if !dst.is_null() { unsafe { *src = p }; } return ILSEQ; }
            used => {
                if !dst.is_null() { unsafe { *dst.add(count) = c }; }
                p = unsafe { p.add(used) }; left -= used; count += 1;
            }
        }
    }
    if !dst.is_null() { unsafe { *src = p }; }
    count
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn mbsrtowcs(dst: *mut wchar_t, src: *mut *const c_char, len: usize, st: *mut c_void) -> usize {
    unsafe { mbsnrtowcs(dst, src, usize::MAX, len, st) }
}
/// Converts up to `nwc` wide characters of `*src` into at most `len` bytes.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn wcsnrtombs(dst: *mut c_char, src: *mut *const wchar_t, nwc: usize, len: usize, _st: *mut c_void) -> usize {
    let mut p = unsafe { *src };
    let mut bytes = 0;
    for _ in 0..nwc {
        let c = unsafe { *p } as u32;
        let mut buf = [0u8; 4];
        let Some(n) = encode(c, &mut buf) else {
            if !dst.is_null() { unsafe { *src = p }; }
            crate::set_errno(EILSEQ);
            return ILSEQ;
        };
        if c == 0 {
            if !dst.is_null() {
                // The terminator needs room too; without it, stop before it.
                if bytes >= len { break; }
                unsafe { *dst.add(bytes) = 0; *src = core::ptr::null() };
            }
            return bytes;
        }
        if !dst.is_null() {
            if bytes + n > len { break; }
            unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), dst.add(bytes).cast(), n) };
        }
        bytes += n;
        p = unsafe { p.add(1) };
    }
    if !dst.is_null() { unsafe { *src = p }; }
    bytes
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn wcslen(s: *const wchar_t) -> usize {
    let mut n = 0;
    while unsafe { *s.add(n) } != 0 { n += 1; }
    n
}
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn wmemchr(s: *const wchar_t, c: wchar_t, n: usize) -> *mut wchar_t {
    for i in 0..n { if unsafe { *s.add(i) } == c { return unsafe { s.add(i) } as *mut wchar_t; } }
    core::ptr::null_mut()
}

/// `assert()` failed: say where, then end the program.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn __assert_fail(expr: *const c_char, file: *const c_char, line: u32, _func: *const c_char) -> ! {
    let text = |p: *const c_char| if p.is_null() { &[][..] } else { unsafe { core::slice::from_raw_parts(p.cast::<u8>(), crate::string::strnlen(p, 512)) } };
    let mut msg = alloc::vec::Vec::new();
    msg.extend_from_slice(b"assertion failed: ");
    msg.extend_from_slice(text(expr));
    msg.extend_from_slice(b" at ");
    msg.extend_from_slice(text(file));
    msg.extend_from_slice(alloc::format!(":{line}").as_bytes());
    crate::log(&msg);
    unsafe { crate::abort() }
}

/// `syscall()`: only the futex calls C++'s atomics make, as program-thread waits.
/// Everything else is refused: C code reaches SexOS through its program, not here.
#[cfg_attr(target_os = "none", no_mangle)]
pub unsafe extern "C" fn syscall(n: c_long, mut args: ...) -> c_long {
    const SYS_FUTEX: c_long = 202;
    if n != SYS_FUTEX { crate::set_errno(crate::ENOSYS); return -1; }
    let (addr, op, val) = unsafe { (args.next_arg::<*const u32>(), args.next_arg::<c_int>(), args.next_arg::<u32>()) };
    match op & 0x7F {
        0 => { sex_pdx::program_threads::wait(addr, val); 0 }
        1 => sex_pdx::program_threads::wake(addr, val as u64) as c_long,
        _ => { crate::set_errno(crate::ENOSYS); -1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_utf8_resumes() {
        let mut st: State = [0; 8];
        let mut wc: wchar_t = 0;
        let (a, b) = ([0xC3u8], [0xA9u8]);
        assert_eq!(unsafe { mbrtowc(&mut wc, a.as_ptr().cast(), 1, st.as_mut_ptr().cast()) }, INCOMPLETE);
        assert_eq!(unsafe { mbrtowc(&mut wc, b.as_ptr().cast(), 1, st.as_mut_ptr().cast()) }, 1);
        assert_eq!(wc, 0xE9);
        assert_eq!(st[0], 0);
    }
    #[test]
    fn wide_to_utf8_respects_capacity() {
        let text: [wchar_t; 2] = [b'A' as wchar_t, 0];
        let mut out = [0x55u8; 2];
        let mut src = text.as_ptr();
        assert_eq!(unsafe { wcsnrtombs(out.as_mut_ptr().cast(), &mut src, 2, 1, core::ptr::null_mut()) }, 1);
        assert_eq!(out, [b'A', 0x55]);
        assert_eq!(src, unsafe { text.as_ptr().add(1) });
        let mut src = text.as_ptr();
        assert_eq!(unsafe { wcsnrtombs(out.as_mut_ptr().cast(), &mut src, 2, 2, core::ptr::null_mut()) }, 1);
        assert_eq!(out, [b'A', 0]);
        assert!(src.is_null());
    }
    #[test]
    fn multibyte_string_converts() {
        let text = "héllo\0";
        let mut out = [0 as wchar_t; 8];
        let mut src = text.as_ptr() as *const c_char;
        assert_eq!(unsafe { mbsrtowcs(out.as_mut_ptr(), &mut src, 8, core::ptr::null_mut()) }, 5);
        assert_eq!(out[1], 0xE9);
        assert!(src.is_null());
    }
}
