// SPDX-License-Identifier: GPL-3.0-only
//! Bounded presentation of already received plain-text data. No endpoint,
//! capability, HTML, TLS or network operation is introduced by this reader.
use sex_pdx::desktop_style::{scale, ADVANCE, INSET, LINE};
use sex_pdx::text_encoding as text;

const CAPACITY: usize = 512;
const MAX_BODY: usize = 12288;
/// Match Display's bounded whole-frame row index, including all chrome.
const MAX_ROWS: usize = 24;
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
    /// Rendered-byte selection span. Chrome and final row padding are excluded.
    pub selection: (u16, u16),
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
    let body = &body[..text::prefix(body,MAX_BODY)];
    let mut end = body.len();
    while end > 0 && matches!(body[end-1],b'\n'|b'\t'|b' ') { end -= 1; }
    &body[..end]
}

/// Consume at least one byte. A newline immediately after a full hard-wrapped
/// row terminates that same row; trailing newlines never create a blank tail.
/// Interior blank paragraphs remain visible, and every scan consumes ≤12288B.
fn line(body: &[u8], start: usize, cols: usize) -> Line {
    let mut end = start;let mut cells=0;
    while end < body.len() && cells < cols && body[end] != b'\n' {
        end=text::next(body,end);cells+=1;
    }
    let next = if body.get(end) == Some(&b'\n') { end+1 } else { end };
    Line { start, end, next }
}

