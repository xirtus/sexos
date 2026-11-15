#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Corset (apps/corset) partitions a blank USB disk, and Files mounts what it made.

Everything is typed as a person would. The gate checks:

* `run corset --probe-system`: Corset's raw write to the system disk is refused by Files
  (`[files.disks.refused] disk=system ... why=system`) and Corset is told so (DENIED).
* A blank 256 MiB QEMU usb-storage disk is plugged in after boot. In Corset, Enter on
  usb0 asks Files; the Files app (started with `loom`, the person's picker) shows the
  question naming the disk's product and size, and Y allows it for that run.
* G makes a GPT; A adds a 100 MiB basic data partition, F formats it FAT32 (CORSETFAT);
  A (Enter for size and type) adds one over the rest of the disk, F formats it exFAT (CORSETEX). Enter shows the
  five steps; Enter applies (a blank disk loses nothing, so no name is typed).
* Before the first write Files lets go of the disk; when Corset is done Files scans it
  again and mounts both volumes under /media. Spindle writes a file into each and reads
  it back.
* Afterwards, on the host: the stick holds a valid GPT with both partitions (sgdisk -v
  when installed) and mtools reads the file on the FAT32 one."""
import argparse,os,re,shutil,struct,subprocess,time
from pathlib import Path
from install_program import install
os.environ.setdefault('SEXOS_GATE_BOUND','1200')
from quil_autosave_gate import boot
from lua_gate import qcodes,SURFACE
ROOT=Path(__file__).resolve().parents[1]
SPINDLE=153
MIB=1024*1024

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 programs=[]
 for name in ('corset','loom'):
  elf=(ROOT/'iso_root/programs'/name).read_bytes();assert elf[:4]==b'\x7fELF',name;programs.append((name,elf))
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,programs)
 stick=a.out/'stick.img'
 with stick.open('wb') as f:f.truncate(256*MIB)
 # Plugged in after boot: present at power-on, the firmware would take it over.
 os.environ.pop('SEXOS_GATE_DEVICES',None)
 g=boot(a.iso,disk,a.out,'corset',a.resolution);t=g['text'];q=g['q']
 def send(ch):q.cmd('send-key',{'keys':[{'type':'qcode','data':k}for k in qcodes(ch)],'hold-time':40})
 def type_line(line):
  g['focus'](SPINDLE);time.sleep(.5)
  base=None
  for n,ch in enumerate(line):
   s=len(t());send(ch)
   m=g['wait'](rf'\[spindle\.input\.recv\] key=printable ch={re.escape(ch)} len=(\d+)\n',s,30)
   if base is None:base=int(m[1])-n
   assert int(m[1])==base+n,f'Spindle dropped a key before {line[:n+1]!r}'
  s=len(t());q.key('ret')
  g['wait'](rf'\[spindle\.input\.recv\] key=enter len={base+len(line)-1}\n',s,30)
 def shell(line,kind):
  s=len(t());type_line(line)
  return g['wait'](rf'\[spindle\.fs\.result\] kind={kind} status=(\d+) ',s,60)[1]
 def launch(line,ready):
  s=len(t());type_line(line)
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,120)
  g['wait'](ready,s,120)
  return int(m[1])
 def press(k,pattern,timeout=60):
  s=len(t());q.key(k);return g['wait'](pattern,s,timeout)
 def type_app(text):
  for n,ch in enumerate(text):
   s=len(t());send(ch);g['wait'](rf'\[corset\.input\] len={n+1}\n',s,30)
 result={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  q.cmd('blockdev-add',{'driver':'raw','node-name':'stickdrive','file':{'driver':'file','filename':str(stick)}})
  s=len(t());q.cmd('device_add',{'driver':'usb-storage','bus':'xhci.0','drive':'stickdrive','id':'stick'})
  g['wait'](r'\[sexusb\.storage\] slot=\d+ disk=0 [^\n]*sectors=524288 ',s,60)
  g['wait'](r'\[files\.media\.usb\.unknown\] disk=0 ',s,60)

  # The person's picker: Files holding the whole filesystem shows the questions.
  files=launch('loom',r'\[files\.app\.ready\] target=\d+ task=\d+ root=/ portal=1')
  # The system disk, never opened raw, whatever is asked.
  s=len(t());corset=launch('run corset --probe-system',r'\[corset\.ready\] target=\d+ task=\d+ disks=2\n')
  probe=re.search(r'\[corset\.probe\] disk=system status=(\d+) message=([^\n]*)\n',t()[s:]);assert probe and probe[1]=='11',probe and probe[0]
  assert re.search(r'\[files\.disks\.refused\] disk=system write=1 lba=\d+ why=system',t()[s:]),'Files did not log the refused system write'
  assert not re.search(r'\[files\.disks\.write\] disk=0x100 ',t()),'a write reached the system disk'
  listed=re.search(r'\[corset\.disks\] count=2 list=([^\n]*)\n',t()[s:])[1].split(',')
  assert 'usb0:524288:0' in listed and any(d.startswith('system:') and d.endswith(':2') for d in listed),listed
  result['system_write']='refused'

  # Enter on usb0 (the cursor starts on the first disk Corset may change): Files asks.
  g['focus'](SURFACE[corset]);time.sleep(.5)
  s=len(t());q.key('ret')
  g['wait'](r'\[corset\.waiting\] disk=usb0\n',s,60)
  asked=g['wait'](r'\[files\.disks\.asked\] row=\d+ disk=0x200 ticket=\d+ text=([^\n]*)\n',s,60)[1]
  assert asked=='usb0 (QEMU QEMU HARDDISK, 256 MiB)',asked
  g['wait'](r'\[files\.app\.disk\.question\] program=corset disk=usb0 \(QEMU QEMU HARDDISK, 256 MiB\)\n',s,60)
  g['focus'](SURFACE[files]);time.sleep(.5)
  press('y',r'\[files\.app\.disk\.answered\] program=corset disk=[^\n]* yes=1\n')
  g['wait'](r'\[files\.disks\.granted\] row=\d+ task=\d+ disk=0x200 name=usb0 product=QEMU QEMU HARDDISK sectors=524288\n',s,60)
  g['wait'](r'\[corset\.opened\] disk=usb0 sectors=524288 table=0 parts=0 system=0\n',s,60)

  g['focus'](SURFACE[corset]);time.sleep(.5)
  press('g',r'\[corset\.planned\] steps=1 destructive=0 text=Create a new GPT')
  def add(size,name,kind,steps):
   q.key('a');time.sleep(.5)
   type_app(size);q.key('ret');time.sleep(.3)
   type_app(name);q.key('ret');time.sleep(.3)
   type_app(kind);s2=len(t());q.key('ret')
   return g['wait'](rf'\[corset\.planned\] steps={steps} destructive=0 text=(Add partition [^\n]*)\n',s2,60)[1]
  def fmt(choice,label,steps):
   q.key('f');time.sleep(.5)
   type_app(choice);q.key('ret');time.sleep(.3)
   type_app(label);s=len(t());q.key('ret')
   return g['wait'](rf'\[corset\.planned\] steps={steps} destructive=0 text=(Format [^\n]*)\n',s,60)[1]
  first=add('100','SHARED','1',2)
  assert first=='Add partition 1 "SHARED" (Basic data (FAT, exFAT)), 100 MiB from 1 MiB to 101 MiB.',first
  fmt('1','CORSETFAT',3)
  press('down',r'\[corset\.cursor\] row=1\n')
  second=add('','MORE','',4)
  assert second.startswith('Add partition 2 "MORE" (Basic data (FAT, exFAT)), 154 MiB from 101 MiB'),second
  fmt('2','CORSETEX',5)
  s=len(t());q.key('ret')
  g['wait'](r'\[corset\.review\] disk=usb0 steps=5 destructive=0\n',s,60)
  g['wait'](r'\[corset\.step\] n=5 destructive=0 text=[^\n]*\n',s,30)  # the review lists its steps after the header
  steps=re.findall(r'\[corset\.step\] n=(\d) destructive=0 text=([^\n]*)\n',t()[s:])
  assert [n for n,_ in steps]==['1','2','3','4','5'],steps
  s=len(t());q.key('ret')
  ap=g['wait'](r'\[corset\.applied\] disk=usb0 table=1 before=(\d+) after=(\d+) parts=2\n',s,900)
  g['wait'](r'\[corset\.done\] disk=usb0\n',s,120)
  # Files let go of the disk before the first write, and took it back after.
  log=t()[s:]
  assert re.search(r'\[files\.disks\.released\] row=\d+ disk=0x200 volumes=0\n',log),'no release before the first write'
  assert log.index('[files.disks.released]')<log.index('[files.disks.write]'),'a write before the release'
  writes=re.findall(r'\[files\.disks\.write\] disk=0x200 lba=(\d+) sectors=(\d+)\n',log);assert writes,'no write logged'
  assert all(int(l)+int(n)<=524288 for l,n in writes)
  g['wait'](r'\[files\.disks\.done\] row=\d+ disk=0x200 rescanned=1\n',s,120)
  g['wait'](r'\[files\.media\.mounted\] name=CORSETFAT format=FAT32 [^\n]*source=usb disk=0\n',s,120)
  g['wait'](r'\[files\.media\.mounted\] name=CORSETEX format=exFAT [^\n]*source=usb disk=0\n',s,120)
  result.update(before_table=int(ap[1]),after_table=int(ap[2]),writes=len(writes))

  # Spindle writes into both and reads back.
  for vol,text in (('CORSETFAT','hello fat'),('CORSETEX','hello exfat')):
   assert shell(f'write /media/{vol}/hello.txt {text}',8)=='0',vol
   s=len(t());type_line(f'cat /media/{vol}/hello.txt')
   g['wait'](rf'\[spindle\.fs\.cat\] {text}\n',s,60)
  s=len(t());type_line('unmount CORSETFAT')
  g['wait'](r'\[spindle\.fs\.line\] Safe to remove\.',s,60)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  q.cmd('screendump',{'filename':str(a.out/'corset.ppm')});time.sleep(.5)
 finally:g['stop']()
 # The stick as other systems see it.
 raw=stick.read_bytes()[:34*512]
 assert raw[512:520]==b'EFI PART','no GPT header'
 entries=[raw[1024+128*i:1024+128*(i+1)] for i in range(4)]
 parts=[(struct.unpack('<Q',e[32:40])[0],struct.unpack('<Q',e[40:48])[0]) for e in entries if any(e[:16])]
 assert parts[0]==(2048,206847),parts;assert parts[1][0]==206848,parts
 result['partitions']=parts
 if shutil.which('sgdisk'):
  v=subprocess.run(['sgdisk','-v',str(stick)],capture_output=True,text=True).stdout
  assert 'No problems found' in v,v;result['sgdisk']='clean'
 if shutil.which('mtype'):
  out=subprocess.run(['mtype','-i',f'{stick}@@{2048*512}','::HELLO.TXT'],capture_output=True,text=True,env={**os.environ,'MTOOLS_SKIP_CHECK':'1'})
  assert out.stdout.strip()=='hello fat',(out.stdout,out.stderr);result['mtools']='reads hello.txt'
 stick.unlink();disk.unlink()
 print({'ok':True,**result})
if __name__=='__main__':main()
