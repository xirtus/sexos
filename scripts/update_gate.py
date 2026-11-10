#!/usr/bin/env python3
"""Signed A/B updates, tried once, in QEMU under UEFI.

The ISO must be built trusting the gates' test key (tools/sexos-release `public test`):
  SEXOS_RELEASE_KEY=$(cargo run -q --release --manifest-path tools/sexos-release/Cargo.toml -- public test) \
    ./scripts/entrypoint_build.sh

1. A blank disk; the live system installs itself (slot A).
2. From the disk: `update` against a release served the way GitHub serves one (a redirect
   to the file). A tampered index is refused before any download of the image; the good
   release lands in slot B.
3. Restart: slot B starts, its trial starts (the menu's default falls back to A), the
   desktop answers and the trial passes (B is the default, version 2).
   The same release again is "up to date".
4. Version 3 into slot A. Restart and stop the machine as soon as Files has started: the
   trial never passed. The next start is slot B again, and the menu says the trial failed.
"""
import argparse,os,re,shutil,subprocess,threading,time,http.server
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
FIRMWARE=os.environ.get('SEXOS_UEFI_CODE',next((p for p in ('/opt/homebrew/share/qemu/edk2-x86_64-code.fd','/usr/share/OVMF/OVMF_CODE_4M.fd') if os.path.exists(p)),'/usr/share/OVMF/OVMF_CODE_4M.fd'))
MIB=1024*1024
TOOL=ROOT/'tools/sexos-release'
def sh(*c,**k):return subprocess.run(c,check=True,capture_output=True,text=True,**k).stdout
def release(dir,version,image):
 dir.mkdir(parents=True,exist_ok=True);shutil.copyfile(image,dir/'sexos-boot.img')
 sh('cargo','run','-q','--release','--manifest-path',str(TOOL/'Cargo.toml'),'--','sign','test',str(version),str(dir/'sexos-boot.img'),str(dir))
