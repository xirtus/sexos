#!/usr/bin/env bash
set -euo pipefail
SCENE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 - "$SCENE_ROOT" "$@" <<'PY'
import argparse,hashlib,json,os,re,subprocess,sys,tempfile
from pathlib import Path
p=argparse.ArgumentParser(description='Actual owned SceneSnapshot/Main lease and cleanup host proof; no native pixel or epoch closure claim')
p.add_argument('--root',type=Path,default=Path(sys.argv[1]));p.add_argument('--out',type=Path);p.add_argument('--fixture',type=Path);p.add_argument('--scene-path',type=Path);p.add_argument('--groups-path',type=Path)
a=p.parse_args(sys.argv[2:]);root=a.root.resolve(strict=True)
managed=None
if a.out:a.out.mkdir(parents=True,exist_ok=False);out=a.out.resolve()
else:managed=tempfile.TemporaryDirectory(prefix='sexos-display-scene-');out=Path(managed.name)
main_path=root/'servers/sexdisplay/src/main.rs';source=main_path.read_text();scene_path=(a.scene_path or root/'servers/sexdisplay/src/scene.rs').resolve(strict=True);groups_path=(a.groups_path or root/'servers/sexdisplay/src/surface_groups.rs').resolve(strict=True)
fixture_path=(a.fixture or root/'tests/display_scene.rs').resolve(strict=True)
def block(text,needle):
    assert text.count(needle)==1,'missing/ambiguous production item '+needle
    start=text.index(needle);opening=text.index('{',start);depth=1;at=opening+1
    # Selected Rust items contain no brace characters in literals; caller
    # excludes logging bodies whose formatting braces are separately balanced.
    while depth and at<len(text):
        depth+=(text[at]=='{')-(text[at]=='}');at+=1
    assert depth==0,needle
    return text[start:at]
def function(text,name):return block(text,'fn '+name+'(')
def compact(s):return re.sub(r'\s+','',re.sub(r'//[^\n]*','',s))
def wiring(s):
    prepare=compact(function(s,'prepare_app_cleanup'));finish=compact(function(s,'finish_display_transactions'));render=compact(function(s,'render'));lease=compact(block(s,'impl SceneLease {'))
    guard='ifunsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return'
    assert prepare.split('{',1)[1].startswith(guard+'false;}'),'cleanup preparation must fence before all service/state effects'
    assert finish.split('{',1)[1].startswith(guard+';}'),'cleanup completion must fence before all render/ACK/reply effects'
    assert 'render('not in finish and 'render_prepared('not in finish and 'APP_PRESENT_BITS'not in finish,'completion may not render or rejoin live global pixel bits'
    assert finish.index('published.filter(|frame|frame.plan.is_full())')<finish.index('lifecycle.pixels_presented(published.present_bits)')<finish.index('lifecycle.scene_committed(')<finish.index('sex_pdx::app_lifecycle::ack('),'full captured receipt/commit/ACK order'
    assert render=='fnrender(fb:*mutu32,w:usize,h:usize,bar:&SilkBar)->bool{render_prepared(fb,w,h,bar,Plan::full()).is_some()}','bootstrap full uses prepared publication'
    prepared=compact(function(s,'render_prepared'));batch=compact(function(s,'raster_batch'))
    assert prepared.index('ifunsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{returnNone;}')<prepared.index('FRAME_OUTPUT.as_mut()'),'reject nested render before exclusive store borrow'
    assert prepared.count('SceneLease::capture(')==1 and prepared.count('raster_batch(')==1 and prepared.count('preparation.commit(')==1,'one scene/preparation/publication per batch'
    assert prepared.index('store.prepare(')<prepared.index('SceneLease::capture(')<prepared.index('letscene=lease.view();')<prepared.index('raster_batch(&target,scene,plan)')<prepared.index('preparation.commit(|pixels|publish_frame(fb,w,h,pixels))')<prepared.index('letpresent_bits=unsafe{APP_PRESENT_BITS};')<prepared.index('record_cursor_presented();')<prepared.index('report_app_presented();')<prepared.index('civil_clock_drawn('),'one owned scene covers raster/publish/posteffects'
    assert prepared.endswith('Some(PublishedFrame{plan,present_bits,canon_ss})}') and not any(x in prepared for x in('mem::forget','ManuallyDrop','drop(lease)')),'lease stays ordinary scoped ownership through receipt construction'
    assert 'requested.is_empty()||!valid_scanout('in prepared and 'if!prepared{drop(preparation);backup.restore();returnNone;}'in prepared
    assert 'if!preparation.commit(|pixels|publish_frame(fb,w,h,pixels)){backup.restore();returnNone;}'in prepared
    assert 'SceneLease'not in batch and batch.count('restore_cursor_under(')==1 and batch.index('restore_cursor_under(')<batch.index('letbody=matchplan.content')<batch.index('plan.strip&&!raster_strip('),'one restore then body/strip from same scene'
    for name in ['draw_atlas_cards_pass','draw_atlas_frame_previews_pass','draw_launcher_panel','draw_cursor_z_top']:assert batch.count(name+'(')==1,name+' at most once per batch'
    assert batch.index('draw_launcher_panel(')<batch.index('draw_cursor_z_top('),'cursor last over launcher'
    assert 'for(index,pixel)inpixels.iter().enumerate(){unsafe{core::ptr::write_volatile(fb.add(index),*pixel);}}'in compact(function(s,'publish_frame'))
    for name in ['raster_full','raster_strip','raster_area','raster_damage']:
        body=compact(function(s,name))
        assert 'scene:&scene::SceneSnapshot' in body and not any(x in body for x in('SceneLease','draw_cursor_z_top(','draw_launcher_panel(','draw_atlas_cards_pass(','draw_atlas_frame_previews_pass(','restore_cursor_under(','civil_clock_drawn(','report_app_presented(','record_cursor_presented(','render_prepared(')),name+' primitive may only paint borrowed target/scene'
    assert 'scrub_cursor_under();ifletSome(output)=FRAME_OUTPUT.as_mut(){output.scrub();}PREPARED_CURSOR=None;'in prepare,'cleanup must remove retained raster pixels and pending cursor metadata'
    assert lease.index('if(*core::ptr::addr_of!(RENDER_SCENE)).is_valid(){returnNone;}')<lease.index('letdestination=&mut*core::ptr::addr_of_mut!(RENDER_SCENE);'),'reject nested capture before creating an exclusive reference'
    assert 'impl Drop for SceneLease' in s and not re.search(r'#\[derive\([^\]]*(?:Clone|Copy)[^\]]*\)\]\s*struct SceneLease',s)
