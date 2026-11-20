// SPDX-License-Identifier: GPL-3.0-only
//! Polyamour: SexOS's native 3D suite.
//!
//! The baseline is Blender's own workspace: the same top bar with File/Edit/
//! Render/Window/Help and workspace tabs, the same default workspaces and area
//! layouts, the same editors with the same headers, the same keymap and the
//! same dark theme, so a Blender user can start working at once. Blender is
//! GPL-2.0-or-later; its layouts, keymap and operator names are the reference.
//! The geometry, rendering and file work is `sex-3d`.
//!
//! Everything here is plain Rust drawing into a pixel buffer, so host tests
//! drive the whole window with synthetic input and read back its pixels.
pub mod anim;
pub mod app;
pub mod compositing;
#[cfg(not(target_os = "hermit"))]
pub mod desktop;
pub mod doc;
pub mod draw;
pub mod edit;
pub mod edit_ops;
pub mod gpencil;
pub mod editors;
pub mod input;
pub mod nodetree;
pub mod omatools;
pub mod persist;
pub mod python;
pub mod physics;
pub mod rig;
pub mod sculpt;
pub mod seqedit;
pub mod screen;
pub mod settings;
pub mod theme;
pub mod transform;
pub mod ui;
pub mod uvedit;

pub use app::App;

/// Blend two 0x00RRGGBB colours; `alpha` 0..255 is `over`'s weight.
pub fn draw_mix(over: u32, under: u32, alpha: u32) -> u32 {
    sex_office::mix(under, over, alpha)
}
