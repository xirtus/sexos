#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Chastity + real SDK client, separate program capabilities: persisted vault
reopened after reboot, impersonation refused, default-deny/allow-once consent,
new consumer run requires fresh consent, and lock makes provider unavailable.
Only fictional fixture credentials are typed; secret material must not be logged.
"""
import argparse,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
MASTER='testonlymasterpassword'
SECRET='autofilltestsecret'

def main():
 p=argparse.ArgumentParser(description=__doc__)
 for n in ('iso','source-disk','out'):p.add_argument('--'+n,required=True,type=Path)
 p.add_argument('--resolution',default='1280x800');a=p.parse_args();out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);install(disk,[(name,(ROOT/'iso_root/programs'/name).read_bytes())for name in ('chastity','chastity-probe')]);rw,rh=map(int,a.resolution.split('x'))
 g=None
 def start(tag):return boot(a.iso,disk,out,tag,a.resolution)
 def keys(text):
  for c in text:g['q'].key({' ':'spc','.':'dot','-':'minus'}.get(c,c));time.sleep(.06)
 def launch(name,marker):
  g['focus'](153);time.sleep(.4);s=len(g['text']());keys('run '+name);g['q'].key('ret');m=g['wait'](marker,s,300);target,task=int(m[1]),int(m[2]);r=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);sid=int(r[1]);g['focus'](sid);time.sleep(.6);return target,task,sid
 def click(name,sid,probe=False):
  text=g['text']();prefix='chastity.probe.control'if probe else'chastity.control';pos={n:(int(x),int(y))for n,x,y in re.findall(r'\['+re.escape(prefix)+r'\] name=(\w+) x=(-?\d+) y=(-?\d+)',text)};assert name in pos,pos
  tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text)[-1]));x,y=pos[name];x+=tile[0]+8;y+=tile[1]+36
  g['q'].cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.2)
  for down in (True,False):g['q'].cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.12)
 try:
  g=start('autofill-save');g['wait'](r'\[app.private.retired\] target=41 ',0,300);_,_,vault=launch('chastity',r'\[chastity.ready\] target=(\d+) task=(\d+)')
  click('master',vault);keys(MASTER);click('confirm',vault);keys(MASTER);s=len(g['text']());click('create',vault);g['wait'](r'\[chastity.created\]',s,120);time.sleep(.4)
  for field,value in [('title','fixture'),('origin','example.org'),('username','alice'),('password',SECRET)]:click(field,vault);keys(value)
  click('add',vault);s=len(g['text']());click('save',vault);g['wait'](r'\[chastity.saved\] bytes=\d+ synced=1',s,300);g['stop']();g=None
  g=start('autofill-live');g['wait'](r'\[app.private.retired\] target=41 ',0,300);_,_,vault=launch('chastity',r'\[chastity.ready\] target=(\d+) task=(\d+)');click('master',vault);keys(MASTER);s=len(g['text']());click('unlock',vault);g['wait'](r'\[chastity.unlocked\] entries=1 imported=0',s,300);g['wait'](r'\[chastity.autofill.ready\]',s,120)
  target,task,consumer=launch('chastity-probe',r'\[chastity.probe.ready\] target=(\d+) task=(\d+)');s=len(g['text']());click('forge',consumer,True);g['wait'](r'\[chastity.probe.impersonation\] denied=1',s,60)
  def request():
   g['focus'](consumer);time.sleep(.2);s=len(g['text']());click('request',consumer,True);g['wait'](r'\[chastity.probe.requested\]',s,60);m=g['wait'](rf'\[chastity.autofill.prompt\] token=(\d+) domain={target} task={task}',s,120);assert '[chastity.probe.received]'not in g['text']()[s:];return s,int(m[1])
  s,first=request();g['focus'](vault);time.sleep(.5);g['q'].cmd('screendump',{'filename':str(out/'consent-default-deny.ppm')});g['q'].key('ret');g['wait'](rf'\[chastity.autofill.denied\] token={first}',s,60);g['wait'](r'\[chastity.probe.refused\] reason=Declined',s,60);assert '[chastity.probe.received]'not in g['text']()
  s,second=request();assert second!=first;g['focus'](vault);time.sleep(.3);g['q'].key('tab');g['q'].key('ret');g['wait'](rf'\[chastity.autofill.approved\] token={second}',s,60);g['wait'](r'\[chastity.probe.received\] expected=1',s,120)
  # A reused consumer row/task cannot inherit the previous approval.
  g['focus'](consumer);s=len(g['text']());g['q'].key('f11');g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120);old_task=task
  target,task,consumer=launch('chastity-probe',r'\[chastity.probe.ready\] target=(\d+) task=(\d+)');assert task!=old_task
  s,third=request();assert third not in(first,second);g['focus'](vault);time.sleep(.3);g['q'].key('ret');g['wait'](rf'\[chastity.autofill.denied\] token={third}',s,60);g['wait'](r'\[chastity.probe.refused\] reason=Declined',s,60);assert '[chastity.probe.received]'not in g['text']()[s:]
  s=len(g['text']());click('lock',vault);g['wait'](r'\[chastity.locked\] reason=user',s,60);g['focus'](consumer);time.sleep(.2);s=len(g['text']());click('request',consumer,True);g['wait'](r'\[chastity.probe.refused\] reason=Unavailable',s,60)
  text=g['text']();assert MASTER not in text and SECRET not in text;assert text.count('[chastity.probe.received] expected=1')==1;assert 'expected=0'not in text
  print({'ok':True,'persisted_vault':True,'denied_first':True,'approved_once':True,'consumer_run_changed':True,'locked_unavailable':True,'secret_serial_leak':False})
 finally:
  if g:g['stop']()
if __name__=='__main__':main()
