#!/usr/bin/env python3
"""Actual opt-in desktop recorder -> synced Matroska -> independent decode.
No hardware, cross-key isolation, crash recovery or exact-pixel claim is made.
"""
import argparse, hashlib, json, os, re, shutil, subprocess, time
from pathlib import Path
from install_program import install
from quil_autosave_gate import boot
from recording_trace import correlate, check_packets
ROOT=Path(__file__).resolve().parents[1]
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def main():
 p=argparse.ArgumentParser(description=__doc__)
 for name in ('iso','source-disk','program','extractor','out'):p.add_argument('--'+name,type=Path,required=True)
 a=p.parse_args();a.out=a.out.resolve();a.out.mkdir(parents=True,exist_ok=True)
 assert not any(a.out.iterdir()),'fresh evidence directory required'
 result={'ok':False,'scope':'actual desktop recording, saved file and independent decode; no isolation/hardware proof'}
 for name in ('iso','source_disk','program','extractor'):result[name+'_sha256']=sha(getattr(a,name))
 result['trace_helper_sha256']=sha(ROOT/'scripts/recording_trace.py')
 result['gate_sha256']=sha(Path(__file__));result['boot_helper_sha256']=sha(ROOT/'scripts/quil_autosave_gate.py')
 disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b')as f:f.truncate(1024**3)
 install(disk,[('record',a.program.read_bytes())]);os.environ.setdefault('SEXOS_GATE_BOUND','700');g=None
 try:
  g=boot(a.iso.resolve(),disk,a.out,'record','1280x800')
  g['wait'](r'\[app.private.retired\] target=42 ',timeout=240)
  g['focus'](153)
  def line(text):
   for k in text:g['q'].key({' ':'spc','-':'minus','.':'dot'}.get(k,k));time.sleep(.06)
   g['q'].key('ret')
  start=len(g['text']());line('record desktop.mkv')
  armed=g['wait'](r'\[display.recording.armed\] session=(\d+) task=(\d+) epoch=(\d+) generation=(\d+) indicator=cpu_pixels',start,240)
  result['session'],result['task']=int(armed[1]),int(armed[2])
  frame_pattern=rf'\[display.recording.frame\] session={armed[1]} task={armed[2]} generation=(\d+) publication_ms=(\d+)'
  deadline=time.monotonic()+90
  while len(re.findall(frame_pattern,g['text']()[start:]))<4:
   assert time.monotonic()<deadline,'four actual captured frames did not arrive'
   time.sleep(.2)
  g['q'].cmd('screendump',{'filename':str(a.out/'active.ppm')})
  g['focus'](153);line('record-stop')
  saved_pattern=rf'\[record.saved\] session={armed[1]} task={armed[2]} frames=(\d+) last_pts_ms=(\d+) synced=1 released=1\r?\n'
  saved=g['wait'](saved_pattern,start,240)
  text=g['text']()[start:];revoked=re.search(rf'\[display.recording.revoked\] session={armed[1]} task={armed[2]} .*scrubbed=1',text)
  assert revoked and revoked.start()<re.search(saved_pattern,text).start(),'Saved must follow actual revoke/scrub'
  assert not re.search(frame_pattern,text[revoked.end():]),'frame published after revoke'
  result['frames'],result['last_pts_ms']=int(saved[1]),int(saved[2]);assert result['frames']>=2
  trace=correlate(text[:re.search(saved_pattern,text).start()],result['session'],result['task'],result['frames']);result['publication_trace']=trace
  assert '[kernel.panic]'not in text and 'KERNEL PANIC'not in text
  g['stop']();g=None
  movie=a.out/'desktop.mkv';subprocess.run([str(a.extractor.resolve()),str(disk),'/recordings/desktop.mkv',str(movie)],check=True)
  probe=json.loads(subprocess.check_output(['ffprobe','-v','error','-show_entries','stream=codec_name,width,height,pix_fmt:packet=pts_time','-of','json',str(movie)],text=True))
  (a.out/'probe.json').write_text(json.dumps(probe,indent=2)+'\n')
  stream,=probe['streams'];assert (stream['codec_name'],stream['width'],stream['height'],stream['pix_fmt'])==('ffv1',640,360,'bgr0')
  pts=[round(float(row['pts_time'])*1000)for row in probe['packets']]
  assert len(pts)==result['frames'] and pts[0]==0 and pts[-1]==result['last_pts_ms']
  assert all(b>a for a,b in zip(pts,pts[1:]))
  check_packets(trace,pts)
  decoded=a.out/'decoded.rgb';subprocess.run(['ffmpeg','-v','error','-y','-i',str(movie),'-fps_mode','passthrough','-f','rawvideo','-pix_fmt','rgb24',str(decoded)],check=True)
  assert decoded.stat().st_size==result['frames']*640*360*3
  assert len(set(decoded.read_bytes()[:640*360*3]))>8,'captured desktop is blank/uniform'
  result.update(ok=True,movie_sha256=sha(movie),decoded_sha256=sha(decoded),pts_ms=pts)
 except BaseException as e:result['error']=repr(e);raise
 finally:
  if g:g['stop']()
  (a.out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
if __name__=='__main__':main()
