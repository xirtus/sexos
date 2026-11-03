#!/usr/bin/env python3
"""Browser fetches a fixed PNG (`/art.png`, key P) and shows it.

Same BOOT endpoint, trust profile and held route as the QOI asset gate; only
the fixed resource differs. The PNG is made here by zlib from the QOI gate's
procedural RGBA oracle, so the decoded pixel hash must equal that gate's
PIXEL_HASH; screen pixels inside the image are checked against the
straight-alpha blend over the Browser background.
"""
import argparse,hashlib,json,os,re,shutil,struct,subprocess,time,zlib
from pathlib import Path
import https_document_gate as doc
import https_html_gate as html
import https_asset_gate as asset
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from network_contention_gate import frozen_binding,MAX_PCAP
from check_disk_persistence import ppm_rgb

REQUEST=b'GET /art.png HTTP/1.1\r\nHost: probe.sexos.test:46080\r\nConnection: close\r\nAccept: image/png\r\n\r\n'
def make_png():
 def chunk(t,d):return struct.pack('>I',len(d))+t+d+struct.pack('>I',zlib.crc32(t+d))
 px=asset.rgba();w=h=48;raw=b''.join(b'\0'+px[y*w*4:(y+1)*w*4]for y in range(h))
 return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw,9))+chunk(b'IEND',b'')
BODY=make_png()
def response(mode,page=BODY):
 return b'\r\n'.join([b'HTTP/1.1 200 OK',b'Content-Type: image/png',b'Content-Length: '+str(len(page)).encode(),b'Connection: close'])+b'\r\n\r\n'+page
class PngPeer(html.HtmlPeer):
 EXCHANGES=((REQUEST,BODY),)
SCOPE='Actual fixed BOOT-controlled HTTPS image/png fetched by key P, decoded by the allocation-free PNG decoder after positive RELEASE, pixel hash equal to the QOI oracle and screen pixels checked. No other formats, sizes or public web claim.'

