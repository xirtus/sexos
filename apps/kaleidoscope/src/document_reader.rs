// SPDX-License-Identifier: GPL-3.0-only
//! Bounded presentation of already received plain-text data. No endpoint,
//! capability, HTML, TLS or network operation is introduced by this reader.
use sex_pdx::desktop_style::{scale, ADVANCE, INSET, LINE};

const CAPACITY: usize = 512;
const MAX_BODY: usize = 12288;
/// A readable measure also leaves room for seven body rows in a tall view
/// without enlarging the fixed display transaction or shrinking the font.
pub const MAX_COLUMNS: usize = 47;
const TOP: u32 = 28 + INSET;
const GLYPH_HEIGHT: u32 = 7;
const HOST: &[u8] = sex_pdx::network_config::HOST_NAME;
const PATH: &[u8] = b" /welcome";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move { Up, Down, PageUp, PageDown, Home, End }

pub struct Frame {
    pub bytes: [u8; CAPACITY],
    pub len: usize,
    /// Zero-based wrapped body line range [first,end), excluding chrome.
    pub first: usize,
    pub end: usize,
    pub body_rows: usize,
    pub rows: usize,
    pub cols: usize,
    pub total_lines: usize,
    /// Retained source-byte intent; rendering/reflow never moves this anchor.
    /// `first` describes the independently clamped current wrapped line.
    pub source_anchor: usize,
}

/// Geometry and one source-byte anchor are the entire retained reader state.
/// No body copy or per-byte/per-line table is retained.
pub struct Reader { width: u32, height: u32, anchor: usize }

struct Line { start: usize, end: usize, next: usize }
struct Layout { total: usize, first: usize, offset: usize }

fn clean(byte: u8) -> u8 {
    match byte { b' '..=b'~' => byte, 0..=127 => b' ', _ => b'?' }
}

/// Blank-only trailing lines/spaces have no final readable text. Excluding that
/// suffix keeps End on the final actual text while preserving interior blanks
/// and every retained source-byte position. The original body is never changed.
fn content(body: &[u8]) -> &[u8] {
    let body = &body[..body.len().min(MAX_BODY)];
    let mut end = body.len();
    while end > 0 && (body[end-1] == b'\n' || clean(body[end-1]) == b' ') { end -= 1; }
    &body[..end]
}

/// Consume at least one byte. A newline immediately after a full hard-wrapped
/// row terminates that same row; trailing newlines never create a blank tail.
/// Interior blank paragraphs remain visible, and every scan consumes ≤12288B.
fn line(body: &[u8], start: usize, cols: usize) -> Line {
    let mut end = start;
    while end < body.len() && end-start < cols && body[end] != b'\n' { end += 1; }
    let next = if body.get(end) == Some(&b'\n') { end+1 } else { end };
    Line { start, end, next }
}

fn offset_at(body: &[u8], cols: usize, wanted: usize) -> usize {
    let mut cursor = 0;
    for _ in 0..wanted {
        if cursor == body.len() { break; }
        cursor = line(body, cursor, cols).next;
    }
    cursor
}

/// One forward count/anchor pass and one forward alignment pass. Each pass
/// touches at most the bounded source extent, regardless of navigation input.
fn layout(body: &[u8], cols: usize, visible: usize, anchor: usize) -> Layout {
    let mut cursor = 0;
    let mut total = 0usize;
    let mut anchored = 0usize;
    while cursor < body.len() {
        let row = line(body, cursor, cols);
        if row.start <= anchor { anchored = total; }
        cursor = row.next;
        total += 1;
    }
    let first = anchored.min(total.saturating_sub(visible));
    Layout { total, first, offset: offset_at(body, cols, first) }
}

impl Reader {
    pub const fn new() -> Self { Self { width: 0, height: 0, anchor: 0 } }
    pub fn resize(&mut self, w: u32, h: u32) { self.width = w; self.height = h; }
    pub fn clear(&mut self) { self.anchor = 0; }
    pub fn source_anchor(&self) -> usize { self.anchor }

