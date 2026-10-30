//! Actual shared UTF-8 data helpers; no transport, authority or shaping claim.
#[path="../crates/sex-pdx/src/text_encoding.rs"] mod text_encoding;
use text_encoding::*;
#[test]fn editable_scalars_and_lf_tab_are_whole_text_but_other_c0_c1_are_not(){
 assert!(valid_text("ASCII\né€𐍈\t漢\u{fffd}".as_bytes()));assert!(valid_text(b""));
 for scalar in 0..=159 {let mut out=[0;4];let bytes=char::from_u32(scalar).unwrap().encode_utf8(&mut out).as_bytes();assert_eq!(valid_text(bytes),scalar==9||scalar==10||scalar>=32&&scalar!=127&&scalar<128,"{scalar}");}
 for bad in [&b"\x80"[..],&b"\xc0\xaf"[..],&b"\xc1\xbf"[..],&b"\xe2\x82"[..],&b"\xed\xa0\x80"[..],&b"\xf4\x90\x80\x80"[..]] {assert!(!valid_text(bad));}
}
#[test]fn actual_two_three_four_byte_cells_boundaries_prefix_and_offsets_are_byte_exact(){
 let text="Aé€𐍈\u{fffd}Z";let bytes=text.as_bytes();let mut boundaries=vec![0];let mut at=0;
 for c in text.chars(){let (value,end)=cell(bytes,at).unwrap();assert_eq!(value,c as u32);assert_eq!(next(bytes,at),end);assert_eq!(previous(bytes,end),at);boundaries.push(end);at=end;}
 assert_eq!(cells(bytes),text.chars().count());assert_eq!(cell(bytes,bytes.len()),None);
 for budget in 0..=bytes.len()+4 {let expected=*boundaries.iter().filter(|&&x|x<=budget).last().unwrap();assert_eq!(prefix(bytes,budget),expected);}
 for (count,&position)in boundaries.iter().enumerate(){assert_eq!(offset(bytes,count),position);assert!(is_boundary(bytes,position));}
 for position in 0..=bytes.len()+1 {assert_eq!(is_boundary(bytes,position),boundaries.contains(&position));}
 assert_eq!(offset(bytes,usize::MAX.min(20)),bytes.len());assert_eq!(previous(bytes,0),0);
}
#[test]fn scalar_encoder_preserves_exact_bytes_and_rejects_non_scalars(){
 for c in ['A','é','€','𐍈','漢','\u{fffd}','\u{10ffff}'] {let mut actual=[0;4];let n=encode(c as u32,&mut actual).unwrap();let mut expected=[0;4];assert_eq!(&actual[..n],c.encode_utf8(&mut expected).as_bytes());}
 for scalar in [0xd800,0xdfff,0x110000,u32::MAX] {assert_eq!(encode(scalar,&mut[0;4]),None);}
}
#[test]fn malformed_read_only_cells_progress_without_repairing_input(){
 let bytes=[0xff,b'A',0xe2,0x82];let original=bytes;let mut at=0;let mut count=0;
 while let Some((_,end))=cell(&bytes,at){assert!(end>at&&end<=bytes.len());at=end;count+=1;}
 assert_eq!(count,4);assert_eq!(bytes,original);assert_eq!(cells(&bytes),4);assert_eq!(prefix(&bytes,3),3);
}
