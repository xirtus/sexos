#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Real ordinary-parent ticket/argument/grant ownership on an explicitly bound image.
Row-reuse stale-token denial is composite, not an isolated task-generation predicate.
No provider restart, restricted-helper role, worker-I/O isolation or installer claim.
"""
import argparse,hashlib,json,os,re,shutil,subprocess,time
from pathlib import Path
from quil_autosave_gate import boot
from moan_gate_geometry import spindle
from program_web_daily_gate import wait_closed_session
from install_program import install
ROOT=Path(__file__).resolve().parents[1]
def sha(p):
 h=hashlib.sha256()
 with p.open('rb')as f:
  while b:=f.read(1024*1024):h.update(b)
 return h.hexdigest()
def main():
 p=argparse.ArgumentParser(description=__doc__)
 for name in('iso','source-disk','fixture','provenance','out'):p.add_argument('--'+name,type=Path,required=True)
 a=p.parse_args();out=a.out.resolve();out.mkdir(parents=True,exist_ok=False)
 provenance=json.loads(a.provenance.read_text())
 assert provenance['production_revision']=='ec2cc405e0ac89a29d93f338056dad4c67534c4c'
 assert sha(a.iso)==provenance['derived_iso_sha256']and sha(a.fixture)==provenance['fixture_sha256']
 assert sha(a.source_disk)=='49bc20df15e412a64472421e13fe86ff1c5165e18b2afccf160d4dc19fe68a14','known read-only blank seed required'
 subprocess.run(['git','diff','--exit-code','ec2cc405e','--','kernel','servers/silk-shell','servers/sexfiles','crates/sex-app','crates/sex-pdx'],cwd=ROOT,check=True)
 inputs={str(f.resolve()):sha(f)for f in(a.iso,a.source_disk,a.fixture,a.provenance,Path(__file__),ROOT/'Cargo.lock',ROOT/'Cargo.toml',ROOT/'sexos_build_spec.toml',ROOT/'spec/abi_inputs.sha256')}
 for directory in('kernel','servers/silk-shell','servers/sexfiles','crates/sex-app','crates/sex-pdx','apps/ribbon-seal','scripts'):
  for f in(ROOT/directory).rglob('*'):
   if f.is_file()and'__pycache__'not in f.parts and'target'not in f.relative_to(ROOT/directory).parts:inputs[str(f)]=sha(f)
 (out/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n');(out/'source-commit.txt').write_text(subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True));shutil.copyfile(__file__,out/'runner-source.py')
 disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);install(disk,[('ribbonseal',a.fixture.read_bytes())]);os.environ['SEXOS_GATE_BOUND']='1200'
 g=boot(a.iso.resolve(),disk,out,'ticket-owner','1280x800');original_wait=g['wait']
 def wait(pattern,start=0,timeout=120,**kwargs):
  m=original_wait('(?:'+pattern+r')|\[ribbon.failed\] invariant=1',start,timeout,**kwargs)
  if m[0].startswith('[ribbon.failed]'):raise RuntimeError('guest ownership invariant failed')
  return m
 g['wait']=wait
 def launch(line,role=None):
  start=spindle(g,line);m=wait(r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',start,180);target,task,surface=map(int,m.groups())
  if role:wait(rf'\[ribbon.ready\] role={role} target={target} task={task} surface={surface} ok=1',start,120)
  return start,target,task,surface
 def saved_start(start,target,task):
  m=wait(rf'\[spindle.session.started\] target={target} task={task} scene=\d+',start,120)
  wait(r'\[spindle.session.saved\] bytes=[1-9]\d* status=0',start+m.end(),120)
 def retired(start,target,task,empty=False):
  if empty:return wait_closed_session(g,start,target,task)
  wait(rf'\[app.private.retired\] target={target} task={task} pages=\d+ reusable=1',start,180)
  m=wait(rf'\[spindle.session.ended\] target={target} task={task}',start,120)
  wait(r'\[spindle.session.saved\] bytes=[1-9]\d* status=0',start+m.end(),120)
 def key(surface,code):
  g['focus'](surface);start=len(g['text']())
  for down in(True,False):g['q'].cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':code}}}]});time.sleep(.04 if down else .12)
  return start
 def child_retired(start,cookie):
  m=wait(rf'\[ribbon.authorized\] cookie={cookie} row=(\d+) task=(\d+) ok=1',start,120);row,task=map(int,m.groups())
  wait(rf'\[app.private.retired\] target={row} task={task} pages=\d+ reusable=1',start,180)
  return dict(row=row,task=task)
 facts={}
 try:
  wait(r'\[app.private.retired\] target=41 ',0,180)
  start,t,task,_=launch('run ribbonseal --seed @/');wait(r'\[ribbon.seed\] files=2 ok=1 developer_only=1',start,120);retired(start,t,task,empty=True)
  start,owner,owner_task,owner_surface=launch('run ribbonseal --owner @/ribbon-xchg @/ribbon-private','owner');saved_start(start,owner,owner_task)
  start,foreign,foreign_task,foreign_surface=launch('run ribbonseal --foreign @/ribbon-xchg','foreign');saved_start(start,foreign,foreign_task);assert owner!=foreign
  g['q'].cmd('screendump',{'filename':str(out/'parents-ready.ppm')})
  start=key(owner_surface,'v');wait(r'\[ribbon.sdk.done\] inherited_argument=1 inherited_file=1 ok=1',start,120);wait(r'\[ribbon.child\] phase=sdk target=\d+ task=\d+ args_ok=1 grant=1 ok=1',start,120);facts['sdk']=child_retired(start,0x5001)
  start=key(owner_surface,'f');pending=wait(rf'\[ribbon.pending\] phase=foreign target={owner} task={owner_task} ticket=(\d+)',start,120);ticket=int(pending[1]);wait(rf'\[files.ticket.denied\] requester={foreign}',start,120)
  wait(rf'\[ribbon.foreign.done\] target={foreign} task={foreign_task} original_args=1 read_denied=1 delegation_denied=1 ok=1',start,120)
  wait(rf'\[ribbon.owner.done\] ticket={ticket} foreign_denied=1 owner_consumed=1 single_use=1 ok=1',start,120)
  wait(r'\[ribbon.refused\] cookie=20993 code=1 ok=1',start,120);wait(r'\[ribbon.refused\] cookie=20995 code=1 ok=1',start,120);facts['foreign']=child_retired(start,0x5202)
  start=key(owner_surface,'r');old=wait(rf'\[ribbon.retire.pending\] target={owner} task={owner_task} ticket=(\d+) ok=1',start,120);stale_ticket=int(old[1]);retired(start,owner,owner_task)
  start,new_owner,new_task,_=launch('run ribbonseal --reuse @/ribbon-xchg');assert new_owner==owner and new_task!=owner_task,'actual same-row/new-task reuse required'
  wait(rf'\[ribbon.reuse.done\] target={owner} old_task={owner_task} new_task={new_task} stale_ticket={stale_ticket} stale_refused=1 no_old_grant=1 plain_child=1 composite_scope=1 ok=1',start,180)
  wait(r'\[ribbon.child\] phase=reuse target=\d+ task=\d+ args_ok=1 grant=0 ok=1',start,120);wait(r'\[ribbon.refused\] cookie=21249 code=1 ok=1',start,120);facts['reuse']=child_retired(start,0x5302)
  retired(start,new_owner,new_task,empty=True);retired(start,foreign,foreign_task,empty=True)
  g['q'].cmd('screendump',{'filename':str(out/'after-ownership.ppm')});g['q'].cmd('quit');g['proc'].wait(30)
 finally:g['stop']()
 assert all(sha(Path(name))==digest for name,digest in inputs.items()),'bound source/binary changed'
 result=dict(ok=True,scope=__doc__,production_revision=provenance['production_revision'],source_commit=(out/'source-commit.txt').read_text().strip(),inputs_sha256=sha(out/'inputs.json'),owner=dict(target=owner,task=owner_task),foreign=dict(target=foreign,task=foreign_task),reuse=dict(target=new_owner,task=new_task,old_ticket=stale_ticket),children=facts,foreign_ticket=ticket,grant_handoff_on_refusal=False,worker_scope='not tested: kernel normalizes live workers to main-run identity; preserved v3 counterexample; no worker-launch exclusion or stale-I/O isolation claim')
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
if __name__=='__main__':main()
