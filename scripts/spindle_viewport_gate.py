#!/usr/bin/env python3
"""Actual keyboard + framebuffer test of a small tile, zoom and unzoom.
Reads an existing durable history image; never builds or edits a source disk.
"""
import argparse,hashlib,json,pathlib,re,shutil,subprocess,time,os
from desktop_close_gate import QMP
from spindle_durable_reboot_gate import snapshot
from check_spindle_viewport_pixels import verify
from check_desktop_pixels import font_from_source

def main():
    p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=pathlib.Path);p.add_argument('--source-disk',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);p.add_argument('--cursor',action='store_true');p.add_argument('--scroll',action='store_true');p.add_argument('--delivery',action='store_true');p.add_argument('--history',action='store_true');p.add_argument('--controls',action='store_true');p.add_argument('--desktop-status',action='store_true');args=p.parse_args()
    iso=args.iso.resolve(strict=True);source=args.source_disk.resolve(strict=True);out=args.out.resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise RuntimeError('output directory must be fresh')
    expected=snapshot(source.read_bytes());disk=out/'nvme.img';shutil.copyfile(source,disk);log=out/'serial.log';sock=pathlib.Path('/tmp')/f'sex-view-{os.getpid()}.sock'
    command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown']
    manifest={'scenario':'cursor'if args.cursor else'scroll'if args.scroll else'delivery'if args.delivery else'viewport','iso_sha256':hashlib.sha256(iso.read_bytes()).hexdigest(),'source_disk':str(source),'input_disk_sha256':hashlib.sha256(source.read_bytes()).hexdigest(),'journal':expected,'command':command}
    (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n');proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);stages=[]
    def text():return log.read_text(errors='replace')if log.exists()else''
    def wait(pattern,start=0,timeout=180):
        # Canonical atomic view records identify the actual app; legacy shell
        # debug text uses the same editor labels and can interleave fragments.
        if r'\[spindle.line.cursor\] ' in pattern:
            pattern=pattern.replace(r'\[spindle.line.cursor\] ',r'\[spindle.cursor.view\] target=12 sequence=\d+ ')+r'(?:\s|$)'
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            if proc.poll()is not None:raise RuntimeError('guest exited')
            value=text();m=re.search(pattern,value[start:])
            if m:return m
            if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at',value):raise RuntimeError('guest fault')
            time.sleep(.1)
        raise RuntimeError('timeout: '+pattern)
    try:
        deadline=time.monotonic()+15
        while not sock.exists():
            if time.monotonic()>deadline:raise RuntimeError('QMP unavailable')
            time.sleep(.1)
        q=QMP(sock);font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text())
        def key(code,pattern,timeout=30):start=len(text());q.key(code);return wait(pattern,start,timeout)
        def capture(name,prompt,small,output=None):
            deadline=time.monotonic()+90;errors=['not painted'];latest=None
            while time.monotonic()<deadline:
                q.cmd('screendump',{'filename':str(out/(name+'.ppm'))});value=text()
                errors=verify(value,(out/(name+'.ppm')).read_bytes(),font,12,prompt,'spindle17'in prompt,False)
                matches=re.findall(r'\[spindle.view.accepted\] target=12 task=12 sequence=(\d+) w=(\d+) h=(\d+) cols=(\d+) top_rows=(\d+) bottom_rows=(\d+) sender=3',value)
                latest=list(map(int,matches[-1]))if matches else None
                if output and latest:
                    metadata=re.findall(rf'\[spindle.view.text\] target=12 sid=154 sequence={latest[0]} paint=(\d+) cols=\d+ rows=\d+ bytes=\d+',value)
                    pieces=re.findall(rf'\[spindle.view.bytes\] target=12 sid=154 seq={latest[0]} paint={metadata[-1]} off=\d+ hex=([0-9a-f]*)',value)if metadata else[]
                    if output.encode()not in b''.join(bytes.fromhex(part)for part in pieces):errors.append('wrong selected output row')
                if not errors and latest and ((latest[2]<=180)==small):break
                time.sleep(.2)
            if errors or not latest or ((latest[2]<=180)!=small):raise RuntimeError(name+': '+str(errors)+' layout='+str(latest))
            stages.append({'name':name,'layout':latest,'prompt':prompt,'expected_output':output,'pixel_errors':errors,'log_offset':len(value),'ppm_sha256':hashlib.sha256((out/(name+'.ppm')).read_bytes()).hexdigest()})
        def zoom():
            key('grave_accent',r'\[shell.palette.item\] idx=0')
            key('k',r'\[shell.palette.select\] old=0 new=9');key('k',r'\[shell.palette.select\] old=9 new=8')
            key('ret',r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=153 ok=1')
        recovered=wait(r'\[files.history.recovered\] logical=12 source=17 source_task=17 target=12 generation=(\d+) bytes=(\d+) source_hash=(0x[0-9a-f]+) owned_hash=(0x[0-9a-f]+) data_hash=(0x[0-9a-f]+) boot_grant=53').groups()
        if (int(recovered[0]),int(recovered[1]),recovered[2],recovered[4])!=(expected['generation'],expected['size'],expected['fingerprint'],expected['data_hash']):raise RuntimeError('wrong durable source')
        loaded=wait(r'\[spindle.history.receipt\] target=12 task=12 kind=loaded handle=(\d+) bytes=(\d+) count=(\d+) hash=(0x[0-9a-f]+) error=0').groups()
        if not int(loaded[0])or loaded[1:3]!=(str(expected['size']),str(expected['count'])):raise RuntimeError('wrong owned load')
        wait(r'\[shell.app.registered.reply\] target=15 task=\d+ surface=300 committed=1');wait(r'\[linen.disk.publish.done\]')
        key('scroll_lock',r'\[shell.focus.set\] id=153');wait(r'\[spindle.view.accepted\] target=12 ')
        key('up',r'\[spindle.history.nav\] dir=up idx=0 len=14 ok=1');capture('small-recalled','> echo spindle17',True)
        if not args.scroll and not args.history and not args.controls:
            key('x',r'\[spindle.line.cursor\] pos=15 len=15');capture('small-edited','> echo spindle17x',True)
            key('backspace',r'\[spindle.line.cursor\] pos=14 len=14');zoom();capture('expanded','> echo spindle17',False)
            zoom();capture('small-restored','> echo spindle17',True)
        if args.controls:
            for remaining in range(13,-1,-1):key('backspace',rf'\[spindle.line.cursor\] pos={remaining} len={remaining}')
            def control(command,pattern):
                for first in range(0,len(command),6):
                    end=min(first+6,len(command));start=len(text());q.key_sequence(['minus'if ch=='-'else ch for ch in command[first:end]])
                    wait(rf'\[spindle.line.cursor\] pos={end} len={end}',start)
                return key('ret',pattern)
            control('history-status',r'\[spindle.history.status\] target=12 state=idle')
            control('history-retry',r'\[spindle.history.control\] target=12 action=retry accepted=0')
            control('history-cancel',r'\[spindle.history.control\] target=12 action=cancel accepted=1')
            wait(r'\[spindle.history.receipt\] target=12 task=12 kind=aborted .*error=0')
            control('history-status',r'\[spindle.history.status\] target=12 state=retry_ready')
            if args.desktop_status:
                wait(r'\[shell.spindle.status.accepted\] target=12 sequence=\d+ state=8 live=1')
                key('grave_accent',r'\[shell.palette.item\] idx=0')
                from check_launcher_pixels import verify as verify_launcher
                deadline=time.monotonic()+90;errors=['not painted']
                while time.monotonic()<deadline:
                    q.cmd('screendump',{'filename':str(out/'quiescent-launcher.ppm')});value=text()
                    errors=verify_launcher(value,(out/'quiescent-launcher.ppm').read_bytes(),font)
                    rows=re.findall(r'\[launcher.text.row\] index=0 row=0 name=Open Spindle status=([^\r\n\[]*)',value)
                    if not rows or rows[-1]!='HISTORY QUIESCENT / RETRY':errors.append('missing exact quiescent launcher label')
                    if not errors:break
                    time.sleep(.2)
                if errors:raise RuntimeError('launcher status pixels: '+str(errors))
                (out/'launcher-status.json').write_text(json.dumps({'ok':True,'errors':errors,'label':rows[-1],'scope':'authenticated live task12 status and actual production-font launcher glyphs'})+'\n')
                key('grave_accent',r'\[command_palette.close\]')
            start=len(text());control('history-retry',r'\[spindle.history.control\] target=12 action=retry accepted=1')
            fresh=wait(r'\[spindle.history.receipt\] target=12 task=12 kind=loaded handle=(\d+) bytes=90 count=4 .*error=0',start)
            if fresh[1]==loaded[0]:raise RuntimeError('retry reused the old handle')
            key('up',r'\[spindle.history.nav\] dir=up idx=0 len=14 ok=1');capture('retried-recalled','> echo spindle17',True)
        if args.cursor:
            key('end',r'\[spindle.line.cursor\] pos=14 len=14')
            for length in range(15,63):key('z',rf'\[spindle.line.cursor\] pos={length} len={length}')
            capture('long-tail','> <'+'z'*36+'|',True)
            key('home',r'\[spindle.line.cursor\] pos=0 len=62');capture('long-home','> |echo spindle17',True)
            for position in range(1,15):key('right',rf'\[spindle.line.cursor\] pos={position} len=62')
            key('x',r'\[spindle.line.cursor\] pos=15 len=63');capture('middle-insert','> echo spindle17x|',True)
            key('backspace',r'\[spindle.line.cursor\] pos=14 len=62');capture('middle-backspace','> echo spindle17|',True)
            key('end',r'\[spindle.line.cursor\] pos=62 len=62');zoom();capture('long-expanded','> <'+'z'*36+'|',False)
        if args.scroll:
            key('pgup',r'\[spindle.page.nav\] dir=up offset=1 ok=1 visible=1 step=1');capture('page-one','> echo spindle17',True,'TYPE HELP')
            key('pgup',r'\[spindle.page.nav\] dir=up offset=2 ok=1 visible=1 step=1');capture('page-oldest','> echo spindle17',True,'SPINDLE TERMINAL')
            key('pgdn',r'\[spindle.page.nav\] dir=down offset=1 ok=1 visible=1 step=1')
            key('pgdn',r'\[spindle.page.nav\] dir=down offset=0 ok=1 visible=1 step=1');capture('page-live','> echo spindle17',True,'History loaded: vali')
            zoom();key('pgup',r'\[spindle.page.nav\] dir=up offset=0 ok=1 visible=11 step=11');capture('page-expanded','> echo spindle17',False)
        if args.delivery:
            start=len(text());q.key_sequence(['z']*8);wait(r'\[spindle.line.cursor\] pos=22 len=22',start)
            key('backspace',r'\[spindle.line.cursor\] pos=21 len=21')
            capture('burst-edited','> echo spindle17'+'z'*7+'|',True)
            q.key_sequence(['x']*8);key('f11',r'\[shell.app.close.request\] target=12 task=12 surface=153')
            wait(r'\[sext.app.pool.ready\] target=12 task=12 .*ok=1')
            wait(r'\[spindle.display.cancelled\] target=12 panes=\d+ pending=0')
            if not re.search(r'\[spindle.input.progress\] target=12 display_pending=1 ',text()):raise RuntimeError('no observed HID processing during pending display work')
            q.cmd('screendump',{'filename':str(out/'after-close.ppm')})
        if snapshot(disk.read_bytes())!=expected:raise RuntimeError('render/input mutated durable history')
        (out/'result.json').write_text(json.dumps({'ok':True,'stages':stages,'recovered':recovered,'loaded':loaded,'journal_unchanged':True,'input_during_display_pending':bool(re.search(r'\[spindle.input.progress\] target=12 display_pending=1 ',text())),'display_close_cancelled':re.findall(r'\[spindle.display.cancelled\] target=12 panes=(\d+) pending=0',text()),'scope':'Actual native task12, durable data from prior image, small tile recall/edit, expand/unzoom preserving input; exact glyph pixels; no implicit handle/capability transfer'},indent=2)+'\n');print('[spindle.viewport.gate] PASS')
    except Exception as e:
        (out/'result.json').write_text(json.dumps({'ok':False,'errors':[str(e)],'stages':stages},indent=2)+'\n');raise
    finally:
        proc.terminate()
        try:proc.wait(timeout=5)
        except subprocess.TimeoutExpired:proc.kill();proc.wait()
        sock.unlink(missing_ok=True)
if __name__=='__main__':main()
