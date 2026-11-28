// SPDX-License-Identifier: GPL-3.0-only
//! Interactive Ratatui-inspired TUI apps for Spindle (`bars`, `btop`, `yazi`).
//! Pure `#![no_std]` + `alloc` ANSI renderers driven over [`crate::host::Host`] so
//! host tests and the SexOS terminal share the exact same implementation.

pub mod bars;
pub mod btop;
pub mod widgets;
pub mod yazi;

use alloc::string::String;
use crate::host::Host;

pub use bars::{BarsApp, BarsMode};
pub use btop::BtopApp;
pub use yazi::YaziApp;

/// Normalized key events understood by interactive TUI views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TuiKey {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Enter,
    Backspace,
    Tab,
    BackTab,
    Escape,
    Home,
    End,
    PageUp,
    PageDown,
    CtrlC,
}

/// Terminal action requested when a TUI exits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuiExit {
    Done,
    Cd(String),
    Cancel,
}

/// Frame bytes plus optional termination state returned from `key` or `tick`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TuiOutput {
    pub bytes: String,
    pub exit: Option<TuiExit>,
}

impl TuiOutput {
    pub fn none() -> Self {
        Self { bytes: String::new(), exit: None }
    }
    pub fn frame(bytes: String) -> Self {
        Self { bytes, exit: None }
    }
    pub fn finish(bytes: String, exit: TuiExit) -> Self {
        Self { bytes, exit: Some(exit) }
    }
}

/// Unified active TUI runner attached to a Spindle pane.
pub enum ActiveTui {
    Bars(BarsApp),
    Btop(BtopApp),
    Yazi(YaziApp),
}

impl ActiveTui {
    pub fn name(&self) -> &'static str {
        match self {
            ActiveTui::Bars(b) => b.name(),
            ActiveTui::Btop(_) => "btop",
            ActiveTui::Yazi(_) => "yazi",
        }
    }

    pub fn is_alt_screen(&self) -> bool {
        !matches!(self, ActiveTui::Bars(_))
    }

    pub fn init(&mut self, host: &dyn Host, cols: usize, rows: usize, now_ms: u64) -> TuiOutput {
        match self {
            ActiveTui::Bars(b) => b.init(cols, now_ms),
            ActiveTui::Btop(b) => b.init(host, cols, rows, now_ms),
            ActiveTui::Yazi(y) => y.init(cols, rows),
        }
    }

    pub fn tick(&mut self, host: &dyn Host, cols: usize, rows: usize, now_ms: u64) -> TuiOutput {
        match self {
            ActiveTui::Bars(b) => b.tick(cols, now_ms),
            ActiveTui::Btop(b) => b.tick(host, cols, rows, now_ms),
            ActiveTui::Yazi(y) => y.tick(cols, rows, now_ms),
        }
    }

    pub fn key(&mut self, key: TuiKey, host: &dyn Host, cols: usize, rows: usize, now_ms: u64) -> TuiOutput {
        match self {
            ActiveTui::Bars(b) => b.key(key, cols),
            ActiveTui::Btop(b) => b.key(key, host, cols, rows, now_ms),
            ActiveTui::Yazi(y) => y.key(key, host, cols, rows),
        }
    }

    pub fn resize(&mut self, host: &dyn Host, cols: usize, rows: usize, now_ms: u64) -> TuiOutput {
        match self {
            ActiveTui::Bars(b) => b.redraw(cols),
            ActiveTui::Btop(b) => TuiOutput::frame(b.render(host, cols, rows, now_ms)),
            ActiveTui::Yazi(y) => TuiOutput::frame(y.render(cols, rows)),
        }
    }
}
