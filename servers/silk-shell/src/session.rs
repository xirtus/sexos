//! Shell's session outlives Shell. A restart puts back the pages BOOT copied, so
//! everything Shell knew is gone; this one page is the exception. Shell asks the
//! kernel to keep it (`crash_log::keep`), writes its windows into it as they change,
//! and a Shell started again after a crash reads them back: where each window was,
//! which workspace it was on, whether it was minimized, floating or zoomed, each
//! workspace's layout and the focus.
//!
//! A half-written page is never trusted: the state word says SAVED only once the
//! checksum matches, and APPLYING while a new Shell restores it, so a Shell that
//! crashes while restoring starts the next time from its defaults.
use super::*;

const MAGIC: u64 = u64::from_le_bytes(*b"SXSESS01");
const MAX_WINDOWS: usize = 40;
const EMPTY: u64 = 0;
const SAVED: u64 = 1;
const APPLYING: u64 = 2;
/// Window states a session keeps (minimized is applied with its lifecycle).
const KEPT_FLAGS: u32 = FRAME_FLAG_MINIMIZED | FRAME_FLAG_ZOOMED | FRAME_FLAG_FLOATING;

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct SavedWindow { surface: u64, x: i32, y: i32, w: u32, h: u32, normal: [i32; 4], flags: u32, scene: u32 }
const NO_WINDOW: SavedWindow = SavedWindow { surface: 0, x: 0, y: 0, w: 0, h: 0, normal: [0; 4], flags: 0, scene: 0 };

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct SavedTiling { layout: u32, before_monocle: u32, master_pct: u32, gap_inner: u32, gap_outer: u32 }
impl SavedTiling {
    fn of(t: &wm::Tiling) -> Self {
        Self { layout: t.layout.index() as u32, before_monocle: t.before_monocle.index() as u32, master_pct: t.master_pct as u32, gap_inner: t.gap_inner, gap_outer: t.gap_outer }
    }
    fn tiling(&self) -> wm::Tiling {
        let layout = |i: u32| wm::Layout::from_index(i.min(255) as u8).unwrap_or(wm::Layout::MasterStack);
        wm::Tiling { layout: layout(self.layout), before_monocle: layout(self.before_monocle),
            master_pct: (self.master_pct.min(255) as u8).clamp(wm::MASTER_MIN, wm::MASTER_MAX),
            gap_inner: self.gap_inner.min(64), gap_outer: self.gap_outer.min(64) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
struct Snapshot { scene: u32, count: u32, focus: u64, tiling: [SavedTiling; ATLAS_MAX_SCENES], windows: [SavedWindow; MAX_WINDOWS] }

#[repr(C, align(4096))]
struct SessionPage { magic: u64, state: u64, sum: u64, snap: Snapshot }
const _: () = assert!(core::mem::size_of::<SessionPage>() == 4096);

const DEFAULT_TILING: SavedTiling = SavedTiling { layout: 0, before_monocle: 0, master_pct: 55, gap_inner: 6, gap_outer: 8 };
const BLANK: Snapshot = Snapshot { scene: 0, count: 0, focus: 0, tiling: [DEFAULT_TILING; ATLAS_MAX_SCENES], windows: [NO_WINDOW; MAX_WINDOWS] };
/// Not zero, so the page is in the image BOOT copies (and so the kernel can keep it).
static mut PAGE: SessionPage = SessionPage { magic: MAGIC, state: EMPTY, sum: 0, snap: BLANK };
static mut KEPT: bool = false;
static mut LAST_SAVE: u64 = 0;

fn checksum(s: &Snapshot) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |v: u64| { h = (h ^ v).wrapping_mul(0x0000_0100_0000_01b3); };
    mix(s.scene as u64); mix(s.count as u64); mix(s.focus);
    for t in &s.tiling { for v in [t.layout, t.before_monocle, t.master_pct, t.gap_inner, t.gap_outer] { mix(v as u64); } }
    for w in &s.windows[..(s.count as usize).min(MAX_WINDOWS)] {
        mix(w.surface); mix(w.x as u32 as u64); mix(w.y as u32 as u64); mix(w.w as u64); mix(w.h as u64);
        for n in w.normal { mix(n as u32 as u64); }
        mix(w.flags as u64); mix(w.scene as u64);
    }
    h
}

unsafe fn snapshot() -> Snapshot {
    let mut snap = BLANK;
    snap.scene = ACTIVE_SCENE_IDX as u32;
    snap.focus = FOCUSED_SURFACE_ID;
    for (saved, t) in snap.tiling.iter_mut().zip((*core::ptr::addr_of!(WM_TILING)).iter()) { *saved = SavedTiling::of(t); }
    // Windows in tiling order, so tiling puts them back where they were.
    let mut ids = [0u32; MAX_FRAMES];
    let mut n = 0;
    for frame in FRAMES.iter().flatten() { ids[n] = frame.frame_id; n += 1; }
    (*core::ptr::addr_of_mut!(WM_ORDER)).sort(&mut ids[..n]);
    for &fid in &ids[..n] {
        if snap.count as usize == MAX_WINDOWS { break; }
        let Some(frame) = FRAMES.iter().flatten().find(|f| f.frame_id == fid) else { continue };
        let Some(sid) = frame.tabs[frame.active_tab as usize].map(|t| t.surface_id) else { continue };
        if !surface_is_alive(sid) { continue; }
        let Some((x, y, w, h)) = get_surface_bounds(sid) else { continue };
        snap.windows[snap.count as usize] = SavedWindow { surface: sid, x, y, w, h,
            normal: [frame.normal_x, frame.normal_y, frame.normal_w as i32, frame.normal_h as i32],
            flags: frame.flags & KEPT_FLAGS, scene: frame.scene_id as u32 };
        snap.count += 1;
    }
    snap
}

/// From the main loop: write the windows down when they have changed.
pub(crate) unsafe fn save() {
    if !KEPT { return; }
    let now = sex_pdx::get_ticks();
    if now.wrapping_sub(LAST_SAVE) < 5 { return; }
    LAST_SAVE = now;
    let page = &mut *core::ptr::addr_of_mut!(PAGE);
    let snap = snapshot();
    if core::ptr::read_volatile(&page.state) == SAVED && page.snap == snap { return; }
    core::ptr::write_volatile(&mut page.state, EMPTY);
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    page.snap = snap;
    page.sum = checksum(&snap);
    page.magic = MAGIC;
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    core::ptr::write_volatile(&mut page.state, SAVED);
}

/// At start: ask the kernel to keep the page, and after a restart, put the last
/// run's windows back.
pub(crate) unsafe fn keep_and_restore(restarted: bool) {
    let page = &mut *core::ptr::addr_of_mut!(PAGE);
    KEPT = sex_pdx::crash_log::keep(page as *mut SessionPage as u64, 1);
    sex_pdx::serial_record!("[shell.session.kept] ok={}", KEPT as u8);
    if !restarted || !KEPT { return; }
    let state = core::ptr::read_volatile(&page.state);
    if page.magic != MAGIC || state != SAVED || page.sum != checksum(&page.snap) || page.snap.count as usize > MAX_WINDOWS {
        sex_pdx::serial_record!("[shell.session.skipped] state={}", state);
        core::ptr::write_volatile(&mut page.state, EMPTY);
        return;
    }
    core::ptr::write_volatile(&mut page.state, APPLYING);
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    let snap = page.snap;
    apply(&snap);
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    core::ptr::write_volatile(&mut page.state, SAVED);
}

unsafe fn apply(snap: &Snapshot) {
    let last_scene = WORKSPACE_COUNT - 1;
    let mut order = wm::Order::<MAX_FRAMES>::new();
    let mut minimized = [0u32; MAX_WINDOWS];
    let (mut restored, mut hidden) = (0usize, 0usize);
    // Spindle's window is the Shell's own to open: a session that had it open opens it.
    let windows = &snap.windows[..snap.count as usize];
    if windows.iter().any(|w| w.surface == SURFACE_ID_SPINDLE) && frame_for_surface(SURFACE_ID_SPINDLE).is_none() && ensure_spindle_frame().is_some() {
        if lifecycle_state(SURFACE_ID_SPINDLE).is_none() { lifecycle_register(SURFACE_ID_SPINDLE, LifecycleState::Visible); }
        else { set_lifecycle_state(SURFACE_ID_SPINDLE, LifecycleState::Visible); }
    }
    for w in windows {
        if !surface_is_alive(w.surface) { continue; }
        let Some(fid) = frame_for_surface(w.surface) else { continue };
        let Some(frame) = FRAMES.iter_mut().flatten().find(|f| f.frame_id == fid) else { continue };
        frame.scene_id = (w.scene.min(last_scene as u32)) as u8;
        frame.flags = (frame.flags & !KEPT_FLAGS) | (w.flags & (FRAME_FLAG_ZOOMED | FRAME_FLAG_FLOATING));
        frame.normal_x = w.normal[0]; frame.normal_y = w.normal[1];
        frame.normal_w = w.normal[2].max(1) as u32; frame.normal_h = w.normal[3].max(1) as u32;
        let (x, y) = clamp_position(w.x, w.y, w.w, w.h);
        update_local_geometry(w.surface, x, y, w.w.max(1), w.h.max(1));
        apply_window_geometry(w.surface, x, y, w.w.max(1), w.h.max(1));
        order.insert(fid);
        if w.flags & FRAME_FLAG_MINIMIZED != 0 { minimized[hidden] = fid; hidden += 1; }
        restored += 1;
    }
    for frame in FRAMES.iter().flatten() { order.insert(frame.frame_id); }
    WM_ORDER = order;
    for (t, saved) in (*core::ptr::addr_of_mut!(WM_TILING)).iter_mut().zip(snap.tiling.iter()) { *t = saved.tiling(); }
    ACTIVE_SCENE_IDX = (snap.scene.min(last_scene as u32)) as u8;
    for &fid in &minimized[..hidden] {
        set_frame_minimized(fid, true);
        if let Some(sid) = active_surface_for_frame(fid) { set_lifecycle_state(sid, LifecycleState::Minimized); }
    }
    sync_scene_visibility();
    tile_active_scene_frames();
    let shown = lay_out_windows_again();
    for scene in 0..WORKSPACE_COUNT { scene_update_flags(scene); }
    pdx_call(SLOT_SILKBAR, OP_SILKBAR_WORKSPACE_ACTIVE, ACTIVE_SCENE_IDX as u64, 0, 0);
    let focus = snap.focus;
    let focusable = focus != 0 && surface_is_alive(focus) && frame_for_surface(focus)
        .and_then(|fid| FRAMES.iter().flatten().find(|f| f.frame_id == fid))
        .is_some_and(|f| f.scene_id == ACTIVE_SCENE_IDX && f.flags & FRAME_FLAG_MINIMIZED == 0);
    if focusable { try_set_focus(focus); } else { clear_focus_if_wrong_scene(); }
    sex_pdx::serial_record!("[shell.session.restored] windows={} minimized={} shown={} scene={} focus={}",
        restored, hidden, shown, ACTIVE_SCENE_IDX, FOCUSED_SURFACE_ID);
}
