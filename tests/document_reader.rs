// SPDX-License-Identifier: GPL-3.0-only
//! Actual pure reader, shared text metrics and BOOT profile data. No Display,
//! pixels, input routing or network authority is established by this wrapper.
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/network_config.rs"]pub mod network_config;
#[path="../apps/kaleidoscope/src/document_reader.rs"]mod document_reader;

#[test]fn https_reader_labels_actual_lab_endpoint_and_erases_when_switching_sources(){
 let mut reader=document_reader::Reader::new();reader.resize(624,706);
 let f=reader.render_https(b"authenticated body\n\x1b\xff",b"/welcome");
 assert!(f.bytes[..f.len].starts_with(b"Controlled HTTPS / lab root"));
 assert!(f.bytes[..f.len].windows(b"probe.sexos.test:46080 /welcome".len()).any(|w|w==b"probe.sexos.test:46080 /welcome"));
 assert!(f.bytes[..f.len].windows(b"authenticated body".len()).any(|w|w==b"authenticated body"));
 assert!(!f.bytes[..f.len].contains(&0x1b));assert!(!f.bytes[..f.len].contains(&0xff));
 let plain=reader.render_path(b"plain",b"/");assert!(plain.bytes[..plain.len].starts_with(b"Plain HTTP / no TLS"));assert!(!plain.bytes[..plain.len].windows(5).any(|w|w==b"46080"));
}

fn row(frame: &document_reader::Frame, index: usize) -> &[u8] {
    let start=index*(frame.cols+1);&frame.bytes[start..start+frame.cols]
}

#[test]fn tall_wide_compact_and_restored_views_keep_readable_measure_and_seven_body_rows(){
    let body=[b'x';12288];let mut reader=document_reader::Reader::new();
    for (w,h) in [(1280,750),(1024,718),(624,706)] {
        reader.resize(w,h);let frame=reader.render_https(&body,b"/welcome");
        assert_eq!((frame.cols,frame.rows,frame.body_rows,frame.len),(47,10,7,479));
        assert!(row(&frame,0).starts_with(b"Controlled HTTPS / lab root"));
        assert!(row(&frame,1).starts_with(b"probe.sexos.test:46080 /welcome"));
        assert!(row(&frame,frame.rows-1).ends_with(b"1-7/262"));
        assert!(frame.bytes[frame.len..].iter().all(|&byte|byte==0));
    }
}

#[test]fn exact_source_anchor_survives_repeated_wrap_and_unusable_resize_roundtrips(){
    let body=[b'x';600];let mut reader=document_reader::Reader::new();reader.resize(82,160);
    assert!(reader.navigate(&body,document_reader::Move::PageDown));
    assert!(reader.navigate(&body,document_reader::Move::Down));
    assert_eq!(reader.source_anchor(),120);assert_eq!(reader.render(&body).first,12);
    for _ in 0..4 {
        for (w,h) in [(124,160),(142,160),(624,706),(0,0),(82,160)] {
            reader.resize(w,h);let frame=reader.render(&body);
            assert_eq!(reader.source_anchor(),120);assert_eq!(frame.source_anchor,120);
        }
        assert_eq!(reader.render(&body).first,12); // Old real implementation drifted to110.
    }
    assert!(reader.navigate(&body,document_reader::Move::Home));
    assert_eq!(reader.source_anchor(),0);
}

#[test]fn footer_reports_actual_visible_range_end_home_and_empty_without_extra_row(){
    let body=[b'x';600];let mut reader=document_reader::Reader::new();reader.resize(624,706);
    let first=reader.render(&body);assert_eq!((first.first,first.end,first.total_lines),(0,7,13));
    assert!(row(&first,9).starts_with(b"PgUp PgDn Home End V back"));
    assert!(row(&first,9).ends_with(b"1-7/13"));
    assert!(reader.navigate(&body,document_reader::Move::End));let end=reader.render(&body);
    assert_eq!((end.first,end.end,end.total_lines),(6,13,13));assert!(row(&end,9).ends_with(b"7-13/13"));
    assert!(reader.navigate(&body,document_reader::Move::Home));let home=reader.render(&body);
    assert!(row(&home,9).ends_with(b"1-7/13"));reader.clear();
    let empty=reader.render(b"");assert!(row(&empty,9).ends_with(b"0-0/0"));
    assert_eq!((empty.rows,empty.body_rows,empty.len),(10,7,479));
}

