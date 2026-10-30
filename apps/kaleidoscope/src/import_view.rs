// SPDX-License-Identifier: GPL-3.0-only
//! Finite import-name input and viewport-aware status. Names are object labels,
//! not paths, selectors for existing objects or authority over storage.
use sex_pdx::desktop_style::{scale,ADVANCE,INSET,LINE};
pub const DEFAULT_NAME:&[u8]=b"web-note";
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Status{Naming,Saving,Saved,Exists,Unavailable,Cancelled,Uncertain,NoDocument,Cleaning,UnsupportedText}
impl Status{pub fn label(self)->&'static[u8]{match self{Self::Naming=>b"New document name",Self::Saving=>b"Saving",Self::Saved=>b"Saved",Self::Exists=>b"Name exists; not replaced",Self::Unavailable=>b"Import failed",Self::Cancelled=>b"Import cancelled",Self::Uncertain=>b"Save uncertain; inspect Files",Self::NoDocument=>b"Fetch an HTTPS document first",Self::UnsupportedText=>b"Unsupported text; read only",Self::Cleaning=>b"Cancel pending; waiting reply"}}}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Key{Changed,Submit,Cancel,Ignored}
pub struct Editor{pub active:bool,name:[u8;16],len:usize}
impl Editor{
 pub const fn new()->Self{Self{active:false,name:[0;16],len:0}}
 pub fn begin(&mut self){self.name.fill(0);self.name[..DEFAULT_NAME.len()].copy_from_slice(DEFAULT_NAME);self.len=DEFAULT_NAME.len();self.active=true;}
 pub fn name(&self)->[u8;16]{self.name}
 pub fn as_bytes(&self)->&[u8]{&self.name[..self.len]}
 pub fn cancel(&mut self){self.active=false;}
 pub fn key(&mut self,code:u8)->Key{
  if !self.active{return Key::Ignored;}
  match code{0x01=>{self.active=false;Key::Cancel},0x1C if self.len>0=>{self.active=false;Key::Submit},0x0E=>{if self.len>0{self.len-=1;self.name[self.len]=0;}Key::Changed},_=>{
   if let Some(byte)=letter(code){if self.len<16{self.name[self.len]=byte;self.len+=1;return Key::Changed;}}
   Key::Ignored
  }}
 }
}
fn letter(code:u8)->Option<u8>{
 match code{0x02..=0x0A=>Some(b'1'+code-2),0x0B=>Some(b'0'),0x0C=>Some(b'-'),0x34=>Some(b'.'),
  0x10..=0x19=>Some(b"qwertyuiop"[(code-0x10)as usize]),0x1E..=0x26=>Some(b"asdfghjkl"[(code-0x1E)as usize]),0x2C..=0x32=>Some(b"zxcvbnm"[(code-0x2C)as usize]),_=>None}
}
pub struct Frame{pub bytes:[u8;512],pub len:usize,pub cols:usize,pub rows:usize}
/// Full rows replace prior content; short tiles retain the current state/name
/// before supplementary instructions. This function never reads the document.
pub fn render(w:u32,h:u32,status:Status,name:&[u8])->Frame{
 let factor=scale(w);let cols=(w.saturating_sub(2*INSET)/(ADVANCE*factor)).saturating_sub(1).min(79)as usize;
 let top=28+INSET;let rows=if cols==0||h<top+7*factor{0}else{(1+(h-top-7*factor)/(LINE*factor))as usize}.min(32).min(if cols==0{0}else{512/(cols+1)});
 let mut f=Frame{bytes:[0;512],len:0,cols,rows};if rows==0{return f;}
 f.len=rows*(cols+1)-1;for row in 0..rows{let at=row*(cols+1);f.bytes[at..at+cols].fill(b' ');if row+1<rows{f.bytes[at+cols]=b'\n';}}
 let mut source=[0u8;192];let mut n=0;let mut add=|bytes:&[u8]|{let count=bytes.len().min(source.len()-n);source[n..n+count].copy_from_slice(&bytes[..count]);n+=count;};
 add(status.label());add(b"\n");add(&name[..name.len().min(16)]);add(b"\n");
 match status{Status::Naming=>add(b"Enter save  Esc cancel\nLetters digits . -\nNew object only"),Status::Saving|Status::Cleaning=>add(b"Q cancel  Close retires\nWaiting for durable reply"),Status::Saved=>add(b"O Files  V read  I import\nNew object saved"),Status::Exists=>add(b"B fetch  O Files\nI retries retained content"),Status::Uncertain=>add(b"O Files to inspect\nClose retires pending work"),Status::UnsupportedText=>add(b"V read  B fetch  Q clear\nUTF-8 text required to save"),_=>add(b"B fetch  O Files  Q clear")}
 let(mut row,mut col)=(0usize,0usize);for byte in &source[..n]{if row>=rows{break;}if *byte==b'\n'{row+=1;col=0;continue;}if col==cols{row+=1;col=0;if row>=rows{break;}}f.bytes[row*(cols+1)+col]=if(b' '..=b'~').contains(byte){*byte}else{b'?'};col+=1;}
 f
}
#[cfg(test)]mod tests{
 use super::*;
 #[test]fn name_edit_is_bounded_and_enter_requires_nonempty(){let mut e=Editor::new();assert_eq!(e.key(0x1C),Key::Ignored);e.begin();assert_eq!(e.as_bytes(),b"web-note");for _ in 0..30{e.key(0x0E);}assert_eq!(e.key(0x1C),Key::Ignored);for _ in 0..30{e.key(0x1E);}assert_eq!(e.as_bytes(),b"aaaaaaaaaaaaaaaa");assert_eq!(e.key(0x1C),Key::Submit);assert!(!e.active);}
 #[test]fn name_never_accepts_control_path_or_high_bytes(){for code in 0..=255{if let Some(b)=letter(code){assert!(b.is_ascii_lowercase()||b.is_ascii_digit()||b==b'-'||b==b'.');}}let mut e=Editor::new();e.begin();let old=e.name();for code in [0x35,0x2B,0x39,0xFF]{assert_eq!(e.key(code),Key::Ignored);assert_eq!(e.name(),old);}assert_eq!(e.key(0x01),Key::Cancel);}
 #[test]fn every_status_is_finite_printable_padded_and_visible_in_both_tiles(){for(w,h)in[(512,219),(640,237),(1024,680),(1280,712),(120,110),(0,0)]{for status in[Status::Naming,Status::Saving,Status::Saved,Status::Exists,Status::Unavailable,Status::Cancelled,Status::Uncertain,Status::NoDocument,Status::UnsupportedText,Status::Cleaning]{let f=render(w,h,status,b"web-note");assert!(f.len<=512);assert!(f.bytes[..f.len].iter().all(|b|*b==b'\n'||(b' '..=b'~').contains(b)));if f.rows>0{assert_eq!(f.len,f.rows*(f.cols+1)-1);assert_eq!(f.bytes[0],status.label()[0]);}assert!(f.bytes[f.len..].iter().all(|b|*b==0));}}}
 #[test]fn short_name_prompt_keeps_name_before_extra_help(){let f=render(512,120,Status::Naming,b"web-note");let s=core::str::from_utf8(&f.bytes[..f.len]).unwrap();assert!(s.contains("New document name")&&s.contains("web-note"));assert!(!s.contains("Saved"));}
}
