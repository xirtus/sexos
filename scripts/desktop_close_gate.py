#!/usr/bin/env python3
"""Run an existing normal ISO; never builds. Real QMP keyboard → shell Close."""
import argparse,hashlib,json,pathlib,re,socket,subprocess,time,os
from check_desktop_close_log import check,responsiveness
from check_desktop_pixels import verify as verify_pixels, verify_hidden, verify_kaleido_layout, final_view, font_from_source
class QMP:
    def __init__(self,path):
        self.sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);self.sock.settimeout(5);self.sock.connect(str(path));self.file=self.sock.makefile('rb');self.seq=0;self.read();self.cmd('qmp_capabilities')
    def read(self):
        line=self.file.readline()
        if not line:raise RuntimeError('QMP closed')
        return json.loads(line)
    def cmd(self,op,args=None):
        self.seq+=1;request={'execute':op,'id':self.seq}
        if args is not None:request['arguments']=args
        self.sock.sendall((json.dumps(request)+'\n').encode())
        while True:
            response=self.read()
            if response.get('id')==self.seq:
                if 'error'in response:raise RuntimeError(str(response))
                return response.get('return')
    def key(self,key):self.cmd('send-key',{'keys':[{'type':'qcode','data':key}],'hold-time':80})
    def key_sequence(self,keys):
        # QMP send-key serializes its hold-time before the next key. Submit
        # ordered hardware events together to measure guest service overlap.
        self.cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':key}}} for key in keys for down in (True,False)]})
