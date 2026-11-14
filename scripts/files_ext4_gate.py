#!/usr/bin/env python3
"""ext4 sticks are written, and Linux finds them whole.

A USB stick formatted by Linux's mkfs.ext4 (metadata_csum, a journal, a file already on
it) is plugged in. From Spindle a folder is made on it, a new file is written there, a
file Linux wrote is appended to, and another is renamed into the folder; the stick is
then unmounted (written out and flushed) and pulled out. On the host, `e2fsck -fn` finds
nothing to fix and `debugfs` reads every file as SexOS wrote it."""
import argparse,os,shutil,subprocess,time
from pathlib import Path
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 shutil.rmtree(a.out,ignore_errors=True);a.out.mkdir(parents=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 seed=a.out/'seed';seed.mkdir();(seed/'linux.txt').write_bytes(b'written by linux\n');(seed/'move.txt').write_bytes(b'moved by sexos\n')
 (seed/'big').mkdir()
 for i in range(300):(seed/'big'/f'linux file {i:03}.txt').write_bytes(b'%d\n'%i)
 (seed/'journalled.txt').write_bytes(b'o'*4096)
 stick=a.out/'ext4.img'
 subprocess.run(['truncate','-s','64M',str(stick)],check=True)
 subprocess.run(['mkfs.ext4','-q','-F','-b','4096','-L','EXTSTICK','-d',str(seed),str(stick)],check=True)
 subprocess.run(['e2fsck','-fyD',str(stick)],capture_output=True)
 assert subprocess.run(['e2fsck','-fn',str(stick)],capture_output=True).returncode==0
 assert '0x81000' in subprocess.run(['debugfs','-R','stat /big',str(stick)],capture_output=True,text=True).stdout
 # The last transaction Linux wrote is still only in the journal.
 blk=subprocess.run(['debugfs','-R','bmap /journalled.txt 0',str(stick)],capture_output=True,text=True).stdout.strip()
 (a.out/'new.blk').write_bytes((b'from the journal\n'+b' '*4079)*2)
 (a.out/'jcmd').write_text(f'jo -c\njw -b {blk} {a.out/"new.blk"}\njc\n')
 subprocess.run(['debugfs','-w','-f',str(a.out/'jcmd'),str(stick)],capture_output=True,check=True)
 assert 'needs_recovery' in subprocess.run(['dumpe2fs','-h',str(stick)],capture_output=True,text=True).stdout
 os.environ.pop('SEXOS_GATE_DEVICES',None);os.environ.setdefault('SEXOS_GATE_BOUND','600')
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,a.out,'ext4',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   shift=k.isupper() or k in ':_'
   code={':':'semicolon','_':'minus'}.get(k,KEYS.get(k,k.lower()))
   if shift:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':code}],'hold-time':80})
   else:q.key(code)
   time.sleep(.06)
 def expect(text,pattern,timeout=60):s=len(t());g['focus'](153);time.sleep(.5);keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  q.cmd('blockdev-add',{'driver':'raw','node-name':'extdrive','file':{'driver':'file','filename':str(stick)}})
  s=len(t())
  for _ in range(40):
   try:q.cmd('device_add',{'driver':'usb-storage','bus':'xhci.0','drive':'extdrive','id':'ext1'});break
   except RuntimeError:time.sleep(.5)
  r=g['wait'](r'\[files\.media\.ext4\.replayed\] blocks=(\d+)',s,120);assert int(r[1])>=1,r[0]
  m=g['wait'](r'\[files\.media\.mounted\] name=EXTSTICK format=ext4 ([^\n]*)',s,120)
  assert 'read_only=1' not in m[0] and 'ro=1' not in m[0],m[0]
  expect('cat /media/EXTSTICK/linux.txt',r'\[spindle.fs.cat\] written by linux')
  expect('cat /media/EXTSTICK/journalled.txt',r'\[spindle.fs.cat\] from the journal')
  expect('write /media/EXTSTICK/big/from-sexos.txt into an indexed folder',r'\[spindle.fs.result\] kind=8 status=0')
  expect('mkdir /media/EXTSTICK/from-sexos',r'\[spindle.fs.result\] kind=3 status=0')
  expect('write /media/EXTSTICK/from-sexos/hello.txt hello from sexos',r'\[spindle.fs.result\] kind=8 status=0')
  expect('mv /media/EXTSTICK/move.txt /media/EXTSTICK/from-sexos/moved.txt',r'\[spindle.fs.result\] kind=\d+ status=0')
  expect('append /media/EXTSTICK/linux.txt and by sexos',r'\[spindle.fs.result\] kind=\d+ status=0')
  expect('cat /media/EXTSTICK/from-sexos/hello.txt',r'\[spindle.fs.cat\] hello from sexos')
  expect('cat /media/EXTSTICK/from-sexos/moved.txt',r'\[spindle.fs.cat\] moved by sexos')
  expect('unmount EXTSTICK',r'\[spindle.fs.line\] Safe to remove\.')
  s=len(t());q.cmd('device_del',{'id':'ext1'});g['wait'](r'\[sexusb\.storage\.gone\] disk=\d+ ',s,60)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
 finally:g['stop']()
 r=subprocess.run(['e2fsck','-fn',str(stick)],capture_output=True,text=True)
 print(r.stdout.strip());assert r.returncode==0,r.stdout+r.stderr
 cat=lambda f:subprocess.run(['debugfs','-R',f'cat "{f}"',str(stick)],capture_output=True).stdout
 assert cat('/from-sexos/hello.txt').rstrip(b'\n')==b'hello from sexos',cat('/from-sexos/hello.txt')
 assert cat('/from-sexos/moved.txt')==b'moved by sexos\n'
 assert cat('/linux.txt').startswith(b'written by linux\nand by sexos'),cat('/linux.txt')
 assert cat('/journalled.txt').startswith(b'from the journal\n'),cat('/journalled.txt')[:40]
 assert cat('/big/from-sexos.txt').rstrip(b'\n')==b'into an indexed folder',cat('/big/from-sexos.txt')
 assert cat('/big/linux file 299.txt')==b'299\n'
 assert 'needs_recovery' not in subprocess.run(['dumpe2fs','-h',str(stick)],capture_output=True,text=True).stdout
 assert b'move.txt' not in subprocess.run(['debugfs','-R','ls /',str(stick)],capture_output=True).stdout
 print({'ok':True})
if __name__=='__main__':main()
