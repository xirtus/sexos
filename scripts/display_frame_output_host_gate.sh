#!/usr/bin/env bash
set -euo pipefail
OUTPUT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_GATE_PATH="${BASH_SOURCE[0]}" python3 - "$OUTPUT_ROOT" "$@" <<'PY'
import argparse,hashlib,json,os,re,subprocess,sys,tempfile
from pathlib import Path
p=argparse.ArgumentParser(description='Actual Display private-frame pipeline host proof; no native scanout, authority, atomic hardware flip or epoch closure claim')
p.add_argument('--root',type=Path,default=Path(sys.argv[1]));p.add_argument('--out',type=Path);p.add_argument('--fixture',type=Path)
a=p.parse_args(sys.argv[2:]);root=a.root.resolve(strict=True);managed=None
if a.out:a.out.mkdir(parents=True,exist_ok=False);out=a.out.resolve()
else:managed=tempfile.TemporaryDirectory(prefix='sexos-display-frame-output-');out=Path(managed.name)
main_path=root/'servers/sexdisplay/src/main.rs';main=main_path.read_text();sdk_path=root/'crates/sex-pdx/src/lib.rs';sdk=sdk_path.read_text()
fixture=(a.fixture or root/'tests/display_frame_pipeline.rs').resolve(strict=True)
def block(text,needle):
    assert text.count(needle)==1,'missing/ambiguous extraction: '+needle
    start=text.index(needle);opening=text.index('{',start);depth=0
    for i in range(opening,len(text)):
        depth+=(text[i]=='{')-(text[i]=='}')
        if depth==0:return text[start:i+1]
    raise AssertionError('unterminated '+needle)
def once(text,old,new):
    assert text.count(old)==1,'mutation target missing/ambiguous: '+old
    return text.replace(old,new,1)
