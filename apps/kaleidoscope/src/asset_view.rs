// SPDX-License-Identifier: GPL-3.0-only
//! Finite image status and geometry; dimensions convey no display authority.
use sex_pdx::{bitmap_batch::Geometry, desktop_style::{scale, ADVANCE, INSET, LINE}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status { Loading, Decoding, Sending, Ready, Unavailable, Invalid, Clearing, Cleared, Uncertain }
impl Status {
    pub fn label(self) -> &'static [u8] { match self {
        Self::Loading=>b"Loading image", Self::Decoding=>b"Decoding image",
        Self::Sending=>b"Publishing image", Self::Ready=>b"Image ready",
        Self::Unavailable=>b"Image unavailable", Self::Invalid=>b"Invalid image",
        Self::Clearing=>b"Clearing image", Self::Cleared=>b"Image cleared",
        Self::Uncertain=>b"Waiting for receipt",
    }}
}
pub struct Frame { pub bytes: [u8;512], pub len: usize, pub rows: usize, pub cols: usize }
/// Two fixed text rows leave the remaining body free of stale reader glyphs.
pub fn render(w:u32,h:u32,status:Status)->Frame {
    let factor=scale(w);
    let cols=(w.saturating_sub(2*INSET)/(ADVANCE*factor)).saturating_sub(1).min(79) as usize;
    let top=28+INSET;
    let rows=if cols==0||h<top+7*factor {0} else {1+(h-top-7*factor)/(LINE*factor)} as usize;
    let rows=rows.min(32).min(if cols==0 {0} else {512/(cols+1)});
    let mut f=Frame{bytes:[0;512],len:0,rows,cols};
    if rows==0{return f;}
    f.len=rows*(cols+1)-1;
    for row in 0..rows {let start=row*(cols+1);f.bytes[start..start+cols].fill(b' ');if row+1<rows{f.bytes[start+cols]=b'\n';}}
    // No wrapping into the image area, including narrow tiles.
    let header=b"QOI / controlled HTTPS lab root";
    let first=header.len().min(cols);f.bytes[..first].copy_from_slice(&header[..first]);
    if rows>1 {
        let start=cols+1;let label=status.label();let n=label.len().min(cols);f.bytes[start..start+n].copy_from_slice(&label[..n]);
        let help=b"  G fetch  Q clear";let n=help.len().min(cols.saturating_sub(n));
        let at=start+label.len().min(cols);f.bytes[at..at+n].copy_from_slice(&help[..n]);
    }
    f
}
/// Coordinates are relative to the owned surface, matching the bitmap ABI.
/// The compositor independently clips to its actual live body and framebuffer.
pub fn geometry(w:u32,h:u32,width:usize,height:usize)->Option<Geometry> {
    if !(1..=48).contains(&width)||!(1..=48).contains(&height){return None;}
    let x=INSET;let y=28+INSET+2*LINE*scale(w)+INSET;
    let available_w=w.saturating_sub(x+INSET);let available_h=h.saturating_sub(y+INSET);
    let factor=(available_w/width as u32).min(available_h/height as u32).min(4);
    if factor==0{return None;}
    Some(Geometry{x:x as u16,y:y as u16,width:width as u8,height:height as u8,scale:factor as u8})
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn geometry_fits_every_small_tile_and_never_overflows() {
        for w in [0,8,48,120,255,256,512,640,1280,u32::MAX] {
            for h in [0,20,80,120,219,712,u32::MAX] {for (iw,ih) in [(1,1),(48,48),(3,47)] {
                if let Some(g)=geometry(w,h,iw,ih){assert!(g.valid());assert!(g.x as u32+iw as u32*g.scale as u32+INSET<=w);assert!(g.y as u32+ih as u32*g.scale as u32+INSET<=h);}
            }}
        }
        for (w,h) in [(0,2),(49,2),(1,49)]{assert!(geometry(1000,800,w,h).is_none());}
    }
    #[test] fn padded_text_never_wraps_into_image_rows() {
        for (w,h) in [(0,0),(32,400),(120,110),(512,219),(1280,712)] {for status in [Status::Loading,Status::Decoding,Status::Sending,Status::Ready,Status::Unavailable,Status::Invalid,Status::Clearing,Status::Cleared,Status::Uncertain] {
            let f=render(w,h,status);assert!(f.len<=512);assert!(f.bytes[f.len..].iter().all(|&v|v==0));
            if f.rows>2 {assert!(f.bytes[2*(f.cols+1)..f.len].iter().all(|&v|v==b' '||v==b'\n'));}
        }}
    }
}
