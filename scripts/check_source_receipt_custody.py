#!/usr/bin/env python3
"""Compile actual Net source dispatch under finite held/source host callbacks."""
import argparse,os,re,subprocess,tempfile
from pathlib import Path

def block(source,start):
 opening=source.index('{',start);end=opening+1;depth=1
 while depth:
  if end>=len(source):raise ValueError('unterminated Net source block')
  depth+=(source[end]=='{')-(source[end]=='}');end+=1
 return source[start:end]

def function(source,name):
 found=list(re.finditer(r'^fn '+re.escape(name)+r'\(',source,re.M))
 if len(found)!=1:raise ValueError('missing/ambiguous Net function '+name)
 return block(source,found[0].start())

def extract(source):
 names=('network_owner','network_request_owner','source_receipt_member','retain_source_denial','source_denial_live','poll_source_denials')
 parts=[function(source,name)for name in names]
 branches=[]
 for wire,kind in [('trust_policy','Trust'),('certificate_time','CertificateTime'),('entropy_seed','Entropy')]:
  opening='if req.type_id==sex_pdx::'+wire+'::OP{'
  if source.count(opening)!=1:raise ValueError('missing/ambiguous typed source dispatch '+wire)
  body=block(source,source.index(opening))
  decode='let request=sex_pdx::'+wire+'::Request::decode([req.arg0,req.arg1,req.arg2]);'
  authenticated='if let (Some(owner),Ok(request))=(network_request_owner(req.caller_pd,req.caller_task),request){'
  deny='if retain_source_denial(source_denials,source_denial::Kind::'+kind+',owner,request.seq){continue;}'
  if not body.index(decode)<body.index(authenticated)<body.index(deny)<body.index('.handle('):raise ValueError('typed source authentication/budget ordering '+wire)
  branches.append(body)
 parts.append('fn actual_dispatch(req:PdxMessage,source_denials:&mut source_denial::Outbox,seed:&mut entropy::Machine,clock:&mut clock_machine::Machine,trust:&mut trust_machine::Machine,seed_source:Option<entropy_source::Info>,clock_source:Option<clock_source::Info>){for _ in 0..1{\n'+'\n'.join(branches)+'\n}}')
 a=source.index('        if let Some(owner)=seed.owner(){');b=source.index('        // Released DNS observations',a)
 parts.append('fn actual_seed_advance(seed:&mut entropy::Machine){\n'+source[a:b]+'\n}')
 return '\n'.join(parts)+'\n'

def wiring(source):
 if source.count('static mut SOURCE_DENIALS:source_denial::Outbox=source_denial::Outbox::new();')!=1:raise ValueError('denial backing must be unique static storage')
 if source.count('let source_denials=unsafe { &mut SOURCE_DENIALS };')!=1:raise ValueError('denial backing must be attached once')
 if 'let mut source_denials=source_denial::Outbox::new()'in source:raise ValueError('unexpected stack denial backing')
 poll='        poll_source_denials(source_denials);'
 pending='||source_denials.pending_reply().is_some();'
 if source.count(poll)!=1 or source.count(pending)!=1:raise ValueError('denial poll/pending not in actual idle loop')
 a=source.index(poll);b=source.index('let reply_pending=',a);c=source.index('let pending=preparing||polling||reply_pending||seed.busy();',b) if 'let pending=preparing||polling||reply_pending||seed.busy();' in source[b:] else -1
 if c<0:raise ValueError('denial pending not driving timed receive loop')
 if not a<b<source.index(pending,b)<c:raise ValueError('denial poll/pending ordering')
 if not source[c:].startswith('let pending=preparing||polling||reply_pending||seed.busy();\n        let req = if pending {'):raise ValueError('pending work bypasses actual receive branch')
 exit_start=source.index('if req.type_id==sex_pdx::app_lifecycle::OP_APP_EXIT')
 exit_body=block(source,exit_start)
 closure=exit_body.index('|notice| {');end=exit_body.index('}) {',closure)
 retire='source_denials.retire(source_denial::Identity{pd:notice.target_pd,task:notice.task_id});'
 if exit_body.count(retire)!=1 or not closure<exit_body.index(retire)<end<exit_body.index('[net.app.cleanup.ack]'):raise ValueError('exact denial retirement must precede service cleanup ACK in attested closure')
 if 'app_custody.receive(&mut sex_pdx::app_lifecycle::KernelAuthority'not in exit_body[:closure]:raise ValueError('cleanup lacks actual held lifecycle authority')