/// A TAB is one blank cell; malformed bytes and unsupported controls are
/// visible replacement scalars. This changes only presentation, never source.
fn display_scalar(scalar:u32)->u32{
    if scalar==9 {32} else if scalar<32||scalar==127||(128..=159).contains(&scalar){text::REPLACEMENT}else{scalar}
}
fn encoded_row_cost(body:&[u8],row:&Line,cols:usize)->usize{
    let mut at=row.start;let mut cost=cols;
    while at<row.end {let(scalar,end)=text::cell(body,at).unwrap();let mut bytes=[0;4];
        cost+=text::encode(display_scalar(scalar),&mut bytes).unwrap()-1;at=end;}
    cost+1
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
        let rows = (physical as usize).min(MAX_ROWS).min(CAPACITY/(cols+1));
        (cols,rows)
    }

    /// Reserve complete padded rows plus the ASCII chrome/footer. A bounded
    /// source pass finds the largest serialized wrapped row, so even a page of
    /// four-byte scalars fits the same 512-byte transaction without clipping.
    fn source_geometry(&self,body:&[u8])->(usize,usize){
        let(cols,rows)=self.geometry();if cols==0||rows<=3{return(cols,rows)}
        let body=content(body);let mut cursor=0;let mut cost=cols+1;
        while cursor<body.len(){let row=line(body,cursor,cols);cost=cost.max(encoded_row_cost(body,&row,cols));cursor=row.next;}
        let body_rows=(rows-3).min((CAPACITY-(3*cols+2))/cost);
        (cols,body_rows+3)
    }

    pub fn render(&mut self, body: &[u8]) -> Frame {
        self.render_path(body,&PATH[1..])
    }
    /// The final validated request path is presentation data. Clean every
    /// byte defensively so it cannot inject a row or replace the reader chrome.
    pub fn render_path(&mut self, body: &[u8], path:&[u8]) -> Frame {
        self.render_path_selected(body,path,None)
    }
    pub fn render_path_selected(&mut self,body:&[u8],path:&[u8],selection:Option<(usize,usize)>)->Frame {
        self.render_source(body,path,b"Plain HTTP / no TLS",HOST,selection)
    }
    pub fn render_https(&mut self,body:&[u8],path:&[u8])->Frame {
        self.render_https_selected(body,path,None)
    }
    pub fn render_https_selected(&mut self,body:&[u8],path:&[u8],selection:Option<(usize,usize)>)->Frame {
        self.render_source(body,path,b"Controlled HTTPS / lab root",b"probe.sexos.test:46080",selection)
    }
    fn render_source(&mut self,body:&[u8],path:&[u8],banner:&[u8],host:&[u8],selection:Option<(usize,usize)>)->Frame {
        let body=&body[..text::prefix(body,MAX_BODY)];
        let valid_utf8=core::str::from_utf8(body).is_ok();
        let selected = selection.and_then(|(a,b)| {
            let (lo,hi)=(a.min(b),a.max(b));
            (lo<hi && hi<=body.len() && (!valid_utf8 ||
                (text::is_boundary(body,lo)&&text::is_boundary(body,hi)))).then_some((lo,hi))
        });
        let body = content(body);
        let (cols,rows) = self.source_geometry(body);
        let mut frame = Frame { bytes: [0;CAPACITY], len: 0, selection: (0,0), first: 0, end: 0,
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
                let at=row_at(&frame,shown+2);
                let mut serialized=[0;MAX_COLUMNS*4];let mut length=0;let mut cells=0;let mut offset=source.start;
                while offset<source.end {
                    let(scalar,end)=text::cell(body,offset).unwrap();let mut bytes=[0;4];
                    let n=text::encode(display_scalar(scalar),&mut bytes).unwrap();
                    serialized[length..length+n].copy_from_slice(&bytes[..n]);
                    if selected.is_some_and(|(lo,hi)|offset>=lo&&end<=hi){
                        if frame.selection==(0,0){frame.selection.0=(at+length)as u16;}
                        frame.selection.1=(at+length+n)as u16;
                    }
                    length+=n;cells+=1;offset=end;
                }
                serialized[length..length+cols-cells].fill(b' ');length+=cols-cells;
                replace_row(&mut frame,shown+2,&serialized[..length]);
                cursor = source.next;
                shown += 1;
            }
            frame.end = frame.first+shown;
        }
        if rows > 1 { footer(&mut frame,banner==b"Controlled HTTPS / lab root"); }
        frame
    }

    /// Vertical selection movement uses the same bounded source rows as paint.
    /// No retained preferred-column or body copy is introduced. Home/End remain
    /// exact raw-body endpoints, including a copyable but unpainted blank tail.
    pub fn selection_move(&self, body:&[u8], caret:usize, action:Move)->usize {
        let raw_len=text::prefix(body,MAX_BODY);
        if action==Move::Home {return 0;}
        if action==Move::End {return raw_len;}
        let (cols,rows)=self.source_geometry(body);let visible=rows.saturating_sub(3);
        if cols==0 || visible==0 {return caret.min(raw_len);}
        let body=content(body);if body.is_empty(){return 0;}
        let caret=caret.min(body.len());
        let mut cursor=0;let mut index=0usize;let mut current=0usize;let mut column=0;
        while cursor<body.len() {
            let row=line(body,cursor,cols);
            if row.start<=caret {current=index;column=text::cells(&body[row.start..caret.min(row.end)]);}
            index+=1;cursor=row.next;
        }
        let last=index.saturating_sub(1);
        let target=match action {
            Move::Up=>current.saturating_sub(1),
            Move::Down=>current.saturating_add(1).min(last),
            Move::PageUp=>current.saturating_sub(visible),
            Move::PageDown=>current.saturating_add(visible).min(last),
            Move::Home|Move::End=>current,
        };
        let row=line(body,offset_at(body,cols,target),cols);
        row.start+text::offset(&body[row.start..row.end],column)
    }

    /// Explicit selection movement can reveal its caret. Ordinary render and
    /// viewport reflow keep the previously retained source anchor unchanged.
    pub fn reveal(&mut self,body:&[u8],caret:usize)->bool {
        let (cols,rows)=self.source_geometry(body);let visible=rows.saturating_sub(3);
        if cols==0 || visible==0 {return false;}
        let body=content(body);let caret=caret.min(body.len());
        let info=layout(body,cols,visible,self.anchor);
        let mut cursor=0;let mut index=0;let mut wanted=0;
        while cursor<body.len() {
            let row=line(body,cursor,cols);
            if row.start<=caret {wanted=index;}
            index+=1;cursor=row.next;
        }
        let first=if wanted<info.first {wanted}
            else if wanted>=info.first.saturating_add(visible) {wanted+1-visible}
            else {return false;};
        let next=offset_at(body,cols,first);
        let changed=next!=self.anchor;self.anchor=next;changed
    }

    /// One HID report scrolls three visual rows per notch. At most 384 rows
    /// are requested, and layout/source lookup each traverse the bounded
    /// retained body once, independently of the step magnitude.
    pub fn scroll_wheel(&mut self, body: &[u8], wheel: i8) -> bool {
        if wheel == 0 { return false; }
        let (cols,rows) = self.source_geometry(body);
        let visible = rows.saturating_sub(3);
        if cols == 0 || visible == 0 { return false; }
        let body = content(body);
        let info = layout(body,cols,visible,self.anchor);
        let last = info.total.saturating_sub(visible);
        let delta = -(wheel as i16) * 3;
        let first = if delta < 0 { info.first.saturating_sub((-delta) as usize) }
            else { info.first.saturating_add(delta as usize).min(last) };
        self.anchor = offset_at(body,cols,first);
        first != info.first
    }

    pub fn navigate(&mut self, body: &[u8], action: Move) -> bool {
        let (cols,rows) = self.source_geometry(body);
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
        let at = row_at(frame,row)+frame.cols-length;
        frame.bytes[at..at+length].copy_from_slice(&range[..length]);
    } else {
        // Very narrow views retain a complete existing control hint instead
        // of presenting a clipped/misleading numeric range.
        put(frame,row,original);
    }
}

