#!/usr/bin/env python3
"""Compile the actual denial-only outbox fixture and retained-fence/layout controls.

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
    else:args+=['--show-output']
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
    return output


def legacy_cache(source):
    """Restore the former full-word cache, preserving its receipt semantics."""
    source=once(source,'cached_seq: Option<u32>','cached: Option<Receipt>')
    source=once(source,'cached_seq: None','cached: None')
    source=once(source,'''let receipt = Receipt { seq, word: match slot.cached_seq {
            Some(cached) if seq < cached => kind.stale(seq),
            _ => kind.denied(seq),
        } };''','''let receipt = match slot.cached {
            Some(cached) if seq == cached.seq => cached,
            Some(cached) if seq < cached.seq => Receipt { seq, word: kind.stale(seq) },
            _ => Receipt { seq, word: kind.denied(seq) },
        };''')
    return once(source,'if slot.cached_seq.is_none_or(|cached| seq > cached) { slot.cached_seq = Some(seq); }',
                'if slot.cached.is_none_or(|cached| seq > cached.seq) { slot.cached = Some(receipt); }')


def storage_measurement(output):
    records=re.findall(r'\[source.denial.host.storage\] bytes=(\d+) slots=(\d+) owners_per_kind=(\d+)',output)
    if len(records)!=1:raise ValueError('missing/ambiguous actual outbox storage measurement')
    return dict(zip(('bytes','slots','owners_per_kind'),map(int,records[0])))


def check(root,out):
    library=out/'libsex_trust_config.rlib'
    result=subprocess.run(['rustc','--edition=2021','--crate-name','sex_trust_config','--crate-type','rlib',
                           str(root/'crates/sex-trust-config/src/lib.rs'),'-o',str(library)],capture_output=True,text=True,timeout=30)
    (out/'trust-config.compile.log').write_text(result.stdout+result.stderr)
    if result.returncode:raise ValueError('actual trust configuration must compile')
    source=(root/MODULE).read_text()
    actual=compile_fixture(root,out,'actual',(root/MODULE).resolve(),library)
    storage=storage_measurement(run_fixture(out,'actual',actual))
    if storage['slots']!=3*storage['owners_per_kind']or storage['bytes']!=48*storage['slots']+8 or storage['bytes']>=4096:
        raise ValueError('actual compact denial layout/bounded budget differs')
    controls=(
        ('lowered-cache-floor','if slot.cached_seq.is_none_or(|cached| seq > cached)','if true',
         'stale_replies_never_lower_or_replace_latest_denied_cache_including_last_sequence'),
        ('evicted-cached-owner','slot.owner.is_none()','slot.pending.is_none()',
         'capacity_is_per_kind_and_never_evicts_pending_or_cached_owner_fences'),
        ('stale-as-denied','Some(cached) if seq < cached => kind.stale(seq),',
         'Some(cached) if seq < cached => kind.denied(seq),',
         'stale_replies_never_lower_or_replace_latest_denied_cache_including_last_sequence'),
        ('duplicate-as-stale','Some(cached) if seq < cached => kind.stale(seq),',
         'Some(cached) if seq <= cached => kind.stale(seq),',
         'full_and_duplicate_keep_exact_receipt_while_different_pending_sequence_backpressures'),
    )
    for name,old,new,test in controls:
        changed=once(source,old,new)
        module=out/(name+'-module.rs');module.write_text(changed)
        binary=compile_fixture(root,out,name,module,library)
        run_fixture(out,name,binary,exact=test,reject=True)
        print('[source.denial.mutant] rejected='+name+' exact='+test)
    name='legacy-full-word-cache';module=out/(name+'-module.rs');module.write_text(legacy_cache(source))
    binary=compile_fixture(root,out,name,module,library)
    test='denial_storage_budget_is_finite_and_contains_no_machine_backing'
    old_storage=storage_measurement(run_fixture(out,name,binary,exact=test,reject=True))
    if (old_storage['slots'],old_storage['owners_per_kind'])!=(storage['slots'],storage['owners_per_kind'])or old_storage['bytes']!=64*storage['slots']+8:
        raise ValueError('compiled legacy-cache control did not restore its actual 64-byte slots')
    print('[source.denial.mutant] rejected='+name+' exact='+test+' bytes='+str(old_storage['bytes']))
    result=dict(ok=True,normal_case_executions=43,compiled_mutants_rejected=len(controls)+1,storage=storage,legacy_storage=old_storage,
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
