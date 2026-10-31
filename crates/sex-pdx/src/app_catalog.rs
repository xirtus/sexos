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
}

const fn quil(ordinal: u8, target: u32, surface: u64, shell_surface: Option<u64>, lifecycle_slot: usize, close_slot: u64, activation_slot: Option<usize>) -> Incarnation {
    Incarnation { app: App::Quil, ordinal, target, surface, shell_surface, lifecycle_slot, close_slot, activation_slot }
}

/// Every BOOT-reserved app incarnation, in activation order per app.
pub const INCARNATIONS: [Incarnation; 6] = [
    quil(0, 9, 156, Some(201), 38, 41, None),
    Incarnation { app: App::Spindle, ordinal: 0, target: 12, surface: 154, shell_surface: Some(153), lifecycle_slot: 39, close_slot: 42, activation_slot: None },
    Incarnation { app: App::Browser, ordinal: 0, target: 15, surface: 300, shell_surface: None, lifecycle_slot: 40, close_slot: 43, activation_slot: None },
    quil(1, 16, 301, None, 45, 46, Some(44)),
    Incarnation { app: App::Spindle, ordinal: 1, target: 17, surface: 302, shell_surface: Some(306), lifecycle_slot: 50, close_slot: 51, activation_slot: Some(49) },
    quil(2, 18, 307, None, 54, 56, Some(55)),
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
