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
def source_contract(text):
    wrappers={'render':'Full','redraw_top_strip':'Strip','redraw_surface_area':'Area','redraw_surface_damage':'Damage(rect)','redraw_cursor':'Cursor'}
    for name,paint in wrappers.items():
        b=fn(text,name);assert 'render_prepared(fb,w,h,bar,Paint::'+paint+')' in b,name+' must use prepared output'
        assert 'write_volatile'not in b and 'raster_'not in b
    for name in ['raster_full','raster_strip','raster_area','raster_damage','raster_cursor']:
        b=fn(text,name);assert "target: &frame_output::Target<'_>"in b and 'let fb=target.pointer();'in b,name+' must use actual private Target'
        assert not re.search(r'FB_PTR|HIGH_HALF_BASE|valid_scanout\(',b),name+' reintroduced physical-address raster coupling'
        assert not any(x+'('in b for x in ['report_app_presented','record_cursor_presented','civil_clock_drawn','repaint_end']),name+' emitted effects before publication'
    body=fn(text,'render_prepared');commit='if !preparation.commit(|pixels|publish_frame(fb,w,h,pixels)){backup.restore();return false;}'
    assert commit in body
    for effect in ['record_cursor_presented();','report_app_presented();','repaint_end(paint.label(),started);','civil_clock_drawn(&captured_bar,w,h,paint.label());']:
        assert body.count(effect)==1 and body.index(commit)<body.index(effect),'effect before successful CPU publication'
    publish=fn(text,'publish_frame');assert publish.index('if !valid_scanout')<publish.index('core::ptr::write_volatile(fb.add(index),*pixel);')
    assert 'w.checked_mul(h)!=Some(pixels.len())'in publish
    assert 'core::arch::asm!("sfence",options(nostack,preserves_flags));'in publish,'missing CPU output fence'
    assert publish.index('core::ptr::write_volatile(fb.add(index),*pixel);')<publish.index('core::arch::asm!("sfence",options(nostack,preserves_flags));'),'CPU stores must precede the output fence'
    startup=fn(text,'_start');acquire='sex_pdx::display_frame_backing(frame_layout)'
    assert text.count(acquire)==1 and startup.index(acquire)<startup.index('FRAME_OUTPUT=Some(frame_output::Store::new(frame_backing))')<startup.index('render(FB_PTR as *mut u32')
    assert 'assert!(FRAME_OUTPUT.is_none(),"DISPLAY_FRAME_ALREADY_INITIALIZED")'in startup
    prepare=fn(text,'prepare_app_cleanup');assert 'if let Some(output)=FRAME_OUTPUT.as_mut(){output.scrub();}'in prepare
    assert prepare.index('output.scrub();')<prepare.index('[display.app.cleanup.prepared]')
    finish=fn(text,'finish_display_transactions');assert finish.index('render(FB_PTR as *mut u32')<finish.index('lifecycle.pixels_presented')<finish.index('lifecycle.scene_committed')<finish.index('sex_pdx::app_lifecycle::ack(')
    store=(root/'servers/sexdisplay/src/frame_output.rs').read_text();scrub=fn(store,'scrub');assert 'self.erase(0);self.erase(1);'in scrub
    # Every physical FB_PTR conversion is routed to a prepared wrapper, or to
    # the preexisting proof's unused *const argument. Other raster fb pointers
    # are obtained from Target above; this is a source boundary check, not a
    # semantic proof for arbitrary future unsafe code.
    for line in text.splitlines():
        if re.search(r'FB_PTR\s+as\s+\*(?:mut|const)\s+u32',line):
            assert any(x in line for x in [name+'(' for name in wrappers]+['top_strip_render_proof(','let fb = FB_PTR as *mut u32;']),line
    assert text.count('let fb = FB_PTR as *mut u32;')==1
    assert re.search(r'let fb = FB_PTR as \*mut u32;\s*let \(w, h\) = \(FB_W as usize, FB_H as usize\);\s*redraw_cursor\(fb,w,h,&bar\);',text)
    proof=block(text,'unsafe fn top_strip_render_proof(');assert proof.count('_fb')==1,'proof must not write physical scanout'
    return ['five-prepared-wrappers','five-private-target-roots','no-raster-effects','post-publication-effects-and-timing','preflight-before-device-copy','one-time-held-acquisition-before-first-render','cleanup-scrubs-both-spans','render-before-cleanup-ack','physical-address-call-boundary']
