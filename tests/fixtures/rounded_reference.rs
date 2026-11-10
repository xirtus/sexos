// SPDX-License-Identifier: GPL-3.0-only
// Independent global geometry and AA coverage, without production row/mask calls.
fn reference_round_alpha(s:&Surface,scale:u32,x:usize,y:usize)->u8{
 let(lx,ly)=(x as i64-s.x as i64,y as i64-s.y as i64);
 if y<28*scale as usize||lx<0||ly<0||lx>=s.w as i64||ly>=s.h as i64{return 0;}
 let radius=(12*scale).min(s.w/2).min(s.h/2)as f64;
 let cx=(lx as u32).min(s.w-1-lx as u32)as f64;let cy=(ly as u32).min(s.h-1-ly as u32)as f64;
 if radius==0.0||cx>=radius||cy>=radius{return 255;}
 let mut samples=0;
 for dy in [0.125f64,0.375,0.625,0.875]{for dx in [0.125f64,0.375,0.625,0.875]{
  if (cx+dx-radius).powi(2)+(cy+dy-radius).powi(2)<=radius*radius{samples+=1;}
 }}((samples*255+8)/16)as u8
}
fn reference_round_blend(fg:u32,bg:u32,a:u8)->u32{
 let a=a as u32;let mut out=fg&0xff000000;
 for shift in [0,8,16]{let f=(fg>>shift)&255;let b=(bg>>shift)&255;out|=((f*a+b*(255-a))/255)<<shift;}out
}
fn reference_frame_rect(s:&Surface,w:usize,h:usize)->(usize,usize,usize,usize){
 let l=(s.x as i64).max(0).min(w as i64);let t=(s.y as i64).max(28).min(h as i64);
 let r=(s.x as i64+s.w as i64).max(l).min(w as i64);let b=(s.y as i64+s.h as i64).max(t).min(h as i64);
 (l as usize,t as usize,(r-l)as usize,(b-t)as usize)
}
fn reference_composite_pixel_selected(x:usize,y:usize,w:usize,h:usize,bg:u32,focused:u64,_selected:Option<u64>)->u32{unsafe{
 let focused=SURFACES.iter().find(|s|s.active&&s.surface_id==focused)
  .and_then(|s|app_catalog::INCARNATIONS.iter().find(|row|row.target==s.owner_pd&&(row.surface==s.surface_id||row.children.contains(&s.surface_id))))
  .and_then(|i|i.shell_surface).filter(|id|SURFACES.iter().any(|s|s.active&&s.owner_pd==3&&s.surface_id==*id&&FRAME_GROUPS.is_frame(s))).unwrap_or(focused);
 // Independent comparison sort over literal builtin precedence and immutable
 // catalog data; do not use the production lookup table/key/row iterator.
 let priority=|s:&Surface|{
  let rank=if s.surface_id==200{0}else if let Some(index)=app_catalog::INCARNATIONS.iter().position(|row|row.frame()==s.surface_id||row.surface==s.surface_id||row.children.contains(&s.surface_id)){1+index}
   else if let Some(index)=[202,203,204,103,102,101,100].iter().position(|id|*id==s.surface_id){1+app_catalog::COUNT+index}else{usize::MAX};
  (if s.owner_pd==3&&s.surface_id==152{0}else if s.surface_id==focused{1}else{2},rank,s.surface_id)
 };
 let mut owners:Vec<_>=SURFACES.iter().filter(|s|s.active&&FRAME_GROUPS.independent(s)).collect();
 owners.sort_by_key(|s|priority(s));let mut layers=Vec::new();
 for s in owners{
  let(l,t,rw,rh)=reference_frame_rect(s,w,h);
  if x<l||x-l>=rw||y<t||y-t>=rh{continue;}
  let a=reference_round_alpha(s,1,x,y);if a==0{continue;}
  layers.push((s.surface_id,a));if a==255{break;}
 }
 let mut result=bg;
 // Shade front-to-back for honest contributor accounting, fold colors back-to-front.
 let painted:Vec<_>=layers.iter().map(|&(id,a)|(reference_composite_pixel_unmasked(x,y,w,h,bg,focused,Some(id)),a)).collect();
 for &(color,a)in painted.iter().rev(){result=reference_round_blend(color,result,a);}
 result
}}
