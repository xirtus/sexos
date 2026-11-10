//! Actual bounded Browser builders in separate host processes per applied scale.
//! The UI scale is controlled data; this proves no IPC/source authority or pixels.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"]mod app_catalog;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
#[path="../crates/sex-pdx/src/bitmap_batch.rs"]pub mod bitmap_batch;
#[path="../crates/sex-pdx/src/text_encoding.rs"]pub mod text_encoding;
#[path="../crates/sex-pdx/src/text_batch.rs"]pub mod text_batch;
#[path="../crates/sex-pdx/src/clock_source.rs"]pub mod clock_source;
mod network_probe{pub enum View{Pending,Verified,Unavailable,Cancelled,Uncertain}}
mod dns_probe{pub enum View{Idle,Pending,Answer([u8;4]),Unavailable,Cancelled,Uncertain}}
mod http_probe{pub enum View{Idle,Pending,Ready,Unavailable,Cancelled,Uncertain}}
mod clock_probe{#[derive(Clone,Copy)]pub enum View{Idle,Pending,Ready(u32),Unavailable,Cancelled,Uncertain,Rollback}}
#[path="../apps/kaleidoscope/src/browser_view.rs"]mod browser_view;
#[path="../apps/kaleidoscope/src/clock_view.rs"]mod clock_view;
#[path="../apps/kaleidoscope/src/import_view.rs"]mod import_view;
#[path="../apps/kaleidoscope/src/asset_view.rs"]mod asset_view;
fn main(){
 let ui=std::env::args().nth(1).unwrap().parse::<u32>().unwrap();assert!((1..=3).contains(&ui));
 desktop_style::set_ui_scale(ui);
 let(w,h,cols,tls_rows,calendar_rows,image)=match ui{
  1=>(1280,800,63,8,8,(544,357)),
  2=>(1280,800,51,9,9,(544,410)),
  3=>(1024,768,26,12,12,(416,474)),_=>unreachable!()};
 let tls=browser_view::tls_panel(w,h,Some(b"hello pending; no peer")).unwrap();
 assert_eq!((tls.cols,tls.rows,tls.complete),(cols,tls_rows,true));assert!(tls.len<=512);
 assert!(core::str::from_utf8(&tls.bytes[..tls.len]).unwrap().contains("hello pending; no peer"));
 let clock=clock_view::render(w,h,clock_probe::View::Ready(1794320670));
 assert_eq!((clock.cols,clock.rows),(cols,calendar_rows));
 assert!(core::str::from_utf8(&clock.bytes[..clock.len]).unwrap().contains("2026-11-10"));
 let import=import_view::render(w,h,import_view::Status::Naming,b"note.txt");
 assert!(import.len<=512&&import.rows>0);
 assert!(core::str::from_utf8(&import.bytes[..import.len]).unwrap().contains("note.txt"));
 let fit=asset_view::geometry(w,h,48,48,asset_view::Mode::Fit).unwrap();
 assert_eq!((fit.x,fit.y,fit.scale),(image.0,image.1,4));
 let actual=asset_view::geometry(w,h,48,48,asset_view::Mode::Actual).unwrap();assert_eq!(actual.scale,1);
 println!("[browser.panel.scale.host] PASS scale={ui} scope=actual_layout_only");
}
