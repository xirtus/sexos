//! Actual register ABI and display-private transaction/paint helpers. No mock
//! bitmap implementation or success inferred from an enqueue result.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path = "../crates/sex-pdx/src/bitmap_batch.rs"] pub mod bitmap_batch;
#[path = "../servers/sexdisplay/src/bitmap_transaction.rs"] mod transaction;
#[path = "../servers/sexdisplay/src/damage.rs"] mod damage;
use bitmap_batch::{hash, Applied, Geometry, Request, CAPACITY};
use transaction::{blend, Stage, Viewport};
fn geometry(width: u8, height: u8, scale: u8) -> Geometry { Geometry { x: 10, y: 40, width, height, scale } }
fn receive(stage: &mut Stage, request: Request) -> Option<transaction::Change> {
    let (op, a, b) = request.wire();
    stage.receive(15, 300, Request::decode(op, a, b).unwrap())
}
fn begin(stage: &mut Stage, sequence: u32, geometry: Geometry) {
    assert_eq!(receive(stage, Request::Begin { sequence, geometry }), None);
}
fn chunks(stage: &mut Stage, sequence: u32, pixels: &[u32]) {
    for (i, input) in pixels.chunks(2).enumerate() {
        let mut pair = [0; 2]; pair[..input.len()].copy_from_slice(input);
        assert_eq!(receive(stage, Request::Chunk { sequence, offset: i*2, count: input.len(), pixels: pair }), None);
    }
}
fn publish(stage: &mut Stage, sequence: u32, g: Geometry, pixels: &[u32]) {
    begin(stage, sequence, g); chunks(stage, sequence, pixels);
    assert_eq!(receive(stage, Request::Commit { sequence, hash: hash(pixels) }).unwrap().new, Some(g));
}
fn ack(stage: &mut Stage, sequence: u32) {
    let mut calls = 0; stage.poll_reply(|owner, value| {
        calls += 1; assert_eq!(owner, 15);
        assert_eq!(Applied::decode(1, value), Some(Applied { surface: 300, sequence })); 0
    }); assert_eq!(calls, 1);
}
#[test]
fn canonical_abi_roundtrips_dimensions_scales_partial_pair_and_receipt() {
    for g in [geometry(1,1,1), geometry(47,47,3), Geometry{x:u16::MAX,y:u16::MAX,..geometry(48,48,4)}] {
        let requests = [Request::Begin{sequence:u32::MAX,geometry:g},
            Request::Chunk{sequence:7,offset:2302,count:2,pixels:[0x00112233,0xff998877]},
            Request::Chunk{sequence:7,offset:0,count:1,pixels:[0x80808080,0]},
            Request::Commit{sequence:7,hash:u64::MAX},Request::Clear{sequence:8}];
        for request in requests { let(op,a,b)=request.wire();assert_eq!(Request::decode(op,a,b),Some(request)); }
    }
    let receipt = Applied{surface:300,sequence:u32::MAX};let word=receipt.encode().unwrap();
    assert_eq!(Applied::decode(1,word),Some(receipt));
    for caller in [0,3,15,u32::MAX] { assert_eq!(Applied::decode(caller,word),None); }
    for word in [word|1<<56,word^1,word&!(0xffff<<8),0x56] {assert_eq!(Applied::decode(1,word),None);}
    assert_eq!(Applied{surface:0,sequence:1}.encode(),None);assert_eq!(Applied{surface:300,sequence:0}.encode(),None);
    let bytes=[0x44u8,0x33,0x22,0x11,0xdd,0xcc,0xbb,0xaa];
    let expected=bytes.iter().fold(0xcbf29ce484222325u64,|h,&v|(h^v as u64).wrapping_mul(0x100000001b3));
    assert_eq!(hash(&[0x11223344,0xaabbccdd]),expected);
}
#[test]
fn malformed_reserved_bits_width_aliases_and_noncanonical_chunks_are_rejected() {
    let (op,a,b)=Request::Begin{sequence:1,geometry:geometry(48,48,4)}.wire();
    for invalid in [(op,a|1<<56,b),(op,a,b|1<<32),(op,a&!0xffff_ffff,b),
        (op,a&!(0xff<<32),b),(op,(a&!(0xff<<32))|49<<32,b),
        (op,a&!(0xff<<40),b),(op,(a&!(0xff<<48))|5<<48,b),(op,a&!(0xff<<48),b)] {
        assert_eq!(Request::decode(invalid.0,invalid.1,invalid.2),None);
    }
    for (op,a,b) in [(bitmap_batch::CHUNK,0,1<<32|1<<24|2<<16),
        (bitmap_batch::CHUNK,0,1<<32|1|2<<16),(bitmap_batch::CHUNK,0,1<<32|2304|2<<16),
        (bitmap_batch::CHUNK,0,1<<32),(bitmap_batch::CHUNK,0,1<<32|3<<16),
        (bitmap_batch::CHUNK,1<<32,1<<32|1<<16),(bitmap_batch::CHUNK,0,2<<16),
        (bitmap_batch::COMMIT,1<<32,0),(bitmap_batch::COMMIT,0,0),
        (bitmap_batch::CLEAR,1,1),(bitmap_batch::CLEAR,0,0)] {
        assert_eq!(Request::decode(op,a,b),None,"{op:x} {a:x} {b:x}");
    }
    for op in [bitmap_batch::BEGIN,bitmap_batch::CHUNK,bitmap_batch::COMMIT,bitmap_batch::CLEAR] {
        for alias in [op|1<<32,op|1<<63] { assert_eq!(Request::decode(alias,1,0),None); }
    }
}
#[test]
fn every_partial_prefix_is_private_and_full_48_square_publishes_once() {
    println!("[display.bitmap.host.storage] stage_bytes={} pixel_payload_bytes={} capacity={}",core::mem::size_of::<Stage>(),2*CAPACITY*4,CAPACITY);
    for g in [geometry(1,1,1),geometry(3,1,2),geometry(47,47,3),geometry(48,48,4)] {
        let pixels:Vec<_>=(0..g.len()).map(|i|0x80000000|i as u32).collect();let mut stage=Stage::EMPTY;
        begin(&mut stage,1,g);
        for (i,input) in pixels.chunks(2).enumerate() {
            let mut pair=[0;2];pair[..input.len()].copy_from_slice(input);
            receive(&mut stage,Request::Chunk{sequence:1,offset:i*2,count:input.len(),pixels:pair});
            assert_eq!(stage.geometry(),None);assert_eq!(stage.sample(15,300,g.x as i64,g.y as i64),None);
        }
        let change=receive(&mut stage,Request::Commit{sequence:1,hash:hash(&pixels)}).unwrap();
        assert_eq!((change.old,change.new),(None,Some(g)));
        for y in 0..g.height as i64 {for x in 0..g.width as i64 {
            assert_eq!(stage.sample(15,300,g.x as i64+x*g.scale as i64,g.y as i64+y*g.scale as i64),Some(pixels[y as usize*g.width as usize+x as usize]));
        }}
        assert_eq!(receive(&mut stage,Request::Commit{sequence:1,hash:hash(&pixels)}),None);ack(&mut stage,1);
        stage.poll_reply(|_,_|panic!("receipt admitted already"));
        assert!(core::mem::size_of::<Stage>()<19000);assert_eq!(CAPACITY*4,9216);
    }
}
#[test]
fn incomplete_bad_hash_and_conflicting_duplicates_keep_old_publication() {
    for mode in 0..7 { let mut stage=Stage::EMPTY;let old=geometry(1,1,1);publish(&mut stage,1,old,&[0xff123456]);ack(&mut stage,1);
        let new=geometry(3,1,1);begin(&mut stage,2,new);
        receive(&mut stage,Request::Chunk{sequence:2,offset:0,count:2,pixels:[1,2]});
        match mode {
            0=>{},
            1=>{chunks(&mut stage,2,&[1,2,3]);},
            2=>{receive(&mut stage,Request::Chunk{sequence:2,offset:0,count:2,pixels:[4,2]});},
            3=>{begin(&mut stage,2,geometry(2,1,1));},
            4=>{stage.receive(15,300,Request::Chunk{sequence:2,offset:0,count:usize::MAX,pixels:[0;2]});},
            5=>{stage.receive(15,300,Request::Chunk{sequence:2,offset:usize::MAX,count:1,pixels:[0;2]});},
            _=>{receive(&mut stage,Request::Chunk{sequence:2,offset:2,count:2,pixels:[3,0]});},
        }
        assert_eq!(receive(&mut stage,Request::Commit{sequence:2,hash:0}),None);
        chunks(&mut stage,2,&[1,2,3]);assert_eq!(receive(&mut stage,Request::Commit{sequence:2,hash:hash(&[1,2,3])}),None);
        assert_eq!(stage.geometry(),Some(old));assert_eq!(stage.sample(15,300,10,40),Some(0xff123456));
        stage.poll_reply(|_,_|panic!("invalid transaction has no receipt"));
        publish(&mut stage,3,new,&[1,2,3]);ack(&mut stage,3);assert_eq!(stage.sample(15,300,12,40),Some(3));
    }
}
#[test]
fn full_retains_exact_ack_no_republication_and_no_new_sequence_overtakes_it() {
    let mut stage=Stage::EMPTY;let g=geometry(1,1,1);publish(&mut stage,7,g,&[0xffabcdef]);
    let mut receipts=Vec::new();for _ in 0..4 {
        stage.poll_reply(|owner,word|{receipts.push((owner,word));u64::MAX});
        begin(&mut stage,8,geometry(2,1,1));assert_eq!(stage.sequence(),7);
        assert_eq!(receive(&mut stage,Request::Clear{sequence:9}),None);
        assert_eq!(receive(&mut stage,Request::Commit{sequence:7,hash:hash(&[0xffabcdef])}),None);
        assert_eq!(stage.sample(15,300,10,40),Some(0xffabcdef));
    }
    assert_eq!(receipts.len(),4);assert!(receipts.iter().all(|r|*r==receipts[0]));ack(&mut stage,7);
    // A duplicate may repeat its retained completion, never its mutation.
    assert_eq!(receive(&mut stage,Request::Commit{sequence:7,hash:hash(&[0xffabcdef])}),None);ack(&mut stage,7);
    let change=receive(&mut stage,Request::Clear{sequence:8}).unwrap();assert_eq!((change.old,change.new),(Some(g),None));
    assert_eq!(stage.sample(15,300,10,40),None);ack(&mut stage,8);
    assert_eq!(receive(&mut stage,Request::Clear{sequence:8}),None);ack(&mut stage,8);
    for stale in [Request::Begin{sequence:7,geometry:g},Request::Commit{sequence:7,hash:hash(&[0xffabcdef])},Request::Clear{sequence:7}] {assert_eq!(receive(&mut stage,stale),None);}
    assert!(stage.scrubbed());
}
#[test]
fn foreign_sender_surface_and_late_chunks_cannot_change_or_clear_owned_state() {
    let mut stage=Stage::EMPTY;let g=geometry(1,1,1);publish(&mut stage,1,g,&[0xffabcdef]);ack(&mut stage,1);
    for (owner,sid) in [(0,300),(3,300),(9,300),(16,300),(15,301),(15,300|1<<32)] {
        for request in [Request::Begin{sequence:2,geometry:g},Request::Chunk{sequence:1,offset:0,count:1,pixels:[0,0]},Request::Commit{sequence:1,hash:0},Request::Clear{sequence:2}] {
            assert_eq!(stage.receive(owner,sid,request),None);assert_eq!(stage.sample(15,300,10,40),Some(0xffabcdef));
        }
    }
    receive(&mut stage,Request::Chunk{sequence:1,offset:0,count:1,pixels:[0,0]});
    assert_eq!(stage.sample(15,300,10,40),Some(0xffabcdef));
    stage.reset_surface(9,300);stage.retire(9);assert_eq!(stage.geometry(),Some(g));
}
#[test]
fn destroy_recreation_preserves_sequence_fence_and_retirement_permanently_erases() {
    let mut stage=Stage::EMPTY;let g=geometry(2,1,1);publish(&mut stage,4,g,&[0xffff0000,0x8000ff00]);
    let mut old_receipts=Vec::new();stage.poll_reply(|owner,word|{old_receipts.push((owner,word));u64::MAX});
    stage.reset_surface(15,300);assert!(stage.pixels_scrubbed());assert!(!stage.scrubbed());
    begin(&mut stage,5,g);assert_eq!(stage.sequence(),4);assert_eq!(receive(&mut stage,Request::Clear{sequence:6}),None);
    stage.poll_reply(|owner,word|{assert_eq!((owner,word),old_receipts[0]);u64::MAX});
    assert_eq!(stage.geometry(),None);ack(&mut stage,4);assert!(stage.scrubbed());
    for request in [Request::Begin{sequence:4,geometry:g},Request::Commit{sequence:4,hash:hash(&[0xffff0000,0x8000ff00])}] {assert_eq!(receive(&mut stage,request),None);}
    // A repeated exact completed COMMIT still returns only the historical ACK.
    ack(&mut stage,4);assert!(stage.pixels_scrubbed());
    assert_eq!(stage.geometry(),None);publish(&mut stage,5,g,&[0xffff0000,0x8000ff00]);ack(&mut stage,5);
    begin(&mut stage,6,g);chunks(&mut stage,6,&[1,2]);stage.retire(15);assert!(stage.scrubbed());
    stage.reset_surface(15,300);
    for sequence in [6,7,u32::MAX] {begin(&mut stage,sequence,g);chunks(&mut stage,sequence,&[1,2]);assert_eq!(receive(&mut stage,Request::Commit{sequence,hash:hash(&[1,2])}),None);assert_eq!(receive(&mut stage,Request::Clear{sequence}),None);}
    stage.poll_reply(|_,_|panic!("retired receipt"));assert!(stage.scrubbed());
    let mut completed=Stage::EMPTY;publish(&mut completed,1,g,&[1,2]);completed.poll_reply(|_,_|u64::MAX);
    completed.retire(15);completed.poll_reply(|_,_|panic!("retirement drops retained completion"));assert!(completed.scrubbed());
}
#[test]
fn maximum_sequence_exhausts_without_wrap_and_clear_cannot_mutate_same_sequence() {
    let mut stage=Stage::EMPTY;let g=geometry(1,1,1);publish(&mut stage,u32::MAX,g,&[0xffffffff]);ack(&mut stage,u32::MAX);
    assert_eq!(receive(&mut stage,Request::Clear{sequence:u32::MAX}),None);
    assert_eq!(stage.receive(15,300,Request::Clear{sequence:0}),None);
    begin(&mut stage,1,g);assert_eq!(stage.sample(15,300,10,40),Some(0xffffffff));
}
#[test]
fn delayed_old_commit_and_clear_cannot_poison_a_fresh_newer_stage() {
    let mut stage=Stage::EMPTY;let g=geometry(2,1,1);publish(&mut stage,1,g,&[0xff112233,0xff445566]);ack(&mut stage,1);
    begin(&mut stage,2,g);chunks(&mut stage,2,&[0xff778899,0xffaabbcc]);
    for request in [Request::Commit{sequence:1,hash:hash(&[0xff112233,0xff445566])},
        Request::Commit{sequence:1,hash:0},Request::Begin{sequence:1,geometry:geometry(48,48,4)},
        Request::Clear{sequence:1},Request::Chunk{sequence:1,offset:0,count:2,pixels:[0,0]}] {
        assert_eq!(receive(&mut stage,request),None);stage.poll_reply(|_,_|panic!("old attempt cannot create a newer receipt"));
    }
    assert_eq!(stage.sequence(),2);assert_eq!(receive(&mut stage,Request::Commit{sequence:2,hash:hash(&[0xff778899,0xffaabbcc])}).unwrap().new,Some(g));
    ack(&mut stage,2);assert_eq!(stage.sample(15,300,10,40),Some(0xff778899));
}
#[test]
fn exact_straight_alpha_scaled_sampling_and_viewport_clipping() {
    for base in [0,0x123456,0xffabcdef] {for rgb in [0,0x010203,0xffffff] {for alpha in 0..=255u32 {
        let rgba=alpha<<24|rgb;let expected=if alpha==0{base}else{
            let mut word=0xff000000;for shift in [0,8,16] {
                let n=((rgb>>shift)&255)*alpha+((base>>shift)&255)*(255-alpha);
                word|=((n+127)/255)<<shift;
            }word
        };assert_eq!(blend(rgba,base),expected);
    }}}
    let mut stage=Stage::EMPTY;let g=Geometry{x:0,y:0,..geometry(48,48,4)};
    let pixels:Vec<_>=(0..g.len()).map(|i|0xff000000|i as u32).collect();publish(&mut stage,1,g,&pixels);ack(&mut stage,1);
    for view in [Viewport{x:-30,y:20,width:200,height:200,inset:8,top:36},
        Viewport{x:70,y:70,width:35,height:48,inset:8,top:36},
        Viewport{x:0,y:0,width:7,height:7,inset:8,top:36},
        Viewport{x:i32::MAX,y:i32::MAX,width:u32::MAX,height:u32::MAX,inset:0,top:0}] {
        let footprint=view.screen(Some(g),160,140);
        for y in 0..140 {for x in 0..160 {
            let color=view.local(x,y).and_then(|(lx,ly)|stage.sample(15,300,lx,ly));
            let inside=x>=footprint.0&&x-footprint.0<footprint.2&&y>=footprint.1&&y-footprint.1<footprint.3;
            assert_eq!(color.is_some(),inside,"x={x} y={y}");
            if let Some(actual)=color {let lx=x as i64-view.x as i64;let ly=y as i64-view.y as i64;assert_eq!(actual,pixels[(ly/4)as usize*48+(lx/4)as usize]);}
        }}
    }
    assert_eq!(stage.sample(15,300,-1,0),None);assert_eq!(stage.sample(15,300,i64::MIN,i64::MIN),None);assert_eq!(stage.sample(15,300,192,0),None);assert_eq!(stage.sample(15,300,0,192),None);
    let invalid=Viewport{x:i32::MIN,y:i32::MIN,width:u32::MAX,height:u32::MAX,inset:0,top:0};
    assert_eq!(invalid.local(usize::MAX,usize::MAX),None);assert_eq!(invalid.local(i64::MAX as usize,i64::MAX as usize),None);
}
#[test]
fn movement_resize_and_clear_damage_cover_every_changed_pixel() {
    let mut stage=Stage::EMPTY;let view=Viewport{x:-10,y:50,width:120,height:120,inset:8,top:36};
    let a=Geometry{x:0,y:20,..geometry(20,20,2)};publish(&mut stage,1,a,&vec![0x8000ff00;a.len()]);ack(&mut stage,1);
    let paint=|stage:&Stage,x,y|view.local(x,y).and_then(|(lx,ly)|stage.sample(15,300,lx,ly)).map_or(0x123456,|rgba|blend(rgba,0x123456));
    let mut before=Vec::new();for y in 0..180 {for x in 0..180 {before.push(paint(&stage,x,y));}}
    let b=Geometry{x:60,y:70,..geometry(12,12,3)};begin(&mut stage,2,b);chunks(&mut stage,2,&vec![0xffaa0011;b.len()]);
    let change=receive(&mut stage,Request::Commit{sequence:2,hash:hash(&vec![0xffaa0011;b.len()])}).unwrap();
    let mut damage=damage::Damage::new();damage.add(view.screen(change.old,180,180),180,180);damage.add(view.screen(change.new,180,180),180,180);
    let rect=damage.rect().unwrap();for y in 0..180 {for x in 0..180 {if before[y*180+x]!=paint(&stage,x,y){assert!(x>=rect.x&&x<rect.right&&y>=rect.y&&y<rect.bottom);}}}
    ack(&mut stage,2);let change=receive(&mut stage,Request::Clear{sequence:3}).unwrap();assert_eq!(change.old,Some(b));assert_eq!(change.new,None);assert_eq!(stage.geometry(),None);
    // Current viewport clips a retained bitmap after resize, without reinterpreting its pixels.
    assert!(Viewport{width:10,height:10,..view}.screen(Some(b),180,180).2==0);
}
#[test]
fn production_uses_owned_live_route_both_paint_branches_and_terminal_cleanup() {
    let source=include_str!("../servers/sexdisplay/src/main.rs");
    for branch in ["c = fill_rect_color(surf, x, y, surf.color);\n        c = bitmap_color(surf, x, y, c);", "c = fill_rect_color(content, x, y, content.color);\n            c = bitmap_color(content, x, y, c);"] {assert!(source.contains(branch));}
    let branch=&source[source.find("                sex_pdx::bitmap_batch::BEGIN |").unwrap()..];
    let branch=&branch[..branch.find("                sex_pdx::text_batch::BEGIN |").unwrap()];
    assert!(branch.find("msg.caller_pd != 15 || msg.arg0 != 300").unwrap()<branch.find("BITMAP_STAGE.receive(").unwrap());
    assert!(branch.find("s.active && s.surface_id == msg.arg0 && s.owner_pd == msg.caller_pd").unwrap()<branch.find("BITMAP_STAGE.receive(").unwrap());
    assert!(source.contains("0x27A | 0x27B | 0x27C | 0x27D)\n                    && !lifecycle.sid_live(msg.arg0)"));
    assert!(source.contains("BITMAP_STAGE.retire(info.target_pd);"));
    assert!(source.contains("BITMAP_STAGE.reset_surface(owner, surface_id);"));
    let destroy=&source[source.find("                0xEE => {").unwrap()..source.find("                0xF5 => {").unwrap()];
    assert!(destroy.find("BITMAP_STAGE.reset_surface(slot.owner_pd, target_id);").unwrap()<destroy.find("*slot = SURFACE_EMPTY;").unwrap());
    assert!(source.find("finish_display_transactions(&mut lifecycle, &bar, fb_live);").unwrap()<source.find("BITMAP_STAGE.poll_reply(sex_pdx::pdx_reply);").unwrap());
}