    fn geometry(&self) -> (usize, usize) {
        if self.width == 0 || self.height == 0 { return (0,0); }
        let factor = scale(self.width);
        // The renderer wraps immediately on its last physical column. Reserve
        // one column so an explicit newline cannot produce an extra blank row.
        let cols = ((self.width.saturating_sub(2*INSET)/(ADVANCE*factor)).saturating_sub(1) as usize).min(MAX_COLUMNS);
        let first_bottom = TOP + GLYPH_HEIGHT*factor;
        if cols == 0 || self.height < first_bottom { return (0,0); }
        let physical = 1 + (self.height-first_bottom)/(LINE*factor);
        let rows = (physical as usize).min(32).min(CAPACITY/(cols+1));
        (cols,rows)
    }

    pub fn render(&mut self, body: &[u8]) -> Frame {
        self.render_path(body,&PATH[1..])
    }
    /// The final validated request path is presentation data. Clean every
    /// byte defensively so it cannot inject a row or replace the reader chrome.
    pub fn render_path(&mut self, body: &[u8], path:&[u8]) -> Frame {
        self.render_source(body,path,b"Plain HTTP / no TLS",HOST)
    }
    pub fn render_https(&mut self,body:&[u8],path:&[u8])->Frame {
        self.render_source(body,path,b"Controlled HTTPS / lab root",b"probe.sexos.test:46080")
    }
    fn render_source(&mut self,body:&[u8],path:&[u8],banner:&[u8],host:&[u8])->Frame {
        let body = content(body);
        let (cols,rows) = self.geometry();
        let mut frame = Frame { bytes: [0;CAPACITY], len: 0, first: 0, end: 0,
            body_rows: rows.saturating_sub(3), rows, cols, total_lines: 0, source_anchor: self.anchor };
        if rows == 0 { return frame; }
        frame.len = rows*(cols+1)-1;
        for row in 0..rows {
            let at = row*(cols+1);
            frame.bytes[at..at+cols].fill(b' ');
            if row+1 != rows { frame.bytes[at+cols] = b'\n'; }
        }
        let header: &[u8] = if banner==b"Controlled HTTPS / lab root" {
            if cols>=26 { banner } else if cols>=16 { b"HTTPS / lab root" }
            else if cols>=9 { b"Lab HTTPS" } else { b"Lab TLS" }
        } else { banner };
        put(&mut frame,0,header);
        if rows > 2 {
            let at = cols+1;
            for (index,byte) in host.iter().chain(b" ").chain(path).take(cols).enumerate() { frame.bytes[at+index] = clean(*byte); }
        }
        let visible = frame.body_rows;
        let info = layout(body,cols,visible,self.anchor);
        frame.total_lines = info.total;
        if visible != 0 {
            frame.first = info.first;
            let mut cursor = info.offset;
            let mut shown = 0;
            while shown < visible && cursor < body.len() {
                let source = line(body,cursor,cols);
                let at = (shown+2)*(cols+1);
                for (target,byte) in frame.bytes[at..at+source.end-source.start].iter_mut().zip(&body[source.start..source.end]) { *target = clean(*byte); }
                cursor = source.next;
                shown += 1;
            }
            frame.end = frame.first+shown;
        }
        if rows > 1 { footer(&mut frame,banner==b"Controlled HTTPS / lab root"); }
        frame
    }

    pub fn navigate(&mut self, body: &[u8], action: Move) -> bool {
        let (cols,rows) = self.geometry();
        let visible = rows.saturating_sub(3);
        if cols == 0 || visible == 0 { return false; }
        let body = content(body);
        let info = layout(body,cols,visible,self.anchor);
        let last = info.total.saturating_sub(visible);
        let first = match action {
            Move::Up => info.first.saturating_sub(1),
            Move::Down => info.first.saturating_add(1).min(last),
            Move::PageUp => info.first.saturating_sub(visible),
            Move::PageDown => info.first.saturating_add(visible).min(last),
            Move::Home => 0,
            Move::End => last,
        };
        self.anchor = offset_at(body,cols,first);
        first != info.first
    }
}

/// Bounded decimal formatting. The maximum source has at most12288 lines;
/// the complete "12288-12288/12288" range occupies17 ASCII bytes.
fn decimal(out: &mut [u8; 24], length: &mut usize, mut value: usize) {
    let mut digits = [0u8; 5];
    let mut count = 0;
    loop {
        digits[count] = b'0'+(value%10) as u8;
        count += 1; value /= 10;
        if value == 0 { break; }
    }
    for digit in digits[..count].iter().rev() { out[*length] = *digit; *length += 1; }
}

