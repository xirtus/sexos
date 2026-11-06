//! Actual register ABI and display-private transaction/paint helpers. No mock
//! bitmap implementation or success inferred from an enqueue result.
#![allow(dead_code)]
extern crate self as sex_pdx;
#[path="../crates/sex-pdx/src/app_catalog.rs"] pub mod app_catalog;
#[path="../crates/sex-pdx/src/app_lifecycle.rs"] pub mod app_lifecycle;
#[path="../servers/sexdisplay/src/bitmap_binding.rs"] mod bitmap_binding;
#[path = "../crates/sex-pdx/src/bitmap_batch.rs"] pub mod bitmap_batch;
#[path = "../servers/sexdisplay/src/bitmap_transaction.rs"] mod transaction;
#[path = "../servers/sexdisplay/src/damage.rs"] mod damage;
use bitmap_batch::{hash, Applied, Geometry, Request, CAPACITY};
use transaction::{blend, Stage, Viewport};
fn held_binding(task:u32)->bitmap_binding::Binding {
    bitmap_binding::Binding::from_catalog(40,app_lifecycle::CatalogInfo{
        target_pd:15,task_id:task,surface_id:300,coordinator_pd:3}).unwrap()
}
fn bound_stage()->Stage {
    let mut stage=Stage::UNBOUND;
    assert!(stage.initialize_once(held_binding(150)));
    stage
}
fn geometry(width: u8, height: u8, scale: u8) -> Geometry { Geometry { x: 10, y: 40, width, height, scale } }
fn receive(stage: &mut Stage, request: Request) -> Option<transaction::Change> {
    let (op, a, b) = request.wire();
    stage.receive(15, 150, 300, Request::decode(op, a, b).unwrap())
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
        let pixels:Vec<_>=(0..g.len()).map(|i|0x80000000|i as u32).collect();let mut stage=bound_stage();
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
        assert_eq!(receive(&mut stage,Request::Commit{sequence:1,hash:hash(&pixels)}),None);stage.release_after_publication();ack(&mut stage,1);
        stage.poll_reply(|_,_|panic!("receipt admitted already"));
        assert!(core::mem::size_of::<Stage>()<19000);assert_eq!(CAPACITY*4,9216);
    }
}
#[test]
fn incomplete_bad_hash_and_conflicting_duplicates_keep_old_publication() {
    for mode in 0..7 { let mut stage=bound_stage();let old=geometry(1,1,1);publish(&mut stage,1,old,&[0xff123456]);stage.release_after_publication();ack(&mut stage,1);
        let new=geometry(3,1,1);begin(&mut stage,2,new);
        receive(&mut stage,Request::Chunk{sequence:2,offset:0,count:2,pixels:[1,2]});
        match mode {
            0=>{},
            1=>{chunks(&mut stage,2,&[1,2,3]);},
            2=>{receive(&mut stage,Request::Chunk{sequence:2,offset:0,count:2,pixels:[4,2]});},
            3=>{begin(&mut stage,2,geometry(2,1,1));},
            4=>{stage.receive(15,150,300,Request::Chunk{sequence:2,offset:0,count:usize::MAX,pixels:[0;2]});},
            5=>{stage.receive(15,150,300,Request::Chunk{sequence:2,offset:usize::MAX,count:1,pixels:[0;2]});},
            _=>{receive(&mut stage,Request::Chunk{sequence:2,offset:2,count:2,pixels:[3,0]});},
        }
        assert_eq!(receive(&mut stage,Request::Commit{sequence:2,hash:0}),None);
        chunks(&mut stage,2,&[1,2,3]);assert_eq!(receive(&mut stage,Request::Commit{sequence:2,hash:hash(&[1,2,3])}),None);
        assert_eq!(stage.geometry(),Some(old));assert_eq!(stage.sample(15,300,10,40),Some(0xff123456));
        stage.poll_reply(|_,_|panic!("invalid transaction has no receipt"));
        publish(&mut stage,3,new,&[1,2,3]);stage.release_after_publication();ack(&mut stage,3);assert_eq!(stage.sample(15,300,12,40),Some(3));
    }
}
#[test]
fn full_retains_exact_ack_no_republication_and_no_new_sequence_overtakes_it() {
    let mut stage=bound_stage();let g=geometry(1,1,1);publish(&mut stage,7,g,&[0xffabcdef]);
    assert!(stage.release_after_publication());
    let mut receipts=Vec::new();for _ in 0..4 {
        stage.poll_reply(|owner,word|{receipts.push((owner,word));u64::MAX});
        begin(&mut stage,8,geometry(2,1,1));assert_eq!(stage.sequence(),7);
        assert_eq!(receive(&mut stage,Request::Clear{sequence:9}),None);
        assert_eq!(receive(&mut stage,Request::Commit{sequence:7,hash:hash(&[0xffabcdef])}),None);
        assert_eq!(stage.sample(15,300,10,40),Some(0xffabcdef));
    }
    assert_eq!(receipts.len(),4);assert!(receipts.iter().all(|r|*r==receipts[0]));stage.release_after_publication();ack(&mut stage,7);
    // A duplicate may repeat its retained completion, never its mutation.
    assert_eq!(receive(&mut stage,Request::Commit{sequence:7,hash:hash(&[0xffabcdef])}),None);stage.release_after_publication();ack(&mut stage,7);
    let change=receive(&mut stage,Request::Clear{sequence:8}).unwrap();assert_eq!((change.old,change.new),(Some(g),None));
    assert_eq!(stage.sample(15,300,10,40),None);stage.release_after_publication();ack(&mut stage,8);
    assert_eq!(receive(&mut stage,Request::Clear{sequence:8}),None);stage.release_after_publication();ack(&mut stage,8);
    for stale in [Request::Begin{sequence:7,geometry:g},Request::Commit{sequence:7,hash:hash(&[0xffabcdef])},Request::Clear{sequence:7}] {assert_eq!(receive(&mut stage,stale),None);}
    assert!(stage.scrubbed());
}
#[test]
fn foreign_sender_surface_and_late_chunks_cannot_change_or_clear_owned_state() {
    let mut stage=bound_stage();let g=geometry(1,1,1);publish(&mut stage,1,g,&[0xffabcdef]);stage.release_after_publication();ack(&mut stage,1);
    for (owner,sid) in [(0,300),(3,300),(9,300),(16,300),(15,301),(15,300|1<<32)] {
        for request in [Request::Begin{sequence:2,geometry:g},Request::Chunk{sequence:1,offset:0,count:1,pixels:[0,0]},Request::Commit{sequence:1,hash:0},Request::Clear{sequence:2}] {
            assert_eq!(stage.receive(owner,150,sid,request),None);assert_eq!(stage.sample(15,300,10,40),Some(0xffabcdef));
        }
    }
    receive(&mut stage,Request::Chunk{sequence:1,offset:0,count:1,pixels:[0,0]});
    assert_eq!(stage.sample(15,300,10,40),Some(0xffabcdef));
    stage.reset_surface(9,300);stage.retire(9,90);assert_eq!(stage.geometry(),Some(g));
}
#[test]
fn destroy_recreation_preserves_sequence_fence_and_retirement_permanently_erases() {
    let mut stage=bound_stage();let g=geometry(2,1,1);publish(&mut stage,4,g,&[0xffff0000,0x8000ff00]);
    assert!(stage.release_after_publication());
    let mut old_receipts=Vec::new();stage.poll_reply(|owner,word|{old_receipts.push((owner,word));u64::MAX});
    stage.reset_surface(15,300);assert!(stage.pixels_scrubbed());assert!(!stage.scrubbed());
    begin(&mut stage,5,g);assert_eq!(stage.sequence(),4);assert_eq!(receive(&mut stage,Request::Clear{sequence:6}),None);
    stage.poll_reply(|owner,word|{assert_eq!((owner,word),old_receipts[0]);u64::MAX});
    assert_eq!(stage.geometry(),None);stage.release_after_publication();ack(&mut stage,4);assert!(stage.scrubbed());
    for request in [Request::Begin{sequence:4,geometry:g},Request::Commit{sequence:4,hash:hash(&[0xffff0000,0x8000ff00])}] {assert_eq!(receive(&mut stage,request),None);}
    // A repeated exact completed COMMIT still returns only the historical ACK.
    stage.release_after_publication();ack(&mut stage,4);assert!(stage.pixels_scrubbed());
    assert_eq!(stage.geometry(),None);publish(&mut stage,5,g,&[0xffff0000,0x8000ff00]);stage.release_after_publication();ack(&mut stage,5);
    begin(&mut stage,6,g);chunks(&mut stage,6,&[1,2]);stage.retire(15,150);assert!(stage.scrubbed());
    stage.reset_surface(15,300);
    for sequence in [6,7,u32::MAX] {begin(&mut stage,sequence,g);chunks(&mut stage,sequence,&[1,2]);assert_eq!(receive(&mut stage,Request::Commit{sequence,hash:hash(&[1,2])}),None);assert_eq!(receive(&mut stage,Request::Clear{sequence}),None);}
    stage.poll_reply(|_,_|panic!("retired receipt"));assert!(stage.scrubbed());
    let mut completed=bound_stage();publish(&mut completed,1,g,&[1,2]);assert!(completed.release_after_publication());completed.poll_reply(|_,_|u64::MAX);
    completed.retire(15,150);completed.poll_reply(|_,_|panic!("retirement drops retained completion"));assert!(completed.scrubbed());
}
#[test]
fn maximum_sequence_exhausts_without_wrap_and_clear_cannot_mutate_same_sequence() {
    let mut stage=bound_stage();let g=geometry(1,1,1);publish(&mut stage,u32::MAX,g,&[0xffffffff]);stage.release_after_publication();ack(&mut stage,u32::MAX);
    assert_eq!(receive(&mut stage,Request::Clear{sequence:u32::MAX}),None);
    assert_eq!(stage.receive(15,150,300,Request::Clear{sequence:0}),None);
    begin(&mut stage,1,g);assert_eq!(stage.sample(15,300,10,40),Some(0xffffffff));
}
#[test]
fn delayed_old_commit_and_clear_cannot_poison_a_fresh_newer_stage() {
    let mut stage=bound_stage();let g=geometry(2,1,1);publish(&mut stage,1,g,&[0xff112233,0xff445566]);stage.release_after_publication();ack(&mut stage,1);
    begin(&mut stage,2,g);chunks(&mut stage,2,&[0xff778899,0xffaabbcc]);
    for request in [Request::Commit{sequence:1,hash:hash(&[0xff112233,0xff445566])},
        Request::Commit{sequence:1,hash:0},Request::Begin{sequence:1,geometry:geometry(48,48,4)},
        Request::Clear{sequence:1},Request::Chunk{sequence:1,offset:0,count:2,pixels:[0,0]}] {
        assert_eq!(receive(&mut stage,request),None);stage.poll_reply(|_,_|panic!("old attempt cannot create a newer receipt"));
    }
    assert_eq!(stage.sequence(),2);assert_eq!(receive(&mut stage,Request::Commit{sequence:2,hash:hash(&[0xff778899,0xffaabbcc])}).unwrap().new,Some(g));
    stage.release_after_publication();ack(&mut stage,2);assert_eq!(stage.sample(15,300,10,40),Some(0xff778899));
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
    let mut stage=bound_stage();let g=Geometry{x:0,y:0,..geometry(48,48,4)};
    let pixels:Vec<_>=(0..g.len()).map(|i|0xff000000|i as u32).collect();publish(&mut stage,1,g,&pixels);stage.release_after_publication();ack(&mut stage,1);
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
    let mut stage=bound_stage();let view=Viewport{x:-10,y:50,width:120,height:120,inset:8,top:36};
    let a=Geometry{x:0,y:20,..geometry(20,20,2)};publish(&mut stage,1,a,&vec![0x8000ff00;a.len()]);stage.release_after_publication();ack(&mut stage,1);
    let paint=|stage:&Stage,x,y|view.local(x,y).and_then(|(lx,ly)|stage.sample(15,300,lx,ly)).map_or(0x123456,|rgba|blend(rgba,0x123456));
    let mut before=Vec::new();for y in 0..180 {for x in 0..180 {before.push(paint(&stage,x,y));}}
    let b=Geometry{x:60,y:70,..geometry(12,12,3)};begin(&mut stage,2,b);chunks(&mut stage,2,&vec![0xffaa0011;b.len()]);
    let change=receive(&mut stage,Request::Commit{sequence:2,hash:hash(&vec![0xffaa0011;b.len()])}).unwrap();
    let mut damage=damage::Damage::new();damage.add(view.screen(change.old,180,180),180,180);damage.add(view.screen(change.new,180,180),180,180);
    let rect=damage.rect().unwrap();for y in 0..180 {for x in 0..180 {if before[y*180+x]!=paint(&stage,x,y){assert!(x>=rect.x&&x<rect.right&&y>=rect.y&&y<rect.bottom);}}}
    stage.release_after_publication();ack(&mut stage,2);let change=receive(&mut stage,Request::Clear{sequence:3}).unwrap();assert_eq!(change.old,Some(b));assert_eq!(change.new,None);assert_eq!(stage.geometry(),None);
    // Current viewport clips a retained bitmap after resize, without reinterpreting its pixels.
    assert!(Viewport{width:10,height:10,..view}.screen(Some(b),180,180).2==0);
}
#[test]
fn production_uses_owned_live_route_both_paint_branches_and_terminal_cleanup() {
    let source=include_str!("../servers/sexdisplay/src/main.rs");
    for branch in ["c = fill_rect_color(surf, x, y, surf.color);\n        c = bitmap_color(surf, x, y, c, scene);", "c = fill_rect_color(content, x, y, content.color);\n            c = bitmap_color(content, x, y, c, scene);"] {assert!(source.contains(branch));}
    let shader=&source[source.find("fn bitmap_color(").unwrap()..source.find("fn bitmap_surface_damage(").unwrap()];
    assert!(shader.contains("scene.bitmaps.matches_surface(surf.owner_pd, surf.surface_id)"));
    assert!(shader.contains("scene.bitmaps.sample(surf.owner_pd, surf.surface_id, x, y)"));
    assert!(!shader.contains("BITMAP_STORES"));
    assert!(source.contains("static mut BITMAP_STORES: bitmap_stores::Stores<{sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser)}> = bitmap_stores::Stores::UNBOUND;"));
    assert!(!source.contains("BITMAP_STAGE"));
    assert_eq!(source.matches("BITMAP_STORES.initialize_once(").count(),1);
    let load=&source[source.find("fn load_lifecycle_catalog()").unwrap()..source.find("fn bitmap_binding_live(").unwrap()];
    assert!(load.find("sex_pdx::app_lifecycle::catalog(slot)").unwrap()<load.find("bitmap_binding::Binding::from_catalog(slot,c)").unwrap());
    assert!(load.find("bitmap_binding::Binding::from_catalog(slot,c)").unwrap()<load.find("BITMAP_STORES.initialize_once(binding)").unwrap());
    assert!(load.contains("assert!(unsafe { BITMAP_STORES.initialize_once(binding) },"));
    assert!(load.contains("assert_eq!(unsafe { BITMAP_STORES.initialized_count() },sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser),"));
    assert!(source.find("let mut lifecycle = load_lifecycle_catalog();").unwrap()<source.find("let Some(msg) = sex_pdx::pdx_try_listen_raw(0)").unwrap());
    let branch=&source[source.find("                sex_pdx::bitmap_batch::BEGIN |").unwrap()..];
    let branch=&branch[..branch.find("                sex_pdx::text_batch::BEGIN |").unwrap()];
    assert!(branch.find("BITMAP_STORES.binding_for(msg.caller_pd,msg.arg0)").unwrap()<branch.find("binding.admits(msg.caller_pd,msg.caller_task,msg.arg0)").unwrap());
    assert!(branch.contains("BITMAP_STORES.sequence(binding.target(),binding.surface()).expect(\"committed bitmap remains bound\")"));
    assert!(branch.find("binding.admits(msg.caller_pd,msg.caller_task,msg.arg0)").unwrap()<branch.find("BITMAP_STORES.receive(").unwrap());
    assert!(branch.find("s.active && s.surface_id == msg.arg0 && s.owner_pd == msg.caller_pd").unwrap()<branch.find("BITMAP_STORES.receive(").unwrap());
    assert!(branch.contains("BITMAP_STORES.receive(msg.caller_pd, msg.caller_task, msg.arg0, request)"));
    assert!(branch.find("bitmap_binding_live(&lifecycle,binding)").unwrap()<branch.find("BITMAP_STORES.receive(").unwrap());
    assert!(source.contains("0x27A | 0x27B | 0x27C | 0x27D | 0x28B | 0x28C)\n                    && !lifecycle.sid_live(msg.arg0)"));
    assert!(source.contains("BITMAP_STORES.retire(info.target_pd,info.task_id);"));
    assert!(source.contains("BITMAP_STORES.reset_surface(owner, surface_id);"));
    let destroy=&source[source.find("                0xEE => {").unwrap()..source.find("                0xF5 => {").unwrap()];
    assert!(destroy.find("BITMAP_STORES.reset_surface(slot.owner_pd, target_id);").unwrap()<destroy.find("*slot = SURFACE_EMPTY;").unwrap());
    assert!(source.contains("pending_paint.complete(published.is_some());"));
    assert!(source.find("finish_display_transactions(&mut lifecycle,published.as_ref());").unwrap()<source.find("BITMAP_STORES.poll_reply(sex_pdx::pdx_reply);").unwrap());
}

