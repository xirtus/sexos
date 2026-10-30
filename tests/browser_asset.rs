// SPDX-License-Identifier: GPL-3.0-only
//! Actual finite decode/view functions. Asset fixture supplies private byte
//! custody only; real TLS and positive RELEASE are tested by resource_peer.
#![allow(dead_code)]
extern crate alloc;
extern crate self as sex_graphics;
extern crate self as sex_tls;
extern crate self as sex_pdx;
#[path="../crates/sex-graphics/src/qoi.rs"] pub mod qoi;
#[path="../crates/sex-pdx/src/bitmap_batch.rs"] pub mod bitmap_batch;
#[path="../crates/sex-pdx/src/desktop_style.rs"] pub mod desktop_style;
#[path="../apps/kaleidoscope/src/asset_decode.rs"] mod asset_decode;
#[path="../apps/kaleidoscope/src/asset_view.rs"] mod asset_view;
pub const MAX_DOCUMENT:usize=12_288;
pub struct Asset(Vec<u8>);
impl Asset {pub fn as_bytes(&self)->&[u8]{&self.0}}
impl Drop for Asset {fn drop(&mut self){for byte in &mut self.0{unsafe{core::ptr::write_volatile(byte,0)}}}}
fn source(w:u32,h:u32)->Asset {
    let mut bytes=b"qoif".to_vec();bytes.extend(w.to_be_bytes());bytes.extend(h.to_be_bytes());bytes.extend([4,0]);
    for n in 0..w*h{bytes.extend([255,n as u8,(n>>4)as u8,255-(n as u8),[0,128,255][n as usize%3]]);}
    bytes.extend([0,0,0,0,0,0,0,1]);Asset(bytes)
}
#[test] fn maximum_image_stays_private_for_seventeen_turns_then_drops_wire_body() {
    let mut image=asset_decode::Image::new();let body=source(48,48);let wire=body.as_bytes().len();
    image.begin(body,7).unwrap();assert_eq!(image.wire_bytes,wire);assert_eq!(image.ticket,7);
    assert_eq!(image.allocated_payload_bytes(),MAX_DOCUMENT+qoi::MAX_PIXELS*4);
    for _ in 0..17 {assert_eq!(image.advance(),Ok(false));assert!(image.pixels().is_empty());assert!(image.pending());}
    assert_eq!(image.advance(),Ok(true));assert!(!image.pending());assert_eq!(image.pixels().len(),2304);
    assert_eq!(image.allocated_payload_bytes(),9216);
    for(n,&pixel)in image.pixels().iter().enumerate(){assert_eq!(pixel,([0u32,128,255][n%3]<<24)|((n as u8 as u32)<<16)|(((n>>4)as u8 as u32)<<8)|(255-n as u8)as u32);}
    assert_eq!(image.advance(),Ok(false));image.clear();assert_eq!(image.allocated_payload_bytes(),0);assert!(image.pixels().is_empty());assert_eq!((image.width,image.height,image.ticket,image.wire_bytes,image.wire_hash),(0,0,0,0,0));
}
#[test] fn malformed_final_marker_never_exposes_a_decoded_prefix() {
    let mut body=source(48,48);*body.0.last_mut().unwrap()=0;
    let mut image=asset_decode::Image::new();image.begin(body,9).unwrap();
    for _ in 0..17{assert_eq!(image.advance(),Ok(false));assert!(image.pixels().is_empty());}
    assert_eq!(image.advance(),Err(()));assert_eq!(image.allocated_payload_bytes(),0);assert!(image.pixels().is_empty());assert!(!image.pending());
}
#[test] fn cancel_and_invalid_replacement_erase_old_ready_and_partial_custody() {
    let mut image=asset_decode::Image::new();image.begin(source(2,2),10).unwrap();assert_eq!(image.advance(),Ok(true));
    assert_eq!(image.begin(source(49,1),11),Err(()));assert_eq!(image.allocated_payload_bytes(),0);assert!(image.pixels().is_empty());
    image.begin(source(48,48),12).unwrap();image.advance().unwrap();image.clear();assert!(!image.pending());assert!(image.pixels().is_empty());assert_eq!(image.allocated_payload_bytes(),0);
    image.begin(source(1,1),13).unwrap();assert_eq!(image.advance(),Ok(true));assert_eq!(image.pixels(),[0x000000ff]);
}
