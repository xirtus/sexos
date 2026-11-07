#!/usr/bin/env python3
"""The Browser opens real addresses: it loads a page, follows a link by number, goes back and refuses a bad certificate."""
import argparse,http.server,os,re,shutil,ssl,subprocess,tempfile,threading,time
from pathlib import Path
PAGE=b'<html><head><title>Test</title></head><body><h1>Front page</h1><p>Hello from a real page.</p><a href="/second.html">second</a> <a href="third.html">third</a></body></html>'
SECOND=b'<html><body><h1>Second page</h1><p>You followed a link.</p></body></html>'
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/moved':self.send_response(302);self.send_header('Location','/second.html');self.end_headers();return
  body={'/':PAGE,'/second.html':SECOND}.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1];threading.Thread(target=server.serve_forever,daemon=True).start()
tmp=tempfile.mkdtemp();key,crt=tmp+'/k.pem',tmp+'/c.pem'
subprocess.run(['openssl','req','-x509','-newkey','ec','-pkeyopt','ec_paramgen_curve:prime256v1','-nodes','-keyout',key,'-out',crt,'-subj','/CN=10.0.2.2','-days','2'],check=True,capture_output=True)
bad=Quiet(('127.0.0.1',0),Handler);c=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);c.load_cert_chain(crt,key);bad.socket=c.wrap_socket(bad.socket,server_side=True);BAD=bad.server_address[1];threading.Thread(target=bad.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');p.add_argument('--public',default='');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 g=boot(a.iso,disk,a.out,'web',a.resolution);t=g['text'];q=g['q']
 def typed(text):
  for k in text:
   shifted={':':'semicolon','_':'minus','?':'slash'}.get(k) or (k.lower() if k.isupper() else None)
   if shifted:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':shifted}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def address(url):
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'l'}],'hold-time':80});time.sleep(.4)
  # The bar starts with the current address.
  for _ in range(150):q.key('backspace');time.sleep(.005)
  typed(url);time.sleep(1);s=len(t());q.key('ret');g['wait'](r'\[browser.web.open\] url='+re.escape(url),s,30);return s
 def page(s,timeout=180):return g['wait'](r'\[browser.web.page\] source=(\d+) text=(\d+) links=(\d+)',s,timeout)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2);g['focus'](300);time.sleep(1)
  base=f'http://10.0.2.2:{PORT}'
  s=address(base+'/');m=page(s);assert m[3]=='2',m[0]
  time.sleep(2);q.cmd('screendump',{'filename':str(a.out/'front.ppm')})
  s=len(t());q.key('1');time.sleep(.2);q.key('ret');m=page(s);assert 'url=http://10.0.2.2' in t()[s:] and '/second.html' in t()[s:],t()[s:][-300:]
  s=len(t());q.key('backspace');m=page(s);assert m[3]=='2','back did not return to the front page'
  s=address(base+'/moved');m=page(s);assert '[browser.web.status.code] 302' in t()[s:] and '[browser.web.status.code] 200' in t()[s:]
  s=address(f'https://10.0.2.2:{BAD}/');g['wait'](r"\[browser.web.status\] This site's certificate isn't trusted",s,120)
  public={}
  for url in [u for u in a.public.split(',') if u]:
   s=address(url);m=page(s,240);public[url]=m[0];time.sleep(2);q.cmd('screendump',{'filename':str(a.out/(re.sub(r'\W','_',url)+'.ppm'))});time.sleep(.5)
  assert 'KERNEL PANIC'not in t() and 'panicked' not in t()
  print({'ok':True,'public':public})
 finally:g['stop']()
if __name__=='__main__':main()