def check(root,out,denial):
 source=(root/'servers/sexnet/src/main.rs').read_text();wiring(source);policy=extract(source)
 retire='source_denials.retire(source_denial::Identity{pd:notice.target_pd,task:notice.task_id});'
 controls=[
  ('missing-idle-pending',source.replace('||source_denials.pending_reply().is_some();',';')),
  ('wrong-cleanup-task',source.replace(retire,retire.replace('task:notice.task_id','task:notice.target_pd'))),
  ('cleanup-outside-attested-closure',source.replace(retire,'').replace('                // Private job custody and collected owner egress are gone before ACK.',retire+'\n                // Private job custody and collected owner egress are gone before ACK.')),
  ('pending-ignored',source.replace('let req = if pending {','let req = if false {')),
 ]
 reports=[]
 for name,changed in controls:
  if changed==source:raise ValueError('structural control did not mutate source: '+name)
  try:wiring(changed)
  except ValueError:reports.append('[source.receipt.wiring] rejected='+name)
  else:raise RuntimeError('structural control accepted: '+name)
 (out/'wiring-controls.log').write_text('\n'.join(reports)+'\n')
 for report in reports:print(report)
 fixture=(root/'tests/source_receipt_dispatch.rs').read_text()
 fixture=re.sub(r'(?<=path=")\.\./([^"\n]+)',lambda m:str(root/m[1]),fixture)
 fixture=fixture.replace(str(root/'servers/sexnet/src/source_denial.rs'),str(denial))
 fixture_path=out/'fixture.rs';fixture_path.write_text(fixture)
 trust=out/'libsex_trust_config.rlib'
 subprocess.run(['rustc','--edition=2021','--crate-name','sex_trust_config','--crate-type','rlib',str(root/'crates/sex-trust-config/src/lib.rs'),'-o',str(trust)],check=True)
 variants=[('actual',policy,False),('bypass-denial',policy.replace('if source_receipt_member(owner){return false;}','return false;'),True),('task-equals-domain',policy.replace('owner.pd!=0 && owner.task!=0 && sex_pdx::app_catalog::is_browser(owner.pd)','owner.pd!=0 && owner.task==owner.pd && sex_pdx::app_catalog::is_browser(owner.pd)'),True)]
 for name,text,rejected in variants:
  if rejected and text==policy:raise ValueError('mutant failed to change production helper: '+name)
  path=out/(name+'-production.rs');path.write_text(text);binary=out/name
  command=['rustc','--edition=2021','--test',str(fixture_path),'--extern','sex_trust_config='+str(trust),'-o',str(binary)]
  compiled=subprocess.run(command,env={**os.environ,'SOURCE_RECEIPT_PRODUCTION':str(path)},text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
  (out/(name+'-compile.log')).write_text(compiled.stdout)
  if compiled.returncode:raise RuntimeError(name+' failed to compile: '+compiled.stdout)
  result=subprocess.run([str(binary),'--test-threads=1'],text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);(out/(name+'.log')).write_text(result.stdout)
  if result.returncode!=(101 if rejected else 0):raise RuntimeError(name+' unexpected test result: '+result.stdout)
  if rejected:print('[source.receipt.mutant] rejected='+name)
  else:print(result.stdout,end='')
 print('[source.receipt.dispatch.host] PASS actual_typed_branches=3 compiled_mutants=2 scope=finite_host_callbacks_not_native_authority')

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);parser.add_argument('--out',type=Path);parser.add_argument('--denial-path',type=Path)
 args=parser.parse_args();root=args.root.resolve();denial=(args.denial_path or root/'servers/sexnet/src/source_denial.rs').resolve(strict=True)
 if args.out:args.out.mkdir(parents=True,exist_ok=True);check(root,args.out.resolve(),denial)
 else:
  with tempfile.TemporaryDirectory(prefix='sexos-source-receipt-')as out:check(root,Path(out),denial)
if __name__=='__main__':main()
