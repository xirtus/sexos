#!/usr/bin/env python3
"""SexOS installs itself beside another system and the machine then starts it from the disk.

Two disks, each a real QEMU boot under UEFI (OVMF):
- linux: a GPT disk as a Linux install leaves it (an EFI System partition with its loader,
  a root partition full of data, free space). Booted live from the ISO, SexOS writes
  nothing to it (the whole disk is hashed before and after); `install` alone only plans;
  `install go` adds SexOS in the free space. Then the other system's partition is byte for
  byte the same, the ESP is clean and keeps its files, the table checks out with sgdisk,
  and with no ISO the firmware starts Limine, whose menu has the other system, and SexOS
  starts from its own partition and keeps a file across a restart.
- blank: a blank disk becomes SexOS's alone (new table and ESP), and starts the same way.
Pass --fetch to install an image fetched over the network instead of the running system.
"""
import argparse,hashlib,http.server,os,re,shutil,subprocess,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
FIRMWARE=os.environ.get('SEXOS_UEFI_CODE','/opt/homebrew/share/qemu/edk2-x86_64-code.fd')
MIB=1024*1024
def sh(*c):return subprocess.run(c,check=True,capture_output=True,text=True).stdout
def digest(path,start=0,length=None):
 h=hashlib.sha256()
 with open(path,'rb') as f:
  f.seek(start);left=length if length is not None else os.path.getsize(path)-start
  while left>0:
   b=f.read(min(left,8*MIB))
   if not b:break
   h.update(b);left-=len(b)
 return h.hexdigest()
def linux_disk(path):
 """GPT: 100 MiB ESP with \\EFI\\debian\\shimx64.efi and no fallback loader, a 300 MiB root."""
 with path.open('wb') as f:f.truncate(4*1024*MIB)
 sh('sgdisk','-n1:2048:+100M','-t1:ef00','-c1:EFI System','-n2:0:+300M','-t2:8300','-c2:debian root',str(path))
 esp=f'{path}@@{2048*512}'
 sh('mformat','-i',esp,'-F','-T',str(100*2048),'-v','ESP','::')
 tmp=path.parent/'shimx64.efi';tmp.write_bytes(b'not really shim, but the other system\'s loader')
 for d in('::/EFI','::/EFI/debian'):sh('mmd','-i',esp,d)
 sh('mcopy','-i',esp,str(tmp),'::/EFI/debian/shimx64.efi');tmp.unlink()
 root=(2048+100*2048)*512
 with path.open('r+b') as f:
  f.seek(root);block=bytes((i*31+7)%256 for i in range(MIB))
  for _ in range(300):f.write(block)
 return dict(root=(root,300*MIB),esp=(2048*512,100*MIB))
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',type=Path,help='unused; kept for older callers');p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--fetch',action='store_true');p.add_argument('--scenario',choices=['linux','blank','both'],default='both');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True)
 results={}
 for scenario in(['linux','blank']if a.scenario=='both'else[a.scenario]):results[scenario]=run(a,scenario)
 print({'ok':True,**results})
