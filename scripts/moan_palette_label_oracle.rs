// Independent expected label patches: explicit approved colors/background/text.
// No guest-reported hash/color or Moan rendering is used.
use weft::{Ui,Theme,Strings,Layout,TextStyle,Color};
fn main(){
 const ACCENTS:[u32;8]=[0x7da6ff,0x55d1e3,0x73d69a,0xd5d76b,0xf1b46b,0xf18a83,0xe58ac8,0xb49aee];
 for(i,color)in ACCENTS.iter().enumerate(){
  let mut theme=Theme::dark().with_density(weft::Density::Compact);
  assert_eq!(theme.palette.control,Color::hex(0x35313b));
  assert_eq!(theme.font_size,14.0);
  theme.palette.window=Color::hex(0x35313b);
  let mut ui=Ui::new(118,19,theme,Strings::new("en"));let root=ui.root();
  ui.set_layout(root,Layout::column().gap(0.0).padding(0.0).fill());
  let text=format!("{} Audio",i+1);let id=ui.text(root,&text,TextStyle::Caption);
  ui.set_color(id,Some(Color::hex(*color)));ui.layout();
  let rect=ui.rect(id);assert_eq!((rect.x,rect.y,rect.w,rect.h),(0.0,0.0,118.0,19.0));
  let mut pixels=vec![0;118*19];ui.paint(&mut pixels).unwrap();
  let mut hash=0xcbf29ce484222325u64;
  for pixel in pixels{for byte in[(pixel>>16)as u8,(pixel>>8)as u8,pixel as u8]{hash=(hash^byte as u64).wrapping_mul(0x100000001b3);}}
  println!("{{\"track\":{},\"x\":{},\"y\":378,\"w\":118,\"h\":19,\"foreground\":\"0x{:06x}\",\"background\":\"0x35313b\",\"font_size\":14,\"style\":\"Caption\",\"hash\":\"0x{:x}\",\"text\":\"{}\"}}",i,8+130*i,color,hash,text);
 }
}