def menu_of(disk,esp_first):return sh('mtype','-i',f'{disk}@@{esp_first*512}','::/EFI/BOOT/LIMINE.CFG')
def state(menu):
 m=re.search(r'^# sexos-state (.*)$',menu,re.M);assert m,menu[:400]
 d=dict(kv.split('=') for kv in m[1].split());d['default']=re.search(r'^DEFAULT_ENTRY=(\d+)$',menu,re.M)[1];return d
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out;out.mkdir(parents=True,exist_ok=True)
 disk=out/'nvme.img';disk.unlink(missing_ok=True)
 with disk.open('wb') as f:f.truncate(4*1024*MIB)
 image=out/'boot.img';subprocess.run(['python3',str(ROOT/'scripts/make_boot_image.py'),'--out',str(image)],check=True,capture_output=True)
 www=out/'www'
 release(www/'v2',2,image);release(www/'v3',3,image)
 # A tampered copy: the index says something its signature does not cover.
 bad=www/'bad';bad.mkdir(exist_ok=True)
 for n in('sexos-update.sig','sexos-boot.img'):shutil.copyfile(www/'v2'/n,bad/n)
 (bad/'sexos-update.txt').write_text((www/'v2'/'sexos-update.txt').read_text().replace('version=2','version=9'))
 served={'n':0}
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   # /latest/vN/NAME redirects to /files/vN/NAME, as GitHub's latest/download does.
   m=re.match(r'^/latest/(\w+)/([\w.-]+)$',self.path)
   if m:self.send_response(302);self.send_header('Location',f'/files/{m[1]}/{m[2]}');self.send_header('Content-Length','0');self.end_headers();return
   m=re.match(r'^/files/(\w+)/([\w.-]+)$',self.path);f=www/m[1]/m[2] if m else None
   if not f or not f.exists():self.send_response(404);self.send_header('Content-Length','0');self.end_headers();return
   body=f.read_bytes();served['n']+=1
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*x):pass
 class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
 server=Quiet(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
 base=f'http://10.0.2.2:{server.server_address[1]}/latest'
 # No option ROM: OVMF's network boot would leave the NIC bus-mastering, and sexnet
 # quarantines a NIC in a state it didn't set (UnknownMaster).
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0,romfile='
 os.environ['SEXOS_GATE_BOUND']='3600'
 vars_fd=out/'vars.fd';shutil.copyfile(os.environ.get('SEXOS_UEFI_VARS','/usr/share/OVMF/OVMF_VARS_4M.fd'),vars_fd)
 os.environ['SEXOS_GATE_DEVICES']=f'-drive if=pflash,format=raw,file={vars_fd}'
 os.environ['SEXOS_GATE_FIRMWARE']=FIRMWARE
 from quil_autosave_gate import boot
 KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
 def session(tag,desktop=True):
  g=boot(a.iso,disk,out,tag,a.resolution);t=g['text'];q=g['q']
  def expect(text,pattern,timeout=120):
   g['focus'](153);time.sleep(.5);s=len(t())
   for k in text:
    if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
    else:q.key(KEYS.get(k,k))
    time.sleep(.05)
   q.key('ret');return g['wait'](pattern,s,timeout)
  if desktop:g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  return g,t,expect
 report={}
 # 1. Install from the live ISO.
 os.environ.pop('SEXOS_GATE_NO_CDROM',None)
 g,t,expect=session('install')
 try:expect('install go',r'\[files.install.done\] bytes=\d+',600)
 finally:g['stop']()
 table=sh('sgdisk','-p',str(disk));assert 'SexOS system B' in table,table
 esp_first=int(re.search(r'^\s+1\s+(\d+)\s',table,re.M)[1])
 st=state(menu_of(disk,esp_first));assert st=={'version':'0','candidate':'0','current':'a','trial':'none','prev':'0','default':'1'},st
 os.environ['SEXOS_GATE_NO_CDROM']='1'
 # 2. Update from slot A.
 g,t,expect=session('update')
 try:
  assert re.search(r'\[files.update.start\] slot=A trial=None .*unchanged=1',t()),'no start check in slot A'
  m=expect(f'update {base}/bad',r'\[files.update.refused\] error=(\w+)|\[spindle.update.stopped\] [^\n]*',300)
  assert m[1]=='BadSignature',m[0];report['tampered']=m[0]
  m=expect(f'update {base}/v2',r'\[files.update.done\] slot=B version=2|\[files.update.(?:refused|failed)\][^\n]*|\[spindle.update.stopped\][^\n]*',2400)
  assert 'done' in m[0],m[0];report['update']=m[0]
  assert '[spindle.fetch.redirect] code=302' in t(),'the redirect was not followed'
 finally:g['stop']()
 st=state(menu_of(disk,esp_first));assert (st['current'],st['trial'],st['candidate'],st['default'])==('b','pending','2','1'),st
 prev=st['prev'];assert int(prev)>1,st  # the entry that starts the other slot
 # 3. Slot B starts; the trial starts, then passes.
 g,t,expect=session('trial-b')
 try:
  assert re.search(r'\[files.update.start\] slot=B trial=Started version=0 default_entry='+prev+r' ok=1',t()),re.findall(r'\[files.update[^\n]*',t())
  g['wait'](r'\[files.update.healthy\] slot=B trial=None version=2 default_entry=1 ok=1',0,300)
  m=expect(f'update {base}/v2',r'\[files.update.current\] version=2 offered=2|\[spindle.update.stopped\][^\n]*',300)
  assert 'current' in m[0],m[0]
 finally:g['stop']()
 st=state(menu_of(disk,esp_first));assert (st['version'],st['current'],st['trial'],st['default'])==('2','b','none','1'),st
 menu=menu_of(disk,esp_first);assert ':SexOS previous system' in menu,menu[:2000]
 # 4. Version 3 into slot A; it starts but never reaches the desktop.
 g,t,expect=session('update-3')
 try:
  m=expect(f'update {base}/v3',r'\[files.update.done\] slot=A version=3|\[files.update.(?:refused|failed)\][^\n]*|\[spindle.update.stopped\][^\n]*',2400)
  assert 'done' in m[0],m[0]
 finally:g['stop']()
 # Power goes the moment Files has marked the trial started, before the desktop answers.
 serial=out/'trial-a-dies-serial.log';killed={'at':None}
 def cut():
  d=time.monotonic()+600
  while time.monotonic()<d:
   if serial.exists() and '[files.update.start] slot=A' in serial.read_text('utf-8','replace'):
    subprocess.run(['pkill','-9','-f',f'file:{serial}']);killed['at']=time.monotonic();return
   time.sleep(.01)
 cutter=threading.Thread(target=cut,daemon=True);cutter.start()
 try:boot(a.iso,disk,out,'trial-a-dies',a.resolution)['stop']()
 except(RuntimeError,TimeoutError):pass
 cutter.join(5);assert killed['at'],'Files never started the trial'
 dead=serial.read_text('utf-8','replace')
 assert re.search(r'\[files.update.start\] slot=A trial=Started version=2 default_entry='+prev+r' ok=1',dead),re.findall(r'\[files.update[^\n]*',dead)
 assert '[files.update.healthy] slot=A' not in dead,'the trial passed before the machine was stopped'
 st=state(menu_of(disk,esp_first));assert (st['current'],st['trial'],st['default'])==('a','started',prev),st
 g,t,expect=session('fallback')
 try:
  assert re.search(r'\[files.update.start\] slot=B trial=Failed version=2 default_entry='+prev+r' ok=1',t()),re.findall(r'\[files.update[^\n]*',t())
  q=g['q'];q.cmd('screendump',{'filename':str(out/'fallback.ppm')});time.sleep(.5)
 finally:g['stop']()
 st=state(menu_of(disk,esp_first));assert (st['version'],st['trial'],st['default'])==('2','failed',prev),st
 assert 'No problems found' in sh('sgdisk','-v',str(disk))
 report['final_state']=st
 print({'ok':True,**report})
if __name__=='__main__':main()
