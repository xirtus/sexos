#!/usr/bin/env python3
"""Rendezvous, the maps program, in QEMU against local fixtures only: a Python server
on this host serves the committed vector tile (gzipped, as tile servers send it),
plain PNG raster tiles for every other tile (the fallback), and canned Nominatim and
OSRM answers (crates/sex-map/tests/fixtures, from scripts/make_map_fixture.py).

`run +net rendezvous http://10.0.2.2:PORT` opens on the fixture tile: it must be
decoded as a vector tile and drawn, with the river's water, the roads and the
buildings in their colours on screen (counted in a screenshot, too). Searching
`fountain` and `station` finds the places; `r` draws the route between them;
`o` keeps the area for offline use. With the server's tiles switched off, the next
run draws the same tile from the program's folder."""
import argparse,gzip,json,os,re,shutil,struct,threading,time,zlib,http.server
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
FIX=ROOT/'crates/sex-map/tests/fixtures'
TILE_GZ=(FIX/'tile-14-8800-5373.mvt.gz').read_bytes()
# sex-map's colours (crates/sex-map/src/render.rs), 0xRRGGBB.
WATER,MAJOR_ROAD,BUILDING,ROUTE=0xAAD3DF,0xFCD6A4,0xD9D0C9,0x266BE0
RASTER=(0xE4,0xDC,0xCC)
def png(w,h,rgb):
 raw=b''.join(b'\0'+bytes(rgb)*w for _ in range(h))
 chunk=lambda t,d:struct.pack('>I',len(d))+t+d+struct.pack('>I',zlib.crc32(t+d)&0xffffffff)
 return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,2,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')
RASTER_PNG=png(256,256,RASTER)
STATE={'tiles':True,'hits':[]}
class Handler(http.server.BaseHTTPRequestHandler):
 protocol_version='HTTP/1.1'
 def send(self,code,body=b'',ctype='application/octet-stream',extra=()):
  self.send_response(code);self.send_header('Content-Type',ctype);self.send_header('Content-Length',str(len(body)))
  for k,v in extra:self.send_header(k,v)
  self.send_header('Connection','close');self.end_headers();self.wfile.write(body)
 def do_GET(self):
  p=self.path;STATE['hits'].append(p)
  if p.startswith('/tiles/') or p.startswith('/raster/'):
   if not STATE['tiles']:return self.send(503,b'tiles are off')
   if p=='/tiles/14/8800/5373.mvt':return self.send(200,TILE_GZ,'application/vnd.mapbox-vector-tile',[('Content-Encoding','gzip')])
   if p.startswith('/raster/'):return self.send(200,RASTER_PNG,'image/png')
   return self.send(404,b'no such tile')
  if p.startswith('/search?'):
   q=re.search(r'[?&]q=([^&]*)',p);q=(q[1] if q else '').lower()
   name='search-fountain.json' if 'fountain' in q else 'search-station.json' if 'station' in q else None
   return self.send(200,(FIX/name).read_bytes() if name else b'[]','application/json')
  if p.startswith('/route/v1/driving/'):return self.send(200,(FIX/'route.json').read_bytes(),'application/json')
  self.send(404)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
