#!/usr/bin/env python3
"""Measure a normal image loading/editing/saving a large fixture via real input."""
import argparse,hashlib,json,os,pathlib,re,shutil,subprocess,time
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from check_desktop_pixels import font_from_source
from check_quil_ui_pixels import verify
from check_document_disk import fnv,recover


def main():
    p=argparse.ArgumentParser();p.add_argument('--iso',type=pathlib.Path,required=True);p.add_argument('--source-disk',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--window',type=int,choices=[1,4],required=True);p.add_argument('--object',type=int,default=5);p.add_argument('--name',default='uialpha');a=p.parse_args()
    iso=a.iso.resolve(strict=True);src=a.source_disk.resolve(strict=True);out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise RuntimeError('fresh output required')
    expected=recover(src.read_bytes(),a.object)['payload']
    if not 0<len(expected)<12288:raise RuntimeError('fixture must allow one real appended character')
    disk=out/'nvme.img';shutil.copyfile(src,disk);sock=pathlib.Path('/tmp')/f'sex-document-window-{os.getpid()}.sock';log=out/'serial.log'
    cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-vga','none','-device','VGA,xres=1280,yres=800','-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown']
    digest=hashlib.sha256(iso.read_bytes()).hexdigest()
    (out/'manifest.json').write_text(json.dumps(dict(iso_sha256=digest,input_disk_sha256=hashlib.sha256(src.read_bytes()).hexdigest(),command=cmd,declared_window=a.window),indent=2)+'\n')
    proc=subprocess.Popen(cmd,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
    def text():return log.read_text(errors='replace')if log.exists()else''
    def wait(pattern,start=0,timeout=600):
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            if proc.poll()is not None:raise RuntimeError('guest exited')
            value=text();m=re.search(pattern,value[start:])
            if m:return m
            if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|\[sexfiles.defer.drop\]',value):raise RuntimeError('guest fault or lost request')
            time.sleep(.02)
        raise RuntimeError('timeout: '+pattern)
    try:
        end=time.monotonic()+15
        while not sock.exists():
            if time.monotonic()>end:raise RuntimeError('QMP unavailable')
            time.sleep(.05)
        q=QMP(sock)
        def key(code,pattern):start=len(text());q.key(code);return wait(pattern,start)
        def focus(sid):return focus_surface(sid,text,key)
        wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[linen.disk.publish.done\]')
        start=len(text());q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':30000}},{'type':'abs','data':{'axis':'y','value':30000}}]})
        wait(r'\[shell.abs.normalize\] raw_x=30000 raw_y=30000 .*reason=(?:ok|duplicate_sample)',start)
        focus(200)
        for _ in range(20):
            start=len(text());q.key('j');m=wait(r'\[shell.action.select_next_linen\] id=(\d+)',start);selected=int(m[1])
            wait(r'\[linen.object_select.current\] id='+str(selected)+r'(?:\s|$)',start)
            labels=re.findall(r'\[linen.list.label\] id=(\d+) row=\d+ text=([^\r\n]+)',text())
            if any(int(i)==selected and name==a.name for i,name in labels):break
        else:raise RuntimeError('named fixture never selected')
        start=len(text());sent=time.monotonic();q.key('ret')
        wait(r'\[linen.quil.disk_doc.intent\] object_id=\d+ path_id='+str(a.object)+r' ok=1',start)
        loaded=wait(r'\[quil.persist.load.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc='+str(a.object),start);load_seconds=time.monotonic()-sent
        if (int(loaded[1]),int(loaded[2],16))!=(len(expected),fnv(expected)):raise RuntimeError('guest loaded bytes differ from fixture')
        focus(201);key('z',r'\[quil.document.edit\].*bytes='+str(len(expected)+1)+r'(?:\s|$)')
        font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text())
        end=time.monotonic()+90;errors=['not painted']
        while time.monotonic()<end:
            q.cmd('screendump',{'filename':str(out/'edited.ppm')});errors=verify(text(),(out/'edited.ppm').read_bytes(),font,'z#',target=9,resolution=(1280,800))
            if not errors:break
            time.sleep(.1)
        if errors:raise RuntimeError('large editor glyphs: '+str(errors))
        start=len(text());sent=time.monotonic();q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'s'}],'hold-time':80})
        saved=wait(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc='+str(a.object),start);save_seconds=time.monotonic()-sent
        payload=expected+b'z'
        if (int(saved[1]),int(saved[2],16))!=(len(payload),fnv(payload)):raise RuntimeError('guest saved wrong revision')
        actual=recover(disk.read_bytes(),a.object)
        if actual.pop('payload')!=payload:raise RuntimeError('independent disk bytes differ')
        start=len(text());sent=time.monotonic();q.key('f11');wait(r'\[sext.app.pool.ready\] target=9 .*ok=1',start);close_seconds=time.monotonic()-sent
        roles={int(x)for x in re.findall(r'\[app.service.ack\] target=9 .*role=(\d+) ',text()[start:])}
        if roles!={1,2,4,8,16}:raise RuntimeError('missing terminal cleanup role')
        result=dict(ok=True,iso_sha256=digest,declared_window=a.window,load_bytes=len(expected),save_bytes=len(payload),load_seconds=load_seconds,save_seconds=save_seconds,close_seconds=close_seconds,observer_poll_seconds=.02,pixel_errors=[],disk_document=actual,roles=sorted(roles),scope='actual native input and large-document bytes; fixture is host-created; timing includes scheduler and storage, no hard latency bound')
        (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
    except Exception as error:
        (out/'result.json').write_text(json.dumps(dict(ok=False,error=str(error)))+'\n');raise
    finally:
        if proc.poll()is None:
            proc.terminate()
            try:proc.wait(timeout=10)
            except subprocess.TimeoutExpired:proc.kill();proc.wait()
        sock.unlink(missing_ok=True)
if __name__=='__main__':main()
