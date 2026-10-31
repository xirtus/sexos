// SPDX-License-Identifier: GPL-3.0-only
//! Finite image status and geometry; dimensions convey no display authority.
use sex_pdx::{bitmap_batch::Geometry, desktop_style::{scale, ADVANCE, INSET, LINE}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status { Loading, Decoding, Sending, Ready, Unavailable, Invalid, Clearing, Cleared, Uncertain, NoFit }
impl Status {
    pub fn label(self) -> &'static [u8] { match self {
        Self::Loading=>b"Loading image", Self::Decoding=>b"Decoding image",
        Self::Sending=>b"Publishing image", Self::Ready=>b"Image ready",
        Self::Unavailable=>b"Image unavailable", Self::Invalid=>b"Invalid image",
        Self::Clearing=>b"Clearing image", Self::Cleared=>b"Image cleared",
        Self::Uncertain=>b"Waiting for receipt", Self::NoFit=>b"Image does not fit",
    }}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Fit, Actual }

pub struct Frame { pub bytes: [u8;512], pub len: usize, pub rows: usize, pub cols: usize }

fn columns(w:u32)->usize {
    (w.saturating_sub(2*INSET)/(ADVANCE*scale(w))).saturating_sub(1).min(79) as usize
}
/// Fixed chrome rows are shared by text composition and image placement.
pub fn chrome_rows(w:u32)->usize { if columns(w)>=30 {4} else {5} }

fn put(frame:&mut Frame,row:usize,text:&[u8]) {
    if row<frame.rows {
        let at=row*(frame.cols+1);let n=text.len().min(frame.cols);
        frame.bytes[at..at+n].copy_from_slice(&text[..n]);
    }
}
fn decimal(out:&mut[u8;32],len:&mut usize,value:usize) {
    // Geometry admits only 1..48 dimensions and 1..4 scale.
    if value>=10 {out[*len]=b'0'+(value/10)as u8;*len+=1;}
    out[*len]=b'0'+(value%10)as u8;*len+=1;
}
fn append(out:&mut[u8;32],len:&mut usize,text:&[u8]) {
    out[*len..*len+text.len()].copy_from_slice(text);*len+=text.len();
}
/// Complete controls occupy dedicated rows. All later rows are blank padding,
/// so a prior reader cannot leave text over the image. Tiny views show only
/// the rows/cells physically available; their image geometry may be unavailable.
pub fn render(w:u32,h:u32,status:Status,mode:Mode,width:usize,height:usize)->Frame {
    let factor=scale(w);let cols=columns(w);let top=28+INSET;
    let rows=if cols==0||h<top+7*factor {0} else {1+(h-top-7*factor)/(LINE*factor)} as usize;
    // Display has 24 indexed rows and the atomic text frame has 512 bytes.
    let rows=rows.min(24).min(if cols==0 {0} else {512/(cols+1)});
    let mut frame=Frame{bytes:[0;512],len:0,rows,cols};
    if rows==0{return frame;}
    frame.len=rows*(cols+1)-1;
    for row in 0..rows {
        let at=row*(cols+1);frame.bytes[at..at+cols].fill(b' ');
        if row+1<rows {frame.bytes[at+cols]=b'\n';}
    }
    put(&mut frame,0,b"QOI / controlled HTTPS lab root");
    put(&mut frame,1,status.label());
    if width==0||height==0 {
        put(&mut frame,2,if matches!(status,Status::Loading|Status::Decoding) {b"Dimensions pending"}else{b"No retained image"});
    } else if let Some(geometry)=geometry(w,h,width,height,mode) {
        let mut metrics=[0;32];let mut len=0;
        append(&mut metrics,&mut len,match mode {Mode::Fit=>b"Fit ",Mode::Actual=>b"1:1 "});
        decimal(&mut metrics,&mut len,width);append(&mut metrics,&mut len,b"x");
        decimal(&mut metrics,&mut len,height);append(&mut metrics,&mut len,b" / ");
        decimal(&mut metrics,&mut len,geometry.scale as usize);append(&mut metrics,&mut len,b"x");
        put(&mut frame,2,&metrics[..len]);
    } else {put(&mut frame,2,b"Image does not fit");}
    if chrome_rows(w)==4 {put(&mut frame,3,b"F Fit  A 1:1  G fetch  Q clear");}
    else {put(&mut frame,3,b"F Fit  A 1:1");put(&mut frame,4,b"G fetch  Q clear");}
    frame
}
/// Center the complete image in the remaining owned body, below all chrome.
/// Actual mode never crops; Fit uses an integer 1..4 scale. The compositor
/// independently checks the exact live owner and clips to actual surface bounds.
pub fn geometry(w:u32,h:u32,width:usize,height:usize,mode:Mode)->Option<Geometry> {
    if !(1..=48).contains(&width)||!(1..=48).contains(&height)||columns(w)==0{return None;}
    let top=28+INSET+(chrome_rows(w)as u32)*LINE*scale(w)+INSET;
    let available_w=w.checked_sub(2*INSET)?;
    let available_h=h.checked_sub(top+INSET)?;
    let factor=match mode {
        Mode::Fit=>(available_w/width as u32).min(available_h/height as u32).min(4),
        Mode::Actual=>1,
    };
    if factor==0{return None;}
    let image_w=width as u32*factor;let image_h=height as u32*factor;
    if image_w>available_w||image_h>available_h{return None;}
    let x=INSET+(available_w-image_w)/2;let y=top+(available_h-image_h)/2;
    Some(Geometry{x:u16::try_from(x).ok()?,y:u16::try_from(y).ok()?,width:width as u8,height:height as u8,scale:factor as u8})
}
