#!/usr/bin/env python3
"""Compile held Browser metadata/transport behavior; no runtime grant creation."""
import argparse,json,os,re,subprocess,tempfile
from pathlib import Path

def paths(text,root):
 return re.sub(r'\.\./(?:crates|apps)/[^"\n]+',lambda m:str(root/m[0][3:]),text)
def run(root,out,name,fixture,env=None,cfg=(),rejected=False):
 source=out/(name+'.rs');source.write_text(fixture);binary=out/name
 subprocess.run(['rustc','--edition=2021','--test',*cfg,str(source),'-o',str(binary)],check=True,env={**os.environ,**(env or{})})
 result=subprocess.run([str(binary),'--test-threads=1'],text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
 (out/(name+'.log')).write_text(result.stdout)
 if result.returncode!=(101 if rejected else 0):raise RuntimeError(name+': '+result.stdout)
 if rejected:print('[browser.identity.mutant] rejected='+name)
 else:print(result.stdout,end='')

def check(root,out):
 main=(root/'apps/kaleidoscope/src/main.rs').read_text();start=main.index('    fn new(_pdx: u32) -> Self {');body=main[start:]
 query=body.index('let identity=sex_pdx::app_lifecycle::catalog(');select=body.index('let selected=incarnation::select(identity)');bind=body.index('BOOT_BROWSER_IDENTITY=Some(selected)');effects=body.index('sex_pdx::device_network_config(')
 assert query<select<bind<effects<body.index('paging_probe::run()')<body.index('service_call::request(')
 assert 'if selected.is_original() { paging_probe::run(); }'in body
 assert 'identity.is_subject(15,KALEIDO_SURFACE_ID,3)'not in main
 assert 'unsafe { BOOT_BROWSER_IDENTITY }.expect("BROWSER_HELD_IDENTITY_BEFORE_SERVICE")'in main
 fixture=paths((root/'tests/browser_incarnation.rs').read_text(),root);run(root,out,'actual',fixture)
 catalog=(root/'crates/sex-pdx/src/app_catalog.rs').read_text();catalog='\n'.join(line for line in catalog.splitlines()if not line.startswith('//!'))+'\n';match=re.search(r'pub const INCARNATIONS: \[Incarnation; (\d+)\] = \[',catalog);assert match;end=catalog.index('\n];',match.end())
 rows='''
    Incarnation { app: App::Browser, ordinal: 1, target: 30, surface: 343, shell_surface: None, lifecycle_slot: 154, close_slot: 155, activation_slot: Some(156), route_slot: 157, children: &[] },
    Incarnation { app: App::Browser, ordinal: 2, target: 31, surface: 344, shell_surface: None, lifecycle_slot: 158, close_slot: 159, activation_slot: Some(160), route_slot: 161, children: &[] },'''
 additions=[line for line in rows.splitlines()if line.strip()and line.strip()not in {v.strip()for v in catalog.splitlines()}];count=len(additions)
 catalog=catalog[:end]+('\n'+'\n'.join(additions)if additions else '')+catalog[end:];catalog=catalog[:match.start(1)]+str(int(match[1])+count)+catalog[match.end(1):];cp=out/'future-catalog.rs';cp.write_text(catalog)
 run(root,out,'future',fixture,{'BROWSER_INCARNATION_CATALOG':str(cp)},['--cfg','browser_incarnation_catalog'])
 helper=root/'apps/kaleidoscope/src/incarnation.rs';text=helper.read_text()
 for name,old,new in [('task-equals-domain','task: c.task_id','task: row.target'),('missing-held-subject',' || !c.is_subject(row.target, row.surface, 3)','')]:
  assert text.count(old)==1;mutant=out/(name+'-helper.rs');mutant.write_text(text.replace(old,new));run(root,out,name,fixture.replace(str(helper),str(mutant)),rejected=True)
 transport=paths((root/'tests/browser_service_call.rs').read_text(),root);run(root,out,'transport',transport)
 service=root/'apps/kaleidoscope/src/service_call.rs';text=service.read_text();assert text.count('identity.task()')==2;mutant=out/'transport-task-mutant.rs';mutant.write_text(text.replace('identity.task()','identity.target()'));run(root,out,'transport-task-equals-domain',transport.replace(str(service),str(mutant)),rejected=True)
 print('[browser.identity.host] PASS actual_metadata=5 future_metadata=5 transport=5 compiled_mutants=3 native_unequal_allocator=0')

def main():
 p=argparse.ArgumentParser();p.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);p.add_argument('--out',type=Path);a=p.parse_args();root=a.root.resolve()
 if a.out:a.out.mkdir(parents=True,exist_ok=False);check(root,a.out.resolve())
 else:
  with tempfile.TemporaryDirectory(prefix='sexos-browser-identity-')as out:check(root,Path(out))
if __name__=='__main__':main()
