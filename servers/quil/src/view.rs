//! Pure bounded document layout. Rendering never modifies stored bytes.
use sex_pdx::document_view::Layout;
#[derive(Clone, Copy)]
pub struct Text {
    pub bytes: [u8; 512],
    pub len: usize,
}
impl Text {
    pub const fn new() -> Self {
        Self {
            bytes: [0; 512],
            len: 0,
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
        if state == b"UNCERTAIN" {
            b"CTRL R QUIESCE / INSPECT"
        } else {
            b"CTRL N O S / ESC CMDS"
        },
        l.cols,
    );
    let width = l.cols.saturating_sub(1).max(1);
    let rows = l.body_rows().min((512 - t.len) / (width + 2));
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
            t.byte(if (32..=126).contains(&b) { b } else { b'?' })
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
