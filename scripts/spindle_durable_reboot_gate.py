#!/usr/bin/env python3
"""Boot an already-tested normal image against a copy of boot A's real disk.
Never builds; requires BOOT data recovery, fresh handle, exact command body,
real Up recall glyphs and a second set of actual NVMe reads.
"""
import argparse,hashlib,json,pathlib,re,shutil,subprocess,time,os
from desktop_close_gate import QMP
from check_spindle_relaunch_pixels import verify
from check_desktop_pixels import font_from_source

def fnv(data):
    h=0xcbf29ce484222325
    for b in data:h=((h^b)*0x100000001b3)&((1<<64)-1)
    return h

def snapshot(disk):
    valid=[]
    for bank in range(2):
        offset=(1808+9*bank)*512;h=bytearray(disk[offset:offset+512]);stored=int.from_bytes(h[48:56],'little');h[48:56]=bytes(8)
        if len(h)!=512 or h[:8]!=b'SEXHST01' or h[12:16]!=(1).to_bytes(4,'little') or any(h[36:40]) or any(h[56:]) or fnv(h)!=stored:continue
        logical=int.from_bytes(h[8:12],'little');generation=int.from_bytes(h[16:24],'little');source=int.from_bytes(h[24:28],'little');size=int.from_bytes(h[32:36],'little');fingerprint=int.from_bytes(h[40:48],'little')
        task=int.from_bytes(h[28:32],'little');payload=bytearray(disk[offset+512:offset+512+size])
        if logical!=12 or source not in (12,17) or task!=source or not generation or not 24<=size<=4096 or len(payload)!=size or fnv(payload)!=fingerprint:continue
        if any(disk[offset+512+size:offset+512+((size+511)//512)*512]):continue
        count=int.from_bytes(payload[8:10],'little');total=int.from_bytes(payload[10:12],'little');checksum=int.from_bytes(payload[16:24],'little');payload[16:24]=bytes(8)
        if payload[:4]!=b'SHI1' or int.from_bytes(payload[4:8],'little')!=source or count>128 or total!=size or any(payload[12:16]) or fnv(payload)!=checksum:continue
        pos=24
        for _ in range(count):
            n=int.from_bytes(payload[pos:pos+2],'little');pos+=2
            if not 0<n<=256 or pos+n>size or any(not 32<=b<=126 for b in payload[pos:pos+n]):break
            pos+=n
        else:
            if pos==size:
                valid.append({'bank':bank,'generation':generation,'source':source,'size':size,'fingerprint':hex(fingerprint),'count':count,'data_hash':hex(fnv(payload[24:]))})
    if not valid:raise RuntimeError('no committed boot A journal snapshot')
    if len(valid)==2 and valid[0]['generation']==valid[1]['generation'] and any(valid[0][k]!=valid[1][k] for k in ['source','size','fingerprint']):raise RuntimeError('ambiguous committed journal generation')
    return max(valid,key=lambda r:r['generation'])

def main():
    p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=pathlib.Path);p.add_argument('--boot-a',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);p.add_argument('--recall',default='echo spindle17');args=p.parse_args()
    iso=args.iso.resolve(strict=True);a=args.boot_a.resolve(strict=True);out=args.out.resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise RuntimeError('output directory must be empty')
    boot_a_result=json.loads((a/'result.json').read_text())
    if not boot_a_result['ok']:raise RuntimeError('boot A did not pass')
    manifest=json.loads((a/'manifest.json').read_text());iso_hash=hashlib.sha256(iso.read_bytes()).hexdigest()
    if manifest.get('sha256',manifest.get('iso_sha256'))!=iso_hash:raise RuntimeError('different boot A and B image')
    expected=snapshot((a/'nvme.img').read_bytes())
    if (a/'spindle-history.json').exists():saved=json.loads((a/'spindle-history.json').read_text())['saved']
    else:
        soak=boot_a_result.get('soak')or{}
        from check_alpha_session import boot_lifecycle_errors
        if boot_lifecycle_errors(boot_a_result):raise RuntimeError('missing complete alpha lifecycle evidence')
        source_log=(a/'serial.log').read_text(errors='replace')
        receipts=re.findall(r'\[spindle.history.receipt\] target=17 task=17 kind=saved handle=(\d+) bytes=(\d+) count=(\d+) hash=(0x[0-9a-f]+) error=0',source_log)
        if not receipts:raise RuntimeError('no acknowledged successor history snapshot')
        saved=receipts[-1]
    if not 0<len(args.recall)<=256 or any(not 32<=ord(c)<=126 for c in args.recall):raise RuntimeError('invalid ASCII recall expectation')
    if (expected['source'],str(expected['size']),str(expected['count']),expected['fingerprint'])!=(17,saved[1],saved[2],saved[3]):raise RuntimeError('boot A committed journal differs from acknowledged save')
    disk=out/'nvme.img';shutil.copyfile(a/'nvme.img',disk);log=out/'serial.log';sock=pathlib.Path('/tmp')/f'sex-durable-{os.getpid()}.sock'
    command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown']
    (out/'manifest.json').write_text(json.dumps({'iso_sha256':iso_hash,'boot_a':str(a),'input_disk_sha256':hashlib.sha256(disk.read_bytes()).hexdigest(),'expected':expected,'command':command},indent=2)+'\n')
    proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT)
    def text():return log.read_text(errors='replace')if log.exists()else''
    def wait(pattern,start=0,timeout=180):
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            if proc.poll()is not None:raise RuntimeError('guest exited')
            value=text();m=re.search(pattern,value[start:])
            if m:return m
            if re.search(r'KERNEL PANIC|PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at',value):raise RuntimeError('guest fault')
            time.sleep(.1)
        raise RuntimeError('timeout: '+pattern)
    try:
        deadline=time.monotonic()+15
        while not sock.exists():
            if time.monotonic()>deadline:raise RuntimeError('QMP unavailable')
            time.sleep(.1)
        q=QMP(sock)
        wait(r'\[boot.history.recovery\] holder=11 slot=53 logical=12 original=12 task=12 successor=17 successor_task=17 base=1808 sectors=18 runtime_mint=0')
        recovered=wait(r'\[files.history.recovered\] logical=12 source=17 source_task=17 target=12 generation=(\d+) bytes=(\d+) source_hash=(0x[0-9a-f]+) owned_hash=(0x[0-9a-f]+) data_hash=(0x[0-9a-f]+) boot_grant=53').groups()
        if (int(recovered[0]),int(recovered[1]),recovered[2],recovered[4])!=(expected['generation'],expected['size'],expected['fingerprint'],expected['data_hash']):raise RuntimeError('BOOT recovery did not match exact durable source bytes')
        loaded=wait(r'\[spindle.history.receipt\] target=12 task=12 kind=loaded handle=(\d+) bytes=(\d+) count=(\d+) hash=(0x[0-9a-f]+) error=0').groups()
        if int(loaded[0])==0 or (int(loaded[1]),int(loaded[2]),loaded[3])!=(expected['size'],expected['count'],recovered[3]):raise RuntimeError('fresh handle did not load owner-rebound snapshot')
        wait(r'\[shell.app.registered.reply\] target=15 task=\d+ surface=300 committed=1',timeout=180)
        wait(r'\[linen.disk.publish.done\]',timeout=180)
        start=len(text());q.key('scroll_lock');wait(r'\[shell.focus.set\] id=153',start)
        # Use the real launcher window action to expose all six command rows.
        # Startup tiles can be too short; clipped glyphs never prove recall.
        start=len(text());q.key('grave_accent');wait(r'\[shell.palette.item\] idx=0',start)
        for index in range(8):
            start=len(text());q.key('j');wait(rf'\[shell.palette.select\] old={index} new={index+1}',start)
        start=len(text());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=153 ok=1',start)
        start=len(text());q.key('up');wait(r'\[spindle.history.nav\] dir=up idx=0 len='+str(len(args.recall))+r' ok=1',start)
        font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text());errors=['not visible'];end=time.monotonic()+120
        while time.monotonic()<end:
            q.cmd('screendump',{'filename':str(out/'recovered-history.ppm')})
            errors=verify(text(),(out/'recovered-history.ppm').read_bytes(),font,require_echo=False,target=12,require_all_ink=False,require_prompt='> '+args.recall)
            if not errors:break
            time.sleep(.2)
        if errors:raise RuntimeError('reboot recall pixels: '+'; '.join(errors))
        # Read-only recovery must not rewrite or reformat the journal.
        if snapshot(disk.read_bytes())!=expected:raise RuntimeError('recovery mutated durable journal')
        (out/'result.json').write_text(json.dumps({'ok':True,'expected':expected,'recovered':recovered,'loaded':loaded,'pixel_errors':errors,'recall':args.recall,'scope':'two actual QEMU boots; durable writer17 to BOOT-authorized new task12; exact data fingerprint, fresh own handle and visible Up recall after native launcher Zoom'},indent=2)+'\n')
        print('[spindle.history.reboot] PASS')
    except Exception as e:
        (out/'result.json').write_text(json.dumps({'ok':False,'errors':[str(e)]},indent=2)+'\n');raise
    finally:
        proc.terminate()
        try:proc.wait(timeout=5)
        except subprocess.TimeoutExpired:proc.kill();proc.wait()
        sock.unlink(missing_ok=True)
if __name__=='__main__':main()