def fn(text,name):return block(text,'fn '+name+'(')
def compact(s):return re.sub(r'\s+','',re.sub(r'//[^\n]*','',s))
def source_contract(text):
    prepared=fn(text,'render_prepared');c=compact(prepared)
    assert c.count('SceneLease::capture(')==1 and c.count('store.prepare(')==1 and c.count('preparation.commit(')==1 and c.count('preparation.target()')==1,'one preparation/capture/publication'
    assert '(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()'in c
    assert c.index('(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()')<c.index('FRAME_OUTPUT.as_mut()'),'nested guard precedes exclusive Store borrow'
    assert c.index('RENDER_FRAME_COUNTER.fetch_add(')<c.index('SceneLease::capture('),'captured Full frame version'
    assert 'letprepared={lettarget=preparation.target();raster_batch(&target,scene,plan)};'in c
    assert c.index('preparation.commit(')<c.index('record_cursor_presented();')<c.index('report_app_presented();')<c.index('civil_clock_drawn(')
    assert 'ifplan.clock_dirty(){civil_clock_drawn(&scene.bar,w,h,plan.label());}'in c
    assert not any(x in c for x in ['drop(lease)','mem::forget','ManuallyDrop'])
    assert 'Some(PublishedFrame{plan,present_bits,canon_ss,text_projections})'in c
    assert 'lettext_projections=scene.text_projections;'in c and c.index('preparation.commit(')<c.index('lettext_projections=scene.text_projections;'),'receipt copies captured metadata only after successful output'
    for name in ['raster_full','raster_strip','raster_area','raster_damage']:
        b=fn(text,name)
        assert "target: &frame_output::Target<'_>"in b and 'scene: &scene::SceneSnapshot'in b and 'let fb=target.pointer();'in b
        assert not re.search(r'FB_PTR|HIGH_HALF_BASE|SceneLease::capture|valid_scanout\(',b)
        assert not any(x+'('in b for x in ['report_app_presented','record_cursor_presented','civil_clock_drawn','repaint_end','draw_cursor_z_top','draw_launcher_panel'])
    batch=compact(fn(text,'raster_batch'));assert 'if!body||plan.strip&&!raster_strip(target,scene){returnfalse;}'in batch
    assert batch.count('restore_cursor_under(')==batch.count('draw_cursor_z_top(')==1 and 'SceneLease::capture('not in batch
    assert batch.index('restore_cursor_under(')<batch.index('letbody=')<batch.index('raster_strip(')<batch.index('draw_cursor_z_top(')
    publish=compact(fn(text,'publish_frame'));assert 'asm!("sfence"'in publish;assert publish.index('if!valid_scanout')<publish.index('core::ptr::write_volatile(')<publish.index('asm!("sfence"')
    assert 'w.checked_mul(h)!=Some(pixels.len())'in publish
    startup=fn(text,'_start');drain=block(startup,'for _drain_i in 0..DRAIN_MAX {')
    assert 'render('not in drain and 'render_prepared('not in drain,'no in-drain device publication'
    assert startup.count('render_prepared(')==1 and startup.index(drain)+len(drain)<startup.index('render_prepared(')
    service=startup.rfind('loop {',0,startup.index('while let Some(stream)=uiir.as_mut()'));assert service>0
    assert startup.index('let mut pending_paint=')<service and 'Dirty::new('not in startup[service:],'pending state must survive service iterations'
    assert startup.count('let mut pending_paint=paint_batch::Dirty::new(')==1 and startup.index('let mut pending_paint=')<startup.index('for _drain_i')
    assert 'pending_paint.complete(published.is_some());'in startup and startup.index('render_prepared(')<startup.index('pending_paint.complete(')<startup.index('finish_display_transactions(&mut lifecycle,published.as_ref())')
    assert 'lettransaction_full_render=unsafe{BITMAP_STORES.needs_publication()}||lifecycle.needs_presentation()'in compact(startup),'bitmap/lifecycle force Full'
    assert 'iftyped_completion_needs_publication(){join_pending_text(&mutpending_paint,w,h);}'in compact(startup),'captured text work derives before plan'
    primary=block(startup,'0x11 => {');assert 'needs_full_redraw=true;'in compact(primary) and 'primary_receipt_pending=true;'in compact(primary)
    assert 'letframe_backing=sex_pdx::display_frame_backing(frame_layout)'in compact(startup) and startup.index('sex_pdx::display_frame_backing(frame_layout)')<startup.index('FRAME_OUTPUT=Some(')<startup.index('render(FB_PTR')
    assert 'assert!(FRAME_OUTPUT.is_none(),"DISPLAY_FRAME_ALREADY_INITIALIZED")'in startup
    prepare=compact(fn(text,'prepare_app_cleanup'));assert 'ifletSome(output)=FRAME_OUTPUT.as_mut(){output.scrub();}'in prepare
    store=(root/'servers/sexdisplay/src/frame_output.rs').read_text();assert 'self.erase(0);self.erase(1);'in fn(store,'scrub')
    finish=compact(fn(text,'finish_display_transactions'));assert 'render('not in finish and 'render_prepared('not in finish and 'APP_PRESENT_BITS'not in finish
    assert 'lifecycle.pixels_presented(published.present_bits)'in finish
    assert 'ifpublished.plan.is_full(){lifecycle.pixels_presented(published.present_bits);'in finish and finish.index('lifecycle.pixels_presented(published.present_bits)')<finish.index('lifecycle.scene_committed(')<finish.index('sex_pdx::app_lifecycle::ack(')
    assert 'ifunsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return;}'in finish
    assert 'commit_published_text(published)'in finish and 'letbitmap=ifpublished.plan.is_full(){unsafe{BITMAP_STORES.release_after_publication()}}else{0};'in finish,'covered text and Full-only bitmap under scene fence'
    assert 'if!published.plan.covers_body(required){continue;}'in compact(fn(text,'commit_published_text'))
    assert 'if!settle_only{unsafe{APP_PRESENT_BITS=0;PREPARED_CURSOR=None;}}'in c and 'if!settle_only{record_cursor_presented();report_app_presented();'in c,'Settle withholds cursor/app effects'
    assert batch.index('ifplan.settle_only(){returntrue;}')<batch.index('restore_cursor_under('),'Settle identity before overlays'
    # All physical FB_PTR conversions must be the startup wrapper, single batch
    # publisher, or the separate offscreen proof's unused pointer parameter.
    for line in text.splitlines():
        if re.search(r'FB_PTR\s+as\s+\*(?:mut|const)\s+u32',line):assert any(x in line for x in ['render(','render_prepared(','top_strip_render_proof(']),line
    return ['one-prepare-capture-publication','nested-guard-before-exclusive-store','immutable-scene-raster-primitives','one-cursor-tail-after-both-passes','post-copy-fence-and-effects','persistent-post-drain-plan','no-primary-in-drain-publication','held-acquisition-before-startup-render','scrub-before-receipt-driven-cleanup']
