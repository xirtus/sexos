#!/usr/bin/env python3
"""WebSocket (RFC 6455) from a page: the handshake with a chosen subprotocol, text
(UTF-8), binary and a 70000-byte message echoed by a local server, a server ping
answered with a pong, and a clean close both ways."""
import argparse,base64,hashlib,http.server,os,shutil,struct,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
GUID=b'258EAFA5-E914-47DA-95CA-C5AB0DC85B11'
events=[]
def read_frame(f):
 h=f.read(2);b0,b1=h[0],h[1];n=b1&127
 if n==126:n=struct.unpack('>H',f.read(2))[0]
 elif n==127:n=struct.unpack('>Q',f.read(8))[0]
 assert b1&128,'client frames are masked'
 m=f.read(4);d=bytearray(f.read(n))
 for i in range(n):d[i]^=m[i%4]
 return b0&15,bytes(d)
def frame(op,data):
 n=len(data);h=bytes([128|op])+(bytes([n])if n<126 else bytes([126])+struct.pack('>H',n)if n<65536 else bytes([127])+struct.pack('>Q',n))
 return h+data
class Handler(http.server.BaseHTTPRequestHandler):
 protocol_version='HTTP/1.1'
 def do_GET(self):
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
  if self.path=='/site/echo':return self.echo()
  f=SITE/self.path.split('?')[0].removeprefix('/site/')
  if not self.path.startswith('/site/') or not f.is_file():self.send_response(404);self.send_header('Content-Length','0');self.end_headers();return
  body=f.read_bytes();self.send_response(200);self.send_header('Content-Type','text/html; charset=utf-8');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def echo(self):
  assert self.headers['Upgrade'].lower()=='websocket' and self.headers['Sec-WebSocket-Version']=='13'
  events.append('protocols '+self.headers.get('Sec-WebSocket-Protocol',''))
  accept=base64.b64encode(hashlib.sha1(self.headers['Sec-WebSocket-Key'].encode()+GUID).digest()).decode()
  self.send_response(101);self.send_header('Upgrade','websocket');self.send_header('Connection','Upgrade');self.send_header('Sec-WebSocket-Accept',accept);self.send_header('Sec-WebSocket-Protocol','echo');self.end_headers();self.wfile.flush()
  self.wfile.write(frame(9,b'pi'));self.wfile.flush()
  while True:
   op,d=read_frame(self.rfile)
   if op==10:events.append('pong '+d.decode());continue
   if op==8:events.append('close '+str(struct.unpack('>H',d[:2])[0])+' '+d[2:].decode());self.wfile.write(frame(8,d[:2]));self.wfile.flush();self.close_connection=True;return
   self.wfile.write(frame(op,d));self.wfile.flush()
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
os.environ.setdefault('SEXOS_GATE_BOUND','900')
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'websocket',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch http://10.0.2.2:{PORT}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope http://10.0.2.2:{PORT}/site/websocket.html');q.key('ret')
  g['wait'](r'\[web.console\] level=0 ws bad scheme SyntaxError',s,300)
  g['wait'](r'\[web.console\] level=0 ws open echo 1',s,120)
  g['wait'](r'\[web.console\] level=0 ws text hello ünïcode',s,60)
  g['wait'](r'\[web.console\] level=0 ws binary 1,2,3,250',s,60)
  g['wait'](r'\[web.console\] level=0 ws text big true',s,60)
  m=g['wait'](r'\[web.console\] level=0 ws close (\d+) (\w+) ',s,60);assert m[1]=='1000' and m[2]=='true',m.group(0)
  assert 'protocols echo, other' in events and 'pong pi' in events and 'close 1000 bye' in events,events
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text and 'ws error' not in text
  print({'ok':True,'events':events})
 finally:g['stop']()
if __name__=='__main__':main()
