#!/usr/bin/env python3
"""Compile the actual denial-only outbox fixture and retained-fence/layout controls.

This is host protocol/composition coverage, including actual SDK and Machine
unit-case executions. It does not prove native dispatch, source authority,
BOOT grants, network traffic, or guest pixels.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
MODULE='servers/sexnet/src/source_denial.rs'
FIXTURE='tests/source_denial.rs'
REFERENCE='tests/fixtures/source_denial_r161.rs'
REFERENCE_SHA256='b29e2c8c6ec4431fee287c67964ffc8d8fef717efbbfe623d0b63a7a5c94dc2f'
EQUIVALENCE='tests/source_denial_equivalence.rs.inc'


def once(source,old,new):
    if source.count(old)!=1:raise ValueError('missing/ambiguous exact mutation site: '+old)
    return source.replace(old,new,1)


def fixture_source(root,module,catalog=None):
    path=root/FIXTURE;substitutions=0
    def replace(match):
        nonlocal substitutions
        original=(path.parent/match[1]).resolve(strict=True)
        if original==(root/MODULE).resolve():original=module;substitutions+=1
        if catalog is not None and original==(root/'crates/sex-pdx/src/app_catalog.rs').resolve():original=catalog
        return '#[path='+json.dumps(str(original))+']'
    source=re.sub(r'#\[path\s*=\s*"([^"\n]+)"\s*\]',replace,path.read_text())
    if substitutions!=1:raise ValueError('actual fixture must include source_denial exactly once')
    return source


def compile_fixture(root,out,name,module,library,catalog=None,equivalence=False):
    source=fixture_source(root,module,catalog)
    if equivalence:
        reference=pinned_reference(root)
        source+='\n#[path='+json.dumps(str(reference))+']mod reference;\n'+(root/EQUIVALENCE).read_text()
    fixture=out/(name+'.rs');fixture.write_text(source);binary=out/name
    command=['rustc','--edition=2021']+(['-O']if equivalence else[])
    result=subprocess.run(command+['--extern','sex_trust_config='+str(library),
                           '--test',str(fixture),'-o',str(binary)],capture_output=True,text=True,timeout=60)
    (out/(name+'.compile.log')).write_text(result.stdout+result.stderr)
    if result.returncode:raise ValueError(name+': actual/mutant must compile; see '+str(out/(name+'.compile.log')))
    if equivalence:pinned_reference(root)
    return binary


def run_fixture(out,name,binary,exact=None,reject=False):
    args=[str(binary),'--test-threads=1']
    if exact:
        args+=['--exact',exact]
        if not reject:args+=['--show-output']
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


def pinned_reference(root):
    path=root/REFERENCE
    if hashlib.sha256(path.read_bytes()).hexdigest()!=REFERENCE_SHA256:
        raise ValueError('previous48-byte reference differs from pinned Round161 bytes')
    return path.resolve(strict=True)


def storage_measurement(output):
    records=re.findall(r'\[source.denial.host.storage\] bytes=(\d+) slots=(\d+) owners_per_kind=(\d+)',output)
    if len(records)!=1:raise ValueError('missing/ambiguous actual outbox storage measurement')
    return dict(zip(('bytes','slots','owners_per_kind'),map(int,records[0])))


def equivalence_checks(root,out,library,catalog64,sensitivity_module):
    """Compare actual APIs/typed words; facades change sizing, never authority."""
    reference=pinned_reference(root)
    cases=[]
    for owners,test in ((2,'differential_short_histories'),(28,'differential_long_traces'),(64,'differential_long_traces')):
        catalog=catalog64 if owners==64 else out/('equivalence-count'+str(owners)+'-catalog.rs')
        if owners!=64:catalog.write_text('pub const COUNT:usize='+str(owners)+';\n')
        name='equivalence-count'+str(owners)
        binary=compile_fixture(root,out,name,(root/MODULE).resolve(),library,catalog,equivalence=True)
        output=run_fixture(out,name,binary,exact=test)
        if owners==2:
            records=re.findall(r'^\[denial.differential.short\] owners=(\d+) histories=(\d+) operations=(\d+) depth=(\d+) alphabet=(\d+) protocols=(\d+)$',output,re.M)
            expected=(2,115248,460992,4,14,3)
            if len(records)!=1 or tuple(map(int,records[0]))!=expected:raise ValueError('actual short equivalence counters differ')
            cases.append(dict(name=name,test=test,owners=2,histories=115248,operations=460992,depth=4,alphabet=14,protocols=3))
        else:
            records=re.findall(r'^\[denial.differential.long\] owners=(\d+) seeds=(\d+) random_operations=(\d+) total_operations=(\d+)$',output,re.M)
            expected=(owners,3,300000,300000+27*owners)
            if len(records)!=1 or tuple(map(int,records[0]))!=expected:raise ValueError('actual long equivalence counters differ')
            cases.append(dict(name=name,test=test,owners=owners,seeds=3,random_operations=300000,operations=300000+27*owners))
    # This reuses an existing source defect through the independent comparator:
    # a comparator which ignores the candidate or performs no comparisons must
    # fail this gate, even when all independent layout tests are still useful.
    name='equivalence-prefloor-false';catalog=out/'equivalence-count2-catalog.rs'
    binary=compile_fixture(root,out,name,sensitivity_module,library,catalog,equivalence=True)
    run_fixture(out,name,binary,exact='differential_short_histories',reject=True)
    pinned_reference(root)
    result=dict(case_executions=3,cases=cases,short_histories=115248,short_operations=460992,
                long_random_operations=600000,long_operations=602484,runtime_comparator_controls_rejected=1,
                reference=REFERENCE,reference_sha256=REFERENCE_SHA256,
                comparator=EQUIVALENCE,comparator_sha256=hashlib.sha256((root/EQUIVALENCE).read_bytes()).hexdigest(),
                scope='finite API/typed-word compatibility against pinned prior implementation; not formal all-u32-history, BOOT, native authority, concurrency or SMP proof')
    print('[source.denial.equivalence] PASS '+json.dumps(result,sort_keys=True))
    return result


def check(root,out):
    library=out/'libsex_trust_config.rlib'
    result=subprocess.run(['rustc','--edition=2021','--crate-name','sex_trust_config','--crate-type','rlib',
                           str(root/'crates/sex-trust-config/src/lib.rs'),'-o',str(library)],capture_output=True,text=True,timeout=30)
    (out/'trust-config.compile.log').write_text(result.stdout+result.stderr)
    if result.returncode:raise ValueError('actual trust configuration must compile')
    source=(root/MODULE).read_text()
    actual=compile_fixture(root,out,'actual',(root/MODULE).resolve(),library)
    storage=storage_measurement(run_fixture(out,'actual',actual))
    if storage['slots']!=3*storage['owners_per_kind']or storage['bytes']!=16*storage['slots']+8 or storage['bytes']>=4096:
        raise ValueError('actual dense denial layout/bounded budget differs')
    # This facade is a storage/host-model boundary, not a BOOT reservation.
    catalog=out/'capacity64-catalog.rs';catalog.write_text('pub const COUNT:usize=64;\n')
    future=compile_fixture(root,out,'capacity64',(root/MODULE).resolve(),library,catalog)
    future_storage=storage_measurement(run_fixture(out,'capacity64',future))
    if future_storage!={'bytes':3080,'slots':192,'owners_per_kind':64}:
        raise ValueError('prospective64-owner storage differs')
    controls=(
        ('pending-floor-before-rejection',
         'if slot.pending_seq != 0 { return slot.pending_seq == seq; }',
         'if seq > slot.cached_seq { slot.cached_seq = seq; } if slot.pending_seq != 0 { return slot.pending_seq == seq; }',
         'full_and_duplicate_keep_exact_receipt_while_different_pending_sequence_backpressures'),
        ('lowered-cache-floor','if seq > slot.cached_seq { slot.cached_seq = seq; }','slot.cached_seq = seq;',
         'stale_replies_never_lower_or_replace_latest_denied_cache_including_last_sequence'),
        ('evicted-cached-owner','slot.owner.pd == 0','slot.pending_seq == 0',
         'capacity_is_per_kind_and_never_evicts_pending_or_cached_owner_fences'),
        ('stale-as-denied','if slot.pending_seq < slot.cached_seq { kind.stale(slot.pending_seq) }',
         'if slot.pending_seq < slot.cached_seq { kind.denied(slot.pending_seq) }',
         'stale_replies_never_lower_or_replace_latest_denied_cache_including_last_sequence'),
        ('duplicate-as-stale','let word = if slot.pending_seq < slot.cached_seq',
         'let word = if slot.pending_seq <= slot.cached_seq',
         'full_and_duplicate_keep_exact_receipt_while_different_pending_sequence_backpressures'),
        ('accepted-clears-floor','self.slots[index].pending_seq = 0;',
         'self.slots[index].pending_seq = 0; self.slots[index].cached_seq = 0;',
         'stale_replies_never_lower_or_replace_latest_denied_cache_including_last_sequence'),
        ('wrong-kind-partition','Kind::from_index(index / OWNERS_PER_KIND)',
         'Kind::from_index(index % 3)',
         'retirement_is_exact_and_clears_only_that_identity_across_all_partitions'),
    )
    for name,old,new,test in controls:
        changed=once(source,old,new)
        module=out/(name+'-module.rs');module.write_text(changed)
        binary=compile_fixture(root,out,name,module,library)
        run_fixture(out,name,binary,exact=test,reject=True)
        print('[source.denial.mutant] rejected='+name+' exact='+test)
    name='legacy48-capacity64';reference=pinned_reference(root)
    binary=compile_fixture(root,out,name,reference,library,catalog)
    test='denial_storage_budget_is_finite_and_contains_no_machine_backing'
    old_storage=storage_measurement(run_fixture(out,name,binary,exact=test,reject=True))
    if old_storage!={'bytes':9224,'slots':192,'owners_per_kind':64} or old_storage['bytes']<4096:
        raise ValueError('compiled previous48-byte layout did not overflow64-owner budget')
    print('[source.denial.mutant] rejected='+name+' exact='+test+' bytes='+str(old_storage['bytes']))
    equivalence=equivalence_checks(root,out,library,catalog,out/'pending-floor-before-rejection-module.rs')
    result=dict(ok=True,normal_case_executions=86,compiled_mutants_rejected=len(controls)+1,
                storage=storage,prospective64_storage=future_storage,legacy64_storage=old_storage,
                reference_sha256=REFERENCE_SHA256,equivalence=equivalence,
                scope='actual_denial_outbox_typed_SDK_and_Machine_host_composition; prospective64metadata_not_BOOT_grants_or_native_source_authority')
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
