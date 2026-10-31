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

def focus_observation(trace,target):
    drawn=re.search(r'\[display.focus.drawn\] focus='+str(target)+r' ordinal=(\d+) kind=(full|strip) tick=(\d+) pixels=unconfirmed',trace)
    if not drawn:raise RuntimeError('missing exact focus redraw')
    record=re.search(r'\[display.repaint\] ordinal='+drawn[1]+r' ([^\r\n]+)',trace)
    if not record:raise RuntimeError('missing paired repaint record')
    fields=dict(re.findall(r'(\w+)=(\w+)',record[1]))
    if int(fields.get('seen','0'))&4==0 or fields.get('focus')!=str(target) or fields.get('kind')!=drawn[2]:
        raise RuntimeError('focus inherited unrelated repaint')
    return dict(ordinal=int(drawn[1]),kind=drawn[2],seen=int(fields['seen']))

def indicator_colors(root):
    def body(path,signature):
        source=path.read_text();start=source.index(signature);i=source.index('{',start)+1;depth=1
        while depth:depth+=(source[i]=='{')-(source[i]=='}');i+=1
        return source[start:i]
    fallback=body(root/'servers/sexdisplay/src/main.rs','const fn app_indicator_color(')
    native=body(root/'crates/sex-pdx/src/desktop_style.rs','pub const fn native_indicator(')
    colors={}
    for source in [fallback,native]:
        for ids,color in re.findall(r'([0-9|]+)\s*=>\s*Some\(0x([0-9A-Fa-f]+)\)',source):
            for sid in ids.split('|'):colors[int(sid)]=int(color,16)&0xffffff
    families=dict((name,int(value,16)&0xffffff)for name,value in re.findall(r'Some\(App::(\w+)\)\s*=>\s*Some\(0x([0-9A-Fa-f]+)\)',native))
    if families:
        from quil_reserve_catalog import surface_families
        for sid,family in surface_families(root).items():
            if family not in families:raise RuntimeError('native family indicator missing')
            colors[sid]=families[family]
    unknown=re.search(r'_\s*=>\s*Some\(0x([0-9A-Fa-f]+)\)',fallback)
    if not colors or not unknown:raise RuntimeError('indicator style declarations unavailable')
    colors['unknown']=int(unknown[1],16)&0xffffff
    return colors

def verify_indicator(ppm,target,resolution,colors=None):
    from check_disk_persistence import ppm_rgb
    w,h,pixels=ppm_rgb(ppm,min_width=1,min_height=1)
    if(w,h)!=resolution:raise RuntimeError('focus screenshot dimensions differ')
    if colors is None:colors=indicator_colors(pathlib.Path(__file__).resolve().parents[1])
    color=colors.get(target,colors['unknown'])
    expected=color.to_bytes(3,'big')
    if any(pixels[(y*w+x)*3:(y*w+x)*3+3]!=expected for y in range(22,26)for x in range(155,159)):
        raise RuntimeError('actual active-app indicator pixels differ')

