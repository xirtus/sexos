#!/usr/bin/env python3
"""Compile entropy lease controls against the actual SDK/service fixture.

The enclosing jobs gate runs the full actual entropy fixture once. Its binary
is reused here for two focused positive controls, followed by compiled mutants.
This is host lease/scrubbing coverage, not native liveness or source authority.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
MODULE='servers/sexnet/src/entropy.rs'
FIXTURE='tests/entropy_service.rs'
SITE='pub fn maintain_lease(&mut self,now:u64){self.clock(now);}'
CONTROLS=(
    ('noop-maintenance','pub fn maintain_lease(&mut self,_now:u64){}',
     'uncertain_ready_lease_preserves_full_read_then_expiry_or_rollback_redacts_pending_and_cache'),
    ('deadline-extension',
     'pub fn maintain_lease(&mut self,now:u64){self.deadline=now.saturating_add(TOTAL_TICKS);self.clock(now);}',
     'uncertain_maintenance_and_live_resume_never_extend_original_exact_deadline'),
)


def once(source,old,new):
    if source.count(old)!=1:raise ValueError('missing/ambiguous exact mutation site: '+old)
    return source.replace(old,new,1)


def fixture_source(root,module):
    fixture=root/FIXTURE;substitutions=0
    def path(match):
        nonlocal substitutions
        original=(fixture.parent/match[1]).resolve(strict=True)
        if original==(root/MODULE).resolve():original=module;substitutions+=1
        return '#[path='+json.dumps(str(original))+']'
    source=re.sub(r'#\[path\s*=\s*"([^"\n]+)"\s*\]',path,fixture.read_text())
    if substitutions!=1:raise ValueError('actual fixture must include entropy exactly once')
    for _,_,test in CONTROLS:
        if len(re.findall(r'\bfn\s+'+test+r'\s*\(',source))!=1:
            raise ValueError('missing/ambiguous actual test: '+test)
    return source


def compile_fixture(root,out,name,module):
    path=out/(name+'.rs');path.write_text(fixture_source(root,module));binary=out/name
    result=subprocess.run(['rustc','--edition=2021','--test',str(path),'-o',str(binary)],
                          capture_output=True,text=True,timeout=30)
    (out/(name+'.compile.log')).write_text(result.stdout+result.stderr)
    if result.returncode:raise ValueError(name+': fixture must compile; see '+str(out/(name+'.compile.log')))
    return binary


def run_control(out,name,binary,test,reject=False):
    result=subprocess.run([str(binary),'--exact',test,'--test-threads=1'],
                          capture_output=True,text=True,timeout=30)
    output=result.stdout+result.stderr;(out/(name+'.log')).write_text(output)
    status='FAILED'if reject else'ok'
    if result.returncode!=(101 if reject else 0)or 'running 1 test'not in output or f'test {test} ... {status}'not in output:
        raise ValueError(name+': exact lease control did not '+('reject compiled mutant'if reject else'pass'))


def check(root,out,baseline_binary=None):
    source=(root/MODULE).read_text()
    # Construct both exact changes before executing controls: an absent mutation
    # site is an error, never a successful counterexample.
    mutants=[(name,once(source,SITE,replacement),test)for name,replacement,test in CONTROLS]
    reused=baseline_binary is not None
    if reused:baseline_binary=baseline_binary.resolve(strict=True)
    else:baseline_binary=compile_fixture(root,out,'actual',(root/MODULE).resolve())
    for name,changed,test in mutants:
        run_control(out,'actual-'+name,baseline_binary,test)
        module=out/(name+'-module.rs');module.write_text(changed)
        binary=compile_fixture(root,out,name,module)
        run_control(out,name,binary,test,True)
        print('[entropy.lease.mutant] rejected='+name+' exact='+test)
    result=dict(ok=True,baseline_reused=reused,focused_normal_case_executions=2,
                compiled_mutants_rejected=2,
                scope='actual_entropy_lease_host_not_native_liveness_source_authority_or_BOOT_grants')
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=ROOT)
    parser.add_argument('--baseline-binary',type=Path,help='actual entropy fixture compiled/run by enclosing host gate')
    parser.add_argument('--out',type=Path,help='new retained evidence directory')
    args=parser.parse_args();root=args.root.resolve(strict=True)
    if args.out:
        args.out.mkdir(parents=True,exist_ok=False)
        result=check(root,args.out.resolve(),args.baseline_binary)
    else:
        with tempfile.TemporaryDirectory(prefix='sexos-entropy-lease-')as directory:
            result=check(root,Path(directory),args.baseline_binary)
    print('[entropy.lease.host] PASS '+json.dumps(result,sort_keys=True))


if __name__=='__main__':main()
