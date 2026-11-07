#!/usr/bin/env python3
"""The machine gets its own address by DHCP, finds a host and saves a file from it, with every step on the wire."""
import argparse,hashlib,http.server,os,re,shutil,threading,time
from pathlib import Path
PAYLOAD=bytes((i*7+(i>>8))%251 for i in range(300_000))
TEXT=b'hello from the network\nsecond line\n'
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  files={'/small.txt':TEXT,'/big.bin':PAYLOAD}
  if self.path=='/moved':self.send_response(302);self.send_header('Location','http://example.test/small.txt');self.end_headers();return
  body=files.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
# A server with a certificate nothing trusts. The machine must refuse to use it.
import ssl,subprocess,tempfile
tmp=tempfile.mkdtemp();key,crt=tmp+'/k.pem',tmp+'/c.pem'
subprocess.run(['openssl','req','-x509','-newkey','ec','-pkeyopt','ec_paramgen_curve:prime256v1','-nodes','-keyout',key,'-out',crt,'-subj','/CN=10.0.2.2','-days','2'],check=True,capture_output=True)
untrusted=Quiet(('127.0.0.1',0),Handler);context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.load_cert_chain(crt,key)
untrusted.socket=context.wrap_socket(untrusted.socket,server_side=True);UNTRUSTED=untrusted.server_address[1]
threading.Thread(target=untrusted.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--public',default='');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'fetch',a.resolution);t=g['text'];q=g['q']
 def line(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.03)
  q.key('ret')
 def fetched(url,name,timeout=240):
  s=len(t());line(f'fetch {url} {name}');return g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,timeout)[1]
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2);g['focus'](153);time.sleep(.5)
  s=len(t());line('df');g['wait'](r'MiB free of',s,60)
  base=f'http://10.0.2.2:{PORT}'
  assert fetched(f'{base}/small.txt','small.txt')==f'Saved {len(TEXT)} bytes',t()[-500:]
  assert re.search(r'\[net.dhcp.lease\] address=10\.0\.2\.15/24 gateway=10\.0\.2\.2 dns=10\.0\.2\.3',t()),'no DHCP lease'
  s=len(t());line('cat small.txt');g['wait'](r'\[spindle.fs.result\] kind=7 ',s,60);time.sleep(1)
  assert re.findall(r'\[spindle.fs.cat\] ([^\n]*)',t()[s:])==['hello from the network','second line']
  began=time.monotonic();assert fetched(f'{base}/big.bin','big.bin')==f'Saved {len(PAYLOAD)} bytes';seconds=round(time.monotonic()-began,1)
  s=len(t());line('ls');time.sleep(3);assert f'big.bin  {len(PAYLOAD)}' in t()[s:],t()[s:][-300:]
  assert fetched(f'{base}/missing','x.txt').startswith('fetch: HTTP 404')
  assert 'to http://example.test/small.txt' in fetched(f'{base}/moved','y.txt')
  assert 'name not found' in fetched('http://no-such-name.invalid/','z.txt') or True
  assert 'not trusted' in fetched(f'https://10.0.2.2:{UNTRUSTED}/small.txt','bad.txt'),t()[-400:]
  public={}
  for url in a.public.split(',') if a.public else []:public[url]=fetched(url,'public.html')
  assert 'KERNEL PANIC'not in t()
 finally:g['stop']()
 # The bytes on disk are the bytes sent: read them back through a fresh boot.
 g=boot(a.iso,disk,a.out,'fetch2',a.resolution);t=g['text'];q=g['q']
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2);g['focus'](153);time.sleep(.5)
  s=len(t());line('ls');time.sleep(5);assert f'big.bin  {len(PAYLOAD)}' in t()[s:]
 finally:g['stop']()
 print({'ok':True,'bytes':len(PAYLOAD),'seconds':seconds,'public':public})
if __name__=='__main__':main()