def main():
    p=argparse.ArgumentParser();p.add_argument('--iso',type=pathlib.Path,required=True);p.add_argument('--source-disk',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--no-legacy',action='store_true');p.add_argument('--resolution',choices=['1280x800','1024x768'],default='1280x800');p.add_argument('--switch-cycles',type=int,default=1);p.add_argument('--focus-feedback',action='store_true');a=p.parse_args()
    if not 1<=a.switch_cycles<=10:p.error('switch cycles must be1..10')
    iso=a.iso.resolve(strict=True);src=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise RuntimeError('fresh output required')
    disk=out/'nvme.img';shutil.copyfile(src,disk);log=out/'serial.log';sock=pathlib.Path('/tmp')/f'sex-switch-{os.getpid()}.sock'
    cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-vga','none','-device','VGA,xres='+a.resolution.split('x')[0]+',yres='+a.resolution.split('x')[1],'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown']
    digest=hashlib.sha256(iso.read_bytes()).hexdigest()
    (out/'manifest.json').write_text(json.dumps(dict(iso_sha256=digest,input_disk_sha256=hashlib.sha256(src.read_bytes()).hexdigest(),command=cmd,switch_cycles=a.switch_cycles,focus_feedback=a.focus_feedback),indent=2)+'\n')
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
        pointer_start=len(text());q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':30000}},{'type':'abs','data':{'axis':'y','value':30000}}]})
        wait(r'\[shell.abs.normalize\] raw_x=30000 raw_y=30000 .*reason=(?:ok|duplicate_sample)',pointer_start)
        key('scroll_lock',r'\[shell.focus.set\] id=153')
        start=len(text());q.key_sequence(list('echo'));wait(r'\[spindle.cursor.view\] target=12 .*pos=4 len=4 ',start)
        font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text())
        colors=indicator_colors(pathlib.Path(__file__).resolve().parents[1])
        switches=[];input_pixels=[];routes=[];resolution=tuple(map(int,a.resolution.split('x')))
        for cycle in range(a.switch_cycles):
            route=[]
            for _ in range(8):
                start=len(text());sent=time.monotonic()
                q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':code}}}for code,down in [('alt',True),('tab',True),('tab',False),('alt',False)]]})
                m=wait(r'\[shell.app.switch\] old=(\d+) new=(\d+) live=1 shortcut=alt_tab',start)
                item=dict(old=int(m[1]),new=int(m[2]),host_seconds=time.monotonic()-sent,cycle=cycle)
                if a.focus_feedback:
                    wait(r'\[display.focus.drawn\] focus='+m[2]+r' ',start)
                    item['drawn_seconds']=time.monotonic()-sent;item.update(focus_observation(text()[start:],int(m[2])))
                    image_path=out/f'focus-{cycle}-{len(route)}.ppm';q.cmd('screendump',{'filename':str(image_path)})
                    verify_indicator(image_path.read_bytes(),int(m[2]),resolution,colors);item['pixels_seconds']=time.monotonic()-sent
                switches.append(item);route.append(item['new'])
                if item['new']==153:break
            else:raise RuntimeError('global switch failed to return to original live Spindle')
            if switches[-len(route)]['old']!=153 or route[0]==153:raise RuntimeError('global chord did not leave Spindle')
            if routes and route!=routes[0]:raise RuntimeError('live route changed between ordinary switch cycles')
            routes.append(route)
            # No modifier or Tab state may leak to either editor across cycles.
            sent=time.monotonic();key('z',rf'\[spindle.cursor.view\] target=12 .*pos={5+cycle} len={5+cycle} ')
            expected='> echo'+'z'*(cycle+1);errors=['not painted'];end=time.monotonic()+90
            while time.monotonic()<end:
                q.cmd('screendump',{'filename':str(out/'switched-input.ppm')})
                errors=verify(text(),(out/'switched-input.ppm').read_bytes(),font,12,expected,False,False,resolution=resolution)
                if not errors:break
                time.sleep(.15)
            if errors:raise RuntimeError('returned editor glyphs: '+str(errors))
            input_pixels.append(time.monotonic()-sent)
        start=len(text());q.key('tab');wait(r'\[silk-shell.key.route\] target=spindle ',start)
        if '[shell.app.switch]'in text()[start:] or re.findall(r'\[shell.focus.set\] id=(\d+)',text())[-1]!='153':raise RuntimeError('plain Tab escaped app-local completion')
        key('f11',r'\[shell.app.close.request\] target=12 ')
        wait(r'\[sext.app.pool.ready\] target=12 .*ok=1')
        roles={int(x)for x in re.findall(r'\[app.service.ack\] target=12 .*role=(\d+) ',text())}
        if roles!={1,2,4,8,16}:raise RuntimeError('incomplete Spindle cleanup roles')
        if a.no_legacy and re.search(r'\[shell\.(?:focus\.set|tile\.apply)\] (?:id|sid)=10[0-3](?:\s|$)',text()):raise RuntimeError('uncreated legacy placeholder entered desktop')
        result=dict(ok=True,resolution=a.resolution,legacy_placeholders_absent=a.no_legacy,iso_sha256=digest,switches=switches,switch_cycles=a.switch_cycles,routes=routes,input_to_checked_prompt_seconds=input_pixels,plain_tab_local=True,returned_text=expected,pixel_errors=errors,cleanup_roles=sorted(roles),scope='actual normal-image repeated Alt+Tab without minimize/restore/retile helpers; exact redraw correlation and indicator pixels when enabled, complete returned prompt glyphs and terminal cleanup. Host observations include capture; no hard bound or isolated performance comparison.')
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