def main():
 from quil_workflow_gate import palette_commands,palette_select
 p=argparse.ArgumentParser();[p.add_argument(f'--{n}',required=True,type=Path)for n in('iso','source-disk','binding','out')];p.add_argument('--model',choices=('e1000','e1000e'),default='e1000e');p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 binding=frozen_binding(root,a.iso.resolve(strict=True),a.binding.resolve(strict=True));held=json.loads(a.binding.read_text())
 for name in('scripts/https_png_gate.py','scripts/https_html_gate.py'):
  if held['runner_sha256'].get(name)!=hashlib.sha256((root/name).read_bytes()).hexdigest():raise RuntimeError(name+' absent from frozen binding')
 html.response=response
 peer=PngPeer(root);peer.start();disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);serial=out/'serial.log';pcap=out/'wire.pcap';sock=Path('/tmp')/f'sex-png-{os.getpid()}.sock'
 command=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-rtc',doc.RTC_NORMAL,'-m','512M','-vga','none','-device','VGA,xres='+a.resolution.replace('x',',yres='),'-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(a.iso.resolve()),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-no-reboot','-no-shutdown','-nic','none','-netdev','user,id=peer','-device',f'{a.model},netdev=peer,mac=52:54:00:12:34:56','-object',f'filter-dump,id=capture,netdev=peer,file={pcap}']
 (out/'art.png').write_bytes(BODY);proc=subprocess.Popen(command,stdout=(out/'qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic();failure=None;checks={}
 def trace():return serial.read_text(errors='replace')if serial.exists()else''
 def health():
  if time.monotonic()-began>300:raise TimeoutError('bounded 300 second PNG gate')
  if proc.poll()is not None:raise RuntimeError('guest exited')
  if peer.errors:raise RuntimeError(str(peer.errors))
  if re.search(r'KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at|serial.record.overflow',trace()):raise RuntimeError('guest failure')
 def wait(pattern,start=0,timeout=90):
  d=time.monotonic()+timeout
  while time.monotonic()<d:
   health();m=re.search(pattern,trace()[start:])
   if m:return m
   time.sleep(.02)
  raise TimeoutError(pattern)
 def key(code,pattern):b=len(trace());q.key(code);return wait(pattern,b)
 try:
  d=time.monotonic()+15
  while not sock.exists():
   health()
   if time.monotonic()>d:raise TimeoutError('QMP unavailable')
   time.sleep(.05)
  q=QMP(sock)
  if a.resolution=='1024x768':wait(r'SexOS compact 1024x768',timeout=10);q.key('down');q.key('ret')
  wait(r'\[shell.app.registered.reply\] target=15 .*committed=1');wait(r'\[browser.net.job.released\] ticket=1 view=Verified private_job=0');focus_surface(300,trace,key);palette_select(key,'ZoomToggle',palette_commands(root))
  b=len(trace());q.key('ret');wait(r'\[shell.window.action\] action=ZoomToggle frame=\d+ sid=300 ok=1',b);wait(r'\[browser.viewport\] target=15 task=15 sequence=\d+ w=\d+ h=\d+ origin=held_shell',b)
  b=len(trace());q.key('p');wait(r'\[shell.browser.input\] code=25 down=1 slot=43 accepted=1',b);wait(r'\[browser.asset.intent\] accepted=1 explicit=1 public_internet=0 format=png',b)
  rel=wait(rf'\[browser.asset.released\] ticket=(\d+) tx=\d+ rx=\d+ body_bytes={len(BODY)} body_hash={hex(asset.fingerprint(BODY))} positive_release=1 authenticated=1',b);t=rel[1]
  dec=wait(rf'\[browser.asset.decoded\] ticket={t} width=48 height=48 pixels=2304 hash={hex(asset.PIXEL_HASH)} bounded_turn=128',b);checks['decoded']=dec[0]
  ready=wait(rf'\[browser.asset.presentation.ready\] generation=\d+ mode=(Fit|Actual) viewport=\d+ ticket={t} sequence=\d+ hash={hex(asset.PIXEL_HASH)} positive_ack=1',b,60);checks['ready']=ready[0]
  shot=out/'png.ppm';result=None;d=time.monotonic()+25
  while time.monotonic()<d:
   health();q.cmd('screendump',{'filename':str(shot)})
   try:
    tile,_=doc.viewport(trace());lay=list(re.finditer(rf'\[browser.asset.presentation.layout\] generation=\d+ mode=\w+ viewport=\d+ ticket={t} sequence=\d+ x=(\d+) y=(\d+) width=48 height=48 scale=(\d+)',trace()))[-1]
    gx,gy,sc=map(int,lay.groups());width,height,pixels=ppm_rgb(shot.read_bytes(),min_width=1,min_height=1);src=asset.rgba();bad=0;n=0
    for y in range(0,48,3):
     for x in range(0,48,3):
      r,g,bb,al=src[(y*48+x)*4:(y*48+x)*4+4];want=tuple((fg*al+bg*(255-al)+127)//255 for fg,bg in zip((r,g,bb),asset.BASE))
      sx,sy=tile['x']+gx+x*sc+sc//2,tile['y']+gy+y*sc+sc//2;got=tuple(pixels[(sy*width+sx)*3:(sy*width+sx)*3+3]);n+=1;bad+=got!=want
    result=dict(ok=bad==0,checked=n,mismatched=bad,x=gx,y=gy,scale=sc)
    if result['ok']:break
   except(IndexError,ValueError,AssertionError)as exc:result=dict(ok=False,error=str(exc))
   time.sleep(.2)
  checks['pixels']=result
  if not result or not result['ok']:raise RuntimeError(f'png pixels: {result}')
 except Exception as exc:failure=repr(exc)
 finally:
  peer.close()
  if proc.poll()is None:
   proc.terminate()
   try:proc.wait(timeout=5)
   except subprocess.TimeoutExpired:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 request_ok=[r.get('request_hex')for r in peer.rows]==[REQUEST.hex()]
 res=dict(ok=failure is None and request_ok and not peer.errors,error=failure,request_exact=request_ok,png_bytes=len(BODY),png_sha256=hashlib.sha256(BODY).hexdigest(),pixel_hash=hex(asset.PIXEL_HASH),checks=checks,resolution=a.resolution,model=a.model,scope=SCOPE)
 (out/'result.json').write_text(json.dumps(res,indent=2,default=str)+'\n')
 if not res['ok']:raise RuntimeError(json.dumps(dict(error=failure,request_exact=request_ok)))
 print(json.dumps(dict(ok=True,resolution=a.resolution,png_bytes=len(BODY))))

if __name__=='__main__':main()
