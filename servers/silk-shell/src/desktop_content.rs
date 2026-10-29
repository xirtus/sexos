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
    if frame==201 {result[0]=Some(Child{sid:156,x:bx,y:by,w:bw,h:bh});}
    if frame==153 && bw>8 && bh>8 {
        let cw=(bw-8)/2;let ch=(bh-8)/2;
        if cw==0 || ch==0 {return result}
        for (i,sid) in [154,160,161,162].iter().enumerate() {
            let right=i%2==1;let bottom=i>=2;
            result[i]=Some(Child{sid:*sid,x:bx.saturating_add(if right {(cw+8)as i32}else{0}),
                y:by.saturating_add(if bottom {(ch+8)as i32}else{0}),
                w:if right{bw-8-cw}else{cw},h:if bottom{bh-8-ch}else{ch}});
        }
    }
    result
}
