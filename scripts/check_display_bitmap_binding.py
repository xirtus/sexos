#!/usr/bin/env python3
"""Held Display bitmap metadata and cached-live host checks, not BOOT grants.

Source wiring is inspected; actual Binding tests and extracted live/text guards
with actual Lifecycle, Stores and text Stage modules are compiled. No guest or
Display pixel claim. Text guards apply only to catalogued Browser subjects.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
INFERENCE_TEST='actual_browser_catalog_and_unequal_task_bind_exactly'
SENDER_TEST='exact_sender_tuple_and_retirement_require_actual_task_not_domain'


def compact(source):return re.sub(r'\s+','',source)


def function(source,name):
    matches=re.findall(r'^fn '+name+r'\b.*?^}',source,re.M|re.S)
    if len(matches)!=1:raise ValueError('expected one actual '+name)
    return matches[0]


def require(source,fragment,label):
    if compact(fragment)not in compact(source):raise ValueError('missing '+label)


def wiring(source):
    require(source,'static mut BITMAP_STORES: bitmap_stores::Stores<{sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser)}> = bitmap_stores::Stores::UNBOUND;','unbound static backing')
    if source.count('BITMAP_STORES.initialize_once(')!=1:raise ValueError('bitmap initialization must occur exactly once')
    if source.count('bitmap_binding::Binding::from_catalog(')!=1:raise ValueError('unexpected packet-time/repeated bitmap constructor')
    load=function(source,'load_lifecycle_catalog');body=compact(load)
    ordered=['sex_pdx::app_lifecycle::catalog(slot)',
             'lifecycle.register(surface_lifecycle::Catalog{',
             'bitmap_binding::Binding::from_catalog(slot,c)',
             'BITMAP_STORES.initialize_once(binding)']
    indices=[body.index(value)for value in ordered]
    if indices!=sorted(indices):raise ValueError('held query/registration/binding/initialization order changed')
    require(load,'for slot in sex_pdx::app_lifecycle::SLOTS','held slot inventory')
    require(load,'slot, target_pd:c.target_pd, task_id:c.task_id, surface_id:c.surface_id, coordinator_pd:c.coordinator_pd','held registry tuple')
    require(load,'if sex_pdx::app_catalog::by_surface(c.target_pd,c.surface_id).is_some_and(|row|row.app==sex_pdx::app_catalog::App::Browser)','exact Browser source association')
    require(load,'assert!(unsafe { BITMAP_STORES.initialize_once(binding) },"display bitmap startup installed once");','startup installation failure fence')
    require(load,'assert_eq!(unsafe { BITMAP_STORES.initialized_count() },sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser),"missing display Browser bitmap binding");','complete held Browser store count')
    start=source[source.index('pub extern "C" fn _start()'):]
    if compact(start).index('letmutlifecycle=load_lifecycle_catalog();')>=compact(start).index('sex_pdx::pdx_try_listen_raw(0)'):
        raise ValueError('bitmap binding must precede dispatch intake')
    begin=source.index('                sex_pdx::bitmap_batch::BEGIN |')
    end=source.index('                sex_pdx::text_batch::BEGIN |',begin)
    dispatch=compact(source[begin:end])
    guards=['BITMAP_STORES.binding_for(msg.caller_pd,msg.arg0)',
            'if!binding.admits(msg.caller_pd,msg.caller_task,msg.arg0)||!bitmap_binding_live(&lifecycle,binding){continue;}',
            'sex_pdx::bitmap_batch::Request::decode(msg.type_id,msg.arg1,msg.arg2)',
            's.active&&s.surface_id==msg.arg0&&s.owner_pd==msg.caller_pd',
            'BITMAP_STORES.receive(msg.caller_pd,msg.caller_task,msg.arg0,request)']
    positions=[dispatch.index(value)for value in guards]
    if positions!=sorted(positions):raise ValueError('bitmap dispatch guard order changed')
    require(dispatch,'let sequence=BITMAP_STORES.sequence(binding.target(),binding.surface()).expect("committed bitmap remains bound");','exact committed owner sequence')
    text_begin=source.index('                sex_pdx::text_batch::BEGIN |')
    text_end=source.index('                // ── Text rendering opcodes (V1)',text_begin)
    text_dispatch=compact(source[text_begin:text_end])
    text_order=['if!browser_text_binding_live(&lifecycle,msg.caller_pd,msg.caller_task,msg.arg0){continue;}',
                'sex_pdx::text_batch::Request::decode(msg.type_id,msg.arg1,msg.arg2)',
                'for(index,slot)inSURFACES.iter().enumerate()',
                'if!slot.active||slot.surface_id!=msg.arg0||slot.owner_pd!=msg.caller_pd{continue;}',
                'ifTEXT_STAGES[index].receive(msg.caller_pd,msg.caller_task,msg.arg0,request).is_some()',
                'letpending=TEXT_STAGES[index].pending().expect("sealedtextcandidate");',
                'letcontext=pending.context();', '[display.text.prepared]']
    text_positions=[text_dispatch.index(value)for value in text_order]
    if text_positions!=sorted(text_positions):raise ValueError('Browser text guard/seal evidence order changed')
    require(text_dispatch,'"[display.text.prepared] target={} task={} surface={} paint={} bytes={} pixels=unconfirmed",context.owner(),context.task(),context.surface(),context.sequence(),pending.len()','exact sealed candidate diagnostic')
    # Intake seals only. Canonical payload/layout and completion eligibility
    # cannot change here, including through an indirect helper call.
    for forbidden in ('apply_pending_text(', 'recompute_text_lines(', 'release_matching(',
                      'release_after_publication(', 'text_buf', 'text_len', 'text_color',
                      'selection_start', 'selection_end', 'SURFACES.iter_mut()',
                      '[display.text.committed]', '[display.browser.text.committed]'):
        if forbidden in text_dispatch:raise ValueError('typed intake changes canonical text or completion: '+forbidden)
    commit=function(source,'commit_published_text');canonical=compact(commit)
    commit_order=['for(index,projection)inpublished.text_projections.iter().copied().enumerate()',
                  'letSome(projection)=projectionelse{continue;};',
                  'ifletscene::TextCoverage::Body(required)=projection.coverage{',
                  'if!published.plan.covers_body(required){continue;}',
                  'letcontext=projection.context;',
                  'letstage=&mutTEXT_STAGES[index];',
                  'letSome(pending)=stage.pending()else{continue;};',
                  'ifpending.context()!=context{continue;}',
                  'letslot=&mutSURFACES[index];',
                  'letlen=pending.len();',
                  'if!apply_pending_text(slot,&pending){continue;}',
                  'assert!(stage.release_matching(context));',
                  '[display.text.committed]', '[display.browser.text.committed]']
    commit_positions=[canonical.index(value)for value in commit_order]
    if commit_positions!=sorted(commit_positions):raise ValueError('captured canonical text commit/release order changed')
    require(commit,'if sex_pdx::app_catalog::app_of(context.owner())==Some(sex_pdx::app_catalog::App::Browser)','Browser-only canonical parallel marker')
    require(commit,'"[display.text.committed] target={} surface={} paint={} bytes={} changed={} pixels=unconfirmed",context.owner(),context.surface(),context.sequence(),len,changed as u8','unchanged historical marker after canonical commit')
    require(commit,'"[display.browser.text.committed] target={} task={} surface={} paint={} bytes={} hash={} changed={} pixels=unconfirmed",context.owner(),context.task(),context.surface(),context.sequence(),len,sex_pdx::text_batch::hash(&slot.text_buf[..len]),changed as u8','actual canonical bytes hash and retained admitted task')
    apply=function(source,'apply_pending_text')
    require(apply,'if !slot.active||slot.owner_pd!=context.owner()||slot.surface_id!=context.surface()||pending.len()>TEXT_BUF_CAP{return false;}','canonical association recheck')
    require(apply,'slot.text_buf.copy_from_slice(pending.bytes());','owned pending bytes canonical copy')
    require(apply,'(slot.selection_start,slot.selection_end)=pending.selection();','same candidate selection copy')
    require(apply,'recompute_text_lines(slot);','actual shared scalar layout recompute')
    # The successful CPU publication carries contexts copied from the owned
    # scene. These identities are descriptive, never newly minted authority.
    prepared=function(source,'render_prepared');publication=compact(prepared)
    publication_order=['SceneLease::capture(w,h,bar)', 'letscene=lease.view();',
                       'raster_batch(&target,scene,plan)',
                       'preparation.commit(|frame|publish_frame(fb,w,h,frame))',
                       'lettext_projections=scene.text_projections;',
                       'Some(PublishedFrame{plan,present_bits,canon_ss,text_projections})']
    publication_positions=[publication.index(value)for value in publication_order]
    if publication_positions!=sorted(publication_positions):raise ValueError('captured text context/publication flow changed')
    require(prepared,'if !preparation.commit(|frame|publish_frame(fb,w,h,frame)){backup.restore();return None;}','failed CPU publication withholds captured completion')
    require(source,'destination.capture(&SURFACES, &FRAME_GROUPS, &BITMAP_STORES, &TEXT_STAGES, bar,DISPLAY_TOKENS, FOCUSED_SURFACE_ID, frame, w, h)','actual Stage inputs to owned scene')
    finish=function(source,'finish_display_transactions');settlement=compact(finish)
    settlement_order=['ifunsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return;}',
                      'ifletSome(published)=published{',
                      'lettext=commit_published_text(published);']
    settlement_positions=[settlement.index(value)for value in settlement_order]
    if settlement_positions!=sorted(settlement_positions):raise ValueError('covered canonical text commit escapes Scene/publication fence')
    require(finish,'let bitmap=if published.plan.is_full(){unsafe{BITMAP_STORES.release_after_publication()}}else{0};','bitmap settlement remains Full-only')
    require(finish,'if published.plan.is_full(){lifecycle.pixels_presented(published.present_bits);for index in 0..surface_lifecycle::CATALOG_CAPACITY {if lifecycle.scene_committed(index) {','lifecycle visibility/cleanup remains Full-only')
    require(finish,'"[display.typed.released] text={} bitmap={} full={} scene_released=1 publication=1 pixels=unconfirmed",text,bitmap,published.plan.is_full() as u8','actual publication kind diagnostic')
    require(source,'text_projections:[Option<scene::TextProjection>;MAX_SURFACES]','captured context and coverage receipt')
    requirement=function(source,'text_projection_requirement')
    require(requirement,'required.add(text_surface_damage(old,w,h),w,h);required.add(text_surface_damage(new,w,h),w,h);','old and new clipped text damage union')
    require(requirement,'old.text_buf==new.text_buf&&old.text_len==new.text_len&&old.text_color==new.text_color&&(old.selection_start,old.selection_end)==(new.selection_start,new.selection_end)&&old.line_starts==new.line_starts&&old.cell_starts==new.cell_starts&&old.line_cells==new.line_cells&&old.cell_count==new.cell_count&&old.line_count==new.line_count&&old.wrap_cols==new.wrap_cols','actual payload, selection and derived layout comparison')
    join=function(source,'join_pending_text')
    require(join,'let Some(pending)=stage.pending()else{continue;};','sealed-only pending requirement')
    require(join,'if h<51{paint.request_full();continue;}','tiny geometry Full promotion')
    require(join,'if ATLAS_PHASE_C_RENDER_STUB_PROOF_ENABLED||ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF_ENABLED{paint.request_area();continue;}','atlas body coverage promotion')
    require(join,'let original=&SURFACES[index];let mut candidate=*original;','private temporary projection')
    require(join,'if !apply_pending_text(&mut candidate,&pending){paint.request_full();continue;}','invalid candidate fail closed')
    require(join,'match text_projection_requirement(original,&candidate,w,h){scene::TextCoverage::NoPixels=>paint.request_settle(),scene::TextCoverage::Body(rect)=>paint.request_damage(rect,w,h),}','explicit no-pixel settlement and body requirement')
    require(start,'let transaction_full_render=unsafe{BITMAP_STORES.needs_publication()}||lifecycle.needs_presentation()||(0..surface_lifecycle::CATALOG_CAPACITY).any(|i|lifecycle.phase(i)==Some(surface_lifecycle::Phase::Prepared));','bitmap and lifecycle Full obligation')
    require(start,'if needs_top_strip_redraw{pending_paint.request_strip();}if typed_completion_needs_publication(){join_pending_text(&mut pending_paint,w,h);}let plan=pending_paint.plan(w,h);','pending projection joins after drain flags before final plan')
    if source.count('commit_published_text(')!=2:raise ValueError('canonical text commit has unexpected entrypoint')
    handoff=compact(start)
    handoff_start=handoff.index('letpublished=iffb_live&&!plan.is_empty(){')
    handoff_end=handoff.index('finish_display_transactions(&mutlifecycle,published.as_ref());',handoff_start)
    if 'pdx_try_listen_raw(' in handoff[handoff_start:handoff_end]:
        raise ValueError('new intake between captured publication and canonical commit')
    for boundary in (prepared,commit,finish):
        if 'pdx_try_listen_raw(' in boundary:raise ValueError('new intake inside text publication/commit boundary')
    cleanup=function(source,'prepare_app_cleanup');clean=compact(cleanup)
    require(cleanup,'let Ok(info)=sex_pdx::app_lifecycle::info(catalog.slot) else {continue;};','actual terminal proof query')
    require(cleanup,'if info.target_pd!=catalog.target_pd || info.task_id!=catalog.task_id || info.required_mask & sex_pdx::app_lifecycle::ROLE_DISPLAY==0 {continue;}','terminal proof tuple and role')
    prepare='lifecycle.prepare(index,info.target_pd,info.task_id,&mutSURFACES,&mutrefs)'
    retire='BITMAP_STORES.retire(info.target_pd,info.task_id);'
    if clean.index(prepare)>=clean.index(retire):raise ValueError('bitmap retirement precedes exact prepared proof')
    require(cleanup,retire,'proof task retirement')
    sample=function(source,'bitmap_color')
    require(sample,'!surf.active || !(unsafe { scene.bitmaps.matches_surface(surf.owner_pd,surf.surface_id) })','internal bound sampling association')
    require(sample,'scene.bitmaps.sample(surf.owner_pd,surf.surface_id,x,y)','bound sample inputs')
    if 'BITMAP_STORES' in sample:raise ValueError('pixel shader reads live bitmap stores')
    return function(source,'bitmap_binding_live')


def once(source,old,new):
    if source.count(old)!=1:raise ValueError('missing/ambiguous exact mutation site: '+old)
    return source.replace(old,new,1)


def metadata_fixture(root,tests,binding):
    replaced=0
    def path(match):
        nonlocal replaced
        original=Path(match[1]);name=match[3]
        if name=='bitmap_binding':target=binding;replaced+=1
        elif name in ('app_catalog','app_lifecycle'):target=root/'crates/sex-pdx/src'/(name+'.rs')
        else:target=(tests.parent/original).resolve(strict=True)
        return '#[path='+json.dumps(str(target))+']'+match[2]
    value=re.sub(r'#\[path\s*=\s*"([^"\n]+)"\s*\](\s*(?:pub\s+)?mod\s+(\w+)\s*;)',path,tests.read_text())
    if replaced!=1:raise ValueError('metadata fixture must include one actual Binding')
    return value


def compile_run(out,name,fixture,exact=None,reject=False):
    source=out/(name+'.rs');source.write_text(fixture);binary=out/name
    result=subprocess.run(['rustc','--edition=2021','--test',str(source),'-o',str(binary)],capture_output=True,text=True,timeout=30)
    (out/(name+'.compile.log')).write_text(result.stdout+result.stderr)
    if result.returncode:raise ValueError(name+': actual/mutant must compile; see '+str(out/(name+'.compile.log')))
    args=[str(binary),'--test-threads=1']
    if exact:args+=['--exact',exact]
    result=subprocess.run(args,capture_output=True,text=True,timeout=30)
    output=result.stdout+result.stderr;(out/(name+'.log')).write_text(output)
    if result.returncode!=(101 if reject else 0):raise ValueError(name+': unexpected test result')
    if exact and (f'test {exact} ... '+('FAILED'if reject else'ok')not in output or 'running 1 test'not in output):raise ValueError(name+': exact control did not execute')
    return output


def live_fixture(root,binding,helper):
    header='''#![allow(dead_code)]
extern crate self as sex_pdx;
'''
    for name,path in [('app_catalog',root/'crates/sex-pdx/src/app_catalog.rs'),
                      ('app_lifecycle',root/'crates/sex-pdx/src/app_lifecycle.rs'),
                      ('bitmap_binding',binding),('surface_lifecycle',root/'servers/sexdisplay/src/surface_lifecycle.rs')]:
        header+='#[path='+json.dumps(str(path))+']pub mod '+name+';\n'
    return header+helper+r'''
fn held()->bitmap_binding::Binding {
    bitmap_binding::Binding::from_catalog(40,app_lifecycle::CatalogInfo{
        target_pd:15,task_id:150,surface_id:300,coordinator_pd:3}).unwrap()
}
fn row()->surface_lifecycle::Catalog {
    surface_lifecycle::Catalog{slot:40,target_pd:15,task_id:150,surface_id:300,coordinator_pd:3}
}
#[test]fn actual_live_helper_accepts_exact_cached_unequal_task() {
    let mut lifecycle=surface_lifecycle::Lifecycle::new();assert!(lifecycle.register(row()));
    assert!(bitmap_binding_live(&lifecycle,held()));assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Live));
}
#[test]fn actual_live_helper_rejects_missing_or_any_wrong_cached_tuple() {
    assert!(!bitmap_binding_live(&surface_lifecycle::Lifecycle::new(),held()));
    let original=row();
    for changed in [surface_lifecycle::Catalog{slot:41,..original},
                    surface_lifecycle::Catalog{target_pd:16,..original},
                    surface_lifecycle::Catalog{task_id:15,..original},
                    surface_lifecycle::Catalog{task_id:151,..original},
                    surface_lifecycle::Catalog{surface_id:301,..original},
                    surface_lifecycle::Catalog{coordinator_pd:7,..original}] {
        let mut lifecycle=surface_lifecycle::Lifecycle::new();assert!(lifecycle.register(changed));
        assert!(!bitmap_binding_live(&lifecycle,held()),"mismatched cache accepted: {changed:?}");
    }
}
#[test]fn actual_live_helper_refuses_prepared_committed_and_acknowledged() {
    let mut lifecycle=surface_lifecycle::Lifecycle::new();assert!(lifecycle.register(row()));
    let mut surfaces=[];let mut refs=surface_lifecycle::SceneRefs::default();
    assert_eq!(lifecycle.prepare(0,15,150,&mut surfaces,&mut refs),Some(0));
    assert_eq!(lifecycle.phase(0),Some(surface_lifecycle::Phase::Prepared));assert!(!bitmap_binding_live(&lifecycle,held()));
    assert!(lifecycle.scene_committed(0));assert!(!bitmap_binding_live(&lifecycle,held()));
    assert!(lifecycle.acknowledge(0,15,150));assert!(!bitmap_binding_live(&lifecycle,held()));
}
'''



def text_fixture(root,binding,main_source):
    header='#![allow(dead_code,static_mut_refs)]\nextern crate self as sex_pdx;\n'
    for name,path in [('app_catalog',root/'crates/sex-pdx/src/app_catalog.rs'),
                      ('app_lifecycle',root/'crates/sex-pdx/src/app_lifecycle.rs'),
                      ('bitmap_batch',root/'crates/sex-pdx/src/bitmap_batch.rs'),
                      ('text_batch',root/'crates/sex-pdx/src/text_batch.rs'),
                      ('text_encoding',root/'crates/sex-pdx/src/text_encoding.rs'),
                      ('bitmap_binding',binding),
                      *[(name,root/'servers/sexdisplay/src'/(name+'.rs')) for name in
                        ('bitmap_transaction','bitmap_stores','surface_lifecycle','text_transaction')]]:
        header+='#[path='+json.dumps(str(path))+']pub mod '+name+';\n'
    header+='static mut BITMAP_STORES:bitmap_stores::Stores<{app_catalog::count(app_catalog::App::Browser)}>=bitmap_stores::Stores::UNBOUND;\n'
    header+=function(main_source,'bitmap_binding_live')+'\n'+function(main_source,'browser_text_binding_live')
    return header+r'''
fn setup(target:u32,task:u32)->surface_lifecycle::Lifecycle {
    unsafe { BITMAP_STORES=bitmap_stores::Stores::UNBOUND; }
    let row=app_catalog::INCARNATIONS.iter().find(|r|r.target==target).unwrap();
    let c=app_lifecycle::CatalogInfo{target_pd:target,task_id:task,surface_id:row.surface,coordinator_pd:3};
    let binding=bitmap_binding::Binding::from_catalog(row.lifecycle_slot,c).unwrap();
    assert!(unsafe { BITMAP_STORES.initialize_once(binding) });
    let mut l=surface_lifecycle::Lifecycle::new();
    assert!(l.register(surface_lifecycle::Catalog{slot:row.lifecycle_slot,target_pd:target,task_id:task,surface_id:row.surface,coordinator_pd:3}));l
}
// The actual Main guard and Stage execute here; wiring checks independently
// require this guard before any actual typed-text Stage mutation in Main.
fn deliver(l:&surface_lifecycle::Lifecycle,s:&mut text_transaction::Stage,pd:u32,task:u32,sid:u64,r:text_batch::Request)->bool {
    if !browser_text_binding_live(l,pd,task,sid){return false;}
    s.receive(pd,task,sid,r).is_some()
}
fn begin()->text_batch::Request {text_batch::Request::Begin{sequence:1,len:2,color:0xffcdd6f4}}
#[test]fn browser_text_accepts_actual_unequal_tasks_and_exact_commit() {
    for (pd,task,sid) in [(15,150,300),(30,300,343),(31,310,344)] {
        let l=setup(pd,task);let mut s=text_transaction::Stage::EMPTY;
        assert!(browser_text_binding_live(&l,pd,task,sid));
        assert!(!deliver(&l,&mut s,pd,task,sid,begin()));
        assert!(!deliver(&l,&mut s,pd,task,sid,text_batch::Request::Chunk{sequence:1,offset:0,count:2,bytes:*b"ok\0\0\0\0\0\0"}));
        assert!(deliver(&l,&mut s,pd,task,sid,text_batch::Request::Commit{sequence:1,hash:text_batch::hash(b"ok")}));
        assert!(s.needs_publication());s.poll_reply(|_,_|panic!("unreleased completion"));assert!(s.release_after_publication());
        let mut count=0;s.poll_reply(|owner,word|{assert_eq!(owner,pd);assert_eq!(text_batch::Applied::decode(1,word).unwrap(),text_batch::Applied{surface:sid as u16,sequence:1});count+=1;0});assert_eq!(count,1);
    }
}
fn no_effect(l:&surface_lifecycle::Lifecycle,pd:u32,task:u32,sid:u64) {
    let mut s=text_transaction::Stage::EMPTY;let before=s.diagnostic();
    assert!(!browser_text_binding_live(l,pd,task,sid));
    assert!(!deliver(l,&mut s,pd,task,sid,begin()));assert_eq!(s.diagnostic(),before);
    s.poll_reply(|_,_|panic!("denial exposed receipt"));
}
#[test]fn browser_text_wrong_task_has_no_stage_effect() {
    let l=setup(15,150);for task in [0,15,151,300]{no_effect(&l,15,task,300);}
}
#[test]fn browser_text_missing_foreign_surface_or_owner_has_no_effect() {
    let l=setup(15,150);
    for (pd,task,sid) in [(15,150,0),(15,150,343),(15,150,344),(30,300,343),(30,300,300),(31,310,300)] {no_effect(&l,pd,task,sid);}
}
#[test]fn browser_text_stale_held_tuple_and_terminal_have_no_effect() {
    let mut l=setup(15,150);let mut surfaces=[];let mut refs=surface_lifecycle::SceneRefs::default();
    assert_eq!(l.prepare(0,15,150,&mut surfaces,&mut refs),Some(0));no_effect(&l,15,150,300);
    assert!(l.scene_committed(0));no_effect(&l,15,150,300);
    assert!(l.acknowledge(0,15,150));no_effect(&l,15,150,300);
    for task in [15,151] {let mut stale=surface_lifecycle::Lifecycle::new();assert!(stale.register(surface_lifecycle::Catalog{slot:40,target_pd:15,task_id:task,surface_id:300,coordinator_pd:3}));no_effect(&stale,15,150,300);}
    no_effect(&surface_lifecycle::Lifecycle::new(),15,150,300);
}
#[test]fn browser_text_leaves_other_producers_policy_unchanged() {
    let l=setup(15,150);for pd in [3,9,12,18]{assert!(browser_text_binding_live(&l,pd,0,200));}
}
'''


def check(root,out,main,binding,tests):
    main_source=main.read_text();helper=wiring(main_source)
    # Verify source-wiring controls independently of mutation construction.
    controls=(('binding.admits(msg.caller_pd,msg.caller_task,msg.arg0)','binding.admits(msg.caller_pd,msg.caller_pd,msg.arg0)'),
              ('BITMAP_STORES.retire(info.target_pd,info.task_id);','BITMAP_STORES.retire(info.target_pd,info.target_pd);'),
              ('scene.bitmaps.matches_surface(surf.owner_pd, surf.surface_id)','scene.bitmaps.matches_surface(15, 300)'),
              ('BITMAP_STORES.binding_for(msg.caller_pd,msg.arg0)','BITMAP_STORES.binding_for(15,300)'),
              ('assert_eq!(unsafe { BITMAP_STORES.initialized_count() },sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser),"missing display Browser bitmap binding");',''),
              ('BITMAP_STORES.initialize_once(binding)','BITMAP_STORES.initialize_once(binding) || BITMAP_STORES.initialize_once(binding)'),
              ('BITMAP_STORES.sequence(binding.target(),binding.surface())','BITMAP_STORES.sequence(15,300)'),
              ('if !browser_text_binding_live(&lifecycle, msg.caller_pd, msg.caller_task, msg.arg0) { continue; }',''),
              ('context.owner(),context.task(),context.surface(),context.sequence(),len,','context.owner(),context.owner(),context.surface(),context.sequence(),len,'),
              ('sex_pdx::text_batch::hash(&slot.text_buf[..len]),changed as u8','0,changed as u8'),
              ('if pending.context()!=context{continue;}',''),
              ('if published.plan.is_full(){unsafe{BITMAP_STORES.release_after_publication()}}else{0}','unsafe{BITMAP_STORES.release_after_publication()}'),
              ('if unsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return;}',''),
              ('let text_projections=scene.text_projections;','let text_projections=[None;MAX_SURFACES];'),
              ('pending_paint.complete(published.is_some());','pending_paint.complete(published.is_some());let _=sex_pdx::pdx_try_listen_raw(0);'),
              ('if !published.plan.covers_body(required){continue;}',''),
              ('if !published.plan.covers_body(required){continue;}','if !true{continue;}'))
    for old,new in controls:
        changed=once(main_source,old,new)
        try:wiring(changed)
        except (ValueError,IndexError):pass
        else:raise ValueError('unbound main-wiring control survived: '+old)
    output=compile_run(out,'metadata',metadata_fixture(root,tests,binding))
    if '5 passed; 0 failed'not in output:raise ValueError('expected five actual metadata tests')
    output=compile_run(out,'actual-live-helper',live_fixture(root,binding,helper))
    if '3 passed; 0 failed'not in output:raise ValueError('expected three actual live-helper tests')
    actual=binding.read_text()
    for name,old,new,control in (
        ('task-inferred-from-pd','task: catalog.task_id,','task: catalog.target_pd,',INFERENCE_TEST),
        ('caller-task-guard-removed',' && caller_task == self.task','',SENDER_TEST),
    ):
        mutant=out/(name+'-binding.rs');mutant.write_text(once(actual,old,new))
        compile_run(out,name,metadata_fixture(root,tests,mutant),exact=control,reject=True)
    text_output=compile_run(out,'actual-browser-text-guard',text_fixture(root,binding,main_source))
    if '5 passed; 0 failed'not in text_output:raise ValueError('expected five actual Browser text guard tests')
    text_mutants=(
        ('text-wrong-task','binding.admits(owner, task, sid)','binding.admits(owner, binding.task(), sid)','browser_text_wrong_task_has_no_stage_effect'),
        ('text-stale-held','&& bitmap_binding_live(lifecycle, binding)','&& true','browser_text_stale_held_tuple_and_terminal_have_no_effect'),
        ('text-missing-binding','BITMAP_STORES.binding_for(owner, sid) }) else { return false; }','BITMAP_STORES.binding_for(owner, sid) }) else { return true; }','browser_text_missing_foreign_surface_or_owner_has_no_effect'),
    )
    for name,old,new,control in text_mutants:
        changed=once(main_source,old,new)
        compile_run(out,name,text_fixture(root,binding,changed),exact=control,reject=True)
    result=dict(ok=True,metadata_tests=5,actual_live_helper_tests=3,compiled_binding_mutants_rejected=2,actual_browser_text_guard_tests=5,compiled_text_guard_mutants_rejected=3,
                wiring_controls_rejected=len(controls),text_commit_wiring='sealed_candidate_owned_scene_covered_publication_then_canonical_commit_and_exact_release_bitmap_lifecycle_Full_only',scope='actual_binding_Lifecycle_helper_host_and_source_wiring_not_runtime_BOOT_grants_or_guest_pixels')
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=ROOT)
    parser.add_argument('--out',type=Path,help='new evidence directory')
    parser.add_argument('--main',type=Path,help='explicit draft Display main override')
    parser.add_argument('--binding',type=Path,help='explicit draft Binding override')
    parser.add_argument('--tests',type=Path,help='explicit draft metadata tests override')
    args=parser.parse_args();root=args.root.resolve(strict=True)
    source=(args.main or root/'servers/sexdisplay/src/main.rs').resolve(strict=True)
    binding=(args.binding or root/'servers/sexdisplay/src/bitmap_binding.rs').resolve(strict=True)
    tests=(args.tests or root/'tests/display_bitmap_binding.rs').resolve(strict=True)
    if args.out:
        args.out.mkdir(parents=True,exist_ok=False);result=check(root,args.out.resolve(),source,binding,tests)
    else:
        with tempfile.TemporaryDirectory(prefix='sexos-display-bitmap-binding-')as directory:
            result=check(root,Path(directory),source,binding,tests)
    print('[display.bitmap.binding] PASS '+json.dumps(result,sort_keys=True))


if __name__=='__main__':main()
