#!/usr/bin/env python3
"""Media apps use AVX: mediabench (apps/mediabench, FFmpeg + dav1d + x264 + libvpx +
libaom through sex-media) decodes H.264, HEVC, VP9 and AV1 clips and encodes synthetic
pictures twice on a booted system, once with SSE only and once with every SIMD level
the CPU and kernel allow. It passes when the full run has AVX2 and FMA3 (the guest CPU
has them), the SSE run has no AVX, and both runs decode every clip to the same
pictures (bit-exact decoders). It prints the decode and encode speed-up per codec;
with --accel kvm (or auto on a runner with /dev/kvm) those are real-CPU numbers.

Build first: scripts/build_mediabench.sh (the export profile, FFMPEG_RECORDING=2,
adds the video encoders). Needs the host's ffmpeg to make the clips."""
import argparse,http.server,json,os,re,shutil,subprocess,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
PROGRAM=Path(os.environ.get('SEXOS_MEDIABENCH',str(ROOT/'target/mediabench/x86_64-sex-app/release/mediabench')))
CLIPS={'h264.mp4':['-c:v','libx264','-preset','medium','-crf','23'],
       'hevc.mp4':['-c:v','libx265','-preset','fast','-crf','26','-x265-params','log-level=error'],
       'vp9.webm':['-c:v','libvpx-vp9','-b:v','2M','-deadline','realtime','-cpu-used','8'],
       'av1.mp4':['-c:v','libaom-av1','-usage','realtime','-cpu-used','8','-crf','35']}
