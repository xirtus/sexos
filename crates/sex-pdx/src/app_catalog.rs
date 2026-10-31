// SPDX-License-Identifier: GPL-3.0-only
//! The BOOT app incarnation catalog: one table every component reads instead
//! of hand-wiring each incarnation. It describes what BOOT already grants; it
//! is data, never authority. Each service still holds and checks its own
//! BOOT capabilities, and the kernel's grants remain the truth that a host
//! check compares this table against.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App { Quil, Spindle, Browser }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Incarnation {
    pub app: App,
    /// Ordinal within the app: 0 is the boot incarnation, then successors.
    pub ordinal: u8,
    /// Domain and main task (equal for every BOOT app incarnation).
    pub target: u32,
    /// The app's own content surface.
    pub surface: u64,
    /// Shell's frame surface for the app, when distinct from `surface`.
    pub shell_surface: Option<u64>,
    /// Shell's lifecycle capability slot for this incarnation.
    pub lifecycle_slot: usize,
    /// Shell's close route slot.
    pub close_slot: u64,
    /// Shell's activation slot that brings this incarnation up (successors).
    pub activation_slot: Option<usize>,
    /// Shell's route for view notices and document controls to this app.
    pub route_slot: u64,
    /// Further content surfaces the app owns inside its frame (viewports).
    pub children: &'static [u64],
}

impl Incarnation {
    /// The Shell frame that holds this incarnation's content.
    pub const fn frame(&self) -> u64 {
        match self.shell_surface { Some(frame) => frame, None => self.surface }
    }
}

const fn quil(ordinal: u8, target: u32, surface: u64, shell_surface: Option<u64>, lifecycle_slot: usize, close_slot: u64, activation_slot: Option<usize>, route_slot: u64) -> Incarnation {
    Incarnation { app: App::Quil, ordinal, target, surface, shell_surface, lifecycle_slot, close_slot, activation_slot, route_slot, children: &[] }
}

/// Every BOOT-reserved app incarnation, in activation order per app.
pub const INCARNATIONS: [Incarnation; 6] = [
    quil(0, 9, 156, Some(201), 38, 41, None, 11),
    Incarnation { app: App::Spindle, ordinal: 0, target: 12, surface: 154, shell_surface: Some(153), lifecycle_slot: 39, close_slot: 42, activation_slot: None, route_slot: 14, children: &[160, 161, 162] },
    // Browser's document notices share its BOOT close route.
    Incarnation { app: App::Browser, ordinal: 0, target: 15, surface: 300, shell_surface: None, lifecycle_slot: 40, close_slot: 43, activation_slot: None, route_slot: 43, children: &[] },
    quil(1, 16, 301, None, 45, 46, Some(44), 47),
    Incarnation { app: App::Spindle, ordinal: 1, target: 17, surface: 302, shell_surface: Some(306), lifecycle_slot: 50, close_slot: 51, activation_slot: Some(49), route_slot: 52, children: &[303, 304, 305] },
    quil(2, 18, 307, None, 54, 56, Some(55), 57),
];

pub fn by_target(target: u32) -> Option<&'static Incarnation> {
    INCARNATIONS.iter().find(|i| i.target == target)
}

/// The incarnation whose exact catalog surface pair this is.
pub fn by_surface(target: u32, surface: u64) -> Option<&'static Incarnation> {
    INCARNATIONS.iter().find(|i| i.target == target && i.surface == surface)
}

pub fn is_document_app(target: u32) -> bool {
    by_target(target).is_some_and(|i| i.app == App::Quil)
}

/// Document-view holders in Shell's fixed delivery order: every Quil
/// incarnation by ordinal, then Browser.
pub fn document_views() -> impl Iterator<Item = &'static Incarnation> {
    INCARNATIONS.iter().filter(|i| i.app == App::Quil).chain(INCARNATIONS.iter().filter(|i| i.app == App::Browser))
}
pub fn document_view_index(target: u32) -> Option<usize> {
    document_views().position(|i| i.target == target)
}
pub fn document_view_frame(frame: u64) -> Option<(usize, &'static Incarnation)> {
    document_views().enumerate().find(|(_, i)| i.frame() == frame)
}
/// Spindle incarnations by ordinal.
pub fn spindle_views() -> impl Iterator<Item = &'static Incarnation> {
    INCARNATIONS.iter().filter(|i| i.app == App::Spindle)
}

