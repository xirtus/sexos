//! Window menu: the Shell chooses, Display draws.
//!
//! A right click on a window's title bar or one of its buttons opens a small
//! menu at the pointer, as in KDE, GNOME and Windows: Maximise (or Restore),
//! Float (or Tile), Ungroup for a tab in a group, Move to workspace N for each
//! other workspace, and Close.
//! The Shell keeps the menu and handles its clicks; Display draws it with the
//! same `Layout`, so a click lands on the row drawn.
//!
//! Wire (`OP_WINDOW_MENU`, Shell -> Display): arg0 = `Menu::wire`, zero hides it.
//! Self-contained (no other module) so the Shell's host test can compile it.

/// Shell -> Display.
pub const OP_WINDOW_MENU: u64 = 0x2E7;
/// Workspaces a menu can offer (the desktop has nine).
pub const MAX_WORKSPACES: u8 = 9;
/// Rows: Maximise, Float, Ungroup, up to eight other workspaces, Close.
pub const MAX_ROWS: usize = 4 + MAX_WORKSPACES as usize - 1;

/// One row of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// Maximise, or Restore when the window is maximised.
    Maximise,
    /// Float, or Tile when the window floats.
    Float,
    /// Take the shown tab out of its group (only when the window is a group).
    Ungroup,
    /// Move the window to this workspace (0-based).
    Workspace(u8),
    Close,
}

/// Number of per-window candy tint swatches shown at the bottom of the window menu.
pub const SWATCH_COUNT: usize = 8;
/// Harmonious 8-color candy palette (`0x00RRGGBB`) for the window menu swatch strip.
pub const CANDY_SWATCHES: [u32; SWATCH_COUNT] = [
    0x00D6_336C, // 1: Berry / SexOS Rose
    0x00A7_8BFA, // 2: Lavender / Mauve
    0x0060_A5FA, // 3: Sky / Sapphire
    0x0034_D399, // 4: Mint / Emerald
    0x00FB_923C, // 5: Peach / Apricot
    0x00FB_BF24, // 6: Honey / Lemon
    0x002D_D4BF, // 7: Aqua / Teal
    0x00F4_72B6, // 8: Coral / Flamingo
];

/// What the menu shows and where; the window it acts on stays with the Shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Menu {
    /// Where it was opened (top-left corner before keeping it on screen).
    pub x: u16,
    pub y: u16,
    pub maximised: bool,
    pub floating: bool,
    /// The window is a group of tabs: the menu offers Ungroup.
    pub grouped: bool,
    /// Workspaces on the desktop, and the one the window is on (0-based).
    pub workspaces: u8,
    pub current: u8,
    /// The row under the pointer, if any.
    pub hover: Option<u8>,
    /// Current per-window candy swatch (0 = Auto, 1..=8 = explicit swatch).
    pub tint: u8,
    /// Candy swatch under the pointer, if any (1..=8).
    pub hover_swatch: Option<u8>,
}

impl Menu {
    pub fn new(x: i32, y: i32, maximised: bool, floating: bool, workspaces: u8, current: u8) -> Self {
        let workspaces = workspaces.clamp(1, MAX_WORKSPACES);
        Self {
            x: x.clamp(0, u16::MAX as i32) as u16, y: y.clamp(0, u16::MAX as i32) as u16,
            maximised, floating, grouped: false, workspaces, current: current.min(workspaces - 1),
            hover: None, tint: 0, hover_swatch: None,
        }
    }

    /// The same menu for a group of tabs.
    pub fn grouped(self, grouped: bool) -> Self { Self { grouped, ..self } }

    /// The same menu showing the window's active candy swatch (0 = Auto, 1..=8).
    pub fn with_tint(self, tint: u8) -> Self {
        Self { tint: if tint as usize <= SWATCH_COUNT { tint } else { 0 }, ..self }
    }

    pub fn rows(&self) -> usize { 3 + self.grouped as usize + self.workspaces as usize - 1 }

    /// The row at `index`, top to bottom.
    pub fn item(&self, index: usize) -> Option<Item> {
        let n = self.rows();
        if index >= n { return None; }
        Some(match index {
            0 => Item::Maximise,
            1 => Item::Float,
            2 if self.grouped => Item::Ungroup,
            i if i == n - 1 => Item::Close,
            i => {
                // Every workspace but the window's own, in order.
                let k = (i - 2 - self.grouped as usize) as u8;
                Item::Workspace(if k >= self.current { k + 1 } else { k })
            }
        })
    }