#[test]fn short_tiles_preserve_truthful_lab_header_complete_controls_and_numeric_range(){
    let body=[b'x';2048];let mut reader=document_reader::Reader::new();
    reader.resize(160,100);let frame=reader.render_https(&body,b"/welcome");
    assert_eq!((frame.cols,frame.rows,frame.body_rows),(23,7,4));
    assert!(row(&frame,0).starts_with(b"HTTPS / lab root"));
    assert!(row(&frame,6).starts_with(b"PgUp PgDn V I"));
    assert!(row(&frame,6).ends_with(b"1-4/90"));
    reader.resize(256,118);let frame=reader.render_https(&body,b"/welcome");
    assert_eq!(frame.cols,19);assert!(row(&frame,0).starts_with(b"HTTPS / lab root"));
    assert!(row(&frame,frame.rows-1).starts_with(b"V I"));
    assert!(row(&frame,frame.rows-1).ends_with(b"1-1/108"));
    // With no visible body rows the old complete short-height controls remain.
    reader.resize(160,61);let frame=reader.render_https(&body,b"/welcome");
    assert_eq!(frame.body_rows,0);assert!(row(&frame,0).starts_with(b"HTTPS / lab root"));
    assert!(row(&frame,2).starts_with(b"V I"));
}

#[test]fn maximum_line_numbers_are_bounded_and_tiny_footer_does_not_clip_a_range(){
    let mut body=[b'\n';12288];body[12287]=b'Z';let mut reader=document_reader::Reader::new();
    reader.resize(624,706);reader.navigate(&body,document_reader::Move::End);let frame=reader.render_https(&body,b"/welcome");
    assert_eq!((frame.first,frame.end,frame.total_lines),(12281,12288,12288));
    assert!(row(&frame,frame.rows-1).ends_with(b"12282-12288/12288"));
    assert!(row(&frame,frame.rows-1).starts_with(b"PgUp PgDn V back I import"));
    reader.resize(82,160);let narrow=reader.render_https(&body,b"/welcome");
    assert_eq!(narrow.cols,10);assert!(row(&narrow,narrow.rows-1).starts_with(b"V I"));
    assert!(!row(&narrow,narrow.rows-1).contains(&b'/'));
    assert!(narrow.len<=512);assert_eq!(reader.source_anchor(),frame.source_anchor);
}

#[test]fn shorter_and_cleared_body_remove_previous_progress_body_and_trust_text(){
    let body=[b'x';12288];let mut reader=document_reader::Reader::new();reader.resize(624,706);
    reader.navigate(&body,document_reader::Move::End);reader.render_https(&body,b"/welcome");
    reader.clear();let small=reader.render_path(b"ok",b"/");assert_eq!(small.source_anchor,0);
    assert!(row(&small,0).starts_with(b"Plain HTTP / no TLS"));assert!(row(&small,9).ends_with(b"1-1/1"));
    assert!(row(&small,2).starts_with(b"ok"));assert!(row(&small,2)[2..].iter().all(|&b|b==b' '));
    for line in 3..9 {assert!(row(&small,line).iter().all(|&b|b==b' '));}
    assert!(!small.bytes[..small.len].windows(8).any(|w|w==b"lab root"));
    assert!(small.bytes[small.len..].iter().all(|&byte|byte==0));
}

