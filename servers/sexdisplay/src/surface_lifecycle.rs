// SPDX-License-Identifier: GPL-3.0-only
//! Display-owned inline resources. Kernel lifecycle status only attests quiescence.

pub const MAX_RECTS: usize = 8;
#[derive(Clone, Copy)]
pub struct Surface {
    pub surface_id: u64,
    pub owner_pd: u32,       // PD that created this surface; 0 = unbound
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub color: u32,
    pub active: bool,
    // Per-surface tab info (V1: set via 0xFD from shell; used for colored tab blocks)
    pub tab_count: u8,
    pub active_tab: u8,
    // Per-surface chrome flags (V1: bit 0 = top bar enabled)
    pub chrome_flags: u8,
    // Per-surface fill rects (up to MAX_RECTS; fill_count tracks highest set index + 1)
    pub fill_count: u8,
    pub fill_sx: [i32; MAX_RECTS],
    pub fill_sy: [i32; MAX_RECTS],
    pub fill_sw: [u32; MAX_RECTS],
    pub fill_sh: [u32; MAX_RECTS],
    pub fill_color: [u32; MAX_RECTS],
    pub text_len: u16,
    pub text_color: u32,
    pub selection_start: u16,
    pub selection_end: u16,
    // TEXT_MODEL_V2: 512-byte text buffer (was 128). OP_TEXT_DRAW offset
    // extends past 255 via arg2 bits 12-15 (offset bits 8-11); legacy
    // senders leave those bits zero and behave identically.
    pub text_buf: [u8; TEXT_BUF_CAP],
    // TEXT_MODEL_V2: per-line start offsets into text_buf, recomputed on
    // every text write. Lines split on '\n' and wrap at wrap_cols. When the
    // buffer contains NO '\n', wrap_cols stays at the legacy 20 so every
    // pre-V2 writer (fixed 20-char padded rows) renders unchanged; a '\n'
    // anywhere opts the surface into width-derived wrap.
    pub line_starts: [u16; TEXT_MAX_ROWS],
    // Derived scalar-cell -> source-byte lookup: at most one cell per input
    // byte, hence 512 entries plus 24 row ordinals. Rebuilt with text/geometry,
    // cleared with Surface reset; byte offsets are presentation data only.
    pub cell_starts: [u16; TEXT_BUF_CAP],
    pub line_cells: [u16; TEXT_MAX_ROWS],
    pub cell_count: u16,
    pub line_count: u8,
    pub wrap_cols: u8,
}

impl Surface {
    pub fn erase_selection(&mut self) {
        unsafe {
            core::ptr::write_volatile(&mut self.selection_start, 0);
            core::ptr::write_volatile(&mut self.selection_end, 0);
        }
    }
}

pub const MAX_SURFACES: usize = 16;
pub const TEXT_BUF_CAP: usize = 512;
pub const TEXT_MAX_ROWS: usize = 24;
pub const TEXT_LEGACY_COLS: u8 = 20;
pub const SURFACE_EMPTY: Surface = Surface {
    surface_id: 0, owner_pd: 0, x: 0, y: 0, w: 0, h: 0, color: 0, active: false,
    tab_count: 0, active_tab: 0, chrome_flags: 0,
    fill_count: 0,
    fill_sx: [0i32; MAX_RECTS], fill_sy: [0i32; MAX_RECTS],
    fill_sw: [0u32; MAX_RECTS], fill_sh: [0u32; MAX_RECTS],
    fill_color: [0u32; MAX_RECTS],
    selection_start: 0, selection_end: 0,
    text_len: 0, text_color: 0x00FFFFFF, text_buf: [0u8; TEXT_BUF_CAP],
    line_starts: [0u16; TEXT_MAX_ROWS], cell_starts: [0; TEXT_BUF_CAP],
    line_cells: [0; TEXT_MAX_ROWS], cell_count: 0, line_count: 0, wrap_cols: TEXT_LEGACY_COLS,
};

