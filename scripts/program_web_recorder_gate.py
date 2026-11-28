#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Kaleidoscope records with MediaRecorder (the browser's own VP8/VP9 + Opus WebM
encoder, sex-media's live recorder) and plays the recording back from its Blob.

Started with --fake-media (a moving test pattern camera and a test tone microphone),
the browser opens recorder.html; the gate clicks Allow on the browser's card. The page
records three seconds of the camera and microphone (VP8 + Opus, a piece a second),
then three seconds of an animated canvas (canvas.captureStream) with the microphone
(VP9 + Opus). Each recording is POSTed to the gate, which checks it with FFmpeg
(codecs, picture size, pictures, sound, about three seconds), and is played back in a
<video> through URL.createObjectURL: it must play to its end with pictures.

Needs the export FFmpeg profile in the image (FFMPEG_RECORDING=2: libvpx VP8/VP9,
Opus, WebM), as the CI recording job builds it, and ffprobe on the host."""
import argparse,http.server,json,os,re,shutil,subprocess,threading,time
from pathlib import Path
from PIL import Image
from browser_gate_viewport import live_viewport
from web_surface_geometry import canvas_box
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
UPLOADS={}
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  path=self.path.split('?')[0]
  f=SITE/path.removeprefix('/site/') if path.startswith('/site/') else None
  if f is None or not f.is_file():self.send_response(404);self.end_headers();return
  body=f.read_bytes();self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def do_POST(self):
  n=int(self.headers.get('Content-Length') or 0);body=self.rfile.read(n)
  name=self.path.removeprefix('/upload/')
  if not re.fullmatch(r'[a-z]+',name) or n>64<<20:self.send_response(400);self.end_headers();return
  UPLOADS[name]=body;self.send_response(200);self.send_header('Content-Length','2');self.end_headers();self.wfile.write(b'ok')
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
PORT=server.server_address[1]
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','?':'slash','=':'equal'}

def probe(path,video,size):
 """FFmpeg reads the recording: its codecs, size, pictures and sound."""
 info=json.loads(subprocess.run(['ffprobe','-v','error','-count_frames','-show_entries','stream=codec_name,codec_type,width,height,nb_read_frames:format=format_name',
  '-of','json',str(path)],check=True,capture_output=True,text=True).stdout)
 streams={s['codec_type']:s for s in info['streams']}
 assert 'webm' in info['format']['format_name'],info['format']
 v,a=streams.get('video'),streams.get('audio')
 assert v and v['codec_name']==video and (v['width'],v['height'])==size,('picture',v)
 assert a and a['codec_name']=='opus',('sound',a)
 frames=int(v['nb_read_frames']);assert frames>=5,('pictures',frames)
 # The last packet's time: about three seconds (no duration in a live WebM).
 last=subprocess.run(['ffprobe','-v','error','-select_streams','v','-show_entries','packet=pts_time','-of','csv=p=0',str(path)],check=True,capture_output=True,text=True).stdout.split()
 end=max(float(t) for t in last if t not in ('','N/A'))
 assert 2.0<=end<=3.6,('last picture at',end)
 # The test tone is in the sound.
 pcm=subprocess.run(['ffmpeg','-v','error','-i',str(path),'-vn','-f','s16le','-ac','1','-ar','8000','-'],check=True,capture_output=True).stdout
 loud=sum(1 for i in range(0,len(pcm)-1,2) if abs(int.from_bytes(pcm[i:i+2],'little',signed=True))>1500)
 assert len(pcm)>=2*8000*2 and loud>len(pcm)//20,('sound',len(pcm),loud)
 return {'pictures':frames,'last':end,'sound_s':len(pcm)/16000}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 rw,rh=map(int,a.resolution.split('x'))
 base=f'http://10.0.2.2:{PORT}'
 url=f'{base}/site/recorder.html'
 g=boot(a.iso,disk,a.out,'webrecorder',a.resolution);t=g['text'];q=g['q']
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
 report={}
 try:
  wait(r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=wait(r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url} --fake-media');q.key('ret')
  wait(r'\[web.capture.ready\] fake=1 ',s,300)
  types=wait(r'\[web.console\] level=0 recorder types (\d+)',s,300)[1]
  assert types=='1110',('MediaRecorder types (vp8 vp9 opus mp4); is the export FFmpeg profile built?',types)
  wait(rf'\[web.capture.prompt\] origin=http://10\.0\.2\.2:{PORT} audio=1 video=1',s,60)
  wait(r'\[web.capture.card\] x=(\d+)',s,60)
  m=wait(r'\[web.ready\] target=(\d+) task=(\d+)',s,120);target,task=m[1],m[2]
  launch=wait(rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60);surface=int(launch[1])
  time.sleep(2)
  tile,pw,ph=live_viewport(g,ROOT,surface)
  ox,oy,_,_=canvas_box(ROOT,tile,pw,ph)
  # The card may have been laid out for an earlier width: look again after the resize.
  cards=re.findall(r'\[web.capture.card\] x=(\d+) y=(\d+) w=(\d+) h=(\d+) allow=(\d+),(\d+) block=(\d+),(\d+)',t()[s:])
  ax,ay=map(int,cards[-1][4:6])
  s=len(t());click(ox+ax,oy+ay)
  wait(r'\[web.capture.answer\] origin=\S+ button=0',s,30)
  assert wait(r'\[web.console\] level=0 recorder stream (\S+)',s,60)[1]=='audio,video'
  # The camera and microphone, VP8 + Opus, a piece a second.
  st=wait(r'\[web.recorder.start\] id=1 mime=(\S+) w=(\d+) h=(\d+)',s,60)
  assert (st[1],st[2],st[3])==('video/webm;codecs=vp8,opus','640','480'),st[0]
  wait(r'\[web.console\] level=0 recorder cam started recording video/webm;codecs=vp8,opus',s,30)
  shot('recording-camera')
  stop=wait(r'\[web.recorder.stop\] id=1 ms=(\d+) pictures=(\d+) skipped=(\d+) samples=(\d+) bytes=(\d+) encode_ms=(\d+)',s,120)
  report['cam_stop']=stop[0]
  assert int(stop[2])>=5 and int(stop[4])>=48000*2,('camera recording',stop[0])
  pieces=len(re.findall(r'\[web.recorder.piece\] id=1 ',t()[s:]));assert pieces>=2,('pieces',pieces)
  blob=wait(r'\[web.console\] level=0 recorder cam blob (\d+) (\S+)',s,60)
  assert int(blob[1])>2000 and blob[2]=='video/webm;codecs=vp8,opus',blob[0]
  assert wait(r'\[web.console\] level=0 recorder cam uploaded (\d+)',s,60)[1]=='200'
  (a.out/'cam.webm').write_bytes(UPLOADS['cam']);assert len(UPLOADS['cam'])==int(blob[1])
  report['cam']=probe(a.out/'cam.webm','vp8',(640,480))
  # The Blob plays back in a video element.
  wait(r'\[web.media.blob\] node=\d+ bytes=\d+',s,60)
  wait(r'\[web.console\] level=0 recorder cam playing 640 480',s,60)
  shot('playing-camera')
  pl=wait(r'\[web.console\] level=0 recorder cam played (\d+) (\S+) (\S+) (\d+) (\d+)',s,120)
  assert int(pl[1])>=5 and 2.0<=float(pl[2])<=3.8,('camera playback',pl[0])
  report['cam_played']=pl[0]
  # An animated canvas and the microphone, VP9 + Opus.
  st=wait(r'\[web.recorder.start\] id=2 mime=(\S+) w=(\d+) h=(\d+)',s,60)
  assert (st[1],st[2],st[3])==('video/webm;codecs=vp9,opus','320','240'),st[0]
  stop=wait(r'\[web.recorder.stop\] id=2 ms=(\d+) pictures=(\d+) skipped=(\d+) samples=(\d+) bytes=(\d+) encode_ms=(\d+)',s,120)
  report['canvas_stop']=stop[0]
  assert int(stop[2])>=5,('canvas recording',stop[0])
  blob=wait(r'\[web.console\] level=0 recorder canvas blob (\d+) (\S+)',s,60)
  assert blob[2]=='video/webm;codecs=vp9,opus',blob[0]
  assert wait(r'\[web.console\] level=0 recorder canvas uploaded (\d+)',s,60)[1]=='200'
  (a.out/'canvas.webm').write_bytes(UPLOADS['canvas'])
  report['canvas']=probe(a.out/'canvas.webm','vp9',(320,240))
  # The canvas's pictures were recorded, not the camera's: mostly its blue.
  png=a.out/'canvas-frame.png'
  subprocess.run(['ffmpeg','-v','error','-y','-ss','1','-i',str(a.out/'canvas.webm'),'-frames:v','1',str(png)],check=True)
  px=Image.open(png).convert('RGB').getpixel((10,200));assert px[2]>100 and px[0]<80,('canvas picture',px)
  pl=wait(r'\[web.console\] level=0 recorder canvas played (\d+) (\S+) (\S+) (\d+) (\d+)',s,120)
  assert int(pl[1])>=5 and 2.0<=float(pl[2])<=3.8 and pl[4:6]==('320','240'),('canvas playback',pl[0])
  report['canvas_played']=pl[0]
  wait(r'\[web.console\] level=0 recorder done',s,60)
  wait(r'\[web.capture.camera\] on=0',s,30)
  s=len(t());g['focus'](surface);time.sleep(.3);q.key('f11');wait(rf'\[web.closed\] target={target} task={task}',s,60)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  report['ok']=True
  print(report)
 finally:
  (a.out/'report.json').write_text(json.dumps(report,indent=1))
  g['stop']()
if __name__=='__main__':main()
