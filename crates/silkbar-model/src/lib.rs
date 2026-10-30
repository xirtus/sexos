#![no_std]

// ── Geometry Constants (default positions for layout construction) ──────────

pub const PANEL_X: usize = 80;
pub const PANEL_Y: usize = 10;
pub const PANEL_W: usize = 1120;
pub const PANEL_H: usize = 38;
pub const PANEL_GLOW: usize = 2;
pub const PANEL_RADIUS_STEP: usize = 3;
pub const PANEL_MARGIN_TOP: usize = 10;

pub const WORKSPACE_COUNT: usize = 5;
pub const MAX_CHIPS: usize = 4;

// ── Selected Window Option Bits (matches OPTION_* in silk-shell) ──────────
/// Bit: selected frame can be closed/destroyed.
pub const OPTION_CLOSE: u32 = 1;
/// Bit: selected frame can be zoomed/maximized.
pub const OPTION_ZOOM: u32 = 2;
/// Bit: selected frame can be minimized/hidden.
pub const OPTION_MINIMIZE: u32 = 4;
/// Bit: selected frame can be moved via rim drag.
pub const OPTION_MOVE: u32 = 8;

pub const WS_Y: usize = 18;
pub const WS_H: usize = 22;
pub const WS_INACTIVE_W: usize = 20;
pub const WS_ACTIVE_W: usize = 30;
pub const WS_X0: usize = 557;
pub const WS_X1: usize = 589;
pub const WS_X2: usize = 625;
pub const WS_X3: usize = 671;
pub const WS_X4: usize = 707;

pub const LAUNCHER_X: usize = 100;
pub const LAUNCHER_Y: usize = 19;
pub const LAUNCHER_W: usize = 20;
pub const LAUNCHER_H: usize = 20;

pub const CHIP_Y: usize = 18;
pub const CHIP_H: usize = 22;
pub const CHIP_W: usize = 18;
pub const CHIP_X0: usize = 930;
pub const CHIP_X1: usize = 958;
pub const CHIP_X2: usize = 986;
pub const CHIP_X3: usize = 1090;
pub const CLOCK_W: usize = 80;
pub const CHIP_X_BELL: usize = 1020; // Bell icon between Battery and Clock

/// Total number of layout boxes: 1 launcher + 5 workspaces + 4 chips + 1 bell
pub const LAYOUT_COUNT: usize = 11;

/// ABI version for SilkBar model layout (not the PDX wire protocol).
/// Increment when `SilkBarUpdate` layout or `UpdateKind` discriminants change.
/// v4: added SetActiveApp(8), SetTintAccent(9), SetPaletteState(10); SilkBar
///     struct extended with active_app_sid, accent_tint_idx, palette_* fields.
/// Consumers query `SILKBAR_ABI_VERSION` for PDX compatibility, not this value.
pub const ABI_VERSION: u32 = 4;
/// Must equal `ABI_VERSION`. Checked by `validate_contract()` at startup.
pub const SILK_DE_BAR_ABI_V1: u32 = 4;
pub const SILK_DE_BAR_LAYOUT_V1: u32 = 11;
pub const SILK_DE_BAR_THEME_V1: u32 = 10;
pub const SILK_DE_REQUIRED_WORKSPACE_SLOTS: usize = WORKSPACE_COUNT;
pub const SILK_DE_REQUIRED_CHIP_SLOTS: usize = MAX_CHIPS;
pub const SILK_DE_CONTRACT_MAGIC: u32 = 0x5344_4241; // "SDBA"
pub const SILK_DE_REQUIRED_MODULES: usize = LAYOUT_COUNT;
pub const SILK_DE_REQUIRED_CHIPS: usize = MAX_CHIPS;
pub const SILKBAR_WORKSPACE_COUNT: usize = WORKSPACE_COUNT;
pub const SILKBAR_WORKSPACE_IDX_MIN: u8 = 0;
pub const SILKBAR_WORKSPACE_IDX_MAX: u8 = (WORKSPACE_COUNT - 1) as u8;
pub const SILKBAR_DEFAULT_ACTIVE_WORKSPACE_IDX: u8 = 2;
pub const SILKBAR_CHIP_COUNT: usize = MAX_CHIPS;
pub const SILKBAR_CHIP_IDX_MIN: u8 = 0;
pub const SILKBAR_CHIP_IDX_MAX: u8 = (MAX_CHIPS - 1) as u8;

// ── PDX Protocol Opcodes (v6: wire names exist, no live transport yet) ──────

/// PDX-facing ABI version (u64 for register-width return).
/// Distinct from `ABI_VERSION` (model layout version).
/// v3: new UpdateKind variants 8/9/10 (SetActiveApp, SetTintAccent, SetPaletteState).
/// Returned by `OP_SILKBAR_GET_ABI`. Consumers use this for PDX compat checks.
pub const SILKBAR_ABI_VERSION: u64 = 3;