#[test]
fn unbound_store_rejects_packets_before_effects_and_never_uses_first_message_binding() {
    let mut stage=Stage::UNBOUND;let g=geometry(1,1,1);
    assert_eq!(stage.binding(),None);assert!(!stage.matches_surface(15,300));
    for request in [Request::Begin{sequence:1,geometry:g},
                    Request::Chunk{sequence:1,offset:usize::MAX,count:usize::MAX,pixels:[9,9]},
                    Request::Commit{sequence:1,hash:hash(&[9])},Request::Clear{sequence:u32::MAX}] {
        assert_eq!(stage.receive(15,150,300,request),None);
        assert_eq!(stage.binding(),None);assert_eq!(stage.sequence(),0);
        assert_eq!(stage.geometry(),None);assert!(stage.scrubbed());
    }
    stage.reset_surface(15,300);stage.retire(15,150);
    assert_eq!(stage.sample(15,300,10,40),None);stage.poll_reply(|_,_|panic!("unbound receipt"));
    // Unmatched packets/retirement hints cannot choose or permanently poison
    // the slot. Only the explicit held-query startup installation may bind it.
    assert!(stage.initialize_once(held_binding(150)));assert_eq!(stage.binding(),Some(held_binding(150)));
    publish(&mut stage,1,g,&[9]);stage.release_after_publication();ack(&mut stage,1);
}

