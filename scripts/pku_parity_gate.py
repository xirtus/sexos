#!/usr/bin/env python3
"""Paired PKU-on/PKU-off boot: authority decisions must not depend on PKU.

PKU accelerates enforcement; it never grants authority. The same sealed image
boots twice from identical disks, once with hardware protection keys and once
without, and the multiset of authority-decision records over the same window
must be identical. This is boot/service-genesis evidence only: without PKU,
hardware does not stop a hostile user load or store in GLOBAL_VAS.
"""
import argparse,collections,hashlib,json,re,shutil,subprocess,time
from pathlib import Path

MODES={'pku':('max,+pku','PKU: Protection Keys enabled in CR4.'),
       'nopku':('max,-pku','PKU: unsupported on this CPU/QEMU path; PKRU ops gated off')}
# Records that state an authority decision: genesis, grants, routes, seals,
# allow/deny outcomes and revocation. Data-bearing transport records are not.
AUTHORITY=re.compile(r'^\[([a-z0-9_.]*(?:grant|deny|denied|allow|authority|negative|policy|revoked|route|seal|genesis|successor)[a-z0-9_.]*)\]')
VOLATILE=re.compile(r'0x[0-9a-f]{6,}|\b(?:tick|ticks|iter|yield_count|a[0-9])=[0-9a-fx]+')
REQUIRED={'bootgraph.cap.grant':8,'boot.app.route':4,'boot.app.successor':3,'boot.paging.genesis':2}
FAULT=re.compile(r'KERNEL PAGE FAULT|DOUBLE FAULT|GENERAL PROTECTION|panicked|fault\.kill')

def records(text):
    """Split interleaved serial output at record tags, then keep authority."""
    found=collections.Counter()
    for record in re.findall(r'\[[a-z0-9_.]+\][^\[\n]*',text):
        match=AUTHORITY.match(record)
        if match:found[VOLATILE.sub('#',record.strip())]+=1
    return found

def reconcile(left,right):
    """Pair a record that another producer's output cut short with its whole
    counterpart: the shorter must be a strict prefix of the longer and both
    must carry the same tag and at least one field. Both are then consumed."""
    paired=0
    for short_side,long_side in ((left,right),(right,left)):
        for short in sorted(short_side):
            if '=' not in short:continue
            for whole in sorted(long_side):
                if short_side[short] and long_side[whole] and whole.startswith(short) and len(whole)>len(short):
                    short_side[short]-=1;long_side[whole]-=1;paired+=1
        for counter in (left,right):
            for key in [k for k,v in counter.items() if v<=0]:del counter[key]
    return paired

def tags(counter):
    total=collections.Counter()
    for record,count in counter.items():total[record[1:record.index(']')]]+=count
    return total

def compare(with_pku,without_pku):
    errors=[];a=records(with_pku);b=records(without_pku)
    for name,(_,banner) in MODES.items():
        text=with_pku if name=='pku' else without_pku
        if banner not in text:errors.append('missing PKU mode banner: '+name)
        if FAULT.search(text):errors.append('fault in '+name+' boot')
    for tag,minimum in REQUIRED.items():
        for name,counter in (('pku',a),('nopku',b)):
            if tags(counter)[tag]<minimum:errors.append(f'{name}: {tag} below {minimum}')
    left=a-b;right=b-a;paired=reconcile(left,right)
    differences=sorted(set(left)|set(right))
    if differences:errors.append(f'{len(differences)} authority records differ')
    return dict(ok=not errors,errors=errors,authority_records=sum(a.values()),distinct=len(a),interleaved_prefix_pairs=paired,
                differences=[dict(record=k[:200],pku=a[k],nopku=b[k]) for k in differences[:40]])

def boot(iso,disk,out,cpu,seconds):
    out.mkdir(parents=True);image=out/'nvme.img';shutil.copyfile(disk,image);serial=out/'serial.log'
    command=['qemu-system-x86_64','-M','q35','-m','512M','-cpu',cpu,'-cdrom',str(iso),'-device','nec-usb-xhci,id=xhci',
             '-device','usb-tablet,bus=xhci.0','-drive',f'if=none,id=nvm,file={image},format=raw','-device','nvme,serial=sexos01,drive=nvm',
             '-serial',f'file:{serial}','-display','none','-no-reboot','-no-shutdown']
    proc=subprocess.Popen(command,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    try:time.sleep(seconds)
    finally:
        proc.terminate()
        try:proc.wait(10)
        except subprocess.TimeoutExpired:proc.kill();proc.wait()
    return command,serial.read_bytes().decode('utf-8','replace')

def main():
    p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--disk',required=True,type=Path)
    p.add_argument('--out',required=True,type=Path);p.add_argument('--seconds',type=int,default=30);a=p.parse_args()
    out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
    if any(out.iterdir()):raise ValueError('fresh parity directory required')
    iso=a.iso.resolve(strict=True);disk=a.disk.resolve(strict=True);texts={};commands={}
    for name,(cpu,_) in MODES.items():commands[name],texts[name]=boot(iso,disk,out/name,cpu,a.seconds)
    result=compare(texts['pku'],texts['nopku'])
    result.update(iso_sha256=hashlib.sha256(iso.read_bytes()).hexdigest(),disk_sha256=hashlib.sha256(disk.read_bytes()).hexdigest(),
                  seconds=a.seconds,commands=commands,serial_sha256={k:hashlib.sha256(v.encode()).hexdigest() for k,v in texts.items()},
                  scope='Paired boot-window authority parity with and without hardware PKU on one image and disk. Not hostile-code isolation: without PKU nothing stops direct user loads/stores in GLOBAL_VAS.')
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:result[k] for k in('ok','errors','authority_records','distinct')}))
    if not result['ok']:raise SystemExit(1)

if __name__=='__main__':main()
