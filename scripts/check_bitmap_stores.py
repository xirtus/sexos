#!/usr/bin/env python3
"""Compile actual bitmap stores and explicit future host metadata controls.

Normal current/future catalogs retain all SDK checks. Only the deliberately
malformed duplicate-SID host catalog bypasses the SDK's duplicate-SID const
guard, so Stores' own independent ambiguity rejection can execute. No catalog
in the repository changes; no future grant, activation or native pixel is proved.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
MODULE='servers/sexdisplay/src/bitmap_stores.rs'
FIXTURE='tests/bitmap_stores.rs'
CATALOG='crates/sex-pdx/src/app_catalog.rs'
AMBIGUOUS='ambiguous_catalog_associations_cannot_install_or_select_a_store'


def once(source,old,new):
    if source.count(old)!=1:raise ValueError('missing/ambiguous exact control site: '+old)
    return source.replace(old,new,1)


def static_backing(source):
    code=re.sub(r'\s+','',re.sub(r'//[^\n]*','',source))
    expected='pubconstUNBOUND:Self=Self{stages:[const{Stage::UNBOUND};N]};'
    if expected not in code or code.count('Stage::UNBOUND')!=1:
        raise ValueError('store backing must initialize directly as a const array')
    if 'self.stages[index].initialize_once(binding)'not in code:
        raise ValueError('installation must update only the existing Stage binding')


def future_catalog(source):
    source='\n'.join(line for line in source.splitlines()if not line.startswith('//!'))+'\n'
    matches=list(re.finditer(r'pub const INCARNATIONS:\s*\[Incarnation;\s*(\d+)\]\s*=\s*\[',source))
    if len(matches)!=1:raise ValueError('expected one actual catalog array')
    match=matches[0];end=source.index('\n];',match.end())
    # These literals are host-only metadata, independently fixed by the tests.
    rows='''
    Incarnation { app: App::Browser, ordinal: 1, target: 30, surface: 343, shell_surface: None, lifecycle_slot: 154, close_slot: 155, activation_slot: Some(156), route_slot: 157, children: &[] },
    Incarnation { app: App::Browser, ordinal: 2, target: 31, surface: 344, shell_surface: None, lifecycle_slot: 158, close_slot: 159, activation_slot: Some(160), route_slot: 161, children: &[] },'''
    additions=[line for line in rows.splitlines()if line.strip()and line.strip()not in {v.strip()for v in source.splitlines()}]
    source=source[:end]+('\n'+'\n'.join(additions)if additions else '')+source[end:]
    return source[:match.start(1)]+str(int(match[1])+len(additions))+source[match.end(1):]


def fixture_source(root,module):
    fixture=root/FIXTURE;substitutions=0
    def path(match):
        nonlocal substitutions
        original=(fixture.parent/match[1]).resolve(strict=True)
        if original==(root/MODULE).resolve():original=module;substitutions+=1
        return '#[path='+json.dumps(str(original))+']'
    result=re.sub(r'#\[path\s*=\s*"([^"\n]+)"\s*\]',path,fixture.read_text())
    if substitutions!=1:raise ValueError('fixture must include actual Stores exactly once')
    return result


def compile_fixture(root,out,name,module,catalog=None,ambiguous=False):
    fixture=out/(name+'.rs');fixture.write_text(fixture_source(root,module));binary=out/name
    args=['rustc','--edition=2021','--test',str(fixture),'-o',str(binary)]
    env=os.environ.copy()
    if catalog:args+=['--cfg','bitmap_stores_catalog'];env['BITMAP_STORES_CATALOG']=str(catalog)
    if ambiguous:args+=['--cfg','bitmap_stores_ambiguous']
    result=subprocess.run(args,capture_output=True,text=True,env=env,timeout=30)
    (out/(name+'.compile.log')).write_text(result.stdout+result.stderr)
    if result.returncode:raise ValueError(name+': actual/mutant fixture must compile')
    return binary


def run_fixture(out,name,binary,exact=None,reject=False):
    args=[str(binary),'--test-threads=1','--nocapture']
    if exact:args+=['--exact',exact]
    result=subprocess.run(args,capture_output=True,text=True,timeout=30)
    output=result.stdout+result.stderr;(out/(name+'.log')).write_text(output)
    if result.returncode!=(101 if reject else 0):raise ValueError(name+': unexpected behavioral result')
    if exact:
        status='FAILED'if reject else'ok'
        if 'running 1 test'not in output or not re.search(r'^test '+re.escape(exact)+r' \.\.\. '+status+r'$',output,re.M):
            raise ValueError(name+': exact named behavioral control did not execute')
    else:
        if 'test result: ok. 10 passed; 0 failed;'not in output:raise ValueError(name+': expected10 actual store cases')
        print(output,end='')
    return output


def storage_measure(output,count):
    matches=re.findall(r'\[display.bitmap.stores.storage\] stores1=(\d+) stores3=(\d+) stage=(\d+) current_count=(\d+)',output)
    if len(matches)!=1:raise ValueError('missing/ambiguous actual storage measurement')
    values=tuple(map(int,matches[0]))
    if values!=(18664,55992,18664,count):raise ValueError('bitmap host backing/count budget changed: '+str(values))
    return values


def check(root,out):
    module=(root/MODULE).resolve(strict=True);source=module.read_text();static_backing(source)
    future=future_catalog((root/CATALOG).read_text());future_path=out/'future-catalog.rs';future_path.write_text(future)
    actual=compile_fixture(root,out,'actual',module)
    measured=storage_measure(run_fixture(out,'actual',actual),3)
    expanded=compile_fixture(root,out,'future',module,future_path)
    storage_measure(run_fixture(out,'future',expanded),3)
    target=once(future,'ordinal: 2, target: 31','ordinal: 2, target: 30')
    surface=once(future,'target: 31, surface: 344','target: 31, surface: 343')
    # The real SDK rejects duplicate SIDs at compile time. Bypass exactly that
    # guard ONLY in this malformed host copy, never actual or normal-future data,
    # to test Stores' independent runtime defense rather than a compile failure.
    surface=once(surface,'assert!(APP_SURFACES[i]!=APP_SURFACES[j]);',
                 '/* malformed host data only: independently exercise Stores ambiguity guard */')
    for name,catalog in [('ambiguous-target',target),('ambiguous-surface',surface)]:
        path=out/(name+'-catalog.rs');path.write_text(catalog)
        binary=compile_fixture(root,out,name,module,path,True)
        run_fixture(out,name,binary,AMBIGUOUS)
        print('[bitmap.stores.ambiguity] PASS '+name+' scope=malformed_host_metadata_only')
    controls=(
        ('fixed-initialization-index','self.stages[index].initialize_once(binding)','self.stages[0].initialize_once(binding)',
         'equal_sequences_private_staging_pixels_and_receipts_remain_per_incarnation'),
        ('first-only-reply','for stage in &mut self.stages{stage.poll_reply(&mut send);}',
         'for stage in &mut self.stages{stage.poll_reply(&mut send);break;}',
         'full_owner_keeps_exact_destination_without_starving_other_stores'),
        ('reset-before-rebind','self.stages[index].initialize_once(binding)',
         '{self.stages[index]=Stage::UNBOUND;self.stages[index].initialize_once(binding)}',
         'retirement_erases_only_exact_old_incarnation_and_cannot_rebind_or_revive'),
    )
    for name,old,new,test in controls:
        changed=once(source,old,new);path=out/(name+'-module.rs');path.write_text(changed)
        binary=compile_fixture(root,out,name,path,future_path)
        run_fixture(out,name,binary,test,True)
        print('[bitmap.stores.mutant] rejected='+name+' exact='+test)
    result=dict(ok=True,actual_case_executions=10,future_case_executions=10,
                ambiguity_runtime_controls=2,compiled_mutants_rejected=3,
                stores1_bytes=measured[0],stores3_bytes=measured[1],stage_bytes=measured[2],
                current_catalog_browsers=3,future_host_catalog_browsers=3,
                scope='actual_Stage_Binding_SDK_host_composition_not_future_BOOT_grants_or_native_pixels')
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=ROOT)
    parser.add_argument('--out',type=Path,help='new retained evidence directory')
    args=parser.parse_args();root=args.root.resolve(strict=True)
    if args.out:
        args.out.mkdir(parents=True,exist_ok=False);result=check(root,args.out.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix='sexos-bitmap-stores-')as directory:result=check(root,Path(directory))
    print('[bitmap.stores.host] PASS '+json.dumps(result,sort_keys=True))


if __name__=='__main__':main()