wiring=source_contract(main)
source_controls=[('missing-cleanup-scrub','if let Some(output)=FRAME_OUTPUT.as_mut(){output.scrub();}',''),('missing-nested-guard','if unsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return None;}',''),('clear-failed-batch','pending_paint.complete(published.is_some());','pending_paint.complete(true);'),('missing-device-fence','asm!("sfence",options(nostack,preserves_flags))','asm!("nop",options(nostack,preserves_flags))'),('global-bits-rejoin','lifecycle.pixels_presented(published.present_bits);','lifecycle.pixels_presented(unsafe{APP_PRESENT_BITS});'),('reset-pending-per-drain','for _drain_i in 0..DRAIN_MAX {','pending_paint=paint_batch::Dirty::new(geometry.width as usize,geometry.height as usize);for _drain_i in 0..DRAIN_MAX {'),('missing-typed-coverage-join','if typed_completion_needs_publication(){join_pending_text(&mut pending_paint,w,h);}','')]
for name,old,new in source_controls:
    altered=once(main,old,new)
    try:source_contract(altered)
    except AssertionError:pass
    else:raise AssertionError('source control accepted: '+name)
def item(name):
    found=re.findall(r'^(?:pub )?(?:static(?: mut)?|const) '+re.escape(name)+r'\b.*?;[ \t]*(?://[^\n]*)?$',main,re.M|re.S)
    assert len(found)==1,name
    return found[0]
def derived(needle):
    prefix=main[:main.index(needle)];match=re.search(r'(#\[derive\([^\n]+\)\])\s*$',prefix);assert match,'missing actual derive '+needle
    return match[1]+'\n'+block(main,needle)