#[test]
fn initialization_never_rebinds_or_revives_after_reset_and_terminal_retirement() {
    let mut stage=Stage::UNBOUND;let first=held_binding(150);let other_task=held_binding(151);
    assert!(stage.initialize_once(first));assert!(!stage.initialize_once(first));assert!(!stage.initialize_once(other_task));
    assert_eq!(stage.binding(),Some(first));
    publish(&mut stage,1,geometry(1,1,1),&[0xff123456]);stage.release_after_publication();ack(&mut stage,1);
    stage.reset_surface(15,300);assert!(stage.pixels_scrubbed());assert_eq!(stage.sequence(),1);
    assert!(!stage.initialize_once(first));assert!(!stage.initialize_once(other_task));
    stage.retire(15,150);assert!(stage.scrubbed());assert_eq!(stage.binding(),Some(first));
    stage.reset_surface(15,300);
    assert!(!stage.initialize_once(first));assert!(!stage.initialize_once(other_task));
    for task in [150,151]{for request in [Request::Begin{sequence:2,geometry:geometry(1,1,1)},
        Request::Chunk{sequence:2,offset:0,count:1,pixels:[1,0]},Request::Commit{sequence:2,hash:hash(&[1])},Request::Clear{sequence:3}] {
        assert_eq!(stage.receive(15,task,300,request),None);assert_eq!(stage.sequence(),1);assert!(stage.scrubbed());
    }}
    stage.poll_reply(|_,_|panic!("retirement cannot manufacture fresh completion"));
}

