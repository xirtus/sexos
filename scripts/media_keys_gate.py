#!/usr/bin/env python3
"""Media keys and the volume OSD. Play plays a video with sound while Spindle has the
focus; the keyboard's play/pause key still reaches Play (Shell asks Bell which program
plays, and hands the key to it), pausing and then playing it again; the stop key
pauses it at the start. A volume key shows the new volume in the panel for a moment
(the OSD), which then goes away.

Build Play first: scripts/build_play.sh."""
import argparse,http.server,os,shutil,threading,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]
CLIP=(ROOT/'apps/play/tests/clip-long.mp4').read_bytes()
FILES={'/clip.mp4':CLIP}
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=FILES.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('play',(ROOT/'iso_root/programs/play').read_bytes())])
 server=Quiet(('127.0.0.1',0),Handler);port=server.server_address[1]
 threading.Thread(target=server.serve_forever,daemon=True).start()
 wav=out/'sound.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'mediakeys',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def say(text,pattern,timeout=180):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def press(key,pattern,timeout=30):
  s=len(t());q.key(key);return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[bell.sound.ready\] card=\S+ stream=\d+ rate=48000 channels=2',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  m=say(f'fetch http://10.0.2.2:{port}/clip.mp4 clip.mp4',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',600);assert m[1].startswith('Saved'),m[1]
  s=len(t());say('run play @clip.mp4',r'\[play.opened\] format=\S+ codec=h264 ',300)
  launch=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[bell.sound.stream.open\] row=\d+ caller='+launch[1],s,60)
  # Spindle has the focus, not Play: the key still reaches the program that plays.
  g['focus'](153);time.sleep(.5)
  s=len(t());q.key('audioplay')
  r=g['wait'](r'\[shell.media.route\] code=0x78 pd=(\d+) sent=(\d)',s,30)
  assert r[1]==launch[1] and r[2]=='1',r[0]
  g['wait'](r'\[play.paused\] 1',s,30)
  press('audioplay',r'\[play.paused\] 0')
  press('audiostop',r'\[play.stopped\] paused=1')
  # Volume OSD: shown in the panel, then gone.
  s=len(t());q.key('volumedown')
  osd=g['wait'](r'\[shell.osd\] text=(Volume 95%  \|{19}\.) shown=1',s,30)
  g['wait'](r'\[shell.notify.hide\]',s,30)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'osd':osd[1],'player_pd':int(r[1])})
 finally:g['stop']()
if __name__=='__main__':main()
