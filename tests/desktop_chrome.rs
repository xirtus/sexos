//! Shared production geometry; literal device-pixel goldens are independent.
#![allow(dead_code)]
#[path="../crates/sex-pdx/src/app_catalog.rs"] mod app_catalog;
#[path="../crates/sex-pdx/src/desktop_style.rs"] mod desktop_style;
use desktop_style::chrome::{Metrics,Rect,step_scale};

#[test]
fn rounded_panel_literal_corner_coverage_and_row_spans(){
    let s=Metrics::new(1).rounded(200,100);
    assert_eq!(s.radius(),12);
    assert_eq!((0..12).map(|x|s.coverage(x,0)).collect::<Vec<_>>(),
        vec![0,0,0,0,0,0,0,32,112,191,239,255]);
    assert_eq!(s.row(0,false),(7,193));
    assert_eq!(s.row(0,true),(11,189));
    for (ui,cut,full)in[(1,7,11),(2,17,22),(3,28,33)]{
        let s=Metrics::new(ui).rounded(200,100);
        assert_eq!(s.row(0,false),(cut,200-cut));
        assert_eq!(s.row(0,true),(full,200-full));
        assert_eq!(s.row(50,false),(0,200));
    }
}

#[test]
fn rounded_spans_exactly_match_visible_and_opaque_pixels_including_tiny_frames(){
    for ui in 1..=3{for w in[0,1,2,7,23,24,25,80]{for h in[0,1,2,7,23,24,25,80]{
        let s=Metrics::new(ui).rounded(w,h);
        assert!(s.radius()<=12*ui&&s.radius()<=w/2&&s.radius()<=h/2);
        for y in -1..=h as i64{
            let any=s.row(y,false);let full=s.row(y,true);
            assert!(any.0<=any.1&&any.1<=w&&full.0<=full.1&&full.1<=w);
            for x in -1..=w as i64{
                let a=s.coverage(x,y);
                assert_eq!(a>0,x>=any.0 as i64&&x<(any.1 as i64));
                assert_eq!(a==255,x>=full.0 as i64&&x<(full.1 as i64));
                if x>=0&&y>=0&&x<w as i64&&y<h as i64{
                    assert_eq!(a,s.coverage(w as i64-1-x,h as i64-1-y));
                }
            }
        }
    }}}
}

#[test]
fn rounded_original_signed_origin_and_captured_scale_do_not_follow_a_clip_edge(){
    let saved=Metrics::new(1).rounded(200,100);
    let later=Metrics::new(3).rounded(200,100);
    assert_eq!((saved.radius(),later.radius()),(12,36));
    // Original x=-20: visible screen x=0 is inside the straight title span,
    // not the start of a new rounded corner at the clipping boundary.
    assert_eq!(saved.coverage(0i64-(-20),0),255);
    assert_eq!(saved.coverage(0,0),0);
    for (x,y)in[(i64::MIN,0),(i64::MAX,0),(0,i64::MIN),(0,i64::MAX)]{
        assert_eq!(saved.coverage(x,y),0);
    }
}

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

#[test]
fn applied_scale_requires_current_authenticated_post_publication_receipt() {
    use desktop_style::chrome::{ScaleClient,ScalePublisher,scale_receipt};
    let mut shell=ScaleClient::new();let mut display=ScalePublisher::new();
    assert_eq!(display.register(2,2,0),Some(true));
    assert!(shell.begin(1));let(seq,step)=shell.request().unwrap();assert_eq!(step,1);
    assert!(!shell.begin(-1));assert!(shell.pending());
    // Backpressure keeps the exact request, not another additive step.
    assert_eq!(shell.request(),Some((seq,1)));
    let value=scale_receipt(seq,2).unwrap();
    assert_eq!(shell.receive(1,1,value),None); // not even admitted
    shell.admitted();assert_eq!(shell.request(),None);
    assert!(display.admit(2,2,seq,2));assert!(display.needs_publication());
    assert_eq!(display.reply(),None);assert!(!display.admit(2,2,seq+1,3));
    // Unrelated/failed/partial publication leaves the receipt unavailable.
    assert!(display.needs_publication());assert_eq!(display.reply(),None);
    display.published();assert!(!display.needs_publication());
    assert_eq!(display.reply(),Some((2,value)));
    assert_eq!(display.reply(),Some((2,value))); // transport failure retains it
    for (pd,v) in [(99,value),(1,value^4),(1,value&!3),(1,value^0x1000_0000_0000_0000)] {
        assert_eq!(shell.receive(pd,1,v),None);assert!(shell.pending());
    }
    assert_eq!(shell.receive(1,1,value),Some(2));display.sent();
    assert!(!display.pending());assert!(!shell.pending());
    assert!(shell.begin(-1));shell.admitted();
    assert_eq!(shell.receive(1,1,value),None); // previous transition cannot replay
    shell.cancel();assert!(!shell.pending());
    assert!(shell.begin(0));assert_ne!(shell.request().unwrap().0,seq);
}
#[test]
fn malformed_scale_requests_do_not_install_publication_state() {
    use desktop_style::chrome::{ScalePublisher,scale_receipt};
    let mut display=ScalePublisher::new();
    assert_eq!(display.register(2,2,0),Some(true));
    for (seq,scale) in [(0,1),(1,0),(1,4),(1u64<<46,1),(u64::MAX,3)] {
        assert_eq!(scale_receipt(seq,scale),None);
        assert!(!display.admit(2,2,seq,scale));assert!(!display.pending());
    }
}

