#!/usr/bin/env python3
"""Needle prints and scans without drivers, against stand-ins on this host
(scripts/needle_fakes.py: an IPP Everywhere printer, an eSCL scanner and a DNS-SD
responder, all reached by the guest at 10.0.2.2 through QEMU's user network).

1. `needle discover 10.0.2.2:PORT`: a unicast DNS-SD browse finds the printer and the
   scanner (PTR, SRV, TXT, A) and saves the printer's address.
2. `needle printers`: Get-Printer-Attributes on the saved printer.
3. `needle print URI --test-page`: text rendered with the system font onto a PWG
   raster page; the printer must receive Validate-Job and a Print-Job whose document
   decodes (independently, here) to an A4 page at 75 dpi with ink on it.
4. `needle scan URL scan.pgm`: ScannerCapabilities, ScanJobs, NextDocument; the page
   is written as a PGM of the scanner's 64x48 picture.
5. `needle print URI scan.pgm`: the scanned page printed (a copy), as raster again.
6. `needle jobs URI`: both jobs, completed."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
from needle_fakes import FakePrinter,FakeScanner,FakeResponder,pwg_pages,SCAN_W,SCAN_H
ROOT=Path(__file__).resolve().parents[1]

PRINTER=FakePrinter();SCANNER=FakeScanner();DNS=FakeResponder(PRINTER.port,SCANNER.port)
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
# Six program runs in one boot.
os.environ.setdefault('SEXOS_GATE_BOUND','1200')
from quil_autosave_gate import boot
KEYS={' ':'spc','.':'dot','-':'minus','/':'slash'}
SHIFTED={'+':'equal','@':'2',':':'semicolon','_':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 elf=(ROOT/'iso_root/programs/needle').read_bytes();assert elf[:4]==b'\x7fELF'
 install(disk,[('needle',elf)])
 g=boot(a.iso,disk,a.out,'needle',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED or k.isupper():
    q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED.get(k,k.lower())}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.05)
 def run(line,done):
  g['focus'](153);time.sleep(.5);s=len(t());keys('run +net needle '+line);q.key('ret')
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,120)
  r=g['wait'](r'\[needle\.(?:failed\] [^\n]*|'+done+')',s,300)
  assert not r[0].startswith('[needle.failed]'),r[0]
  g['focus'](int(m[3]));time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s,120)
  return t()[s:],r
 printer=f'ipp://10.0.2.2:{PRINTER.port}/ipp/print'
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  out,_=run(f'discover 10.0.2.2:{DNS.port}',r'discover\] found=(\d+)')
  assert DNS.queries,'no DNS-SD query reached the responder'
  assert re.search(rf'\[needle.printer\] name=Needle_Test uri={re.escape(printer)} formats=application/pdf,image/pwg-raster,image/urf color=T duplex=T',out),out
  assert f'[needle.scanner] name=Needle_Test uri=http://10.0.2.2:{SCANNER.port}/eSCL' in out,out
  out,_=run('printers',r'printer\] name=needle-test ')
  assert re.search(rf'\[needle.printer\] name=needle-test uri={re.escape(printer)} state=idle raster=75dpi media=iso_a4_210x297mm formats=application/pdf,image/pwg-raster,image/urf,application/octet-stream',out),out
  out,r=run(f'print {printer} --test-page',r'job\] id=(\d+) state=(\w+)')
  assert r[2]=='completed',r[0]
  assert [op for op,_ in PRINTER.requests].count(0x04)>=1,'no Validate-Job'
  job=PRINTER.jobs[0];assert job['format']=='image/pwg-raster',job['format']
  pages=pwg_pages(job['data']);assert len(pages)==1,len(pages)
  head,px=pages[0]
  assert (head['width'],head['height'],head['dpi'],head['bpp'],head['cspace'],head['media'])==(620,877,75,8,18,'iso_a4_210x297mm'),head
  ink=sum(1 for b in px if b<200);assert 200<ink<len(px)//10,ink
  assert re.search(rf'\[needle.sent\] id=1 format=image/pwg-raster bytes={len(job["data"])} pages=1',out),out
  out,r=run(f'scan http://10.0.2.2:{SCANNER.port}/eSCL scan.pgm',r'scan\] pages=(\d+) bytes=(\d+)')
  assert r[1]=='1' and int(r[2])==len(f'P5\n{SCAN_W} {SCAN_H}\n255\n')+SCAN_W*SCAN_H,r[0]
  assert SCANNER.served==1 and 'Grayscale8' in SCANNER.settings[0] and 'image/png' in SCANNER.settings[0],SCANNER.settings
  out,r=run(f'print {printer} scan.pgm',r'job\] id=(\d+) state=(\w+)')
  assert r[1]=='2' and r[2]=='completed',r[0]
  copy=pwg_pages(PRINTER.jobs[1]['data'])[0][1]
  # The scanned frame is dark, its inside light: the copy has both.
  dark=sum(1 for b in copy if b<64);assert dark>500,dark
  out,r=run(f'jobs {printer}',r'jobs\] count=(\d+)')
  assert r[1]=='2' and out.count('state=completed')==2,out
  q.cmd('screendump',{'filename':str(a.out/'needle.ppm')});time.sleep(.5)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'jobs':[(j['id'],j['format'],len(j['data'])) for j in PRINTER.jobs],'scans':SCANNER.served})
 finally:
  g['stop']();PRINTER.stop();SCANNER.stop();DNS.stop()
if __name__=='__main__':main()
