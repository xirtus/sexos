// SPDX-License-Identifier: GPL-3.0-only
//! Standalone tests compile the actual pure Browser snapshot composer.
mod network_probe { #[derive(Clone,Copy)]pub enum View {Pending,Verified,Unavailable,Cancelled,Uncertain} }
mod dns_probe { #[derive(Clone,Copy)]pub enum View {Idle,Pending,Answer([u8;4]),Unavailable,Cancelled,Uncertain} }
mod http_probe { #[derive(Clone,Copy)]pub enum View {Idle,Pending,Ready,Unavailable,Cancelled,Uncertain} }
#[path="../apps/kaleidoscope/src/browser_view.rs"]mod browser_view;
use browser_view::compose;
use dns_probe::View as D;
use http_probe::View as H;
use network_probe::View as E;
const PREVIEW:&[u8]=b"Kaleidoscope\nCached / sample network text\n\nHello sample.";
fn padded(bytes:&[u8])->[u8;64] { let mut out=[b' ';64];out[..bytes.len()].copy_from_slice(bytes);out }

#[test]fn preserved_preview_and_exact_echo_status_extents() {
 for(view,label)in[(E::Pending,b"\n\nPeer probe: checking\nR retry  C cancel" as &[u8]),(E::Verified,b"\n\nPeer probe: bytes verified\nR retry  C cancel"),(E::Unavailable,b"\n\nPeer probe: unavailable\nR retry  C cancel"),(E::Cancelled,b"\n\nPeer probe: cancelled\nR retry  C cancel"),(E::Uncertain,b"\n\nPeer probe: waiting receipt\nR retry  C cancel")] {
  let text=compose(PREVIEW,view,D::Idle,H::Idle,&[0;64]).unwrap();assert_eq!(&text.bytes[..PREVIEW.len()],PREVIEW);assert_eq!(&text.bytes[PREVIEW.len()..PREVIEW.len()+64],&padded(label));
 }
}
#[test]fn exact_dns_regions_preserve_legacy_offsets_and_padding() {
 for(view,label)in[(D::Idle,b"idle" as &[u8]),(D::Pending,b"checking"),(D::Unavailable,b"unavailable"),(D::Cancelled,b"cancelled"),(D::Uncertain,b"waiting receipt")] {
  let mut expected=b"\n\nControlled DNS: ".to_vec();expected.extend_from_slice(label);expected.extend_from_slice(b"\nD query  X cancel");
  let text=compose(PREVIEW,E::Unavailable,view,H::Idle,&[0;64]).unwrap();let start=PREVIEW.len()+64;assert_eq!(&text.bytes[start..start+64],&padded(&expected));
 }
}
#[test]fn dotted_decimal_is_exact_at_each_digit_boundary() {
 for(address,label)in[([0,9,10,99],"0.9.10.99"),([100,255,0,1],"100.255.0.1"),([10,0,2,2],"10.0.2.2")] {
  let text=compose(PREVIEW,E::Unavailable,D::Answer(address),H::Idle,&[0;64]).unwrap();let expected=format!("\n\nControlled DNS: {label}\nD query  X cancel");let start=PREVIEW.len()+64;assert_eq!(&text.bytes[start..start+64],&padded(expected.as_bytes()));
 }
}
#[test]fn ready_contains_complete_label_and_all64_body_bytes() {
 let body=*b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
 let text=compose(PREVIEW,E::Verified,D::Answer([10,0,2,2]),H::Ready,&body).unwrap();let start=PREVIEW.len()+128;
 assert_eq!(&text.bytes[start..start+64],&padded(b"\n\nPlain HTTP / no TLS: ready\nH fetch  J cancel  V read"));assert_eq!(&text.bytes[start+64..start+66],b"\n\n");assert_eq!(&text.bytes[start+66..text.len],&body);
}
#[test]fn every_nonready_state_clears_entire_old_body() {
 for(view,label)in[(H::Idle,"idle"),(H::Pending,"checking"),(H::Unavailable,"unavailable"),(H::Cancelled,"cancelled"),(H::Uncertain,"waiting receipt")] {
  let text=compose(PREVIEW,E::Unavailable,D::Idle,view,&[0xff;64]).unwrap();let start=PREVIEW.len()+128;let expected=format!("\n\nPlain HTTP / no TLS: {label}\nH fetch  J cancel  V read");assert_eq!(&text.bytes[start..start+64],&padded(expected.as_bytes()));assert_eq!(&text.bytes[text.len-64..text.len],&[b' ';64]);
 }
}
#[test]fn exact_capacity_and_all_unused_bytes_are_spaces() {
 for length in[0,105,254] {let preview=vec![b'p';length];let text=compose(&preview,E::Pending,D::Idle,H::Idle,&[0;64]).unwrap();assert_eq!(text.len,length+258);assert!(text.len<=512);assert!(text.bytes[text.len..].iter().all(|b|*b==b' '));}
 assert!(compose(&[b'p';255],E::Pending,D::Idle,H::Idle,&[0;64]).is_none());
}
#[test]fn rejects_nonprintable_preview_and_ready_body() {
 for byte in[0,9,13,31,127,128,255] {
  assert!(compose(&[byte],E::Pending,D::Idle,H::Idle,&[0;64]).is_none());let mut body=[b' ';64];body[63]=byte;assert!(compose(PREVIEW,E::Pending,D::Idle,H::Ready,&body).is_none());
 }
}
#[test]fn printable_ascii_and_newline_remain_exact_presentation_data() {
 let body=padded(b"<title>literal & text</title>\nH fetch / ?");let text=compose(b"preview\n",E::Cancelled,D::Cancelled,H::Ready,&body).unwrap();assert_eq!(&text.bytes[text.len-64..text.len],&body);assert!(text.bytes[..text.len].iter().all(|b|*b==b'\n'||(32..=126).contains(b)));
}

#[path="../apps/kaleidoscope/src/body_text.rs"]mod body_text;
#[test]fn seed_status_extent_preserves_idle_and_fits_actual_max_preview(){
 let preview=body_text::build(&[b'z';192],&[]);
 for label in [b"checking" as &[u8],b"ready 32 bytes",b"unavailable",b"cancelled",b"waiting receipt"]{
  let before=compose(preview.as_bytes(),E::Verified,D::Answer([255;4]),H::Ready,&[b'x';64]).unwrap();
  let base=before.len;let old=before.bytes;
  let idle=browser_view::with_seed(before,None).unwrap();assert_eq!(idle.bytes,old);assert_eq!(idle.len,base);
  let text=browser_view::with_seed(idle,Some(label)).unwrap();assert_eq!(&text.bytes[..base],&old[..base]);assert_eq!(text.len,base+64);assert!(text.len<=512);
  let mut expected=b"\n\nPrivate seed: ".to_vec();expected.extend_from_slice(label);expected.extend_from_slice(b"\nE seed  Q clear");assert_eq!(&text.bytes[base..base+64],&padded(&expected));assert!(text.bytes[text.len..].iter().all(|b|*b==b' '));
 }
}
#[test]fn seed_status_rejects_unknown_labels_or_capacity_without_truncation(){
 for label in [b"secret bytes" as &[u8],b"ready\0",b"READY"]{let text=compose(PREVIEW,E::Verified,D::Idle,H::Idle,&[0;64]).unwrap();assert!(browser_view::with_seed(text,Some(label)).is_none());}
 let text=compose(&[b'p';254],E::Verified,D::Idle,H::Idle,&[0;64]).unwrap();assert!(browser_view::with_seed(text,Some(b"checking")).is_none());
}
