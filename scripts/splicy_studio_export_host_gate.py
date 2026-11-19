#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Splicy Studio's three export formats, from Splicy's own pure-Rust encoders, checked by FFmpeg.

Builds Splicy Studio for the host (profile `shot`), imports a 2 s VP9/Opus clip, and exports it
with `--export=` as WebM (VP9 + Opus), MP4 (AV1 + Opus) and MP4 (H.264 + AAC): the same engine
command the Export panel runs. FFmpeg must find the right codecs, decode every file without an
error, count every picture, see the source's picture (PSNR) and hear its sound (loudness).
Then `--clean-up` (Remove Restarts, Remove Filler Words) must cut a restart out of the sequence.
Usage: splicy_studio_export_host_gate.py OUT_DIR"""
import json,re,subprocess,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
STUDIO=ROOT/'apps/splicy-studio'
SECONDS,FPS=2,30
FORMATS=[('webm','talk.webm',('vp9','opus'),'matroska,webm'),('av1','talk-av1.mp4',('av1','opus'),'mov,mp4'),('h264','talk-h264.mp4',('h264','aac'),'mov,mp4')]
def run(*a,**k):return subprocess.run(a,check=True,capture_output=True,text=True,**k)
def loudness(path):
 m=re.search(r'mean_volume: (-?[\d.]+) dB',subprocess.run(['ffmpeg','-hide_banner','-i',str(path),'-af','volumedetect','-f','null','-'],capture_output=True,text=True).stderr)
 assert m,f'{path.name}: no sound'
 return float(m[1])
def psnr(a,b):
 m=re.search(r'average:([\d.]+|inf)',subprocess.run(['ffmpeg','-hide_banner','-i',str(a),'-i',str(b),'-lavfi','[0:v][1:v]psnr','-f','null','-'],capture_output=True,text=True).stderr)
 assert m,f'no PSNR between {a.name} and {b.name}'
 return float(m[1])
def main():
 out=Path(sys.argv[1]);out.mkdir(parents=True,exist_ok=True)
 # stable lock entries for the SexOS crates Splicy uses by path, as build_splicy_studio.sh does
 subprocess.run(['cargo','update','--workspace','--quiet'],cwd=STUDIO,check=True)
 subprocess.run(['cargo','build','--locked','--profile','shot'],cwd=STUDIO,check=True)
 src=out/'source.webm'
 run('ffmpeg','-v','error','-y','-f','lavfi','-i',f'testsrc2=size=320x180:rate={FPS}:duration={SECONDS + 1}',
  '-f','lavfi','-i',f'sine=frequency=440:sample_rate=48000:duration={SECONDS + 1}','-af',"volume='0.1+0.9*mod(t,1)':eval=frame",
  '-vf','format=yuv420p','-colorspace','bt709','-color_primaries','bt709','-color_trc','bt709','-color_range','tv',
  '-c:v','libvpx-vp9','-b:v','600k','-c:a','libopus','-ac','2',str(src))
 args=[str(STUDIO/'target/shot/splicy-studio'),src.name]+[f'--export={f}:{name}:{SECONDS}' for f,name,_,_ in FORMATS]+['studio.png','320','200']
 log=run(*args,cwd=out).stdout
 reference=out/'reference.mp4'
 run('ffmpeg','-v','error','-y','-i',str(src),'-t',str(SECONDS),'-c:v','libx264','-qp','0','-c:a','aac',str(reference))
 heard=loudness(reference);report={};failed={}
 for fmt,name,codecs,container in FORMATS:
  try:report[fmt]=check(fmt,name,codecs,container,log,out,reference,heard)
  except AssertionError as e:
   failed[fmt]=repr(e)
   diagnose(out/name)
 # The Text panel's Remove Restarts (after Scarlet): the first "Keep the," goes, 0.9 s, so the
 # second saying plays; the whole sequence then exports 0.9 s shorter.
 (out/'restart.words').write_text('0 300 Keep\n350 600 the,\n900 1100 keep\n1150 1350 the\n1400 1700 ending\n2000 2500 here.\n')
 clean=run(str(STUDIO/'target/shot/splicy-studio'),src.name,'restart.words','--clean-up','--export=h264:clean.mp4:all','clean.png','320','200',cwd=out).stdout
 try:
  assert re.search(r'\[splicy.studio.cleanup\] restarts=1 ticks=[1-9]',clean),clean
  assert '[splicy.studio.cleanup] found_restarts=1' in clean,clean
  took=float(run('ffprobe','-v','error','-show_entries','format=duration','-of','csv=p=0',str(out/'clean.mp4')).stdout)
  assert abs(took-(SECONDS+1-0.9))<0.05,('clean-up duration',took)
  report['clean_up']={'seconds':took}
 except AssertionError as e:failed['clean_up']=repr(e)
 print(json.dumps({'ok':not failed,'ffmpeg':run('ffmpeg','-version').stdout.splitlines()[0],'source_mean_db':heard,'exports':report,'failed':failed}))
 if failed:sys.exit(1)
def diagnose(path):
 # What FFmpeg was given: the codec setup bytes, every audio packet's first and last bytes, and
 # the packet the error follows (-debug_ts names each packet as it is read).
 say=lambda *a:print(*a,file=sys.stderr)
 streams=json.loads(subprocess.run(['ffprobe','-v','error','-show_streams','-show_data','-of','json',str(path)],capture_output=True,text=True).stdout or '{}')
 for st in streams.get('streams',[]):
  say('stream',st.get('codec_name'),st.get('channels'),'extradata',re.sub(r'[^0-9a-f]','',''.join(l[9:49] for l in st.get('extradata','').splitlines())))
 pk=json.loads(subprocess.run(['ffprobe','-v','error','-select_streams','a','-show_packets','-show_data','-of','json',str(path)],capture_output=True,text=True).stdout or '{}')
 for n,p in enumerate(pk.get('packets',[])):
  h=re.sub(r'[^0-9a-f]','',''.join(l[9:49] for l in p.get('data','').splitlines()))
  say(f"pkt {n} pts={p.get('pts')} size={p.get('size')} head={h[:16]} tail={h[-8:]}")
 dbg=subprocess.run(['ffmpeg','-hide_banner','-v','verbose','-debug_ts','-xerror','-i',str(path),'-map','0:a','-f','null','-'],capture_output=True,text=True).stderr.splitlines()
 for k,l in enumerate(dbg):
  if 'Error parsing' in l or 'rror' in l and 'opus' in l:
   say('\n'.join(dbg[max(0,k-6):k+2]));break
def check(fmt,name,codecs,container,log,out,reference,heard):
 if True:
  m=re.search(rf'\[splicy.studio.export\] format={fmt} file={re.escape(name)} bytes=(\d+) ms=(\d+)',log)
  assert m,f'{fmt}: no export line in\n{log}'
  path=out/name;assert path.stat().st_size==int(m[1])>1000,(fmt,m[1])
  probe=json.loads(run('ffprobe','-v','error','-count_frames','-show_entries','stream=codec_name,codec_type,width,height,nb_read_frames,sample_rate:format=format_name','-of','json',str(path)).stdout)
  streams={s['codec_type']:s for s in probe['streams']}
  assert probe['format']['format_name'].startswith(container),(fmt,probe['format'])
  assert (streams['video']['codec_name'],streams['audio']['codec_name'])==codecs,(fmt,streams)
  assert (streams['video']['width'],streams['video']['height'])==(320,180),(fmt,streams['video'])
  pictures=int(streams['video']['nb_read_frames']);assert pictures==SECONDS*FPS,(fmt,pictures)
  assert streams['audio']['sample_rate']=='48000',(fmt,streams['audio'])
  errors=subprocess.run(['ffmpeg','-v','error','-xerror','-i',str(path),'-f','null','-'],capture_output=True,text=True)
  assert errors.returncode==0 and not errors.stderr.strip(),(fmt,errors.stderr[-600:])
  quality=psnr(path,reference);assert quality>30,(fmt,'PSNR',quality)
  level=loudness(path);assert abs(level-heard)<1.5,(fmt,'loudness',level,heard)
  return {'bytes':int(m[1]),'ms':int(m[2]),'pictures':pictures,'psnr':round(quality,1),'mean_db':level}
if __name__=='__main__':main()
