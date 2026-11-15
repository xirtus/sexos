// SPDX-License-Identifier: GPL-3.0-only
//! Quil's engine, apart from any window, so the host tests prove it:
//!
//! - [`buffer`]: a piece table over the file as opened, read lazily, so files of any
//!   size open at once and edit without copying.
//! - [`editor`]: many selections at once, undo and redo, line commands, clipboard.
//! - [`vim`] and [`keymap`]: Vim mode, VS Code and Emacs keys, named commands.
//! - [`fuzzy`]: the scorer behind Go to File and the command palette.
//! - [`project`]: folder walking, the file tree, project search and replace.
//! - [`highlight`] and [`workspace`]: colouring and tabs.
#![no_std]
extern crate alloc;

pub mod buffer;
pub mod editor;
pub mod fuzzy;
pub mod highlight;
pub mod keymap;
pub mod project;
pub mod vim;
pub mod workspace;

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;
