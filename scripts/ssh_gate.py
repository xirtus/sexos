#!/usr/bin/env python3
"""The `ssh` program logs in to a real OpenSSH server on this host.

1. `run +net ssh -p PORT USER@10.0.2.2 echo sexos-ssh-ok` on a fresh disk: the program
   makes its own Ed25519 key, prints the authorized_keys line, remembers the host key,
   and is refused (the server doesn't know the key yet).
2. The gate adds that line to the server's authorized_keys.
3. The same command again: the remembered host key matches, the key logs in, the
   command's output comes back and its exit status is 0.

The server is OpenSSH's sshd on 127.0.0.1, which QEMU's user network shows the guest
as 10.0.2.2."""
import argparse,getpass,os,re,shutil,socket,subprocess,tempfile,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]

def free_port():
 with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]

class Sshd:
 def __init__(self):
  self.dir=Path(tempfile.mkdtemp(prefix='sexos-sshd-'));self.port=free_port();self.user=getpass.getuser()
  key=self.dir/'host_ed25519';subprocess.run(['ssh-keygen','-q','-t','ed25519','-N','','-f',str(key)],check=True)
  self.fingerprint=subprocess.run(['ssh-keygen','-l','-f',str(key)+'.pub'],check=True,capture_output=True,text=True).stdout.split()[1]
  self.auth=self.dir/'authorized_keys';self.auth.write_text('')
  cfg=self.dir/'sshd_config'
  cfg.write_text(f'Port {self.port}\nListenAddress 127.0.0.1\nHostKey {key}\nAuthorizedKeysFile {self.auth}\nStrictModes no\nUsePAM no\nPermitRootLogin yes\nPasswordAuthentication no\nPidFile {self.dir}/pid\n')
  sudo=[] if os.geteuid()==0 else ['sudo','-n']
  if sudo:subprocess.run(sudo+['mkdir','-p','/run/sshd'],check=True)
  else:os.makedirs('/run/sshd',exist_ok=True)
  self.sudo=sudo;self.proc=subprocess.Popen(sudo+['/usr/sbin/sshd','-D','-e','-f',str(cfg)],stderr=open(self.dir/'sshd.log','w'))
  for _ in range(100):
   try:socket.create_connection(('127.0.0.1',self.port),timeout=1).close();break
   except OSError:time.sleep(.1)
 def stop(self):
  try:subprocess.run(self.sudo+['kill',(self.dir/'pid').read_text().strip()])
  except OSError:pass
  self.proc.kill()

SSHD=Sshd()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','.':'dot','-':'minus','/':'slash'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 elf=(ROOT/'iso_root/programs/ssh').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('ssh',elf)])
 g=boot(a.iso,disk,a.out,'ssh',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   elif k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.05)
 def run(line):
  g['focus'](153);time.sleep(.5);s=len(t());keys(line);q.key('ret')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,120)
  return s,m
 def close(m,s):
  g['focus'](int(m[3]));time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,120)
 line=f'run +net ssh -p {SSHD.port} {SSHD.user}@10.0.2.2 echo sexos-ssh-ok'
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  s,m=run(line)
  key=g['wait'](r'\[ssh.key\] (ssh-ed25519 [A-Za-z0-9+/=]+)\n',s,300)[1]
  host=g['wait'](r'\[ssh.host\] ([^\n]+)\n',s,300)[1]
  assert host==f'[10.0.2.2]:{SSHD.port} ssh-ed25519 {SSHD.fingerprint}',host
  r=g['wait'](r'\[ssh.(failed|done)\] ([^\n]*)\n',s,300);assert r[0]=='[ssh.failed] AuthFailed\n',r[0]
  close(m,s)
  SSHD.auth.write_text(key+' sexos\n')
  s,m=run(line)
  g['wait'](r'\[ssh.authenticated\]',s,300)
  g['wait'](r'\[ssh.out\] sexos-ssh-ok\n',s,120)
  d=g['wait'](r'\[ssh.done\] status=(-?\d+)\n',s,120);assert d[1]=='0',d[0]
  # The key was made once and the host remembered: no new key or new host this time.
  assert '[ssh.key] '+key in t()[s:] and 'Made a new key' not in t()[s:] and 'New host' not in t()[s:]
  q.cmd('screendump',{'filename':str(a.out/'ssh.ppm')});time.sleep(.5)
  close(m,s)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'host':host,'key':key[:40]+'...'})
 finally:
  g['stop']();SSHD.stop()
if __name__=='__main__':main()