/// Opcode: ping → returns 0 (connectivity check).
pub const OP_SILKBAR_PING: u64 = 0xF0;
/// Opcode: get ABI version → returns SILKBAR_ABI_VERSION.
pub const OP_SILKBAR_GET_ABI: u64 = 0xF1;
/// Opcode: push SilkBarUpdate (unpacked across arg0/arg1/arg2).
pub const OP_SILKBAR_UPDATE: u64 = 0xF2;

#[derive(Clone, Copy, PartialEq)]
pub enum ModuleSlot {
    Launcher = 0,
    Workspace0 = 1,
    Workspace1 = 2,
    Workspace2 = 3,
    Workspace3 = 4,
    Workspace4 = 5,
    Chip0 = 6,
    Chip1 = 7,
    Chip2 = 8,
    Clock = 9,
    Bell = 10,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ChipSlot {
    Chip0 = 0,
    Chip1 = 1,
    Chip2 = 2,
    Clock = 3,
}

// ── Data Model Types ────────────────────────────────────────────────────────

/// Identity of a module slot in the panel layout.
#[derive(Clone, Copy, PartialEq)]
pub enum Module {
    Launcher,
    Workspaces(usize),
    StatusChip(usize),
    Clock,
    Bell,
}

/// Action triggered when this module slot is clicked.
#[derive(Clone, Copy, PartialEq)]
pub enum Action {
    None,
    OpenLauncher,
    SwitchWorkspace(u8),
    ToggleModule(Module),
    OpenClock,
    OpenBell,
}

/// Geometry + identity + action for one module slot.
#[derive(Clone, Copy)]
pub struct LayoutBox {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub module: Module,
    pub action: Action,
}

#[derive(Clone, Copy)]
pub struct WorkspaceState {
    pub index: u8,
    pub active: bool,
    pub urgent: bool,
}

/// Chip kind (visual variant).
#[derive(Clone, Copy, PartialEq)]
pub enum ChipKind {
    Net,
    Wifi,
    Battery,
    Clock,
}

#[derive(Clone, Copy)]
pub struct ChipState {
    pub kind: ChipKind,
    pub visible: bool,
}

#[derive(Clone, Copy)]
pub struct SilkBar {
    /// Layout boxes define position + identity for every module.
    pub layout: [LayoutBox; LAYOUT_COUNT],
    /// Workspace state data (indexed by Module::Workspaces(i)).
    pub workspaces: [WorkspaceState; WORKSPACE_COUNT],
    /// Chip state data (indexed by Module::StatusChip(i)).
    pub chips: [ChipState; MAX_CHIPS],
    pub clock_hh: u8,
    pub clock_mm: u8,
    pub clock_ss: u8,
    /// Bitmask of selected-window options (OPTION_CLOSE|ZOOM|MINIMIZE|MOVE).
    /// Display-only — no action behavior in V1.
    pub selected_options_mask: u32,
    /// Bell event presence state (V1: aggregate counts from OP_BELL_LIST poll).
    pub bell_state: BellState,
    /// Phase 1 ABI extension (v4): active app / tint / palette state.
    /// Populated by SetActiveApp(8), SetTintAccent(9), SetPaletteState(10).
    pub phase1: SilkBarPhase1Ext,
}

/// Aggregate Bell event state for privacy-safe SilkBar presence display.
/// Populated by SilkBar polling OP_BELL_LIST; consumed by sexdisplay renderer.
/// All counts are clamped to 0..255 (queue max is 16, so this is generous).
#[derive(Clone, Copy)]
pub struct BellState {
    /// Total visible (non-dismissed, non-redacted) events across all lanes.
    pub total_visible: u8,
    /// FullHidden events skipped by privacy gate at poll time.
    pub redacted_count: u8,
    /// Bit 0: bell_available (1 = Bell responded, 0 = unreachable/timeout).
    /// Bits 1-7: reserved (future: mute indicator, overflow flag, etc.).
    pub flags: u8,
    _pad: u8,
}

/// Phase 1 ABI extension (v4): fields added after BellState.
/// Populated by SetActiveApp(8), SetTintAccent(9), SetPaletteState(10) updates.
/// Zero-initialized in DEFAULT_SILK_BAR; old consumers receive 0s (no-op).
#[derive(Clone, Copy)]
pub struct SilkBarPhase1Ext {
    /// Focused surface ID (0 = none). e.g. 200=Linen, 201=Quil, 153=Spindle.
    pub active_app_sid: u32,
    /// Active accent/tint index (0-7, Atlas palette).
    pub accent_tint_idx: u8,
    /// Command palette visible (true = open).
    pub palette_open: bool,
    /// Selected row index in command palette (0-255).
    pub palette_selected: u8,
    /// Available row count in command palette (0-255).
    pub palette_available: u8,
    /// Explicit padding to keep struct naturally aligned.
    _pad: [u8; 1],
}

// ── Theme (v2: 10 semantic tokens) ─────────────────────────────────────────

/// 10-token semantic color palette.
/// Bevel / highlight / shadow colors are derived at runtime via Theme::bevels().
pub struct Theme {
    /// Desktop background gradient top
    pub bg_top: u32,
    /// Desktop background gradient bottom
    pub bg_bottom: u32,
    /// Panel body fill
    pub panel_fill: u32,
    /// Panel outer glow
    pub panel_glow: u32,
    /// Foreground / label color
    pub text: u32,
    /// Inactive / secondary elements
    pub muted: u32,
    /// Active / focused elements
    pub active: u32,
    /// Urgent / alert elements
    pub urgent: u32,
    /// Chip background fill
    pub chip_fill: u32,
    /// Chip border / shadow
    pub chip_border: u32,
}

// ── Appearance Token Index Constants ───────────────────────────────────────
/// Named indices into the `[u32; 8]` appearance token array shared across
/// silk-shell producer and sexdisplay consumer via OP_APPEARANCE_TOKENS (0xFC).
/// These eliminate positional magic numbers and prevent reorder drift (M7).
pub const APPEARANCE_TOKEN_FOCUS_SURFACE:   usize = 0;
pub const APPEARANCE_TOKEN_FRAME_RIM:       usize = 1;
pub const APPEARANCE_TOKEN_FRAME_TOP_BAR:   usize = 2;
pub const APPEARANCE_TOKEN_ACTIVE_TAB:      usize = 3;
pub const APPEARANCE_TOKEN_INACTIVE_TAB:    usize = 4;
pub const APPEARANCE_TOKEN_CLOSE_LIGHT:     usize = 5;
pub const APPEARANCE_TOKEN_MINIMIZE_LIGHT:  usize = 6;
pub const APPEARANCE_TOKEN_ZOOM_LIGHT:      usize = 7;
pub const APPEARANCE_TOKEN_COUNT: usize = 8;

/// Lighten each RGB component by 0x22 (saturating).
#[inline]
pub fn lighten(c: u32) -> u32 {
    let r = ((c & 0xFF) + 0x22).min(0xFF);
    let g = (((c >> 8) & 0xFF) + 0x22).min(0xFF);
    let b = (((c >> 16) & 0xFF) + 0x22).min(0xFF);
    (c & 0xFF000000) | (b << 16) | (g << 8) | r
}

/// Darken each RGB component by 0x44 (saturating).
#[inline]
pub fn darken(c: u32) -> u32 {
    let r = (c & 0xFF).saturating_sub(0x44);
    let g = ((c >> 8) & 0xFF).saturating_sub(0x44);
    let b = ((c >> 16) & 0xFF).saturating_sub(0x44);
    (c & 0xFF000000) | (b << 16) | (g << 8) | r
}

// ── Derived Bevel Palette ──────────────────────────────────────────────────

/// Full set of 14 derived bevel / highlight / shadow colors computed from Theme.
#[derive(Clone, Copy)]
pub struct BevelPalette {
    pub ws_active_highlight: u32,
    pub ws_active_shadow: u32,
    pub ws_inactive_highlight: u32,
    pub ws_inactive_shadow: u32,
    pub chip_highlight: u32,
    pub glow_top_inner: u32,
    pub glow_bottom_inner: u32,
    pub glow_bottom_outer: u32,
    pub panel_border_top: u32,
    pub panel_border_bottom: u32,
    pub launcher_highlight: u32,
    pub launcher_shadow: u32,
    pub launcher_fill: u32,
    pub panel_body: u32,
}

impl Theme {
    /// Derive the full bevel palette from the 10 semantic tokens.
    /// Uses simple lighten/darken operations on the base colors.
    pub fn bevels(&self) -> BevelPalette {
        BevelPalette {
            ws_active_highlight:  lighten(self.active),
            ws_active_shadow:     darken(self.active),
            ws_inactive_highlight: lighten(self.muted),
            ws_inactive_shadow:   darken(self.muted),
            chip_highlight:       lighten(self.chip_fill),
            glow_top_inner:       darken(self.panel_glow),
            glow_bottom_inner:    darken(darken(self.panel_glow)),
            glow_bottom_outer:    darken(darken(darken(self.panel_glow))),
            panel_border_top:     lighten(self.panel_fill),
            panel_border_bottom:  darken(self.panel_fill),
            launcher_highlight:   lighten(self.panel_fill),
            launcher_shadow:      darken(self.panel_fill),
            launcher_fill:        self.panel_fill,
            panel_body:           self.panel_fill,
        }
    }
}

// ── Hit Test ────────────────────────────────────────────────────────────────

/// Returns the Action for the module at (x, y) in the panel, or `Action::None`.
pub fn hit_test_action(bar: &SilkBar, x: usize, y: usize) -> Action {
    if y < PANEL_Y || y >= PANEL_Y + PANEL_H || x < PANEL_X || x >= PANEL_X + PANEL_W {
        return Action::None;
    }
    for i in 0..LAYOUT_COUNT {
        let lb = &bar.layout[i];
        if x >= lb.x && x < lb.x + lb.w && y >= lb.y && y < lb.y + lb.h {
            return lb.action;
        }
    }
    Action::None
}

/// Keep the finite status group inside a supported desktop without changing
/// module identity or overlapping the fixed launcher/workspace group. Both
/// display and shell use this geometry; a narrow unsupported width is explicit.
pub fn for_desktop_width(width:usize)->Option<SilkBar>{
    let mut bar=DEFAULT_SILK_BAR;
    let edge=CHIP_X3.checked_add(CLOCK_W)?.checked_add(16)?;
    let shift=edge.saturating_sub(width);
    let left_end=bar.layout[..6].iter().map(|b|b.x+b.w).max()?;
    if CHIP_X0.checked_sub(shift)?<left_end.checked_add(16)?{return None;}
    for item in &mut bar.layout[6..]{item.x=item.x.checked_sub(shift)?;}
    Some(bar)
}

// ── Update ABI (v4: stable PDX message types) ──────────────────────────────

/// Discriminant for `SilkBarUpdate.kind`.
/// `#[repr(u32)]` so it maps directly across the PDX ABI boundary.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq)]
pub enum UpdateKind {
    SetWorkspaceActive = 0,
    SetWorkspaceUrgent = 1,
    SetChipVisible = 2,
    SetChipKind = 3,
    SetClock = 4,
    SetThemeToken = 5,
    /// Carries selected-window options mask to sexdisplay.
    /// a = options bitmask (OPTION_CLOSE|ZOOM|MINIMIZE|MOVE).
    /// Display-only — no action behavior in V1.
    SetSelectedOptions = 6,
    /// Bell presence: a=packed (total_visible|redacted<<8|flags<<16), b=0.
    SetBellPresence = 7,
    /// Active app surface ID: a=surface_id (0=none), index=0, b=0.
    /// silk-shell producer sends FOCUSED_SURFACE_ID on focus/scene change.
    SetActiveApp = 8,
    /// Active accent/tint index: a=accent_idx (0-7), index=0, b=0.
    /// silk-shell producer sends ACTIVE_TINT_IDX on focus/scene change.
    SetTintAccent = 9,
    /// Command palette state: a=packed(open<<0 | selected<<1 | available<<9), b=0.
    /// silk-shell producer sends on palette toggle/selection change.
    SetPaletteState = 10,
}

/// ABI-stable update message for mutating a `SilkBar` from a PDX caller.
/// Layout is `#[repr(C)]` with natural alignment padding.
///
/// # ABI Contract
/// - Total size must be 16 bytes (asserted at compile time).
/// - `kind` maps to `UpdateKind` discriminants.
/// - Invalid `kind` or out-of-bounds `index` are silently rejected by `apply_update`.
/// - The queue producer (future PDX server) pushes `SilkBarUpdate` values.
///   The queue consumer (sexdisplay render loop) drains and applies them.
///   There is no overwrite: a full queue returns `false` from `push()`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SilkBarUpdate {
    /// `UpdateKind` discriminant (passed as u32 for C-ABI safety).
    pub kind: u32,
    /// Slot index (workspace index, chip index, or theme token index).
    pub index: u8,
    /// Primary value field.
    pub a: u32,
    /// Secondary value field (e.g. clock minutes).
    pub b: u32,
}

