#!/usr/bin/env python3
"""Real global Alt+Tab and app-local Spindle Tab, with terminal cleanup."""
import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import time
from desktop_close_gate import QMP
from check_desktop_pixels import font_from_source
from check_spindle_viewport_pixels import verify


def main():
    p=argparse.ArgumentParser();p.add_argument('--iso',type=pathlib.Path,required=True);p.add_argument('--source-disk',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--no-legacy',action='store_true');p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');a=p.parse_args()
    iso=a.iso.resolve(strict=True);src=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise RuntimeError('fresh output required')
    disk=out/'nvme.img';shutil.copyfile(src,disk);log=out/'serial.log';sock=pathlib.Path('/tmp')/f'sex-switch-{os.getpid()}.sock'
    cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-vga','none','-device','VGA,xres='+a.resolution.split('x')[0]+',yres='+a.resolution.split('x')[1],'-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown']
    digest=hashlib.sha256(iso.read_bytes()).hexdigest()
    (out/'manifest.json').write_text(json.dumps(dict(iso_sha256=digest,input_disk_sha256=hashlib.sha256(src.read_bytes()).hexdigest(),command=cmd),indent=2)+'\n')
    proc=subprocess.Popen(cmd,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
    def text():return log.read_text(errors='replace')if log.exists()else''
    def wait(pattern,start=0,timeout=180):
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            if proc.poll()is not None:raise RuntimeError('guest exited')
            value=text();m=re.search(pattern,value[start:])
            if m:return m
            if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at',value):raise RuntimeError('guest fault')
            time.sleep(.05)
        raise RuntimeError('timeout: '+pattern)
    try:
        end=time.monotonic()+15
        while not sock.exists():
            if time.monotonic()>end:raise RuntimeError('QMP unavailable')
            time.sleep(.05)
        q=QMP(sock)
        def key(code,pattern):start=len(text());q.key(code);return wait(pattern,start)
        if a.resolution=='1024x768':
            wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
        wait(r'\[shell.app.registered.reply\] target=15 .*committed=1')
        wait(r'\[spindle.history.receipt\] target=12 .*kind=loaded .*error=0')
        key('scroll_lock',r'\[shell.focus.set\] id=153')
        start=len(text());q.key_sequence(list('echo'));wait(r'\[spindle.cursor.view\] target=12 .*pos=4 len=4 ',start)
        switches=[]
        for _ in range(8):
            start=len(text());sent=time.monotonic()
            q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':code}}}for code,down in [('alt',True),('tab',True),('tab',False),('alt',False)]]})
            m=wait(r'\[shell.app.switch\] old=(\d+) new=(\d+) live=1 shortcut=alt_tab',start)
            switches.append(dict(old=int(m[1]),new=int(m[2]),host_seconds=time.monotonic()-sent))
            if int(m[2])==153:break
        else:raise RuntimeError('global switch failed to return to original live Spindle')
        if switches[0]['old']!=153 or switches[0]['new']==153:raise RuntimeError('global chord did not leave Spindle')
        # No modifier or Tab state may leak to either editor across focus changes.
        key('z',r'\[spindle.cursor.view\] target=12 .*pos=5 len=5 ')
        font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text())
        errors=['not painted'];end=time.monotonic()+90
        while time.monotonic()<end:
            q.cmd('screendump',{'filename':str(out/'switched-input.ppm')})
            errors=verify(text(),(out/'switched-input.ppm').read_bytes(),font,12,'> echoz',False,False,resolution=tuple(map(int,a.resolution.split('x'))))
            if not errors:break
            time.sleep(.15)
        if errors:raise RuntimeError('returned editor glyphs: '+str(errors))
        start=len(text());q.key('tab');wait(r'\[silk-shell.key.route\] target=spindle ',start)
        if '[shell.app.switch]'in text()[start:] or re.findall(r'\[shell.focus.set\] id=(\d+)',text())[-1]!='153':raise RuntimeError('plain Tab escaped app-local completion')
        key('f11',r'\[shell.app.close.request\] target=12 ')
        wait(r'\[sext.app.pool.ready\] target=12 .*ok=1')
        roles={int(x)for x in re.findall(r'\[app.service.ack\] target=12 .*role=(\d+) ',text())}
        if roles!={1,2,4,8,16}:raise RuntimeError('incomplete Spindle cleanup roles')
        if a.no_legacy and re.search(r'\[shell\.(?:focus\.set|tile\.apply)\] (?:id|sid)=10[0-3](?:\s|$)',text()):raise RuntimeError('uncreated legacy placeholder entered desktop')
        result=dict(ok=True,resolution=a.resolution,legacy_placeholders_absent=a.no_legacy,iso_sha256=digest,switches=switches,plain_tab_local=True,returned_text='> echoz',pixel_errors=errors,cleanup_roles=sorted(roles),scope='actual normal-image keyboard chords, untouched app-local Tab/editor state, complete returned prompt glyphs and terminal cleanup; no hidden frame activation')
        (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
    except Exception as e:
        (out/'result.json').write_text(json.dumps(dict(ok=False,error=str(e)))+'\n');raise
    finally:
        if proc.poll()is None:
            proc.terminate()
            try:proc.wait(timeout=10)
            except subprocess.TimeoutExpired:proc.kill();proc.wait()
        sock.unlink(missing_ok=True)
if __name__=='__main__':main()