/// Replace only the complete ASCII footer after the final serialized LF.
/// UTF-8 body rows have variable byte widths; column arithmetic is unsafe here.
pub fn replace_footer(bytes:&mut[u8;CAPACITY],len:usize,cols:usize,label:&[u8])->bool{
    if len>CAPACITY||label.iter().any(|b|!(b' '..=b'~').contains(b)){return false}
    let Some(last)=bytes[..len].iter().rposition(|b|*b==b'\n')else{return false};let at=last+1;
    if len-at!=cols||bytes[at..len].iter().any(|b|!(b' '..=b'~').contains(b)){return false}
    bytes[at..len].fill(b' ');let n=cols.min(label.len());bytes[at..at+n].copy_from_slice(&label[..n]);true
}

fn row_at(frame:&Frame,row:usize)->usize{
    if row==0{return 0}
    frame.bytes[..frame.len].iter().enumerate().filter(|(_,b)|**b==b'\n').nth(row-1).map_or(frame.len,|(at,_)|at+1)
}
fn replace_row(frame:&mut Frame,row:usize,bytes:&[u8]){
    let start=row_at(frame,row);let end=frame.bytes[start..frame.len].iter().position(|b|*b==b'\n').map_or(frame.len,|n|start+n);
    let tail=frame.len-end;let new_len=start+bytes.len()+tail;
    debug_assert!(new_len<=CAPACITY);
    frame.bytes.copy_within(end..frame.len,start+bytes.len());
    frame.bytes[start..start+bytes.len()].copy_from_slice(bytes);frame.len=new_len;
    frame.bytes[new_len..].fill(0);
}

