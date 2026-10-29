#!/usr/bin/env python3
"""Run an existing normal ISO; never builds. Real QMP keyboard → shell Close."""
import argparse,hashlib,json,pathlib,re,socket,subprocess,time,os
from check_desktop_close_log import check,responsiveness
from check_desktop_pixels import verify as verify_pixels, verify_hidden, verify_kaleido_layout, final_view, font_from_source
from check_launcher_pixels import verify as verify_launcher
from check_relaunch_pixels import verify as successor_pixels
from check_app_activation_log import validate as activation_errors
from check_spindle_relaunch_pixels import verify as spindle_pixels
from check_spindle_history_log import validate as history_errors
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
        # Persist in the original incarnation before its irreversible close.
        focus(201)
        key('esc',r'\[quil.palette.action\] kind=esc clear=1')
        key('h',r'\[quil.text.recv\]')
        key('i',r'\[quil.text.recv\]')
        key('esc',r'kind=esc toggle_on=1')
        key('down',r'\[quil.palette.selected\] row=1')
        key('ret',r'\[quil.persist.save.ok\]',timeout=180)
        saved=re.findall(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',text())[-1]
        close(15,300)
        # Existing Quil shell frame survives independently until its own close.
        close(9,201)
        # BOOT's one fresh successor is now ready. The same launcher row
        # activates it, then its real registration/presentation completes.
        key('grave_accent',r'\[shell.palette.item\] idx=0')
        key('j',r'\[shell.palette.select\] old=0 new=1')
        key('ret',r'\[shell.app.relaunch\] predecessor=9 successor=16 task=16 surface=301 receipt_replayed=1')
        wait(r'\[app.surface.presented\] target=16 surface=301 sequence=1 ok=1',timeout=180)
        wait(r'\[shell.focus.set\] id=301',timeout=90)
        key('down',r'\[quil.palette.selected\] row=1')
        key('down',r'\[quil.palette.selected\] row=2')
        key('ret',r'\[quil.persist.load.ok\]',timeout=180)
        loaded=re.findall(r'\[quil.persist.load.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',text())[-1]
        if loaded!=saved:raise RuntimeError('successor did not load exact predecessor saved document')
        key('esc',r'\[quil.palette.action\] kind=esc clear=1')
        key('z',r'\[quil.text.recv\]')
        status_match=wait(r'\[quil.text.view\] target=16 surface=301 status=([^\r\n]*) '+str(int(saved[0])+1)+r'B\*',timeout=30)
        expected_status=status_match[1]+' '+str(int(saved[0])+1)+'B*'
        pixel_errors=['not rendered'];deadline=time.monotonic()+60
        while time.monotonic()<deadline:
            q.cmd('screendump',{'filename':str(out/'successor-input.ppm')})
            pixel_errors=successor_pixels(text(),(out/'successor-input.ppm').read_bytes(),font,expected_status)
            if not pixel_errors:break
            time.sleep(.2)
        if pixel_errors:raise RuntimeError('successor glyph pixels: '+'; '.join(pixel_errors))
        close(16,301)
        (out/'relaunch.json').write_text(json.dumps({'ok':True,'predecessor':9,'successor':16,'surface':301,'saved':saved,'loaded':loaded,'typed_status':expected_status,'pixel_errors':pixel_errors,'scope':'real BOOT activation, native presentation, document identity, input glyphs, successor private cleanup'},indent=2)+'\n')
        def spindle_command(command,target=17):
            for length,char in enumerate(command,1):
                qcode={' ':'spc','-':'minus'}.get(char,char)
                # Spaces extend the editor but can leave a padded grid's
                # pixels unchanged. Await the actual unbudgeted editor receipt;
                # the final four-grid framebuffer remains mandatory below.
                key(qcode,rf'\[spindle.input.recv\] key=printable ch={re.escape(char)} len={length}(?:\r|\n)|\[spindle.line.cursor\] pos={length} len={length}(?:\r|\n)')
            key('ret',rf'\[spindle.instance.command\] target={target} task={target} name='+re.escape(command.split()[0])+r' ok=1',timeout=90)
        key('scroll_lock',r'\[shell.focus.set\] id=153')
        spindle_command('echo predecessor12',target=12)
        old_history=wait(r'\[spindle.history.receipt\] target=12 task=12 kind=saved handle=(\d+) bytes=(\d+) count=1 hash=(0x[0-9a-f]+) error=0',timeout=90).groups()
        close(12,153)
        # Spindle has a separate immutable grant and four fresh grid subjects.
        key('grave_accent',r'\[shell.palette.item\] idx=0')
        key('ret',r'\[shell.app.relaunch\] predecessor=12 successor=17 task=17 surface=302 receipt_replayed=1')
        wait(r'\[app.surface.presented\] target=17 surface=302 sequence=1 ok=1',timeout=180)
        wait(r'\[shell.focus.set\] id=306',timeout=60)
        wait(r'\[spindle.history.receipt\] target=17 task=17 kind=missing handle=0 bytes=0 count=0 ',timeout=90)
        spindle_command('linen-list')
        wait(r'\[linen.spindle.authority\] target=17 slot=50 op=66 status=5 allowed=1',timeout=60)
        bell_start=len(text())
        spindle_command('notify spindle17')
        bell_recv=wait(r'\[bell.notify.recv\] caller_pd=17 category=0 requested=1',bell_start,timeout=60)
        wait(r'\[bell.notify.ok\] caller_pd=17 final_lane=\d+ event_id=\d+',bell_start+bell_recv.end(),timeout=60)
        spindle_command('echo spindle17')
        wait(r'\[spindle.history.receipt\] target=17 task=17 kind=saved handle=\d+ bytes=\d+ count=3 hash=0x[0-9a-f]+ error=0',timeout=90)
        save_start=len(text())
        spindle_command('save')
        saved_history=wait(r'\[spindle.history.receipt\] target=17 task=17 kind=saved handle=(\d+) bytes=(\d+) count=(\d+) hash=(0x[0-9a-f]+) error=0',save_start,timeout=90).groups()
        load_start=len(text())
        spindle_command('load')
        loaded_history=wait(r'\[spindle.history.receipt\] target=17 task=17 kind=loaded handle=(\d+) bytes=(\d+) count=(\d+) hash=(0x[0-9a-f]+) error=0',load_start,timeout=90).groups()
        if saved_history[1:]!=loaded_history[1:] or saved_history[0]==loaded_history[0] or int(saved_history[0])==0 or int(loaded_history[0])==0:
            raise RuntimeError('history roundtrip did not use fresh own handles and exact saved bytes/count/hash')
        key('up',r'\[spindle.history.nav\] dir=up idx=0 len=14 ok=1')
        (out/'spindle-history.json').write_text(json.dumps({'ok':True,'predecessor_saved':old_history,'successor_started_private_empty':True,'saved':saved_history,'loaded':loaded_history,'recall':'echo spindle17','scope':'same-boot owner-private RamFS; fresh handle receipts, exact snapshot fingerprint, real Up recall; no inherited predecessor history'},indent=2)+'\n')
        spindle_errors=['not rendered'];deadline=time.monotonic()+90
        while time.monotonic()<deadline:
            q.cmd('screendump',{'filename':str(out/'spindle-successor.ppm')})
            spindle_errors=spindle_pixels(text(),(out/'spindle-successor.ppm').read_bytes(),font)
            if not spindle_errors:break
            time.sleep(.2)
        if spindle_errors:raise RuntimeError('Spindle grid pixels: '+'; '.join(spindle_errors))
        close(17,306)
        history_failures=history_errors(text())
        (out/'history-log.json').write_text(json.dumps({'ok':not history_failures,'errors':history_failures},indent=2)+'\n')
        if history_failures:raise RuntimeError('history receipt joins: '+'; '.join(history_failures))
        (out/'spindle-relaunch.json').write_text(json.dumps({'ok':True,'predecessor':12,'successor':17,'task':17,'frame':306,'content':[302,303,304,305],'pixel_errors':spindle_errors,'scope':'real activation, native presentation, Linen observer, Bell notification, command echo glyphs and five-role private cleanup'},indent=2)+'\n')
        key('grave_accent',r'\[shell.palette.item\] idx=0')
        key('ret',r'\[launcher.execute.denied\] idx=0 .*reason=boot_reserve_exhausted')
        key('grave_accent',r'\[command_palette.close\]')
        # Both Quil reservations are terminal. Enter must stay in
        # the launcher with an explicit denial and actual readable closed text.
        key('grave_accent',r'\[shell.palette.item\] idx=0')
        key('j',r'\[shell.palette.select\] old=0 new=1')
        key('ret',r'\[launcher.execute.denied\] idx=1 .*reason=boot_reserve_exhausted')
        activation_failures=activation_errors(text())+activation_errors(text(),spindle=True)
        (out/'activation-log.json').write_text(json.dumps({'ok':not activation_failures,'errors':activation_failures,'scope':'ordered real activation/retirement records; pixels checked separately'},indent=2)+'\n')
        if activation_failures:raise RuntimeError('activation log: '+'; '.join(activation_failures))
        launcher_errors=['not rendered'];deadline=time.monotonic()+90
        while time.monotonic()<deadline:
            q.cmd('screendump',{'filename':str(out/'launcher-closed.ppm')})
            launcher_errors=verify_launcher(text(),(out/'launcher-closed.ppm').read_bytes(),font)
            if not launcher_errors:break
            time.sleep(.2)
        (out/'launcher-pixels.json').write_text(json.dumps({'ok':not launcher_errors,'errors':launcher_errors,'scope':'actual launcher labels and closed-state glyphs after rejected Enter'},indent=2)+'\n')
        if launcher_errors:raise RuntimeError('launcher pixels: '+'; '.join(launcher_errors))
        key('grave_accent',r'\[command_palette.close\]')
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
        errors=check(text(),include_successor=True,include_spindle_successor=True)
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