parts=[item(n)for n in ['CURSOR_ARROW_W','CURSOR_ARROW_H','CURSOR_UNDER_SAVED','CURSOR_UNDER_X','CURSOR_UNDER_Y','CURSOR_UNDER_VALID','PREPARED_CURSOR','RENDER_SCENE','SURFACES','FRAME_GROUPS','BITMAP_STORES','TEXT_STAGES','DISPLAY_TOKENS','FOCUSED_SURFACE_ID']]
parts+=[derived('struct PreparedCursor{'),block(main,'struct PublishedFrame{'),block(main,'struct OutputBackup{'),block(main,'impl OutputBackup{'),block(main,'impl Drop for OutputBackup{'),block(main,'struct SceneLease {'),block(main,'impl SceneLease {'),block(main,'impl Drop for SceneLease {')]
parts+=[fn(main,n)for n in ['valid_scanout','raster_batch','render_prepared','render','typed_completion_needs_publication','join_pending_text']]
parts+=[fn(main,'publish_frame').replace('fn publish_frame(', 'fn actual_publish_frame(',1)]
# Exact Main plan/render/completion statements; only the host function boundary
# supplies the same local variables. No copied pending-work policy in the test.
start=main.index('let plan=pending_paint.plan(w,h);');end=main.index('pending_paint.complete(published.is_some());',start)+len('pending_paint.complete(published.is_some());')
parts+=['fn actual_attempt(pending_paint:&mut Dirty,fb_live:bool,w:usize,h:usize,mut bar:&mut SilkBar)->Option<PublishedFrame>{\n'+main[start:end]+'\npublished\n}']
production='\n\n'.join(parts)+'\n';production_path=out/'actual-main-pipeline.rs';production_path.write_text(production)
parent=[item(n)for n in ['MAX_FB_W','MAX_FB_H','BAR_BG_H','FOCUS_SURFACE_COLOR','FRAME_RIM_COLOR','FRAME_TOP_BAR_COLOR','TAB_ACTIVE_COLOR','TAB_INACTIVE_COLOR','FRAME_LIGHT_CLOSE_COLOR','FRAME_LIGHT_MINIMIZE_COLOR','FRAME_LIGHT_ZOOM_COLOR','DEFAULT_RENDER_TOKENS','SURFACE_CHROME_TOP_BAR','FRAME_TOP_BAR_HEIGHT_PX']]
parent+=['#[derive(Clone, Copy)]\n#[repr(C)]\n'+block(main,'struct RenderTokensV1 {')]
parent+=['use surface_lifecycle::{TEXT_BUF_CAP,TEXT_MAX_ROWS,TEXT_LEGACY_COLS};']
parent+=[fn(main,n)for n in ['lerp_color_xrgb','vertical_gradient_xrgb','bg','surface_text_scale','recompute_text_lines','apply_pending_text','text_surface_damage','text_projection_requirement','changed_text_rows']]
parent+=[fn(main,'scene_bar_blur_rows').replace('fn scene_bar_blur_rows(', 'fn actual_scene_bar_blur_rows(',1)]
parent_path=out/'actual-scene-parent.rs';parent_path.write_text('\n\n'.join(parent))
acquire_path=out/'actual-sdk-acquisition.rs';acquire_path.write_text('pub '+fn(sdk,'display_frame_backing')+'\n');(out/'main-source.rs').write_text(main)
def normalize(text):
    return re.sub(r'#\[path\s*=\s*"([^\"]+)"\]',lambda m: '#[path='+json.dumps(str((root/'tests'/m[1]).resolve(strict=True)))+']',text)
def invoke(cmd,name,env=None):
    r=subprocess.run(cmd,text=True,capture_output=True,env=env,timeout=120)
    (out/(name+'.log')).write_text('$ '+' '.join(map(str,cmd))+'\n'+r.stdout+r.stderr)
    return r
inputs=[Path(os.environ['OUTPUT_GATE_PATH']).resolve(strict=True),main_path,sdk_path,fixture,root/'tests/display_frames.rs',root/'tests/display_frame_output.rs',root/'kernel/src/memory/va_allocator.rs',root/'servers/sexdisplay/src/frame_output.rs',root/'servers/sexdisplay/src/damage.rs',root/'crates/sex-pdx/src/display_frames.rs',root/'crates/sex-pdx/src/memory_region.rs']+sorted((root/'crates/silkbar-model/src').rglob('*.rs'))
inputs += [root/n for n in ['servers/sexdisplay/src/text_transaction.rs','crates/sex-pdx/src/text_batch.rs','crates/sex-pdx/src/text_encoding.rs']]
inputs += [root/n for n in ['tests/display_paint_batch.rs','servers/sexdisplay/src/paint_batch.rs','servers/sexdisplay/src/scene.rs','servers/sexdisplay/src/surface_lifecycle.rs','servers/sexdisplay/src/surface_groups.rs','servers/sexdisplay/src/bitmap_binding.rs','servers/sexdisplay/src/bitmap_transaction.rs','servers/sexdisplay/src/bitmap_stores.rs','crates/sex-pdx/src/app_catalog.rs','crates/sex-pdx/src/app_lifecycle.rs','crates/sex-pdx/src/bitmap_batch.rs','crates/sex-pdx/src/desktop_style.rs','crates/sex-pdx/src/spindle_view.rs']]
input_hashes={str(v):hashlib.sha256(v.read_bytes()).hexdigest()for v in inputs}
lib=out/'libsilkbar_model.rlib';r=invoke(['rustc','--edition=2021','--crate-name','silkbar_model','--crate-type','rlib',str(root/'crates/silkbar-model/src/lib.rs'),'-o',str(lib)],'model.compile');assert r.returncode==0,r.stderr
def compile_test(name,text,pipeline=production_path,case=None,expect_failure=False,count=None,paint=None):
    path=out/(name+'.rs');normalized=normalize(text)
    if paint:normalized=normalized.replace(json.dumps(str(root/'servers/sexdisplay/src/paint_batch.rs')),json.dumps(str(paint)))
    path.write_text(normalized);binary=out/name
    env=dict(os.environ,DISPLAY_FRAME_MAIN=str(pipeline),DISPLAY_FRAME_ACQUIRE=str(acquire_path),DISPLAY_FRAME_SCENE_PARENT=str(parent_path))
    r=invoke(['rustc','--edition=2021','--extern','silkbar_model='+str(lib),'--test',str(path),'-o',str(binary)],name+'.compile',env)
    assert r.returncode==0,name+' must compile: '+r.stderr
    args=[str(binary),'--test-threads=1','--nocapture']+(['--exact',case]if case else [])
    r=invoke(args,name+'.run',env)
    if expect_failure:
        assert r.returncode==101 and 'running 1 test' in r.stdout and re.search(r'test '+re.escape(case)+r' \.\.\. FAILED',r.stdout),name+' did not reject via exact runtime case: '+r.stdout+r.stderr
    else:
        assert r.returncode==0,name+': '+r.stdout+r.stderr
        match=re.search(r'test result: ok\. (\d+) passed; 0 failed;',r.stdout);assert match,name
        if count is not None:assert int(match[1])==count,(name,match[1],count)
        return int(match[1])
