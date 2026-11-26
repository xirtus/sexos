#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Benign public-SDK processing/crash/restart/reuse on a distinct development image.
No parent-memory/cross-domain attack or independent missing-ACK coverage claim.
"""
import argparse,hashlib,json,os,re,shutil,subprocess,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from moan_gate_geometry import spindle
from program_gate_geometry import maximize,current_tile
from program_web_daily_gate import wait_closed_session
from moan_recovery_receipts import admission
from moan_recovery_pcm import verify
ROOT=Path(__file__).resolve().parents[1]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser(description=__doc__)
 for name in('iso','source-disk','program','helper','provenance','out'):p.add_argument('--'+name,type=Path,required=True)
 a=p.parse_args();out=a.out.resolve();out.mkdir(parents=True,exist_ok=False)
 provenance=json.loads(a.provenance.read_text());assert provenance['scope']=='development_consented_service_restart','explicit development scope required'
 assert provenance['iso_sha256']==sha(a.iso)and provenance['program_sha256']==sha(a.program)and provenance['helper_sha256']==sha(a.helper)
 assert sha(a.source_disk)=='49bc20df15e412a64472421e13fe86ff1c5165e18b2afccf160d4dc19fe68a14','exact preserved blank seed required'
 required={'servers/silk-shell','servers/sexfiles','servers/sexnet','servers/sexbell','servers/sexdisplay','apps/spindle'}
 assert required<=set(provenance['iso_files']),'all changed policy providers and crash-probe Spindle require exact binding'
 extracted=out/'iso-inputs';extracted.mkdir()
 for name in sorted(required):
  path=extracted/Path(name).name
  with (out/('extract-'+Path(name).name+'.log')).open('w')as log:subprocess.run(['xorriso','-osirrox','on','-indev',str(a.iso.resolve()),'-extract','/'+name,str(path)],stdout=log,stderr=subprocess.STDOUT,check=True)
  assert sha(path)==provenance['iso_files'][name],name

 inputs={str(p.resolve()):sha(p)for p in(a.iso,a.source_disk,a.program,a.helper,a.provenance)}
 for directory in('scripts','fixtures/moan-recovery','crates/moan-plugin','crates/sex-app','crates/sex-pdx','servers/silk-shell','servers/sexfiles','apps/moanplug'):
  for f in(ROOT/directory).rglob('*'):
   if f.is_file()and'target'not in f.relative_to(ROOT/directory).parts and'__pycache__'not in f.parts:inputs[str(f.resolve())]=sha(f)
 (out/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n');(out/'source-commit.txt').write_text(subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True));shutil.copyfile(__file__,out/'runner-source.py')
 disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);install(disk,[('moanrecover',a.program.read_bytes()),('moanplug',a.helper.read_bytes())]);os.environ['SEXOS_GATE_BOUND']='1800'
 g=boot(a.iso.resolve(),disk,out,'moan-recovery','1280x800');original_wait=g['wait'];facts=[];successful=[];failed=[];all_children=[]
 def wait(pattern,start=0,timeout=120,**kw):
  m=original_wait('(?:'+pattern+r')|\[moanrecover.failed\][^\n]*',start,timeout,**kw)
  if m[0].startswith('[moanrecover.failed]'):raise RuntimeError(m[0])
  return m
 g['wait']=wait
 try:
  wait(r'\[app.private.retired\] target=41 ',0,180)
  start=spindle(g,'run moanrecover @/data/moanrecover');parent=wait(r'\[moanrecover.ready\] target=(\d+) task=(\d+)',start,180);pd,task=map(int,parent.groups());surface=int(wait(rf'\[shell.program.launch\] target={pd} task={task} surface=(\d+)',start)[1]);tile,cursor=maximize(g,surface);w,h=tile[2]-16,tile[3]-42
  wait(rf'\[moanrecover.control\] id=8 x=\d+ y=\d+ w={w} h={h}',cursor)
  def click(which):
   g['focus'](surface);tile=current_tile(g['text'](),surface);w,h=tile[2]-16,tile[3]-42
   positions=re.findall(rf'\[moanrecover.control\] id={which} x=(-?\d+) y=(-?\d+) w={w} h={h}',g['text']());assert positions,(which,w,h);x,y=map(int,positions[-1]);assert 0<=x<w and 0<=y<h
   x+=tile[0]+8;y+=tile[1]+34;assert 0<=x<1280 and 0<=y<800
   g['q'].cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/1279)}},{'type':'abs','data':{'axis':'y','value':round(y*32767/799)}}]});time.sleep(.1);start=len(g['text']())
   for down in(True,False):g['q'].cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.1)
   return start
  def running(start,mode,offset=None):
   match=wait(rf'\[moanrecover.started\] mode={mode} cookie=(\d+)',start if offset is None else offset);cookie=int(match[1])
   identity=wait(rf'\[moan.role.started\] parent={pd} parent_task={task} cookie={cookie} pd=(\d+) task=(\d+)',start,180);child,ct=map(int,identity.groups())
   wait(rf'\[moanplug.native\] pd={child} task={ct} ready=1',start,180)
   proof=admission(g['text'](),child,ct,start);all_children.append((child,ct));return cookie,child,ct,proof,start+match.end()
  def ended(start,mode,cookie,child,ct,processed):
   wait(rf'\[moanrecover.result\] mode={mode} cookie={cookie} complete={int(processed)} processed={int(processed)} retired=1',start,180)
   wait(rf'\[moanrecover.identity\] cookie={cookie} pd={child} task={ct}',start,180)
   wait(rf'\[app.private.retired\] target={child} task={ct} pages=\d+ reusable=1',start,180)
   (successful if processed else failed).append((cookie,mode));alive=click(7);wait(r'\[moanrecover.alive\] n=[1-9]\d*',alive)
  for mode in(0,3,1,0):
   start=click(mode);cookie,child,ct,proof,_=running(start,mode);processed=mode!=1;ended(start,mode,cookie,child,ct,processed)
   if mode==1:wait(rf'fault.kill \[cpu.fault.terminal\] pd={child} kind=invalid_opcode ',start)
   facts.append(dict(case='clap'if mode==0 else'vst3'if mode==3 else'own_intentional_crash',cookie=cookie,admission=proof))
  # One consented fault of each existing restartable service, one exact helper
  # cancellation/retirement and one fresh successful effect afterward. No promise
  # of an unavailable immediate reverse epoch-notification path.
  for name,service in(('network',13),('bell',10),('display',1),('files',11),('shell',3)):
   start=click(2);cookie,child,ct,proof,_=running(start,2)
   fault=spindle(g,'crash '+name);wait(rf'\[service.restarted\] pd={service} task=\d+ restarts=[1-9]\d* pages=\d+ dropped_messages=\d+ admitted=1',fault,180)
   cancel=click(6);wait(r'\[moanrecover.cancelled\] requested=1',cancel);ended(start,2,cookie,child,ct,False)
   next_start=click(0);nc,np,nt,npf,_=running(next_start,0);ended(next_start,0,nc,np,nt,True)
   facts.append(dict(case='consented_service_restart',service=service,cancelled=dict(cookie=cookie,admission=proof),recovered=dict(cookie=nc,admission=npf)))
  start=click(4);first=running(start,2);second=running(start,2,first[4]);assert first[1]!=second[1],'two actual independent helper rows'
  refusal=click(5);match=wait(r'\[moanrecover.refused\] mode=2 cookie=(\d+) reason=[^\n]+',refusal);third=int(match[1]);failed.append((third,2));wait(r'\[moanrecover.alive\] n=[1-9]\d*',click(7))
  cancel=click(6);wait(r'\[moanrecover.cancelled\] requested=1',cancel)
  for item in(first,second):ended(start,2,*item[:3],False)
  assert not re.search(r'\[moan.role.reserved\]',g['text']()[refusal:]),'third request must not allocate another helper row'
  assert not re.search(rf'\[shell.launch.asked\] parent={pd} task={task} cookie={third} ',g['text']()[refusal:]),'no ordinary launch fallback'
  facts.append(dict(case='end_to_end_two_job_bound',third_cookie=third,scope='Files capacity may refuse before Shell allocator; no ordinary fallback; not independent pool branch coverage'))
  g['q'].cmd('screendump',{'filename':str(out/'parent-after-recovery.ppm')})
  close=click(8);wait(r'\[moanrecover.closed\]',close);wait_closed_session(g,close,pd,task);g['q'].cmd('quit');g['proc'].wait(30)
 finally:g['stop']()
 # Read-only cold extraction and independent every-sample output proof.
 extractor=out/'sexfs-extract';subprocess.run(['rustc','+stable','--edition=2021','-O','-A','dead_code','-A','unused_attributes',str(ROOT/'scripts/sexfs_extract.rs'),'-o',str(extractor)],check=True)
 pcm_proofs=[]
 for cookie,mode in successful:
  files=[]
  for prefix in('input','output'):
   path=out/f'{prefix}-{cookie}.bin';subprocess.run([str(extractor),str(disk),f'/data/moanrecover/{prefix}-{cookie}.bin',str(path)],check=True);files.append(path.read_bytes())
  pcm_proofs.append(dict(cookie=cookie,**verify(*files,vst=mode==3)))
 for cookie,_ in failed:
  path=out/f'unexpected-output-{cookie}.bin';result=subprocess.run([str(extractor),str(disk),f'/data/moanrecover/output-{cookie}.bin',str(path)],capture_output=True,text=True)
  (out/f'absent-output-{cookie}.log').write_text(result.stdout+result.stderr)
  assert result.returncode!=0 and'guest file'in result.stderr and'NotFound'in result.stderr,'failed/cancelled job must have no published output'
 assert len(set(all_children))==len(all_children),'exact task identities never repeat'
 assert any(a==c and b!=d for i,(a,b)in enumerate(all_children)for c,d in all_children[i+1:]),'actual same-row new-task reuse required'
 assert all(sha(Path(p))==v for p,v in inputs.items()),'source/build inputs changed'
 result=dict(ok=True,scope=__doc__,parent=dict(pd=pd,task=task),cases=facts,pcm=pcm_proofs,isolated_parent_memory=False,cross_domain_attack_coverage=False,independent_missing_ack_runtime_coverage=False,inputs_sha256=sha(out/'inputs.json'),artifacts={p.name:sha(p)for p in out.iterdir()if p.is_file()})
 (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
if __name__=='__main__':main()
