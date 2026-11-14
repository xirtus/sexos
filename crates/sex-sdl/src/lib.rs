// SPDX-License-Identifier: GPL-3.0-only
//! SDL2 for SexOS programs. A C program that uses SDL links this crate and gets a
//! static SDL2 (vendor/SDL2-2.30.9, zlib) built for SexOS. These are the backends:
//!
//! * video: the program's own window. SDL's window surface is the window's pixel
//!   area, and `SDL_UpdateWindowSurface` shows it.
//! * events: keys, pointer and close from Shell, as SDL keyboard, text, mouse and
//!   window events.
//! * audio: the program's sound ring, which Bell mixes into the sound card. SDL's
//!   mixing thread is a sex-libc pthread, so it runs on one of the program's workers.
//! * timers: sex-libc's `clock_gettime` and `nanosleep`.
//!
//! Joysticks, haptics, sensors, HIDAPI, OpenGL/Vulkan and shared objects are off.
//!
//! The C side (crates/sex-sdl/c) calls the `extern "C"` functions below. Nothing here
//! goes past the program's own window and sound ring, so SDL holds only the authority
//! of the program's BOOT row.
//!
//! A program uses it with `use sex_sdl as _;`. Its build script finds SDL's headers in
//! `DEP_SDL2_INCLUDE` (`#include <SDL.h>`).
use sex_libc as _;

#[cfg(target_os = "hermit")]
mod shim {
    use core::ffi::{c_char, CStr};
    use sex_std::sex_app::{self, sound::Sound, Event, Window};
    use sex_std::sex_pdx::app_pointer::Kind;
    use std::sync::Mutex;

    #[repr(C)]
    pub struct SexSdlEvent {
        kind: i32,
        code: i32,
        shift: i32,
        control: i32,
        text: i32,
        x: i32,
        y: i32,
        buttons: i32,
        pointer: i32,
        detail: i32,
    }

    /// The window is the main task's: SDL's video calls come from the thread that
    /// called SDL_Init, as SDL requires.
    fn window() -> &'static mut Window { sex_std::window() }

    #[no_mangle]
    pub unsafe extern "C" fn sex_sdl_window_pixels(len: *mut u64) -> *mut u32 {
        let pixels = window().pixels();
        if !len.is_null() { unsafe { *len = pixels.len() as u64 }; }
        pixels.as_mut_ptr()
    }

    #[no_mangle]
    pub unsafe extern "C" fn sex_sdl_window_size(w: *mut i32, h: *mut i32) {
        let (width, height) = window().size();
        unsafe { *w = width as i32; *h = height as i32; }
    }

    static PRESENTED: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

    #[no_mangle]
    pub extern "C" fn sex_sdl_window_present(w: i32, h: i32) {
        let window = window();
        window.show_pixels(w.max(0) as usize, h.max(0) as usize);
        // The first frames and then one in a thousand, so a game does not flood the record.
        let n = PRESENTED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1;
        if n <= 4 || n % 1000 == 0 { sex_std::record(&format!("[sdl.present] n={n} w={w} h={h}")); }
    }

    #[no_mangle]
    pub unsafe extern "C" fn sex_sdl_window_title(title: *const c_char) {
        if title.is_null() { return; }
        let bytes = unsafe { CStr::from_ptr(title) }.to_bytes();
        window().title(bytes);
    }

    #[no_mangle]
    pub unsafe extern "C" fn sex_sdl_poll(out: *mut SexSdlEvent) -> i32 {
        let Some(event) = window().poll() else { return 0 };
        let mut e = SexSdlEvent { kind: 0, code: 0, shift: 0, control: 0, text: 0, x: 0, y: 0, buttons: 0, pointer: 0, detail: 0 };
        match event {
            Event::Key(key) => {
                e.kind = 1;
                e.code = key.code as i32;
                e.shift = key.shift as i32;
                e.control = key.control as i32;
                e.text = sex_app::typed(key).map_or(0, |c| c as i32);
                sex_std::record(&format!("[sdl.key] code={:#04x} shift={} control={} text={}", key.code, e.shift, e.control, e.text));
            }
            Event::Pointer(p) => {
                e.kind = 2;
                e.x = p.x as i32;
                e.y = p.y as i32;
                e.buttons = p.buttons as i32;
                (e.pointer, e.detail) = match p.kind {
                    Kind::Move => (1, 0),
                    Kind::Down { button } => (2, button as i32),
                    Kind::Up { button } => (3, button as i32),
                    Kind::Wheel { steps } => (4, steps as i32),
                    Kind::Leave => (5, 0),
                };
            }
            Event::Close => {
                e.kind = 3;
                sex_std::record("[sdl.close]");
            }
            Event::Resize { width, height } => {
                e.kind = 4;
                e.x = width as i32;
                e.y = height as i32;
            }
            // SDL games take no drops.
            Event::Drop { .. } | Event::DragEnded { .. } => return 0,
        }
        unsafe { out.write(e) };
        1
    }

    /// The sound ring, shared by SDL's audio thread and the main task. The ring is
    /// memory the whole program maps, so any of its tasks may write it; the mutex
    /// keeps them from writing it at once.
    struct Shared(Option<Sound>);
    unsafe impl Send for Shared {}
    static SOUND: Mutex<Shared> = Mutex::new(Shared(None));

    #[no_mangle]
    pub extern "C" fn sex_sdl_audio_open(rate: u32) -> i32 {
        let Ok(mut guard) = SOUND.lock() else { return 0 };
        let held = &mut guard.0;
        if held.is_none() { *held = Sound::open(); } else if let Some(s) = held.as_mut() { s.reopen(); }
        let Some(sound) = held.as_mut() else { return 0 };
        sound.set_rate(rate);
        sex_std::record(&format!("[sdl.audio.open] rate={rate}"));
        1
    }

    #[no_mangle]
    pub unsafe extern "C" fn sex_sdl_audio_write(frames: *const i16, count: u64) -> u64 {
        let Ok(mut held) = SOUND.lock() else { return 0 };
        let Some(sound) = held.0.as_mut() else { return 0 };
        let frames = unsafe { core::slice::from_raw_parts(frames as *const [i16; 2], count as usize) };
        sound.write(frames) as u64
    }

    #[no_mangle]
    pub extern "C" fn sex_sdl_audio_room() -> u64 {
        SOUND.lock().ok().and_then(|s| s.0.as_ref().map(|s| s.room() as u64)).unwrap_or(0)
    }

    #[no_mangle]
    pub extern "C" fn sex_sdl_audio_pending() -> u64 {
        SOUND.lock().ok().and_then(|s| s.0.as_ref().map(|s| s.pending())).unwrap_or(0)
    }

    #[no_mangle]
    pub extern "C" fn sex_sdl_audio_close() {
        let Ok(mut held) = SOUND.lock() else { return };
        if let Some(sound) = held.0.as_mut() {
            sex_std::record(&format!("[sdl.audio.close] written={} played={}", sound.written(), sound.played()));
            sound.close();
        }
    }

    #[no_mangle]
    pub unsafe extern "C" fn sex_sdl_record(line: *const c_char) {
        if line.is_null() { return; }
        sex_std::record(&unsafe { CStr::from_ptr(line) }.to_string_lossy());
    }
}
