// SPDX-License-Identifier: GPL-3.0-only
//! (Stub, to be filled in.)
use super::note;
use crate::doc::Doc;
use crate::draw::Rect;
use crate::input::Input;
use crate::ui::Ui;

#[derive(Default)]
pub struct State {}

pub fn header(_s: &mut State, _d: &mut Doc, _ui: &mut Ui, _r: Rect) {}

pub fn body(_s: &mut State, _d: &mut Doc, ui: &mut Ui, r: Rect) {
    note(ui, r, "properties");
}

pub fn hotkey(_s: &mut State, _d: &mut Doc, _i: &Input, _r: Rect) -> bool {
    false
}