/// Compile-time size check for the PDX ABI struct.
pub const SILKBAR_UPDATE_SIZE: usize = core::mem::size_of::<SilkBarUpdate>();

impl SilkBarUpdate {
    /// Construct an update from raw fields.
    pub const fn new(kind: u32, index: u8, a: u32, b: u32) -> Self {
        SilkBarUpdate { kind, index, a, b }
    }
}

/// Convenience wrapper: construct a `SilkBarUpdate` from raw fields and apply it.
pub fn apply_raw_update(bar: &mut SilkBar, kind: u32, index: u8, a: u32, b: u32) -> bool {
    let update = SilkBarUpdate { kind, index, a, b };
    apply_update(bar, update)
}

/// Apply a `SilkBarUpdate` to a mutable `SilkBar`.
///
/// Returns `true` on success, `false` if the update kind is unknown,
/// the index is out of bounds, or the payload is invalid.
/// Invalid updates are silently ignored (no panic, no undefined state).
///
/// # Producer / Consumer Model
/// - **Producer** (future PDX server): pushes `SilkBarUpdate` values via `push()`.
/// - **Consumer** (sexdisplay render loop): drains the queue via `drain_into()`
///   once per frame, which calls `apply_update()` for each entry.
/// - **No overwrite**: a full queue rejects new pushes.
/// - **No crash**: malformed updates are silently dropped.
pub fn apply_update(bar: &mut SilkBar, update: SilkBarUpdate) -> bool {
    match update.kind {
        0 => {
            // SetWorkspaceActive: index=ws_idx, a=0|1
            let idx = update.index as usize;
            if idx >= WORKSPACE_COUNT {
                return false;
            }
            bar.workspaces[idx].active = update.a != 0;
            true
        }
        1 => {
            // SetWorkspaceUrgent: index=ws_idx, a=0|1
            let idx = update.index as usize;
            if idx >= WORKSPACE_COUNT {
                return false;
            }
            bar.workspaces[idx].urgent = update.a != 0;
            true
        }
        2 => {
            // SetChipVisible: index=chip_idx, a=0|1
            let idx = update.index as usize;
            if idx >= MAX_CHIPS {
                return false;
            }
            bar.chips[idx].visible = update.a != 0;
            true
        }
        3 => {
            // SetChipKind: index=chip_idx, a=ChipKind as u32
            let idx = update.index as usize;
            if idx >= MAX_CHIPS {
                return false;
            }
            let kind = match update.a {
                0 => ChipKind::Net,
                1 => ChipKind::Wifi,
                2 => ChipKind::Battery,
                3 => ChipKind::Clock,
                _ => return false,
            };
            bar.chips[idx].kind = kind;
            true
        }
        4 => {
            // SetClock: a=hh (0-23), b packed = (mm << 8) | ss
            let hh = update.a.min(23) as u8;
            let mm = (update.b >> 8).min(59) as u8;
            let ss = (update.b as u8).min(59);
            bar.clock_hh = hh;
            bar.clock_mm = mm;
            bar.clock_ss = ss;
            true
        }
        5 => {
            // GATE: SetThemeToken (kind=5) is intentionally no-op in V1.
            // Theme tokens are delivered via OP_APPEARANCE_TOKENS (0xFC) to sexdisplay.
            // This slot is reserved for future in-model theme storage.
            false
        }
        6 => {
            // SetSelectedOptions: a = options bitmask (display-only, no action).
            bar.selected_options_mask = update.a;
            true
        }
        7 => {
            // SetBellPresence: a = packed (total_visible|redacted_count<<8|flags<<16)
            bar.bell_state.total_visible = update.a as u8;
            bar.bell_state.redacted_count = (update.a >> 8) as u8;
            bar.bell_state.flags = (update.a >> 16) as u8;
            true
        }
        8 => {
            // SetActiveApp: a = surface_id (0 = none)
            bar.phase1.active_app_sid = update.a;
            true
        }
        9 => {
            // SetTintAccent: a = accent_idx (0-7)
            if update.a > 7 {
                return false;
            }
            bar.phase1.accent_tint_idx = update.a as u8;
            true
        }
        10 => {
            // SetPaletteState: a = packed(open<<0 | selected<<1 | available<<9)
            bar.phase1.palette_open      = (update.a & 1) != 0;
            bar.phase1.palette_selected  = ((update.a >> 1) & 0xFF) as u8;
            bar.phase1.palette_available = ((update.a >> 9) & 0xFF) as u8;
            true
        }
        _ => false,
    }
}