wiring=source_contract(main)
source_controls=[('missing-output-fence','core::arch::asm!("sfence",options(nostack,preserves_flags));','/* removed CPU output fence */'),('missing-cleanup-scrub','if let Some(output)=FRAME_OUTPUT.as_mut(){output.scrub();}','/* omitted backing scrub */'),('physical-raster-pointer','fn raster_cursor(target: &frame_output::Target<\'_>, bar: &SilkBar) -> bool {\n    let fb=target.pointer();',"fn raster_cursor(target: &frame_output::Target<'_>, bar: &SilkBar) -> bool {\n    let fb=FB_PTR as *mut u32;"),('early-acquisition-bypass','sex_pdx::display_frame_backing(frame_layout)','unchecked_frame_backing(frame_layout)')]
for name,old,new in source_controls:
    altered=once(main,old,new)
    try:source_contract(altered)
    except AssertionError:pass
    else:raise AssertionError('source control accepted: '+name)
# Keep exact small types and methods from Main; no duplicate output state model.
parts=[]
for name in ['CURSOR_ARROW_W','CURSOR_ARROW_H']:
    found=re.findall(r'^const '+name+r':[^\n]+;',main,re.M);assert len(found)==1;parts+=found
for name in ['CURSOR_UNDER_SAVED','CURSOR_UNDER_X','CURSOR_UNDER_Y','CURSOR_UNDER_VALID','PREPARED_CURSOR']:
    found=re.findall(r'^static mut '+name+r':.*?;[ \t]*(?=\n|$)',main,re.M|re.S);assert len(found)==1;parts+=found
def derived(needle):
    prefix=main[:main.index(needle)];match=re.search(r'(#[\[]derive\([^\n]+\)[\]])\s*$',prefix);assert match,'missing actual derive '+needle
    return match[1]+'\n'+block(main,needle)
parts+=[derived('struct PreparedCursor{'),derived('enum Paint{'),block(main,'impl Paint{'),block(main,'struct OutputBackup{'),block(main,'impl OutputBackup{'),block(main,'impl Drop for OutputBackup{')]
parts+=[fn(main,n)for n in ['valid_scanout','publish_frame','render_prepared','render','redraw_top_strip','redraw_surface_area','redraw_surface_damage','redraw_cursor']]
production='\n\n'.join(parts)+'\n';production_path=out/'actual-main-pipeline.rs';production_path.write_text(production)
acquire_path=out/'actual-sdk-acquisition.rs';acquire_path.write_text('pub '+fn(sdk,'display_frame_backing')+'\n')
(out/'main-source.rs').write_text(main)
def normalize(text):
    return re.sub(r'#\[path\s*=\s*"([^\"]+)"\]',lambda m: '#[path='+json.dumps(str((root/'tests'/m[1]).resolve(strict=True)))+']',text)
def invoke(cmd,name,env=None):
    r=subprocess.run(cmd,text=True,capture_output=True,env=env,timeout=120)
    (out/(name+'.log')).write_text('$ '+' '.join(map(str,cmd))+'\n'+r.stdout+r.stderr)
    return r