wiring(source)
(out/'main-source.rs').write_text(source)
# Structural controls must change source and be rejected, never ignored.
def once(text,old,new):
    assert text.count(old)==1,'missing/ambiguous control site '+old
    return text.replace(old,new,1)
wiring_controls=[('prepare-fence','if unsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return false;}',''),('finish-fence','if unsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return;}','')]
for name,old,new in wiring_controls:
    bad=once(source,old,new)
    try:wiring(bad)
    except AssertionError:pass
    else:raise AssertionError('source control survived '+name)
def item(text,name):
    found=re.findall(r'^(?:pub )?(?:static(?: mut)?|const) '+re.escape(name)+r'\b.*?;[ \t]*(?://[^\n]*)?$',text,re.M|re.S)
    assert len(found)==1,name
    return found[0]
parent=[]
for name in ['MAX_FB_W','MAX_FB_H','BAR_BG_H','BAR_BG_W_CAP','BAR_BG_BUF','BAR_BLUR_BUF','FOCUS_SURFACE_COLOR','FRAME_RIM_COLOR','FRAME_TOP_BAR_COLOR','TAB_ACTIVE_COLOR','TAB_INACTIVE_COLOR','FRAME_LIGHT_CLOSE_COLOR','FRAME_LIGHT_MINIMIZE_COLOR','FRAME_LIGHT_ZOOM_COLOR','DEFAULT_RENDER_TOKENS']:parent.append(item(source,name))
parent.append('#[derive(Clone, Copy)]\n#[repr(C)]\n'+block(source,'struct RenderTokensV1 {'))
parent+=['use silkbar_model::{SilkBar,DEFAULT_THEME};']
for name in ['lerp_color_xrgb','vertical_gradient_xrgb','bg','scene_bar_blur_rows','fill_bar_bg_buffer','blur_bar_bg_buffer_radius1']:parent.append(function(source,name))
parent_path=out/'parent.rs';parent_path.write_text('\n'.join(parent))
lease=[]
for name in ['RENDER_SCENE','SURFACES','FRAME_GROUPS','BITMAP_STORES','DISPLAY_TOKENS','FOCUSED_SURFACE_ID','RENDER_FRAME_COUNTER','TEXT_STAGES','TOKEN_BUF_CALL1_RECEIVED','TOKEN_BUF_OWNER','TOKEN_BUF_ARG0','TOKEN_BUF_ARG1','TOKEN_BUF_ARG2','CURSOR_UNDER_VALID','APP_PRESENT_BITS','CLOCK_CANON_SS','FRAME_OUTPUT','PREPARED_CURSOR']:lease.append(item(source,name))
lease +=[block(source,'struct PublishedFrame{'),'#[derive(Clone,Copy)]\n'+block(source,'struct PreparedCursor{'),block(source,'struct SceneLease {'),block(source,'impl SceneLease {'),block(source,'impl Drop for SceneLease {'),function(source,'prepare_app_cleanup'),function(source,'finish_display_transactions')]
lease_source='\n'.join(lease);lease_path=out/'lease.rs';lease_path.write_text(lease_source)
# Re-resolve ordinary module paths from the canonical repository tests folder,
# even when a temporary proposed fixture is supplied before promotion.
def fixture(scene,lease):
    def path(m):
        rel=m[1];actual=(root/'tests'/rel).resolve(strict=True)
        if rel.endswith('/scene.rs'):actual=scene
        elif rel.endswith('/surface_groups.rs'):actual=groups_path
        return '#[path='+json.dumps(str(actual))+']'
    return re.sub(r'#\[path="([^"\n]+)"\]',path,fixture_path.read_text())