os.environ.setdefault('SEXOS_GATE_BOUND','1200')
from install_program import install
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus',':':('shift','semicolon'),'+':('shift','equal')}
def colours(ppm):
 data=ppm.read_bytes();parts=data.split(b'\n',3);w,h=map(int,parts[1].split());px=parts[3]
 count={}
 for c in (WATER,MAJOR_ROAD,BUILDING,ROUTE):
  needle=bytes(((c>>16)&255,(c>>8)&255,c&255));n=0;i=px.find(needle)
  while i!=-1:
   if i%3==0:n+=1
   i=px.find(needle,i+1)
  count[c]=n
 return count
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('rendezvous',(ROOT/'iso_root/programs/rendezvous').read_bytes())])
 g=boot(a.iso,disk,a.out,'rendezvous',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   v=KEYS.get(k,k)
   if isinstance(v,tuple):q.cmd('send-key',{'keys':[{'type':'qcode','data':x}for x in v],'hold-time':80})
   else:q.key(v)
   time.sleep(.08)
 def start(args):
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net rendezvous{args}');q.key('ret')
  ready=g['wait'](r'\[rendezvous.ready\] target=(\d+) task=(\d+) vector=(\S+) search=\S+ route=\S+ start=\S+ offline_regions=(\d+)\n',s,600)
  launch=g['wait'](rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)',s,60)
  return ready,int(launch[1]),s
 def frame(s,timeout=300,pred=lambda m:True):
  end=time.time()+timeout;pos=s
  while True:
   m=g['wait'](r'\[rendezvous.frame\] zoom=\S+ center=\S+ tiles=(\d+) vector=(\d+) raster=(\d+) water=(\d+) road=(\d+) building=(\d+) route=(\d+) labels=(\d+) markers=(\d+)\n',pos,max(5,end-time.time()))
   d=dict(zip(('tiles','vector','raster','water','road','building','route','labels','markers'),map(int,m.groups())))
   if pred(d):return d
   pos=t().find(m[0],pos)+len(m[0])
 def close(r,sid):
  g['focus'](sid);time.sleep(.5);s=len(t());q.key('f11');g['wait'](r'\[rendezvous.closed\]\n',s,60);g['wait'](rf'\[app.private.retired\] target={r[1]} task={r[2]} ',s,180)
 def search(sid,word,results,first):
  g['focus'](sid);time.sleep(.3);s=len(t());keys('/'+word);q.key('ret')
  g['wait'](rf'\[rendezvous.search\] query={word} results={results} first={re.escape(first)}\n',s,120)
  time.sleep(.5);s=len(t());q.key('ret');return g['wait'](rf'\[rendezvous.place\] name={re.escape(first)} lat=(\S+) lon=(\S+)\n',s,30)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  r,sid,s=start(f' http://10.0.2.2:{PORT}')
  assert r[3]==f'http://10.0.2.2:{PORT}/tiles/{{z}}/{{x}}/{{y}}.mvt' and r[4]=='0',r[0]
  g['wait'](r'\[rendezvous.tile\] id=14/8800/5373 kind=vector source=network layers=8 features=13\n',s,300)
  g['wait'](r'\[rendezvous.tile\] id=14/\d+/\d+ kind=raster source=network size=256x256\n',s,300)
  # The whole desktop for the map (Super+M).
  s2=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'meta_l'},{'type':'qcode','data':'m'}],'hold-time':80})
  g['wait'](r'\[shell.wm.action\] action=ToggleMonocle done=1',s2,30)
  f=frame(s2,300,lambda d:d['vector']==1 and d['raster']>0)
  assert f['water']>5000 and f['road']>1000 and f['building']>1000 and f['labels']>=2,f
  time.sleep(1);shot=a.out/'rendezvous-map.ppm';q.cmd('screendump',{'filename':str(shot)});time.sleep(1)
  c=colours(shot);assert c[WATER]>5000 and c[MAJOR_ROAD]>1000 and c[BUILDING]>1000,c
  search(sid,'fountain',1,'Fountain Square')
  search(sid,'station',2,'Old Mill Station')
  s=len(t());q.key('r')
  g['wait'](r'\[rendezvous.route\] points=4 distance=812 duration=95\n',s,120)
  f=frame(s,120,lambda d:d['route']>0)
  assert f['route']>500 and f['markers']==2,f
  time.sleep(1);shot2=a.out/'rendezvous-route.ppm';q.cmd('screendump',{'filename':str(shot2)});time.sleep(1)
  c2=colours(shot2);assert c2[ROUTE]>500,c2
  s=len(t());q.key('o')
  o=g['wait'](r'\[rendezvous.offline\] region=\S+ tiles=(\d+) kept=(\d+) zooms=13-14\n',s,600)
  assert int(o[1])==int(o[2])>=2,o[0]
  close(r,sid)
  # The server's tiles go away; the kept region draws from the program's folder.
  STATE['tiles']=False;hits=len(STATE['hits'])
  r,sid,s=start('')
  assert r[4]=='1',r[0]
  g['wait'](r'\[rendezvous.tile\] id=14/8800/5373 kind=vector source=offline layers=8 features=13\n',s,300)
  f=frame(s,300,lambda d:d['vector']==1)
  assert f['water']+f['road']+f['building']>2000,f
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'rendezvous-offline.ppm')});time.sleep(1)
  close(r,sid)
  text=t();assert 'KERNEL PANIC' not in text and 'panicked' not in text
  print({'ok':True,'map':c,'route':c2[ROUTE],'offline_tiles':int(o[2])})
 finally:g['stop']()
if __name__=='__main__':main()
