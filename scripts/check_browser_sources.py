#!/usr/bin/env python3
"""Check actual Browser source plans and explicit future host metadata controls.

Future rows exist only in generated host catalogs. Plan data creates no grants,
resolves no task, and proves no native source access or BOOT table readback.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT=Path(__file__).resolve().parents[1]
MODULE='crates/sex-pdx/src/browser_sources.rs'
CATALOG='crates/sex-pdx/src/app_catalog.rs'
FIXTURE='tests/browser_sources.rs'
FUTURE_ROWS='''
    Incarnation { app: App::Browser, ordinal: 1, target: 30, surface: 343, shell_surface: None, lifecycle_slot: 154, close_slot: 155, activation_slot: Some(156), route_slot: 157, children: &[] },
    Incarnation { app: App::Browser, ordinal: 2, target: 31, surface: 344, shell_surface: None, lifecycle_slot: 158, close_slot: 159, activation_slot: Some(160), route_slot: 161, children: &[] },'''


def once(source,old,new):
    if source.count(old)!=1:raise ValueError('missing/ambiguous exact control site: '+old)
    return source.replace(old,new,1)


def descriptive_source(source,catalog):
    code=re.sub(r'//[^\n]*','',source)
    fields=re.findall(r'pub struct Plan\s*\{([^}]+)\}',code,re.S)
    if len(fields)!=1:raise ValueError('expected unique descriptive Plan')
    expected=['target:u32','surface:u64','lifecycle_slot:usize',
              'entropy_slot:u64','clock_slot:u64','trust_slot:u64']
    if [re.sub(r'\s+','',item)for item in fields[0].split(',')if item.strip()]!=expected:
        raise ValueError('Plan must retain only private task-free metadata fields')
    methods=re.findall(r'pub\s+(?:const\s+)?fn\s+(\w+)\s*\(',code)
    if set(methods)!={item.split(':')[0]for item in expected}|{'by_index','by_target'}or len(methods)!=8:
        raise ValueError('unexpected public Plan constructor/query/provider API')
    if re.search(r'\b(?:unsafe|Grant|invoke|extern|task|pdx_call_checked)\b',code):
        raise ValueError('plan must not contain runtime authority or task resolution')
    if 'app_catalog::count(App::Browser)'not in code or 'pub const PLANS:[Plan;BROWSER_COUNT]=build();'not in code:
        raise ValueError('plan budget must derive from the actual catalog')
    rows=re.findall(r'Incarnation\s*\{([^}]*app:\s*App::Browser[^}]*)\}',catalog,re.S)
    if len(rows)!=1:raise ValueError('this cohort must keep exactly one actual Browser row')
    for key,value in [('ordinal',0),('target',15),('surface',300),('lifecycle_slot',40)]:
        found=re.findall(r'\b'+key+r':\s*(\d+)\b',rows[0])
        if found!=[str(value)]:raise ValueError('actual Browser association changed: '+key)


def append_rows(catalog,rows,count):
    # Copy the complete actual catalog; only this generated host copy gains rows.
    source='\n'.join(line for line in catalog.splitlines()if not line.startswith('//!'))+'\n'
    found=list(re.finditer(r'pub const INCARNATIONS:\s*\[Incarnation;\s*(\d+)\]\s*=\s*\[',source))
    if len(found)!=1:raise ValueError('expected actual unique incarnation array')
    match=found[0];end=source.index('\n];',match.end())
    source=source[:end]+rows+source[end:]
    return source[:match.start(1)]+str(int(match[1])+count)+source[match.end(1):]


def fixture_source(root,module):
    fixture=root/FIXTURE;substitutions=0
    def path(match):
        nonlocal substitutions
        original=(fixture.parent/match[1]).resolve(strict=True)
        if original==(root/MODULE).resolve():original=module;substitutions+=1
        return '#[path='+json.dumps(str(original))+']'
    result=re.sub(r'#\[path\s*=\s*"([^"\n]+)"\s*\]',path,fixture.read_text())
    if substitutions!=1:raise ValueError('fixture must include actual source-plan module exactly once')
    return result


def compile_source(out,name,source,catalog=None,test=True):
    path=out/(name+'.rs');path.write_text(source);binary=out/name
    args=['rustc','--edition=2021'];env=os.environ.copy()
    if test:args+=['--test']
    if catalog:
        args+=['--cfg','browser_source_catalog'];env['BROWSER_SOURCE_CATALOG']=str(catalog)
    result=subprocess.run(args+[str(path),'-o',str(binary)],capture_output=True,text=True,env=env,timeout=30)
    (out/(name+'.compile.log')).write_text(result.stdout+result.stderr)
    return binary,result


def normal(root,out,name,module,catalog=None):
    binary,compiled=compile_source(out,name,fixture_source(root,module),catalog)
    if compiled.returncode:raise ValueError(name+': actual fixture must compile')
    result=subprocess.run([str(binary),'--test-threads=1'],capture_output=True,text=True,timeout=30)
    output=result.stdout+result.stderr;(out/(name+'.log')).write_text(output)
    if result.returncode or 'test result: ok. 13 passed; 0 failed;'not in output:
        raise ValueError(name+': expected13 actual fixture/SDK case executions')
    print(output,end='')


def rejected_const(root,out,name,module,catalog,reason):
    _,result=compile_source(out,name,fixture_source(root,module),catalog)
    required=('error[E0080]','evaluation of `browser_sources::PLANS` failed',
              'evaluation panicked: '+reason)
    if result.returncode==0 or any(part not in result.stderr for part in required):
        raise ValueError(name+': expected exact const plan rejection: '+reason)
    print('[browser.sources.const] rejected='+name+' reason='+reason)


def check(root,out):
    module=(root/MODULE).resolve(strict=True);source=module.read_text()
    catalog=(root/CATALOG).read_text();descriptive_source(source,catalog)
    future=append_rows(catalog,FUTURE_ROWS,2);future_path=out/'future-catalog.rs';future_path.write_text(future)
    normal(root,out,'actual',module);normal(root,out,'future',module,future_path)
    fourth='''
    Incarnation { app: App::Browser, ordinal: 3, target: 32, surface: 345, shell_surface: None, lifecycle_slot: 162, close_slot: 163, activation_slot: Some(164), route_slot: 165, children: &[] },'''
    controls=(
        ('unknown-ordinal',append_rows(catalog,FUTURE_ROWS+fourth,3),'unsupported Browser source-plan ordinal'),
        ('duplicate-ordinal',once(future,'ordinal: 2, target: 31','ordinal: 1, target: 31'),'Browser source-plan ordinal order'),
        ('target-alias',once(future,'ordinal: 2, target: 31','ordinal: 2, target: 30'),'Browser target aliases another catalog row'),
        ('surface-alias',once(future,'target: 31, surface: 344','target: 31, surface: 343'),'Browser surface aliases another catalog row'),
        ('source-lifecycle-alias',once(future,'surface: 344, shell_surface: None, lifecycle_slot: 158','surface: 344, shell_surface: None, lifecycle_slot: 70'),'source/lifecycle slot collision'),
        ('high-lifecycle-slot',once(future,'surface: 344, shell_surface: None, lifecycle_slot: 158','surface: 344, shell_surface: None, lifecycle_slot: 0x10000009e'),'Browser lifecycle slot range'),
    )
    for name,changed,reason in controls:
        path=out/(name+'-catalog.rs');path.write_text(changed)
        rejected_const(root,out,name,module,path,reason)
    mutations=(
        ('cross-plan-slot-alias','1=>[70,71,72]','1=>[60,71,72]','source plans alias'),
        ('within-triple-slot-alias','0=>[60,61,62]','0=>[60,60,62]','source triple aliases'),
        ('zero-slot','0=>[60,61,62]','0=>[0,61,62]','Browser source slot range'),
        ('high-slot-truncation','1=>[70,71,72]','1=>[0x100000046,71,72]','Browser source slot range'),
    )
    for name,old,new,reason in mutations:
        changed=once(source,old,new);path=out/(name+'-module.rs');path.write_text(changed)
        rejected_const(root,out,name,path,future_path,reason)
    privacy='''#![allow(dead_code)]
#[path=CATALOG]mod app_catalog;
#[path=MODULE]mod browser_sources;
fn main(){let _=browser_sources::Plan{target:15,surface:300,lifecycle_slot:40,entropy_slot:60,clock_slot:61,trust_slot:62};}
'''.replace('CATALOG',json.dumps(str(root/CATALOG))).replace('MODULE',json.dumps(str(module)))
    _,result=compile_source(out,'private-constructor',privacy,test=False)
    if result.returncode==0 or 'error[E0451]'not in result.stderr or 'of struct `Plan` are private'not in result.stderr:
        raise ValueError('external Plan forging must fail specifically at private fields')
    print('[browser.sources.private] rejected=external-field-construction diagnostic=E0451')
    result=dict(ok=True,actual_catalog_case_executions=13,future_catalog_case_executions=13,
                const_plan_controls_rejected=10,private_constructor_rejected=True,
                actual_browser_plans=1,future_host_browser_plans=3,plan_task_fields=0,
                scope='host_descriptive_metadata_and_pure_mechanism_not_future_BOOT_grants_or_native_authority')
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=ROOT)
    parser.add_argument('--out',type=Path,help='new retained evidence directory')
    args=parser.parse_args();root=args.root.resolve(strict=True)
    if args.out:
        args.out.mkdir(parents=True,exist_ok=False);result=check(root,args.out.resolve())
    else:
        with tempfile.TemporaryDirectory(prefix='sexos-browser-sources-')as directory:result=check(root,Path(directory))
    print('[browser.sources.host] PASS '+json.dumps(result,sort_keys=True))


if __name__=='__main__':main()