def main():
    p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);p.add_argument('--qemu',default='qemu-system-x86_64');p.add_argument('--cpu',default='max,+pku');args=p.parse_args()
    iso=args.iso.resolve(strict=True);out=args.out.resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise RuntimeError('output directory must be empty for fresh evidence')
    log=out/'serial.log';disk=out/'nvme.img';sock=pathlib.Path('/tmp')/f'sex-close-{os.getpid()}.sock'
    with disk.open('wb')as file:file.truncate(1024*1024)
    (out/'manifest.json').write_text(json.dumps({'iso':str(iso),'sha256':hashlib.sha256(iso.read_bytes()).hexdigest(),'profile':'normal','cpu':args.cpu,'transport':'real_QMP_keyboard'},indent=2)+'\n')
    proc=subprocess.Popen([args.qemu,'-M','q35','-cpu',args.cpu,'-m','512M','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown'],stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
    def text():return log.read_text(errors='replace')if log.exists()else''
    def wait(pattern,start=0,timeout=90):
        deadline=time.monotonic()+timeout
        while time.monotonic()<deadline:
            if proc.poll()is not None:raise RuntimeError('QEMU exited')
            value=text();match=re.search(pattern,value[start:])
            if match:return match
            if re.search(r'KERNEL PAGE FAULT|DOUBLE FAULT|KERNEL PANIC|GP FAULT|panicked at',value):raise RuntimeError('guest fault')
            time.sleep(.1)
        raise RuntimeError(f'timed out waiting for {pattern}')
    actions=[]
    try:
        deadline=time.monotonic()+15
        while not sock.exists():
            if time.monotonic()>deadline:raise RuntimeError('QMP unavailable')
            time.sleep(.1)
        q=QMP(sock)
        def key(name,pattern,timeout=15):
            start=len(text());q.key(name);actions.append({'key':name,'log_offset':start,'host_time':time.monotonic()});return wait(pattern,start,timeout)
        # The display's ordinary first-winning-pixel marker may precede the
        # app's correlated request and the shell's queued geometry/chrome.
        wait(r'\[app.surface.presented\] target=15 surface=300 sequence=1 ok=1',timeout=180)
        wait(r'\[shell.app.registered.reply\] target=15 task=\d+ surface=300 committed=1',timeout=180)
        wait(r'\[linen.disk.publish.done\]',timeout=180)
        font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text())
        startup_errors=['not rendered'];deadline=time.monotonic()+90
        while time.monotonic()<deadline:
            q.cmd('screendump',{'filename':str(out/'before-close.ppm')})
            current=text();framebuffer=(out/'before-close.ppm').read_bytes()
            startup_errors=verify_kaleido_layout(current,framebuffer,font)+verify_pixels(current,framebuffer,font)
            if not startup_errors:break
            time.sleep(.2)
        (out/'startup-layout-pixels.json').write_text(json.dumps({'ok':not startup_errors,'errors':startup_errors,'scope':'native app tiled glyphs and complete committed Linen labels after correlated presentation'},indent=2)+'\n')
        if startup_errors:raise RuntimeError('startup layout pixels: '+'; '.join(startup_errors))
        def focus(sid):
            for _ in range(16):
                matches=re.findall(r'\[shell.focus.set\] id=(\d+)',text())
                if matches and int(matches[-1])==sid:return
                key('tab',r'\[shell.focus.set\] id=\d+')
            raise RuntimeError(f'could not focus {sid}')
        def close(target,sid):
            focus(sid);key('f11',rf'\[shell.app.close.request\] target={target} task=\d+ surface={sid}')
            wait(rf'\[sext.app.pool.ready\] target={target} task=\d+ .*ok=1',timeout=180)
            q.cmd('screendump',{'filename':str(out/f'after-close-{target}.ppm')})
        close(15,300)
        # Existing Quil shell frame survives independently until its own close.
        close(9,201)
        key('scroll_lock',r'\[shell.focus.set\] id=153')
        close(12,153)
        # Run real rescan and enqueue J immediately, without awaiting backend.
        # A too-fast completion is not overlap evidence: retry naturally at most3.
        measured=[]
        for attempt in range(3):
            key('grave_accent',r'\[shell.palette.item\] idx=0')
            key('j',r'\[shell.palette.select\] old=0 new=1')
            key('j',r'\[shell.palette.select\] old=1 new=2')
            start=len(text())
            sent=time.monotonic();q.key_sequence(['ret','j'])
            actions.append({'key':'ret','log_offset':start,'host_time':sent,'transport':'ordered_input_events'})
            actions.append({'key':'j','log_offset':start,'host_time':sent,'during_refresh':True,'transport':'ordered_input_events'})
            wait(r'\[shell.action.select_next_linen\] id=[1-9]\d*',start,30)
            latency=time.monotonic()-sent
            wait(r'\[linen.remote.snapshot.ok\] count=[1-9]\d* .*complete=1',start,90)
            wait(r'\[linen.object_select.current\] id=[1-9]\d*',start,30)
            # Let completion repaint finish so its current-selection record exists.
            deadline=time.monotonic()+10
            while time.monotonic()<deadline and not responsiveness(text(),start):time.sleep(.1)
            evidence=responsiveness(text(),start)
            measured.append({'attempt':attempt,'host_key_to_action_seconds':latency,'overlap':bool(evidence),'evidence':evidence})
            if evidence:break
        (out/'responsiveness.json').write_text(json.dumps({'scope':'host observed functional overlap; no hard real-time bound','attempts':measured},indent=2)+'\n')
        if not any(m['overlap']for m in measured):raise RuntimeError('no measured input/refresh overlap after3 attempts')
        key('j',r'\[shell.action.select_next_linen\] id=[1-9]\d*')
        # Wait for real filename glyphs, not merely a queued paint or selection log.
        deadline=time.monotonic()+90
        pixel_errors=['not rendered']
        while time.monotonic()<deadline:
            q.cmd('screendump',{'filename':str(out/'survivors.ppm')})
            pixel_errors=verify_pixels(text(),(out/'survivors.ppm').read_bytes(),font)
            if not pixel_errors:break
            time.sleep(.2)
        (out/'filename-pixels.json').write_text(json.dumps({'ok':not pixel_errors,'errors':pixel_errors,'scope':'actual filename and selected-marker glyphs in guest framebuffer'},indent=2)+'\n')
        if pixel_errors:raise RuntimeError('filename pixels: '+'; '.join(pixel_errors))
        # Exercise the real hide/restore path after proving this text is visible.
        # A successful queued operation cannot substitute for disappearing ink
        # and complete restored glyphs with the same selected object.
        visible_intent=text();selected=final_view(visible_intent)[0]['selected']
        key('f8',r'\[silk.lifecycle.minimize.hidden\] sid=200 frame=\d+')
        hidden_errors=['not hidden'];deadline=time.monotonic()+90
        while time.monotonic()<deadline:
            q.cmd('screendump',{'filename':str(out/'linen-minimized.ppm')})
            hidden_errors=verify_hidden(visible_intent,(out/'linen-minimized.ppm').read_bytes())
            if not hidden_errors:break
            time.sleep(.2)
        if hidden_errors:raise RuntimeError('minimized pixels: '+'; '.join(hidden_errors))
        key('f8',r'\[silk.lifecycle.restore.ok\] frame=\d+ sid=200',timeout=30)
        restored_errors=['not restored'];deadline=time.monotonic()+90
        while time.monotonic()<deadline:
            q.cmd('screendump',{'filename':str(out/'linen-restored.ppm')})
            current=text()
            restored_errors=verify_pixels(current,(out/'linen-restored.ppm').read_bytes(),font)
            if final_view(current)[0]['selected']!=selected:restored_errors.append('selection changed during restore')
            if not restored_errors:break
            time.sleep(.2)
        (out/'minimize-restore.json').write_text(json.dumps({'ok':not restored_errors,'errors':restored_errors,'selected':selected,'scope':'visible glyphs then absent ink then restored complete glyphs'},indent=2)+'\n')
        if restored_errors:raise RuntimeError('restored pixels: '+'; '.join(restored_errors))
        errors=check(text())
        (out/'result.json').write_text(json.dumps({'ok':not errors,'errors':errors,'actions':actions},indent=2)+'\n')
        for error in errors:print('FAIL:',error)
        print('[desktop.close.gate.result] '+('FAIL'if errors else'PASS'))
        return bool(errors)
    except Exception as error:
        (out/'result.json').write_text(json.dumps({'ok':False,'errors':[str(error)],'actions':actions},indent=2)+'\n')
        raise
    finally:
        (out/'actions.json').write_text(json.dumps(actions,indent=2)+'\n')
        proc.terminate()
        try:proc.wait(timeout=5)
        except subprocess.TimeoutExpired:proc.kill();proc.wait()
        sock.unlink(missing_ok=True)
if __name__=='__main__':raise SystemExit(main())
