#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Actual Weft MIDI edits, independent synth/sampler PCM checks, v2 reboot and pixels.
Telemetry never substitutes for recorded audio. This is a workstation slice, not
external MIDI hardware, third-party plugins, live inserts or full DAW parity.
"""
import argparse,array,hashlib,json,math,os,re,shutil,struct,subprocess,time,wave
from pathlib import Path
from check_disk_persistence import ppm_rgb
from install_program import install
from quil_autosave_gate import boot
from moan_project_gate import Controls
from moan_gate_geometry import ready,viewport,origin,control,discard
from moan_gate import open_recording
from program_web_daily_gate import wait_closed_session,require_empty_restored_session
ROOT=Path(__file__).resolve().parents[1]
RATE=48000

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def capture_oracle(path):
    raw=path.read_bytes()
    assert 44<len(raw)<=16*1024*1024+44 and (len(raw)-44)%4==0,'bounded stereo recording'
    assert raw[:4]==b'RIFF' and raw[8:20]==b'WAVEfmt '+struct.pack('<I',16) and raw[36:40]==b'data','QEMU recording container'
    riff,data=struct.unpack_from('<I',raw,4)[0],struct.unpack_from('<I',raw,40)[0]
    assert (riff,data) in ((0,0),(len(raw)-8,len(raw)-44)),'malformed recording lengths'
    # The shared adapter admits only QEMU's exact bounded unfinished-header
    # profile; it never changes the recording on disk or accepts arbitrary RIFF.
    with open_recording(path) as f:
        assert (f.getnchannels(),f.getsampwidth(),f.getframerate())==(2,2,RATE)
        assert f.getnframes()<RATE*300
        samples=array.array('h',f.readframes(f.getnframes()))
    if __import__('sys').byteorder!='little':samples.byteswap()
    left=samples[::2];right=samples[1::2]
    assert len(left)==len(right) and left and max(abs(a-b)for a,b in zip(left,right))<=2,'stereo MIDI output differs'
    window=480;loud=[]
    for i in range(0,len(left)-window,window):
        values=left[i:i+window];loud.append(sum(x*x for x in values)/window>100**2)
    # Count musical onsets, not threshold islands in a decaying noisy sample.
    # A half-second silent fence separates jobs. Synths still require EVERY
    # interior 10ms window to pass the original continuity threshold below.
    active=[i for i,on in enumerate(loud)if on];groups=[]
    for i in active:
        if not groups or i-groups[-1][-1]>50:groups.append([i])
        else:groups[-1].append(i)
    assert len(groups)==4,('expected four isolated sound onsets',[(g[0]/100,g[-1]/100)for g in groups])
    bursts=[]
    for group in groups:
        start,end=group[0],group[-1]+1;part=left[start*window:end*window]
        ups=[k for k,(a,b)in enumerate(zip(part,part[1:]))if a<0<=b]
        hz=(len(ups)-1)*RATE/(ups[-1]-ups[0])if len(ups)>1 else 0
        bursts.append(dict(start=start/100,seconds=(end-start)/100,hz=hz,rms=math.sqrt(sum(x*x for x in part)/len(part)),peak=max(abs(x)for x in part),threshold_tail_islands=sum(not loud[i-1]for i in group if i>start)))
    low,high,slow,fast=bursts
    for group in groups[:2]:
        assert all(loud[i]for i in range(group[0],group[-1]+1)),('synth dropout',group)
    assert abs(low['hz']-220)<5 and abs(high['hz']-440)<8,bursts
    assert all(3.9<b['seconds']<4.1 for b in (low,high)),bursts
    assert .42<high['rms']/low['rms']<.58,('velocity did not change actual energy',bursts)
    assert .30<slow['seconds']<.45 and .15<fast['seconds']<.23,('sampler octave duration',bursts)
    assert 1.7<fast['hz']/slow['hz']<2.4,('sampler octave pitch',bursts)
    assert .65<fast['rms']/slow['rms']<1.35,('sampler PCM energy',bursts)
    assert slow['peak']>500 and fast['peak']>500,bursts
    sampler=[]
    for group,duration in zip(groups[2:],(.4,.2)):
        begin=max(0,(group[0]-1)*window)
        onset=next((i for i in range(begin,(group[0]+1)*window)if abs(left[i])>32),None)
        assert onset is not None
        support=left[onset:onset+round((duration+.1)*RATE)]
        energy=[x*x for x in support];total=sum(energy);assert total>0
        cumulative=0;times={}
        for i,e in enumerate(energy):
            cumulative+=e
            for fraction in (.5,.9,.99):
                if fraction not in times and cumulative>=fraction*total:times[fraction]=i/RATE
        # Actual source sample has finite 0.2s support at C4: octave down doubles
        # it. No late islands/new onset or unrelated signal may extend that tail.
        assert all(abs(x)<=32 for x in support[round((duration+.005)*RATE):]),'sampler sound after finite support'
        for i in range(group[0],group[-1]+1):
            if not loud[i]:assert i/100-onset/RATE>duration*.75,'early sampler dropout'
        sampler.append(dict(onset=onset/RATE,energy=total,decay50=times[.5],decay90=times[.9],decay99=times[.99]))
    assert .4<sampler[1]['energy']/sampler[0]['energy']<.6,('octave total energy scaling',sampler)
    for key in ('decay50','decay90','decay99'):
        assert 1.7<sampler[0][key]/sampler[1][key]<2.3,('octave decay time scaling',sampler)
    for before,after in zip(bursts,bursts[1:]):
        assert .5<after['start']-(before['start']+before['seconds'])<10,'unexpected sound spacing'
    return dict(ok=True,bursts=bursts,sampler=sampler,scope='Recorded stereo PCM: unchanged strict synth pitch/continuous duration/velocity energy; sampler independent onset, total energy, finite support and octave decay time scaling. No UI telemetry used as audio.')

def project_oracle(data):
    assert len(data)<=96*1024**2 and data[:8]==b'MOANPRJ\0'
    assert struct.unpack_from('<HHI',data,8)==(2,0,RATE)
    assert struct.unpack_from('<Q',data,16)[0]==len(data)
    checksum=0xcbf29ce484222325
    for b in data[:24]+data[32:]:checksum=((checksum^b)*0x100000001b3)&((1<<64)-1)
    assert checksum==struct.unpack_from('<Q',data,24)[0]
    sources,tracks,selected,reserved=struct.unpack_from('<HHHH',data,32)
    assert (sources,tracks,selected,reserved)==(3,3,0,0)
    at=64;patterns=[]
    for _ in range(sources):
        frames,=struct.unpack_from('<I',data,at);at+=4;assert frames==192000
        pcm=data[at:at+frames*4];assert len(pcm)==frames*4 and any(pcm);at+=frames*4
        tag,instrument,bpm,count=struct.unpack_from('<HHHH',data,at);at+=8
        assert tag==1 and instrument in (0,1,2)and bpm==120 and count<=128
        notes=[]
        for _ in range(count):
            start,length,key,velocity=struct.unpack_from('<IIHH',data,at);at+=12
            assert 0<length<=frames and start+length<=frames and key<128 and 0<velocity<128
            notes.append(dict(start=start,frames=length,key=key,velocity=velocity))
        patterns.append(dict(instrument=instrument,frames=frames,notes=notes,pcm_sha256=hashlib.sha256(pcm).hexdigest()))
    assert patterns[0]['instrument']==2 and patterns[0]['notes']==[dict(start=0,frames=48000,key=60,velocity=48)],patterns
    assert [(p['instrument'],len(p['notes']))for p in patterns[1:]]==[(0,8),(1,8)]
    for _ in range(tracks):
        count,=struct.unpack_from('<H',data,at);assert count==1;at+=30
        source,res,start,offset,length,gain,res2,fade_in,fade_out=struct.unpack_from('<HHQIIHHII',data,at);at+=32
        assert source<3 and (res,start,offset,length,gain,res2,fade_in,fade_out)==(0,0,0,192000,100,0,0,0)
    assert at==len(data)
    return dict(ok=True,version=2,tracks=tracks,patterns=patterns,bytes=len(data),sha256=hashlib.sha256(data).hexdigest())

class Probe(Controls):
    def command(self,surface,*keys):
        self.focus(surface);start=len(self.text());self.chord(*keys);return start
    def click(self,surface,name):
        x,y=control(self.g,surface,name,(self.rw,self.rh));self.move(x,y);self.button(True);self.button(False)
    def midi(self,start,notes=None,instrument=None):
        n=r'\d+'if notes is None else str(notes);i=r'\d+'if instrument is None else str(instrument)
        return self.g['wait'](rf'\[moan.midi\] track=(\d+) instrument=({i}) notes=({n}) frames=(\d+) bpm=(\d+) hash=(0x[0-9a-f]+)',start,120)
    def canvas(self,surface):
        start=self.command(surface,'ctrl','p');m=self.g['wait'](r'\[moan.piano\] x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+)',start,120)
        x,y,w,h=map(int,m.groups());tile,size=viewport(self.g,surface,start);ox,oy=origin(tile,size)
        assert 0<=x and 0<=y and x+w<=size[0]and y+h<=size[1],(m[0],size)
        return ox+x,oy+y,w,h
    def add_note(self,surface,key):
        x,y,w,h=self.canvas(surface);start=len(self.text());self.move(x+35*w/560,y+((71-key)*10+4)*h/240);self.button(True);self.button(False);self.midi(start,1)
    def shot(self,surface,name,predicate,semantic):
        self.focus(surface);start=len(self.text());deadline=time.monotonic()+10;attempts=[]
        while True:
            path=self.out/(name+f'-attempt{len(attempts)+1}.ppm');self.q.cmd('screendump',{'filename':str(path)})
            try:width,height,pixels=ppm_rgb(path.read_bytes(),min_width=1,min_height=1);predicate(width,height,pixels);ok=True;error=None
            except (AssertionError,ValueError)as e:ok=False;error=str(e)
            attempts.append(dict(file=path.name,sha256=sha(path),ok=ok,error=error,serial_end=len(self.text())))
            if ok:
                final=self.out/(name+'.ppm');shutil.copyfile(path,final)
                manifest=dict(file=final.name,sha256=sha(final),inputs_sha256=sha(self.out/'inputs.json'),semantic=semantic,serial_start=start,serial_end=len(self.text()),serial_prefix_sha256=hashlib.sha256(self.text().encode()).hexdigest(),attempts=attempts)
                (self.out/(name+'.json')).write_text(json.dumps(manifest,indent=2)+'\n');return manifest
            if time.monotonic()>=deadline:raise AssertionError((name,attempts))
            time.sleep(.1)

def piano_pixels(rect,key,velocity,length):
    x,y,w,h=rect
    def check(width,height,pixels):
        def pixel(px,py):return int.from_bytes(pixels[(py*width+px)*3:(py*width+px)*3+3],'big')
        assert 0<=x and 0<=y and x+w<=width and y+h<=height
        yy=y+round(((71-key)*10+4)*h/240);colour=0x388d74+(velocity//4)*0x100
        for fraction in (.1,.3,.6,.8):
            xx=x+round(w*(32+528*(length/192000)*fraction)/560);assert pixel(xx,yy)==colour,('piano note pixel',xx,yy,hex(pixel(xx,yy)),hex(colour))
        other=y+round(((71-key-1)*10+4)*h/240);assert pixel(x+round(w*.1),other)!=colour,'wrong/all-key piano fill'
    return check

def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ('iso','source-disk','out'):p.add_argument('--'+name,required=True,type=Path)
    p.add_argument('--resolution',default='1280x800');a=p.parse_args();out=a.out.resolve();out.mkdir(parents=True,exist_ok=True);assert not any(out.iterdir())
    inputs=dict(base_revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),iso_sha256=sha(a.iso),elf_sha256=sha(ROOT/'iso_root/programs/moan'),files_elf_sha256=sha(ROOT/'iso_root/programs/loom'),source_seed_sha256=sha(a.source_disk),source_sha256={})
    for directory in ('apps/moan','crates/moan-core','crates/serenade-core','crates/moan-export','crates/weft','crates/sex-app','crates/sex-pdx','crates/sex-font','assets/fonts'):
        for f in (ROOT/directory).rglob('*'):
            if f.is_file()and '__pycache__'not in f.parts and 'target'not in f.relative_to(ROOT/directory).parts:inputs['source_sha256'][str(f.relative_to(ROOT))]=sha(f)
    for name in ('Cargo.lock','scripts/moan_workstation_gate.py','scripts/moan_project_gate.py','scripts/moan_gate_geometry.py','scripts/desktop_focus.py'):inputs['source_sha256'][name]=sha(ROOT/name)
    (out/'inputs.json').write_text(json.dumps(inputs,indent=2)+'\n')
    for relative,digest in inputs['source_sha256'].items():
        source=ROOT/relative;assert sha(source)==digest
        destination=out/'source-snapshot'/relative;destination.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(source,destination);assert sha(destination)==digest
    disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);install(disk,[(name,(ROOT/'iso_root/programs'/name).read_bytes())for name in ('moan','files')])
    wav=out/'workstation.wav';os.environ['SEXOS_GATE_DEVICES']=f'-audiodev wav,id=snd0,path={wav},out.frequency=48000,out.channels=2 -device virtio-sound-pci,audiodev=snd0'
    os.environ.setdefault('SEXOS_GATE_BOUND','1200');shots=[];g=boot(a.iso,disk,out,'workstation-save',a.resolution)
    try:
        c=Probe(g,out,a.resolution);g['wait'](r'\[app.private.retired\] target=41 ',0,180);g['wait'](r'\[bell.sound.ready\]',0,180)
        start,ft,ftask,files_surface=c.launch('files','files');g['wait'](r'\[files.app.ready\].*portal=1',start,180)
        start,target,task,surface=c.launch('moan');g['wait'](r'\[moan.heap.limit\] bytes=268435456',start,180);g['wait'](r'\[moan.ready\]',start,180);ready(g,surface)
        c.click(surface,'Silk_Keys');c.midi(start,8,0)
        start=c.command(surface,'ctrl','shift','backspace');c.midi(start,0)
        c.add_note(surface,57)
        for _ in range(4):start=c.command(surface,'ctrl','shift','g');c.midi(start,1)
        rect=c.canvas(surface);shots.append(c.shot(surface,'piano-a3',piano_pixels(rect,57,96,192000),dict(key=57,velocity=96,frames=192000)))
        start=c.command(surface,'ctrl','spc');g['wait'](r'\[moan.play\] tracks=1 from=0',start,120)
        meter=c.command(surface,'ctrl','m');m=g['wait'](r'\[moan.meter\] track=0 x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) level=([1-9]\d*)',meter,120)
        tile,size=viewport(g,surface,meter);ox,oy=origin(tile,size);mx,my,mw,mh,level=map(int,m.groups());assert level>3000
        def meter_pixels(width,height,pixels):
            xx,yy=ox+mx+mw//4,oy+my+mh//2;assert 0<=xx<width and 0<=yy<height
            assert bytes(pixels[(yy*width+xx)*3:(yy*width+xx)*3+3])==bytes.fromhex('db4b7e'),'actual post-fader meter fill missing'
        shots.append(c.shot(surface,'mixer-playing',meter_pixels,dict(track=0,source='actual processed samples',level=level)))
        g['wait'](r'\[moan.finished\] frames=288000',start,180)
        c.command(surface,'ctrl','shift','up');c.command(surface,'ctrl','shift','v')
        rect=c.canvas(surface);shots.append(c.shot(surface,'piano-a4-soft',piano_pixels(rect,69,48,192000),dict(key=69,velocity=48,frames=192000)))
        start=c.command(surface,'ctrl','spc');g['wait'](r'\[moan.finished\] frames=288000',start,180)
        start=c.command(surface,'ctrl','n');discard(g,surface,(c.rw,c.rh),start);g['wait'](r'\[moan.project.new\] tracks=0 clean=1',start,120)
        c.command(surface,'ctrl','shift','d');start=c.command(surface,'ctrl','shift','backspace');c.midi(start,0);c.add_note(surface,48)
        for _ in range(2):c.command(surface,'ctrl','shift','g')
        start=c.command(surface,'ctrl','spc');g['wait'](r'\[moan.finished\] frames=288000',start,180)
        c.command(surface,'ctrl','shift','up');start=c.command(surface,'ctrl','spc');g['wait'](r'\[moan.finished\] frames=288000',start,180)
        c.command(surface,'ctrl','shift','i');c.command(surface,'ctrl','shift','b');c.command(surface,'ctrl','shift','r');select_start=len(c.text());c.q.key('home')
        selected=g['wait'](r'\[moan.midi\] track=0 instrument=2 notes=1 frames=192000 bpm=120 hash=(0x[0-9a-f]+)',select_start,120);saved_pattern=selected[1]
        rect=c.canvas(surface);shots.append(c.shot(surface,'sampler-edited',piano_pixels(rect,60,48,48000),dict(key=60,velocity=48,frames=48000)))
        start=c.command(surface,'ctrl','s');g['wait'](r'\[files.app.question\] program=moan mode=Save',start,120);c.focus(files_surface);c.chord('ctrl','h');c.chord('ctrl','s')
        for _ in range(24):c.q.key('backspace');time.sleep(.12)
        c.type('workstation.moan');c.q.key('ret');saved=g['wait'](r'\[moan.project.saved\] bytes=(\d+) tracks=3 clips=3 hash=(0x[0-9a-f]+) synced=1',start,240);saved_bytes,saved_hash=int(saved[1]),saved[2]
        arrange_start=c.command(surface,'ctrl','t');m=g['wait'](r'\[moan.timeline\] x=(-?\d+) y=(-?\d+) w=(\d+) h=(\d+) start=0 span=192000',arrange_start,120)
        tile,size=viewport(g,surface,arrange_start);ox,oy=origin(tile,size);tx,ty,tw,th=map(int,m.groups());assert th==188
        def arrangement_pixels(width,height,pixels):
            # Approved ordered track palette; selected blue remains blue.
            for row,colour in enumerate((0x48597d,0x265e66,0x336045)):
                xx,yy=ox+tx+round(tw*.93),oy+ty+20+row*56+4
                actual=int.from_bytes(pixels[(yy*width+xx)*3:(yy*width+xx)*3+3],'big')
                assert actual==colour,('actual three-track lane',row,hex(actual),hex(colour))
        shots.append(c.shot(surface,'saved-three-track-arrangement',arrangement_pixels,dict(tracks=3,clips=3,project_hash=saved_hash,synced=True)))
        c.close(surface,target,task);closed=len(c.text());c.focus(files_surface);c.q.key('f11');wait_closed_session(g,closed,ft,ftask)
        c.q.cmd('quit');g['proc'].wait(30)
    finally:g['stop']()
    audio=capture_oracle(wav)
    extractor=out/'sexfs-extract';subprocess.run(['rustc','--edition=2021','-O','-A','dead_code','-A','unused_attributes',str(ROOT/'scripts/sexfs_extract.rs'),'-o',str(extractor)],check=True)
    project=out/'workstation.moan';subprocess.run([str(extractor),str(disk),'/workstation.moan',str(project)],check=True);assert project.stat().st_size==saved_bytes;metadata=project_oracle(project.read_bytes())
    os.environ.pop('SEXOS_GATE_DEVICES',None);g=boot(a.iso,disk,out,'workstation-reopen',a.resolution)
    try:
        c=Probe(g,out,a.resolution);g['wait'](r'\[app.private.retired\] target=41 ',0,180);require_empty_restored_session(g)
        start,target,task,surface=c.launch('moan','run moan @/workstation.moan');g['wait'](rf'\[moan.project.opened\] bytes={saved_bytes} tracks=3 clips=3 hash={saved_hash} sources_shared=1 clean=1',start,240);g['wait'](r'\[moan.ready\]',start,180);ready(g,surface)
        g['wait'](rf'\[moan.midi\] track=0 instrument=2 notes=1 frames=192000 bpm=120 hash={saved_pattern}',start,120)
        rect=c.canvas(surface);shots.append(c.shot(surface,'reopened-midi',piano_pixels(rect,60,48,48000),dict(saved_pattern=saved_pattern,key=60,velocity=48,frames=48000)))
        start=c.command(surface,'ctrl','shift','up');c.midi(start,1);start=c.command(surface,'ctrl','z');g['wait'](rf'\[moan.midi\].*hash={saved_pattern}',start,120);g['wait'](r'\[moan.selection\].*dirty=0',start,120)
        c.close(surface,target,task);c.q.cmd('quit');g['proc'].wait(30)
    finally:g['stop']()
    assert all(sha(ROOT/name)==digest for name,digest in inputs['source_sha256'].items()),'source changed while gate ran'
    result=dict(ok=True,audio=audio,project=metadata,reboot_reopen=True,post_reopen_edit_undo_clean=True,screenshots=shots,inputs_sha256=sha(out/'inputs.json'),scope=__doc__)
    (out/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(dict(ok=True,screenshots=len(shots),audio=audio,project_bytes=saved_bytes)))
if __name__=='__main__':main()
