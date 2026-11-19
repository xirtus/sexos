#!/usr/bin/env python3
"""Swingers on SexOS downloads a torrent from a real BitTorrent client on this host.

aria2 seeds a 4 MB file on 127.0.0.1 (the guest sees the host as 10.0.2.2). In SexOS,
`run +net swingers get HASH --peer 10.0.2.2:PORT` fetches the metadata over ut_metadata,
downloads and checks every piece, and records the SHA-256 of the file it wrote, which
must equal the host file's."""
import argparse,hashlib,os,re,shutil,socket,subprocess,tempfile,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','.':'dot','-':'minus','/':'slash'}
SHIFTED={'+':'equal',':':'semicolon'}

def free_port():
 with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 elf=(ROOT/'iso_root/programs/swingers').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('swingers',elf)])
 work=Path(tempfile.mkdtemp(prefix='sexos-swingers-'));seed=work/'seed';seed.mkdir()
 (seed/'film.bin').write_bytes(os.urandom(4_000_000));want=hashlib.sha256((seed/'film.bin').read_bytes()).hexdigest()
 subprocess.run(['cargo','build','-q','--locked','-p','swingers'],cwd=ROOT,check=True)
 made=subprocess.run([str(ROOT/'target/debug/swingers'),'create',str(seed/'film.bin'),'--out',str(work/'film.torrent'),'--v1'],check=True,capture_output=True,text=True).stdout
 ih=re.search(r'btih%3A([0-9a-f]{40})',made)[1];port=free_port()
 aria=subprocess.Popen(['aria2c','-q','--enable-dht=false','--bt-enable-lpd=false','--seed-ratio=0','--seed-time=30',f'--listen-port={port}','-d',str(seed),'-V',str(work/'film.torrent')],stdout=open(work/'aria.log','w'),stderr=subprocess.STDOUT)
 g=boot(a.iso,disk,a.out,'swingers',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED[k]}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.05)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t())
  keys(f'run +net swingers get {ih} --peer 10.0.2.2:{port}');q.key('ret')
  g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,120)
  g['wait'](r'\[swingers.added\] '+ih,s,120)
  m=g['wait'](r'\[swingers.(file|failed)\] ([^\n]*)\n',s,600)
  assert m[1]=='file',m[0]
  got=m[2].split()[-1];assert got==want,(got,want)
  g['wait'](r'\[swingers.done\] '+ih,s,60)
  q.cmd('screendump',{'filename':str(a.out/'swingers.ppm')});time.sleep(.5)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'info_hash':ih,'sha256':got})
 finally:
  g['stop']();aria.kill();shutil.rmtree(work,ignore_errors=True)
if __name__=='__main__':main()