fn utf8_rows(frame:&document_reader::Frame)->Vec<&str>{
 assert!(frame.len<=512);assert!(frame.bytes[frame.len..].iter().all(|b|*b==0));
 let text=core::str::from_utf8(&frame.bytes[..frame.len]).unwrap();let rows:Vec<_>=text.split('\n').collect();
 assert_eq!(rows.len(),frame.rows);for row in &rows{assert_eq!(row.chars().count(),frame.cols);assert!(row.chars().all(|c|!c.is_control()));}rows
}
#[test]fn utf8_wrap_counts_scalars_selection_counts_original_and_serialized_bytes(){
 let body="é中😀abcdeé中\nZ".as_bytes();let original=body.to_vec();let mut r=document_reader::Reader::new();r.resize(82,160);
 let f=r.render_https_selected(body,b"/welcome",Some((2,9)));let rows=utf8_rows(&f);assert_eq!(f.cols,10);
 assert!(rows[2].starts_with("é中😀abcdeé中"));assert!(rows[3].starts_with('Z'));assert_eq!(f.total_lines,2);
 let start=2*11+2;assert_eq!(f.selection,(start as u16,(start+7)as u16));assert_eq!(&f.bytes[start..start+7],"中😀".as_bytes());
 assert_eq!(body,original);assert_eq!(r.render_https_selected(body,b"/",Some((3,9))).selection,(0,0));
}
#[test]fn utf8_navigation_reflow_and_vertical_caret_keep_source_byte_boundaries(){
 let body="é中😀abcdefghijk\nxy\né中😀last".repeat(15);let bytes=body.as_bytes();let mut r=document_reader::Reader::new();r.resize(82,160);
 assert!(r.navigate(bytes,document_reader::Move::PageDown));let anchor=r.source_anchor();assert!(text_encoding::is_boundary(bytes,anchor));
 for width in [142,624,82]{r.resize(width,160);let f=r.render_https(bytes,b"/welcome");utf8_rows(&f);assert_eq!(r.source_anchor(),anchor);}
 r.resize(82,160);let to=r.selection_move(bytes,9,document_reader::Move::Down);assert!(text_encoding::is_boundary(bytes,to));
 assert!(r.navigate(bytes,document_reader::Move::End));let f=r.render_https(bytes,b"/");utf8_rows(&f);assert_eq!(f.end,f.total_lines);
}
#[test]fn maximum_four_byte_text_and_invalid_readonly_bytes_produce_complete_bounded_frames(){
 let body="😀".repeat(3072);let original=body.clone();let mut r=document_reader::Reader::new();r.resize(624,706);
 let f=r.render_https(body.as_bytes(),b"/welcome");let rows=utf8_rows(&f);assert_eq!(f.body_rows,1);assert!(rows[2].starts_with('😀'));
 assert!(r.navigate(body.as_bytes(),document_reader::Move::End));let f=r.render_https(body.as_bytes(),b"/");utf8_rows(&f);assert_eq!(f.end,f.total_lines);assert_eq!(body,original);
 let raw=b"a\xff\xc0\x80\0\r\xc2\x80\tZ";let f=r.render(raw);let rows=utf8_rows(&f);assert!(rows[2].starts_with("a������ Z"));assert_eq!(raw,b"a\xff\xc0\x80\0\r\xc2\x80\tZ");
}
#[test]fn scalar_at_byte_limit_is_never_partially_retained_in_a_frame_or_source_endpoint(){
 let mut body=vec![b'x';12287];body.extend_from_slice("😀".as_bytes());let mut r=document_reader::Reader::new();r.resize(82,160);
 assert_eq!(r.selection_move(&body,0,document_reader::Move::End),12287);r.navigate(&body,document_reader::Move::End);let f=r.render(&body);utf8_rows(&f);
 assert!(!f.bytes[..f.len].windows(3).any(|b|b=="�".as_bytes()));assert_eq!(body.len(),12291);
}
#[test]fn clipboard_footer_rewrite_uses_serialized_offset_and_preserves_selected_utf8_body(){
 let body="é中😀".repeat(80);let mut reader=document_reader::Reader::new();reader.resize(624,706);
 let mut frame=reader.render_https_selected(body.as_bytes(),b"/",Some((2,30)));utf8_rows(&frame);
 let footer=frame.bytes[..frame.len].iter().rposition(|b|*b==b'\n').unwrap()+1;
 assert_ne!(footer,(frame.rows-1)*(frame.cols+1));let before=frame.bytes;let selection=frame.selection;
 assert!(document_reader::replace_footer(&mut frame.bytes,frame.len,frame.cols,b"COPIED / CTRL A C"));
 assert_eq!(&frame.bytes[..footer],&before[..footer]);assert_eq!(frame.selection,selection);assert!((selection.1 as usize)<footer);
 let rows=utf8_rows(&frame);assert!(rows.last().unwrap().starts_with("COPIED / CTRL A C"));
 let before=frame.bytes;assert!(!document_reader::replace_footer(&mut frame.bytes,frame.len,frame.cols,"é".as_bytes()));assert_eq!(frame.bytes,before);
 assert!(!document_reader::replace_footer(&mut frame.bytes,513,frame.cols,b"label"));assert_eq!(frame.bytes,before);
}

#[test]fn tall_narrow_mixed_utf8_caps_the_whole_frame_and_caret_selection_at_display_rows(){
 let body="éabcdefghi\n".repeat(100);let bytes=body.as_bytes();let mut reader=document_reader::Reader::new();reader.resize(82,2000);
 let frame=reader.render_https_selected(bytes,b"/welcome",Some((0,602)));utf8_rows(&frame);assert_eq!((frame.rows,frame.body_rows),(24,21));
 assert!(reader.reveal(bytes,602));let frame=reader.render_https_selected(bytes,b"/welcome",Some((0,602)));utf8_rows(&frame);
 assert_eq!((frame.rows,frame.body_rows,frame.end),(24,21,51));assert_eq!(reader.source_anchor()%12,0);
 assert!(frame.selection.0<frame.selection.1);assert!((frame.selection.1 as usize)<frame.len);
 let serialized=core::str::from_utf8(&frame.bytes[..frame.len]).unwrap();assert!(serialized.is_char_boundary(frame.selection.0 as usize));assert!(serialized.is_char_boundary(frame.selection.1 as usize));
 assert_eq!(reader.selection_move(bytes,602,document_reader::Move::Up),590);
}
#[test]fn valid_utf8_mid_scalar_selection_endpoints_reject_the_whole_visual_range(){
 let mut reader=document_reader::Reader::new();reader.resize(82,160);let body="é中😀z".as_bytes();
 for range in [(1,2),(0,1),(2,4),(4,2),(5,7),(1,10)]{assert_eq!(reader.render_https_selected(body,b"/",Some(range)).selection,(0,0),"{range:?}");}
 let frame=reader.render_https_selected(body,b"/",Some((2,9)));assert_eq!(&frame.bytes[frame.selection.0 as usize..frame.selection.1 as usize],"中😀".as_bytes());
}
