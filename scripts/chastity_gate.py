#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Real Chastity Weft UI: create/generate/add/save, lock and reject wrong password,
then reboot and unlock the persisted encrypted snapshot. Test credentials only.
"""
import argparse
import re
import shutil
import time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
PASSWORD='testonlymasterpassword'

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for n in ('iso','source-disk','out'):p.add_argument('--'+n,required=True,type=Path)
    p.add_argument('--resolution',default='1280x800');a=p.parse_args()
    out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
    install(disk,[('chastity',(ROOT/'iso_root/programs/chastity').read_bytes())])
    rw,rh=map(int,a.resolution.split('x'))
    def setup(tag):
        g=boot(a.iso,disk,out,tag,a.resolution);q=g['q'];t=g['text']
        def keys(text):
            for c in text:q.key({' ':'spc','.':'dot'}.get(c,c));time.sleep(.07)
        def launch():
            g['wait'](r'\[app.private.retired\] target=41 ',0,300);g['focus'](153);time.sleep(.5);s=len(t());keys('run chastity');q.key('ret')
            m=g['wait'](r'\[chastity.ready\] target=(\d+) task=(\d+)',s,300);target,task=m[1],m[2]
            launch=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);sid=int(launch[1]);g['focus'](sid);time.sleep(1);return target,task,sid
        def click(name,sid):
            time.sleep(.3);positions={name:(int(x),int(y)) for name,x,y in re.findall(r'\[chastity.control\] name=(\w+) x=(-?\d+) y=(-?\d+)',t())};assert name in positions,positions
            tile=list(map(int,re.findall(rf'\[shell.tile.apply\] sid={sid} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',t())[-1]));x,y=positions[name];x+=tile[0]+8;y+=tile[1]+36
            q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.2)
            for down in (True,False):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.15)
        return g,keys,launch,click
    g=None
    try:
        g,keys,launch,click=setup('chastity-create');target,task,sid=launch();t=g['text'];q=g['q']
        click('master',sid);keys(PASSWORD);click('confirm',sid);keys(PASSWORD);s=len(t());click('create',sid);g['wait'](r'\[chastity.created\]',s,120)
        click('title',sid);keys('fixture');click('origin',sid);keys('example.org');click('username',sid);keys('alice')
        s=len(t());click('generate',sid);g['wait'](r'\[chastity.generated\] length=24',s,60);click('add',sid)
        s=len(t());click('save',sid);g['wait'](r'\[chastity.saved\] bytes=\d+ synced=1',s,300)
        s=len(t());click('lock',sid);g['wait'](r'\[chastity.locked\] reason=user',s,60);click('master',sid);keys('wrongpassword');s=len(t());click('unlock',sid);g['wait'](r'\[chastity.refused\] kind=Authentication',s,300)
        q.cmd('screendump',{'filename':str(out/'locked-wrong-password.ppm')});assert PASSWORD not in t()
        g['stop']();g=None
        g,keys,launch,click=setup('chastity-reopen');target,task,sid=launch();t=g['text'];q=g['q'];click('master',sid);keys(PASSWORD);s=len(t());click('unlock',sid);g['wait'](r'\[chastity.unlocked\] entries=1 imported=0',s,300)
        q.cmd('screendump',{'filename':str(out/'unlocked-after-reboot.ppm')});assert PASSWORD not in t();s=len(t());q.key('f11');g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,120)
        print({'ok':True,'persisted_entries':1,'wrong_password_refused':True,'secret_serial_leak':False})
    finally:
        if g:g['stop']()
if __name__=='__main__':main()