FILES={}
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=FILES.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
def kvm_usable():return os.access('/dev/kvm',os.R_OK|os.W_OK)
def make_clips(out,size,frames):
 made={}
 for name,codec in CLIPS.items():
  path=out/name
  r=subprocess.run(['ffmpeg','-hide_banner','-loglevel','error','-y','-f','lavfi','-i',f'testsrc2=size={size}:rate=30','-frames:v',str(frames),'-pix_fmt','yuv420p',*codec,str(path)],capture_output=True,text=True)
  if r.returncode==0 and path.exists():made[name]=path.read_bytes()
  else:print(f'[mediabench.gate] host ffmpeg cannot make {name}: {r.stderr.strip()[:200]}')
 assert 'h264.mp4' in made,'the host ffmpeg made no H.264 clip'
 return made
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800')
 p.add_argument('--accel',choices=('tcg','kvm','auto'),default='auto',help='kvm: real-CPU timings with -cpu host');p.add_argument('--frames',type=int);p.add_argument('--size');a=p.parse_args()
 accel=a.accel if a.accel!='auto' else ('kvm' if kvm_usable() else 'tcg')
 if accel=='kvm':
  assert kvm_usable(),'--accel kvm needs a readable and writable /dev/kvm'
  os.environ['SEXOS_GATE_ACCEL']='kvm';os.environ['SEXOS_GATE_CPU']='host'
 # TCG is slow: small pictures. KVM: 720p, as people watch.
 size=a.size or ('1280x720' if accel=='kvm' else '320x240');frames=a.frames or (60 if accel=='kvm' else 12)
 os.environ.setdefault('SEXOS_GATE_MEMORY','1G')
 a.out.mkdir(parents=True,exist_ok=True)
 assert PROGRAM.read_bytes()[:4]==b'\x7fELF',f'build {PROGRAM} first (scripts/build_mediabench.sh)'
 clips=make_clips(a.out,size,frames)
 FILES['/mediabench']=PROGRAM.read_bytes()
 for k,v in clips.items():FILES['/'+k]=v
 server=Quiet(('127.0.0.1',0),Handler);port=server.server_address[1]
 threading.Thread(target=server.serve_forever,daemon=True).start()
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 from quil_autosave_gate import boot
 disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'mediabench',a.resolution);t=g['text'];q=g['q']
 KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
 def keys(text):
  for k in text:
   if k=='@':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'2'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def expect(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout),s
 def fetched(url,name):
  m,_=expect(f'fetch http://10.0.2.2:{port}{url} {name}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',600);assert m[1].startswith('Saved'),m[1]
 def close(s):
  m=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,60)
  s2=len(t());g['focus'](int(m[3]));time.sleep(.3);q.key('f11');g['wait'](rf'\[app.private.retired\] target={m[1]} task={m[2]} ',s2,90)
 runs={}
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  fetched('/mediabench','/bin/mediabench')
  for name in clips:fetched('/'+name,name)
  files=' '.join('@'+n for n in clips)
  for mode in ('sse','full'):
   flag='--sse ' if mode=='sse' else ''
   done,s=expect(f'run mediabench {flag}--encode --frames {frames} --size {size} {files}',rf'\[mediabench\.done\] simd={mode} decoded=(\d+) encoded=(\d+) failed=(\d+)',3600)
   text=t()[s:]
   assert done[3]=='0',f'{mode}: {done[0]} ' + ' '.join(re.findall(r'\[mediabench\.fail\][^\n]*',text))
   assert int(done[1])==len(clips),done[0]
   cpu=re.search(r'\[mediabench\.cpu\] simd=\S+ flags=(\S+) avx=(\d) avx2=(\d) fma3=(\d) avx512=(\d) xcr0=(\S+)',text);assert cpu,'no cpu record'
   dec={m[0]:dict(codec=m[1],frames=int(m[2]),w=int(m[3]),h=int(m[4]),cycles=int(m[6]),ms=int(m[7]),hash=m[8]) for m in re.findall(r'\[mediabench\.decode\] file=(\S+) codec=(\S+) simd=\S+ frames=(\d+) w=(\d+) h=(\d+) bytes=(\d+) cycles=(\d+) ms=(\d+) hash=(\w+)',text)}
   enc={m[0]:dict(frames=int(m[1]),bytes=int(m[4]),cycles=int(m[5]),ms=int(m[6]),hash=m[7]) for m in re.findall(r'\[mediabench\.encode\] codec=(\S+) simd=\S+ frames=(\d+) w=(\d+) h=(\d+) bytes=(\d+) cycles=(\d+) ms=(\d+) hash=(\w+)',text)}
   runs[mode]=dict(cpu=dict(flags=cpu[1],avx=cpu[2],avx2=cpu[3],fma3=cpu[4],avx512=cpu[5],xcr0=cpu[6]),decode=dec,encode=enc)
   close(s)
  sse,full=runs['sse'],runs['full']
  assert sse['cpu']['avx']=='0' and sse['cpu']['avx2']=='0','the SSE run still had AVX: '+str(sse['cpu'])
  assert int(full['cpu']['xcr0'],16)&6==6,'the kernel did not enable AVX state: '+str(full['cpu'])
  assert full['cpu']['avx2']=='1' and full['cpu']['fma3']=='1','the full run has no AVX2/FMA3 (guest CPU without them?): '+str(full['cpu'])
  report={'ok':True,'accel':accel,'size':size,'frames':frames,'cpu':full['cpu'],'decode':{},'encode':{}}
  for name,d in full['decode'].items():
   s0=sse['decode'][name]
   assert d['frames']==s0['frames']==frames,f'{name}: frames sse={s0["frames"]} full={d["frames"]} want {frames}'
   assert d['hash']==s0['hash'],f'{name} ({d["codec"]}): SSE and AVX2 decodes differ ({s0["hash"]} vs {d["hash"]})'
   report['decode'][d['codec']]={'sse_cycles':s0['cycles'],'avx2_cycles':d['cycles'],'speedup':round(s0['cycles']/max(d['cycles'],1),2),'fps_avx2':round(d['frames']*1000/max(d['ms'],1),1)}
  for codec,e in full['encode'].items():
   e0=sse['encode'].get(codec);assert e0,f'{codec} encoded only with AVX'
   assert e['bytes']>0 and e0['bytes']>0,codec
   report['encode'][codec]={'sse_cycles':e0['cycles'],'avx2_cycles':e['cycles'],'speedup':round(e0['cycles']/max(e['cycles'],1),2),'same_output':e['hash']==e0['hash']}
  assert 'KERNEL PANIC' not in t() and 'panicked' not in t() and '[mediabench.fatal]' not in t()
  (a.out/'mediabench.json').write_text(json.dumps(report,indent=1))
  for kind in ('decode','encode'):
   for codec,r in report[kind].items():print(f'[mediabench.speedup] {kind} {codec} x{r["speedup"]} (SSE {r["sse_cycles"]} cycles, AVX2 {r["avx2_cycles"]})')
  print(json.dumps(report))
 finally:g['stop']()
if __name__=='__main__':main()
