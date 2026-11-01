#!/usr/bin/env python3
"""Shell fresh-operation task custody: source wiring and compiled host controls.

This checks the existing held-entry operation policy, not new BOOT grants,
callback/Close protocol task coverage, native input delivery, or guest pixels.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
GUARD_TEST='every_existing_fresh_operation_requires_actual_task_before_payload_admission'
BROWSER_TEST='browser_geometry_and_workspace_controls_preserve_unequal_held_task'
CLEANUP_FUNCTIONS=('shell_retire_app_surface','shell_lifecycle_poll','shell_lifecycle_hint',
                   'shell_close_poll','shell_request_app_close','shell_registration_reply_poll')


def compact(source):return re.sub(r'\s+','',source)


def function(source,name,unsafe=False):
    found=re.findall(r'^'+('unsafe 'if unsafe else'')+r'fn '+name+r'\b.*?^}',source,re.M|re.S)
    if len(found)!=1:raise ValueError('expected one actual '+name)
    return found[0]


def wiring(source):
    calls=re.findall(r'\.allows_operation\(([^()]*)\)',source)
    wanted='msg.caller_task,msg.type_id,[msg.arg0,msg.arg1,msg.arg2]'
    if len(calls)!=2 or any(compact(call)!=wanted for call in calls):
        raise ValueError('both existing operation callsites must forward stamped caller_task')
    refresh=compact(function(source,'workspace_refresh_receive',True))
    if 'e.target==msg.caller_pd&&e.allows_operation('+wanted+')'not in refresh:
        raise ValueError('workspace invalidation must select exact caller PD and task')
    if 'e.target==msg.caller_pd&&!e.allows_operation('+wanted+')'not in compact(source):
        raise ValueError('global operation filter must select exact caller PD and task')
    expected='''fn document_view_sender(e:&app_custody::Entry,msg:&sex_pdx::PdxMessage)->bool{
        e.target==msg.caller_pd && e.task!=0 && e.task==msg.caller_task
        && app_catalog::document_view_index(e.target).is_some()
        && msg.arg0==e.target as u64 && msg.arg1==e.task as u64 && msg.arg2==0
        && !e.dead && !e.retired && !e.close_pending
    }'''
    if compact(function(source,'document_view_sender'))!=compact(expected):
        raise ValueError('document_view_sender must check exact stamped task, payload, family and live custody')
    for name,lookup,effect in [('READY','find','DOCUMENT_VIEWS['),('OPEN_LINEN','any','focus_or_open_linen();')]:
        matches=re.findall(r'^\s*if msg\.type_id==sex_pdx::document_view::'+name+r'\{[^\n]+',source,re.M)
        if len(matches)!=1:raise ValueError('expected one actual '+name+' branch')
        body=compact(matches[0]);guard=lookup+'(|e|document_view_sender(e,&msg))'
        if guard not in body or effect not in body or body.index(guard)>=body.index(effect):
            raise ValueError(name+' effect must follow exact document_view_sender')
    for obsolete in (r'\bBROWSER_INPUT\b',r'\bbrowser_input_poll\b',r'browser_input::(?:Delivery|Outbox)\b'):
        if re.search(obsolete,source):raise ValueError('unused singleton input custody remains: '+obsolete)


def once(source,old,new):
    if source.count(old)!=1:raise ValueError('missing/ambiguous exact mutation site: '+old)
    return source.replace(old,new,1)


def fixture_source(root,module):
    fixture=root/'tests/shell_app_cleanup.rs';substitutions=0
    def path(match):
        nonlocal substitutions
        original=(fixture.parent/match[1]).resolve(strict=True)
        if original==(root/'servers/silk-shell/src/app_custody.rs').resolve():original=module;substitutions+=1
        return '#[path='+json.dumps(str(original))+']'
    value=re.sub(r'#\[path\s*=\s*"([^"\n]+)"\s*\]',path,fixture.read_text())
    if substitutions!=1:raise ValueError('fixture must compile actual custody module once')
    return value


def compile_fixture(root,out,name,module,cleanup):
    source=out/(name+'.rs');source.write_text(fixture_source(root,module));binary=out/name
    result=subprocess.run(['rustc','--edition=2021','--test',str(source),'-o',str(binary)],
                          env={**os.environ,'SHELL_CLEANUP_SOURCE':str(cleanup)},capture_output=True,text=True,timeout=30)
    (out/(name+'.compile.log')).write_text(result.stdout+result.stderr)
    if result.returncode:raise ValueError(name+': actual/mutant must compile; see '+str(out/(name+'.compile.log')))
    return binary


def control(out,name,binary,test,reject=False):
    result=subprocess.run([str(binary),'--exact',test,'--test-threads=1'],capture_output=True,text=True,timeout=30)
    output=result.stdout+result.stderr;(out/(name+'.log')).write_text(output)
    expected='FAILED'if reject else'ok'
    if result.returncode!=(101 if reject else 0)or 'running 1 test'not in output or f'test {test} ... {expected}'not in output:
        raise ValueError(name+': exact named behavioral control did not '+('reject mutant'if reject else'pass'))


def check(root,out,cleanup=None,baseline=None):
    source=(root/'servers/silk-shell/src/main.rs').read_text();wiring(source)
    # Counterexample construction is outside the rejection handler: a missing
    # mutation site is a failed check, never evidence of a rejected regression.
    changes=(('e.allows_operation(msg.caller_task,msg.type_id,[msg.arg0,msg.arg1,msg.arg2])',
              'e.allows_operation(msg.caller_pd,msg.type_id,[msg.arg0,msg.arg1,msg.arg2])'),
             ('e.allows_operation(msg.caller_task,msg.type_id, [msg.arg0,msg.arg1,msg.arg2])',
              'e.allows_operation(msg.caller_pd,msg.type_id, [msg.arg0,msg.arg1,msg.arg2])'),
             (' && e.task==msg.caller_task',''),
             ('any(|e|document_view_sender(e,&msg))','any(|e|true)'))
    for old,new in changes:
        changed=once(source,old,new)
        try:wiring(changed)
        except ValueError:pass
        else:raise ValueError('unbound main-wiring control survived: '+old)
    try:wiring(source+'\nstatic mut BROWSER_INPUT:u32=0;\n')
    except ValueError:pass
    else:raise ValueError('obsolete singleton control survived')
    if cleanup is None:
        cleanup=out/'cleanup-source.rs'
        cleanup.write_text('\n'.join(function(source,name,True)for name in CLEANUP_FUNCTIONS))
    else:cleanup=cleanup.resolve(strict=True)
    module=root/'servers/silk-shell/src/app_custody.rs';actual=module.read_text()
    reused=baseline is not None
    if baseline is None:baseline=compile_fixture(root,out,'actual',module,cleanup)
    else:baseline=baseline.resolve(strict=True)
    # The enclosing gate already runs all22 normal cases. Reuse its binary,
    # running only the two exact positive controls needed by these mutants.
    control(out,'actual-task-control',baseline,GUARD_TEST)
    control(out,'actual-browser-control',baseline,BROWSER_TEST)
    task_mutant=once(actual,'caller_task == 0 || caller_task != self.task || ','')
    lines=[line for line in actual.splitlines()if '0x274 | 0x279 =>'in line]
    if len(lines)!=1:raise ValueError('expected one actual Browser operation arm')
    old=lines[0];restored=once(old,' && args ==',' && (!crate::app_catalog::is_browser(self.target)||self.task==self.target) && args ==')
    browser_mutant=once(actual,old,restored)
    for name,changed,test in [('missing-stamped-task',task_mutant,GUARD_TEST),('old-browser-task-equals-pd',browser_mutant,BROWSER_TEST)]:
        path=out/(name+'-module.rs');path.write_text(changed)
        binary=compile_fixture(root,out,name,path,cleanup)
        control(out,name,binary,test,True)
    result=dict(ok=True,baseline_reused=reused,focused_baseline_tests=2,compiled_mutants_rejected=2,
                wiring_controls_rejected=len(changes)+1,
                scope='existing_fresh_operation_custody_host_and_source_wiring_not_callback_Close_coverage_or_future_BOOT_grants')
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=ROOT)
    parser.add_argument('--cleanup-source',type=Path,help='actual cleanup extraction already made by enclosing host gate')
    parser.add_argument('--baseline-binary',type=Path,help='actual normal fixture just compiled/run by enclosing host gate')
    parser.add_argument('--out',type=Path,help='new retained evidence directory')
    args=parser.parse_args();root=args.root.resolve(strict=True)
    if args.out:
        args.out.mkdir(parents=True,exist_ok=False);result=check(root,args.out.resolve(),args.cleanup_source,args.baseline_binary)
    else:
        with tempfile.TemporaryDirectory(prefix='sexos-shell-input-custody-')as directory:
            result=check(root,Path(directory),args.cleanup_source,args.baseline_binary)
    print('[shell.input.custody] PASS '+json.dumps(result,sort_keys=True))


if __name__=='__main__':main()