counts={}
for name,file,expected in [('layout','tests/display_frames.rs',7),('store','tests/display_frame_output.rs',9),('va','kernel/src/memory/va_allocator.rs',6),('paint_batch','tests/display_paint_batch.rs',13)]:
    counts[name]=compile_test(name,(root/file).read_text(),count=expected)
fixture_text=fixture.read_text();counts['pipeline']=compile_test('pipeline',fixture_text,count=13)
# Unforgeable ownership discipline is a type property, not a native authority
# test. Require the intended diagnostic, never arbitrary compiler failure.
control_prefix='''#![allow(dead_code)]\n#[path="../crates/sex-pdx/src/memory_region.rs"]mod memory_region;\npub use memory_region::MemoryRegionInfo;\npub const ERR_CAP_INVALID:u64=0xFFFF_FFFF_FFFF_FFFC;\n#[path="../crates/sex-pdx/src/display_frames.rs"]mod display_frames;\n'''
for trait in ['Copy','Clone','Send','Sync']:
    path=out/('forbid-'+trait+'.rs');path.write_text(normalize(control_prefix+'fn need<T:'+trait+'>(){}\nfn main(){need::<display_frames::FrameBacking>();}\n'))
    r=invoke(['rustc','--edition=2021','--crate-name','forbid_'+trait.lower(),str(path),'-o',str(out/('forbid-'+trait))],'forbid-'+trait+'.compile')
    assert r.returncode!=0 and 'error[E0277]' in r.stderr and trait in r.stderr and 'FrameBacking' in r.stderr,'unexpected trait diagnostic: '+r.stderr
