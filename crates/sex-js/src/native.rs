// SPDX-License-Identifier: GPL-3.0-only
//! The native tier: quickjs.wasm translated to machine code at build time (wasm2c +
//! clang, see build.rs). Same guest, same exports and imports as on wasmi, so the
//! glue in sexjs.c and prelude.js is unchanged; linear memory keeps bounds checks.
use super::{args_of, Error, Page, BYTE_OPS};
use alloc::{alloc::{alloc_zeroed, dealloc, realloc, Layout}, format, string::String, vec::Vec};
use core::ffi::c_void;

extern "C" {
    fn sxn_size() -> usize;
    fn sxn_instantiate(i: *mut c_void, ctx: *mut c_void, max_bytes: u64) -> i32;
    fn sxn_free(i: *mut c_void);
    fn sxn_memory(i: *mut c_void, size: *mut u64) -> *mut u8;
    fn sxn_init(i: *mut c_void, heap: u32, out: *mut u32) -> i32;
    fn sxn_alloc(i: *mut c_void, n: u32, out: *mut u32) -> i32;
    fn sxn_release(i: *mut c_void, p: u32) -> i32;
    fn sxn_eval(i: *mut c_void, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, out: *mut u32) -> i32;
    fn sxn_module(i: *mut c_void, a: u32, b: u32, c: u32, d: u32, e: u32, f: u32, out: *mut u32) -> i32;
    fn sxn_ready(i: *mut c_void, a: u32, out: *mut u32) -> i32;
    fn sxn_dispatch(i: *mut c_void, a: u32, b: u32, c: u32, out: *mut u32) -> i32;
}

const ALIGN: usize = 16;

/// The runtime's allocator: grows or frees a block, zeroing what is new.
#[no_mangle]
unsafe extern "C" fn sexjs_rt_realloc(ptr: *mut u8, old: usize, new: usize) -> *mut u8 {
    unsafe {
        if new == 0 {
            if !ptr.is_null() && old != 0 { dealloc(ptr, Layout::from_size_align_unchecked(old, ALIGN)); }
            return core::ptr::null_mut();
        }
        if ptr.is_null() || old == 0 { return alloc_zeroed(Layout::from_size_align_unchecked(new, ALIGN)); }
        let p = realloc(ptr, Layout::from_size_align_unchecked(old, ALIGN), new);
        if !p.is_null() && new > old { core::ptr::write_bytes(p.add(old), 0, new - old); }
        p
    }
}

/// What the guest's imports reach: the instance (for its memory) and the page.
struct Ctx {
    instance: *mut c_void,
    page: *mut dyn Page,
    result: Vec<u8>,
    #[cfg(feature = "baseline-jit")]
    jit: super::baseline::Jit,
}

unsafe fn memory<'a>(ctx: &Ctx) -> &'a mut [u8] {
    let mut size = 0u64;
    unsafe {
        let p = sxn_memory(ctx.instance, &mut size);
        core::slice::from_raw_parts_mut(p, size as usize)
    }
}

#[no_mangle]
unsafe extern "C" fn w2c_dom_call(ctx: *mut Ctx, op: u32, ptr: u32, len: u32) -> u32 {
    let ctx = unsafe { &mut *ctx };
    let data = unsafe { memory(ctx) };
    let page = unsafe { &mut *ctx.page };
    let Some(args) = args_of(data, ptr as i32, len as i32) else { return u32::MAX };
    let r = if op >= BYTE_OPS { page.call_bytes(op, &args) } else {
        let Some(text) = args.iter().map(|a| core::str::from_utf8(a).ok()).collect::<Option<Vec<&str>>>() else { return u32::MAX };
        page.call(op, &text).map(String::into_bytes)
    };
    match r {
        Some(r) => { let n = r.len() as u32; ctx.result = r; n }
        None => u32::MAX,
    }
}

#[no_mangle]
unsafe extern "C" fn w2c_dom_result(ctx: *mut Ctx, out: u32) {
    let ctx = unsafe { &mut *ctx };
    let data = unsafe { memory(ctx) };
    let r = core::mem::take(&mut ctx.result);
    let at = out as usize;
    if let Some(dst) = data.get_mut(at..at + r.len()) { dst.copy_from_slice(&r); }
}

