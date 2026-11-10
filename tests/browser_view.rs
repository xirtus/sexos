// SPDX-License-Identifier: GPL-3.0-only
//! Standalone tests compile the actual pure Browser snapshot composer.
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
mod network_probe { #[derive(Clone,Copy)]pub enum View {Pending,Verified,Unavailable,Cancelled,Uncertain} }
mod dns_probe { #[derive(Clone,Copy)]pub enum View {Idle,Pending,Answer([u8;4]),Unavailable,Cancelled,Uncertain} }
mod http_probe { #[derive(Clone,Copy)]pub enum View {Idle,Pending,Ready,Unavailable,Cancelled,Uncertain} }
#[path="../apps/kaleidoscope/src/browser_view.rs"]mod browser_view;
use browser_view::compose;
use dns_probe::View as D;
use http_probe::View as H;
use network_probe::View as E;
const PREVIEW:&[u8]=b"Kaleidoscope\nCached / sample network text\n\nHello sample.";
#[test]fn tls_preparation_whole_extent_never_claims_authenticated_content(){
 let preview=body_text::build(&[b'z';192],&[]);
 for label in [b"preparing" as &[u8],b"hello pending; no peer",b"unavailable",b"cancelled",b"rejected",b"timed out",b"waiting receipt"]{
  let before=compose(preview.as_bytes(),E::Verified,D::Idle,H::Idle,&[0;64]).unwrap();let base=before.len;let old=before.bytes;
  let after=browser_view::with_tls(before,Some(label)).unwrap();assert_eq!(&after.bytes[..base],&old[..base]);assert_eq!(after.len,base+64);
  let mut expected=b"\n\nTLS preparation: ".to_vec();expected.extend_from_slice(label);expected.extend_from_slice(b"\nL prep Q clear Y peer");assert_eq!(&after.bytes[base..base+64],&padded(&expected));assert!(after.bytes[after.len..].iter().all(|b|*b==b' '));
 }
 for label in [b"secure" as &[u8],b"authenticated",b"hello\0"]{let text=compose(PREVIEW,E::Verified,D::Idle,H::Idle,&[0;64]).unwrap();assert!(browser_view::with_tls(text,Some(label)).is_none());}
}
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
 let mut text=compose(&[b'p';254],E::Verified,D::Idle,H::Idle,&[0;64]).unwrap();text.len=text.bytes.len()-63;assert!(browser_view::with_seed(text,Some(b"checking")).is_none());
}

#[test]fn clock_snapshot_chrome_replaces_one_auxiliary_extent_without_clipping(){
 let preview=body_text::build(&[b'z';192],&[]);
 for label in [b"2026-10-30T13:45:07Z" as &[u8],b"checking",b"unavailable",b"waiting receipt",b"rollback; blocked",b"cancelled"]{
  let before=compose(preview.as_bytes(),E::Verified,D::Idle,H::Idle,&[0;64]).unwrap();let old=before.bytes;let base=before.len;
  let after=browser_view::with_clock(before,label).unwrap();assert_eq!(&after.bytes[..base],&old[..base]);assert_eq!(after.len,base+64);assert!(after.len<=512);
  let mut expected=b"\n\nCertificate time: ".to_vec();expected.extend_from_slice(label);expected.extend_from_slice(b"\nT sample  Q clear");assert_eq!(&after.bytes[base..base+64],&padded(&expected));assert!(after.bytes[after.len..].iter().all(|b|*b==b' '));
 }
 for label in [vec![b'x';25],vec![0],vec![0xff]]{let text=compose(PREVIEW,E::Verified,D::Idle,H::Idle,&[0;64]).unwrap();assert!(browser_view::with_clock(text,&label).is_none());}
}

#[test]fn https_status_replaces_whole_extent_for_every_actual_outcome(){
 let preview=body_text::build(&[b'z';192],&[]);
 for label in [b"idle" as &[u8],b"loading",b"ready",b"cancelled",b"waiting receipt",b"certificate rejected",b"signature rejected",b"TLS rejected",b"invalid TLS record",b"invalid document",b"connection failed"]{
  let before=compose(preview.as_bytes(),E::Verified,D::Answer([10,0,2,2]),H::Cancelled,&[0;64]).unwrap();let base=before.len;let old=before.bytes;
  let after=browser_view::with_https(before,label).unwrap();assert_eq!(&after.bytes[..base],&old[..base]);assert_eq!(after.len,base+64);
  let mut expected=b"\n\nHTTPS / lab root: ".to_vec();expected.extend_from_slice(label);expected.extend_from_slice(b"\nB fetch V read Q clear");assert_eq!(&after.bytes[base..base+64],&padded(&expected));assert!(after.len<=512);
 }
 for label in [b"public web" as &[u8],b"ready\0",b"unavailable"]{let text=compose(PREVIEW,E::Verified,D::Idle,H::Cancelled,&[0;64]).unwrap();assert!(browser_view::with_https(text,label).is_none());}
}

