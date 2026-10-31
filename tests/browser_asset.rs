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

fn frame_row(frame:&asset_view::Frame,row:usize)->&[u8]{
    let at=row*(frame.cols+1);&frame.bytes[at..at+frame.cols]
}
#[test]fn image_modes_center_complete_pixels_in_the_same_reserved_body(){
    use asset_view::{geometry,Mode};
    let fit=geometry(1280,750,48,48,Mode::Fit).unwrap();
    let actual=geometry(1280,750,48,48,Mode::Actual).unwrap();
    assert_eq!(fit,bitmap_batch::Geometry{x:544,y:333,width:48,height:48,scale:4});
    assert_eq!(actual,bitmap_batch::Geometry{x:616,y:405,width:48,height:48,scale:1});
    // Both centers agree; no stretching or fractional resampling is introduced.
    assert_eq!((fit.x as u32*2+48*4,fit.y as u32*2+48*4),(1280,858));
    assert_eq!((actual.x as u32*2+48,actual.y as u32*2+48),(1280,858));
    assert_eq!(format!("{:?}",Mode::Fit),"Fit");assert_eq!(format!("{:?}",Mode::Actual),"Actual");
}
#[test]fn fit_uses_both_axes_and_actual_never_crops_when_the_image_does_not_fit(){
    use asset_view::{geometry,Mode};
    assert_eq!(geometry(512,219,48,48,Mode::Fit).unwrap().scale,1);
    assert_eq!(geometry(512,219,48,48,Mode::Actual).unwrap().scale,1);
    assert_eq!(geometry(512,400,48,24,Mode::Fit).unwrap().scale,4);
    assert_eq!(geometry(240,150,48,24,Mode::Fit).unwrap().scale,2);
    for mode in[Mode::Fit,Mode::Actual]{
        assert!(geometry(63,700,48,48,mode).is_none());
        assert!(geometry(512,171,48,48,mode).is_none());
        assert!(geometry(512,172,48,48,mode).is_some());
        assert!(geometry(256,189,48,48,mode).is_none());
        assert!(geometry(256,190,48,48,mode).is_some());
    }
}
#[test]fn every_admitted_geometry_is_centered_bounded_and_has_representable_coordinates(){
    use asset_view::{geometry,chrome_rows,Mode};
    for w in[0,8,48,64,80,120,195,196,255,256,388,504,512,640,1280,65535,131000,u32::MAX]{
        for h in[0,20,80,120,171,172,189,190,219,712,65535,131000,u32::MAX]{
            for(iw,ih)in[(1,1),(48,48),(3,47),(47,3)]{for mode in[Mode::Fit,Mode::Actual]{
                if let Some(g)=geometry(w,h,iw,ih,mode){
                    let top=36+chrome_rows(w)as u32*9*desktop_style::scale(w)+8;
                    let(left,right)=(g.x as u32-8,w-8-(g.x as u32+iw as u32*g.scale as u32));
                    let(above,below)=(g.y as u32-top,h-8-(g.y as u32+ih as u32*g.scale as u32));
                    assert!(g.valid());assert!(left.abs_diff(right)<=1);assert!(above.abs_diff(below)<=1);
                    if mode==Mode::Actual{assert_eq!(g.scale,1);}
                }
            }}
        }
    }
    for mode in[Mode::Fit,Mode::Actual]{
        assert!(geometry(u32::MAX,u32::MAX,48,48,mode).is_none());
        for(w,h)in[(0,2),(49,2),(1,49),(usize::MAX,1)]{assert!(geometry(1000,800,w,h,mode).is_none());}
    }
}
#[test]fn complete_status_metrics_and_controls_fit_supported_wide_and_compact_tiles(){
    use asset_view::{render,Status,Mode};
    for(w,h)in[(512,219),(504,219),(624,706),(1280,750)]{for mode in[Mode::Fit,Mode::Actual]{
        let f=render(w,h,Status::Ready,mode,48,48);assert_eq!(asset_view::chrome_rows(w),4);
        assert!(frame_row(&f,0).starts_with(b"QOI / controlled HTTPS lab root"));
        assert!(frame_row(&f,1).starts_with(b"Image ready"));
        let metrics=match(mode,w){(Mode::Fit,512|504)=>b"Fit 48x48 / 1x".as_slice(),(Mode::Fit,_)=>b"Fit 48x48 / 4x",_=>b"1:1 48x48 / 1x"};
        assert!(frame_row(&f,2).starts_with(metrics));
        assert!(frame_row(&f,3).starts_with(b"F Fit  A 1:1  G fetch  Q clear"));
        for row in 4..f.rows{assert!(frame_row(&f,row).iter().all(|&byte|byte==b' '));}
    }}
}
#[test]fn narrow_views_reserve_both_complete_help_rows_and_show_truthful_no_fit(){
    use asset_view::{render,Status,Mode};
    let f=render(256,219,Status::Ready,Mode::Actual,48,48);
    assert_eq!(f.cols,19);assert_eq!(asset_view::chrome_rows(256),5);
    assert!(frame_row(&f,2).starts_with(b"1:1 48x48 / 1x"));
    assert!(frame_row(&f,3).starts_with(b"F Fit  A 1:1"));
    assert!(frame_row(&f,4).starts_with(b"G fetch  Q clear"));
    let f=render(256,180,Status::Ready,Mode::Fit,48,48);
    assert!(frame_row(&f,2).starts_with(b"Image does not fit"));
    assert!(asset_view::geometry(256,180,48,48,Mode::Fit).is_none());
}
#[test]fn every_status_and_tiny_view_is_whole_bounded_padded_and_within_display_row_limit(){
    use asset_view::{render,Status,Mode};
    for(w,h)in[(0,0),(32,400),(120,110),(255,700),(256,200),(512,219),(1280,712),(u32::MAX,u32::MAX)]{
        for mode in[Mode::Fit,Mode::Actual]{for status in[Status::Loading,Status::Decoding,Status::Sending,Status::Ready,Status::Unavailable,Status::Invalid,Status::Clearing,Status::Cleared,Status::Uncertain,Status::NoFit]{
            let f=render(w,h,status,mode,48,48);assert!(f.len<=512);assert!(f.rows<=24);
            assert!(f.bytes[f.len..].iter().all(|&v|v==0));
            assert!(f.bytes[..f.len].iter().all(|&v|v==b'\n'||(32..=126).contains(&v)));
            if f.rows==0{assert_eq!(f.len,0);continue;}
            assert_eq!(f.len,f.rows*(f.cols+1)-1);
            if f.rows>1{assert!(frame_row(&f,1).starts_with(&status.label()[..status.label().len().min(f.cols)]));}
            assert_eq!(f.bytes[..f.len].iter().filter(|&&v|v==b'\n').count(),f.rows-1);
        }}
    }
    for status in[Status::Loading,Status::Decoding,Status::Invalid,Status::Cleared,Status::Unavailable]{
        for(width,height)in[(0,0),(0,48),(48,0)]{
            let f=render(512,219,status,Mode::Fit,width,height);
            let label=if matches!(status,Status::Loading|Status::Decoding){b"Dimensions pending".as_slice()}else{b"No retained image"};
            assert!(frame_row(&f,2).starts_with(label));
            assert!(!frame_row(&f,2).starts_with(b"Image does not fit"));
        }
    }
    let f=render(512,49,Status::Loading,Mode::Fit,0,0);assert_eq!(f.rows,0);
    let f=render(512,50,Status::Loading,Mode::Fit,0,0);assert_eq!(f.rows,1);
}

#[test]fn no_fit_status_is_explicit_without_renumbering_existing_receipts(){
    assert_eq!(asset_view::Status::NoFit.label(),b"Image does not fit");
    assert_eq!(asset_view::Status::Uncertain as u8,8);
    assert_eq!(asset_view::Status::NoFit as u8,9);
}
