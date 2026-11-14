#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Moan's real Weft window: pointer-created tracks, Bell playback, split and undo.
Keeps screenshot and recorded sound; capture permission and file pickers are not
claimed by this transport/edit gate.
"""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from program_egui_gate import colours
from sound_gate import segments
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--microphone',action='store_true');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);install(disk,[('moan',(ROOT/'iso_root/programs/moan').read_bytes())])
 wav=out/'moan-app.wav';wav.unlink(missing_ok=True);os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 
 if a.microphone:os.environ['SEXOS_GATE_DEVICES']='-audiodev none,id=snd0 -device virtio-sound-pci,audiodev=snd0'
 g=boot(a.iso,disk,out,'moan-app',a.resolution);q=g['q'];text=g['text'];rw,rh=map(int,a.resolution.split('x'))
 def keys(line):
  for k in line:q.key({' ':'spc','-':'minus','.':'dot'}.get(k,k));time.sleep(.08)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);g['wait'](r'\[bell.sound.ready\]',0,120);g['focus'](153);time.sleep(.5);start=len(text());keys('run moan');q.key('ret')
  m=g['wait'](r'\[moan.ready\] target=(\d+) task=(\d+)',start,300);target,task=m[1],m[2]
  launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',start,60);surface=int(launch[1]);g['wait'](r'\[moan.control\] name=range_end ',start,60);time.sleep(2)
  pos={n:(int(x),int(y)) for n,x,y in re.findall(r'\[moan.control\] name=(\S+) x=(-?\d+) y=(-?\d+)',text()[start:])}
  tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',text())[-1]));ox,oy=tile[0]+8,tile[1]+36
  def click(name):
   x,y=pos[name];x+=ox;y+=oy
   q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.2)
   for down in [True,False]:q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.15)
  s=len(text());click('Tone');g['wait'](r'\[moan.project\] tracks=1 frames=96000 selected=0 clips=1',s,60)
  s=len(text());click('Play');g['wait'](r'\[moan.play\] tracks=1 from=0',s,60);g['wait'](r'\[moan.finished\] frames=192000',s,120)
  s=len(text());click('Tone');g['wait'](r'\[moan.project\] tracks=2 frames=96000 selected=1 clips=2',s,60)
  click('position');q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':d,'key':{'type':'qcode','data':k}}}for d,ks in ((True,('ctrl','a')),(False,('a','ctrl')))for k in ks]});keys('1')
  s=len(text());click('Split');g['wait'](r'\[moan.project\] tracks=2 frames=96000 selected=1 clips=3',s,60)
  s=len(text());click('Undo');g['wait'](r'\[moan.project\] tracks=2 frames=96000 selected=1 clips=2',s,60)
  if a.microphone:
   s=len(text());click('Record');asked=g['wait'](r'\[moan.record.request\] row=(\d+)',s,60);row=asked[1];time.sleep(2);assert '[moan.recorded]' not in text()[s:]
   g['focus'](153);time.sleep(.5);keys(f'volume {row} mic on');q.key('ret');g['wait'](r'\[bell.mic.first\]',s,120);time.sleep(2)
   g['focus'](surface);s=len(text());click('Stop');saved=g['wait'](r'\[moan.recorded\] frames=(\d+) lost=(\d+)',s,60);assert int(saved[1])>=48000,saved[0]
  screenshot=out/'moan.ppm';q.cmd('screendump',{'filename':str(screenshot)});time.sleep(.5);assert len(colours(screenshot.read_bytes(),tile))>8
  s=len(text());g['focus'](surface);q.key('f11');g['wait'](r'\[moan.closed\]',s,60);g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
  q.cmd('quit');g['proc'].wait(30);duration=None
  if not a.microphone:
   found=segments(wav);duration=sum(f[1] for f in found if abs(f[2]-400)<8);assert 1.9<duration<2.1,found
  print({'ok':True,'tone_seconds':duration,'edited_tracks':2,'split_undo':True,'microphone':a.microphone})
 finally:g['stop']()
if __name__=='__main__':main()