#[no_mangle]
unsafe extern "C" fn w2c_wasi__snapshot__preview1_clock_time_get(ctx: *mut Ctx, _id: u32, _p: u64, at: u32) -> u32 {
    let ctx = unsafe { &mut *ctx };
    let now = unsafe { (*ctx.page).now_ns() };
    let data = unsafe { memory(ctx) };
    match data.get_mut(at as usize..at as usize + 8) { Some(d) => { d.copy_from_slice(&now.to_le_bytes()); 0 } None => 21 }
}

#[no_mangle]
unsafe extern "C" fn w2c_wasi__snapshot__preview1_random_get(ctx: *mut Ctx, ptr: u32, len: u32) -> u32 {
    let ctx = unsafe { &mut *ctx };
    // QuickJS hash seeding, identical to wasmi; this is not a crypto API.
    let mut seed = unsafe { (*ctx.page).now_ns() } ^ 0x9E37_79B9_7F4A_7C15;
    let data = unsafe { memory(ctx) };
    let Some(dst) = data.get_mut(ptr as usize..ptr as usize + len as usize) else { return 21 };
    for b in dst { seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; *b = seed as u8; }
    0
}

#[no_mangle]
unsafe extern "C" fn w2c_wasi__snapshot__preview1_fd_write(ctx: *mut Ctx, _fd: u32, iovs: u32, count: u32, written: u32) -> u32 {
    let data = unsafe { memory(&*ctx) };
    let mut total = 0u32;
    for i in 0..count as usize {
        let at = iovs as usize + i * 8;
        let Some(b) = data.get(at + 4..at + 8) else { return 21 };
        total = total.wrapping_add(u32::from_le_bytes(b.try_into().unwrap()));
    }
    if let Some(o) = data.get_mut(written as usize..written as usize + 4) { o.copy_from_slice(&total.to_le_bytes()); }
    0
}

#[no_mangle]
extern "C" fn w2c_wasi__snapshot__preview1_fd_close(_: *mut Ctx, _fd: u32) -> u32 { 8 }
#[no_mangle]
extern "C" fn w2c_wasi__snapshot__preview1_fd_fdstat_get(_: *mut Ctx, _fd: u32, _at: u32) -> u32 { 8 }
#[no_mangle]
extern "C" fn w2c_wasi__snapshot__preview1_fd_seek(_: *mut Ctx, _fd: u32, _o: u64, _w: u32, _at: u32) -> u32 { 8 }

#[no_mangle]
extern "C" fn w2c_dom_jit_available(_: *mut Ctx) -> u32 { if cfg!(feature = "baseline-jit") {2} else {0} }

// An older guest understands only integer results. Keep its import but never
// enter protocol-2 code through it; an accidental stale pair uses interpreter fallback.
#[no_mangle]
extern "C" fn w2c_dom_jit(_: *mut Ctx, _: u32, _: u32, _: u32, _: u32, _: u32) -> u32 { 0 }

#[no_mangle]
unsafe extern "C" fn w2c_dom_jit_v2(ctx: *mut Ctx, pc: u32, len: u32, argv: u32, argc: u32, out: u32) -> u32 {
    #[cfg(feature = "baseline-jit")]
    {
        let ctx = unsafe { &mut *ctx };
        let data = unsafe { memory(ctx) };
        return ctx.jit.run(data, pc, len, argv, argc, out) as u32;
    }
    #[cfg(not(feature = "baseline-jit"))]
    { let _ = (ctx, pc, len, argv, argc, out); 0 }
}

/// One instance of the native engine, with the page it serves.
pub(crate) struct Native<P: Page + 'static> {
    instance: *mut c_void,
    ctx: *mut Ctx,
    page: *mut P,
    dead: bool,
}

fn trap(code: i32) -> Error { Error::Trapped(format!("native trap {code}")) }

