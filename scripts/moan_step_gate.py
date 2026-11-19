#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Real Step Sequencer edits, independent captured onset/gate/velocity proof,
cold project persistence and prefilled Stop cancellation. No Live Loops claim."""
import argparse, array, hashlib, json, math, os, re, shutil, struct, subprocess, sys
from pathlib import Path
from moan_workstation_gate import Probe, sha
from moan_gate import open_recording
from moan_gate_geometry import ready, viewport, origin
from install_program import install
from quil_autosave_gate import boot
from program_web_daily_gate import wait_closed_session, require_empty_restored_session
ROOT=Path(__file__).resolve().parents[1]
RATE=48000
STEPS=(1,3,7,13)

def samples(path):
    raw=path.read_bytes()
    assert 44<len(raw)<=16*1024**2+44 and (len(raw)-44)%4==0
    assert raw[:4]==b'RIFF' and raw[8:20]==b'WAVEfmt '+struct.pack('<I',16) and raw[36:40]==b'data'
    assert (struct.unpack_from('<I',raw,4)[0],struct.unpack_from('<I',raw,40)[0]) in ((0,0),(len(raw)-8,len(raw)-44))
    with open_recording(path) as f:
        assert (f.getnchannels(),f.getsampwidth(),f.getframerate())==(2,2,RATE)
        pcm=array.array('h',f.readframes(f.getnframes()))
    if sys.byteorder!='little':pcm.byteswap()
    assert max(abs(a-b)for a,b in zip(pcm[::2],pcm[1::2]))<=2
    return pcm[::2]

def timing_oracle(path,bpm=120):
    pcm=samples(path);window=48
    loud=[sum(x*x for x in pcm[i:i+window])/window>100**2 for i in range(0,len(pcm)-window,window)]
    bursts=[];i=0
    while i<len(loud):
        if not loud[i]:i+=1;continue
        end=i+1
        while end<len(loud) and loud[end]:end+=1
        part=pcm[i*window:end*window];ups=[j for j,(a,b)in enumerate(zip(part,part[1:])) if a<0<=b]
        assert len(ups)>4
        bursts.append(dict(start=i/1000,seconds=(end-i)/1000,hz=(len(ups)-1)*RATE/(ups[-1]-ups[0]),rms=math.sqrt(sum(x*x for x in part)/len(part))))
        i=end
    assert len(bursts)==4,('actual step onset count',bursts)
    # Independent rational beat positions; do not import the Rust step helper or
    # trust telemetry. One eighth note =30/BPM seconds.
    expected=[step*30/bpm for step in STEPS]
    latency=bursts[0]['start']-expected[0];assert -.003<=latency<=.025,('initial onset latency',latency)
    for i,(burst,start) in enumerate(zip(bursts,expected)):
        assert abs(burst['start']-latency-start)<.003,('musical onset',i,burst,start)
        duration=30/bpm*(.75 if i<3 else .5)
        assert abs(burst['seconds']-duration)<.006,('actual note gate',i,burst,duration)
        assert abs(burst['hz']-440*2**((60-69)/12))<4,('actual C4 pitch',burst)
    assert .43<bursts[3]['rms']/(sum(b['rms']for b in bursts[:3])/3)<.57,('actual velocity energy',bursts)
    return dict(ok=True,bpm=bpm,bursts=bursts,latency=latency,scope='Independent recorded PCM onset/gate/pitch/energy, not note telemetry')

def stop_oracle(path,marker,queued=64512):
    assert marker<queued-int(.55*RATE),('Stop probe arrived after prebuffer was consumed',marker)
    pcm=samples(path);energetic=[i for i,x in enumerate(pcm)if abs(x)>100];assert energetic,'Stop probe never produced sound'
    assert max(energetic)<marker+int(.55*RATE),('Stop failed to cancel queued audio',marker,max(energetic))
    assert len(pcm)<int(2.0*RATE),('Stop stream stayed active',len(pcm))
    return dict(ok=True,queued_frames=queued,recorded_frames_at_ack=marker,last_energetic_frame=max(energetic),captured_frames=len(pcm),maximum_remaining_seconds=.55)

def project_oracle(data):
    assert data[:8]==b'MOANPRJ\0' and len(data)<2*1024**2
    assert struct.unpack_from('<HHI',data,8)==(2,0,RATE)
    assert struct.unpack_from('<Q',data,16)[0]==len(data)
    checksum=0xcbf29ce484222325
    for value in data[:24]+data[32:]:checksum=((checksum^value)*0x100000001b3)&((1<<64)-1)
    assert struct.unpack_from('<Q',data,24)[0]==checksum
    assert struct.unpack_from('<HHHH',data,32)==(1,1,0,0)
    assert struct.unpack_from('<I',data,64)[0]==192000
    at=68+192000*4
    assert struct.unpack_from('<HHHH',data,at)==(1,0,120,4);at+=8
    notes=[]
    for i,step in enumerate(STEPS):
        note=struct.unpack_from('<IIHH',data,at);at+=12
        expected=(step*12000,9000 if i<3 else 6000,60,96 if i<3 else 48)
        assert note==expected,(note,expected);notes.append(note)
    assert struct.unpack_from('<H',data,at)[0]==1;at+=30
    source,res,start,offset,length,gain,res2,fade_in,fade_out=struct.unpack_from('<HHQIIHHII',data,at);at+=32
    assert (source,res,start,offset,length,gain,res2,fade_in,fade_out)==(0,0,0,0,192000,100,0,0,0)
    assert at==len(data)
    return dict(ok=True,notes=notes,bytes=len(data),sha256=hashlib.sha256(data).hexdigest())

class Stepper(Probe):
    def grid(self,surface):
        start=self.command(surface,'ctrl','j')
        m=self.g['wait'](r'\[moan.steps\] x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) gate=(\d+) velocity=(\d+)',start,120)
        x,y,w,h,gate,velocity=map(int,m.groups());tile,size=viewport(self.g,surface,start);ox,oy=origin(tile,size)
        assert size==(self.rw-16,self.rh-70),('actual framebuffer/client mismatch',size,(self.rw,self.rh))
        assert 0<=x and 0<=y and x+w<=size[0] and y+h<=size[1],(m[0],size)
        return (ox+x,oy+y,w,h),gate,velocity
    def toggle(self,surface,index,count):
        (x,y,w,h),_,_=self.grid(surface);start=len(self.text())
        self.move(x+(64+(index+.5)*1056/16)*w/1120,y+(24+3*22+11)*h/200)
        self.button(True);self.button(False);self.midi(start,count,0)
    def slider(self,surface,name,value,lo,hi):
        self.grid(surface);start=self.command(surface,'ctrl','shift','l')
        m=self.g['wait'](rf'\[moan.slider\] name={name} x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) value=(\d+)',start,120)
        tile,size=viewport(self.g,surface,start);ox,oy=origin(tile,size);x,y,w,h,_=map(int,m.groups())
        self.move(ox+x+8+(w-16)*(value-lo)/(hi-lo),oy+y+h/2);self.button(True);self.button(False)
        # Physical pixel rounding can land one integer step away. Exact public
        # keyboard adjustment reaches the chosen value, never edits fixture data.
        for _ in range(4):
            start=self.command(surface,'ctrl','shift','l')
            m=self.g['wait'](rf'\[moan.slider\] name={name} x=-?\d+ y=-?\d+ w=\d+ h=\d+ value=(\d+)',start,120);actual=int(m[1])
            if actual==value:return
            self.q.key('left'if actual>value else'right')
        raise AssertionError(('slider control failed',name,actual,value))

def pixels(rect):
    x,y,w,h=rect
    def check(width,height,rgb):
        assert x>=0 and y>=0 and x+w<=width and y+h<=height
        def colour(step,row):
            xx=x+round((64+(step+.5)*1056/16)*w/1120);yy=y+round((24+row*22+10)*h/200)
            return int.from_bytes(rgb[(yy*width+xx)*3:(yy*width+xx)*3+3],'big')
        for index in STEPS:assert colour(index,3)==0x388d74,('active step pixel',index,hex(colour(index,3)))
        for index in (0,2,4,8,15):assert colour(index,3)!=0x388d74,('inactive step falsely lit',index)
        for index in STEPS:assert colour(index,2)!=0x388d74,'wrong pitch row'
    return check

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('iso','source-disk','out'):parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--resolution',default='1280x800');parser.add_argument('--narrow',action='store_true');args=parser.parse_args()
    out=args.out.resolve();out.mkdir(parents=True,exist_ok=True);assert not any(out.iterdir())
    inputs=dict(base_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),iso_sha256=sha(args.iso),elf_sha256=sha(ROOT/'iso_root/programs/moan'),files_elf_sha256=sha(ROOT/'iso_root/programs/loom'),source_seed_sha256=sha(args.source_disk),source_sha256={})
    for directory in ('apps/moan','crates/moan-core','crates/serenade-core','crates/moan-export','crates/weft','crates/sex-app','crates/sex-pdx','crates/sex-font','assets/fonts'):
        for source in (ROOT/directory).rglob('*'):
            if source.is_file()and'__pycache__'not in source.parts and'target'not in source.relative_to(ROOT/directory).parts:inputs['source_sha256'][str(source.relative_to(ROOT))]=sha(source)
    for name in ('Cargo.lock','scripts/moan_step_gate.py','scripts/moan_workstation_gate.py','scripts/moan_gate.py','scripts/moan_project_gate.py','scripts/moan_gate_geometry.py','scripts/desktop_focus.py'):inputs['source_sha256'][name]=sha(ROOT/name)
    (out/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n')
    for name,digest in inputs['source_sha256'].items():
        source=ROOT/name;assert sha(source)==digest;dest=out/'source-snapshot'/name;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(source,dest)
    disk=out/'nvme.img';shutil.copyfile(args.source_disk,disk);install(disk,[(name,(ROOT/'iso_root/programs'/name).read_bytes())for name in ('moan','loom')])
    wav=out/'steps.wav';os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0';os.environ.setdefault('SEXOS_GATE_BOUND','1200')
    shots=[];g=boot(args.iso,disk,out,'step-save',args.resolution)
    try:
        c=Stepper(g,out,args.resolution);g['wait'](r'\[app.private.retired\] target=41 ',0,180);g['wait'](r'\[bell.sound.ready\]',0,180)
        start,ft,ftask,files_surface=c.launch('files','files');g['wait'](r'\[files.app.ready\].*portal=1',start,180)
        start,target,task,surface=c.launch('moan');g['wait'](r'\[moan.ready\]',start,180);ready(g,surface)
        c.command(surface,'ctrl','shift','i');start=c.command(surface,'ctrl','shift','backspace');c.midi(start,0,0)
        for count,index in enumerate(STEPS[:3],1):c.toggle(surface,index,count)
        c.slider(surface,'step_gate',50,1,100);c.slider(surface,'step_velocity',48,1,127);c.toggle(surface,STEPS[3],4)
        rect,gate,velocity=c.grid(surface);assert(gate,velocity)==(50,48)
        shots.append(c.shot(surface,'step-sequencer',pixels(rect),dict(steps=STEPS,keys=[60]*4,gate=[75,75,75,50],velocity=[96,96,96,48],resolution=args.resolution)))
        start=c.command(surface,'ctrl','spc');g['wait'](r'\[moan.prefilled\] frames=64512 from=0',start,120);g['wait'](r'\[moan.finished\] frames=288000',start,180)
        if args.narrow:
            c.close(surface,target,task,dirty=True);closed=len(c.text());c.focus(files_surface);c.q.key('f11');wait_closed_session(g,closed,ft,ftask)
            saved=None
        else:
            start=c.command(surface,'ctrl','s');g['wait'](r'\[files.app.question\] program=moan mode=Save',start,120);c.focus(files_surface);c.chord('ctrl','h');c.chord('ctrl','s')
            for _ in range(24):c.q.key('backspace');__import__('time').sleep(.12)
            c.type('steps.moan');c.q.key('ret');saved=g['wait'](r'\[moan.project.saved\] bytes=(\d+) tracks=1 clips=1 hash=(0x[0-9a-f]+) synced=1',start,240)
            c.close(surface,target,task);closed=len(c.text());c.focus(files_surface);c.q.key('f11');wait_closed_session(g,closed,ft,ftask)
        c.q.cmd('quit');g['proc'].wait(30)
    finally:g['stop']()
    audio=timing_oracle(wav);project=None;stop=None
    if not args.narrow:
        extractor=out/'sexfs-extract';subprocess.run(['rustc','--edition=2021','-O','-A','dead_code','-A','unused_attributes',str(ROOT/'scripts/sexfs_extract.rs'),'-o',str(extractor)],check=True)
        project_file=out/'steps.moan';subprocess.run([str(extractor),str(disk),'/steps.moan',str(project_file)],check=True);project=project_oracle(project_file.read_bytes());assert project['bytes']==int(saved[1])
        stop_wav=out/'stop.wav';os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={stop_wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
        g=boot(args.iso,disk,out,'step-reopen',args.resolution)
        try:
            c=Stepper(g,out,args.resolution);g['wait'](r'\[app.private.retired\] target=41 ',0,180);require_empty_restored_session(g)
            start,target,task,surface=c.launch('moan','run moan @/steps.moan');g['wait'](rf'\[moan.project.opened\] bytes={saved[1]} tracks=1 clips=1 hash={saved[2]} sources_shared=1 clean=1',start,240);g['wait'](r'\[moan.ready\]',start,180);ready(g,surface)
            rect,_,_=c.grid(surface);shots.append(c.shot(surface,'step-reopened',pixels(rect),dict(project_sha256=project['sha256'],cold_reopen=True)))
            start=c.command(surface,'ctrl','spc');g['wait'](r'\[moan.prefilled\] frames=64512 from=0',start,120)
            start=c.command(surface,'esc');g['wait'](r'\[moan.stopped\] queued_discarded=1',start,120);marker=max(0,(stop_wav.stat().st_size-44)//4)
            assert marker<64512-int(.55*RATE),('Stop probe arrived after prebuffer was consumed',marker)
            # A fresh real stopped-state screenshot allows publication to settle;
            # neither a delay nor the Stop receipt substitutes for audio below.
            rect,_,_=c.grid(surface);shots.append(c.shot(surface,'step-stopped',pixels(rect),dict(stopped=True,queued_frames=64512,recorded_frames_at_ack=marker)))
            c.close(surface,target,task);c.q.cmd('quit');g['proc'].wait(30)
        finally:g['stop']()
        stop=stop_oracle(stop_wav,marker)
    assert all(sha(ROOT/name)==digest for name,digest in inputs['source_sha256'].items()),'source changed during gate'
    result=dict(ok=True,audio=audio,project=project,stop=stop,screenshots=shots,inputs_sha256=sha(out/'inputs.json'),scope='Narrow actual editor+audible onset proof only'if args.narrow else __doc__)
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
if __name__=='__main__':main()
