//! Pure bounded document layout. Rendering never modifies stored bytes.
use sex_pdx::document_view::Layout;
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
    let mut head = [0; 96];
    let mut n = 0;
    if let Some(name) = naming {
        for &b in b"NAME? ".iter().chain(name.iter()).chain(b"#") {
            head[n] = b;
            n += 1;
        }
    } else {
        let suffix = state.len() + if dirty { 3 } else { 2 };
        for &b in name.iter().take(l.cols.saturating_sub(suffix)) {
            head[n] = b;
            n += 1;
        }
        head[n] = b' ';
        n += 1;
        if dirty {
            head[n] = b'*';
            n += 1;
            head[n] = b' ';
            n += 1;
        }
        for &b in state {
            head[n] = b;
            n += 1;
        }
    }
    t.line(&head[..n], l.cols);
    if let Some(selected) = palette {
        for (i, label) in [
            b"QUIESCE STORAGE".as_slice(),
            b"SAVE",
            b"RELOAD SAVED",
            b"NEW",
            b"SAVE AS",
        ]
        .iter()
        .enumerate()
        {
            let mut row = [b' '; 32];
            row[0] = if i == selected as usize { b'>' } else { b' ' };
            row[2..2 + label.len()].copy_from_slice(label);
            t.line(&row[..2 + label.len()], l.cols);
        }
        return t;
    }
    let labels = [b"NEW  ", b"OPEN ", b"SAVE ", b"AS   "];
    let per = 4 / l.toolbar_rows;
    for row in 0..l.toolbar_rows {
        let mut line = [0; 20];
        for i in 0..per {
            line[i * 5..i * 5 + 5].copy_from_slice(labels[row * per + i]);
        }
        t.line(&line[..per * 5], l.cols);
    }
    t.line(
        if state.ends_with(b"UNCERTAIN") {
            b"CTRL R QUIESCE / INSPECT"
        } else {
            b"CTRL N O S / ESC CMDS"
        },
        l.cols,
    );
    let width = l.cols.saturating_sub(1).max(1);
    // Reserve chrome by geometry, not the current status/name byte length.
    // Otherwise a transient longer status can remove a body row and scroll an
    // unchanged caret before a failed operation has even completed.
    let header_budget = l.cols + 1;
    let toolbar_budget = l.toolbar_rows * ((per * 5).min(l.cols) + 1);
    let hint_budget = b"CTRL R QUIESCE / INSPECT"
        .len()
        .max(b"CTRL N O S / ESC CMDS".len())
        .min(l.cols)
        + 1;
    let chrome_budget = header_budget + toolbar_budget + hint_budget;
    let rows = l
        .body_rows()
        .min(512usize.saturating_sub(chrome_budget) / (width + 2));
    if rows == 0 {
        return t;
    }
    let mut cline = 0;
    let mut line = 0;
    let mut col = 0;
    let cursor = cursor.min(buf.len());
    for (i, &b) in buf.iter().enumerate() {
        if i == cursor {
            cline = line;
        }
        col += 1;
        if b == b'\n' || col >= width && i + 1 < buf.len() {
            line += 1;
            col = 0;
        }
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
    let mut line = 0;
    let mut col = 0;
    for i in 0..=buf.len() {
        if line >= *top + rows {
            break;
        }
        if line >= *top && i == cursor {
            t.byte(b'#');
        }
        if i == buf.len() {
            break;
        }
        let b = buf[i];
        if line >= *top && b != b'\n' {
            let at = t.len;
            t.byte(if (32..=126).contains(&b) { b } else { b'?' });
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
        if b == b'\n' || col >= width && i + 1 < buf.len() {
            if line >= *top {
                t.byte(b'\n');
            }
            line += 1;
            col = 0;
        }
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
    let cursor = cursor.min(buf.len());
    let mut line = 0usize;
    let mut current = 0usize;
    let mut start = 0usize;
    let mut col = 0usize;
    for (i, &b) in buf.iter().enumerate() {
        col += 1;
        if b == b'\n' || col >= width && i + 1 < buf.len() {
            if i + 1 <= cursor {
                current = line + 1;
                start = i + 1;
            }
            line += 1;
            col = 0;
        }
    }
    if *goal == usize::MAX {
        *goal = cursor - start;
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
    for (i, &b) in buf.iter().enumerate() {
        col += 1;
        if b == b'\n' || col >= width && i + 1 < buf.len() {
            if line == target {
                return Some(start + (*goal).min(i - start));
            }
            start = i + 1;
            line += 1;
            col = 0;
        }
    }
    Some(start + (*goal).min(buf.len() - start))
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
    if !incoming
        .iter()
        .all(|&b| b == b'\n' || (32..=126).contains(&b))
    {
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