#[test]
fn exact_held_task_is_required_before_each_operation_including_poisoning_chunks() {
    let g=geometry(1,1,1);let old=0xff123456;let next=0xffabcdef;
    for wrong_task in [0,15,151,u32::MAX]{for operation in 0..4 {
        let mut stage=bound_stage();publish(&mut stage,1,g,&[old]);stage.release_after_publication();ack(&mut stage,1);
        if operation>0{begin(&mut stage,2,g);}
        if operation>1{chunks(&mut stage,2,&[next]);}
        let sequence=stage.sequence();
        let request=match operation {
            0=>Request::Begin{sequence:2,geometry:geometry(2,1,1)},
            1=>Request::Chunk{sequence:2,offset:0,count:1,pixels:[0,0]},
            2=>Request::Commit{sequence:2,hash:hash(&[next])},
            _=>Request::Clear{sequence:3},
        };
        assert_eq!(stage.receive(15,wrong_task,300,request),None);
        assert_eq!(stage.sequence(),sequence);assert_eq!(stage.geometry(),Some(g));
        assert_eq!(stage.sample(15,300,10,40),Some(old));stage.poll_reply(|_,_|panic!("wrong task receipt"));
        if operation==0{begin(&mut stage,2,g);}
        if operation<=1{chunks(&mut stage,2,&[next]);}
        // Rejected task metadata must not poison an otherwise complete exact
        // transaction, and an accepted-looking foreign COMMIT cannot publish.
        assert_eq!(receive(&mut stage,Request::Commit{sequence:2,hash:hash(&[next])}).unwrap().new,Some(g));
        stage.release_after_publication();ack(&mut stage,2);assert_eq!(stage.sample(15,300,10,40),Some(next));
    }}
}

