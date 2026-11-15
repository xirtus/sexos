#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Persist nonunity project v3, then hear/export an overloaded wide mix after cold boot."""
import sys,os,ast,re,json,shutil,hashlib,time,argparse,subprocess,struct,array,math
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'scripts'))
from moan_workstation_gate import Probe,sha
from moan_gate_geometry import ready,viewport,origin
from quil_autosave_gate import boot
from install_program import install
from moan_gate import open_recording
from program_web_daily_gate import wait_closed_session,require_empty_restored_session
p=argparse.ArgumentParser(description=__doc__)
for name in('iso','source-disk','seed-elf','out'):p.add_argument('--'+name,type=Path,required=True)
a=p.parse_args();OUT=a.out.resolve();OUT.mkdir(exist_ok=False,parents=True)
inputs={str(f):sha(f)for f in(a.iso,a.source_disk,a.seed_elf,Path(__file__),ROOT/'iso_root/programs/moan',ROOT/'iso_root/programs/files',ROOT/'Cargo.lock')}
for d in('apps/moan','crates/moan-core','crates/moan-export','crates/weft','crates/serenade-core','crates/sex-app','crates/sex-pdx','crates/sex-font','assets/fonts','scripts'):
 for f in(ROOT/d).rglob('*'):
  if f.is_file()and'__pycache__'not in f.parts and'target'not in f.relative_to(ROOT/d).parts:inputs[str(f)]=sha(f)
(OUT/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n');(OUT/'source-commit.txt').write_text(subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True))
source=ROOT/'scripts/moan_workspace_gate.py';node=next(n for n in ast.parse(source.read_text()).body if isinstance(n,ast.ClassDef)and n.name=='Driver')
wav=OUT/'boot1.wav';ns=dict(Probe=Probe,re=re,time=time,wav=wav,viewport=viewport,origin=origin);exec(compile(ast.Module(body=[node],type_ignores=[]),str(source),'exec'),ns);Driver=ns['Driver']
disk=OUT/'nvme.img';shutil.copyfile(a.source_disk,disk);install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes())for n in('moan','files')]+[('moanrack',a.seed_elf.read_bytes())]);os.environ['SEXOS_GATE_BOUND']='1800'
def capture(path):os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={path},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
def save(c,g,fs,surface,name,trigger):
 g['wait'](r'\[files.app.question\] program=moan mode=Save',trigger,120);c.focus(fs);c.chord('ctrl','h');c.chord('ctrl','s')
 for _ in range(24):c.q.key('backspace');time.sleep(.03)
 c.type(name);c.q.key('ret');g['wait'](rf'\[files.pick.granted\] row=\d+ path=/{name}',trigger,120);c.focus(surface)
capture(wav);g=boot(a.iso,disk,OUT,'master-save','1280x800')
try:
 c=Driver(g,OUT,'1280x800');g['wait'](r'\[app.private.retired\] target=41 ',0,180)
 seed=c.run('run moanrack @/ --seed-live-mixer');g['wait'](r'\[moanrack.live.seed\] ok=1 frames=576000 left_hz=400 right_hz=600 developer_only=1',seed,180);launch=g['wait'](r'\[shell.program.launch\] target=(\d+) task=(\d+) ',seed,120);g['wait'](rf'\[app.private.retired\] target={launch[1]} task={launch[2]} ',seed,120);wait_closed_session(g,seed,int(launch[1]),int(launch[2]))
 st,ft,ftask,fs=c.launch('files','files');g['wait'](r'\[files.app.ready\].*portal=1',st,180)
 st,target,task,surface=c.launch('moan');g['wait'](r'\[moan.ready\]',st,180);ready(g,surface)
 for i in range(3):
  st=c.command(surface,'ctrl','o');c.select_file(fs,surface,'live.wav',st);fresh=c.command(surface,'ctrl','shift','l');g['wait'](rf'\[moan.project\] tracks={i+1} frames=576000 ',fresh,120)
 c.command(surface,'ctrl','m');c.click(surface,'Master')
 for i in range(3):c.mix(surface,i,'volume',200)
 c.mix(surface,None,'master_level',50)
 st=c.command(surface,'ctrl','s');save(c,g,fs,surface,'headroom.moan',st);saved=g['wait'](r'\[moan.project.saved\] bytes=(\d+) tracks=3 clips=3 hash=(0x[0-9a-f]+) synced=1',st,300);saved_bytes,saved_hash=int(saved[1]),saved[2]
 c.close(surface,target,task);closed=len(c.text());c.focus(fs);c.q.key('f11');wait_closed_session(g,closed,ft,ftask);c.q.cmd('quit');g['proc'].wait(30)
finally:g['stop']()
extractor=OUT/'sexfs-extract';subprocess.run(['rustc','--edition=2021','-O','-A','dead_code','-A','unused_attributes',str(ROOT/'scripts/sexfs_extract.rs'),'-o',str(extractor)],check=True)
def extract(name):
 p=OUT/name;subprocess.run([str(extractor),str(disk),'/'+name,str(p)],check=True);return p
