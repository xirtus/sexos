#!/usr/bin/env python3
"""Compile immutable source binding with current/future host metadata and controls.

Actual SDK Plan/INFO decoders are included. Future catalog rows are explicit
host metadata only. No runtime holder query, BOOT grants or native authority
is established by this checker.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
MODULE='servers/sexnet/src/source_binding.rs'
FIXTURE='tests/source_binding.rs'


def once(source,old,new):
    if source.count(old)!=1:raise ValueError('missing/ambiguous exact mutation site: '+old)
    return source.replace(old,new,1)


def fixture_source(root,module):
    path=root/FIXTURE;substitutions=0
    def replace(match):
        nonlocal substitutions
        original=(path.parent/match[1]).resolve(strict=True)
        if original==(root/MODULE).resolve():original=module;substitutions+=1
        return '#[path='+json.dumps(str(original))+']'
    source=re.sub(r'#\[path\s*=\s*"([^"\n]+)"\s*\]',replace,path.read_text())
    if substitutions!=1:raise ValueError('actual fixture must include source_binding exactly once')
    return source


def compile_fixture(root,out,name,module,library,future=False):
    fixture=out/(name+'.rs');fixture.write_text(fixture_source(root,module));binary=out/name
    args=['rustc','--edition=2021','--extern','sex_trust_config='+str(library),
          '--test',str(fixture),'-o',str(binary)]
    if future:args+=['--cfg','browser_source_catalog']
    result=subprocess.run(args,capture_output=True,text=True,timeout=60)
    (out/(name+'.compile.log')).write_text(result.stdout+result.stderr)
    if result.returncode:raise ValueError(name+': actual/mutant must compile; see '+str(out/(name+'.compile.log')))
    return binary


def run_fixture(out,name,binary,exact=None,reject=False):
    args=[str(binary),'--test-threads=1']
    if exact:args+=['--exact',exact]
    result=subprocess.run(args,capture_output=True,text=True,timeout=30)
    output=result.stdout+result.stderr;(out/(name+'.log')).write_text(output)
    if result.returncode!=(101 if reject else 0):raise ValueError(name+': unexpected behavioral test result')
    if exact:
        expected='FAILED'if reject else'ok'
        if 'running 1 test'not in output or f'test {exact} ... {expected}'not in output:
            raise ValueError(name+': required exact control did not execute')
    else:
        if '27 passed; 0 failed'not in output:raise ValueError('expected27 actual SDK/binding case executions')
        print(output,end='')


def check(root,out):
    library=out/'libsex_trust_config.rlib'
    result=subprocess.run(['rustc','--edition=2021','--crate-name','sex_trust_config','--crate-type','rlib',
                           str(root/'crates/sex-trust-config/src/lib.rs'),'-o',str(library)],capture_output=True,text=True,timeout=30)
    (out/'trust-config.compile.log').write_text(result.stdout+result.stderr)
    if result.returncode:raise ValueError('actual trust configuration must compile')
    source=(root/MODULE).read_text()
    actual=compile_fixture(root,out,'actual',(root/MODULE).resolve(),library)
    run_fixture(out,'actual',actual)
    future=compile_fixture(root,out,'future-host-catalog',(root/MODULE).resolve(),library,future=True)
    run_fixture(out,'future-host-catalog',future)
    controls=(
        ('task-inference','Some(Self{plan,task:c.task_id,entropy,clock,trust})',
         'Some(Self{plan,task:c.target_pd,entropy,clock,trust})',
         'binding_preserves_actual_held_unequal_task_and_each_typed_decoder_tuple'),
        ('crossed-slot-labels','if catalog.0!=plan.lifecycle_slot() || entropy.0!=plan.entropy_slot()\n            || clock.0!=plan.clock_slot() || trust.0!=plan.trust_slot(){return None}',
         'if catalog.0!=plan.lifecycle_slot(){return None}',
         'binding_query_errors_absent_wrong_kind_and_slot_labels_fail_closed'),
        ('profile-bypass',' || !trust.valid()','',
         'binding_rechecks_retained_source_profile_root_and_version'),
    )
    for name,old,new,test in controls:
        changed=once(source,old,new)
        module=out/(name+'-module.rs');module.write_text(changed)
        binary=compile_fixture(root,out,name,module,library)
        run_fixture(out,name,binary,exact=test,reject=True)
        print('[source.binding.mutant] rejected='+name+' exact='+test)
    result=dict(ok=True,actual_case_executions=27,future_host_case_executions=27,compiled_mutants_rejected=3,
                scope='actual_binding_Plan_INFO_decoders_with_host_future_catalog_not_runtime_holder_queries_or_BOOT_grants')
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=ROOT)
    parser.add_argument('--out',type=Path,help='new retained evidence directory')
    args=parser.parse_args();root=args.root.resolve(strict=True)
    if args.out:
        args.out.mkdir(parents=True,exist_ok=False);result=check(root,args.out.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix='sexos-source-binding-')as directory:result=check(root,Path(directory))
    print('[source.binding.host] PASS '+json.dumps(result,sort_keys=True))


if __name__=='__main__':main()
