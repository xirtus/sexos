#!/usr/bin/env python3
"""Needle's whole flow on the build host, without QEMU: the host build of
apps/needle (`cargo build -p needle`) against the same fakes scripts/needle_gate.py
uses, with the same checks. Run from the repository root after the build:
python3 scripts/needle_host_test.py [--bin target/debug/needle]"""
import argparse,re,subprocess,sys,tempfile
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from needle_fakes import FakePrinter,FakeScanner,FakeResponder,pwg_pages,SCAN_W,SCAN_H
ROOT=Path(__file__).resolve().parents[1]

def main():
 p=argparse.ArgumentParser();p.add_argument('--bin',type=Path,default=ROOT/'target/debug/needle');a=p.parse_args()
 printer=FakePrinter('127.0.0.1');scanner=FakeScanner();dns=FakeResponder(printer.port,scanner.port,(127,0,0,1))
 work=Path(tempfile.mkdtemp(prefix='needle-host-'))
 def run(*args):
  r=subprocess.run([str(a.bin.resolve()),*args],cwd=work,capture_output=True,text=True,timeout=120)
  assert r.returncode==0,(args,r.stdout,r.stderr)
  return r.stdout
 try:
  uri=printer.uri
  out=run('discover',f'127.0.0.1:{dns.port}')
  assert re.search(rf'\[needle.printer\] name=Needle_Test uri={re.escape(uri)} formats=application/pdf,image/pwg-raster,image/urf color=T duplex=T',out),out
  assert f'[needle.scanner] name=Needle_Test uri=http://127.0.0.1:{scanner.port}/eSCL' in out and '[needle.discover] found=2' in out,out
  out=run('printers')
  assert re.search(rf'\[needle.printer\] name=needle-test uri={re.escape(uri)} state=idle raster=75dpi media=iso_a4_210x297mm formats=application/pdf,image/pwg-raster,image/urf,application/octet-stream',out),out
  out=run('print',uri,'--test-page')
  assert '[needle.job] id=1 state=completed' in out,out
  job=printer.jobs[0];assert job['format']=='image/pwg-raster'
  head,px=pwg_pages(job['data'])[0]
  assert (head['width'],head['height'],head['dpi'],head['bpp'],head['cspace'],head['media'])==(620,877,75,8,18,'iso_a4_210x297mm'),head
  ink=sum(1 for b in px if b<200);assert 200<ink<len(px)//10,ink
  out=run('scan',f'http://127.0.0.1:{scanner.port}/eSCL','scan.pgm')
  pgm_len=len(f"P5\n{SCAN_W} {SCAN_H}\n255\n")+SCAN_W*SCAN_H
  assert f"[needle.scan] pages=1 bytes={pgm_len} " in out,out
  pgm=(work/'scan.pgm').read_bytes();assert pgm.startswith(f'P5\n{SCAN_W} {SCAN_H}\n255\n'.encode())
  assert 'Grayscale8' in scanner.settings[0] and 'image/png' in scanner.settings[0]
  out=run('print',uri,'scan.pgm')
  assert '[needle.job] id=2 state=completed' in out,out
  dark=sum(1 for b in pwg_pages(printer.jobs[1]['data'])[0][1] if b<64);assert dark>500,dark
  (work/'doc.pdf').write_bytes(b'%PDF-1.4\n%fake\n')
  out=run('print',uri,'doc.pdf');assert printer.jobs[2]['format']=='application/pdf' and printer.jobs[2]['data']==b'%PDF-1.4\n%fake\n',out
  out=run('jobs',uri);assert '[needle.jobs] count=3' in out and out.count('state=completed')==3,out
  r=subprocess.run([str(a.bin.resolve()),'cancel',uri,'1'],cwd=work,capture_output=True,text=True,timeout=60)
  assert r.returncode==1 and 'status=client-error-not-possible' in r.stdout,r.stdout
  print({'ok':True,'jobs':[(j['id'],j['format'],len(j['data'])) for j in printer.jobs]})
 finally:
  printer.stop();scanner.stop();dns.stop()
if __name__=='__main__':main()