#[test]
fn pending_receipt_blocks_legacy_and_incarnation_reset_fences_delayed_old_reply(){
    use desktop_style::chrome::{ScaleClient,ScalePublisher,scale_receipt};
    let mut display=ScalePublisher::new();let mut old=ScaleClient::for_incarnation(0);
    display.register(3,3,0);assert!(old.begin(1));let(seq,_)=old.request().unwrap();old.admitted();
    assert!(display.admit(3,3,seq,2));
    assert!(!display.allows_change(3,3,0)); // legacy must not alter the retained scale
    assert!(!display.allows_change(3,3,seq+1));
    assert_eq!(display.register(3,3,0),Some(false));assert!(display.pending());
    assert_eq!(display.register(3,33,1),None);assert!(display.pending());
    let old_reply=scale_receipt(seq,2).unwrap();
    // Kernel restart keeps PD/task 3; the own-service restart count advances.
    let mut fresh=ScaleClient::for_incarnation(1);assert!(fresh.begin(0));fresh.admitted();
    assert_eq!(fresh.receive(1,1,old_reply),None);
    assert_eq!(display.register(3,3,1),Some(true));assert!(!display.pending());
    assert!(!display.allows_change(3,3,seq));
    assert_eq!(display.register(3,3,0),None); // old registration cannot rewind
    let(seq,_)={let mut c=ScaleClient::for_incarnation(1);c.begin(0);c.request().unwrap()};
    assert!(display.admit(3,3,seq,1));display.published();
    assert_eq!(fresh.receive(1,1,display.reply().unwrap().1),Some(1));
    let mut exhausted=ScaleClient::for_incarnation(1<<14);assert!(!exhausted.begin(1));
}

#[test]fn narrow_framebuffers_refuse_an_unfittable_scale_without_overflow(){
 use desktop_style::chrome::fit_scale;
 for (w,requested,want) in [(640,3,2),(800,3,2),(960,3,3),(1024,3,3),(1280,2,2),(1,3,1),(8192,99,3)]{
  assert_eq!(fit_scale(w,requested,320),want);
 }
 assert_eq!(fit_scale(8192,3,u32::MAX),1);
}

#[test]fn scaled_overlay_fits_workarea_with_independent_centered_goldens(){
 for(ui,w,h,expected)in[(1,1280,800,Rect{x:400,y:274,w:480,h:280}),
 (2,1280,800,Rect{x:160,y:148,w:960,h:560}),
 (3,1024,768,Rect{x:24,y:108,w:976,h:636})]{
  let m=Metrics::new(ui);let r=m.centered_panel(w,h,480,280);assert_eq!(r,expected);
  assert!(r.y>=m.bar());assert!(r.x+r.w<=w&&r.y+r.h<=h);
 }
 for ui in[0,1,2,3,u32::MAX]{for(w,h)in[(0,0),(1,1),(15,27),(320,240),(u32::MAX,u32::MAX)]{
  let r=Metrics::new(ui).centered_panel(w,h,u32::MAX,u32::MAX);
  assert!(r.x<=w&&r.y<=h&&r.w<=w-r.x&&r.h<=h-r.y);
 }}
}

#[test]fn initial_panel_hit_matches_applied_workarea_and_original_signed_mask(){
 for scale in 1..=3{let m=Metrics::new(scale);
  let frame=(-20,-20,300,300);
  for point in [(-1,100),(320,100),(100,-1),(100,200),(100,(28*scale-1)as i32)]{
   assert!(!m.panel_hit(frame,point,(320,200)),"protected/outside scale{scale} {point:?}");
  }
  assert!(m.panel_hit(frame,(0,(28*scale)as i32),(320,200)));
  assert!(!m.panel_hit(frame,(0,0),(0,200)));
  assert!(!m.panel_hit(frame,(0,0),(320,0)));
  assert!(!m.panel_hit(frame,(0,27),(320,27)));
  assert!(!m.panel_hit((20,100,100,80),(20,100),(320,200))); // transparent corner
  assert!(m.panel_hit((20,100,100,80),(60,100),(320,200)));
 }
}