fn footer(frame: &mut Frame, https: bool) {
    let full: &[u8] = if https { b"PgUp PgDn V back I import" } else { b"PgUp PgDn Home End V back" };
    let original: &[u8] = if frame.cols>=25 { full }
        else if frame.cols>=6 { if https { b"V I" } else { b"V back" } }
        else if https { b"I" } else { b"V" };
    if frame.body_rows == 0 { put(frame,frame.rows-1,original); return; }
    let mut range = [0;24]; let mut length = 0;
    decimal(&mut range,&mut length,if frame.end>frame.first { frame.first+1 } else { 0 });
    range[length] = b'-'; length += 1;
    decimal(&mut range,&mut length,frame.end);
    range[length] = b'/'; length += 1;
    decimal(&mut range,&mut length,frame.total_lines);
    let compact: &[u8] = if https { b"PgUp PgDn V I" } else { b"PgUp PgDn V" };
    let minimal: &[u8] = if https { b"V I" } else { b"V" };
    let controls = [full,compact,minimal].into_iter().find(|controls|controls.len()+1+length<=frame.cols);
    let row = frame.rows-1;
    if let Some(controls) = controls {
        put(frame,row,controls);
        let at = row*(frame.cols+1)+frame.cols-length;
        frame.bytes[at..at+length].copy_from_slice(&range[..length]);
    } else {
        // Very narrow views retain a complete existing control hint instead
        // of presenting a clipped/misleading numeric range.
        put(frame,row,original);
    }
}

