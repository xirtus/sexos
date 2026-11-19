// SPDX-License-Identifier: GPL-3.0-only
//! Colours of Blender's default dark theme (4.x), so the suite looks familiar.
//! All 0x00RRGGBB.

pub const TOPBAR: u32 = 0x181818;
pub const AREA_GAP: u32 = 0x161616;
pub const HEADER: u32 = 0x303030;
pub const HEADER_TRANSPARENT: u32 = 0x262626;
pub const REGION: u32 = 0x303030;
pub const PANEL: u32 = 0x3d3d3d;
pub const PANEL_HEADER: u32 = 0x3d3d3d;
pub const PANEL_BODY: u32 = 0x353535;
pub const OUTLINER: u32 = 0x282828;
pub const OUTLINER_ALT: u32 = 0x2b2b2b;
pub const OUTLINER_SEL: u32 = 0x334d80;
pub const OUTLINER_ACTIVE: u32 = 0x3b5689;
pub const PROPERTIES: u32 = 0x2b2b2b;
pub const VIEWPORT: u32 = 0x3d3d3d;
pub const TIMELINE: u32 = 0x2b2b2b;
pub const TEXT_EDITOR: u32 = 0x1d1d1d;
pub const NODE_BG: u32 = 0x1d1d1d;
pub const STATUSBAR: u32 = 0x181818;

pub const TEXT: u32 = 0xe6e6e6;
pub const TEXT_DIM: u32 = 0x9a9a9a;
pub const TEXT_HI: u32 = 0xffffff;

pub const BUTTON: u32 = 0x545454;
pub const BUTTON_HOVER: u32 = 0x656565;
pub const BUTTON_DARK: u32 = 0x282828;
pub const BUTTON_DARK_HOVER: u32 = 0x333333;
pub const MENU_BUTTON: u32 = 0x303030;
pub const SELECTED: u32 = 0x4772b3;
pub const SELECTED_HOVER: u32 = 0x5680c2;
pub const OUTLINE: u32 = 0x3d3d3d;

pub const MENU_BG: u32 = 0x181818;
pub const MENU_ITEM_HOVER: u32 = 0x4772b3;
pub const MENU_SEP: u32 = 0x2d2d2d;

pub const TAB_ACTIVE: u32 = 0x303030;
pub const TAB_INACTIVE: u32 = 0x1d1d1d;

pub const AXIS_X: u32 = 0xff3352;
pub const AXIS_Y: u32 = 0x8bdc00;
pub const AXIS_Z: u32 = 0x2890ff;

pub const ACTIVE_OBJECT: u32 = 0xffaa40;
pub const SELECTED_OBJECT: u32 = 0xf15800;
pub const PLAYHEAD: u32 = 0x4772b3;
pub const KEYFRAME: u32 = 0xe6e6e6;

/// UI metrics at 1x scale (Blender's "widget unit" is 20 px).
pub const UNIT: i32 = 20;
pub const HEADER_H: i32 = 26;
pub const TOPBAR_H: i32 = 26;
pub const STATUSBAR_H: i32 = 22;
pub const GAP: i32 = 2;
pub const FONT_PX: u32 = 12;