// ── Update Queue (v5: fixed ring buffer) ───────────────────────────────────

/// Capacity of the fixed-size update ring buffer.
pub const UPDATE_QUEUE_CAP: usize = 32;

/// Lock-free ring buffer of `SilkBarUpdate` entries.
/// `#[repr(C)]` for stable ABI when shared across PDX boundary.
#[repr(C)]
pub struct SilkBarUpdateQueue {
    head: u32,
    tail: u32,
    updates: [SilkBarUpdate; UPDATE_QUEUE_CAP],
}

impl SilkBarUpdateQueue {
    /// Create an empty queue (all slots available).
    pub fn empty() -> Self {
        SilkBarUpdateQueue {
            head: 0,
            tail: 0,
            updates: [SilkBarUpdate { kind: 0, index: 0, a: 0, b: 0 }; UPDATE_QUEUE_CAP],
        }
    }

    /// Enqueue an update.
    /// Returns `false` if the queue is full (no overwrite).
    pub fn push(&mut self, update: SilkBarUpdate) -> bool {
        let cap = UPDATE_QUEUE_CAP as u32;
        let next_tail = (self.tail + 1) % cap;
        if next_tail == self.head {
            return false; // full
        }
        self.updates[self.tail as usize] = update;
        self.tail = next_tail;
        true
    }