fn put(frame: &mut Frame, row: usize, text: &[u8]) {
    let length = text.len().min(frame.cols);
    let at = row_at(frame,row);
    frame.bytes[at..at+length].copy_from_slice(&text[..length]);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn wheel_steps_clamp_and_reverse_without_changing_retained_utf8() {
        let body=(0..1000).map(|n|format!("{n:04} café 中 😀\n")).collect::<String>();
        let original=body.as_bytes().to_vec();
        for wheel in i8::MIN..=i8::MAX {
            let mut r=reader(624,706);
            r.navigate(body.as_bytes(),Move::PageDown);
            let prior=r.render(body.as_bytes());
            let desired=(prior.first as i32-wheel as i32*3).clamp(0,(prior.total_lines-prior.body_rows)as i32)as usize;
            let changed=r.scroll_wheel(body.as_bytes(),wheel);
            let frame=r.render(body.as_bytes());
            assert_eq!(frame.first,desired);assert_eq!(changed,desired!=prior.first);
            assert!(body.is_char_boundary(r.source_anchor()));valid(&frame,624,706);
            assert_eq!(body.as_bytes(),original);
        }
        let mut r=reader(624,706);
        assert!(!r.scroll_wheel(body.as_bytes(),1));assert!(r.scroll_wheel(body.as_bytes(),-1));
        assert_eq!(r.render(body.as_bytes()).first,3);assert!(r.scroll_wheel(body.as_bytes(),1));
        assert_eq!(r.source_anchor(),0);
        r.navigate(body.as_bytes(),Move::End);assert!(!r.scroll_wheel(body.as_bytes(),i8::MIN));
        let end=r.render(body.as_bytes());assert_eq!(end.end,end.total_lines);
    }
    #[test] fn wheel_is_inert_without_visible_body_and_preserves_anchor_over_resize() {
        let body=[b'x';MAX_BODY];let mut r=reader(82,160);r.scroll_wheel(&body,-3);
        let anchor=r.source_anchor();assert_eq!(anchor,90);
        for (w,h) in [(0,0),(160,43),(160,52),(160,61)] {
            r.resize(w,h);assert!(!r.scroll_wheel(&body,-1));assert_eq!(r.source_anchor(),anchor);
        }
        r.resize(82,160);assert_eq!(r.render(&body).first,9);
        assert!(!r.scroll_wheel(&body,0));assert_eq!(r.source_anchor(),anchor);
        r.clear();assert!(!r.scroll_wheel(b"",-1));assert_eq!(r.source_anchor(),0);
    }
    fn reader(w: u32,h: u32) -> Reader { let mut reader=Reader::new();reader.resize(w,h);reader }
    fn row(frame: &Frame,index: usize) -> &[u8] { frame.bytes[..frame.len].split(|b|*b==b'\n').nth(index).unwrap() }
    fn valid(frame: &Frame,w: u32,h: u32) {
        assert!(frame.len<=512);assert!(frame.cols<=MAX_COLUMNS);assert!(frame.rows<=MAX_ROWS);
        if frame.rows==0 {assert_eq!(frame.len,0);return;}
        assert!(frame.rows<=512/(frame.cols+1));assert!(frame.first<=frame.end);assert!(frame.end<=frame.total_lines);
        let factor=scale(w);assert!((frame.cols as u32+1)*ADVANCE*factor<=w.saturating_sub(2*INSET));
        assert!(TOP+GLYPH_HEIGHT*factor+(frame.rows as u32-1)*LINE*factor<=h);
        let serialized=core::str::from_utf8(&frame.bytes[..frame.len]).unwrap();
        assert_eq!(serialized.split('\n').count(),frame.rows);
        for row in serialized.split('\n'){assert_eq!(row.chars().count(),frame.cols);assert!(row.chars().all(|c|!c.is_control()));}
        assert!(frame.bytes[frame.len..].iter().all(|b|*b==0));
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
    #[test] fn preserves_utf8_and_visibly_replaces_unsupported_controls_without_injecting_chrome() {
        let frame=reader(624,706).render(b"a\t\rb\x00\x1bc\x7f\xc3\xa9\nnext");assert!(row(&frame,2).starts_with("a \u{fffd}b\u{fffd}\u{fffd}c\u{fffd}é".as_bytes()));assert!(row(&frame,3).starts_with(b"next"));valid(&frame,624,706);
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
    #[test] fn selected_source_bytes_exclude_chrome_and_terminal_padding_and_normalize_reverse() {
        let body=b"alpha\nbeta\nlast   ";let mut r=reader(82,160);
        let frame=r.render_path_selected(body,b"/secret",Some((2,9)));
        let start=2*(frame.cols+1)+2;let end=3*(frame.cols+1)+3;
        assert_eq!(frame.selection,(start as u16,end as u16));
        assert_eq!(&frame.bytes[start..start+3],b"pha");
        assert_eq!(&frame.bytes[end-3..end],b"bet");
        assert_eq!(r.render_path_selected(body,b"/secret",Some((9,2))).selection,frame.selection);
        let all=r.render_https_selected(body,b"/",Some((0,body.len())));
        assert_eq!(all.selection,(2*(all.cols+1)as u16,(4*(all.cols+1)+4)as u16));
        // A newline-only selection has no character cell; trimmed blank tails
        // stay copyable as raw source data but do not select invented padding.
        for range in[(5,6),(15,body.len()),(1,1),(0,body.len()+1)] {
            assert_eq!(r.render_path_selected(body,b"/",Some(range)).selection,(0,0));
        }
        assert_eq!(r.render(body).selection,(0,0));
    }
    #[test] fn selected_hard_wrap_reflow_and_viewport_clipping_keep_exact_source_endpoints() {
        let body=[b'x';600];let mut r=reader(82,160);
        let narrow=r.render_path_selected(&body,b"/",Some((7,23)));
        assert_eq!(narrow.selection,((2*11+7)as u16,(4*11+3)as u16));
        assert!(r.navigate(&body,Move::Down));let anchor=r.source_anchor();
        let clip=r.render_path_selected(&body,b"/",Some((0,23)));
        assert_eq!(clip.selection,((2*11)as u16,(3*11+3)as u16));assert_eq!(r.source_anchor(),anchor);
        r.resize(142,160);let wide=r.render_https_selected(&body,b"/",Some((7,23)));
        assert_eq!(wide.selection,((2*21+7)as u16,(3*21+3)as u16));assert_eq!(r.source_anchor(),anchor);
        let off=r.render_path_selected(&body,b"/",Some((500,510)));assert_eq!(off.selection,(0,0));
        r.resize(0,0);assert_eq!(r.render_path_selected(&body,b"/",Some((0,600))).selection,(0,0));
    }
    #[test] fn selection_vertical_page_motion_matches_wrap_and_clamps_short_lines_and_raw_end() {
        let mut r=reader(82,160);let body=b"0123456789abcdefghij\nxy\n0123456789end   ";
        assert_eq!(r.selection_move(body,7,Move::Down),17);
        assert_eq!(r.selection_move(body,17,Move::Down),23); // short row's newline
        assert_eq!(r.selection_move(body,22,Move::Up),11);
        assert_eq!(r.selection_move(body,0,Move::Up),0);
        assert_eq!(r.selection_move(body,7,Move::PageDown),37); // final visible row, trimmed tail
        assert_eq!(r.selection_move(body,37,Move::PageUp),3);
        assert_eq!(r.selection_move(body,7,Move::Home),0);
        assert_eq!(r.selection_move(body,7,Move::End),body.len());
        r.resize(0,0);assert_eq!(r.selection_move(body,usize::MAX,Move::Down),body.len());
    }
    #[test] fn explicit_reveal_follows_caret_both_directions_and_normal_render_never_moves_anchor() {
        let body=[b'x';600];let mut r=reader(82,160);
        assert!(!r.reveal(&body,5));assert_eq!(r.source_anchor(),0);
        assert!(r.reveal(&body,250));assert_eq!(r.source_anchor(),150);
        let frame=r.render_path_selected(&body,b"/",Some((240,251)));
        assert_eq!((frame.first,frame.end),(15,26));assert_eq!(r.source_anchor(),150);
        assert!(!r.reveal(&body,250));assert!(r.reveal(&body,12));assert_eq!(r.source_anchor(),10);
        assert!(r.reveal(&body,600));assert_eq!(r.source_anchor(),490);
        let old=r.source_anchor();r.resize(142,160);r.render_path_selected(&body,b"/",Some((590,600)));assert_eq!(r.source_anchor(),old);
        r.resize(0,0);assert!(!r.reveal(&body,0));assert_eq!(r.source_anchor(),old);
    }

}