project=extract('headroom.moan');fixture=extract('live.wav');data=project.read_bytes();assert data[:8]==b'MOANPRJ\0'and len(data)==saved_bytes
assert struct.unpack_from('<HHI',data,8)==(3,0,48000);assert struct.unpack_from('<Q',data,16)[0]==len(data)
value=0xcbf29ce484222325
for byte in data[:24]+data[32:]:value=((value^byte)*0x100000001b3)&((1<<64)-1)
assert struct.unpack_from('<Q',data,24)[0]==value,'internal project checksum'
whole=0xcbf29ce484222325
for byte in data:whole=((whole^byte)*0x100000001b3)&((1<<64)-1)
assert hex(whole)==saved_hash,'whole-file saved receipt'
sources,tracks,selected,master=struct.unpack_from('<HHHH',data,32);assert(sources,tracks,selected,master)==(3,3,2,50)
with open_recording(fixture)as f:
 assert(f.getframerate(),f.getnchannels(),f.getsampwidth(),f.getnframes())==(48000,2,2,576000);raw=f.readframes(576000)
at=64
for _ in range(3):
 frames,=struct.unpack_from('<I',data,at);at+=4;assert frames==576000
 assert data[at:at+frames*4]==raw,'project altered original source PCM';at+=frames*4
 assert data[at:at+4]==bytes(4);at+=4
for i in range(3):
 assert struct.unpack_from('<HHhBB',data,at)==(1,200,0,0,0);assert data[at+8:at+30]==bytes(22);at+=30
 assert struct.unpack_from('<HHQIIHHII',data,at)==(i,0,0,0,576000,100,0,0,0);at+=32
assert at==len(data)
wav=OUT/'cold-play.wav';ns['wav']=wav;capture(wav);g=boot(a.iso,disk,OUT,'master-reopen','1280x800')
try:
 c=Driver(g,OUT,'1280x800');g['wait'](r'\[app.private.retired\] target=41 ',0,180);require_empty_restored_session(g)
 st,ft,ftask,fs=c.launch('files','files');g['wait'](r'\[files.app.ready\].*portal=1',st,180)
 st,target,task,surface=c.launch('moan','run moan @/headroom.moan');g['wait'](rf'\[moan.project.opened\] bytes={saved_bytes} tracks=3 clips=3 hash={saved_hash} sources_shared=1 clean=1',st,300);g['wait'](r'\[moan.ready\]',st,180);ready(g,surface)
 c.command(surface,'ctrl','m');c.click(surface,'Master');assert c.mix(surface,None,'master_level')[-1]==5000
 for i in range(3):assert c.mix(surface,i,'volume')[-1]==200
 c.screenshot(surface,'master-50-cold.ppm')
 st=c.command(surface,'ctrl','spc');g['wait'](r'\[moan.prefilled\] frames=64512 from=0',st,120);g['wait'](r'\[moan.finished\] frames=672000',st,180)
 st=c.command(surface,'ctrl','shift','1');c.command(surface,'ctrl','shift','e');save(c,g,fs,surface,'headroom.wav',st);made=g['wait'](r'\[moan.export\] format=wav frames=672000 bytes=(\d+) synced=1',st,300);export_bytes=int(made[1])
 c.close(surface,target,task);closed=len(c.text());c.focus(fs);c.q.key('f11');wait_closed_session(g,closed,ft,ftask);c.q.cmd('quit');g['proc'].wait(30)
finally:g['stop']()
with open_recording(wav)as f:
 assert(f.getframerate(),f.getnchannels(),f.getsampwidth())==(48000,2,2);pcm=array.array('h',f.readframes(f.getnframes()))
if sys.byteorder!='little':pcm.byteswap()
measured=[]
for channel,amplitude,frequency in((0,18000,400),(1,24000,600)):
 part=pcm[48000*2+channel:48000*8*2:2];assert len(part)==7*48000
 rms=math.sqrt(sum(x*x for x in part)/len(part));expected=amplitude/math.sqrt(2);assert abs(rms/expected-1)<.04
 assert abs(max(part)-amplitude)<6 and abs(min(part)+amplitude)<6
 crossings=[i for i,(x,y)in enumerate(zip(part,part[1:]))if x<0<=y];hz=(len(crossings)-1)*48000/(crossings[-1]-crossings[0]);assert abs(hz-frequency)<1
 assert all(sum(v*v for v in part[i:i+480])/480>100**2 for i in range(0,len(part)-480,480))
 measured.append(dict(channel=channel,rms=rms,expected_rms=expected,hz=hz,peak=max(part),minimum=min(part)))
export=extract('headroom.wav');assert export.stat().st_size==export_bytes
probe=json.loads(subprocess.check_output(['ffprobe','-v','error','-show_streams','-of','json',str(export)],text=True));stream,=probe['streams'];assert(stream['codec_name'],int(stream['sample_rate']),stream['channels'])==('pcm_s16le',48000,2)
decoded=subprocess.check_output(['ffmpeg','-v','error','-i',str(export),'-f','s16le','-acodec','pcm_s16le','-']);assert len(decoded)==672000*4
values=array.array('h',raw)
if sys.byteorder!='little':values.byteswap()
expected=array.array('h',(v*3 for v in values))
if sys.byteorder!='little':expected.byteswap()
assert decoded==expected.tobytes()+bytes(96000*4),'pre-clamp master export differs from independent6x sum then50percent reference'
assert all(sha(Path(f))==v for f,v in inputs.items()),'source changed'
result=dict(ok=True,scope=__doc__,inputs_sha256=sha(OUT/'inputs.json'),source_commit=(OUT/'source-commit.txt').read_text().strip(),version=3,master=50,track_volume=200,tracks=3,source_frames=576000,project_bytes=saved_bytes,project_sha256=sha(project),project_hash=saved_hash,cold_playback=measured,export_frames=672000,export_sha256=sha(export),decoded_sha256=hashlib.sha256(decoded).hexdigest())
(OUT/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
