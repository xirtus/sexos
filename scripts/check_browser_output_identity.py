#!/usr/bin/env python3
"""Check Browser output wiring and compiled immutable bitmap binding controls.

Future identities exist only as host fixtures. This neither grants future BOOT
subjects nor proves Display admission, native pixels, or runtime authority.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
CONTROL='immutable_constructor_binds_begin_chunks_commit_clear_and_exact_applied'
MODULE='apps/kaleidoscope/src/bitmap_delivery.rs'
FIXTURE='tests/browser_bitmap_delivery.rs'


def compact(value):
    return re.sub(r'\s+','',value)


def main_wiring(source):
    found=re.findall(r'^    fn new\(_pdx: u32\) -> Self \{.*?^    }',source,re.M|re.S)
    if len(found)!=1:raise ValueError('expected actual unique App::new body')
    body=compact(found[0]);whole=compact(source)
    prefix='''fn new(_pdx: u32) -> Self {
        let identity=sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT).expect("BROWSER_SELF_AUTHORITY");
        let selected=incarnation::select(identity).expect("BROWSER_EXACT_BOOT_IDENTITY");
        let surface=selected.surface();'''
    if not body.startswith(compact(prefix)):raise ValueError('output surface must follow successful held query and selection')
    for name in ('let selected=','let surface='):
        if body.count(compact(name))!=1:raise ValueError('ambiguous selected identity/surface binding')
    required=(
        'bitmap:bitmap_delivery::Outbox::new(selected)',
        'ui:text_delivery::Outbox::atomic(surface)',
        'service_call::enqueue(sex_pdx::SLOT_DISPLAY,0xEC,[surface,',
        'service_call::enqueue(sex_pdx::SLOT_DISPLAY,0xFB,[surface,packed,arg2])',
        'service_call::enqueue(sex_pdx::SLOT_DISPLAY,0xEB,[surface,50,100])',
        '''assert_eq!(service_call::request(sex_pdx::SLOT_SHELL,3,0xFA,
            [surface,BROWSER_TITLE_ID,(selected.target()as u64)<<8]).expect("shell app registration"),surface);''',
        '''service_call::request(sex_pdx::SLOT_DISPLAY,1,
            sex_pdx::app_lifecycle::OP_APP_PRESENT,[surface,1,0])''',
        '''sex_pdx::serial_record!("[app.surface.registered] target={} surface={} owner={}",selected.target(),surface,selected.target());''',
        '''sex_pdx::serial_record!("[app.surface.presented] target={} surface={} sequence=1 ok=1",selected.target(),surface);''',
    )
    for expected in required:
        if compact(expected)not in body:raise ValueError('unbound App::new output: '+expected.split('(')[0])
    for constructor in ('bitmap_delivery::Outbox::new(', 'text_delivery::Outbox::atomic('):
        if whole.count(constructor)!=1:raise ValueError('ambiguous output constructor wiring')
    if 'constBROWSER_TITLE_ID:u64=300;'not in whole:raise ValueError('descriptive Browser family title changed')
    for forwarding in (
        'sex_pdx::pdx_call_checked(sex_pdx::SLOT_DISPLAY,request.opcode,request.surface,request.packed,request.control)',
        'sex_pdx::pdx_call_checked(sex_pdx::SLOT_DISPLAY,op.opcode,op.surface,op.packed,op.control)',
    ):
        if forwarding not in whole:raise ValueError('live output transport must forward its bound operation surface')


def replace_once(source,old,new):
    if source.count(old)!=1:raise ValueError('expected one actual mutation guard: '+old)
    return source.replace(old,new,1)


def fixture_source(root,module):
    fixture=root/FIXTURE
    substitutions=0
    def path(match):
        nonlocal substitutions
        original=(fixture.parent/match[1]).resolve(strict=True)
        if original==(root/MODULE).resolve():
            substitutions+=1;original=module
        return '#[path = '+json.dumps(str(original))+']'
    value=re.sub(r'#\[path\s*=\s*"([^"\n]+)"\s*\]',path,fixture.read_text())
    if substitutions!=1:raise ValueError('fixture must include the actual bitmap module exactly once')
    if not re.search(r'^fn '+CONTROL+r'\(',value,re.M):raise ValueError('required future-binding test is missing')
    return value


def compile_fixture(root,out,name,module):
    path=out/(name+'.rs');path.write_text(fixture_source(root,module));binary=out/name
    result=subprocess.run(['rustc','--edition=2021','--test',str(path),'-o',str(binary)],
                          capture_output=True,text=True,timeout=30)
    (out/(name+'.compile.log')).write_text(result.stdout+result.stderr)
    if result.returncode:raise ValueError(name+': actual fixture must compile; see '+str(out/(name+'.compile.log')))
    return binary


def run_control(out,name,binary,rejected=False):
    result=subprocess.run([str(binary),'--exact',CONTROL,'--test-threads=1'],
                          capture_output=True,text=True,timeout=30)
    output=result.stdout+result.stderr;(out/(name+'.log')).write_text(output)
    expected='FAILED'if rejected else'ok'
    if result.returncode!=(101 if rejected else 0)or 'running 1 test'not in output or f'test {CONTROL} ... {expected}'not in output:
        raise ValueError(name+': exact future-binding control did not '+('reject compiled mutant'if rejected else'pass'))


def check(root,out,baseline_binary=None):
    main=(root/'apps/kaleidoscope/src/main.rs').read_text();main_wiring(main)
    # Static main-wiring counterexamples must reject; these do not claim a
    # compiled App::new execution. The actual bitmap behavior is compiled below.
    changes=(
        ('let surface=selected.surface();','let surface=300;'),
        ('bitmap:bitmap_delivery::Outbox::new(selected)','bitmap:bitmap_delivery::Outbox::new(original)'),
        ('ui:text_delivery::Outbox::atomic(surface)','ui:text_delivery::Outbox::atomic(300)'),
        ('[surface, BROWSER_TITLE_ID, (selected.target() as u64) << 8]','[300, BROWSER_TITLE_ID, (15u64) << 8]'),
        ('OP_APP_PRESENT, [surface, 1, 0]','OP_APP_PRESENT, [300, 1, 0]'),
    )
    for old,new in changes:
        changed=replace_once(main,old,new)
        try:main_wiring(changed)
        except ValueError:pass
        else:raise ValueError('unbound main-wiring counterexample survived: '+old)
    if baseline_binary is None:baseline_binary=compile_fixture(root,out,'actual',(root/MODULE).resolve())
    else:baseline_binary=baseline_binary.resolve(strict=True)
    # The enclosing host gate already ran the complete actual fixture.
    # Reuse that binary and rerun just this exact control, avoiding recompiling
    # the normal module or rerunning its full suite. Standalone mode compiles it.
    run_control(out,'actual-control',baseline_binary)
    source=(root/MODULE).read_text()
    for name,old,new in (
        ('fixed-operation-surface','surface: self.identity.surface(),','surface: 300,'),
        ('fixed-receipt-surface','applied.surface as u64 != self.identity.surface()','applied.surface as u64 != 300'),
    ):
        path=out/(name+'-module.rs');path.write_text(replace_once(source,old,new))
        binary=compile_fixture(root,out,name,path)
        run_control(out,name,binary,True)
    result=dict(ok=True,main_wiring=True,wiring_controls_rejected=len(changes),compiled_mutants_rejected=2,
                exact_control=CONTROL,baseline_reused=baseline_binary!=(out/'actual'),
                scope='actual_bitmap_outbox_host_and_source_wiring_not_future_BOOT_Display_admission_or_pixels')
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=ROOT)
    parser.add_argument('--baseline-binary',type=Path,help='actual bitmap fixture just compiled/run by enclosing host gate')
    parser.add_argument('--out',type=Path,help='new directory retaining compiled controls and logs')
    args=parser.parse_args()
    if args.out:
        args.out.mkdir(parents=True,exist_ok=False)
        result=check(args.root.resolve(),args.out.resolve(),args.baseline_binary)
    else:
        with tempfile.TemporaryDirectory(prefix='sexos-browser-output-')as directory:
            result=check(args.root.resolve(),Path(directory),args.baseline_binary)
    print('[browser.output.identity] PASS '+json.dumps(result,sort_keys=True))


if __name__=='__main__':main()
