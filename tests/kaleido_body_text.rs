#[path="../apps/kaleidoscope/src/body_text.rs"] mod body_text;
fn text(body:&[u8],summary:&[u8])->String {String::from_utf8(body_text::build(body,summary).as_bytes().to_vec()).unwrap()}
#[test] fn title_status_and_body_have_explicit_lines_without_offset_padding(){
    assert_eq!(text(b"Hello SexOS HTTP OK",b"HTTP 200"),"Kaleidoscope\nCached / sample network text\n\nHello SexOS HTTP OK");
}
#[test] fn empty_data_has_honest_empty_state(){assert!(text(b"",b"").ends_with("No text available."));}
#[test] fn cached_body_wins_and_summary_is_fallback(){
    assert!(text(b"A cached page",b"Other text").ends_with("A cached page"));
    assert!(text(b" \r\n\0",b"Network reply").ends_with("Network reply"));
}
#[test] fn network_controls_cannot_inject_layout_or_invisible_glyphs(){
    let preview=text(b"  Hello\r\n\tworld\0\x1b\x7f!\xff  ",b"");
    assert_eq!(preview,"Kaleidoscope\nCached / sample network text\n\nHello world !?");
    assert_eq!(preview.bytes().filter(|&b|b==b'\n').count(),3);
}
#[test] fn oversized_data_is_bounded_and_marked_truncated(){
    let output=text(&[b'x';1024],b"");
    assert!(output.len()<=body_text::CAPACITY);
    assert!(output.ends_with("..."));
    assert_eq!(output.split("\n\n").last().unwrap().len(),96);
    assert!(body_text::CAPACITY<=256,"fits existing offset encoding");
}
#[test] fn chunk_transport_roundtrip_preserves_newline_layout(){
    let preview=body_text::build(&[b'x';64],b"");
    let mut buffer=[0u8;256];
    for (index,chunk) in preview.as_bytes().chunks(8).enumerate(){
        let mut packed=0u64;for (i,&b) in chunk.iter().enumerate(){packed|=(b as u64)<<(8*i);}
        let args=(index*8)as u64|((chunk.len()as u64)<<8)|(0x00CDD6F4u64<<32);
        let offset=(args&255)as usize;let count=((args>>8)&15)as usize;
        for i in 0..count {buffer[offset+i]=(packed>>(8*i))as u8;}
    }
    assert_eq!(&buffer[..preview.as_bytes().len()],preview.as_bytes());
}
