#!/usr/bin/env python3
"""Downloads: opening a 20 MB attachment saves it to Downloads/ in Files as it arrives
(never whole in memory), with progress; the file read back from disk hashes the same as
what the server sent; about:downloads lists it; a second download of the same name gets
"name (2)". Ctrl+Shift+J pauses a paced live transfer without advancing its
file offset, then resumes it; Escape cancels another transfer and cleans up."""
import argparse,http.server,os,random,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
DATA=random.Random(6455).randbytes(20*1024*1024)
def fnv(b):
 h=0xcbf29ce484222325
 for x in b:h=((h^x)*0x100000001b3)&0xFFFFFFFFFFFFFFFF
 return h
class Handler(http.server.BaseHTTPRequestHandler):
 broken_requests=0
 resume_ranges=[]
 close_ranges=[]
 changed_calls=0
 changed_ranges=[]
 def do_GET(self):
  if self.path=='/files/resumable-valid':
   url=f'http://10.0.2.2:{self.server.server_port}/files/resume.bin'.encode()
   body=b'v1\n'+str(2*1024*1024).encode()+b'\t'+b'Downloads/resume.bin'.hex().encode()+b'\t'+url.hex().encode()+b'\t'+b'"resume-v1"'.hex().encode()+b'\n'
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if self.path=='/files/resumable-corrupt':
   body=b'v1\nnot-a-record\n'
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if self.path.startswith('/files/big.bin'):
   self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Disposition','attachment; filename="big.bin"');self.send_header('Content-Length',str(len(DATA)));self.end_headers()
   for i in range(0,len(DATA),1<<16):self.wfile.write(DATA[i:i+(1<<16)])
   return
  if self.path=='/files/slow.bin':
   body=DATA[:8*1024*1024]
   self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Disposition','attachment; filename="slow.bin"');self.send_header('Content-Length',str(len(body)));self.end_headers()
   try:
    for i in range(0,len(body),1<<16):
     self.wfile.write(body[i:i+(1<<16)]);self.wfile.flush();time.sleep(.025)
   except (BrokenPipeError,ConnectionResetError):pass
   return
  if self.path=='/files/pause.bin':
   body=DATA[:12*1024*1024]
   self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Disposition','attachment; filename="pause.bin"');self.send_header('Content-Length',str(len(body)));self.end_headers()
   try:
    for i in range(0,len(body),1<<16):
     self.wfile.write(body[i:i+(1<<16)]);self.wfile.flush();time.sleep(.05)
   except (BrokenPipeError,ConnectionResetError):pass
   return
  if self.path=='/files/small.zip':
   body=b'PK\x03\x04small';self.send_response(200);self.send_header('Content-Type','application/zip');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if self.path=='/files/retry.bin':
   Handler.broken_requests+=1
   body=bytes(range(256))*4
   self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers()
   self.wfile.write(body if Handler.broken_requests>1 else body[:256]);self.close_connection=True;return
  if self.path=='/files/resume.bin':
   body=DATA[:2*1024*1024];range_=self.headers.get('Range')
   if range_:
    Handler.resume_ranges.append((range_,self.headers.get('If-Range')))
    at=int(range_.removeprefix('bytes=').removesuffix('-'))
    self.send_response(206);self.send_header('Content-Range',f'bytes {at}-{len(body)-1}/{len(body)}');piece=body[at:]
   else:self.send_response(200);piece=body[:65536]
   self.send_header('ETag','"resume-v1"');self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Length',str(len(body) if not range_ else len(piece)));self.end_headers();self.wfile.write(piece);self.close_connection=True;return
  if self.path=='/files/close.bin':
   body=DATA[:4*1024*1024];range_=self.headers.get('Range')
   if range_:
    Handler.close_ranges.append((range_,self.headers.get('If-Range')))
    at=int(range_.removeprefix('bytes=').removesuffix('-'))
    self.send_response(206);self.send_header('Content-Range',f'bytes {at}-{len(body)-1}/{len(body)}');piece=body[at:]
   else:self.send_response(200);piece=body
   self.send_header('ETag','"close-v1"');self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Length',str(len(piece)));self.end_headers()
   try:
    for i in range(0,len(piece),1<<16):
     self.wfile.write(piece[i:i+(1<<16)]);self.wfile.flush()
     if not range_:time.sleep(.04)
   except (BrokenPipeError,ConnectionResetError):pass
   self.close_connection=True;return
  if self.path=='/files/change.bin':
   body=b'B'*(1024*1024);range_=self.headers.get('Range');Handler.changed_calls+=1
   if range_:
    Handler.changed_ranges.append((range_,self.headers.get('If-Range')))
    at=int(range_.removeprefix('bytes=').removesuffix('-'))
    self.send_response(206);self.send_header('Content-Range',f'bytes {at}-{len(body)-1}/{len(body)}');piece=body[at:]
   else:self.send_response(200);piece=b'A'*65536 if Handler.changed_calls==1 else body
   self.send_header('ETag','"change-v1"' if Handler.changed_calls==1 else '"change-v2"');self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Length',str(len(body) if not range_ else len(piece)));self.end_headers();self.wfile.write(piece);self.close_connection=True;return
  self.send_response(404);self.end_headers()
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','=':'equal'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'download',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='?':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'slash'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 base=f'http://10.0.2.2:{PORT}/files'
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch http://10.0.2.2:{PORT}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  # A hard stop before the first download write leaves an empty hidden part.
  # A nonempty hidden part may still contain recoverable bytes and must stay.
  for command in ('mkdir Downloads','mkdir Kaleidoscope','mkdir Kaleidoscope/empty-parts',
                  'write Kaleidoscope/empty-parts/empty.bin.ticket .kaleidoscope-empty.bin.part',
                  'touch Downloads/.kaleidoscope-empty.bin.part',
                  'write Downloads/.kaleidoscope-recoverable.bin.part x',
                  'touch Downloads/.kaleidoscope-unrecorded.bin.part','touch Downloads/.notes.part'):
   g['focus'](153);time.sleep(.3);s=len(t());keys(command);q.key('ret')
   g['wait'](r'\[spindle.fs.line\] (ok|already exists)' if command.startswith('mkdir ') else r'\[spindle.fs.line\] ok',s,90)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {base}/big.bin');q.key('ret')
  g['wait'](r'\[web.download.empty_parts.cleaned\] count=1',s,120)
  g['wait'](r'\[web.download.start\] path=Downloads/big.bin ',s,300)
  g['wait'](r'\[web.download.progress\] bytes=\d+ total=20971520',s,300)
  m=g['wait'](r'\[web.download.done\] path=Downloads/big.bin bytes=(\d+) ticks=\d+ fnv=([0-9a-f]+)',s,900)
  assert int(m[1])==len(DATA) and int(m[2],16)==fnv(DATA),(m.group(0),hex(fnv(DATA)))
  trace=t()[s:]
  ordered=[trace.find(f'[web.download.promote] step={step} path=Downloads/big.bin') for step in ('part','rename','name')]
  assert all(at>=0 for at in ordered) and ordered==sorted(ordered) and ordered[-1]<trace.find('[web.download.done] path=Downloads/big.bin'),ordered
  g['wait'](r'\[web.loaded\] ',s,60)
  # The same name again lands beside it; a zip type is a download too.
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/big.bin?again');q.key('ret')
  g['wait'](r'\[web.download.done\] path=Downloads/big \(2\)\.bin bytes=20971520 ',s,900)
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/small.zip');q.key('ret')
  g['wait'](r'\[web.download.done\] path=Downloads/small.zip bytes=9 ',s,300)
  m=g['wait'](r'\[web.loaded\] bytes_ok=1 items=(\d+) links=(\d+)',s,60);assert int(m[2])>=3,m.group(0)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'j'}],'hold-time':80})
  g['wait'](r'\[web.download.panel\] done=3 failed=0',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-durable.ppm')})
  assert (a.out/'browser-download-durable.ppm').stat().st_size>10000
  q.cmd('screendump',{'filename':str(a.out/'browser-download-empty-parts-cleaned.ppm')})
  assert (a.out/'browser-download-empty-parts-cleaned.ppm').stat().st_size>10000
  # A truncated transfer leaves no hidden .part file, and the panel offers retry.
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/retry.bin');q.key('ret')
  g['wait'](r'\[web.download.failed\] name=retry.bin reason=',s,300)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'j'}],'hold-time':80})
  g['wait'](r'\[web.download.panel\] done=3 failed=1',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-retry.ppm')})
  assert (a.out/'browser-download-retry.ppm').stat().st_size>10000
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/retry.bin');q.key('ret')
  g['wait'](r'\[web.download.done\] path=Downloads/retry.bin bytes=1024 ',s,300)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'j'}],'hold-time':80})
  g['wait'](r'\[web.download.panel\] done=4 failed=0',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-recovered.ppm')})
  assert (a.out/'browser-download-recovered.ppm').stat().st_size>10000
  g['wait'](r'\[web.download.history.saved\] count=4 kept=1',0,120)
  # A live transfer stops polling and writing while paused, then resumes the
  # same flow at the exact staged file offset. The complete file still hashes.
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/pause.bin');q.key('ret')
  g['wait'](r'\[web.download.start\] path=Downloads/pause.bin ',s,300)
  g['wait'](r'\[web.download.progress\] bytes=\d+ total=12582912',s,300)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'shift'},{'type':'qcode','data':'j'}],'hold-time':80})
  paused=g['wait'](r'\[web.download.pause\] paused=1 bytes=(\d+)\n',s,120)
  paused_bytes=int(paused[1]);assert 0<paused_bytes<len(DATA[:12*1024*1024]),paused[0]
  time.sleep(1)
  assert '[web.download.progress]' not in t()[s+paused.end():], 'download advanced while paused'
  assert '[web.download.done] path=Downloads/pause.bin' not in t()[s+paused.end():], 'paused transfer completed'
  q.cmd('screendump',{'filename':str(a.out/'browser-download-paused.ppm')})
  assert (a.out/'browser-download-paused.ppm').stat().st_size>10000
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'shift'},{'type':'qcode','data':'j'}],'hold-time':80})
  resumed=g['wait'](r'\[web.download.pause\] paused=0 bytes=(\d+)\n',s,120)
  assert int(resumed[1])==paused_bytes,(resumed[0],paused[0])
  done=g['wait'](r'\[web.download.done\] path=Downloads/pause.bin bytes=(\d+) ticks=\d+ fnv=([0-9a-f]+)',s,900)
  assert int(done[1])==12*1024*1024 and int(done[2],16)==fnv(DATA[:12*1024*1024]),done[0]
  # Escape interrupts an in-flight transfer and removes its hidden partial file.
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/slow.bin');q.key('ret')
  g['wait'](r'\[web.download.start\] path=Downloads/slow.bin ',s,300)
  g['wait'](r'\[web.download.progress\] bytes=\d+ total=8388608',s,300)
  s=len(t());q.key('esc')
  g['wait'](r'\[web.download.cancelled\] path=Downloads/slow.bin cleaned=1\n',s,120)
  g['wait'](r'\[web.download.panel\] done=5 failed=1',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-cancelled.ppm')})
  assert (a.out/'browser-download-cancelled.ppm').stat().st_size>10000
  s=len(t());q.key('f11');g['wait'](r'\[web.closed\] target=\d+ task=\d+',s,120)
  g['focus'](153);time.sleep(.5);s=len(t());keys('run +net kaleidoscope about:downloads');q.key('ret')
  g['wait'](r'\[web.download.history.loaded\] count=5',s,300)
  g['wait'](r'\[web.download.panel\] done=5 failed=0',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-history-reopened.ppm')})
  assert (a.out/'browser-download-history-reopened.ppm').stat().st_size>10000
  # Remove only the newest row from the panel. Its saved file must stay in Downloads.
  s=len(t());q.key('slash');time.sleep(.3);keys('about:downloads?forget=4');q.key('ret')
  g['wait'](r'\[web.download.history.forgot\] path=Downloads/pause.bin kept=1',s,120)
  g['wait'](r'\[web.download.panel\] done=4 failed=0',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-item-forgotten.ppm')})
  assert (a.out/'browser-download-item-forgotten.ppm').stat().st_size>10000
  s=len(t());q.key('f11');g['wait'](r'\[web.closed\] target=\d+ task=\d+',s,120)
  g['focus'](153);time.sleep(.3);s=len(t());keys('ls Downloads');q.key('ret')
  assert 'pause.bin' in g['wait'](r'\[spindle.fs.line\] [^\n]*pause\.bin[^\n]*',s,90)[0]
  # Reopen to prove the removed row does not return from the recovery copy.
  g['focus'](153);time.sleep(.5);s=len(t());keys('run +net kaleidoscope about:downloads');q.key('ret')
  g['wait'](r'\[web.download.history.loaded\] count=4',s,300)
  g['wait'](r'\[web.download.panel\] done=4 failed=0',s,120)
  # A validated hidden part survives this browser process and resumes by Range.
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/resume.bin');q.key('ret')
  g['wait'](r'\[web.download.resume.kept\] path=Downloads/resume.bin bytes=65536\n',s,300)
  s=len(t());q.key('f11');g['wait'](r'\[web.closed\] target=\d+ task=\d+',s,120)
  # A readable but malformed live record must fall back to the saved previous
  # record, preserving the synced part and its exact Range/ETag resume.
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/resumable-valid /Kaleidoscope/resumable.tsv.previous');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/resumable-corrupt /Kaleidoscope/resumable.tsv');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys('run +net kaleidoscope about:downloads');q.key('ret')
  g['wait'](r'\[web.download.resume.loaded\] count=1',s,300)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-resume-recovered.ppm')})
  assert (a.out/'browser-download-resume-recovered.ppm').stat().st_size>10000
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-resumable.ppm')})
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/resume.bin');q.key('ret')
  g['wait'](r'\[web.download.resume\] path=Downloads/resume.bin offset=65536 total=2097152',s,300)
  m=g['wait'](r'\[web.download.done\] path=Downloads/resume.bin bytes=2097152 ticks=\d+ fnv=([0-9a-f]+)',s,300)
  assert int(m[1],16)==fnv(DATA[:2*1024*1024]) and Handler.resume_ranges==[('bytes=65536-','"resume-v1"')],Handler.resume_ranges
  # Closing a live, validated transfer also keeps a hidden part for restart.
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/close.bin');q.key('ret')
  g['wait'](r'\[web.download.progress\] bytes=\d+ total=4194304',s,300)
  s=len(t());q.key('f11')
  kept=g['wait'](r'\[web.download.resume.kept\] path=Downloads/close.bin bytes=(\d+)\n',s,120)
  assert 0<int(kept[1])<4194304,kept[0]
  g['wait'](r'\[web.closed\] target=\d+ task=\d+',s,120)
  g['focus'](153);time.sleep(.5);s=len(t());keys('run +net kaleidoscope about:downloads');q.key('ret')
  g['wait'](r'\[web.download.resume.loaded\] count=1',s,300)
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/close.bin');q.key('ret')
  resumed=g['wait'](r'\[web.download.resume\] path=Downloads/close.bin offset=(\d+) total=4194304',s,300)
  assert int(resumed[1])==int(kept[1]),(resumed[0],kept[0])
  m=g['wait'](r'\[web.download.done\] path=Downloads/close.bin bytes=4194304 ticks=\d+ fnv=([0-9a-f]+)',s,300)
  assert int(m[1],16)==fnv(DATA[:4*1024*1024]) and len(Handler.close_ranges)==1 and Handler.close_ranges[0][1]=='"close-v1"',Handler.close_ranges
  # A changed ETag must discard the old bytes and fetch the new object whole.
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/change.bin');q.key('ret')
  g['wait'](r'\[web.download.resume.kept\] path=Downloads/change.bin bytes=65536\n',s,300)
  s=len(t());q.key('slash');time.sleep(.3);keys(f'{base}/change.bin');q.key('ret')
  g['wait'](r'\[web.download.resume.restart\] reason=range_response',s,300)
  m=g['wait'](r'\[web.download.done\] path=Downloads/change.bin bytes=1048576 ticks=\d+ fnv=([0-9a-f]+)',s,300)
  assert int(m[1],16)==fnv(b'B'*(1024*1024)) and Handler.changed_ranges==[('bytes=65536-','"change-v1"')],Handler.changed_ranges
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-resumed.ppm')})
  assert (a.out/'browser-download-resumable.ppm').stat().st_size>10000 and (a.out/'browser-download-resumed.ppm').stat().st_size>10000
  # Clear the panel's saved list without removing the downloaded files.
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'j'}],'hold-time':80})
  g['wait'](r'\[web.download.panel\] done=\d+ failed=0',s,120)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'alt'},{'type':'qcode','data':'j'}],'hold-time':80})
  g['wait'](r'\[web.download.history.cleared\] count=\d+ kept=1',s,120)
  g['wait'](r'\[web.download.panel\] done=0 failed=0',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-history-cleared.ppm')})
  assert (a.out/'browser-download-history-cleared.ppm').stat().st_size>10000
  s=len(t());q.key('f11');g['wait'](r'\[web.closed\] target=\d+ task=\d+',s,120)
  g['focus'](153);time.sleep(.5);s=len(t());keys('run +net kaleidoscope about:downloads');q.key('ret')
  g['wait'](r'\[web.download.history.loaded\] count=0',s,300)
  g['wait'](r'\[web.download.panel\] done=0 failed=0',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'browser-download-hidden-parts-review.ppm')})
  assert (a.out/'browser-download-hidden-parts-review.ppm').stat().st_size>10000
  g['focus'](153);time.sleep(.3);s=len(t());keys('ls Downloads');q.key('ret')
  g['wait'](r'\[spindle.fs.line\] \.kaleidoscope-recoverable\.bin\.part  2',s,90)
  assert '.notes.part' in t()[s:],t()[s:]
  assert '.kaleidoscope-unrecorded.bin.part' in t()[s:],t()[s:]
  assert '.kaleidoscope-empty.bin.part' not in t()[s:],t()[s:]
  # Power loss after a ticket and CREATE must clean only that zero-byte part.
  for command in ('write Kaleidoscope/empty-parts/reboot.bin.ticket .kaleidoscope-reboot.bin.part',
                  'touch Downloads/.kaleidoscope-reboot.bin.part',
                  'write Kaleidoscope/empty-parts/stale.bin.ticket .kaleidoscope-stale.bin.part',
                  'write Kaleidoscope/empty-parts/nonempty.bin.ticket .kaleidoscope-nonempty.bin.part',
                  'write Downloads/.kaleidoscope-nonempty.bin.part x',
                  'touch Kaleidoscope/empty-parts/keep.txt'):
   g['focus'](153);time.sleep(.3);s=len(t());keys(command);q.key('ret')
   g['wait'](r'\[spindle.fs.line\] ok',s,90)
  g['stop']()
  g=boot(a.iso,disk,a.out,'download-powercut',a.resolution);t=g['text'];q=g['q']
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  g['focus'](153);time.sleep(.5);s=len(t());keys('run +net kaleidoscope about:downloads');q.key('ret')
  g['wait'](r'\[web.download.empty_parts.cleaned\] count=1',s,300)
  g['focus'](153);time.sleep(.3);s=len(t());keys('ls Downloads');q.key('ret')
  g['wait'](r'\[spindle.fs.line\] \.kaleidoscope-nonempty\.bin\.part  2',s,90)
  assert '.kaleidoscope-reboot.bin.part' not in t()[s:],t()[s:]
  g['focus'](153);time.sleep(.3);s=len(t());keys('ls Kaleidoscope/empty-parts');q.key('ret')
  g['wait'](r'\[spindle.fs.line\] keep\.txt  0',s,90)
  assert 'reboot.bin.ticket' not in t()[s:] and 'stale.bin.ticket' not in t()[s:] and 'nonempty.bin.ticket' not in t()[s:],t()[s:]
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'fnv':m.group(0)})
 finally:g['stop']()
if __name__=='__main__':main()