#[test]
fn bound_full_historical_receipt_keeps_destination_and_sequence_after_reset() {
    let mut stage=bound_stage();let binding=held_binding(150);let g=geometry(1,1,1);
    publish(&mut stage,7,g,&[0xff112233]);
    assert!(stage.release_after_publication());
    let expected=(15,Applied{surface:300,sequence:7}.encode().unwrap());
    for _ in 0..3{stage.poll_reply(|target,word|{assert_eq!((target,word),expected);99});}
    assert!(!stage.initialize_once(held_binding(151)));
    stage.reset_surface(15,343);stage.reset_surface(3,300);
    assert_eq!(stage.sample(15,300,10,40),Some(0xff112233));
    stage.reset_surface(15,300);assert!(stage.pixels_scrubbed());assert_eq!(stage.sequence(),7);
    stage.retire(15,151);assert_eq!(stage.binding(),Some(binding));
    for _ in 0..3 {
        assert_eq!(receive(&mut stage,Request::Clear{sequence:8}),None);
        assert_eq!(stage.sequence(),7);assert_eq!(stage.geometry(),None);
        stage.poll_reply(|target,word|{assert_eq!((target,word),expected);u64::MAX});
    }
    stage.poll_reply(|target,word|{assert_eq!((target,word),expected);0});assert!(stage.scrubbed());
    let change=receive(&mut stage,Request::Clear{sequence:8}).unwrap();assert_eq!((change.old,change.new),(None,None));
    assert!(stage.release_after_publication());
    stage.poll_reply(|target,word|{assert_eq!((target,word),(15,Applied{surface:300,sequence:8}.encode().unwrap()));0});
    assert!(stage.scrubbed());
}