    /// Dequeue the oldest update, or `None` if empty.
    pub fn pop(&mut self) -> Option<SilkBarUpdate> {
        if self.head == self.tail {
            return None; // empty
        }
        let cap = UPDATE_QUEUE_CAP as u32;
        let update = self.updates[self.head as usize];
        self.head = (self.head + 1) % cap;
        Some(update)
    }

    /// Drain all pending updates into `bar`, applying each via `apply_update`.
    /// Returns the number of successfully applied updates.
    pub fn drain_into(&mut self, bar: &mut SilkBar) -> u32 {
        let mut count = 0u32;
        while let Some(update) = self.pop() {
            if apply_update(bar, update) {
                count += 1;
            }
        }
        count
    }
}

// ── ABI Assertions (compile time) ──────────────────────────────────────────

/// `SilkBarUpdate` must be exactly 16 bytes for a stable PDX ABI.
const _: () = assert!(core::mem::size_of::<SilkBarUpdate>() == 16);

/// Ring-buffer capacity must be 32 (power of two simplifies future mask-based indexing).
const _: () = assert!(UPDATE_QUEUE_CAP == 32);

/// `ABI_VERSION` must be non-zero.
const _: () = assert!(ABI_VERSION > 0);

// ── Invariant Tests (runtime) ──────────────────────────────────────────────

