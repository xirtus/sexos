#!/usr/bin/env python3
"""Web over HTTP/1.1, the way youtube.com talks: `run +net kaleidoscope URL` loads a page sent
chunked and gzip-compressed with Set-Cookie headers; its script reads and writes
document.cookie, POSTs JSON with its own headers to an API (the cookies go along, the
HttpOnly one too, which the script itself can't see), then loads media bytes with an
XMLHttpRequest Range request (206 Partial Content). Requests to another origin carry
an Origin header and no cookies, and their replies reach the page only when that site
allows it (Access-Control-Allow-Origin). Twelve slow requests made together are
served side by side, on flows the network service makes as they are asked for, and
finished connections stay open so later requests skip a new connection (keep-alive)."""
import argparse,gzip,http.server,json,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site/tube'
MEDIA=bytes(i%251 for i in range(64*1024))
SEEN={}
LOCK=threading.Lock();LIVE=[0,0] # requests in /tube/slow now, and the most at once
COUNT={'connections':0,'requests':0} # on the page's own server: kept connections carry many requests
class Handler(http.server.BaseHTTPRequestHandler):
 protocol_version='HTTP/1.1'
 def setup(self):
  super().setup()
  with LOCK:COUNT['connections']+=1
 def parse_request(self):
  ok=super().parse_request()
  with LOCK:COUNT['requests']+=1
  return ok
 def chunked(self,body,ctype,extra=()):
  self.send_response(200);self.send_header('Content-Type',ctype);self.send_header('Transfer-Encoding','chunked');self.send_header('Content-Encoding','gzip')
  for k,v in extra:self.send_header(k,v)
  self.end_headers();z=gzip.compress(body)
  for i in range(0,len(z),700):part=z[i:i+700];self.wfile.write(b'%x\r\n'%len(part)+part+b'\r\n')
  self.wfile.write(b'0\r\n\r\n')
 def plain(self,code,body,ctype,extra=()):
  self.send_response(code);self.send_header('Content-Type',ctype);self.send_header('Content-Length',str(len(body)))
  for k,v in extra:self.send_header(k,v)
  self.end_headers();self.wfile.write(body)
 def do_GET(self):
  if self.path=='/web':return self.plain(200,(ROOT/'iso_root/programs/kaleidoscope').read_bytes(),'application/octet-stream')
  if self.path=='/tube/':
   SEEN['accept-encoding']=self.headers.get('Accept-Encoding','');SEEN['ua']=self.headers.get('User-Agent','')
   return self.chunked((SITE/'index.html').read_bytes().replace(b'{{OTHER}}',f'http://10.0.2.2:{OTHER_PORT}'.encode()),'text/html; charset=utf-8',[('Set-Cookie','sid=s3cret; Path=/; HttpOnly'),('Set-Cookie','visitor=v1; Path=/; Max-Age=3600')])
  m=re.fullmatch(r'/tube/slow/(\d+)',self.path)
  if m:
   with LOCK:LIVE[0]+=1;LIVE[1]=max(LIVE)
   time.sleep(1.5)
   with LOCK:LIVE[0]-=1
   return self.plain(200,b'slow'+m[1].encode(),'text/plain')
  if self.path=='/tube/drop':
   # Keep-alive by the headers, then close anyway, as servers do with idle connections.
   self.plain(200,b'dropped','text/plain');self.close_connection=True;return
  if self.path=='/tube/after':return self.plain(200,b'after','text/plain')
  if self.path=='/tube/media.bin':
   m=re.fullmatch(r'bytes=(\d+)-(\d+)',self.headers.get('Range',''))
   if not m:return self.plain(200,MEDIA,'application/octet-stream')
   a,b=int(m[1]),int(m[2]);return self.plain(206,MEDIA[a:b+1],'application/octet-stream',[('Content-Range',f'bytes {a}-{b}/{len(MEDIA)}'),('Accept-Ranges','bytes')])
  self.plain(404,b'not here','text/plain')
 def do_POST(self):
  n=int(self.headers.get('Content-Length','0'));body=self.rfile.read(n)
  if not self.path.startswith('/tube/api/player'):return self.plain(404,b'no','text/plain')
  j=json.loads(body)
  reply=json.dumps({'videoId':j['videoId'],'client':self.headers.get('X-Tube-Client',''),'cookie':self.headers.get('Cookie','').replace(' ',''),'media':'/tube/media.bin'}).encode()
  self.chunked(reply,'application/json')
 def log_message(self,*a):pass
