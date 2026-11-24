// SPDX-License-Identifier: GPL-3.0-only
//! The windows a real machine starts with.
//!
//! Quil, Linen and Kaleidoscope are boot apps, so each has a window from the first
//! frame on. In a virtual machine that layout stays: it is the reference desktop the
//! desktop gates click through and time. On a real machine nobody asked for them
//! (Andreas closed all three on every x17 boot before he could use Spindle), so once
//! each has registered and drawn, the Shell minimizes it and gives Spindle the focus.
//! Minimized, they are a click away in the bar, and their services keep running.
//!
//! A Shell started again after a crash leaves every window as it was.
use super::*;

/// The boot windows to put away: Linen, Quil, Kaleidoscope.
const WINDOWS: [u64; 3] = [SURFACE_ID_LINEN, SURFACE_ID_QUIL, 300];
/// Ticks (ms) a window has had since it was first ready, so its first paint is done
/// before it goes: an app that draws after the Shell hid it would show again.
const SETTLE_TICKS: u64 = 1_500;
/// Ticks after which any window still not ready is left alone.
const GIVE_UP_TICKS: u64 = 60_000;

static mut DONE: bool = false;
static mut STARTED: u64 = 0;
static mut READY_AT: [u64; 3] = [0; 3];
static mut LEFT: u8 = 0b111;
static mut MINIMIZED: u8 = 0;

/// CPUID leaf 1, ECX bit 31: a hypervisor is present.
fn in_vm() -> bool { core::arch::x86_64::__cpuid(1).ecx & (1 << 31) != 0 }

/// Whether `sid`'s window exists and its app has registered (has a place).
unsafe fn ready(sid: u64) -> Option<u32> {
    let fid = frame_for_surface(sid)?;
    if !surface_is_alive(sid) { return None; }
    // Apps with a custody row are ready once their registration is answered. Only an
    // app whose window is its own surface (Kaleidoscope) gets custody bounds; Quil's
    // window is the Shell's frame 201 around its content, which never does.
    if let Some(e) = APP_CUSTODY.iter().flatten().find(|e| e.owns_surface(sid)) {
        if e.dead || e.registration_reply || (e.surface == sid && e.bounds.is_none()) { return None; }
    }
    Some(fid)
}

/// From the main loop.
pub(crate) unsafe fn poll() {
    if DONE { return; }
    let now = sex_pdx::get_ticks();
    if STARTED == 0 {
        STARTED = now.max(1);
        if sex_pdx::crash_log::restarts() > 0 || in_vm() {
            DONE = true;
            sex_pdx::serial_record!("[shell.boot.windows] tidy=0 reason={}",
                if in_vm() { "virtual_machine" } else { "restarted" });
            return;
        }
    }
    for (i, &sid) in WINDOWS.iter().enumerate() {
        if LEFT & (1 << i) == 0 { continue; }
        let Some(fid) = ready(sid) else { continue };
        if READY_AT[i] == 0 { READY_AT[i] = now.max(1); continue; }
        if now.wrapping_sub(READY_AT[i]) < SETTLE_TICKS { continue; }
        LEFT &= !(1 << i);
        // The person may already have put it away (or closed it) themselves.
        if !frame_is_minimized(fid) && minimize_frame(fid) { MINIMIZED |= 1 << i; }
    }
    if LEFT != 0 && now.wrapping_sub(STARTED) < GIVE_UP_TICKS { return; }
    DONE = true;
    // The ordinary open path: maps and tiles the frame if it is not up yet, restores it
    // if minimized, and focuses it.
    let focused = open_spindle_in_active_scene();
    sex_pdx::serial_record!("[shell.boot.windows] tidy=1 minimized={:#05b} left={:#05b} spindle_focus={} ms={}",
        MINIMIZED, LEFT, focused as u8, now.wrapping_sub(STARTED));
}
