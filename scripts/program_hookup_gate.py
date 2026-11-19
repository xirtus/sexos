#!/usr/bin/env python3
"""Hookup, the phone link (KDE Connect protocol), in QEMU against a test peer on this
host (scripts/hookup_peer.py playing the phone):

`run +net hookup 10.0.2.2:UDP @note.txt` announces itself to the peer by UDP; the
peer links to its TCP port (forwarded by QEMU), asks to pair, and the gate accepts
with `y`. The peer then sends a notification, its clipboard and a media player, and
shares a 150 KB file that must land in Hookup's received/ folder with the same bytes
(Hookup reads it back from disk). Space sends play/pause to the phone's player and
`s` offers note.txt, which the peer fetches and compares. On a second run the device
is still trusted: it links already paired."""
import argparse,os,re,shutil,socket,sys,threading,time,http.server
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'scripts'))
from hookup_peer import Peer,fnv
NOTE=b'Hookup sends this note back to the phone.\n'*200
PHOTO=bytes(((i*2654435761)>>13)&0xff for i in range(150_000))
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path!='/note.txt':self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(NOTE)));self.end_headers();self.wfile.write(NOTE)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
def free_port():
 with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
LINK,PAYLOAD=free_port(),free_port()
os.environ['SEXOS_GATE_NIC']=f'-netdev user,id=n0,hostfwd=tcp:127.0.0.1:{LINK}-:1716,hostfwd=tcp:127.0.0.1:{PAYLOAD}-:1739 -device e1000e,netdev=n0'
os.environ.setdefault('SEXOS_GATE_BOUND','1200')
from install_program import install
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus',':':('shift','semicolon'),'@':('shift','2'),'+':('shift','equal')}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('hookup',(ROOT/'iso_root/programs/hookup').read_bytes())])
 log=open(a.out/'peer.log','w')
 peer=Peer(0,{1716:LINK,1739:PAYLOAD},lambda e:(log.write(repr(e)+'\n'),log.flush()))
 g=boot(a.iso,disk,a.out,'hookup',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   v=KEYS.get(k,k)
   if isinstance(v,tuple):q.cmd('send-key',{'keys':[{'type':'qcode','data':x}for x in v],'hold-time':80})
   else:q.key(v)
   time.sleep(.08)
 def spindle(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 phone={}
 def run_phone(first):
  try:
   phone['hook']=peer.link(600);phone['paired']=peer.pair(240)
   if first:
    peer.send_extras();done=peer.share('photo.jpg',PHOTO)
    phone['taken']=done.wait(180)
    phone['media']=peer.wait_media(240)
    phone['file']=peer.wait_file(240)
   phone['ok']=True
  except Exception as e:phone['error']=repr(e)
 def start(first):
  phone.clear();th=threading.Thread(target=run_phone,args=(first,),daemon=True);th.start()
  s=len(t());spindle(f'run +net hookup 10.0.2.2:{peer.udp_port} @note.txt',r'\[shell.program.launch\]')
  ready=g['wait'](r'\[hookup.ready\] target=(\d+) task=(\d+) device=(\w+) offers=1\n',s,600)
  launch=g['wait'](rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)',s,60)
  g['wait'](rf'\[hookup.announced\] to=10\.0\.2\.2:{peer.udp_port} ok=1\n',s,120)
  return ready[1],ready[2],int(launch[1]),s,th
 def close(target,task,sid):
  g['focus'](sid);time.sleep(.5);s=len(t());q.key('f11');g['wait'](r'\[hookup.closed\]\n',s,60);g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=spindle(f'fetch http://10.0.2.2:{PORT}/note.txt note.txt',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  target,task,sid,s,th=start(True)
  g['wait'](r'\[hookup.linked\] device=peer0phone0test0device0id0000001 name=Test Phone type=phone protocol=7 incoming=1 encrypted=0 paired=0 from=10\.0\.2\.2\n',s,300)
  g['wait'](r'\[hookup.pair.asked\] device=peer0phone0test0device0id0000001 name=Test Phone\n',s,120)
  time.sleep(1);g['focus'](sid);time.sleep(.5);q.key('y')
  g['wait'](r'\[hookup.paired\] device=peer0phone0test0device0id0000001 name=Test Phone\n',s,60)
  g['wait'](r'\[hookup.notification\] app=Chat title=Ana text=Meet at the fountain at 8 cancel=0\n',s,60)
  g['wait'](r'\[hookup.clipboard\] bytes=19 text=copied on the phone\n',s,60)
  g['wait'](r'\[hookup.media\] player=Radio title=Night Drive artist=The Hookups playing=1\n',s,60)
  r=g['wait'](r'\[hookup.received\] name=photo\.jpg bytes=(\d+) disk=(\d+) fnv=([0-9a-f]{16})\n',s,300)
  assert int(r[1])==int(r[2])==len(PHOTO) and r[3]==fnv(PHOTO),(r[0],len(PHOTO),fnv(PHOTO))
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'hookup.ppm')});time.sleep(.5)
  g['focus'](sid);time.sleep(.5);q.key('spc')
  g['wait'](r'\[hookup.media.sent\] action=PlayPause player=Radio\n',s,30)
  time.sleep(1);q.key('s')
  g['wait'](r'\[hookup.offer\] path=@/[^\n]*note\.txt ok=1\n',s,30)
  g['wait'](rf'\[hookup.sent\] name=note\.txt bytes={len(NOTE)}\n',s,240)
  th.join(120)
  assert phone.get('ok'),phone
  assert phone['paired'] and phone['taken'],phone
  assert phone['media']['action']=='PlayPause' and phone['media']['player']=='Radio',phone['media']
  name,data=phone['file'];assert name=='note.txt' and data==NOTE,(name,len(data))
  assert 'kdeconnect.share.request' in phone['hook']['incomingCapabilities'],phone['hook']
  close(target,task,sid)
  # Trusted is kept in Hookup's folder: the next run links already paired.
  target,task,sid,s,th=start(False)
  g['wait'](r'\[hookup.linked\] device=peer0phone0test0device0id0000001 name=Test Phone type=phone protocol=7 incoming=1 encrypted=0 paired=1 ',s,300)
  th.join(120);assert phone.get('ok') and phone['paired'],phone
  close(target,task,sid)
  text=t();assert 'KERNEL PANIC' not in text and '[hookup.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'received':len(PHOTO),'sent':len(NOTE)})
 finally:g['stop']()
if __name__=='__main__':main()
