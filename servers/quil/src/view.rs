//! Pure bounded document layout. Rendering never modifies stored bytes.
use sex_pdx::{document_view::{Layout,TOOLBAR_CELL,TOOLBAR_LABELS},text_encoding as utf8};
#[derive(Clone, Copy)]
pub struct Text {
    pub bytes: [u8; 512],
    pub len: usize,
    pub selection: Option<(usize, usize)>,
}
impl Text {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 512],
            len: 0,
            selection: None,
        }
    }
    fn byte(&mut self, b: u8) {
        if self.len < 512 {
            self.bytes[self.len] = b;
            self.len += 1;
        }
    }
    fn scalar(&mut self, bytes: &[u8]) {
        if bytes.len() <= self.bytes.len() - self.len {
            self.bytes[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            self.len += bytes.len();
        }
    }
    fn line(&mut self, b: &[u8], cols: usize) {
        for &b in b.iter().take(cols) {
            self.byte(if (32..=126).contains(&b) { b } else { b'?' })
        }
        self.byte(b'\n');
    }
}
pub fn render(
    buf: &[u8],
    cursor: usize,
    top: &mut usize,
    l: Layout,
    name: &[u8],
    dirty: bool,
    state: &[u8],
    palette: Option<u8>,
    naming: Option<&[u8]>,
) -> Text {
    render_selection(
        buf, cursor, top, l, name, dirty, state, palette, naming, None,
    )
}
/// Selection is expressed in source bytes; the resulting range addresses only
/// the actual rendered frame, excluding headers, toolbars and the caret marker.
pub fn render_selection(
    buf: &[u8],
    cursor: usize,
    top: &mut usize,
    l: Layout,
    name: &[u8],
    dirty: bool,
    state: &[u8],
    palette: Option<u8>,
    naming: Option<&[u8]>,
    selection: Option<(usize, usize)>,
) -> Text {
    let mut t = Text::new();
    if l.rows == 0 { return t; }
    let mut head = [0; 96];
    let mut n = 0;
    let header_limit = l.cols.min(head.len());
    if let Some(name) = naming {
        for &b in b"Name: ".iter().chain(name.iter()).chain(b"#").take(header_limit) {
            head[n] = b;
            n += 1;
        }
    } else {
        let suffix = state.len().saturating_add(if dirty { 3 } else { 2 });
        for &b in name.iter().take(header_limit.saturating_sub(suffix)) {
            head[n] = b;
            n += 1;
        }
        if n < header_limit { head[n] = b' '; n += 1; }
        if dirty {
            for b in [b'*',b' '] { if n < header_limit { head[n] = b; n += 1; } }
        }
        for &b in state.iter().take(header_limit.saturating_sub(n)) { head[n] = b; n += 1; }
    }
    if l.header_rows() != 0 { t.line(&head[..n], l.cols); }
    if let Some(selected) = palette {
        let selected = (selected as usize).min(4);
        let labels = [b"Inspect storage  Ctrl R".as_slice(),b"Save  Ctrl S",b"Reload saved",b"New  Ctrl N",b"Save As  Ctrl Shift S"];
        let available = l.rows.saturating_sub(l.header_rows());
        let count = available.min(labels.len());
        let first = selected.saturating_sub(count.saturating_sub(1));
        for (i,label) in labels.iter().enumerate().skip(first).take(count) {
            let mut row = [b' '; 40];
            row[0] = if i == selected { b'>' } else { b' ' };
            row[2..2 + label.len()].copy_from_slice(label);
            t.line(&row[..2 + label.len()], l.cols);
        }
        for label in [b"Ctrl A select all".as_slice(),b"Ctrl C/X/V copy/cut/paste",b"Ctrl Shift U Unicode",b"Esc returns to document"].iter().take(available-count) {
            t.line(label,l.cols);
        }
        return t;
    }
    let per = l.toolbar_per_row();
    for row in 0..l.toolbar_rows {
        let mut line = [0; 4*TOOLBAR_CELL];
        for i in 0..per {
            line[i * TOOLBAR_CELL..(i+1)*TOOLBAR_CELL].copy_from_slice(TOOLBAR_LABELS[row*per+i]);
        }
        t.line(&line[..per * TOOLBAR_CELL], l.cols);
    }
    let normal_hint: &[u8] = if l.cols >= b"Ctrl N/O/S | Ctrl Shift S | Esc menu".len() {b"Ctrl N/O/S | Ctrl Shift S | Esc menu"}
        else if l.cols >= b"Ctrl N/O/S | Esc menu".len() {b"Ctrl N/O/S | Esc menu"} else {b"Esc menu"};
    let uncertain_hint: &[u8] = if l.cols >= 22 {b"Ctrl R inspect storage"} else {b"Esc menu"};
    if l.hint_rows() != 0 { t.line(if state.ends_with(b"UNCERTAIN") {uncertain_hint} else {normal_hint},l.cols); }
    let width = l.cols.saturating_sub(1).max(1);
    // Reserve chrome by geometry, not the current status/name byte length.
    // Otherwise a transient longer status can remove a body row and scroll an
    // unchanged caret before a failed operation has even completed.
    let header_budget = l.header_rows() * (l.cols + 1);
    let toolbar_budget = l.toolbar_rows * ((per * TOOLBAR_CELL).min(l.cols) + 1);
    let hint_budget = l.hint_rows() * (normal_hint.len().max(uncertain_hint.len()).min(l.cols) + 1);
    let chrome_budget = header_budget + toolbar_budget + hint_budget;
    // Reserve by the largest actual wrapped row, not four bytes per cell
    // across the entire document. One emoji must not collapse an otherwise
    // ASCII viewport. This bound depends only on source and geometry, so a
    // transient status/name change still cannot reflow an unchanged body.
    let mut max_row_bytes = 0;
    let mut row_bytes = 0;
    let mut row_cells = 0;
    let mut at = 0;
    while let Some((scalar,end)) = utf8::cell(buf,at) {
        if scalar != b'\n' as u32 {
            row_bytes += if core::str::from_utf8(&buf[at..end]).is_ok()&&utf8::is_text_scalar(scalar){end-at}else{3};
        }
        row_cells += 1;
        if scalar == b'\n' as u32 || row_cells >= width && end < buf.len() {
            max_row_bytes=max_row_bytes.max(row_bytes);row_bytes=0;row_cells=0;
        }
        at=end;
    }
    max_row_bytes=max_row_bytes.max(row_bytes);
    let rows = l.body_rows().min(512usize.saturating_sub(chrome_budget) / (max_row_bytes + 2));
    if rows == 0 {
        return t;
    }
    let mut cline = 0;
    let mut line = 0;
    let mut col = 0;
    let cursor = utf8::prefix(buf,cursor.min(buf.len()));
    let mut i = 0;
    while let Some((scalar,end)) = utf8::cell(buf,i) {
        if i == cursor {
            cline = line;
        }
        col += 1;
        if scalar == b'\n' as u32 || col >= width && end < buf.len() {
            line += 1;
            col = 0;
        }
        i=end;
    }
    if cursor == buf.len() {
        cline = line;
    }
    if cline < *top {
        *top = cline
    }
    if cline >= *top + rows {
        *top = cline + 1 - rows
    }
    // One physical text cell can express the caret, but not source plus caret.
    // Keep the stored document and selection intact; avoid an overflowing row.
    if l.cols == 1 { t.byte(b'#'); return t; }
    let mut line = 0;
    let mut col = 0;
    let mut i = 0;
    loop {
        if line >= *top + rows {
            break;
        }
        if line >= *top && i == cursor {
            t.byte(b'#');
        }
        if i == buf.len() {
            break;
        }
        let (scalar,end) = utf8::cell(buf,i).unwrap();
        if line >= *top && scalar != b'\n' as u32 {
            let at = t.len;
            // A valid source scalar reaches Display unchanged. Invalid read-only
            // bytes get one visible replacement cell, never a stored rewrite.
            if core::str::from_utf8(&buf[i..end]).is_ok() && utf8::is_text_scalar(scalar) {
                t.scalar(&buf[i..end]);
            } else { t.scalar("\u{fffd}".as_bytes()); }
            if t.len > at
                && selection.is_some_and(|(start, end)| {
                    start < end && end <= buf.len() && i >= start && i < end
                })
            {
                let start = t.selection.map_or(at, |(start, _)| start);
                t.selection = Some((start, t.len));
            }
        }
        col += 1;
        if scalar == b'\n' as u32 || col >= width && end < buf.len() {
            if line >= *top {
                t.byte(b'\n');
            }
            line += 1;
            col = 0;
        }
        i=end;
    }
    t
}