/// Run all queue invariant tests.
/// Returns `true` if all pass.
///
/// Tests:
/// 1. Empty queue returns `None` on pop.
/// 2. Push then pop returns the same update.
/// 3. Full queue rejects additional pushes.
/// 4. `drain_into` applies a clock update correctly.
pub fn validate_invariants() -> bool {
    // 1. Empty queue
    let mut q = SilkBarUpdateQueue::empty();
    if q.pop().is_some() {
        return false;
    }

    // 2. Push then pop identity
    let u = SilkBarUpdate { kind: 4, index: 0, a: 10, b: 30 };
    if !q.push(u) {
        return false;
    }
    match q.pop() {
        Some(p) => {
            if p.kind != 4 || p.index != 0 || p.a != 10 || p.b != 30 {
                return false;
            }
        }
        None => return false,
    }

    // 3. Full queue rejects overwrite
    let mut q2 = SilkBarUpdateQueue::empty();
    let dummy = SilkBarUpdate { kind: 0, index: 0, a: 0, b: 0 };
    // Fill to capacity - 1 (one slot always reserved for empty-vs-full distinction)
    for _ in 0..UPDATE_QUEUE_CAP - 1 {
        if !q2.push(dummy) {
            return false;
        }
    }
    // Next push must fail
    if q2.push(dummy) {
        return false;
    }

    // 4. drain_into applies clock update
    let mut q3 = SilkBarUpdateQueue::empty();
    let clock = SilkBarUpdate { kind: 4, index: 0, a: 15, b: (45 << 8) | 30 };
    if !q3.push(clock) {
        return false;
    }
    let mut bar = DEFAULT_SILK_BAR;
    let count = q3.drain_into(&mut bar);
    if count != 1 {
        return false;
    }
    if bar.clock_hh != 15 || bar.clock_mm != 45 || bar.clock_ss != 30 {
        return false;
    }

    true
}

/// Startup validation gate — call from silkbar and sexdisplay at `_start`.
/// Returns 0 on pass, nonzero reason code on failure:
///   1 = ABI/layout contract check failed
///   2 = deterministic vector check failed
pub fn validate_silkbar_contract() -> u32 {
    if !validate_contract() {
        return 1;
    }
    if !validate_deterministic_vectors() {
        return 2;
    }
    0
}

