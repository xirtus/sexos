#!/usr/bin/env python3
"""Kaleidoscope's RTCPeerConnection: one page with two peer connections calling each
other over the machine's own UDP (host candidates on 10.0.2.15, looped back by sexnet).

Started with --fake-media, the browser opens webrtc.html; the gate allows the camera
and microphone in the browser's card. pc1 sends the fake camera (VP8, Kaleidoscope's
own encoder, at half size) and the test tone (G.711); pc2's remote video track shows in
a <video>. The gate checks the offer's SDP, both connections reach `connected`, getStats
bytes rise, the remote frames are decoded, and the remote <video>'s pixels show the
moving test pattern (two screenshots differ, and the picture is not flat)."""
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
server=Quiet(('127.0.0.1',0),Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
PORT=server.server_address[1]
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','?':'slash','=':'equal'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 rw,rh=map(int,a.resolution.split('x'))
 url=f'http://10.0.2.2:{PORT}/site/webrtc.html'
 g=boot(a.iso,disk,a.out,'webrtc',a.resolution);t=g['text'];q=g['q']
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
  wait(r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch http://10.0.2.2:{PORT}/web /bin/kaleidoscope');q.key('ret')
  m=wait(r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url} --fake-media');q.key('ret')
  wait(r'\[web.capture.ready\] fake=1 ',s,300)
  wait(r'\[web.console\] level=0 webrtc asked function',s,300)
  wait(rf'\[web.capture.prompt\] origin=http://10\.0\.2\.2:{PORT} audio=1 video=1',s,60)
  m=wait(r'\[web.ready\] target=(\d+) task=(\d+)',s,120);target,task=m[1],m[2]
  launch=wait(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(2)
  tile,pw,ph=live_viewport(g,ROOT,surface)
  ox,oy,_,_=canvas_box(ROOT,tile,pw,ph)
  cards=re.findall(r'\[web.capture.card\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) allow=(\d+),(\d+) block=(\d+),(\d+)',t()[s:])
  cx,cy,cw,ch,ax,ay,bx,by=map(int,cards[-1])
  s=len(t());click(ox+ax,oy+ay)
  wait(r'\[web.capture.answer\] origin=\S+ button=0',s,30)
  offer=wait(r'\[web.console\] level=0 webrtc offer (.+)',s,120)[1].split()
  assert offer==['true']*5,('offer SDP lacks fingerprint, ice-ufrag, BUNDLE, VP8 or opus',offer)
  assert wait(r'\[web.console\] level=0 webrtc negotiated (\S+ \S+)',s,60)[1]=='stable stable'
  wait(r'\[web.rtc.socket\] peer=\d+ ok=1 port=\d+ hosts=[1-9]',s,60)
  wait(r'\[web.console\] level=0 webrtc candidate host udp',s,60)
  wait(r'\[web.console\] level=0 webrtc pc1 connection connected',s,120)
  wait(r'\[web.console\] level=0 webrtc pc2 connection connected',s,60)
  wait(r'\[web.console\] level=0 webrtc track video 1',s,10)
  wait(r'\[web.rtc.frame\] key=rtc:\d+:1 n=1 w=320 h=240 key_frame=1',s,120)
  wait(r'\[web.console\] level=0 webrtc remote unmuted',s,60)
  wait(r'\[web.console\] level=0 webrtc remote playing',s,60)
  wait(r'\[web.rtc.sound\] key=rtc:\d+:0 packets=1',s,60)
  # getStats: bytes sent rise, and pc2 decodes what pc1 encodes.
  stats=[]
  for _ in range(3):
   m=wait(r'\[web.console\] level=0 webrtc stats (\d+) (\d+) (\d+) (\d+) (\w+)',len(t()),30);stats.append(tuple(map(int,m.groups()[:4]))+(m[5],))
  assert stats[-1][0]>stats[0][0] and stats[-1][4]=='rising',('bytes sent do not rise',stats)
  assert stats[-1][2]>stats[0][2] and stats[-1][3]>stats[0][3]>0,('remote frames not decoded',stats)
  rect=wait(r'\[web.console\] level=0 webrtc rect (-?\d+) (-?\d+) (\d+) (\d+)',s,60);vx,vy,vw,vh=map(int,rect.group(1,2,3,4))
  # The remote video shows the fake camera's moving picture.
  def video_pixels(img):return [img.getpixel((ox+vx+x,oy+BAR+vy+y)) for y in range(8,vh-8,12) for x in range(8,vw-8,12)]
  one=video_pixels(shot('webrtc-remote-1'));time.sleep(1.5);two=video_pixels(shot('webrtc-remote-2'))
  assert len(set(one))>=4,('remote video shows no picture',len(set(one)))
  assert sum(p!=q for p,q in zip(one,two))>=8,'remote picture does not move'
  assert all(max(p)>16 for p in one[:4]) or len(set(one))>8,'remote video is black'
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');wait(rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'stats':stats[-1],'video':(vx,vy,vw,vh)})
 finally:g['stop']()
if __name__=='__main__':main()