fn put(frame: &mut Frame, row: usize, text: &[u8]) {
    let length = text.len().min(frame.cols);
    let at = row*(frame.cols+1);
    frame.bytes[at..at+length].copy_from_slice(&text[..length]);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reader(w: u32,h: u32) -> Reader { let mut reader=Reader::new();reader.resize(w,h);reader }
    fn row(frame: &Frame,index: usize) -> &[u8] { let at=index*(frame.cols+1);&frame.bytes[at..at+frame.cols] }
    fn valid(frame: &Frame,w: u32,h: u32) {
        assert!(frame.len<=512);assert!(frame.cols<=MAX_COLUMNS);assert!(frame.rows<=32);
        if frame.rows==0 {assert_eq!(frame.len,0);return;}
        assert_eq!(frame.len,frame.rows*(frame.cols+1)-1);
        assert!(frame.rows<=512/(frame.cols+1));assert!(frame.first<=frame.end);assert!(frame.end<=frame.total_lines);
        let factor=scale(w);assert!((frame.cols as u32+1)*ADVANCE*factor<=w.saturating_sub(2*INSET));
        assert!(TOP+GLYPH_HEIGHT*factor+(frame.rows as u32-1)*LINE*factor<=h);
        for (index,byte) in frame.bytes[..frame.len].iter().enumerate() {
            if index%(frame.cols+1)==frame.cols {assert_eq!(*byte,b'\n');}
            else {assert!((b' '..=b'~').contains(byte));}
        }
    }
    #[test] fn unknown_zero_and_unusable_geometry_produce_empty_whole_replacement() {
        let mut unknown=Reader::new();assert_eq!(unknown.render(b"body").len,0);
        for (w,h) in [(0,100),(100,0),(16,100),(22,100),(100,42),(300,49)] {
            let mut r=reader(w,h);let frame=r.render(b"body");assert_eq!(frame.len,0);assert_eq!(frame.bytes,[0;512]);assert!(!r.navigate(b"body",Move::End));
        }
    }
    #[test] fn actual_viewport_metrics_caps_and_explicit_newlines_avoid_implicit_wrap() {
        for (w,h) in [(624,706),(1280,800),(1024,768),(255,200),(256,200),(28,800),(u32::MAX,u32::MAX)] {
            let mut r=reader(w,h);let frame=r.render(&[b'x';2048]);valid(&frame,w,h);
        }
        let frame=reader(624,706).render(b"body");assert_eq!((frame.cols,frame.rows,frame.body_rows,frame.len),(47,10,7,479));
        let frame=reader(1280,800).render(b"body");assert_eq!((frame.cols,frame.rows,frame.body_rows),(47,10,7));
    }
    #[test] fn one_two_and_three_rows_keep_only_available_chrome_and_disable_hidden_scrolling() {
        for (h,rows) in [(43,1),(52,2),(61,3)] {
            let mut r=reader(160,h);let frame=r.render(b"many\nbody\nlines\nhere");assert_eq!(frame.rows,rows);assert_eq!(frame.body_rows,0);assert_eq!((frame.first,frame.end),(0,0));assert_eq!(frame.total_lines,4);assert!(!r.navigate(b"many\nbody\nlines\nhere",Move::End));valid(&frame,160,h);
            assert!(row(&frame,0).starts_with(b"Plain HTTP / no TLS"));if rows>1 {assert!(row(&frame,rows-1).starts_with(b"V back"));}
        }
        let frame=reader(28,52).render(b"body");assert_eq!(frame.cols,1);assert_eq!(row(&frame,0),b"P");assert_eq!(row(&frame,1),b"V");
    }
    #[test] fn fixed_host_path_and_footer_are_separate_from_body_controls() {
        let mut r=reader(624,706);let frame=r.render(b"hello\n\nworld");assert!(row(&frame,0).starts_with(b"Plain HTTP / no TLS"));assert!(row(&frame,1).starts_with(b"probe.sexos.test /welcome"));assert!(row(&frame,frame.rows-1).starts_with(b"PgUp PgDn Home End V back"));
        assert_eq!(frame.total_lines,3);assert!(row(&frame,2).starts_with(b"hello"));assert!(row(&frame,3).iter().all(|b|*b==b' '));assert!(row(&frame,4).starts_with(b"world"));
    }
    #[test] fn sanitizes_controls_non_ascii_and_carriage_return_without_injecting_chrome() {
        let frame=reader(624,706).render(b"a\t\rb\x00\x1bc\x7f\xc3\xa9\nnext");assert!(row(&frame,2).starts_with(b"a  b  c ??"));assert!(row(&frame,3).starts_with(b"next"));valid(&frame,624,706);
    }
    #[test] fn final_path_replaces_initial_chrome_without_injecting_rows_or_stale_bytes() {
        let mut r=reader(624,706);let final_view=r.render_path(b"final body",b"/guide/text");
        assert!(row(&final_view,1).starts_with(b"probe.sexos.test /guide/text"));
        assert!(row(&final_view,2).starts_with(b"final body"));valid(&final_view,624,706);
        let shorter=r.render_path(b"new",b"/");assert!(row(&shorter,1).starts_with(b"probe.sexos.test /"));
        assert!(row(&shorter,1)[HOST.len()+2..].iter().all(|b|*b==b' '));
        let hostile=r.render_path(b"safe",b"/a\nb\x1b\xff");assert!(row(&hostile,1).starts_with(b"probe.sexos.test /a b ?"));
        assert!(row(&hostile,2).starts_with(b"safe"));valid(&hostile,624,706);
    }
    #[test] fn hard_wrap_and_explicit_newline_boundary_do_not_double_paragraph_rows() {
        let frame=reader(82,160).render(b"0123456789\nabcdefghijk\n\nlast\n");assert_eq!(frame.cols,10);assert_eq!(frame.total_lines,5);
        assert_eq!(row(&frame,2),b"0123456789");assert_eq!(row(&frame,3),b"abcdefghij");assert_eq!(row(&frame,4),b"k         ");assert_eq!(row(&frame,5),b"          ");assert_eq!(row(&frame,6),b"last      ");
    }
    #[test] fn reads_beyond_preview64_and_end_includes_last_actual_source_byte() {
        let mut body=[b'x';300];body[220..225].copy_from_slice(b"TAIL!");body[299]=b'Z';let mut r=reader(82,160);
        let first=r.render(&body);assert_eq!((first.first,first.end),(0,11));assert!(r.navigate(&body,Move::End));let last=r.render(&body);assert_eq!((last.first,last.end,last.total_lines),(19,30,30));assert!(last.bytes[..last.len].windows(5).any(|v|v==b"TAIL!"));assert_eq!(row(&last,last.rows-2)[9],b'Z');assert!(!r.navigate(&body,Move::Down));
    }
    #[test] fn line_and_page_navigation_clamp_and_home_end_are_exact() {
        let body=[b'x';600];let mut r=reader(82,160);let page=r.render(&body).body_rows;assert_eq!(page,11);
        assert!(!r.navigate(&body,Move::Up));assert!(r.navigate(&body,Move::Down));assert_eq!(r.render(&body).first,1);
        assert!(r.navigate(&body,Move::PageDown));assert_eq!(r.render(&body).first,1+page);
        assert!(r.navigate(&body,Move::PageUp));assert_eq!(r.render(&body).first,1);assert!(r.navigate(&body,Move::Home));assert!(!r.navigate(&body,Move::Home));
        assert!(r.navigate(&body,Move::End));let last=r.render(&body);assert_eq!((last.first,last.end,last.total_lines),(49,60,60));assert!(!r.navigate(&body,Move::End));assert!(!r.navigate(&body,Move::PageDown));
    }
    #[test] fn empty_and_trailing_newlines_do_not_create_blank_tail_pages() {
        let mut r=reader(82,160);let frame=r.render(b"");assert_eq!((frame.total_lines,frame.first,frame.end),(0,0,0));assert!(!r.navigate(b"",Move::End));
        for body in [b"one\n" as &[u8],b"0123456789\n"] {let frame=r.render(body);assert_eq!(frame.total_lines,1);assert_eq!(frame.end,1);}
        let frame=r.render(b"\n");assert_eq!(frame.total_lines,0);
        let frame=r.render(b"a\n\n\n");assert_eq!(frame.total_lines,1);assert_eq!(frame.end,1);
        let mut tail=[b'\n';1024];tail[..4].copy_from_slice(b"last");tail[1000..].fill(b' ');assert!(!r.navigate(&tail,Move::End));let frame=r.render(&tail);assert_eq!((frame.total_lines,frame.first,frame.end),(1,0,1));assert!(row(&frame,2).starts_with(b"last"));
    }
    #[test] fn maximum_body_and_defensive_oversize_input_stay_bounded() {
        let mut r=reader(82,160);let body=[b'q';MAX_BODY+2];assert!(r.navigate(&body,Move::End));let frame=r.render(&body);assert_eq!(frame.total_lines,1229);assert_eq!((frame.first,frame.end),(1218,1229));assert_eq!(&row(&frame,frame.rows-2)[..8],b"qqqqqqqq");assert_eq!(&row(&frame,frame.rows-2)[8..],b"  ");valid(&frame,82,160);
        let mut paragraphs=[b'\n';MAX_BODY];paragraphs[MAX_BODY-1]=b'Z';r.navigate(&paragraphs,Move::End);let frame=r.render(&paragraphs);assert_eq!(frame.total_lines,MAX_BODY);assert_eq!(frame.end,MAX_BODY);assert_eq!(row(&frame,frame.rows-2)[0],b'Z');valid(&frame,82,160);
    }
    #[test] fn resize_aligns_old_source_anchor_to_new_wrap_and_clamps_away_blank_tail() {
        let body=[b'x';600];let mut r=reader(82,160);r.navigate(&body,Move::PageDown);r.navigate(&body,Move::Down);let before=r.render(&body);assert_eq!(r.anchor,120);assert_eq!(before.first,12);
        r.resize(142,160);let resized=r.render(&body);assert_eq!(resized.cols,20);assert_eq!(resized.first,6);assert_eq!(r.anchor,120);
        r.resize(112,160);let resized=r.render(&body);assert_eq!(resized.cols,15);assert_eq!(r.anchor,120);
        r.resize(124,160);let resized=r.render(&body);assert_eq!(resized.cols,17);assert_eq!(r.anchor,120);assert_eq!(resized.source_anchor,120);
        r.navigate(&body,Move::End);r.resize(624,706);let last=r.render(&body);assert_eq!(last.end,last.total_lines);assert_eq!(last.first,last.total_lines-last.body_rows);assert!(last.first>0);
    }
    #[test] fn clear_resets_anchor_and_shorter_replacement_erases_old_rows_with_padding() {
        let mut r=reader(82,160);r.navigate(&[b'x';600],Move::End);r.clear();let frame=r.render(b"ok");assert_eq!(r.anchor,0);assert_eq!((frame.first,frame.end),(0,1));assert_eq!(row(&frame,2),b"ok        ");for index in 3..frame.rows-1 {assert_eq!(row(&frame,index),b"          ");}
        r.resize(0,0);let empty=r.render(b"ok");assert_eq!(empty.len,0);assert_eq!(empty.bytes,[0;512]);
    }
}