class Other(http.server.BaseHTTPRequestHandler):
 # A second origin: /open allows the page's origin, /closed doesn't say.
 def do_GET(self):
  body=json.dumps({'origin':self.headers.get('Origin',''),'cookie':self.headers.get('Cookie','')}).encode()
  self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)))
  if self.path=='/open':self.send_header('Access-Control-Allow-Origin',self.headers.get('Origin',''))
  self.send_header('Set-Cookie','tracker=1; Path=/');self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
other=Quiet(('127.0.0.1',0),Other);OTHER_PORT=other.server_address[1]
for s_ in (server,other):threading.Thread(target=s_.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 base=f'http://10.0.2.2:{PORT}'
 g=boot(a.iso,disk,a.out,'webhttp11',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {base}/tube/');q.key('ret')
  page=g['wait'](r'\[web.http\] method=GET status=200 bytes=(\d+) encoding=gzip chunked=1 cookies=(\d+) ticks=\d+ url=[^\n]*/tube/\n',s,300)
  assert int(page[1])==len((SITE/'index.html').read_bytes().replace(b'{{OTHER}}',f'http://10.0.2.2:{OTHER_PORT}'.encode())),('decoded size',page[1]);assert page[2]=='2',('cookies kept',page[2])
  first=g['wait'](r'\[web.console\] level=0 tube cookie ([^\n]*)\n',s,300)
  assert first[1].strip()=='visitor=v1',('HttpOnly cookie reached the script',first[1])
  post=g['wait'](r'\[web.console\] level=0 tube post ([^\n]*)\n',s,300)
  want='200 application/json video=sexos-demo client=web cookie=sid=s3cret;visitor=v1;script_pref=f1'
  assert post[1]==want,(post[1],want)
  rng=g['wait'](r'\[web.console\] level=0 tube range ([^\n]*)\n',s,300)
  want=f'206 bytes 1000-1015/{len(MEDIA)} 16 first={1000%251} last={1015%251}'
  assert rng[1]==want,(rng[1],want)
  cross=g['wait'](r'\[web.console\] level=0 tube cross open ([^\n]*)\n',s,300)
  assert cross[1]==f'origin={base} cookie=none',cross[1]
  closed=g['wait'](r'\[web.console\] level=0 tube cross closed ([^\n]*)\n',s,300)
  assert closed[1]=='blocked',closed[1]
  par=g['wait'](r'\[web.console\] level=0 tube parallel ([^\n]*)\n',s,300)
  assert par[1].startswith(','.join(f'slow{n}' for n in range(1,13))+' ms='),par[1]
  # Twelve 1.5 s replies one after another take 18 s; side by side they overlap. The
  # network service makes flows as they are asked for, so all twelve travel at once.
  assert LIVE[1]==12,('requests at once',LIVE[1])
  beside=g['wait'](r'\[web.console\] level=0 tube beside stuck ([^\n]*)\n',s,300)
  assert beside[1].startswith('slow0 ms=') and int(beside[1].split('ms=')[1])<5000,beside[1]
  after=g['wait'](r'\[web.console\] level=0 tube after drop ([^\n]*)\n',s,240)
  assert after[1]=='after',after[1]
  assert re.search(r'\[net.kept.retry\] flow=\d+ sent=1',t()),'the closed kept connection was not retried'
  made=re.findall(r'\[net.open.flows\] made=(\d+)',t());assert made and int(made[-1])>=12,made
  assert 'gzip' in SEEN.get('accept-encoding',''),SEEN
  assert re.search(r'\[net.receive.stats\] [^\n]*reused=1',t()),'no request went out on a kept connection'
  assert COUNT['requests']>COUNT['connections'],('connections were not reused',COUNT)
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'page':page[0].strip(),'post':post[1],'range':rng[1],'cross':cross[1],'closed':closed[1],'parallel':par[1],'at_once':LIVE[1],'beside_stuck':beside[1],'reuse':COUNT,'after_drop':after[1]})
 finally:g['stop']()
if __name__=='__main__':main()
