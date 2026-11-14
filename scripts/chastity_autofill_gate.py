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
from lua_gate import qcodes
from quil_autosave_gate import boot
from program_gate_geometry import maximize, current_tile
import os
os.environ.setdefault("SEXOS_GATE_BOUND","2400")
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
  for c in text:
   g['q'].cmd('send-key',{'keys':[{'type':'qcode','data':k}for k in qcodes(c)],'hold-time':40});time.sleep(.12)
 def launch(name,marker):
  g['focus'](153);time.sleep(.4);s=len(g['text']());keys('run '+name);g['q'].key('ret');m=g['wait'](marker,s,300);target,task=int(m[1]),int(m[2]);r=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);sid=int(r[1]);g['focus'](sid);time.sleep(.6);return target,task,sid
 def click(name,sid,probe=False):
  text=g['text']();prefix='chastity.probe.control'if probe else'chastity.control';pos={n:(int(x),int(y))for n,x,y in re.findall(r'\['+re.escape(prefix)+r'\] name=(\w+) x=(-?\d+) y=(-?\d+)',text)};assert name in pos,pos
  tile=current_tile(text,sid);x,y=pos[name];x+=tile[0]+8;y+=tile[1]+36
  g['q'].cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.2)
  for down in (True,False):g['q'].cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.12)
 try:
  g=start('autofill-save');g['wait'](r'\[app.private.retired\] target=41 ',0,300);_,_,vault=launch('chastity',r'\[chastity.window\] target=(\d+) task=(\d+)')
  keys(MASTER);g['q'].key('ret');time.sleep(.5);keys(MASTER);s=len(g['text']());g['q'].key('ret');g['wait'](r'\[chastity.created\] entries=0',s,900)
  g['q'].key('a');time.sleep(.5)
  for value in ['fixture','alice',SECRET,'https://example.org','fixture notes','']:
   keys(value);s=len(g['text']());g['q'].key('ret');time.sleep(.3)
  g['wait'](r'\[chastity.saved\] entries=1 bytes=\d+',s,900);g['stop']();g=None
  g=start('autofill-live');g['wait'](r'\[app.private.retired\] target=41 ',0,300);_,_,vault=launch('chastity',r'\[chastity.window\] target=(\d+) task=(\d+)');keys(MASTER);s=len(g['text']());g['q'].key('ret');g['wait'](r'\[chastity.unlocked\] entries=1 ',s,900);g['wait'](r'\[chastity.autofill.ready\]',s,120)
  target,task,consumer=launch('chastity-probe',r'\[chastity.probe.ready\] target=(\d+) task=(\d+)');s=len(g['text']());g['q'].key('ctrl-i');g['wait'](r'\[chastity.probe.impersonation\] denied=1',s,60)
  tile,resize=maximize(g,vault);g['wait'](r'\[chastity.layout\] width=\d+ height=\d+',resize,60);assert tile[2]>=816 and tile[3]>=642,tile
  def request():
   g['focus'](consumer);time.sleep(.2);s=len(g['text']());g['q'].key('ctrl-r');g['wait'](r'\[chastity.probe.requested\]',s,60);m=g['wait'](rf'\[chastity.autofill.prompt\] token=(\d+) domain={target} task={task}',s,120);assert '[chastity.probe.received]'not in g['text']()[s:];return s,int(m[1])
  s,first=request();g['focus'](vault);time.sleep(.5);g['q'].cmd('screendump',{'filename':str(out/'consent-default-deny.ppm')});g['q'].key('ret');g['wait'](rf'\[chastity.autofill.denied\] token={first}',s,60);g['wait'](r'\[chastity.probe.refused\] reason=Declined',s,60);assert '[chastity.probe.received]'not in g['text']()
  s,second=request();assert second!=first;g['focus'](vault);time.sleep(.3);g['q'].key('tab');g['q'].key('ret');g['wait'](rf'\[chastity.autofill.approved\] token={second}',s,60);g['wait'](r'\[chastity.probe.received\] expected=1',s,120)
  # A reused consumer row/task cannot inherit the previous approval.
  g['focus'](consumer);s=len(g['text']());g['q'].key('f11');g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120);old_task=task
  target,task,consumer=launch('chastity-probe',r'\[chastity.probe.ready\] target=(\d+) task=(\d+)');assert task!=old_task
  tile,resize=maximize(g,vault);g['wait'](r'\[chastity.layout\] width=\d+ height=\d+',resize,60);s,third=request();assert third not in(first,second);g['focus'](vault);time.sleep(.3);g['q'].key('ret');g['wait'](rf'\[chastity.autofill.denied\] token={third}',s,60);g['wait'](r'\[chastity.probe.refused\] reason=Declined',s,60);assert '[chastity.probe.received]'not in g['text']()[s:]
  g['focus'](vault);time.sleep(.3);s=len(g['text']());g['q'].key('l');g['wait'](r'\[chastity.locked\] reason=asked',s,60);g['focus'](consumer);time.sleep(.2);s=len(g['text']());g['q'].key('ctrl-r');g['wait'](r'\[chastity.probe.refused\] reason=Unavailable',s,60)
  text=g['text']();assert MASTER not in text and SECRET not in text;assert text.count('[chastity.probe.received] expected=1')==1;assert 'expected=0'not in text
  print({'ok':True,'persisted_vault':True,'denied_first':True,'approved_once':True,'consumer_run_changed':True,'locked_unavailable':True,'secret_serial_leak':False})
 finally:
  if g:g['stop']()
if __name__=='__main__':main()
