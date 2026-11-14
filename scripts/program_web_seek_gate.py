#!/usr/bin/env python3
"""Web seeks ahead in a video still downloading. The test site sends the 10 s MP4 at
100 KB/s and takes Range requests; a second after the video plays, the page jumps to
8 s. Web must ask for a piece from there (`[web.media.range]`) rather than wait for
the whole download, and play on from 8 s within a few seconds.

Needs the host's ffmpeg to make the clips, FFmpeg built for programs, then the ISO."""
import argparse,os,re,shutil,sys,threading,time
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
import program_web_stream_gate as sg
from quil_autosave_gate import boot

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);sg.clips(a.out)
 sg.Handler.rate=100*1024
 server=sg.Quiet(('127.0.0.1',0),sg.Handler);port=server.server_address[1]
 threading.Thread(target=server.serve_forever,daemon=True).start()
 disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 wav=a.out/'web-seek.wav';wav.unlink(missing_ok=True)
 os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
 base=f'http://10.0.2.2:{port}';url=f'{base}/site/seek.html'
 g=boot(a.iso,disk,a.out,'webseek',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(sg.KEYS.get(k,k))
   time.sleep(.06)
 console=lambda pat,s,timeout=300:g['wait'](r'\[web.console\] level=0 '+pat,s,timeout)
 total=len(sg.CLIPS['stream.mp4'])
 try:
  g['wait'](r'\[bell.sound.ready\]',0,180)
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,600);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {url}');q.key('ret')
  node=g['wait'](rf'\[web.media.stream\] node=(\d+) total={total} ',s,600)[1]
  console('playing',s,300)
  console('seek to 8',s,60)
  rng=g['wait'](rf'\[web.media.range\] node={node} from=(\d+) ',s,60)
  start=int(rng[1]);assert total//2<start<total,f'piece from {start} of {total}'
  moved=console(r'moved after (\d+)',s,60)
  ms=int(moved[1])
  # The whole clip takes about 14 s at this rate; from 8 s on it is under 3 s.
  assert ms<6000,f'8 s showed only after {ms} ms'
  before=t()[s:t().index(moved[0],s)]
  assert not re.search(rf'\[web.media.load\] node={node} status=200 bytes={total} ',before),'the whole download came first'
  console(r'ended at (\S+)',s,60)
  ended=g['wait'](rf'\[web.media.ended\] node={node} frames=(\d+) sound_frames=\d+ played=\d+ err=(-?\d+)',s,60)
  assert ended[2]=='0' and int(ended[1])>=40,ended[0]
  assert 'web.media.range.refused' not in t()[s:] and 'media.decode.failed' not in t()[s:]
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t()
  print({'ok':True,'total':total,'range_from':start,'moved_ms':ms,'frames':int(ended[1])})
 finally:g['stop']()
if __name__=='__main__':main()
