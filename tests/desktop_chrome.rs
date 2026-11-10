//! Shared production geometry; literal device-pixel goldens are independent.
#![allow(dead_code)]
#[path="../crates/sex-pdx/src/app_catalog.rs"] mod app_catalog;
#[path="../crates/sex-pdx/src/desktop_style.rs"] mod desktop_style;
use desktop_style::chrome::{Metrics,Rect,step_scale};

#[test]
fn approved_geometry_has_one_separator_and_contiguous_workarea() {
    for (scale,bar,title,target,body) in [
        (1,28,26,24,Rect{x:8,y:34,w:624,h:438}),
        (2,56,52,48,Rect{x:16,y:68,w:608,h:396}),
        (3,84,78,72,Rect{x:24,y:102,w:592,h:354}),
    ] {
        let m=Metrics::new(scale);
        assert_eq!((m.bar(),m.title()),(bar,title));
        assert_eq!(m.workarea(1280,800),Rect{x:0,y:bar,w:1280,h:800-bar});
        assert_eq!(m.body(640,480),body);
        for i in 0..3 {
            let r=m.control(i).unwrap();assert_eq!((r.w,r.h),(target,target));
            assert!(r.y+r.h<=title);
            if i>0 {assert_eq!(m.control(i-1).unwrap().x+target,r.x);}
        }
        assert_eq!(m.control(3),None);
    }
}
#[test]
fn exclusive_bounds_cannot_hit_next_control_or_wrap_at_u32_limit() {
    let r=Metrics::new(3).control(0).unwrap();
    assert!(r.contains(3,3));assert!(r.contains(74,74));
    for (x,y) in [(2,3),(3,2),(75,3),(3,75),(u32::MAX,u32::MAX)] {assert!(!r.contains(x,y));}
    assert!(!Rect{x:u32::MAX-1,y:0,w:4,h:1}.contains(0,0));
}
#[test]
fn tiny_frame_and_workarea_never_underflow_or_escape() {
    for scale in 1..=3 {for w in 0..100 {for h in 0..110 {
        let m=Metrics::new(scale);let b=m.body(w,h);let a=m.workarea(w,h);
        assert!(b.x<=w&&b.y<=h&&b.w<=w-b.x&&b.h<=h-b.y);
        assert!(a.y<=h&&a.h==h-a.y);
    }}}
    assert_eq!(Metrics::new(99).px(u32::MAX),u32::MAX);
}
#[test]
fn steps_are_bounded_even_for_adversarial_payloads_and_reset_is_boot_owned() {
    assert_eq!([step_scale(1,2,-1),step_scale(1,2,1),step_scale(3,1,1),step_scale(3,2,0)],[1,2,3,2]);
    assert_eq!([step_scale(0,0,i64::MIN),step_scale(u32::MAX,99,i64::MAX),step_scale(1,0,0)],[1,3,1]);
    for before in 1..=3 {for boot in 1..=3 {
        assert_eq!(step_scale(before,boot,0),boot);
        assert_eq!(step_scale(before,boot,i64::MIN),step_scale(before,boot,-1));
        assert_eq!(step_scale(before,boot,i64::MAX),step_scale(before,boot,1));
    }}
}
