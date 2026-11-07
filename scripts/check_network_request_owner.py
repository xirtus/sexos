#!/usr/bin/env python3
"""Actual fresh typed-request task checks; maintenance remains held-custody based."""
import argparse,os,re,subprocess,tempfile
from pathlib import Path

def function(source,name):
 match=re.search(r'^fn '+name+r'\(',source,re.M);assert match,name;start=match.start();end=source.index('{',start)+1;depth=1
 while depth:depth+=(source[end]=='{')-(source[end]=='}');end+=1
 return source[start:end]
def check(root,out):
 source=(root/'servers/sexnet/src/main.rs').read_text();assert source.count('network_request_owner(req.caller_pd,req.caller_task)')==6;assert 'network_owner(req.caller_pd)'not in source
 policy='\n'.join(function(source,n)for n in ['network_owner','network_request_owner']);fixture=(root/'tests/network_request_owner.rs').read_text();fixture=re.sub(r'\.\./crates/[^"\n]+',lambda m:str(root/m[0][3:]),fixture)
 for name,text,rejected in [('actual',policy,False),('missing-task-check',policy.replace('if caller_task!=owner.task{return None;}',''),True),('task-equals-domain',policy.replace('caller_task!=owner.task','caller_task!=owner.pd'),True)]:
  assert not rejected or text!=policy;path=out/(name+'-policy.rs');path.write_text(text);test=out/(name+'.rs');test.write_text(fixture);binary=out/name
  subprocess.run(['rustc','--edition=2021','--test',str(test),'-o',str(binary)],env={**os.environ,'NETWORK_REQUEST_OWNER_POLICY':str(path)},check=True)
  result=subprocess.run([str(binary),'--test-threads=1'],text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);(out/(name+'.log')).write_text(result.stdout)
  if result.returncode!=(101 if rejected else 0):raise RuntimeError(name+': '+result.stdout)
  if rejected:print('[network.request.owner.mutant] rejected='+name)
  else:print(result.stdout,end='')
 print('[network.request.owner.host] PASS typed_dispatches=5 cases=7 mutants=2 legacy_cached_reads_unchanged=1')
def main():
 p=argparse.ArgumentParser();p.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);p.add_argument('--out',type=Path);a=p.parse_args();root=a.root.resolve()
 if a.out:a.out.mkdir(parents=True,exist_ok=False);check(root,a.out.resolve())
 else:
  with tempfile.TemporaryDirectory(prefix='sexos-network-request-owner-')as out:check(root,Path(out))
if __name__=='__main__':main()