impl<P: Page + 'static> Native<P> {
    pub fn new(page: P, memory_bytes: usize) -> Result<Self, Error> {
        let page = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(page));
        let size = unsafe { sxn_size() };
        let instance = unsafe { alloc_zeroed(Layout::from_size_align(size, ALIGN).unwrap()) } as *mut c_void;
        let ctx = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Ctx { instance, page: page as *mut dyn Page, result: Vec::new(), #[cfg(feature = "baseline-jit")] jit: super::baseline::Jit::default() }));
        let mut me = Self { instance, ctx, page, dead: true };
        let r = unsafe { sxn_instantiate(instance, ctx as *mut c_void, memory_bytes as u64) };
        if r != 0 { return Err(Error::Engine(format!("native start trapped ({r})"))); }
        me.dead = false;
        Ok(me)
    }
    #[cfg(feature = "baseline-jit")]
    pub fn set_jit_enabled(&mut self, enabled: bool) { unsafe { (*self.ctx).jit.enabled = enabled; } }
    #[cfg(feature = "baseline-jit")]
    pub fn jit_stats(&self) -> super::baseline::Stats { unsafe { (*self.ctx).jit.stats } }
    pub fn page(&self) -> &P { unsafe { &*self.page } }
    pub fn page_mut(&mut self) -> &mut P { unsafe { &mut *self.page } }

    fn run(&mut self, f: impl FnOnce(*mut c_void, *mut u32) -> i32) -> Result<i32, Error> {
        if self.dead { return Err(Error::Trapped(String::from("engine trapped earlier"))); }
        let mut out = 0u32;
        match f(self.instance, &mut out) {
            0 => Ok(out as i32),
            code => { self.dead = true; Err(trap(code)) }
        }
    }
    pub fn init(&mut self, heap: i32) -> Result<i32, Error> { self.run(|i, o| unsafe { sxn_init(i, heap as u32, o) }) }
    pub fn alloc(&mut self, n: i32) -> Result<i32, Error> { self.run(|i, o| unsafe { sxn_alloc(i, n as u32, o) }) }
    pub fn free(&mut self, p: i32) { let _ = self.run(|i, _| unsafe { sxn_release(i, p as u32) }); }
    pub fn eval(&mut self, a: (i32, i32, i32, i32, i32, i32)) -> Result<i32, Error> {
        self.run(|i, o| unsafe { sxn_eval(i, a.0 as u32, a.1 as u32, a.2 as u32, a.3 as u32, a.4 as u32, a.5 as u32, o) })
    }
    pub fn module(&mut self, a: (i32, i32, i32, i32, i32, i32)) -> Result<i32, Error> {
        self.run(|i, o| unsafe { sxn_module(i, a.0 as u32, a.1 as u32, a.2 as u32, a.3 as u32, a.4 as u32, a.5 as u32, o) })
    }
    pub fn ready(&mut self, a: i32) -> Result<i32, Error> { self.run(|i, o| unsafe { sxn_ready(i, a as u32, o) }) }
    pub fn dispatch(&mut self, a: (i32, i32, i32)) -> Result<i32, Error> {
        self.run(|i, o| unsafe { sxn_dispatch(i, a.0 as u32, a.1 as u32, a.2 as u32, o) })
    }
    pub fn write(&mut self, at: usize, bytes: &[u8]) -> Result<(), Error> {
        let data = unsafe { memory(&*self.ctx) };
        data.get_mut(at..at + bytes.len()).ok_or_else(|| Error::Trapped(String::from("write")))?.copy_from_slice(bytes);
        Ok(())
    }
    pub fn memory_bytes(&self) -> usize { unsafe { memory(&*self.ctx).len() } }
}

impl<P: Page + 'static> Drop for Native<P> {
    fn drop(&mut self) {
        unsafe {
            sxn_free(self.instance);
            dealloc(self.instance as *mut u8, Layout::from_size_align(sxn_size(), ALIGN).unwrap());
            drop(alloc::boxed::Box::from_raw(self.ctx));
            drop(alloc::boxed::Box::from_raw(self.page));
        }
    }
}