#[test]
fn wrong_task_retirement_cannot_erase_staging_publication_or_retained_completion() {
    let mut stage=bound_stage();let g=geometry(1,1,1);
    publish(&mut stage,1,g,&[0xff123456]);stage.release_after_publication();ack(&mut stage,1);begin(&mut stage,2,g);chunks(&mut stage,2,&[0xffabcdef]);
    for (owner,task) in [(15,0),(15,15),(15,151),(0,150),(3,150),(u32::MAX,150)] {
        stage.retire(owner,task);assert_eq!(stage.sequence(),2);assert_eq!(stage.geometry(),Some(g));
        assert_eq!(stage.sample(15,300,10,40),Some(0xff123456));
    }
    receive(&mut stage,Request::Commit{sequence:2,hash:hash(&[0xffabcdef])}).unwrap();
    stage.retire(15,15);assert_eq!(stage.sample(15,300,10,40),Some(0xffabcdef));stage.release_after_publication();ack(&mut stage,2);
    publish(&mut stage,3,g,&[0xff010203]);stage.retire(15,150);
    assert!(stage.scrubbed());assert_eq!(stage.sample(15,300,10,40),None);
    stage.poll_reply(|_,_|panic!("exact retirement removes pending completion"));
    assert!(!stage.initialize_once(held_binding(150)));
}