/// Contract gate for Silk DE top strip.
/// Use this at producer/consumer startup to reject ABI/layout drift early.
pub fn validate_contract() -> bool {
    if SILK_DE_CONTRACT_MAGIC == 0 {
        return false;
    }
    if ABI_VERSION != SILK_DE_BAR_ABI_V1 {
        return false;
    }
    if LAYOUT_COUNT as u32 != SILK_DE_BAR_LAYOUT_V1 {
        return false;
    }
    if SILK_DE_BAR_THEME_V1 != 10 {
        return false;
    }
    if WORKSPACE_COUNT != SILK_DE_REQUIRED_WORKSPACE_SLOTS {
        return false;
    }
    if MAX_CHIPS != SILK_DE_REQUIRED_CHIP_SLOTS {
        return false;
    }
    if LAYOUT_COUNT != SILK_DE_REQUIRED_MODULES {
        return false;
    }
    if MAX_CHIPS != SILK_DE_REQUIRED_CHIPS {
        return false;
    }
    if DEFAULT_SILK_BAR.layout.len() != SILK_DE_REQUIRED_MODULES {
        return false;
    }
    if DEFAULT_SILK_BAR.chips.len() != SILK_DE_REQUIRED_CHIPS {
        return false;
    }
    if SILKBAR_WORKSPACE_COUNT != WORKSPACE_COUNT {
        return false;
    }
    if SILKBAR_CHIP_COUNT != MAX_CHIPS {
        return false;
    }
    if SILKBAR_WORKSPACE_IDX_MIN != 0 || SILKBAR_CHIP_IDX_MIN != 0 {
        return false;
    }
    if SILKBAR_WORKSPACE_IDX_MAX as usize + 1 != SILKBAR_WORKSPACE_COUNT {
        return false;
    }
    if SILKBAR_CHIP_IDX_MAX as usize + 1 != SILKBAR_CHIP_COUNT {
        return false;
    }
    if SILKBAR_DEFAULT_ACTIVE_WORKSPACE_IDX > SILKBAR_WORKSPACE_IDX_MAX {
        return false;
    }
    if SILKBAR_UPDATE_SIZE != 16 {
        return false;
    }
    if UPDATE_QUEUE_CAP != 32 {
        return false;
    }
    if DEFAULT_THEME.bg_top == 0 || DEFAULT_THEME.bg_bottom == 0
        || DEFAULT_THEME.panel_fill == 0 || DEFAULT_THEME.panel_glow == 0
        || DEFAULT_THEME.text == 0 || DEFAULT_THEME.active == 0
        || DEFAULT_THEME.urgent == 0 || DEFAULT_THEME.chip_fill == 0
        || DEFAULT_THEME.chip_border == 0
    {
        return false;
    }
    // Keep renderer-safe top strip geometry bounded and non-zero.
    if PANEL_W == 0 || PANEL_H == 0 || PANEL_W > 4096 || PANEL_H > 256 {
        return false;
    }
    // F4: ChipSlot discriminants must match CHIP_SLOTS array indices.
    // If these drift, the renderer chip_color() CHIP_SLOTS array will
    // index into wrong ModuleSlot positions. Bell is excluded — it is a
    // ModuleSlot (discriminant 10), not a ChipSlot.
    if ChipSlot::Chip0 as usize != 0 { return false; }
    if ChipSlot::Chip1 as usize != 1 { return false; }
    if ChipSlot::Chip2 as usize != 2 { return false; }
    if ChipSlot::Clock as usize != 3 { return false; }
    true
}

pub fn contract_fingerprint() -> u64 {
    let mut fp = SILK_DE_CONTRACT_MAGIC as u64;
    fp ^= (SILK_DE_BAR_ABI_V1 as u64) << 32;
    fp ^= (SILK_DE_BAR_LAYOUT_V1 as u64) << 24;
    fp ^= (SILK_DE_BAR_THEME_V1 as u64) << 16;
    fp ^= (SILK_DE_REQUIRED_WORKSPACE_SLOTS as u64) << 8;
    fp ^= SILK_DE_REQUIRED_CHIP_SLOTS as u64;
    fp ^= (SILKBAR_UPDATE_SIZE as u64) << 40;
    fp ^= (UPDATE_QUEUE_CAP as u64) << 48;
    fp
}

/// Deterministic vector validation for Silk DE top-strip model behavior.
/// This is a headless gate for update semantics before renderer/runtime boot.
pub fn validate_deterministic_vectors() -> bool {
    let mut bar = DEFAULT_SILK_BAR;
    let mut q = SilkBarUpdateQueue::empty();

    let vectors = [
        // Workspace transitions
        SilkBarUpdate::new(UpdateKind::SetWorkspaceActive as u32, 0, 1, 0),
        SilkBarUpdate::new(UpdateKind::SetWorkspaceActive as u32, 2, 0, 0),
        SilkBarUpdate::new(UpdateKind::SetWorkspaceUrgent as u32, 1, 1, 0),
        // Chip transitions
        SilkBarUpdate::new(UpdateKind::SetChipVisible as u32, 2, 0, 0),
        SilkBarUpdate::new(UpdateKind::SetChipKind as u32, 1, ChipKind::Battery as u32, 0),
        // Clock transition (rollover edge sample)
        SilkBarUpdate::new(UpdateKind::SetClock as u32, 0, 23, (59u32 << 8) | 58u32),
        // Selected-window options
        SilkBarUpdate::new(UpdateKind::SetSelectedOptions as u32, 0, OPTION_MOVE, 0),
        // Phase 1 ABI extension (v4): new variants
        SilkBarUpdate::new(UpdateKind::SetActiveApp as u32, 0, 200, 0),  // Linen SID
        SilkBarUpdate::new(UpdateKind::SetTintAccent as u32, 0, 3, 0),    // accent index 3
        SilkBarUpdate::new(UpdateKind::SetPaletteState as u32, 0,
            1 | (2 << 1) | (7 << 9), 0),  // open=true, selected=2, available=7
    ];

    for update in vectors {
        if !q.push(update) {
            return false;
        }
    }

    let applied = q.drain_into(&mut bar);
    if applied != 10 {
        return false;
    }

    if !bar.workspaces[0].active || bar.workspaces[2].active {
        return false;
    }
    if !bar.workspaces[1].urgent {
        return false;
    }
    if bar.chips[2].visible {
        return false;
    }
    if bar.chips[1].kind != ChipKind::Battery {
        return false;
    }
    if bar.clock_hh != 23 || bar.clock_mm != 59 || bar.clock_ss != 58 {
        return false;
    }
    if bar.selected_options_mask != OPTION_MOVE {
        return false;
    }
    if bar.phase1.active_app_sid != 200 {
        return false;
    }
    if bar.phase1.accent_tint_idx != 3 {
        return false;
    }
    if !bar.phase1.palette_open {
        return false;
    }
    if bar.phase1.palette_selected != 2 {
        return false;
    }
    if bar.phase1.palette_available != 7 {
        return false;
    }

    true
}

