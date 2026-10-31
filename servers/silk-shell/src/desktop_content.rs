//! Bounded presentation of the last committed Linen table and boot native frames.
#[derive(Clone, Copy)]
pub struct Item<'a> { pub id: u64, pub name: &'a [u8] }
pub struct List {
    pub text: [u8; 512], pub len: usize, pub first: usize, pub rows: usize,
    pub selected_row: Option<usize>, pub row_y: u32, pub row_h: u32,
}
impl List {
    fn line(&mut self, bytes: &[u8], cols: usize) {
        // Leave one column unused: the display wraps before a following newline
        // when a line occupies exactly wrap_cols columns.
        let limit = cols.saturating_sub(1).min(80);
        let n = bytes.len().min(limit);
        for (i, &byte) in bytes[..n].iter().enumerate() {
            if self.len + 1 >= self.text.len() { return; }
            self.text[self.len] = if i + 1 == n && bytes.len() > n { b'~' }
                else if (32..=126).contains(&byte) { byte } else { b'?' };
            self.len += 1;
        }
        if self.len < self.text.len() { self.text[self.len] = b'\n'; self.len += 1; }
    }
}
/// Inputs are the published table only; no staging or transport state is read.
pub fn list(items: &[Item<'_>], selected: u64, width: u32, height: u32) -> List {
    let scale = if width >= 256 { 2 } else { 1 };
    let line_h = 9 * scale;
    let cols = (width.saturating_sub(16) / (6 * scale)).clamp(1,80) as usize;
    let mut out = List { text: [0;512], len:0, first:0, rows:0,
        selected_row:None, row_y:36 + 2*line_h, row_h:2*line_h };
    if cols < 4 || height < out.row_y + out.row_h { return out; }
    out.rows = ((height - out.row_y) / out.row_h).min(5).min(items.len() as u32) as usize;
    if out.rows == 0 { return out; }
    let selected_index = items.iter().position(|item| item.id == selected).unwrap_or(0);
    out.first = selected_index / out.rows * out.rows;
    out.rows = out.rows.min(items.len() - out.first);
    out.line(b"LINEN / FILES",cols);
    out.line(b"J/K SELECT  ENTER OPEN",cols);
    for row in 0..out.rows {
        let item = items[out.first + row];
        let mut line = [0u8; 128];
        let chosen = item.id == selected;
        line[0] = if chosen { b'>' } else { b' ' }; line[1] = b' ';
        let name = if item.name.is_empty() { b"(UNTITLED)" as &[u8] } else { item.name };
        let n = name.len().min(line.len()-2);
        for i in 0..n { line[i+2] = name[i]; }
        out.line(&line[..n+2],cols); out.line(b"",cols);
        if chosen { out.selected_row = Some(row); }
    }
    out
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Child { pub sid:u64, pub x:i32, pub y:i32, pub w:u32, pub h:u32 }
/// Called only after live immutable catalog authorization in the shell.
pub fn children(frame:u64,x:i32,y:i32,w:u32,h:u32)->[Option<Child>;4] {
    let mut result=[None;4];
    let bw=w.saturating_sub(16); let bh=h.saturating_sub(44);
    if bw==0 || bh==0 {return result}
    let bx=x.saturating_add(8);let by=y.saturating_add(36);
    if let Some(row)=crate::app_catalog::INCARNATIONS.iter().find(|row|row.shell_surface==Some(frame)) {
        match row.app {
            crate::app_catalog::App::Quil=>{result[0]=Some(Child{sid:row.surface,x:bx,y:by,w:bw,h:bh});},
            crate::app_catalog::App::Spindle=>if let Some(ids)=crate::app_catalog::spindle_grid(row.target){
                let view=sex_pdx::spindle_view::Layout::new(w,h);
                for(i,sid)in ids.iter().enumerate(){let p=view.panes[i];result[i]=Some(Child{sid:*sid,x:x.saturating_add(p.x as i32),y:y.saturating_add(p.y as i32),w:p.w,h:p.h});}
            },
            crate::app_catalog::App::Browser=>{},
        }
    }
    result
}

/// Immutable BOOT visual association, never caller-supplied authority.
pub const fn parent_frame(child:u64)->Option<u64>{
    let mut index=0;while index<crate::app_catalog::COUNT {
        let row=&crate::app_catalog::INCARNATIONS[index];
        if let Some(frame)=row.shell_surface {
            if row.surface==child{return Some(frame)}
            let mut i=0;while i<row.children.len(){if row.children[i]==child{return Some(frame)}i+=1;}
        }index+=1;
    }None
}