pub const CATALOG_CAPACITY: usize = 6;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Catalog {
    pub slot: usize,
    pub target_pd: u32,
    pub task_id: u32,
    pub surface_id: u64,
    pub coordinator_pd: u32,
}
impl Catalog {
    /// Existing shell prototypes have a separate frame surface. This is a
    /// fixed boot subject association, activated only by the exact app grant.
    pub fn frame_surface_id(self) -> Option<u64> {
        crate::app_catalog::by_surface(self.target_pd, self.surface_id).and_then(|i| i.shell_surface)
    }
    pub fn is_content(self, sid: u64) -> bool {
        self.surface_id == sid || crate::app_catalog::is_content(self.target_pd, self.surface_id, sid)
    }
    fn contains(self, sid: u64) -> bool {
        self.is_content(sid) || self.frame_surface_id() == Some(sid)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Live, Prepared, Committed, Acknowledged }
#[derive(Clone, Copy)]
struct Entry { catalog: Catalog, phase: Phase, presentation: Presentation }
#[derive(Clone, Copy, PartialEq, Eq)]
enum PresentPhase { Idle, WaitingPixels, ReplyPending, Delivered }
#[derive(Clone, Copy)]
struct Presentation { request: u64, phase: PresentPhase }
impl Presentation { const EMPTY: Self = Self { request: 0, phase: PresentPhase::Idle }; }
/// Fixed native content routes: no catalog scan in the per-pixel compositor.
pub const fn native_pixel_bit(owner: u32, sid: u64) -> u8 {
    crate::app_catalog::native_pixel_bit(owner, sid)
}
pub struct Lifecycle { entries: [Option<Entry>; CATALOG_CAPACITY] }
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SceneRefs {
    pub focused_sid: u64,
    pub active_app_sid: u32,
    pub token_owner: u32,
    pub token_pending: bool,
    pub cursor_under_valid: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Admission { Existing(usize), Vacant(usize, u32), Denied, Full }
impl Lifecycle {
    pub const fn new() -> Self { Self { entries: [None; CATALOG_CAPACITY] } }
    pub fn register(&mut self, catalog: Catalog) -> bool {
        if catalog.target_pd == 0 || catalog.task_id == 0 || catalog.surface_id == 0
            || catalog.coordinator_pd == 0 || catalog.coordinator_pd == catalog.target_pd { return false; }
        for entry in self.entries.iter().flatten() {
            if entry.catalog == catalog { return true; }
            if entry.catalog.slot == catalog.slot || entry.catalog.target_pd == catalog.target_pd
                || entry.catalog.contains(catalog.surface_id) || catalog.contains(entry.catalog.surface_id)
                || entry.catalog.frame_surface_id().is_some() && entry.catalog.frame_surface_id() == catalog.frame_surface_id()
                || entry.catalog.coordinator_pd != catalog.coordinator_pd { return false; }
        }
        if let Some(entry) = self.entries.iter_mut().find(|e| e.is_none()) {
            *entry = Some(Entry { catalog, phase: Phase::Live, presentation: Presentation::EMPTY }); true
        } else { false }
    }
    pub fn catalog(&self, index: usize) -> Option<Catalog> {
        self.entries.get(index).and_then(|e| e.map(|e| e.catalog))
    }
    pub fn phase(&self, index: usize) -> Option<Phase> {
        self.entries.get(index).and_then(|e| e.map(|e| e.phase))
    }
    pub fn is_coordinator(&self, caller: u32) -> bool {
        caller != 0 && self.entries.iter().flatten().any(|e| e.catalog.coordinator_pd == caller)
    }
    pub fn caller_live(&self, caller: u32) -> bool {
        caller != 0 && !self.entries.iter().flatten().any(|e| e.catalog.target_pd == caller && e.phase != Phase::Live)
    }
    pub fn sid_live(&self, sid: u64) -> bool {
        !self.entries.iter().flatten().any(|e| e.catalog.contains(sid) && e.phase != Phase::Live)
    }
    pub fn coordinator_controls(&self, caller: u32, sid: u64) -> bool {
        self.caller_live(caller) && self.entries.iter().flatten().any(|e|
            e.phase == Phase::Live && e.catalog.coordinator_pd == caller && e.catalog.contains(sid))
    }
    pub fn admit(&self, surfaces: &[Surface], caller: u32, sid: u64) -> Admission {
        if sid==152 && caller!=3 {return Admission::Denied;}
        if sid == 0 || !self.caller_live(caller) || !self.sid_live(sid) { return Admission::Denied; }
        let catalog = self.entries.iter().flatten().find(|e| e.catalog.contains(sid)).map(|e| e.catalog);
        let owner = match catalog {
            Some(c) if c.frame_surface_id() == Some(sid) && caller == c.coordinator_pd => c.coordinator_pd,
            Some(c) if c.is_content(sid) && (caller == c.target_pd || caller == c.coordinator_pd) => c.target_pd,
            Some(_) => return Admission::Denied,
            None => caller,
        };
        if let Some((index, surface)) = surfaces.iter().enumerate().find(|(_, s)| s.active && s.surface_id == sid) {
            // A denied existing ID must never fall through into a fresh slot.
            return if surface.owner_pd == owner { Admission::Existing(index) } else { Admission::Denied };
        }
        match surfaces.iter().position(|s| !s.active) {
            Some(index) => Admission::Vacant(index, owner), None => Admission::Full,
        }
    }
    /// Called only after a successful capability status proves this exact boot task quiescent.
    /// Returns count once; duplicates leave the already committed scene untouched.
    pub fn prepare(&mut self, index: usize, target: u32, task: u32,
        surfaces: &mut [Surface], refs: &mut SceneRefs) -> Option<usize> {
        let entry = self.entries.get_mut(index)?.as_mut()?;
        if entry.catalog.target_pd != target || entry.catalog.task_id != task || entry.phase != Phase::Live { return None; }
        // Fence first. Every subsequent incoming mutation is checked against this tombstone.
        entry.phase = Phase::Prepared;
        entry.presentation = Presentation::EMPTY;
        let mut count = 0;
        for surface in surfaces.iter_mut() {
            if surface.owner_pd == target || surface.surface_id == entry.catalog.surface_id
                || (entry.catalog.frame_surface_id() == Some(surface.surface_id) && surface.owner_pd == entry.catalog.coordinator_pd) {
                if refs.focused_sid == surface.surface_id { refs.focused_sid = 0; }
                if refs.active_app_sid as u64 == surface.surface_id { refs.active_app_sid = 0; }
                surface.erase_selection();
                *surface = SURFACE_EMPTY;
                count += 1;
            }
        }
        // Clear catalog references even if the corresponding surface was minimized already.
        if entry.catalog.contains(refs.focused_sid) { refs.focused_sid = 0; }
        if entry.catalog.contains(refs.active_app_sid as u64) { refs.active_app_sid = 0; }
        if refs.token_owner == target { refs.token_owner = 0; refs.token_pending = false; }
        // Old save-under pixels must never restore the retired scene.
        refs.cursor_under_valid = false;
        Some(count)
    }
    /// One serialized request per app. IDs never wrap; duplicate exact requests
    /// replay their completed result, while a newer request needs a new frame.
    pub fn request_present(&mut self, caller: u32, sid: u64, request: u64, reserved: u64) -> bool {
        if request == 0 || request > (u64::MAX >> 8) || reserved != 0 { return false; }
        let Some(entry) = self.entries.iter_mut().flatten().find(|e|
            e.catalog.target_pd == caller && e.catalog.surface_id == sid && e.phase == Phase::Live) else { return false; };
        let p = &mut entry.presentation;
        if p.request == request {
            if p.phase == PresentPhase::Delivered { p.phase = PresentPhase::ReplyPending; }
            return true;
        }
        if request < p.request || matches!(p.phase, PresentPhase::WaitingPixels | PresentPhase::ReplyPending) { return false; }
        *p = Presentation { request, phase: PresentPhase::WaitingPixels };
        true
    }
    pub fn needs_presentation(&self) -> bool {
        self.entries.iter().flatten().any(|e| e.phase == Phase::Live && e.presentation.phase == PresentPhase::WaitingPixels)
    }
    /// Must follow a completed framebuffer pass; admission or active state is insufficient.
    pub fn pixels_presented(&mut self, visible_bits: u8) {
        for entry in self.entries.iter_mut().flatten() {
            if entry.phase == Phase::Live && entry.presentation.phase == PresentPhase::WaitingPixels
                && visible_bits & native_pixel_bit(entry.catalog.target_pd, entry.catalog.surface_id) != 0 {
                entry.presentation.phase = PresentPhase::ReplyPending;
            }
        }
    }
    pub fn presentation_reply(&self, index: usize) -> Option<(u32, u64)> {
        let entry = self.entries.get(index)?.as_ref()?;
        if entry.phase != Phase::Live || entry.presentation.phase != PresentPhase::ReplyPending { return None; }
        Some((entry.catalog.target_pd, entry.presentation.request << 8))
    }
    /// Called only after enqueue succeeds. QueueFull leaves the exact value retained.
    pub fn presentation_reply_sent(&mut self, index: usize, value: u64) -> bool {
        let Some(entry) = self.entries.get_mut(index).and_then(|e| e.as_mut()) else { return false; };
        if entry.phase != Phase::Live || entry.presentation.phase != PresentPhase::ReplyPending
            || value != entry.presentation.request << 8 { return false; }
        entry.presentation.phase = PresentPhase::Delivered;
        true
    }
    pub fn scene_committed(&mut self, index: usize) -> bool {
        match self.entries.get_mut(index).and_then(|e| e.as_mut()) {
            Some(e) if e.phase == Phase::Prepared => { e.phase = Phase::Committed; true }, _ => false,
        }
    }
    pub fn acknowledge(&mut self, index: usize, target: u32, task: u32) -> bool {
        match self.entries.get_mut(index).and_then(|e| e.as_mut()) {
            Some(e) if e.catalog.target_pd == target && e.catalog.task_id == task && e.phase == Phase::Committed => {
                e.phase = Phase::Acknowledged; true
            }, _ => false,
        }
    }
}

/// Geometry application shared by both admission branches and the host harness.
pub fn upsert(surfaces: &mut [Surface], admission: Admission, sid: u64,
    x: i32, y: i32, w: u32, h: u32, color: u32) -> bool {
    let index = match admission {
        Admission::Existing(i) => i,
        Admission::Vacant(i, owner) => {
            surfaces[i] = Surface { surface_id: sid, owner_pd: owner, active: true, ..SURFACE_EMPTY };
            surfaces[i].x = x; surfaces[i].y = y; surfaces[i].w = w; surfaces[i].h = h; surfaces[i].color = color;
            return true;
        }, _ => return false,
    };
    let s = &mut surfaces[index];
    let changed = (s.x,s.y,s.w,s.h,s.color) != (x,y,w,h,color);
    s.x=x; s.y=y; s.w=w; s.h=h; s.color=color;
    changed
}