    pub fn label(&self, item: Item) -> &'static str {
        const MOVE: [&str; MAX_WORKSPACES as usize] = [
            "Move to workspace 1", "Move to workspace 2", "Move to workspace 3",
            "Move to workspace 4", "Move to workspace 5", "Move to workspace 6",
            "Move to workspace 7", "Move to workspace 8", "Move to workspace 9",
        ];
        match item {
            Item::Maximise => if self.maximised { "Restore" } else { "Maximise" },
            Item::Float => if self.floating { "Tile" } else { "Float" },
            Item::Ungroup => "Ungroup",
            Item::Workspace(n) => MOVE.get(n as usize).copied().unwrap_or("Move"),
            Item::Close => "Close",
        }
    }

    /// Bits: x 0-15, y 16-31, workspaces 32-35, current 36-39, maximised 40,
    /// floating 41, grouped 42, hover 44-47 (15 = none), tint 48-51 (0..=8),
    /// hover_swatch 52-55 (0 = none, 1..=8), shown 63.
    pub fn wire(&self) -> u64 {
        let hover = self.hover.map_or(15, |h| h.min(14)) as u64;
        let tint = (self.tint as usize).min(SWATCH_COUNT) as u64;
        let hover_swatch = self.hover_swatch.map_or(0, |s| (s as usize).min(SWATCH_COUNT)) as u64;
        self.x as u64 | (self.y as u64) << 16 | (self.workspaces as u64) << 32 | (self.current as u64) << 36
            | (self.maximised as u64) << 40 | (self.floating as u64) << 41 | (self.grouped as u64) << 42
            | hover << 44 | tint << 48 | hover_swatch << 52 | 1 << 63
    }

    /// None for zero (hidden) and for words no Shell sends.
    pub fn decode(w: u64) -> Option<Self> {
        if w >> 63 == 0 || (w >> 56) & 0x7F != 0 || (w >> 43) & 1 != 0 { return None; }
        let workspaces = (w >> 32 & 0xF) as u8;
        let current = (w >> 36 & 0xF) as u8;
        if workspaces == 0 || workspaces > MAX_WORKSPACES || current >= workspaces { return None; }
        let tint = (w >> 48 & 0xF) as u8;
        let hs = (w >> 52 & 0xF) as u8;
        if tint as usize > SWATCH_COUNT || hs as usize > SWATCH_COUNT { return None; }
        let menu = Self {
            x: w as u16, y: (w >> 16) as u16, workspaces, current,
            maximised: w >> 40 & 1 != 0, floating: w >> 41 & 1 != 0, grouped: w >> 42 & 1 != 0,
            hover: None, tint, hover_swatch: (hs != 0).then_some(hs),
        };
        let hover = (w >> 44 & 0xF) as u8;
        if hover != 15 && hover as usize >= menu.rows() { return None; }
        Some(Self { hover: (hover != 15).then_some(hover), ..menu })
    }
}

pub type Rect = (usize, usize, usize, usize);
fn inside(r: Rect, x: usize, y: usize) -> bool { x >= r.0 && y >= r.1 && x - r.0 < r.2 && y - r.1 < r.3 }

/// Where the menu and its rows go on a `width` x `height` screen at `scale`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub panel: Rect,
    pub rows: usize,
    pub row_h: usize,
    pub pad: usize,
    /// The candy swatch strip below the action rows.
    pub swatch_h: usize,
    /// The scale the menu is drawn at: the UI scale, or less when the
    /// screen is too short for the menu at the UI scale.
    pub scale: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit { Row(usize), Swatch(u8), Inside, Outside }

impl Layout {
    pub const WIDTH: usize = 200;
    pub const ROW: usize = 24;
    pub const SWATCH_ROW: usize = 26;
    pub const PAD: usize = 4;

    /// None when the screen cannot hold the menu even at scale 1. A screen
    /// too small for the UI scale (800 px tall at 3x) gets a smaller menu.
    pub fn new(menu: &Menu, width: usize, height: usize, scale: u32) -> Option<Self> {
        let rows = menu.rows();
        (1..=scale.clamp(1, 3)).rev().find_map(|scale| {
            let s = scale as usize;
            let (w, row_h, swatch_h, pad) = (Self::WIDTH * s, Self::ROW * s, Self::SWATCH_ROW * s, Self::PAD * s);
            let h = rows * row_h + swatch_h + 2 * pad;
            if width < w || height < h { return None; }
            // Kept on screen: a menu opened near the right or bottom edge moves in.
            let x = (menu.x as usize).min(width - w);
            let y = (menu.y as usize).min(height - h);
            Some(Self { panel: (x, y, w, h), rows, row_h, pad, swatch_h, scale })
        })
    }

    pub fn row(&self, index: usize) -> Rect {
        let (x, y, w, _) = self.panel;
        (x + self.pad, y + self.pad + index * self.row_h, w - 2 * self.pad, self.row_h)
    }

    /// Bounding rectangle of the bottom candy swatch strip.
    pub fn swatch_strip(&self) -> Rect {
        let (x, y, w, _) = self.panel;
        (x + self.pad, y + self.pad + self.rows * self.row_h, w - 2 * self.pad, self.swatch_h)
    }

    /// Click target rectangle for candy swatch `index` (`0..SWATCH_COUNT`).
    pub fn swatch_cell(&self, index: usize) -> Rect {
        let (sx, sy, sw, sh) = self.swatch_strip();
        let cell_w = sw / SWATCH_COUNT;
        let rem = sw - cell_w * SWATCH_COUNT;
        (sx + rem / 2 + index * cell_w, sy, cell_w, sh)
    }

    pub fn hit(&self, x: usize, y: usize) -> Hit {
        if !inside(self.panel, x, y) { return Hit::Outside; }
        if let Some(i) = (0..self.rows).find(|&i| inside(self.row(i), x, y)) {
            return Hit::Row(i);
        }
        if let Some(i) = (0..SWATCH_COUNT).find(|&i| inside(self.swatch_cell(i), x, y)) {
            return Hit::Swatch((i + 1) as u8);
        }
        Hit::Inside
    }
}