#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/text_batch.rs"]pub mod text_batch;
#[path="../servers/silk-shell/src/linen_text_delivery.rs"]mod text_delivery;
#[path="../servers/sexdisplay/src/text_transaction.rs"]mod display_transaction;
pub const ERR_SERVICE_NOT_READY:u64=u64::MAX-1;
mod tls_probe{#[derive(Clone,Copy)]pub enum View{Idle,Preparing,HelloPending,Unavailable,Cancelled,Rejected,TimedOut,Uncertain}}

// Compile the current production Main helpers; the registered gate extracts
// them fresh, including for the compiled wrong-viewport control.
include!(env!("BROWSER_TLS_MAIN"));

fn tls_rows(f:&browser_view::TlsFrame)->Vec<&str>{if f.len==0{Vec::new()}else{core::str::from_utf8(&f.bytes[..f.len]).unwrap().split('\n').map(str::trim_end).collect()}}
#[test]fn tls_actual_small_and_expanded_tiles_have_complete_status_and_help(){
 for(w,h,cols,rows,len)in[(512,359,40,12,491),(640,375,51,9,467),(1024,718,63,8,511),(1280,750,63,8,511)]{
  let f=browser_view::tls_panel(w,h,Some(b"hello pending; no peer")).unwrap();assert!(f.complete);assert_eq!((f.cols,f.rows,f.len),(cols,rows,len));
  assert_eq!(&tls_rows(&f)[..5],&["Kaleidoscope / TLS","TLS preparation:","hello pending; no peer","Local only / no peer","L prep Q clear Y peer"]);
  assert!(tls_rows(&f)[5..].iter().all(|line|line.is_empty()));assert!(f.bytes[f.len..].iter().all(|b|*b==0));
 }
}
#[test]fn tls_every_state_replaces_old_status_with_exact_whole_lines(){
 for(label,view,code)in[(b"preparing" as &[u8],tls_probe::View::Preparing,1),(b"hello pending; no peer",tls_probe::View::HelloPending,2),(b"unavailable",tls_probe::View::Unavailable,3),(b"cancelled",tls_probe::View::Cancelled,4),(b"rejected",tls_probe::View::Rejected,5),(b"timed out",tls_probe::View::TimedOut,6),(b"waiting receipt",tls_probe::View::Uncertain,7)]{
  let f=browser_view::tls_panel(512,359,Some(label)).unwrap();assert!(f.complete);assert_eq!(tls_rows(&f)[2].as_bytes(),label);
  let m=tls_frame_metadata(&f,123,view);assert_eq!(m.count as u64,text_batch::hash(&f.bytes[..f.len]));assert_eq!((m.first,m.rows),(123,12));assert_eq!((m.selected>>26)&7,code);assert_eq!((m.selected>>29)&1,1);assert_eq!((m.selected>>30)&63,40);assert_eq!(m.selected&((1<<20)|(1<<24)|1024),0);
 }
 assert_eq!(tls_view_code(tls_probe::View::Idle),0);
}
#[test]fn tls_narrow_wrap_retains_every_logical_byte(){
 let f=browser_view::tls_panel(128,500,Some(b"hello pending; no peer")).unwrap();assert!(f.complete);assert_eq!(f.cols,17);
 let lines=tls_rows(&f);assert_eq!(&lines[..9],&["Kaleidoscope / TL","S","TLS preparation:","hello pending; no"," peer","Local only / no p","eer","L prep Q clear Y","peer"]);
}
#[test]fn tls_rim_and_height_boundaries_use_explicit_fallback(){
 for h in[56,74,92,110,119,120,127]{let f=browser_view::tls_panel(512,h,Some(b"hello pending; no peer")).unwrap();assert!(!f.complete);assert_eq!(tls_rows(&f)[0],"Enlarge tile");assert!(!core::str::from_utf8(&f.bytes[..f.len]).unwrap().contains("hello pending"));}
 let f=browser_view::tls_panel(512,128,Some(b"hello pending; no peer")).unwrap();assert!(f.complete);assert_eq!(f.rows,5);
 assert_eq!(browser_view::tls_panel(512,55,Some(b"preparing")).unwrap().len,0);
 let f=browser_view::tls_panel(256,128,Some(b"hello pending; no peer")).unwrap();assert!(!f.complete);assert_eq!(f.cols,19);
 assert!(browser_view::tls_panel(256,182,Some(b"hello pending; no peer")).unwrap().complete);
}
#[test]fn tls_all_measures_stay_inside_physical_canvas_and512_bytes(){
 for w in[0,1,15,16,17,20,40,80,127,128,255,256,512,640,1024,1280,u32::MAX]{for h in[0,1,35,36,49,50,57,58,75,76,121,122,129,130,184,359,375,768,u32::MAX]{
  let f=browser_view::tls_panel(w,h,Some(b"hello pending; no peer")).unwrap();assert!(f.len<=512&&f.cols<=63&&f.rows<=24);
  if f.rows==0{assert_eq!(f.len,0);assert!(!f.complete);continue;}
  assert_eq!(f.len,f.rows*(f.cols+1)-1);let s=desktop_style::scale(w);assert!(34+(f.rows as u32-1)*9*s+7*s<=h-8);
  for row in 0..f.rows-1{assert_eq!(f.bytes[row*(f.cols+1)+f.cols],b'\n');}assert_ne!(f.bytes[f.len-1],b'\n');assert!(f.bytes[f.len..].iter().all(|b|*b==0));
 }}
}
#[test]fn tls_unsupported_labels_never_enter_panel(){for label in[None,Some(b"peer verified" as &[u8]),Some(b"hello\npeer"),Some(b"authenticated"),Some(b"")]{assert!(browser_view::tls_panel(512,359,label).is_none());}}
#[test]fn tls_real_outbox_stage_retains_original_metadata_and_rejects_late_zoom_ack(){
 use text_delivery::{Outbox,Admission,Poll};let mut out=Outbox::atomic(343);let mut stage=display_transaction::Stage::EMPTY;
 let first=browser_view::tls_panel(512,359,Some(b"preparing")).unwrap();let old=tls_frame_metadata(&first,7,tls_probe::View::Preparing);assert!(out.queue_selected(&first.bytes[..first.len],0xcdd6f4,old,(0,0)));
 let mut deferred=None;assert_eq!(out.poll(1,|op|{deferred=Some(op);Admission::Full}),Poll::Deferred);
 assert_eq!(out.poll(1,|op|{assert_eq!(Some(op),deferred);let req=text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap();stage.receive(30,300,343,req);Admission::Accepted}),Poll::Pending);
 let mut old_paint=0;
 while old_paint==0{out.poll(4,|op|{let req=text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap();let committed=stage.receive(30,300,343,req);if let text_batch::Request::Commit{sequence,hash}=req{assert_eq!(hash,old.count as u64);assert_eq!(committed.unwrap().0,first.len);old_paint=sequence;}Admission::Accepted});}
 let next=browser_view::tls_panel(1024,718,Some(b"hello pending; no peer")).unwrap();let desired=tls_frame_metadata(&next,8,tls_probe::View::HelloPending);assert!(out.queue_selected(&next.bytes[..next.len],0xcdd6f4,desired,(0,0)));
 let wire=text_batch::Applied{surface:343,sequence:old_paint}.encode().unwrap();assert!(out.receive(2,wire).is_none());assert!(out.receive(1,text_batch::Applied{surface:344,sequence:old_paint}.encode().unwrap()).is_none());
 assert!(stage.needs_publication());stage.poll_reply(|_,_|panic!("unreleased old panel"));assert!(stage.release_after_publication());
 let mut receipt=None;stage.poll_reply(|owner,word|{assert_eq!((owner,word),(30,wire));receipt=out.receive(1,word);0});
 let receipt=receipt.unwrap();assert_eq!(receipt.metadata,old);assert_ne!(receipt.metadata,desired);assert_eq!(receipt.bytes,first.len);
 let mut new_paint=0;while new_paint==0{out.poll(4,|op|{let req=text_batch::Request::decode(op.opcode,op.packed,op.control).unwrap();let committed=stage.receive(30,300,343,req);if let text_batch::Request::Commit{sequence,..}=req{assert_eq!(committed.unwrap().0,next.len);new_paint=sequence;}Admission::Accepted});}
 assert!(out.receive(1,wire).is_none());stage.poll_reply(|_,_|panic!("unreleased resized panel"));assert!(stage.release_after_publication());
 let mut reply=None;stage.poll_reply(|owner,word|{assert_eq!((owner,word),(30,text_batch::Applied{surface:343,sequence:new_paint}.encode().unwrap()));reply=out.receive(1,word);0});assert_eq!(reply.unwrap().metadata,desired);
 // Explicit terminal cancellation does not revive the previous sequence.
 assert!(out.queue_selected(&first.bytes[..first.len],0xcdd6f4,old,(0,0)));out.cancel();assert!(out.receive(1,wire).is_none());
}


