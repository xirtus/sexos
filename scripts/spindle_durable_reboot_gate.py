#!/usr/bin/env python3
"""Boot an already-tested normal image against a copy of boot A's real disk.
Never builds; requires BOOT data recovery, fresh handle, exact command body,
real Up recall glyphs and a second set of actual NVMe reads.
"""
import argparse,gzip,hashlib,json,pathlib,re,shutil,subprocess,time,os
from desktop_close_gate import QMP
from check_spindle_relaunch_pixels import verify
from check_desktop_pixels import font_from_source
from history_roster_evidence import verify_boot_roster,verify_writer,source_spindles

def fnv(data):
    h=0xcbf29ce484222325
    for b in data:h=((h^b)*0x100000001b3)&((1<<64)-1)
    return h

def _snapshot(disk,roster):
    valid=[]
    for bank in range(2):
        offset=(1808+9*bank)*512;h=bytearray(disk[offset:offset+512]);stored=int.from_bytes(h[48:56],'little');h[48:56]=bytes(8)
        if len(h)!=512 or h[:8]!=b'SEXHST01' or h[12:16]!=(1).to_bytes(4,'little') or any(h[36:40]) or any(h[56:]) or fnv(h)!=stored:continue
        logical=int.from_bytes(h[8:12],'little');generation=int.from_bytes(h[16:24],'little');source=int.from_bytes(h[24:28],'little');size=int.from_bytes(h[32:36],'little');fingerprint=int.from_bytes(h[40:48],'little')
        task=int.from_bytes(h[28:32],'little');payload=bytearray(disk[offset+512:offset+512+size])
        if logical!=12 or source==0 or task==0 or not generation or not 24<=size<=4096 or len(payload)!=size or fnv(payload)!=fingerprint:continue
        if roster is not None:
            try:verify_writer(dict(source=source,task=task,logical=logical),roster)
            except ValueError:continue
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
                valid.append({'bank':bank,'generation':generation,'source':source,'task':task,'logical':logical,'size':size,'fingerprint':hex(fingerprint),'count':count,'data_hash':hex(fnv(payload[24:]))})
    if not valid:raise RuntimeError('no committed boot A journal snapshot')
    if len(valid)==2 and valid[0]['generation']==valid[1]['generation'] and any(valid[0][k]!=valid[1][k] for k in ['source','task','logical','size','fingerprint']):raise RuntimeError('ambiguous committed journal generation')
    return max(valid,key=lambda r:r['generation'])

def decode_snapshot(disk):
    """Parse journal data only. This does not establish a recovery relationship."""
    return _snapshot(disk,None)

def snapshot(disk,roster):
    """Only a roster joined to the actual held BOOT query may admit a writer."""
    return _snapshot(disk,roster)


def zoom_receipt(trace,start=0):
    """Join complete post-action records; damaged window-action text is no proof.

    The independently emitted active rectangle and committed frame zoom must
    name the same frame and Spindle surface, in that order. Match against the
    original trace so an action offset cannot invent a new record boundary.
    """
    if type(start)is not int or not 0<=start<=len(trace):raise ValueError('invalid Zoom action start')
    records=re.compile(r'^\[silk.lifecycle.zoom.active\] frame=(\d+) sid=(\d+) zx=(-?\d+) zy=(-?\d+) zw=(\d+) zh=(\d+)\r?\n|^\[shell.frame.zoom\] frame=(\d+) surface=(\d+)\r?\n',re.M)
    active={}
    for record in records.finditer(trace,start):
        if record[1]is not None:
            frame,sid,x,y,w,h=map(int,record.groups()[:6])
            active[frame]=(sid,x,y,w,h,record.start(),record.end())
        else:
            frame,sid=map(int,record.groups()[6:]);prior=active.get(frame)
            if frame==0 or sid!=153 or prior is None or prior[:5]!=(153,0,50,1280,750):continue
            return dict(kind='complete_active_geometry_and_frame_zoom',frame=frame,surface=153,
                geometry=dict(x=0,y=50,w=1280,h=750),action_start=start,
                active_log_offset=prior[5],frame_zoom_log_offset=record.start())
    return None

