#!/usr/bin/env python3
"""Compiled policy expansion, with explicit held-grant/registry/backend doubles."""
import json,os,re,subprocess,tempfile
from pathlib import Path

def main():
 root=Path(__file__).resolve().parents[1];catalog=(root/'crates/sex-pdx/src/app_catalog.rs').read_text();m=re.search(r'pub const INCARNATIONS: \[Incarnation; (\d+)\] = \[',catalog);assert m;end=catalog.index('\n];',m.end())
 extra='''
 quil(8,100,400,None,210,211,Some(212),213),
 Incarnation{app:App::Spindle,ordinal:2,target:101,surface:401,shell_surface:Some(402),lifecycle_slot:214,close_slot:215,activation_slot:Some(216),route_slot:217,children:&[403,404,405]},
 Incarnation{app:App::Browser,ordinal:1,target:102,surface:406,shell_surface:None,lifecycle_slot:218,close_slot:219,activation_slot:Some(220),route_slot:221,children:&[]},'''
 catalog=catalog[:end]+extra+catalog[end:];catalog=catalog[:m.start(1)]+str(int(m[1])+3)+catalog[m.end(1):]
 with tempfile.TemporaryDirectory(prefix='sexos-incarnation-policy-')as temp:
  out=Path(temp);cp=out/'catalog.rs';cp.write_text(catalog);source=(root/'servers/silk-shell/src/main.rs').read_text();parts=[]
  for name in ['quil_activation_edge','app_activation_info','app_reserve_remaining','app_launcher_status','app_route_slot']:
   a=source.index('fn '+name+'(');i=source.index('{',a)+1;depth=1
   while depth:depth+=(source[i]=='{')-(source[i]=='}');i+=1
   parts.append(source[a:i])
  shell=out/'shell.rs';shell.write_text('\n'.join(parts));fixture=(root/'tests/incarnation_expansion.rs').read_text()
  fixture=re.sub(r'#\[path="(\.\./[^"]+)"\]',lambda m:'#[path='+json.dumps(str(cp if m[1].endswith('/app_catalog.rs')else(root/m[1][3:]).resolve()))+']',fixture)
  test=out/'test.rs';test.write_text(fixture);binary=out/'tests';subprocess.run(['rustc','--edition=2021','--test',str(test),'-o',str(binary)],check=True,env={**os.environ,'EXPANDED_SHELL_POLICY':str(shell)});subprocess.run([str(binary),'--test-threads=1'],check=True)
  # Compiled regressions must fail: stopping at the former third incarnation
  # or applying Browser restrictions only to the original hard-coded caller.
  original_shell=shell.read_text()
  for name,changed in [('old_chain',original_shell.replace('row.app==app&&row.ordinal!=0','row.app==app&&row.ordinal!=0&&row.target!=100'))]:
   assert changed!=original_shell;shell.write_text(changed)
   mutant=out/(name+'-test');subprocess.run(['rustc','--edition=2021','--test',str(test),'-o',str(mutant)],check=True,env={**os.environ,'EXPANDED_SHELL_POLICY':str(shell)})
   result=subprocess.run([str(mutant),'--test-threads=1'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
   if result.returncode!=101:raise RuntimeError('old-chain regression survived: '+result.stdout)
   print('[incarnation.policy.control] old_chain_compile_rejected_by_tests=1')
  shell.write_text(original_shell)
  doc=(root/'servers/sexfiles/src/document.rs').read_text();changed=doc.replace('if app_catalog::is_browser(caller){','if caller==15{');assert changed!=doc
  d=out/'document-mutant.rs';d.write_text(changed);old=json.dumps(str(root/'servers/sexfiles/src/document.rs'));new=json.dumps(str(d));assert old in fixture
  test.write_text(fixture.replace(old,new));mutant=out/'browser-test';subprocess.run(['rustc','--edition=2021','--test',str(test),'-o',str(mutant)],check=True,env={**os.environ,'EXPANDED_SHELL_POLICY':str(shell)})
  result=subprocess.run([str(mutant),'--test-threads=1'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
  if result.returncode!=101:raise RuntimeError('original-browser-only regression survived: '+result.stdout)
  print('[incarnation.policy.control] original_browser_only_compile_rejected_by_tests=1')
  view=(root/'crates/sex-pdx/src/document_view.rs').read_text();guard='!crate::app_catalog::DOCUMENT_TARGETS.contains(&target)';assert view.count(guard)==2
  # Queue and decode must independently admit a future catalogued subject.
  for index in [0,1]:
   places=[m.start()for m in re.finditer(re.escape(guard),view)];start=places[index]
   changed=view[:start]+'!matches!(target,9|15|16|18)'+view[start+len(guard):]
   path=out/f'viewport-{index}.rs';path.write_text(changed);old=json.dumps(str(root/'crates/sex-pdx/src/document_view.rs'));assert old in fixture
   test.write_text(fixture.replace(old,json.dumps(str(path))));binary=out/f'viewport-{index}-test'
   subprocess.run(['rustc','--edition=2021','--test',str(test),'-o',str(binary)],check=True,env={**os.environ,'EXPANDED_SHELL_POLICY':str(shell)})
   result=subprocess.run([str(binary),'--test-threads=1'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
   if result.returncode!=101:raise RuntimeError('old viewport admission survived: '+result.stdout)
   print(f'[incarnation.policy.control] viewport_guard={index} old_allowlist_rejected_by_tests=1')
  delivery=(root/'servers/silk-shell/src/linen_text_delivery.rs').read_text()
  start=delivery.index('pub const DESKTOP_SURFACES');end=delivery.index('pub struct DesktopStates',start)
  changed=delivery[:start]+'pub const DESKTOP_SURFACES:[u64;16]=[200,201,153,300,301,306,307,156,154,160,161,162,302,303,304,305];\n'+delivery[end:]
  path=out/'old-frame-admission.rs';path.write_text(changed);old=json.dumps(str(root/'servers/silk-shell/src/linen_text_delivery.rs'));assert old in fixture
  test.write_text(fixture.replace(old,json.dumps(str(path))));binary=out/'old-frame-admission-test'
  subprocess.run(['rustc','--edition=2021','--test',str(test),'-o',str(binary)],check=True,env={**os.environ,'EXPANDED_SHELL_POLICY':str(shell)})
  result=subprocess.run([str(binary),'--test-threads=1'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
  if result.returncode!=101:raise RuntimeError('old desktop surfaces survived: '+result.stdout)
  print('[incarnation.policy.control] old_frame_inventory_rejected_by_tests=1')
  style=(root/'crates/sex-pdx/src/desktop_style.rs').read_text();guard='let row=&crate::app_catalog::INCARNATIONS[i];';assert style.count(guard)==1
  changed=style.replace(guard,guard+'if row.target>18{i+=1;continue;}')
  path=out/'old-family-inventory.rs';path.write_text(changed);old=json.dumps(str(root/'crates/sex-pdx/src/desktop_style.rs'));assert old in fixture
  test.write_text(fixture.replace(old,json.dumps(str(path))));binary=out/'old-family-inventory-test'
  subprocess.run(['rustc','--edition=2021','--test',str(test),'-o',str(binary)],check=True,env={**os.environ,'EXPANDED_SHELL_POLICY':str(shell)})
  result=subprocess.run([str(binary),'--test-threads=1'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
  if result.returncode!=101:raise RuntimeError('old visual family inventory survived: '+result.stdout)
  print('[incarnation.policy.control] old_family_inventory_rejected_by_tests=1')
  print('[incarnation.policy] PASS expanded_quil_spindle_browser=1 actual_modules=1 scope=host_doubles_not_boot_authority')

if __name__=='__main__':main()
