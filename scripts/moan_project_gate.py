#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Moan editable project gate: waveform drag/cut, Files save, reboot, held-route
reopen, another edit/undo, clean baseline, and actual timeline screenshots."""
import argparse,json,os,re,shutil,time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from program_egui_gate import colours
ROOT=Path(__file__).resolve().parents[1]
class Controls:
    def __init__(self,g,out,resolution):self.g=g;self.q=g['q'];self.text=g['text'];self.out=out;self.rw,self.rh=map(int,resolution.split('x'))
    def chord(self,*keys):
        self.q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':key}}}for down,group in ((True,keys),(False,tuple(reversed(keys))))for key in group]})
        time.sleep(.15)
    def type(self,text):
        for c in text:
            if c=='+':self.chord('shift','equal')
            elif c=='@':self.chord('shift','2')
            else:self.q.key({' ':'spc','-':'minus','.':'dot','/':'slash'}.get(c,c))
            time.sleep(.07)
    def run(self,line):
        self.g['focus'](153);time.sleep(.3);start=len(self.text());self.type(line);self.q.key('ret');return start
    def launch(self,name,line=None):
        start=self.run(line or 'run '+name);m=self.g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',start,120)
        return start,int(m[1]),int(m[2]),int(m[3])
    def focus(self,surface):self.g['focus'](surface);time.sleep(.4)
    def tile(self,surface):return list(map(int,re.findall(rf'\[shell.tile.apply\] sid={surface} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',self.text())[-1]))
    def move(self,x,y):
        self.q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(max(0,min(self.rw-1,x))*32767/(self.rw-1))}},{'type':'abs','data':{'axis':'y','value':round(max(0,min(self.rh-1,y))*32767/(self.rh-1))}}]});time.sleep(.2)
    def button(self,down):self.q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.2)
    def timeline(self,surface):
        self.focus(surface);start=len(self.text());self.chord('ctrl','t');m=self.g['wait'](r'\[moan.timeline\] x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) start=(\d+) span=(\d+)',start,60)
        x,y,w,h,start_frame,span=map(int,m.groups());tile=self.tile(surface)
        return tile[0]+8+x,tile[1]+36+y,w,h,start_frame,span
    def drag(self,surface,left,right,track=1):
        x,y,w,h,_,_=self.timeline(surface);row=y+20+track*56+25;start=len(self.text());self.move(x+left,row);self.button(True);self.move(x+right,row);self.button(False)
        m=self.g['wait'](rf'\[moan.selection\] track={track} cursor=\d+ from=(\d+) to=(\d+) dirty=\d',start,60)
        # The initial Down receipt is empty selection; wait for nonempty final range.
        self.g['wait'](rf'\[moan.selection\] track={track} cursor=\d+ from=[1-9]\d* to=[1-9]\d* dirty=\d',start,60)
    def screenshot(self,surface,name):
        self.focus(surface);time.sleep(.5);path=self.out/name;self.q.cmd('screendump',{'filename':str(path)});time.sleep(.3);assert len(colours(path.read_bytes(),self.tile(surface)))>8
    def close(self,surface,target,task,dirty=False):
        self.focus(surface);start=len(self.text());self.q.key('f11')
        if dirty:time.sleep(.4);self.q.key('tab');self.q.key('ret')
        self.g['wait'](r'\[moan.closed\]',start,60);self.g['wait'](rf'\[app.private.retired\] target={target} task={task} ',start,120)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('iso','source-disk','out'):parser.add_argument('--'+name,required=True,type=Path)
    parser.add_argument('--resolution',default='1280x800');a=parser.parse_args();out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
    disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);install(disk,[(name,(ROOT/'iso_root/programs'/name).read_bytes())for name in ('moan','files')]);os.environ.setdefault('SEXOS_GATE_BOUND','1200')
    g=boot(a.iso,disk,out,'project-save',a.resolution)
    try:
        c=Controls(g,out,a.resolution);g['wait'](r'\[app.private.retired\] target=41 ',0,180)
        start,_,_,files_surface=c.launch('files','run +fs files');g['wait'](r'\[files.app.ready\].*portal=1',start,180)
        start,target,task,surface=c.launch('moan');g['wait'](r'\[moan.ready\]',start,300);c.focus(surface)
        for n in (1,2):
            start=len(c.text());c.chord('ctrl','shift','t');g['wait'](rf'\[moan.project\] tracks={n} frames=96000',start,60)
        c.drag(surface,70,210);start=len(c.text());c.chord('ctrl','shift','k');g['wait'](r'\[moan.project\] tracks=2 frames=96000 selected=1 clips=3',start,60)
        c.screenshot(surface,'edited-waveform.ppm')
        start=len(c.text());c.chord('ctrl','s');g['wait'](r'\[files.app.question\] program=moan mode=Save',start,120)
        c.focus(files_surface);c.chord('ctrl','h');c.chord('ctrl','s')
        # Files' raw SaveAs editor has no Ctrl+A: erase its project.moan hint.
        for _ in range(24):c.q.key('backspace');time.sleep(.03)
        c.type('edit.moan');c.q.key('ret');saved=g['wait'](r'\[moan.project.saved\] bytes=(\d+) tracks=2 clips=3 hash=(0x[0-9a-f]+) synced=1',start,180)
        saved_bytes,saved_hash=int(saved[1]),saved[2];c.close(surface,target,task)
        c.q.cmd('quit');g['proc'].wait(30)
    finally:g['stop']()
    # A new boot must read the project from the synchronized disk, not old memory.
    g=boot(a.iso,disk,out,'project-reopen',a.resolution)
    try:
        c=Controls(g,out,a.resolution);g['wait'](r'\[app.private.retired\] target=41 ',0,180)
        start,target,task,surface=c.launch('moan','run moan @/edit.moan')
        g['wait'](rf'\[moan.project.opened\] bytes={saved_bytes} tracks=2 clips=3 hash={saved_hash} sources_shared=1 clean=1',start,300)
        g['wait'](r'\[moan.ready\]',start,120);c.timeline(surface);c.screenshot(surface,'reopened-waveform.ppm')
        # The reopened clip can still be edited, then undone to its clean baseline.
        x,y,w,h,_,_=c.timeline(surface);c.move(x+490,y+20+56+25);c.button(True);c.button(False)
        start=len(c.text());c.chord('ctrl','e');g['wait'](r'\[moan.project\] tracks=2 frames=96000 selected=1 clips=4',start,60)
        start=len(c.text());c.chord('ctrl','z');g['wait'](r'\[moan.project\] tracks=2 frames=96000 selected=1 clips=3',start,60);g['wait'](r'\[moan.selection\].*dirty=0',start,60)
        c.screenshot(surface,'reopened-edit-undo.ppm');c.close(surface,target,task)
        result=dict(ok=True,bytes=saved_bytes,hash=saved_hash,tracks=2,clips=3,reboot_reopen=True,pointer_cut=True,edit_undo_baseline=True)
        (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(result)
    finally:g['stop']()
if __name__=='__main__':main()
