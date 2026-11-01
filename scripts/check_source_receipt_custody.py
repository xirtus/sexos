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
 names=('network_owner','network_request_owner','source_receipt_member','retain_source_denial','source_receipt_live','poll_source_denials','maintain_seed_source')
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
 # The production service module names overlap SDK wire modules in the host
 # fixture. Only namespace aliases and a call-through wrapper are synthetic;
 # the actual polling function is extracted intact on every run.
 parts.append('mod actual_source_poll { use super::*; use crate::clock_machine as certificate_time; use crate::trust_machine as trust_policy;\n'+function(source,'poll_source_receipts')+'\npub fn run(seed:&mut entropy::Machine,clock:&mut clock_machine::Machine,trust:&mut trust_machine::Machine){poll_source_receipts(seed,clock,trust)}\n}\nuse actual_source_poll::run as poll_source_receipts;')
 return '\n'.join(parts)+'\n'

def wiring(source):
 main=source[source.index('pub extern "C" fn _start()'):]
 maintain='maintain_seed_source(&mut seed);';deliver='poll_source_receipts(&mut seed,&mut clock,&mut trust);'
 if main.count(maintain)!=1 or main.count(deliver)!=1:raise ValueError('actual main must invoke maintenance/delivery exactly once per loop')
 seed_init=main.index('let mut seed=entropy::Machine::new();')
 loop=main.index('loop {',seed_init)
 if not loop<main.index(maintain)<main.index('dns.maintain_records('):raise ValueError('seed maintenance moved outside pre-I/O custody pass')
 if not main.index('if let Some((owner,word))=dns.pending_reply()')<main.index(deliver)<main.index('poll_source_denials(source_denials);')<main.index('let reply_pending='):raise ValueError('source delivery moved outside pending/idle custody pass')
 lease='seed.maintain_lease(sex_pdx::get_ticks());'
 poll_source=function(source,'poll_source_receipts');maintenance=function(source,'maintain_seed_source')
 if poll_source.count('seed.pending_owner()')!=1 or poll_source.count('seed.pending_reply()')!=1 or poll_source.count(lease)!=2:raise ValueError('seed poll requires owner-only selection and both pending/ownerless lease paths')
 if not poll_source.index('seed.pending_owner()')<poll_source.index('let live=source_receipt_live(')<poll_source.index(lease)<poll_source.index('seed.pending_reply()')<poll_source.index('target==owner')<poll_source.index('pdx_reply(owner.pd,word)'):raise ValueError('seed owner/query/fresh lease/copy/identity/send ordering changed')
 if maintenance.index('sex_pdx::get_ticks()')<maintenance.index('match source_receipt_live(') or 'seed.advance(sex_pdx::get_ticks(),'not in maintenance or 'None=>{seed.maintain_lease(sex_pdx::get_ticks());},'not in maintenance:raise ValueError('seed maintenance ticks must follow liveness query')
 if 'network_owner('in maintenance+poll_source:raise ValueError('source custody still retires on collapsed uncertain liveness')
 if main.count('let mut clock=certificate_time::Machine::new();')!=1 or main.index('let mut clock=certificate_time::Machine::new();')>loop:raise ValueError('clock high-water/poison machine must remain boot-global')
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
 poll_fn=function(source,'poll_source_receipts');lease='seed.maintain_lease(sex_pdx::get_ticks());'
 late_lease=poll_fn.replace(lease,'',1).replace('if let Some((target,word))=seed.pending_reply(){','if let Some((target,word))=seed.pending_reply(){'+lease)
 maintenance_fn=function(source,'maintain_seed_source')
 stale_ticks=maintenance_fn.replace('let before_seed=seed.snapshot();','let stale_now=sex_pdx::get_ticks();let before_seed=seed.snapshot();').replace('seed.advance(sex_pdx::get_ticks(),','seed.advance(stale_now,')
 controls=[
  ('missing-idle-pending',source.replace('||source_denials.pending_reply().is_some();',';')),
  ('wrong-cleanup-task',source.replace(retire,retire.replace('task:notice.task_id','task:notice.target_pd'))),
  ('cleanup-outside-attested-closure',source.replace(retire,'').replace('                // Private job custody and collected owner egress are gone before ACK.',retire+'\n                // Private job custody and collected owner egress are gone before ACK.')),
  ('pending-ignored',source.replace('let req = if pending {','let req = if false {')),
  ('missing-source-maintenance',source.replace('        maintain_seed_source(&mut seed);','')),
  ('missing-source-poll',source.replace('        poll_source_receipts(&mut seed,&mut clock,&mut trust);','')),
  ('source-poll-after-idle',source.replace('        poll_source_receipts(&mut seed,&mut clock,&mut trust);','').replace('        let req = if pending {','        poll_source_receipts(&mut seed,&mut clock,&mut trust);\n        let req = if pending {')),
  ('lease-after-secret-copy',source.replace(poll_fn,late_lease)),
  ('maintenance-ticks-before-query',source.replace(maintenance_fn,stale_ticks)),

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
 # Mutate only the actual extracted helpers. Missing or ambiguous insertion
 # sites are harness failures, never accepted as a rejected mutant.
 def replace_once(text,old,new):
  if text.count(old)!=1:raise ValueError('missing/ambiguous race mutation site: '+old)
  return text.replace(old,new,1)
 maintenance=function(policy,'maintain_seed_source')
 stale_ticks=replace_once(maintenance,'let before_seed=seed.snapshot();','let old_now=sex_pdx::get_ticks();let before_seed=seed.snapshot();')
 stale_ticks=replace_once(stale_ticks,'seed.advance(sex_pdx::get_ticks(),','seed.advance(old_now,')
 poll=function(policy,'poll_source_receipts')
 early_copy=replace_once(poll,'if let Some(owner)=seed.pending_owner(){','if let Some((owner,early_word))=seed.pending_reply(){')
 early_copy=replace_once(early_copy,'if let Some((target,word))=seed.pending_reply(){','if let Some((target,word))=Some((owner,early_word)){')
 race_cases={
  'race-stale-maintenance':'source_poll_positive_live_query_crossing_deadline_prevents_collecting_provider_call',
  'race-early-secret-copy':'source_poll_positive_live_query_crossing_deadline_cannot_copy_or_deliver_old_read',
 }
 variants=[
  ('actual',policy,False),
  ('race-stale-maintenance',replace_once(policy,maintenance,stale_ticks),True),
  ('race-early-secret-copy',replace_once(policy,poll,early_copy),True),
  ('bypass-denial',policy.replace('if source_receipt_member(owner){return false;}','return false;'),True),
  ('task-equals-domain',policy.replace('owner.pd!=0 && owner.task!=0 && sex_pdx::app_catalog::is_browser(owner.pd)','owner.pd!=0 && owner.task==owner.pd && sex_pdx::app_catalog::is_browser(owner.pd)'),True),
  ('uncertainty-retires',policy.replace('Some(false)=>{','Some(false)|None=>{'),True),
  ('uncertainty-samples',policy.replace('None=>{seed.maintain_lease(sex_pdx::get_ticks());},','None=>seed.advance(sex_pdx::get_ticks(),||sex_pdx::entropy_word(sex_pdx::entropy_source::SLOT).map(|word|word.value)),'),True),
 ]
 for name,text,rejected in variants:
  if rejected and text==policy:raise ValueError('mutant failed to change production helper: '+name)
  path=out/(name+'-production.rs');path.write_text(text);binary=out/name
  command=['rustc','--edition=2021','--test',str(fixture_path),'--extern','sex_trust_config='+str(trust),'-o',str(binary)]
  compiled=subprocess.run(command,env={**os.environ,'SOURCE_RECEIPT_PRODUCTION':str(path)},text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
  (out/(name+'-compile.log')).write_text(compiled.stdout)
  if compiled.returncode:raise RuntimeError(name+' failed to compile: '+compiled.stdout)
  test_command=[str(binary),'--test-threads=1']
  if name in race_cases:test_command+=['--exact',race_cases[name]]
  result=subprocess.run(test_command,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT);(out/(name+'.log')).write_text(result.stdout)
  if result.returncode!=(101 if rejected else 0):raise RuntimeError(name+' unexpected test result: '+result.stdout)
  if name in race_cases and not re.search(r'^test '+re.escape(race_cases[name])+r' \.\.\. FAILED$',result.stdout,re.M):raise RuntimeError(name+' must fail its exact lifecycle-query timing case')
  if name.startswith('uncertainty-') and not re.search(r'^test source_poll_.* FAILED$',result.stdout,re.M):raise RuntimeError(name+' must be rejected by an actual source-poll control')
  if rejected:print('[source.receipt.mutant] rejected='+name)
  else:print(result.stdout,end='')
 print('[source.receipt.dispatch.host] PASS actual_typed_branches=3 compiled_mutants=6 scope=finite_host_callbacks_not_native_authority')

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--root',type=Path,default=Path(__file__).resolve().parents[1]);parser.add_argument('--out',type=Path);parser.add_argument('--denial-path',type=Path)
 args=parser.parse_args();root=args.root.resolve();denial=(args.denial_path or root/'servers/sexnet/src/source_denial.rs').resolve(strict=True)
 if args.out:args.out.mkdir(parents=True,exist_ok=True);check(root,args.out.resolve(),denial)
 else:
  with tempfile.TemporaryDirectory(prefix='sexos-source-receipt-')as out:check(root,Path(out),denial)
if __name__=='__main__':main()
