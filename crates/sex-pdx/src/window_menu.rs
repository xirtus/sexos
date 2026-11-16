//! Window menu: the Shell chooses, Display draws.
//!
//! A right click on a window's title bar or one of its buttons opens a small
//! menu at the pointer, as in KDE, GNOME and Windows: Maximise (or Restore),
//! Float (or Tile), Move to workspace N for each other workspace, and Close.
//! The Shell keeps the menu and handles its clicks; Display draws it with the
//! same `Layout`, so a click lands on the row drawn.
//!
//! Wire (`OP_WINDOW_MENU`, Shell -> Display): arg0 = `Menu::wire`, zero hides it.
//! Self-contained (no other module) so the Shell's host test can compile it.

/// Shell -> Display.
pub const OP_WINDOW_MENU: u64 = 0x2E7;
/// Workspaces a menu can offer (the desktop has nine).
pub const MAX_WORKSPACES: u8 = 9;
/// Rows: Maximise, Float, up to eight other workspaces, Close.
pub const MAX_ROWS: usize = 3 + MAX_WORKSPACES as usize - 1;

/// One row of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// Maximise, or Restore when the window is maximised.
    Maximise,
    /// Float, or Tile when the window floats.
    Float,
    /// Move the window to this workspace (0-based).
    Workspace(u8),
    Close,
}

/// What the menu shows and where; the window it acts on stays with the Shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Menu {
    /// Where it was opened (top-left corner before keeping it on screen).
    pub x: u16,
    pub y: u16,
    pub maximised: bool,
    pub floating: bool,
    /// Workspaces on the desktop, and the one the window is on (0-based).
    pub workspaces: u8,
    pub current: u8,
    /// The row under the pointer, if any.
    pub hover: Option<u8>,
}

impl Menu {
    pub fn new(x: i32, y: i32, maximised: bool, floating: bool, workspaces: u8, current: u8) -> Self {
        let workspaces = workspaces.clamp(1, MAX_WORKSPACES);
        Self {
            x: x.clamp(0, u16::MAX as i32) as u16, y: y.clamp(0, u16::MAX as i32) as u16,
            maximised, floating, workspaces, current: current.min(workspaces - 1), hover: None,
        }
    }

    pub fn rows(&self) -> usize { 3 + self.workspaces as usize - 1 }

    /// The row at `index`, top to bottom.
    pub fn item(&self, index: usize) -> Option<Item> {
        let n = self.rows();
        if index >= n { return None; }
        Some(match index {
            0 => Item::Maximise,
            1 => Item::Float,
            i if i == n - 1 => Item::Close,
            i => {
                // Every workspace but the window's own, in order.
                let k = (i - 2) as u8;
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
            Item::Workspace(n) => MOVE.get(n as usize).copied().unwrap_or("Move"),
            Item::Close => "Close",
        }
    }

    /// Bits: x 0-15, y 16-31, workspaces 32-35, current 36-39, maximised 40,
    /// floating 41, hover 44-47 (15 = none), shown 63.
    pub fn wire(&self) -> u64 {
        let hover = self.hover.map_or(15, |h| h.min(14)) as u64;
        self.x as u64 | (self.y as u64) << 16 | (self.workspaces as u64) << 32 | (self.current as u64) << 36
            | (self.maximised as u64) << 40 | (self.floating as u64) << 41 | hover << 44 | 1 << 63
    }

    /// None for zero (hidden) and for words no Shell sends.
    pub fn decode(w: u64) -> Option<Self> {
        if w >> 63 == 0 || w >> 48 & 0x7FFF != 0 || w >> 42 & 3 != 0 { return None; }
        let workspaces = (w >> 32 & 0xF) as u8;
        let current = (w >> 36 & 0xF) as u8;
        if workspaces == 0 || workspaces > MAX_WORKSPACES || current >= workspaces { return None; }
        let menu = Self {
            x: w as u16, y: (w >> 16) as u16, workspaces, current,
            maximised: w >> 40 & 1 != 0, floating: w >> 41 & 1 != 0, hover: None,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit { Row(usize), Inside, Outside }

impl Layout {
    pub const WIDTH: usize = 200;
    pub const ROW: usize = 24;
    pub const PAD: usize = 4;

    /// None when the screen cannot hold the menu.
    pub fn new(menu: &Menu, width: usize, height: usize, scale: u32) -> Option<Self> {
        let s = scale.clamp(1, 3) as usize;
        let (w, row_h, pad) = (Self::WIDTH * s, Self::ROW * s, Self::PAD * s);
        let rows = menu.rows();
        let h = rows * row_h + 2 * pad;
        if width < w || height < h { return None; }
        // Kept on screen: a menu opened near the right or bottom edge moves in.
        let x = (menu.x as usize).min(width - w);
        let y = (menu.y as usize).min(height - h);
        Some(Self { panel: (x, y, w, h), rows, row_h, pad })
    }

    pub fn row(&self, index: usize) -> Rect {
        let (x, y, w, _) = self.panel;
        (x + self.pad, y + self.pad + index * self.row_h, w - 2 * self.pad, self.row_h)
    }

    pub fn hit(&self, x: usize, y: usize) -> Hit {
        if !inside(self.panel, x, y) { return Hit::Outside; }
        (0..self.rows).find(|&i| inside(self.row(i), x, y)).map_or(Hit::Inside, Hit::Row)
    }
}