def run(a,scenario):
 out=a.out/scenario;out.mkdir(parents=True,exist_ok=True)
 disk=out/'nvme.img';disk.unlink(missing_ok=True)
 if scenario=='linux':parts=linux_disk(disk)
 else:
  with disk.open('wb') as f:f.truncate(4*1024*MIB)
  parts={}
 image=out/'boot.img';subprocess.run(['python3',str(ROOT/'scripts/make_boot_image.py'),'--out',str(image)],check=True,capture_output=True)
 body=image.read_bytes()
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*x):pass
 class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
 server=Quiet(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ['SEXOS_GATE_BOUND']='2400'
 # UEFI throughout, as on every PKU PC, with its own variable store: without one OVMF
 # saves its variables as a file on the first FAT volume it finds (NvVars on the ESP).
 vars_fd=out/'vars.fd';shutil.copyfile(os.environ.get('SEXOS_UEFI_VARS','/usr/share/OVMF/OVMF_VARS_4M.fd'),vars_fd)
 os.environ['SEXOS_GATE_DEVICES']=f'-drive if=pflash,format=raw,file={vars_fd}'
 os.environ.pop('SEXOS_GATE_NO_CDROM',None);os.environ['SEXOS_GATE_FIRMWARE']=FIRMWARE
 from quil_autosave_gate import boot
 KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
 def session(tag):
  g=boot(a.iso,disk,out,tag,a.resolution);t=g['text'];q=g['q']
  def expect(text,pattern,timeout=120):
   g['focus'](153);time.sleep(.5);s=len(t())
   for k in text:
    if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
    else:q.key(KEYS.get(k,k))
    time.sleep(.05)
   q.key('ret');return g['wait'](pattern,s,timeout)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  return g,t,expect
 report={}
 before=digest(disk)
 g,t,expect=session('live')
 try:
  layout=re.search(r'\[sexdrive.disk.layout\] kind=(\S+)',t());assert layout,'no disk layout decision'
  report['live_layout']=layout[1]
  if scenario=='linux':
   assert layout[1]=='foreign-gpt',layout[1]
   assert '[sexdrive.disk.foreign] writes=refused' in t()
  else:assert layout[1]=='whole',layout[1]
  # Only a plan: nothing written.
  expect('install',r'\[spindle.install.line\] Nothing written yet',300)
  plan=re.findall(r'\[spindle.install.line\] ([^\n]*)',t());report['plan']=plan
  if scenario=='linux':assert any('debian' in l for l in plan),plan
  if a.fetch:
   m=expect(f'fetch http://10.0.2.2:{server.server_address[1]}/boot.img boot.img',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',1800)
   if scenario=='linux':assert m[1].startswith('fetch:'),'a foreign disk has no filesystem to save into'
  assert 'KERNEL PANIC' not in t()
 finally:g['stop']()
 if scenario=='linux':assert digest(disk)==before,'the live system wrote to the other system\'s disk'
 g,t,expect=session('install')
 try:
  expect('install go',r'\[files.install.done\] bytes=\d+',600)
  assert 'KERNEL PANIC' not in t()
 finally:g['stop']()
 table=sh('sgdisk','-p',str(disk));verify=sh('sgdisk','-v',str(disk))
 assert 'No problems found' in verify,verify
 assert 'SexOS system A' in table and 'SexOS' in table,table
 report['table']=table.splitlines()[-6:]
 if scenario=='linux':
  root,length=parts['root'];assert digest(disk,root,length)==digest_linux_root(),'the other system\'s partition changed'
  esp=f'{disk}@@{parts["esp"][0]}'
  assert sh('mtype','-i',esp,'::/EFI/debian/shimx64.efi').startswith('not really shim'),'other loader changed'
  menu=sh('mtype','-i',esp,'::/EFI/BOOT/LIMINE.CFG');assert ':debian' in menu and 'IMAGE_PATH=boot:///EFI/debian/shimx64.efi' in menu,menu
  espimg=out/'esp.img'
  with disk.open('rb') as f:f.seek(parts['esp'][0]);espimg.write_bytes(f.read(parts['esp'][1]))
  fsck=subprocess.run(['/sbin/fsck.vfat','-n',str(espimg)],capture_output=True,text=True);assert fsck.returncode==0,fsck.stdout+fsck.stderr
  espimg.unlink()
 # No ISO now: the firmware finds Limine on the ESP, Limine finds SexOS in slot A.
 os.environ['SEXOS_GATE_NO_CDROM']='1';os.environ['SEXOS_GATE_FIRMWARE']=FIRMWARE
 g,t,expect=session('disk')
 try:
  layout=re.search(r'\[sexdrive.disk.layout\] kind=(\S+) base=(\d+)',t());assert layout and layout[1]=='store',layout and layout[0]
  report['installed_layout']=layout[0]
  expect('write kept.txt from the installed system',r'\[spindle.fs.result\] kind=8 status=0')
  time.sleep(5)  # Files commits once writes rest
  q=g['q'];q.cmd('screendump',{'filename':str(out/'booted-from-disk.ppm')});time.sleep(.5)
  assert 'KERNEL PANIC' not in t()
 finally:g['stop']()
 g,t,expect=session('again')
 try:
  expect('cat kept.txt',r'\[spindle.fs.cat\] from the installed system')
  assert 'KERNEL PANIC' not in t()
 finally:g['stop']()
 if scenario=='linux':assert digest(disk,*parts['root'])==digest_linux_root(),'the other system\'s partition changed after SexOS ran'
 return report
def digest_linux_root():
 h=hashlib.sha256();block=bytes((i*31+7)%256 for i in range(MIB))
 for _ in range(300):h.update(block)
 return h.hexdigest()
if __name__=='__main__':main()