#[test]
fn live_viewport_keeps_delayed_commit_until_published_receipt_then_latest_clock() {
    use text_delivery::{Admission, Metadata, Outbox, Poll};
    let mut out = Outbox::atomic(300);
    let mut stage = display_transaction::Stage::EMPTY;
    let old_bytes = b"old viewport panel";
    let old = Metadata { count: text_batch::hash(old_bytes) as usize, first: 7, rows: 1, selected: 1 << 24 };
    assert!(out.queue(old_bytes, 0xcdd6f4, old));
    let mut delayed_commit = None;
    assert_eq!(out.poll(128, |op| {
        let req = text_batch::Request::decode(op.opcode, op.packed, op.control).unwrap();
        if let text_batch::Request::Commit { .. } = req {
            // Transport admission precedes actual Display dispatch.
            delayed_commit = Some(req);
        } else {
            assert!(stage.receive(15, 150, op.surface, req).is_none());
        }
        Admission::Accepted
    }), Poll::Pending);
    assert!(!stage.needs_publication());
    let delayed_commit = delayed_commit.unwrap();
    let old_paint = match delayed_commit { text_batch::Request::Commit { sequence, .. } => sequence, _ => unreachable!() };

    // Model three live viewport updates, each invalidating the UI cache and
    // queuing a fresh 511-byte clock panel without cancelling the active batch.
    // Payloads are transport fixtures, not pixel or clock-composer evidence.
    let mut latest_bytes = [b' '; 511];
    let mut latest = old;
    for viewport in 8..=10 {
        latest_bytes.fill(b'A' + (viewport - 8) as u8);
        latest_bytes[..18].copy_from_slice(b"Certificate time: ");
        latest = Metadata { count: text_batch::hash(&latest_bytes) as usize, first: viewport, rows: 8, selected: 1 << 24 };
        assert!(out.queue(&latest_bytes, 0xcdd6f4, latest));
        assert_eq!(out.poll(128, |_| panic!("new operation before historical Applied")), Poll::Pending);
    }
    let (len, _, bytes) = stage.receive(15, 150, 300, delayed_commit).unwrap();
    assert_eq!(&bytes[..len], old_bytes);
    stage.poll_reply(|_, _| panic!("reply before old output publication"));
    assert!(stage.release_after_publication());
    let old_word = text_batch::Applied { surface: 300, sequence: old_paint }.encode().unwrap();
    for _ in 0..3 {
        let mut attempts = 0;
        stage.poll_reply(|owner, word| {
            assert_eq!((owner, word), (15, old_word));
            attempts += 1;
            ERR_SERVICE_NOT_READY
        });
        assert_eq!(attempts, 1);
        assert!(!stage.needs_publication());
        assert_eq!(out.poll(128, |_| panic!("Full receipt must retain historical active batch")), Poll::Pending);
    }
    stage.poll_reply(|owner, word| {
        assert_eq!((owner, word), (15, old_word));
        let receipt = out.receive(1, word).unwrap();
        assert_eq!((receipt.metadata, receipt.bytes), (old, old_bytes.len()));
        0
    });
    let mut begin_count = 0;
    let mut commits = 0;
    assert_eq!(out.poll(128, |op| {
        let req = text_batch::Request::decode(op.opcode, op.packed, op.control).unwrap();
        if let text_batch::Request::Begin { sequence, .. } = req {
            assert!(sequence > old_paint);
            begin_count += 1;
        }
        if let Some((len, _, bytes)) = stage.receive(15, 150, op.surface, req) {
            assert_eq!(len, 511);
            assert_eq!(&bytes[..len], &latest_bytes);
            commits += 1;
        }
        Admission::Accepted
    }), Poll::Pending);
    assert_eq!((begin_count, commits), (1, 1));
    assert!(out.receive(1, old_word).is_none());
    stage.poll_reply(|_, _| panic!("reply before latest output publication"));
    assert!(stage.release_after_publication());
    let mut received = false;
    stage.poll_reply(|owner, word| {
        assert_eq!(owner, 15);
        let receipt = out.receive(1, word).unwrap();
        assert_eq!((receipt.metadata, receipt.bytes), (latest, 511));
        received = true;
        0
    });
    assert!(received);
    assert_eq!(out.poll(128, |_| panic!("superseded viewport reappeared")), Poll::Idle);
}
