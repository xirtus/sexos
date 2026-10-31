// SPDX-License-Identifier: GPL-3.0-only
//! Actual bounded snapshot formatter and shared calendar conversion. No source
//! authenticity, syscall authority or native framebuffer claim is made here.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/clock_source.rs"]pub mod clock_source;
#[path="../crates/sex-pdx/src/desktop_style.rs"]pub mod desktop_style;
mod clock_probe {
    #[derive(Clone,Copy)]pub enum View {Idle,Pending,Ready(u32),Unavailable,Cancelled,Uncertain,Rollback}
}
#[path="../apps/kaleidoscope/src/clock_view.rs"]mod clock_view;
use clock_probe::View;
fn rows(f:&clock_view::Frame)->Vec<&str>{core::str::from_utf8(&f.bytes[..f.len]).unwrap().split('\n').map(str::trim_end).collect()}
fn sample(year:i64,month:u8,day:u8,hour:u8,minute:u8,second:u8)->u32{
    u32::try_from(sex_time::Gregorian{year,month,day,hour,minute,second}.to_timestamp().unwrap().to_unix().unwrap()).unwrap()
}
#[test]fn three_civil_views_match_known_epoch_relative_days_and_boundary_seconds(){
    for (value,date,native,count) in [
        (clock_source::MIN_SECONDS as u32,"2020-01-01T00:00:00Z",221_788_800u64,"13.0.7.2.7"),
        (sample(2020,2,29,0,0,0),"2020-02-29T00:00:00Z",226_886_400,"13.0.7.5.6"),
        (sample(2026,10,30,13,45,7),"2026-10-30T13:45:07Z",437_319_907,"13.0.14.1.1"),
        (clock_source::MAX_SECONDS as u32,"2099-12-31T23:59:59Z",2_746_396_799,"13.4.8.5.6"),
    ] {
        let frame=clock_view::render(640,237,View::Ready(value));let lines=rows(&frame);
        assert_eq!(lines[0],"Clock / last query snapshot");assert_eq!(lines[1],date);
        assert_eq!(lines[2],format!("SexTime {native} s"));assert_eq!(lines[3],format!("Long Count {count}"));
        assert_eq!(&lines[4..8],&["UTC day / GMT 584283","q35 CMOS / 1-second precision","Snapshot only / not live","T sample  Q clear"]);
    }
}
#[test]fn last_source_second_rolls_gregorian_and_long_count_only_at_utc_midnight(){
    let before=sample(2024,2,28,23,59,59);let day=clock_view::render(512,219,View::Ready(before));let next=clock_view::render(512,219,View::Ready(before+1));let later=clock_view::render(512,219,View::Ready(before+2));
    let(a,b,c)=(rows(&day),rows(&next),rows(&later));assert_eq!(a[1],"2024-02-28T23:59:59Z");assert_eq!(b[1],"2024-02-29T00:00:00Z");assert_eq!(c[1],"2024-02-29T00:00:01Z");assert_ne!(a[3],b[3]);assert_eq!(b[3],c[3]);
}
#[test]fn both_tiles_and_fullscreen_keep_all_eight_rows_without_hidden_enlarge_loop(){
    for(w,h)in[(504,219),(512,219),(640,237),(624,706),(1008,674),(1024,680),(1264,706),(1280,712),(u32::MAX,u32::MAX)]{
        let f=clock_view::render(w,h,View::Ready(clock_source::MAX_SECONDS as u32));let lines=rows(&f);
        assert!(f.cols<=63);assert!(f.rows>=8,"{w}x{h}");assert_eq!(lines[1],"2099-12-31T23:59:59Z");assert_eq!(lines[7],"T sample  Q clear");assert!(!lines[0].contains("enlarge"));
    }
}
#[test]fn compact_measure_preserves_whole_numbers_and_explicit_source(){
    // scale2 widths268..400 expose20..31 columns.
    for w in [268,280,320,388,400]{let f=clock_view::render(w,360,View::Ready(clock_source::MAX_SECONDS as u32));let lines=rows(&f);assert!(20<=f.cols&&f.cols<32);assert_eq!(lines[0],"Clock / last query");assert_eq!(lines[1],"2099-12-31T23:59:59Z");assert_eq!(lines[2],"SexTime 2746396799 s");assert_eq!(lines[3],"LC 13.4.8.5.6");assert_eq!(&lines[4..8],&["UTC day GMT584283","q35 CMOS / 1s","Snapshot / not live","T sample  Q clear"]);}
}
#[test]fn every_nonready_and_invalid_snapshot_replaces_all_old_calendar_numbers(){
    let ready=clock_view::render(640,237,View::Ready(clock_source::MAX_SECONDS as u32));assert!(rows(&ready)[1].starts_with("2099"));
    for(view,state)in[(View::Idle,"No sample"),(View::Pending,"Pending"),(View::Unavailable,"Unavailable"),(View::Cancelled,"Cancelled"),(View::Uncertain,"Waiting receipt"),(View::Rollback,"Rollback / blocked"),(View::Ready(0),"Invalid snapshot"),(View::Ready(clock_source::MIN_SECONDS as u32-1),"Invalid snapshot"),(View::Ready(clock_source::MAX_SECONDS as u32+1),"Invalid snapshot"),(View::Ready(u32::MAX),"Invalid snapshot")]{
        let f=clock_view::render(640,237,view);let lines=rows(&f);assert_eq!(&lines[1..4],&[state,"No date snapshot","SexTime / Long Count unavailable"]);let text=core::str::from_utf8(&f.bytes[..f.len]).unwrap();assert!(!text.contains("2099-")&&!text.contains("2746396799")&&!text.contains("13.4.8.5.6"));
    }
}
#[test]fn tiny_tile_fallback_never_exposes_truncated_numeric_date(){
    for(w,h)in[(0,0),(16,200),(40,500),(120,110),(256,360),(512,120),(640,160),(1280,49)]{
        let f=clock_view::render(w,h,View::Ready(clock_source::MAX_SECONDS as u32));let text=core::str::from_utf8(&f.bytes[..f.len]).unwrap();assert!(!text.contains("2099")&&!text.contains("2746396799")&&!text.contains("13.4"));if f.rows>0{assert!(rows(&f)[0].starts_with("Ready / enlarge tile"[..f.cols.min(20)].trim_end()));}
    }
}
#[test]fn geometry_is_a_whole_bounded_padded_replacement_at_all_thresholds(){
    for w in [0,1,16,22,255,256,267,268,400,401,504,512,640,1024,1280,u32::MAX]{for h in [0,1,35,42,43,49,50,120,175,176,219,237,800,u32::MAX]{
        let f=clock_view::render(w,h,View::Pending);assert!(f.cols<=63&&f.rows<=24&&f.len<=512);assert!(f.bytes[f.len..].iter().all(|v|*v==0));
        if f.rows==0{assert_eq!(f.len,0);continue;}
        assert_eq!(f.len,f.rows*(f.cols+1)-1);let lines=f.bytes[..f.len].split(|v|*v==10).collect::<Vec<_>>();assert_eq!(lines.len(),f.rows);assert!(lines.iter().all(|l|l.len()==f.cols&&l.iter().all(|v|(32..=126).contains(v))));
        let factor=desktop_style::scale(w);assert!(36+desktop_style::GLYPH_HEIGHT*factor+(f.rows as u32-1)*desktop_style::LINE*factor<=h);
    }}
}
#[test]fn legacy_status_remains_exact_and_does_not_render_epoch_outside_source_policy(){
    assert!(clock_view::status(View::Idle).is_none());
    for(view,text)in[(View::Pending,"checking"),(View::Unavailable,"unavailable"),(View::Cancelled,"cancelled"),(View::Uncertain,"waiting receipt"),(View::Rollback,"rollback; blocked"),(View::Ready(1_356_048_000),"unavailable"),(View::Ready(0),"unavailable")]{let s=clock_view::status(view).unwrap();assert_eq!(&s.bytes[..s.len],text.as_bytes());}
    for value in [clock_source::MIN_SECONDS as u32,clock_source::MAX_SECONDS as u32]{let s=clock_view::status(View::Ready(value)).unwrap();assert_eq!(&s.bytes[..s.len],rows(&clock_view::render(640,237,View::Ready(value)))[1].as_bytes());}
}