/// Move through every rendered line without a stack-sized line index. The
/// caller retains the goal column across short lines. A soft-wrap boundary
/// belongs to the following line, so it is not a valid end of the previous one.
pub fn vertical(
    buf: &[u8],
    cursor: usize,
    cols: usize,
    goal: &mut usize,
    up: bool,
) -> Option<usize> {
    vertical_steps(buf, cursor, cols, goal, up, 1)
}
/// Page movement scans the source twice, independent of the number of rows.
pub fn vertical_steps(
    buf: &[u8],
    cursor: usize,
    cols: usize,
    goal: &mut usize,
    up: bool,
    steps: usize,
) -> Option<usize> {
    let width = cols.saturating_sub(1).max(1);
    let cursor = utf8::prefix(buf,cursor.min(buf.len()));
    let mut line = 0usize;
    let mut current = 0usize;
    let mut start = 0usize;
    let mut col = 0usize;
    let mut i = 0;
    while let Some((scalar,end)) = utf8::cell(buf,i) {
        col += 1;
        if scalar == b'\n' as u32 || col >= width && end < buf.len() {
            if end <= cursor { current = line + 1; start = end; }
            line += 1; col = 0;
        }
        i=end;
    }
    if *goal == usize::MAX {
        *goal = utf8::cells(&buf[start..cursor]);
    }
    let target = if up {
        if current == 0 {
            return None;
        }
        current.saturating_sub(steps)
    } else {
        if current >= line {
            return None;
        }
        current.saturating_add(steps).min(line)
    };
    let mut line = 0usize;
    let mut start = 0usize;
    let mut col = 0usize;
    let mut i = 0;
    while let Some((scalar,end)) = utf8::cell(buf,i) {
        col += 1;
        if scalar == b'\n' as u32 || col >= width && end < buf.len() {
            if line == target { return Some(start + utf8::offset(&buf[start..i],*goal)); }
            start=end;line+=1;col=0;
        }
        i=end;
    }
    Some(start + utf8::offset(&buf[start..],*goal))
}

/// Preflight a whole replacement before an undo entry or byte is changed.
pub fn replacement_len(len: usize, range: (usize, usize), incoming: usize) -> Option<usize> {
    let (start, end) = range;
    if len > 12288 || start > end || end > len {
        return None;
    }
    len.checked_sub(end - start)?
        .checked_add(incoming)
        .filter(|&n| n <= 12288)
}
/// One atomic bounded edit. Returns the new caret; rejects without changing
/// bytes or length. No truncation, hidden allocation or second document copy.
pub fn replace(
    buf: &mut [u8; 12288],
    len: &mut usize,
    range: (usize, usize),
    incoming: &[u8],
) -> Option<usize> {
    let next = replacement_len(*len, range, incoming.len())?;
    if !utf8::valid_text(incoming) || !utf8::valid_text(&buf[..*len])
        || !utf8::is_boundary(&buf[..*len],range.0) || !utf8::is_boundary(&buf[..*len],range.1) {
        return None;
    }
    let (start, end) = range;
    let old = *len;
    buf.copy_within(end..old, start + incoming.len());
    buf[start..start + incoming.len()].copy_from_slice(incoming);
    if next < old {
        buf[next..old].fill(0)
    }
    *len = next;
    Some(start + incoming.len())
}