fn completed_request(stage: &mut Stage, clear: bool) -> Request {
    if clear {
        let request=Request::Clear{sequence:7};
        assert_eq!(receive(stage,request).unwrap(),transaction::Change{old:None,new:None});
        request
    } else {
        publish(stage,7,geometry(1,1,1),&[0xff123456]);
        Request::Commit{sequence:7,hash:hash(&[0xff123456])}
    }
}

#[test]
fn unreleased_completion_cannot_reply_or_be_bypassed_by_duplicates() {
    for clear in [false,true] {
        let mut stage=bound_stage();let request=completed_request(&mut stage,clear);
        // Failed/absent Full publication supplies no release. Neither an empty
        // Clear nor repeated COMMIT may turn admission into settlement.
        for _ in 0..4 {
            assert!(stage.needs_publication());
            assert_eq!(receive(&mut stage,request),None);
            stage.poll_reply(|_,_|panic!("unreleased completion replied"));
            begin(&mut stage,8,geometry(2,1,1));
            assert_eq!(receive(&mut stage,Request::Clear{sequence:9}),None);
            assert_eq!(stage.sequence(),7);
        }
        assert!(stage.release_after_publication());assert!(!stage.needs_publication());
        assert!(!stage.release_after_publication());
        let expected=(15,Applied{surface:300,sequence:7}.encode().unwrap());
        let mut attempts=0;
        for _ in 0..4 {
            stage.poll_reply(|owner,word|{attempts+=1;assert_eq!((owner,word),expected);99});
            begin(&mut stage,8,geometry(2,1,1));
            assert_eq!(receive(&mut stage,Request::Clear{sequence:9}),None);
            assert_eq!(stage.sequence(),7);assert!(!stage.needs_publication());
        }
        assert_eq!(attempts,4);ack(&mut stage,7);
        assert!(!stage.release_after_publication());
        stage.poll_reply(|_,_|panic!("publication replayed sent completion"));
        assert_eq!(receive(&mut stage,request),None);
        assert!(!stage.needs_publication());ack(&mut stage,7);
        begin(&mut stage,8,geometry(1,1,1));assert_eq!(stage.sequence(),8);
        assert!(!stage.needs_publication());assert!(!stage.release_after_publication());
    }
}

