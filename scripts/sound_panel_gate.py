#!/usr/bin/env python3
"""Per-program volume. Two Plays play at once, one a 440 Hz tone and one 880 Hz. The
notifications window (Super+N) shows the sound panel with both programs; choosing the
second and pressing Left ten times takes it to 50%. QEMU records the virtio sound card:
the 880 Hz tone gets quieter while the 440 Hz tone stays as loud as it was.

Needs ffmpeg on the host and Play built (scripts/build_play.sh)."""
import argparse,http.server,math,os,re,shutil,struct,subprocess,tempfile,threading,time
from pathlib import Path
from install_program import install
ROOT=Path(__file__).resolve().parents[1]

def tone(hz,seconds=30):
 d=Path(tempfile.mkdtemp());out=d/f'{hz}.mp4'
 subprocess.run(['ffmpeg','-v','error','-y','-f','lavfi','-i',f'color=c=gray:size=160x120:rate=10:duration={seconds}',
  '-f','lavfi','-i',f'sine=frequency={hz}:sample_rate=48000:duration={seconds}','-c:v','libx264','-pix_fmt','yuv420p',
  '-c:a','aac','-b:a','96k','-shortest',str(out)],check=True)
 return out.read_bytes()

def goertzel(x,hz,rate=48000):
 w=2*math.pi*hz/rate;c=2*math.cos(w);s1=s2=0.0
 for v in x:s0=v+c*s1-s2;s2,s1=s1,s0
 return math.sqrt(max(s1*s1+s2*s2-c*s1*s2,0))*2/len(x)

def levels(path,rate=48000,step=0.25):
 data=path.read_bytes();at=data.find(b'data');pcm=data[at+8:]
 left=struct.unpack(f'<{len(pcm)//2}h',pcm[:len(pcm)//2*2])[::2];n=int(rate*step)
 return [(goertzel(left[i:i+n],440),goertzel(left[i:i+n],880)) for i in range(0,len(left)-n,n)]

class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 play=os.environ.get('SEXOS_PLAY',str(ROOT/'target/play/x86_64-sex-app/release/play'))
 install(disk,[('play',Path(play).read_bytes())])
 files={'/low.mp4':tone(440),'/high.mp4':tone(880)}
 class Handler(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   body=files.get(self.path)
   if body is None:self.send_response(404);self.end_headers();return
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*a):pass
 server=Quiet(('127.0.0.1',0),Handler);port=server.server_address[1]
 threading.Thread(target=server.serve_forever,daemon=True).start()
 wav=out/'sound.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 from quil_autosave_gate import boot
 g=boot(a.iso,disk,out,'panel',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def say(text,pattern,timeout=180):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def fetched(name):
  m=say(f'fetch http://10.0.2.2:{port}/{name} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',600);assert m[1].startswith('Saved'),m[1]
 try:
  g['wait'](r'\[bell.sound.ready\] card=\S+ stream=\d+ rate=48000 channels=2',0,120)
  g['wait'](r'\[app.private.retired\] target=41 ',0,300);time.sleep(2)
  fetched('low.mp4');fetched('high.mp4')
  s=len(t());say('run play @low.mp4',r'\[play.opened\] ',300)
  low=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  s=len(t());say('run play @high.mp4',r'\[play.opened\] ',300)
  high=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  g['wait'](r'\[bell.sound.stream.open\] row=\d+ caller='+high[1],s,60)
  time.sleep(2)
  # The sound panel lists both programs.
  # From a desktop window: Collar keeps a program from opening the notifications.
  g['focus'](153);time.sleep(.5)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'meta_l'},{'type':'qcode','data':'n'}],'hold-time':80})
  panel=g['wait'](rf'\[shell.sound.panel\] SOUND  master 100% \| > Program {low[1]}  100% playing \|   Program {high[1]}  100% playing',s,60)
  q.key('down');time.sleep(.3)
  for n in range(10):
   ss=len(t());q.key('left');g['wait'](rf'\[shell.sound.panel.set\] row=\d+ volume={95-5*n} muted=0 status=0x0',ss,30);time.sleep(.1)
  g['wait'](rf'\[shell.sound.panel\] SOUND  master 100% \| > Program {low[1]}  100% playing \|   Program {high[1]}  50% playing|\[shell.sound.panel\] SOUND  master 100% \|   Program {low[1]}  100% playing \| > Program {high[1]}  50% playing',s,30)
  time.sleep(3)
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  q.cmd('quit');g['proc'].wait(30)
  lv=[(l,h) for l,h in levels(wav) if l>500]
  top=max(h for _,h in lv)
  loud=[(l,h) for l,h in lv if h>0.8*top];quiet=[(l,h) for l,h in lv if 0.15*top<h<0.65*top]
  assert len(loud)>=4 and len(quiet)>=4,f'880 Hz did not step down: top={top:.0f} loud={len(loud)} quiet={len(quiet)}'
  l_loud=sorted(l for l,_ in loud)[len(loud)//2];l_quiet=sorted(l for l,_ in quiet)[len(quiet)//2]
  h_quiet=sorted(h for _,h in quiet)[len(quiet)//2];h_loud=sorted(h for _,h in loud)[len(loud)//2]
  assert abs(l_quiet/l_loud-1)<0.15,f'440 Hz changed: {l_loud:.0f} -> {l_quiet:.0f}'
  print({'ok':True,'440_before':round(l_loud),'440_after':round(l_quiet),'880_before':round(h_loud),'880_after':round(h_quiet)})
 finally:g['stop']()
if __name__=='__main__':main()