def invoke(args,name,env=None):
    r=subprocess.run(args,capture_output=True,text=True,env=env,timeout=40);(out/(name+'.log')).write_text(r.stdout+r.stderr);return r
lib=out/'libsilkbar_model.rlib';r=invoke(['rustc','--edition=2021','--crate-name','silkbar_model','--crate-type','rlib',str(root/'crates/silkbar-model/src/lib.rs'),'-o',str(lib)],'model.compile');assert r.returncode==0,r.stderr
cases=len(re.findall(r'#\[test\]',fixture_path.read_text()));assert cases>=11,'six Scene and five lease cases required'
def compile_run(name,scene,lease,exact=None,rejected=False):
    f=out/(name+'.rs');f.write_text(fixture(scene,lease));binary=out/name;env=os.environ.copy();env.update(DISPLAY_SCENE_PARENT=str(parent_path),DISPLAY_SCENE_LEASE=str(lease))
    r=invoke(['rustc','--edition=2021','--extern','silkbar_model='+str(lib),'--test',str(f),'-o',str(binary)],name+'.compile',env);assert r.returncode==0,name+' must compile: '+r.stderr
    args=[str(binary),'--test-threads=1','--nocapture']+(['--exact',exact]if exact else[]);r=invoke(args,name+'.runtime');text=r.stdout+r.stderr
    assert r.returncode==(101 if rejected else 0),name+': '+text
    if exact:assert 'running 1 test'in text and re.search(r'^test '+re.escape(exact)+r' \.\.\. '+('FAILED'if rejected else'ok')+r'$',text,re.M),name+' exact runtime case not executed'
    else:assert f'test result: ok. {cases} passed; 0 failed;'in text;print(text,end='')
compile_run('actual',scene_path,lease_path)
controls=[('surface-copy-omitted','scene','self.surfaces.copy_from_slice(surfaces);','/* omitted capture */','every_surface_slot_preserves_all_bytes_selection_layout_fill_and_cursor'),('scrub-length-only','scene','core::ptr::write_volatile(surface,SURFACE_EMPTY);','surface.text_len=0;','failed_bitmap_capture_scrubs_the_entire_old_scene'),('nested-recapture','lease','if (*core::ptr::addr_of!(RENDER_SCENE)).is_valid() { return None; }','/* recapture allowed */','actual_lease_rejects_nested_capture_without_changing_live_snapshot'),('drop-without-scrub','lease','(*core::ptr::addr_of_mut!(RENDER_SCENE)).scrub();','/* no drop scrub */','actual_lease_drop_early_return_and_host_unwind_scrub_owned_payload'),('prepare-during-lease','lease','if unsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return false;}','/* no preparation fence */','actual_cleanup_functions_withhold_effects_and_ack_until_render_lease_drops'),('finish-during-lease','lease','if unsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return;}','/* no completion fence */','actual_cleanup_functions_withhold_effects_and_ack_until_render_lease_drops')]
for name,target,old,new,test in controls:
    base=scene_path.read_text()if target=='scene'else lease_source;changed=once(base,old,new);path=out/(name+'-module.rs');path.write_text(changed)
    compile_run(name,path if target=='scene'else scene_path,path if target=='lease'else lease_path,test,True)
    print('[display.scene.mutant] rejected='+name+' exact='+test)
assert main_path.read_text()==source,'Main changed during host proof; rerun against a fixed source capture'
proof=dict(ok=True,actual_cases=cases,compiled_runtime_mutants=len(controls),source_fence_controls=len(wiring_controls),scope='actual_Scene_Main_lease_cleanup_with_finite_service_and_renderer_boundaries_not_native_pixels_authority_or_epoch_closure',sha256={str(v):hashlib.sha256(v.read_bytes()).hexdigest()for v in[main_path,scene_path,groups_path,fixture_path,root/'servers/sexdisplay/src/frame_output.rs',root/'crates/sex-pdx/src/display_frames.rs',root/'crates/sex-pdx/src/memory_region.rs',root/'servers/sexdisplay/src/paint_batch.rs',root/'servers/sexdisplay/src/damage.rs']})
(out/'result.json').write_text(json.dumps(proof,indent=2)+'\n');print('[display.scene.host] PASS '+json.dumps(proof,sort_keys=True))
if managed:managed.cleanup()
PY