#[test]
fn reset_before_release_retains_custody_but_erases_every_pixel() {
    for clear in [false,true] {for eligible in [false,true] {
        let mut stage=bound_stage();let request=completed_request(&mut stage,clear);
        if eligible {assert!(stage.release_after_publication());}
        stage.reset_surface(15,300);
        assert_eq!(stage.binding(),Some(held_binding(150)));
        assert_eq!(stage.sequence(),7);assert!(stage.pixels_scrubbed());
        assert!(!stage.scrubbed());assert_eq!(stage.needs_publication(),!eligible);
        let mut snapshot=transaction::Published::EMPTY;stage.snapshot_into(&mut snapshot);
        assert!(snapshot.pixels_scrubbed());assert_eq!(snapshot.geometry(),None);
        assert_eq!(snapshot.sample(15,300,10,40),None);
        assert_eq!(receive(&mut stage,request),None);
        assert_eq!(stage.needs_publication(),!eligible);
        if !eligible {stage.poll_reply(|_,_|panic!("reset released historical receipt early"));}
        begin(&mut stage,8,geometry(1,1,1));assert_eq!(stage.sequence(),7);
        assert_eq!(receive(&mut stage,Request::Clear{sequence:8}),None);
        assert_eq!(stage.release_after_publication(),!eligible);
        ack(&mut stage,7);assert!(stage.scrubbed());
        // Sent history may requeue after reset, without replaying any payload.
        stage.reset_surface(15,300);assert_eq!(receive(&mut stage,request),None);
        assert!(!stage.needs_publication());ack(&mut stage,7);
        assert!(stage.pixels_scrubbed());assert_eq!(stage.sample(15,300,10,40),None);
    }}
}

#[test]
fn terminal_retirement_cancels_unreleased_eligible_and_sent_history() {
    for clear in [false,true] {for phase in 0..3 {
        let mut stage=bound_stage();let request=completed_request(&mut stage,clear);
        if phase>0 {assert!(stage.release_after_publication());}
        if phase==2 {ack(&mut stage,7);}
        stage.retire(15,15);assert_eq!(stage.needs_publication(),phase==0);
        stage.retire(15,150);assert!(stage.scrubbed());
        assert!(!stage.needs_publication());assert!(!stage.release_after_publication());
        stage.reset_surface(15,300);assert_eq!(receive(&mut stage,request),None);
        assert!(!stage.release_after_publication());
        stage.poll_reply(|_,_|panic!("terminal history replied"));
        assert_eq!(stage.sequence(),7);assert_eq!(stage.binding(),Some(held_binding(150)));
    }}
}

#[test]
fn independent_eligible_owner_progresses_while_other_receipt_is_full() {
    let mut first=bound_stage();completed_request(&mut first,true);
    let mut second=Stage::UNBOUND;
    let binding=bitmap_binding::Binding::from_catalog(154,app_lifecycle::CatalogInfo{
        target_pd:30,task_id:300,surface_id:343,coordinator_pd:3}).unwrap();
    assert!(second.initialize_once(binding));
    assert!(second.receive(30,300,343,Request::Clear{sequence:11}).is_some());
    assert!(first.release_after_publication());assert!(second.release_after_publication());
    let mut attempts=Vec::new();
    for _ in 0..3 {
        first.poll_reply(|owner,word|{attempts.push((owner,word));99});
        second.poll_reply(|owner,word|{attempts.push((owner,word));0});
    }
    assert_eq!(attempts,vec![
        (15,Applied{surface:300,sequence:7}.encode().unwrap()),
        (30,Applied{surface:343,sequence:11}.encode().unwrap()),
        (15,Applied{surface:300,sequence:7}.encode().unwrap()),
        (15,Applied{surface:300,sequence:7}.encode().unwrap()),
    ]);
    assert!(!first.needs_publication());assert!(!second.needs_publication());
    ack(&mut first,7);
}

#[test]
fn unbound_incomplete_or_rejected_transfer_cannot_acquire_release() {
    let mut unbound=Stage::UNBOUND;
    assert!(!unbound.needs_publication());assert!(!unbound.release_after_publication());
    let mut stage=bound_stage();begin(&mut stage,1,geometry(1,1,1));chunks(&mut stage,1,&[9]);
    assert!(!stage.needs_publication());assert!(!stage.release_after_publication());
    assert_eq!(receive(&mut stage,Request::Commit{sequence:1,hash:0}),None);
    assert!(!stage.needs_publication());assert!(!stage.release_after_publication());
    stage.poll_reply(|_,_|panic!("rejected transfer replied"));
}
