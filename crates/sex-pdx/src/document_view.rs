//! BOOT-authenticated geometry and document controls; coordinates are data.
use crate::desktop_style as s;
pub const OP: u64 = 0x272;
pub const ACTION: u64 = 0x273;
pub const OPEN_LINEN: u64 = 0x274;
pub const READY: u64 = 0x275;
pub const NEW: u8 = 1;
pub const OPEN: u8 = 2;
pub const SAVE: u8 = 3;
pub const SAVE_AS: u8 = 4;
pub const QUIESCE: u8 = 5;
/// Fixed visible button cells, shared by the app view and Shell hit testing.
pub const TOOLBAR_CELL: usize = 8;
pub const TOOLBAR_LABELS: [&[u8]; 4] = [b"New     ", b"Open    ", b"Save    ", b"Save As "];
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub w: u32,
    pub h: u32,
    pub scale: u32,
    pub cols: usize,
    pub rows: usize,
    pub top: u32,
    pub toolbar_rows: usize,
}
impl Layout {
    pub fn new(target: u32, w: u32, h: u32) -> Self {
        let scale = s::scale(w);
        let cols = (w.saturating_sub(2 * s::INSET) / (s::ADVANCE * scale))
            .saturating_sub(1)
            .clamp(1, 79) as usize;
        let top = if matches!(target,15|16|18) { 36 } else { s::TEXT_TOP };
        let desired_toolbar = if cols >= 4 * TOOLBAR_CELL {
            1
        } else if cols >= 2 * TOOLBAR_CELL {
            2
        } else if cols >= TOOLBAR_CELL {
            4
        } else {
            0
        };
        let rows = if w >= 2 * s::INSET + s::GLYPH_WIDTH * scale
            && h.saturating_sub(top) >= s::GLYPH_HEIGHT * scale {
            1 + (h - top - s::GLYPH_HEIGHT * scale) / (s::LINE * scale)
        } else {
            0
        } as usize;
        // Every useful view retains an editor/caret row. Short views expose
        // the full command list through Esc instead of invisible button rows.
        let toolbar_rows = if desired_toolbar != 0 && rows >= desired_toolbar + 2 {
            desired_toolbar
        } else { 0 };
        Self {
            w,
            h,
            scale,
            cols,
            rows,
            top,
            toolbar_rows,
        }
    }
    pub fn header_rows(self) -> usize { usize::from(self.rows >= 2) }
    pub fn toolbar_per_row(self) -> usize {
        match self.toolbar_rows { 1 => 4, 2 => 2, 4 => 1, _ => 0 }
    }
    pub fn hint_rows(self) -> usize {
        usize::from(self.cols >= 8 && self.rows >= self.header_rows() + self.toolbar_rows + 2)
    }
    pub fn chrome_rows(self) -> usize {
        self.header_rows() + self.toolbar_rows + self.hint_rows()
    }
    pub fn body_rows(self) -> usize {
        // Display indexes at most 24 rows in the entire committed text frame,
        // including chrome. A tall narrow view must not scroll its caret into
        // unindexed rows merely because all of those bytes fit the transport.
        self.rows.saturating_sub(self.chrome_rows()).min(24usize.saturating_sub(self.chrome_rows()))
    }
    pub fn hit(self, x: u32, y: u32) -> Option<u8> {
        let per = self.toolbar_per_row();
        if per == 0 || x >= self.w || y >= self.h || x < s::INSET
            || y < self.top + self.header_rows() as u32 * s::LINE * self.scale {
            return None;
        }
        let col = ((x - s::INSET) / (s::ADVANCE * self.scale)) as usize;
        let row = ((y - self.top) / (s::LINE * self.scale)) as usize - self.header_rows();
        let item = row * per + col / TOOLBAR_CELL;
        if row >= self.toolbar_rows || col >= self.cols || col >= per * TOOLBAR_CELL || item >= 4 {
            return None;
        }
        Some(item as u8 + 1)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notice {
    pub target: u32,
    pub task: u32,
    pub w: u32,
    pub h: u32,
    pub sequence: u64,
}
impl Notice {
    pub fn args(self) -> [u64; 3] {
        [
            (self.task as u64) << 32 | self.target as u64,
            (self.h as u64) << 32 | self.w as u64,
            self.sequence,
        ]
    }
    pub fn decode(caller: u32, target: u32, task: u32, args: [u64; 3], last: u64) -> Option<Self> {
        let n = Self {
            target: args[0] as u32,
            task: (args[0] >> 32) as u32,
            w: args[1] as u32,
            h: (args[1] >> 32) as u32,
            sequence: args[2],
        };
        if caller != 3
            || !matches!(target, 9 | 15 | 16 | 18)
            || task == 0
            || n.target != target
            || n.task != task
            || n.sequence <= last
            || n.w == 0
            || n.h == 0
            || n.w > 4096
            || n.h > 4096
        {
            return None;
        }
        Some(n)
    }
}
/// One desired notice per fixed BOOT subject; full queues retain exact data.
pub struct Delivery {
    desired: Option<Notice>,
    last: Option<(u32, u32)>,
    sequence: u64,
    dead: bool,
}
impl Delivery {
    pub const fn new() -> Self {
        Self {
            desired: None,
            last: None,
            sequence: 0,
            dead: false,
        }
    }
    pub fn queue(&mut self, target: u32, task: u32, w: u32, h: u32) {
        if self.dead
            || !matches!(target, 9 | 15 | 16 | 18)
            || task == 0
            || w == 0
            || h == 0
            || w > 4096
            || h > 4096
        {
            return;
        }
        if self.desired.map(|n| (n.w, n.h)).or(self.last) == Some((w, h)) {
            return;
        }
        let Some(sequence) = self.sequence.checked_add(1) else {
            self.retire();
            return;
        };
        self.sequence = sequence;
        self.desired = Some(Notice {
            target,
            task,
            w,
            h,
            sequence,
        });
    }
    pub fn refresh(&mut self) {
        if !self.dead {
            self.last = None;
        }
    }
    pub fn pending(&self) -> bool {
        self.desired.is_some()
    }
    pub fn retire(&mut self) {
        self.dead = true;
        self.desired = None;
        self.last = None;
    }
    pub fn poll(
        &mut self,
        live: bool,
        ready: bool,
        mut send: impl FnMut(Notice) -> Result<(), u64>,
    ) {
        let Some(n) = self.desired else { return };
        if !live {
            self.retire();
            return;
        }
        if !ready {
            return;
        }
        match send(n) {
            Ok(()) => {
                self.last = Some((n.w, n.h));
                self.desired = None
            }
            Err(crate::ERR_SERVICE_NOT_READY) => {}
            Err(_) => self.retire(),
        }
    }
}

/// A read-only list invalidation on Quil's existing BOOT shell route. No object
/// number or address is accepted; the shell independently rescans its own scope.
pub const WORKSPACE_CHANGED: u64 = 0x279;
pub struct WorkspaceRefresh { target:u32, task:u32, pending:bool, closed:bool }
impl WorkspaceRefresh {
    pub const fn new(target:u32,task:u32)->Self {Self{target,task,pending:false,closed:!((matches!(target,9|16|18)&&task!=0)||(target==15&&task==15))}}
    pub fn changed(&mut self){if !self.closed{self.pending=true;}}
    pub fn pending(&self)->bool{self.pending&&!self.closed}
    pub fn close(&mut self){self.pending=false;self.closed=true;}
    pub fn poll(&mut self,mut send:impl FnMut([u64;3])->Result<(),u64>){
        if !self.pending(){return;}
        match send([self.target as u64,self.task as u64,0]){
            Ok(())=>self.pending=false,
            Err(crate::ERR_SERVICE_NOT_READY)=>{},
            Err(_)=>self.close(),
        }
    }
}
