#!/usr/bin/env python3
"""Actual text/input/Close through all three finite Quil incarnations."""
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
from check_quil_ui_pixels import verify


def main():
    p=argparse.ArgumentParser();p.add_argument('--iso',type=pathlib.Path,required=True);p.add_argument('--source-disk',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');a=p.parse_args()
    iso=a.iso.resolve(strict=True);src=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise RuntimeError('fresh output required')
    disk=out/'nvme.img';shutil.copyfile(src,disk);log=out/'serial.log';sock=pathlib.Path('/tmp')/f'sex-quil-chain-{os.getpid()}.sock'
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
        from desktop_focus import focus_surface
        def focus(sid):return focus_surface(sid,text,key)
        font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text())
        proof=[]
        for target,sid,letter in [(9,201,'a'),(16,301,'b'),(18,307,'c')]:
            if target!=9:
                offset=len(text());key('grave_accent',r'\[shell.palette.item\] idx=0')
                key('j',r'\[shell.palette.select\] old=0 new=1')
                key('ret',rf'\[shell.app.relaunch\] predecessor={9 if target==16 else 16} successor={target} ')
                wait(rf'\[app.surface.presented\] target={target} surface={sid} .*ok=1',offset)
            focus(sid)
            offset=len(text());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'n'}],'hold-time':80})
            wait(r'\[quil.new.ok\] bytes=0',offset)
            key(letter,r'\[quil.document.edit\]')
            errors=['not painted'];end=time.monotonic()+90
            while time.monotonic()<end:
                q.cmd('screendump',{'filename':str(out/f'quil-{target}.ppm')})
                errors=verify(text(),(out/f'quil-{target}.ppm').read_bytes(),font,letter+'#',target=target,resolution=tuple(map(int,a.resolution.split('x'))))
                if not errors:break
                time.sleep(.15)
            if errors:raise RuntimeError(f'Quil {target} glyphs: '+str(errors))
            offset=len(text());key('f11',rf'\[shell.app.close.request\] target={target} ')
            wait(rf'\[sext.app.pool.ready\] target={target} .*ok=1',offset)
            roles={int(v)for v in re.findall(rf'\[app.service.ack\] target={target} .*role=(\d+) ',text()[offset:])}
            if roles!={1,2,4,8,16}:raise RuntimeError('missing Quil cleanup role')
            proof.append(dict(target=target,surface=sid,pixel_errors=errors,roles=sorted(roles),terminal=True))
        key('grave_accent',r'\[shell.palette.item\] idx=0')
        wait(r'\[shell.palette.item\] idx=1 .*status=CLOSED / RESERVE EXHAUSTED')
        result=dict(ok=True,iso_sha256=digest,resolution=a.resolution,incarnations=proof,reserve_exhausted=True,
                    scope='actual new-note input, exact glyphs, five-role cleanup and zero/reuse through the three finite BOOT subjects')
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