def main():
    p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=pathlib.Path);p.add_argument('--boot-a',required=True,type=pathlib.Path);p.add_argument('--out',required=True,type=pathlib.Path);p.add_argument('--recall',default='echo spindle17');p.add_argument('--binding',type=pathlib.Path);args=p.parse_args()
    iso=args.iso.resolve(strict=True);a=args.boot_a.resolve(strict=True);out=args.out.resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise RuntimeError('output directory must be empty')
    boot_a_result=json.loads((a/'result.json').read_text())
    if not boot_a_result['ok']:raise RuntimeError('boot A did not pass')
    manifest=json.loads((a/'manifest.json').read_text());iso_hash=hashlib.sha256(iso.read_bytes()).hexdigest()
    if manifest.get('sha256',manifest.get('iso_sha256'))!=iso_hash:raise RuntimeError('different boot A and B image')
    source_log=(a/'serial.log').read_text(errors='replace')
    source_roster=verify_boot_roster(pathlib.Path(__file__).resolve().parents[1],source_log)
    binding=None
    if args.binding is not None:
        from alpha_evidence import replay
        from network_contention_gate import frozen_binding
        root=pathlib.Path(__file__).resolve().parents[1]
        binding=frozen_binding(root,iso,args.binding.resolve(strict=True))
        replay(root,iso,args.binding.resolve(strict=True),a)
        seed=gzip.decompress((a/'nvme.img.gz').read_bytes())
    else:
        seed=(a/'nvme.img').read_bytes()
    expected=snapshot(seed,source_roster)
    last=source_roster['participants'][-1]
    original=source_roster['participants'][0]
    if (a/'spindle-history.json').exists():saved=json.loads((a/'spindle-history.json').read_text())['saved']
    else:
        soak=boot_a_result.get('soak')or{}
        from check_alpha_session import boot_lifecycle_errors
        if boot_lifecycle_errors(boot_a_result):raise RuntimeError('missing complete alpha lifecycle evidence')
        source_log=(a/'serial.log').read_text(errors='replace')
        receipts=re.findall(rf"\[spindle.history.receipt\] target={last['target']} task={last['task']} kind=saved handle=(\d+) bytes=(\d+) count=(\d+) hash=(0x[0-9a-f]+) error=0",source_log)
        if not receipts:raise RuntimeError('no acknowledged successor history snapshot')
        saved=receipts[-1]
    if not 0<len(args.recall)<=256 or any(not 32<=ord(c)<=126 for c in args.recall):raise RuntimeError('invalid ASCII recall expectation')
    if (expected['source'],str(expected['size']),str(expected['count']),expected['fingerprint'])!=(last['target'],saved[1],saved[2],saved[3]):raise RuntimeError('boot A committed journal differs from acknowledged save')
    disk=out/'nvme.img';disk.write_bytes(seed);log=out/'serial.log';sock=pathlib.Path('/tmp')/f'sex-durable-{os.getpid()}.sock'
    command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku','-m','512M','-cdrom',str(iso),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{log}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown']
    (out/'manifest.json').write_text(json.dumps({'iso_sha256':iso_hash,'boot_a':str(a),'input_disk_sha256':hashlib.sha256(disk.read_bytes()).hexdigest(),'expected':expected,'command':command,'binding':binding,'roster':source_roster},indent=2)+'\n')
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
    def wait_zoom(start,timeout=180):
        end=time.monotonic()+timeout
        while time.monotonic()<end:
            if proc.poll()is not None:raise RuntimeError('guest exited')
            value=text();receipt=zoom_receipt(value,start)
            if receipt:return receipt
            if re.search(r'KERNEL PANIC|PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at',value):raise RuntimeError('guest fault')
            time.sleep(.1)
        raise RuntimeError('timeout: complete matching Spindle active geometry and frame Zoom receipts')
    try:
        deadline=time.monotonic()+15
        while not sock.exists():
            if time.monotonic()>deadline:raise RuntimeError('QMP unavailable')
            time.sleep(.1)
        q=QMP(sock)
        wait(r'\[bootgraph.boot.handoff\]')
        cold_roster=verify_boot_roster(pathlib.Path(__file__).resolve().parents[1],text())
        verify_writer(expected,cold_roster)
        recovered=wait(rf"\[files.history.recovered\] logical=12 source={last['target']} source_task={last['task']} target={original['target']} generation=(\d+) bytes=(\d+) source_hash=(0x[0-9a-f]+) owned_hash=(0x[0-9a-f]+) data_hash=(0x[0-9a-f]+) boot_grant=53").groups()
        if (int(recovered[0]),int(recovered[1]),recovered[2],recovered[4])!=(expected['generation'],expected['size'],expected['fingerprint'],expected['data_hash']):raise RuntimeError('BOOT recovery did not match exact durable source bytes')
        loaded=wait(rf"\[spindle.history.receipt\] target={original['target']} task={original['task']} kind=loaded handle=(\d+) bytes=(\d+) count=(\d+) hash=(0x[0-9a-f]+) error=0").groups()
        if int(loaded[0])==0 or (int(loaded[1]),int(loaded[2]),loaded[3])!=(expected['size'],expected['count'],recovered[3]):raise RuntimeError('fresh handle did not load owner-rebound snapshot')
        wait(r'\[shell.app.registered.reply\] target=15 task=\d+ surface=300 committed=1',timeout=180)
        wait(r'\[linen.disk.publish.done\]',timeout=180)
        start=len(text());q.key('scroll_lock');wait(r'\[shell.focus.set\] id=153',start)
        # Use the real launcher window action to expose all six command rows.
        # Startup tiles can be too short; clipped glyphs never prove recall.
        start=len(text());q.key('grave_accent');wait(r'\[shell.palette.item\] idx=0',start)
        for index in range(8):
            start=len(text());q.key('j');wait(rf'\[shell.palette.select\] old={index} new={index+1}',start)
        start=len(text());q.key('ret');zoom=wait_zoom(start)
        start=len(text());q.key('up');wait(r'\[spindle.history.nav\] dir=up idx=0 len='+str(len(args.recall))+r' ok=1',start)
        font=font_from_source((pathlib.Path(__file__).resolve().parents[1]/'servers/sexdisplay/src/main.rs').read_text());errors=['not visible'];end=time.monotonic()+120
        while time.monotonic()<end:
            q.cmd('screendump',{'filename':str(out/'recovered-history.ppm')})
            errors=verify(text(),(out/'recovered-history.ppm').read_bytes(),font,require_echo=False,target=12,require_all_ink=False,require_prompt='> '+args.recall)
            if not errors:break
            time.sleep(.2)
        if errors:raise RuntimeError('reboot recall pixels: '+'; '.join(errors))
        # Stop the sole disk writer before inspecting its terminal bytes.
        proc.terminate()
        try:proc.wait(timeout=5)
        except subprocess.TimeoutExpired:proc.kill();proc.wait(timeout=5)
        final=disk.read_bytes()
        if snapshot(final,cold_roster)!=expected:raise RuntimeError('recovery mutated durable journal')
        (out/'result.json').write_text(json.dumps({'ok':True,'expected':expected,'recovered':recovered,'loaded':loaded,'zoom_receipt':zoom,'pixel_errors':errors,'recall':args.recall,'stopped':True,'final_disk_sha256':hashlib.sha256(final).hexdigest(),'binding':binding,'scope':'Cold recovery from exact parent journal through held BOOT roster; fresh original handle, actual writer task, completed frame Zoom and visible Up recall. Bound parent replay only when binding is supplied; auxiliary observed native probe, no standalone read-only artifact replay.'},indent=2)+'\n')
        print('[spindle.history.reboot] PASS')
    except Exception as e:
        (out/'result.json').write_text(json.dumps({'ok':False,'errors':[str(e)]},indent=2)+'\n');raise
    finally:
        if proc.poll()is None:
            proc.terminate()
            try:proc.wait(timeout=5)
            except subprocess.TimeoutExpired:proc.kill();proc.wait(timeout=5)
        sock.unlink(missing_ok=True)
if __name__=='__main__':main()
