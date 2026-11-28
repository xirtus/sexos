#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Kaleidoscope runs pages' Web Audio graphs (sex-webaudio) and plays them.

Started with --fake-media (a 440 Hz test tone microphone), the browser opens
webaudio.html, which asks for the microphone. The gate clicks Allow on the browser's
card; the page feeds the microphone into an AnalyserNode and reports the loudest bin
(440 Hz is bin 19 of a 2048 point FFT at 48 kHz), and plays an oscillator through the
destination for three seconds, which the browser renders into the program's sound
ring (the gate checks frames were heard and the ring was held)."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from PIL import Image
from browser_gate_viewport import live_viewport
from web_surface_geometry import canvas_box
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
BAR=30
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  path=self.path.split('?')[0]
  f=SITE/path.removeprefix('/site/') if path.startswith('/site/') else None
  if f is None or not f.is_file():self.send_response(404);self.end_headers();return
  body=f.read_bytes();self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
servers=[Quiet(('127.0.0.1',0),Handler)]
for sv in servers:threading.Thread(target=sv.serve_forever,daemon=True).start()
PORT=servers[0].server_address[1]
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','?':'slash','=':'equal'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 rw,rh=map(int,a.resolution.split('x'))
 base=f'http://10.0.2.2:{PORT}'
 url=f'{base}/site/webaudio.html'
 g=boot(a.iso,disk,a.out,'webaudio',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in ':+?':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':{':':'semicolon','+':'equal','?':'slash'}[k]}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def move(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.3)
 def click(x,y):
  move(x,y)
  for down in (True,False):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.2)
 def shot(name):
  path=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(path)});time.sleep(.5)
  img=Image.open(path).convert('RGB');img.save(a.out/f'{name}.png');return img
 def wait(pattern,s,timeout=60):return g['wait'](pattern,s,timeout)
 try:
  wait(r'\[app.private.retired\] target=41 ',0,120);wait(r'\[bell.sound.ready\]',0,180);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=wait(r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url} --fake-media');q.key('ret')
  wait(r'\[web.capture.ready\] fake=1 ',s,300)
  api=wait(r'\[web.console\] level=0 webaudio api (.+)',s,300);assert api[1].strip()=='function function function',api[0]
  wait(rf'\[web.capture.prompt\] origin=http://10\.0\.2\.2:{PORT} audio=1 video=0',s,60)
  wait(r'\[web.capture.card\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) allow=(\d+),(\d+) block=(\d+),(\d+)',s,60)
  m=wait(r'\[web.ready\] target=(\d+) task=(\d+)',s,120);target,task=m[1],m[2]
  launch=wait(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(2)
  # Nothing is heard from the microphone before the person answers.
  assert '[web.capture.mic] on=1' not in t()[s:],'microphone started before an answer'
  tile,pw,ph=live_viewport(g,ROOT,surface)
  ox,oy,_,_=canvas_box(ROOT,tile,pw,ph)
  cards=re.findall(r'\[web.capture.card\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) allow=(\d+),(\d+) block=(\d+),(\d+)',t()[s:])
  cx,cy,cw,ch,ax,ay,bx,by=map(int,cards[-1])
  shot('webaudio-prompt')
  s=len(t());click(ox+ax,oy+ay)
  wait(r'\[web.capture.answer\] origin=\S+ button=0',s,30)
  wait(r'\[web.capture.mic\] on=1 source=fake',s,30)
  assert wait(r'\[web.console\] level=0 webaudio state (\S+ \d+)',s,60)[1]=='running 48000'
  assert wait(r'\[web.console\] level=0 webaudio stream (.+)',s,30)[1].strip()=='1 audio'
  wait(r'\[web.audio.running\] contexts=\d+',s,30)
  wait(r'\[web.audio.ring\] held=1',s,30)
  peak=wait(r'\[web.console\] level=0 webaudio peak (\d+) (\d+) (\d+) (\w+) (\w+)',s,60)
  b,hz,swing=map(int,peak[1:4])
  assert b in (18,19) and abs(hz-440)<=30,('analyser missed the 440 Hz tone',peak[0])
  assert 20<=swing<=40 and peak[4:6]==('true','true'),('analyser level or clock',peak[0])
  shot('webaudio-heard')
  assert wait(r'\[web.console\] level=0 webaudio ended (\w+)',s,60)[1]=='true'
  outs=[tuple(map(int,o)) for o in re.findall(r'\[web.audio.out\] frames=(\d+) heard=(\d+) peak=(\d+) ring=(\d)',t()[s:])]
  if not outs:outs=[tuple(map(int,wait(r'\[web.audio.out\] frames=(\d+) heard=(\d+) peak=(\d+) ring=(\d)',s,30)[1:5]))]
  frames,heard=max(o[0] for o in outs),max(o[1] for o in outs);loud=max(o[2] for o in outs)
  # Three seconds of a 0.2 tone (6553 of 32768), faded in over the first half second.
  assert heard>=48000*2 and 4000<=loud<=8000,('oscillator not heard',outs)
  assert any(o[3]==1 for o in outs),('sound ring not held',outs)
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');wait(rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'bin':b,'hz':hz,'swing':swing,'frames':frames,'heard':heard,'peak':loud})
 finally:g['stop']()
if __name__=='__main__':main()
