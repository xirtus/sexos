#!/usr/bin/env python3
"""SexOS installs itself: a boot image fetched over the network is written to the disk, and the machine then starts from that disk alone, under UEFI, with the files it had."""
import argparse,http.server,os,shutil,subprocess,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
FIRMWARE=os.environ.get('SEXOS_UEFI_CODE','/opt/homebrew/share/qemu/edk2-x86_64-code.fd')
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 image=a.out/'boot.img';subprocess.run(['python3',str(ROOT/'scripts/make_boot_image.py'),'--out',str(image)],check=True,capture_output=True)
 body=image.read_bytes()
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*x):pass
 class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
 server=Quiet(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ['SEXOS_GATE_BOUND']='2400'
 from quil_autosave_gate import boot
 KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
 def session(tag):
  g=boot(a.iso,disk,a.out,tag,a.resolution);t=g['text'];q=g['q']
  def expect(text,pattern,timeout=120):
   g['focus'](153);time.sleep(.5);s=len(t())
   for k in text:
    if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
    else:q.key(KEYS.get(k,k))
    time.sleep(.05)
   q.key('ret');return g['wait'](pattern,s,timeout)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  return g,t,expect
 g,t,expect=session('cd')
 try:
  expect('write kept.txt from before the install',r'\[spindle.fs.result\] kind=8 status=0')
  m=expect(f'fetch http://10.0.2.2:{server.server_address[1]}/boot.img boot.img',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',1800);assert m[1]==f'Saved {len(body)} bytes',m[1]
  expect('install boot.img',r'\[files.install.done\] bytes='+str(len(body)),300)
  assert 'KERNEL PANIC' not in t()
 finally:g['stop']()
 # No ISO now: UEFI finds the disk's boot partition.
 os.environ['SEXOS_GATE_NO_CDROM']='1';os.environ['SEXOS_GATE_FIRMWARE']=FIRMWARE
 g,t,expect=session('disk')
 try:
  assert 'NVMe' in t() or 'BdsDxe' in t(),'firmware log missing'
  expect('cat kept.txt',r'\[spindle.fs.cat\] from before the install')
  q=g['q'];q.cmd('screendump',{'filename':str(a.out/'booted-from-disk.ppm')});time.sleep(.5)
  assert 'KERNEL PANIC' not in t()
  print({'ok':True,'image_bytes':len(body)})
 finally:g['stop']()
if __name__=='__main__':main()
