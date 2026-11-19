#!/usr/bin/env python3
"""Gossip, the feed reader on sex-app: an OPML list naming a feed served from this host
(crates/sex-feed/tests/fixture.rss) is fetched into Spindle's folder and given to
`run +net gossip @feeds.opml`. Gossip subscribes, fetches the feed over the network
service and lists its three items unread; opening the second marks it read; the first
item's podcast episode is downloaded into Gossip's folder; the list is exported back
to OPML. On the next run the feed is fetched again and the second item is still read."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
RSS=(ROOT/'crates/sex-feed/tests/fixture.rss').read_text()
EPISODE=b'ID3 fixture audio'[:16]
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body={'/feed.xml':FILES['feed'],'/feeds.opml':FILES['opml'],'/episode3.mp3':EPISODE}.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  HITS.append(self.path)
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1];HITS=[]
BASE=f'http://10.0.2.2:{PORT}'
FILES={'feed':RSS.replace('http://10.0.2.2/',BASE+'/').encode(),
 'opml':f'<?xml version="1.0"?><opml version="1.0"><head><title>Fixture</title></head><body><outline text="Fixture"><outline type="rss" text="Fixture feed" xmlUrl="{BASE}/feed.xml"/></outline></body></opml>'.encode()}
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from install_program import install
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus',':':('shift','semicolon'),'@':('shift','2'),'+':('shift','equal')}
TITLES=['Episode 3: Lanterns & Ferries','A post with full content','No guid here']
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('gossip',(ROOT/'iso_root/programs/gossip').read_bytes())])
 g=boot(a.iso,disk,a.out,'gossip',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   v=KEYS.get(k,k)
   if isinstance(v,tuple):q.cmd('send-key',{'keys':[{'type':'qcode','data':x}for x in v],'hold-time':80})
   else:q.key(v)
   time.sleep(.08)
 def spindle(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def start(args):
  s=len(t());spindle(f'run +net gossip{args}',r'\[shell.program.launch\]')
  ready=g['wait'](r'\[gossip.ready\] target=(\d+) task=(\d+) feeds=(\d+)',s,180)
  launch=g['wait'](rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)',s,60)
  g['wait'](rf'\[display.app.pixels\] target={ready[1]} ',s,60);time.sleep(2)
  g['focus'](int(launch[1]));time.sleep(.5)
  return ready,s
 def close(ready):
  s=len(t());q.key('f11');g['wait'](rf'\[gossip.closed\] target={ready[1]} task={ready[2]}',s,60);g['wait'](rf'\[app.private.retired\] target={ready[1]} task={ready[2]} ',s,90)
 def open_feed(reads):
  s=len(t());q.key('ret')
  g['wait'](rf'\[gossip.feed\] feed={re.escape(BASE)}/feed.xml title=Gossip Fixture Radio items=3 unread={3-sum(reads)}\n',s,30)
  for i,(title,read) in enumerate(zip(TITLES,reads)):
   g['wait'](rf'\[gossip.item\] index={i} read={read} enclosure={int(i==0)} title={re.escape(title)}\n',s,30)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=spindle(f'fetch {BASE}/feeds.opml feeds.opml',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  # First run: the OPML list subscribes to the feed, which is fetched with all unread.
  ready,s=start(' @feeds.opml')
  g['wait'](r'\[gossip.loaded\] feeds=0 read=0\n',s,60);g['wait'](r'\[gossip.imported\] feeds=1 added=1\n',s,60)
  g['wait'](rf'\[gossip.fetched\] feed={re.escape(BASE)}/feed.xml status=200 items=3 unread=3\n',s,60)
  assert ready[3]=='1',ready[0]
  open_feed([0,0,0])
  # The second item opens and is read.
  q.key('down');time.sleep(.3);s=len(t());q.key('ret')
  g['wait'](r'\[gossip.read\] index=1 id=http://example.org/2 title=A post with full content\n',s,30);g['wait'](r'\[gossip.saved.read\] count=1\n',s,30)
  time.sleep(1.5);q.cmd('screendump',{'filename':str(a.out/'gossip-item.ppm')});time.sleep(.5)
  # Back to the list; the first item's episode is downloaded into the folder.
  q.key('esc');time.sleep(.3);q.key('up');time.sleep(.3);s=len(t());q.key('d')
  g['wait'](r'\[gossip.enclosure.saved\] file=episode3.mp3 bytes=16 type=audio/mpeg\n',s,60);g['wait'](r'\[gossip.enclosure.kept\] file=episode3.mp3 player=none\n',s,30)
  assert '/episode3.mp3' in HITS,HITS
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'gossip-items.ppm')});time.sleep(.5)
  # Back to the feeds; the subscriptions are exported as OPML.
  q.key('esc');time.sleep(.3);s=len(t());q.key('x');g['wait'](r'\[gossip.exported\] feeds=1 bytes=\d+\n',s,30)
  close(ready)
  # Second run: the feed is fetched again and the second item is still read.
  ready,s=start('')
  g['wait'](r'\[gossip.loaded\] feeds=1 read=1\n',s,60)
  g['wait'](rf'\[gossip.fetched\] feed={re.escape(BASE)}/feed.xml status=200 items=3 unread=2\n',s,60)
  open_feed([0,1,0])
  close(ready)
  assert HITS.count('/feed.xml')>=2,HITS
  text=t();assert 'KERNEL PANIC' not in text and '[gossip.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'requests':HITS})
 finally:g['stop']()
if __name__=='__main__':main()
