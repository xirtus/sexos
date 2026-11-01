#!/usr/bin/env python3
"""Compile the actual denial-only outbox fixture and two retained-fence controls.

This is host protocol/composition coverage, including actual SDK and Machine
unit-case executions. It does not prove native dispatch, source authority,
BOOT grants, network traffic, or guest pixels.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
MODULE='servers/sexnet/src/source_denial.rs'
FIXTURE='tests/source_denial.rs'


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
    if substitutions!=1:raise ValueError('actual fixture must include source_denial exactly once')
    return source


def compile_fixture(root,out,name,module,library):
    fixture=out/(name+'.rs');fixture.write_text(fixture_source(root,module));binary=out/name
    result=subprocess.run(['rustc','--edition=2021','--extern','sex_trust_config='+str(library),
                           '--test',str(fixture),'-o',str(binary)],capture_output=True,text=True,timeout=60)
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
        if '43 passed; 0 failed'not in output:raise ValueError('expected43 actual SDK/Machine/denial case executions')
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
    controls=(
        ('lowered-cache-floor','if slot.cached.is_none_or(|cached| seq > cached.seq)','if true',
         'stale_replies_never_lower_or_replace_latest_denied_cache_including_last_sequence'),
        ('evicted-cached-owner','slot.owner.is_none()','slot.pending.is_none()',
         'capacity_is_per_kind_and_never_evicts_pending_or_cached_owner_fences'),
    )
    for name,old,new,test in controls:
        changed=once(source,old,new)
        module=out/(name+'-module.rs');module.write_text(changed)
        binary=compile_fixture(root,out,name,module,library)
        run_fixture(out,name,binary,exact=test,reject=True)
        print('[source.denial.mutant] rejected='+name+' exact='+test)
    result=dict(ok=True,normal_case_executions=43,compiled_mutants_rejected=2,
                scope='actual_denial_outbox_typed_SDK_and_Machine_host_composition_not_native_source_authority_or_BOOT_grants')
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=ROOT)
    parser.add_argument('--out',type=Path,help='new retained evidence directory')
    args=parser.parse_args();root=args.root.resolve(strict=True)
    if args.out:
        args.out.mkdir(parents=True,exist_ok=False);result=check(root,args.out.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix='sexos-source-denial-')as directory:result=check(root,Path(directory))
    print('[source.denial.host] PASS '+json.dumps(result,sort_keys=True))


if __name__=='__main__':main()
