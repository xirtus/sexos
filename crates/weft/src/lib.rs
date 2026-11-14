// SPDX-License-Identifier: GPL-3.0-only
//! Weft: the SexOS UI toolkit.
//!
//! A window's interface is a [`Ui`]: a tree of retained widgets laid out by taffy
//! (flexbox and grid), painted with tiny-skia shapes and sex-font text into the
//! window's pixels, and described to screen readers as an AccessKit tree. Programs
//! build widgets once, feed [`input::Event`]s to [`Ui::handle`], act on the
//! [`Response`]s that come back, and call [`Ui::paint`] when [`Ui::needs_paint`].
//!
//! Every program gets the same keyboard model (Tab order, arrows, Enter, Escape, F10,
//! Shift+F10, Ctrl+Shift+P), menus, a command palette, dialogs, toasts, tooltips,
//! document tabs, themes (light, dark, high contrast, any accent, any scale) and
//! translations from Fluent files. See `docs/handoff/WEFT_2026_11_14.md`.
#![no_std]
extern crate alloc;

use alloc::string::String;

pub mod a11y;
mod build;
mod draw;
pub mod edit;
mod events;
pub mod geom;
pub mod i18n;
pub mod icons;
pub mod input;
pub mod menu;
mod overlays;
pub mod paint;
#[cfg(feature = "sexos")]
pub mod sexos;
pub mod text;
pub mod theme;
mod ui;
pub mod widget;

pub use geom::{Point, Rect};
pub use i18n::{Arg, Strings};
pub use input::{Button, Event, Key, Pointer, PointerKind, Shortcut};
pub use menu::{Command, Commands, Menu, MenuItem};
pub use theme::{Color, Density, Mode, Theme};
pub use ui::{DialogId, Ui};
pub use widget::{Align, ButtonStyle, Column, Fill, Id, Layout, ListModel, Rows, Selection, Tab, TextStyle};

/// Something the person did that the program may want to act on.
#[derive(Clone, Debug, PartialEq)]
pub enum Response {
    /// A button was pressed (click, Space or Enter).
    Clicked(Id),
    /// A check box, switch or radio button changed to this state.
    Toggled(Id, bool),
    /// A field's text or a slider's value changed.
    Changed(Id),
    /// Enter in a field.
    Submitted(Id),
    /// A list's selection changed.
    Selected(Id),
    /// A row was opened: double-click or Enter.
    Activated(Id, usize),
    /// A tree row should open (`true`) or close: change the model, then
    /// [`Ui::model_changed`].
    Expand(Id, usize, bool),
    /// A table's header was clicked: sort by this column (Weft shows the arrow once
    /// the program calls [`Ui::set_sort`]).
    Sort(Id, usize),
    /// Something happened to a tab strip.
    Tab(Id, TabEvent),
    /// A command ran, from a shortcut, a menu, the palette or a toast.
    Command(String),
    /// A dialog was answered.
    Dialog(DialogId, DialogValue),
    /// Text was copied or cut: put it on the system clipboard.
    Copy(String),
    /// The person asked to close the window.
    Close,
}

/// What happened to a tab strip. Weft has already done what it can (switched,
/// reordered, pinned); closing and opening are the program's to do.
#[derive(Clone, Debug, PartialEq)]
pub enum TabEvent {
    /// This tab is now the active one.
    Selected(usize),
    /// The person asked to close this tab: call [`Ui::remove_tab`] when ready.
    Closed(usize),
    /// Close every tab but this one (after asking about unsaved ones).
    CloseOthers(usize),
    /// The New Tab button or Ctrl+T.
    New,
    /// A tab was dragged from one place to another (already moved).
    Moved { from: usize, to: usize },
    /// A tab was pinned or unpinned (already moved to its new place).
    Pinned(usize, bool),
    /// A tab was dragged out of the strip, dropped at this window point: open it in a
    /// new window (and remove it here).
    TornOff(usize, Point),
}

/// A dialog's answer.
#[derive(Clone, Debug, PartialEq)]
pub enum DialogValue {
    /// The index of the button pressed.
    Button(usize),
    /// Escape, or the Cancel button.
    Cancelled,
    /// A prompt's text.
    Text(String),
    /// A colour picker's colour.
    Color(Color),
    /// A font picker's face and size.
    Font { family: String, size: f32 },
}