/// Whether `sid` is content of the incarnation with this exact catalog pair.
pub fn is_content(target: u32, surface: u64, sid: u64) -> bool {
    by_surface(target, surface).is_some_and(|i| i.surface == sid || i.children.contains(&sid))
}

/// One bit per incarnation (its catalog index) for an owner's content
/// surface; 0 for anything else. Const so the per-pixel compositor pays no
/// runtime scan.
pub const fn native_pixel_bit(owner: u32, sid: u64) -> u8 {
    let mut index = 0;
    while index < INCARNATIONS.len() {
        let i = &INCARNATIONS[index];
        if i.target == owner {
            if i.surface == sid { return 1 << index; }
            let mut c = 0;
            while c < i.children.len() {
                if i.children[c] == sid { return 1 << index; }
                c += 1;
            }
            return 0;
        }
        index += 1;
    }
    0
}

/// The app an exact incarnation domain belongs to (const for table setup).
pub const fn app_of(target: u32) -> Option<App> {
    let mut index = 0;
    while index < INCARNATIONS.len() {
        if INCARNATIONS[index].target == target { return Some(INCARNATIONS[index].app); }
        index += 1;
    }
    None
}
pub const fn is_quil(target: u32) -> bool { matches!(app_of(target), Some(App::Quil)) }
pub const fn is_spindle(target: u32) -> bool { matches!(app_of(target), Some(App::Spindle)) }
pub const fn is_browser(target: u32) -> bool { matches!(app_of(target), Some(App::Browser)) }
/// Whether this incarnation's content sits inside a separate Shell frame.
pub const fn has_shell_frame(target: u32) -> bool {
    let mut index = 0;
    while index < INCARNATIONS.len() {
        if INCARNATIONS[index].target == target { return INCARNATIONS[index].shell_surface.is_some(); }
        index += 1;
    }
    false
}

/// Every incarnation domain, for the kernel's BOOT plan.
pub const TARGETS: [u32; INCARNATIONS.len()] = {
    let mut out = [0; INCARNATIONS.len()];
    let mut i = 0;
    while i < INCARNATIONS.len() { out[i] = INCARNATIONS[i].target; i += 1; }
    out
};
/// Every (domain, content surface) pair, for the kernel's resource catalog.
pub const CONTENT: [(u32, u64); INCARNATIONS.len()] = {
    let mut out = [(0, 0); INCARNATIONS.len()];
    let mut i = 0;
    while i < INCARNATIONS.len() { out[i] = (INCARNATIONS[i].target, INCARNATIONS[i].surface); i += 1; }
    out
};
/// The boot (ordinal 0) incarnation of an app.
pub fn boot_incarnation(app: App) -> &'static Incarnation {
    INCARNATIONS.iter().find(|i| i.app == app && i.ordinal == 0).expect("catalog app without a boot incarnation")
}
/// The incarnation this one succeeds (same app, previous ordinal).
pub fn predecessor(i: &Incarnation) -> Option<&'static Incarnation> {
    let ordinal = i.ordinal.checked_sub(1)?;
    INCARNATIONS.iter().find(|p| p.app == i.app && p.ordinal == ordinal)
}
/// Quil incarnation targets in ordinal order (document buffer clients).
pub const QUIL_TARGETS: [u32; 3] = [9, 16, 18];
const _: () = {
    let mut i = 0; let mut q = 0;
    while i < INCARNATIONS.len() {
        if matches!(INCARNATIONS[i].app, App::Quil) { assert!(INCARNATIONS[i].target == QUIL_TARGETS[q]); q += 1; }
        i += 1;
    }
    assert!(q == QUIL_TARGETS.len());
};
