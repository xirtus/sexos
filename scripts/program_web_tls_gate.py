#!/usr/bin/env python3
"""HTTPS the way public sites offer it, through the network service's TLS: `run +net web`
loads a page from a TLS 1.3-only server, and the page's script fetches from two TLS
1.2-only servers held to one suite and one curve each (ECDHE-RSA-AES256-GCM-SHA384
on P-384 with an RSA certificate; ECDHE-ECDSA-CHACHA20-POLY1305 on P-256). Each
connection's agreed version, suite and group are read back from the service's log.

The servers' certificates chain to a root made for this gate. The image must be built
with that root (`--make-root DIR` writes it, then build with
SEXOS_TLS_GATE_ROOT="$(cat DIR/ca.hex)"); public images trust only the public roots."""
import argparse,http.server,os,re,shutil,ssl,subprocess,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
HOST='10.0.2.2'

def openssl(d,*args):subprocess.run(['openssl',*args],cwd=d,check=True,capture_output=True)
def make_root(d):
 d.mkdir(parents=True,exist_ok=True)
 openssl(d,'req','-x509','-newkey','ec','-pkeyopt','ec_paramgen_curve:P-256','-nodes','-keyout','ca.key','-out','ca.pem','-days','3650','-subj','/CN=SexOS TLS gate root','-addext','basicConstraints=critical,CA:TRUE','-addext','keyUsage=critical,keyCertSign')
 openssl(d,'x509','-in','ca.pem','-outform','DER','-out','ca.der')
 (d/'ca.hex').write_text((d/'ca.der').read_bytes().hex())
def leaf(d,name,key):
 (d/'leaf.ext').write_text(f'subjectAltName=IP:{HOST}\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n')
 new=['-newkey','rsa:2048'] if key=='rsa' else ['-newkey','ec','-pkeyopt','ec_paramgen_curve:P-256']
 openssl(d,'req',*new,'-nodes','-keyout',f'{name}.key','-out',f'{name}.csr','-subj',f'/CN={name}')
 openssl(d,'x509','-req','-in',f'{name}.csr','-CA','ca.pem','-CAkey','ca.key','-CAcreateserial','-out',f'{name}.pem','-days','30','-extfile','leaf.ext')
 return d/f'{name}.pem',d/f'{name}.key'

PAGE='''<!doctype html><html><head><title>tls gate</title></head><body><p>secure</p><script>
async function go(){
 for(const [n,u] of [['bee','{{B}}'],['sea','{{C}}']]){
  try{const r=await fetch(u);console.log('tls '+n+' '+r.status+' '+(await r.text()));}
  catch(e){console.log('tls '+n+' error '+e);}
 }
}
go();
</script></body></html>'''
def server(cert,key,body,ctype,versions,ciphers=None,curve=None):
 c=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);c.minimum_version=c.maximum_version=versions;c.load_cert_chain(cert,key)
 if ciphers:c.set_ciphers(ciphers)
 if curve:c.set_ecdh_curve(curve)
 class H(http.server.BaseHTTPRequestHandler):
  protocol_version='HTTP/1.1'
  def do_GET(self):
   b=body() if callable(body) else body
   self.send_response(200);self.send_header('Content-Type',ctype);self.send_header('Content-Length',str(len(b)));self.send_header('Access-Control-Allow-Origin','*');self.end_headers();self.wfile.write(b)
  def log_message(self,*a):pass
 class S(http.server.ThreadingHTTPServer):
  daemon_threads=True
  def get_request(self):
   raw,addr=self.socket.accept();return c.wrap_socket(raw,server_side=True),addr
  def handle_error(self,*a):pass
 s=S(('0.0.0.0',0),H);threading.Thread(target=s.serve_forever,daemon=True).start();return s.server_address[1]

KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--make-root',type=Path);p.add_argument('--root-dir',type=Path);p.add_argument('--iso',type=Path);p.add_argument('--source-disk',type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 if a.make_root:make_root(a.make_root);print(a.make_root/'ca.hex');return
 d=a.root_dir;assert d and (d/'ca.key').exists(),'--root-dir must hold the root the image was built with'
 ec=leaf(d,'ec','ec');rsa=leaf(d,'rsa','rsa')
 pb=server(*rsa,b'bee','text/plain',ssl.TLSVersion.TLSv1_2,'ECDHE-RSA-AES256-GCM-SHA384','secp384r1')
 pc=server(*ec,b'sea','text/plain',ssl.TLSVersion.TLSv1_2,'ECDHE-ECDSA-CHACHA20-POLY1305','prime256v1')
 page=PAGE.replace('{{B}}',f'https://{HOST}:{pb}/b').replace('{{C}}',f'https://{HOST}:{pc}/c').encode()
 pa=server(*ec,page,'text/html; charset=utf-8',ssl.TLSVersion.TLSv1_3)
 # The program itself comes over plain HTTP, as in the other web gates.
 class Bin(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   b=(ROOT/'iso_root/programs/web').read_bytes();self.send_response(200);self.send_header('Content-Length',str(len(b)));self.end_headers();self.wfile.write(b)
  def log_message(self,*a):pass
 bins=http.server.ThreadingHTTPServer(('127.0.0.1',0),Bin);threading.Thread(target=bins.serve_forever,daemon=True).start()
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 from quil_autosave_gate import boot
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 g=boot(a.iso,disk,a.out,'webtls',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch http://{HOST}:{bins.server_address[1]}/web /bin/web');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net web https://{HOST}:{pa}/');q.key('ret')
  got=g['wait'](r'\[web.http\] method=GET status=(\d+) bytes=(\d+)[^\n]*url=https://[^\n]*:'+str(pa)+r'/\n',s,300)
  assert got[1]=='200' and int(got[2])==len(page),got[0]
  bee=g['wait'](r'\[web.console\] level=0 tls bee ([^\n]*)\n',s,300);assert bee[1]=='200 bee',bee[1]
  sea=g['wait'](r'\[web.console\] level=0 tls sea ([^\n]*)\n',s,300);assert sea[1]=='200 sea',sea[1]
  agreed=re.findall(r'\[net.tls.agreed\] flow=\d+ version=(\w+) suite=(\w+) group=Some\((\w+)\)',t()[s:])
  want={('TLSv1_2','TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384','secp384r1'),('TLSv1_2','TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256','secp256r1')}
  assert want<=set(agreed),agreed
  assert any(v=='TLSv1_3' for v,_,_ in agreed),agreed
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'page':got[0].strip(),'bee':bee[1],'sea':sea[1],'agreed':sorted(set(agreed))})
 finally:g['stop']()
if __name__=='__main__':main()