inputs=[Path(os.environ['OUTPUT_GATE_PATH']).resolve(strict=True),main_path,sdk_path,fixture,root/'tests/display_frames.rs',root/'tests/display_frame_output.rs',root/'kernel/src/memory/va_allocator.rs',root/'servers/sexdisplay/src/frame_output.rs',root/'servers/sexdisplay/src/damage.rs',root/'crates/sex-pdx/src/display_frames.rs',root/'crates/sex-pdx/src/memory_region.rs']+sorted((root/'crates/silkbar-model/src').rglob('*.rs'))
input_hashes={str(v):hashlib.sha256(v.read_bytes()).hexdigest()for v in inputs}
lib=out/'libsilkbar_model.rlib';r=invoke(['rustc','--edition=2021','--crate-name','silkbar_model','--crate-type','rlib',str(root/'crates/silkbar-model/src/lib.rs'),'-o',str(lib)],'model.compile');assert r.returncode==0,r.stderr
def compile_test(name,text,pipeline=production_path,case=None,expect_failure=False,count=None):
    path=out/(name+'.rs');path.write_text(normalize(text));binary=out/name
    env=dict(os.environ,DISPLAY_FRAME_MAIN=str(pipeline),DISPLAY_FRAME_ACQUIRE=str(acquire_path))
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
for name,file,expected in [('layout','tests/display_frames.rs',7),('store','tests/display_frame_output.rs',8),('va','kernel/src/memory/va_allocator.rs',6)]:
    counts[name]=compile_test(name,(root/file).read_text(),count=expected)
fixture_text=fixture.read_text();counts['pipeline']=compile_test('pipeline',fixture_text,count=6)
# Unforgeable ownership discipline is a type property, not a native authority
# test. Require the intended diagnostic, never arbitrary compiler failure.
control_prefix='''#![allow(dead_code)]\n#[path="../crates/sex-pdx/src/memory_region.rs"]mod memory_region;\npub use memory_region::MemoryRegionInfo;\npub const ERR_CAP_INVALID:u64=0xFFFF_FFFF_FFFF_FFFC;\n#[path="../crates/sex-pdx/src/display_frames.rs"]mod display_frames;\n'''
for trait in ['Copy','Clone','Send','Sync']:
    path=out/('forbid-'+trait+'.rs');path.write_text(normalize(control_prefix+'fn need<T:'+trait+'>(){}\nfn main(){need::<display_frames::FrameBacking>();}\n'))
    r=invoke(['rustc','--edition=2021','--crate-name','forbid_'+trait.lower(),str(path),'-o',str(out/('forbid-'+trait))],'forbid-'+trait+'.compile')
    assert r.returncode!=0 and 'error[E0277]' in r.stderr and trait in r.stderr and 'FrameBacking' in r.stderr,'unexpected trait diagnostic: '+r.stderr
controls=[
 ('early-presentation','if !prepared{drop(preparation);backup.restore();return false;}','report_app_presented();\n    if !prepared{drop(preparation);backup.restore();return false;}','actual_wrappers_copy_before_present_with_low_va_private_target'),
 ('skip-cpu-copy','for(index,pixel)in pixels.iter().enumerate(){unsafe{core::ptr::write_volatile(fb.add(index),*pixel);}}','/* omitted CPU copy */','actual_wrappers_copy_before_present_with_low_va_private_target'),
 ('omit-raster-rollback','if !prepared{drop(preparation);backup.restore();return false;}','if !prepared{drop(preparation);return false;}','every_wrapper_restores_bookkeeping_and_retains_front_after_raster_abort')]
for name,old,new,case in controls:
    path=out/(name+'-actual-main.rs');path.write_text(once(production,old,new))
    compile_test(name,fixture_text,pipeline=path,case=case,expect_failure=True)
    print('[display.frame.output.mutant] rejected='+name+' exact='+case)
assert main_path.read_text()==main and sdk_path.read_text()==sdk,'source changed during proof; rerun'
assert all(hashlib.sha256(v.read_bytes()).hexdigest()==input_hashes[str(v)] for v in inputs),'an actual input changed during proof; rerun'
proof=dict(ok=True,cases=counts,compiled_runtime_mutants=len(controls),compiler_trait_controls=4,source_controls=len(source_controls),wiring=wiring,scope='actual SDK acquisition, Layout, Store, VA and extracted Main output pipeline; host-owned exclusive memory and finite raster/scanout/effect doubles; no native framebuffer/BOOT authority, physical atomicity, full epoch or whole-system replay proof',sha256=input_hashes)
(out/'result.json').write_text(json.dumps(proof,indent=2)+'\n');print('[display.frame.output.host] PASS '+json.dumps(proof,sort_keys=True))
if managed:managed.cleanup()
PY
