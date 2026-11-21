// SPDX-License-Identifier: GPL-3.0-only
//! Protected final-pass recording marker in the existing Chip0 stop target.
use silkbar_model::{SilkBar,ModuleSlot};
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Rect {pub x:usize,pub y:usize,pub size:usize,pub scale:usize}
pub fn rect(bar:&SilkBar,w:usize,h:usize,scale:u32)->Option<Rect>{
    let scale=usize::try_from(scale).ok()?;
    if !(1..=3).contains(&scale){return None;}
    let b=&bar.layout[ModuleSlot::Chip0 as usize];let size=12*scale;
    if b.w<size||b.h<size||b.x.checked_add(b.w)?>w||b.y.checked_add(b.h)?>h
        ||b.y+b.h>28*scale{return None;}
    Some(Rect{x:b.x+(b.w-size)/2,y:b.y+(b.h-size)/2,size,scale})
}
fn pixel(r:Rect,x:usize,y:usize,foreground:u32,background:u32)->u32{
    let (x,y)=((x/r.scale)as i32,(y/r.scale)as i32);
    let (dx,dy)=(2*x-11,2*y-11);let d=dx*dx+dy*dy;
    if (72..=132).contains(&d)||(4..8).contains(&x)&&(4..8).contains(&y){foreground}else{background}
}
/// # Safety
/// Destination is the exclusive prepared CPU frame, with exactly w*h pixels.
/// Main paints this after overlays and the cursor, before CPU publication.
pub unsafe fn paint(dst:*mut u32,w:usize,h:usize,bar:&SilkBar,scale:u32,fg:u32,bg:u32)->bool{
    let Some(r)=rect(bar,w,h,scale)else{return false;};
    for y in 0..r.size{for x in 0..r.size{core::ptr::write(dst.add((r.y+y)*w+r.x+x),pixel(r,x,y,fg,bg));}}
    true
}
pub fn matches(front:&[u32],w:usize,h:usize,bar:&SilkBar,scale:u32,fg:u32,bg:u32)->bool{
    if w.checked_mul(h)!=Some(front.len())||fg==bg{return false;}
    let Some(r)=rect(bar,w,h,scale)else{return false;};
    (0..r.size).all(|y|(0..r.size).all(|x|front[(r.y+y)*w+r.x+x]==pixel(r,x,y,fg,bg)))
}