controls=[
 ('early-presentation','if !prepared{drop(preparation);backup.restore();return None;}','report_app_presented();\n    if !prepared{drop(preparation);backup.restore();return None;}','actual_combined_batches_publish_once_with_one_immutable_scene'),
 ('stale-unchanged-rows','let Some(first)=(0..w).find(|&x|row[x]!=old[x])else{continue};','let Some(first)=(0..w).find(|&x|row[x]!=old[x])else{continue};let first=first.max(1);','actual_combined_batches_publish_once_with_one_immutable_scene'),
 ('skip-cpu-copy','for(index,pixel)in row[first..end].iter().enumerate(){unsafe{core::ptr::write_volatile(out.add(first+index),*pixel);}}','/* omitted CPU copy */','actual_combined_batches_publish_once_with_one_immutable_scene'),
 ('omit-raster-rollback','if !prepared{drop(preparation);backup.restore();return None;}','if !prepared{drop(preparation);return None;}','every_body_failure_restores_bookkeeping_and_retains_front'),
 ('recapture','let scene=lease.view();','drop(lease);let lease=SceneLease::capture(w,h,bar).unwrap();let scene=lease.view();','actual_combined_batches_publish_once_with_one_immutable_scene'),
 ('drop-strip-pass','if !body||plan.strip&&!raster_strip(target,scene){return false;}','if !body{return false;}','actual_combined_batches_publish_once_with_one_immutable_scene'),
 ('clear-failed-main','pending_paint.complete(published.is_some());','pending_paint.complete(true);','failed_second_pass_preserves_dirty_and_retry_merges_exact_work'),
 ('lost-candidate-context','let text_projections=scene.text_projections;','let text_projections=[None;MAX_SURFACES];','failed_output_keeps_canonical_text_and_success_carries_owned_candidate_context'),
 ('settle-raster-effects','if plan.settle_only(){return true;}','/* no identity bypass */','pure_settle_preserves_device_underlay_and_effect_state_then_carries_no_pixels_projection'),
 ('settle-state-reset','if !settle_only{unsafe{APP_PRESENT_BITS=0;PREPARED_CURSOR=None;}}','unsafe{APP_PRESENT_BITS=0;PREPARED_CURSOR=None;}','pure_settle_preserves_device_underlay_and_effect_state_then_carries_no_pixels_projection'),
 ('settle-presentation-effects','if !settle_only{\n        record_cursor_presented();','if true{\n        record_cursor_presented();','pure_settle_preserves_device_underlay_and_effect_state_then_carries_no_pixels_projection')]
for name,old,new,case in controls:
    path=out/(name+'-actual-main.rs');path.write_text(once(production,old,new))
    compile_test(name,fixture_text,pipeline=path,case=case,expect_failure=True)
    print('[display.frame.output.mutant] rejected='+name+' exact='+case)
paint_controls=[('lost-union','damage.add((old.x,old.y,old.right-old.x,old.bottom-old.y),width,height);','let _=old;','disjoint_touching_nested_and_duplicate_damage_union_is_conservative'),('cleared-failure','if published{self.content=Content::None;self.strip=false;}','{self.content=Content::None;self.strip=false;}','failure_retains_exact_work_new_requests_merge_and_only_success_clears'),('lost-strip','self.strip=true;','self.strip=false;','all_dominance_permutations_and_duplicates_have_identical_normalized_plans')]
for name,old,new,case in paint_controls:
    path=out/(name+'-paint.rs');path.write_text(once((root/'servers/sexdisplay/src/paint_batch.rs').read_text(),old,new))
    compile_test(name,(root/'tests/display_paint_batch.rs').read_text(),paint=path,case=case,expect_failure=True)
    print('[display.paint.batch.mutant] rejected='+name+' exact='+case)
assert main_path.read_text()==main and sdk_path.read_text()==sdk,'source changed during proof; rerun'
assert all(hashlib.sha256(v.read_bytes()).hexdigest()==input_hashes[str(v)] for v in inputs),'an actual input changed during proof; rerun'
proof=dict(ok=True,cases=counts,compiled_runtime_mutants=len(controls),paint_batch_runtime_mutants=len(paint_controls),compiler_trait_controls=4,source_controls=len(source_controls),wiring=wiring,scope='actual SDK acquisition, Layout, Store, VA, PaintBatch, SceneSnapshot/Lease, Main raster_batch/output and extracted plan-completion sequence; host-owned exclusive memory and finite raster/scanout/effect doubles; no native framebuffer/BOOT authority, physical atomicity, full epoch or whole-system replay proof',sha256=input_hashes)
(out/'result.json').write_text(json.dumps(proof,indent=2)+'\n');print('[display.frame.output.host] PASS '+json.dumps(proof,sort_keys=True))
if managed:managed.cleanup()
PY