// ── Default Instances ───────────────────────────────────────────────────────

pub const DEFAULT_SILK_BAR: SilkBar = SilkBar {
    layout: [
        LayoutBox { x: LAUNCHER_X, y: LAUNCHER_Y, w: LAUNCHER_W, h: LAUNCHER_H, module: Module::Launcher, action: Action::OpenLauncher },
        LayoutBox { x: WS_X0, y: WS_Y, w: WS_INACTIVE_W, h: WS_H, module: Module::Workspaces(0), action: Action::SwitchWorkspace(1) },
        LayoutBox { x: WS_X1, y: WS_Y, w: WS_INACTIVE_W, h: WS_H, module: Module::Workspaces(1), action: Action::SwitchWorkspace(2) },
        LayoutBox { x: WS_X2, y: WS_Y, w: WS_ACTIVE_W,   h: WS_H, module: Module::Workspaces(2), action: Action::SwitchWorkspace(3) },
        LayoutBox { x: WS_X3, y: WS_Y, w: WS_INACTIVE_W, h: WS_H, module: Module::Workspaces(3), action: Action::SwitchWorkspace(4) },
        LayoutBox { x: WS_X4, y: WS_Y, w: WS_INACTIVE_W, h: WS_H, module: Module::Workspaces(4), action: Action::SwitchWorkspace(5) },
        LayoutBox { x: CHIP_X0, y: CHIP_Y, w: CHIP_W, h: CHIP_H, module: Module::StatusChip(0), action: Action::ToggleModule(Module::StatusChip(0)) },
        LayoutBox { x: CHIP_X1, y: CHIP_Y, w: CHIP_W, h: CHIP_H, module: Module::StatusChip(1), action: Action::ToggleModule(Module::StatusChip(1)) },
        LayoutBox { x: CHIP_X2, y: CHIP_Y, w: CHIP_W, h: CHIP_H, module: Module::StatusChip(2), action: Action::ToggleModule(Module::StatusChip(2)) },
        LayoutBox { x: CHIP_X3, y: CHIP_Y, w: CLOCK_W, h: CHIP_H, module: Module::Clock,        action: Action::OpenClock },
        LayoutBox { x: CHIP_X_BELL, y: CHIP_Y, w: CHIP_W, h: CHIP_H, module: Module::Bell,      action: Action::OpenBell },
    ],
    workspaces: [
        WorkspaceState { index: 1, active: false, urgent: false },
        WorkspaceState { index: 2, active: false, urgent: false },
        WorkspaceState { index: 3, active: true,  urgent: false },
        WorkspaceState { index: 4, active: false, urgent: false },
        WorkspaceState { index: 5, active: false, urgent: false },
    ],
    chips: [
        ChipState { kind: ChipKind::Net,     visible: true },
        ChipState { kind: ChipKind::Wifi,    visible: true },
        ChipState { kind: ChipKind::Battery, visible: true },
        ChipState { kind: ChipKind::Clock,   visible: true },
    ],
    clock_hh: 10,
    clock_mm: 42,
    clock_ss: 0,
    selected_options_mask: 0,
    bell_state: BellState {
        total_visible: 0,
        redacted_count: 0,
        flags: 0,
        _pad: 0,
    },
    phase1: SilkBarPhase1Ext {
        active_app_sid: 0,
        accent_tint_idx: 0,
        palette_open: false,
        palette_selected: 0,
        palette_available: 0,
        _pad: [0],
    },
};

pub const DEFAULT_THEME: Theme = Theme {
    bg_top:      0x00081424,
    bg_bottom:   0x001E1E2E,
    panel_fill:  0x00313244,
    panel_glow:  0x0045475A,
    text:        0x00CDD6F4,
    muted:       0x00304068,
    active:      0x00A8A0FF,
    urgent:      0x00FF6666,
    chip_fill:   0x0089B4FA,
    chip_border: 0x006F86A8,
};
