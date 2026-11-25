#!/usr/bin/env python3
"""Chastity approves one HTTPS login fill in Kaleidoscope, after a tab switch
cancelled the previous request. Only fictional credentials are used; serial
output and artifacts must not contain the password.
"""
import argparse,http.server,os,re,shutil,ssl,threading,time,urllib.parse
from pathlib import Path
from install_program import install
from lua_gate import qcodes
from program_gate_geometry import maximize,current_tile
from program_web_tls_gate import leaf
from quil_autosave_gate import boot

ROOT=Path(__file__).resolve().parents[1]
HOST='10.0.2.2'
MASTER='testonlymasterpassword'
SECRET='autofilltestsecret'
os.environ.setdefault('SEXOS_GATE_BOUND','2400')
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'

def server(root):
 cert,key=leaf(root,'autofill','ec')
 ctx=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
 ctx.minimum_version=ssl.TLSVersion.TLSv1_3
 ctx.load_cert_chain(cert,key)
 posts=[]
 page=b'''<!doctype html><title>Login fixture</title>
 <h1>Login fixture</h1><form method=post action=/session>
 <label>User <input name=user autocomplete=username></label><br>
 <label>Password <input name=pass type=password autocomplete=current-password></label>
 <button type=submit>Sign in</button></form><script>
 let sent=false;
 setInterval(()=>{const u=document.querySelector('[name=user]'),p=document.querySelector('[name=pass]');
 if(!sent&&u.value&&p.value){sent=true;setTimeout(()=>document.querySelector('form').submit(),4000)}},200);
 </script>'''
 class H(http.server.BaseHTTPRequestHandler):
  protocol_version='HTTP/1.1'
  def do_GET(self):
   body=page if self.path=='/' else b'<title>Login complete</title><h1>Login complete</h1>'
   self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def do_POST(self):
   body=self.rfile.read(int(self.headers.get('Content-Length','0')))
   fields=urllib.parse.parse_qs(body.decode(),keep_blank_values=True)
   posts.append(fields.get('user')==['alice'] and fields.get('pass')==[SECRET])
   body=b'<title>Login complete</title><h1>Login complete</h1>'
   self.send_response(200);self.send_header('Content-Type','text/html');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*a):pass
 class S(http.server.ThreadingHTTPServer):
  daemon_threads=True
  def get_request(self):
   raw,addr=self.socket.accept();return ctx.wrap_socket(raw,server_side=True),addr
  def handle_error(self,*a):pass
 srv=S(('0.0.0.0',0),H);threading.Thread(target=srv.serve_forever,daemon=True).start()
 return srv.server_address[1],posts

def main():
 p=argparse.ArgumentParser(description=__doc__)
 for n in ('iso','source-disk','root-dir','out'):p.add_argument('--'+n,required=True,type=Path)
 p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 assert (a.root_dir/'ca.key').exists(),'root-dir must match the CI image trust root'
 port,posts=server(a.root_dir)
 url=f'https://{HOST}:{port}/'
 out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[(n,(ROOT/'iso_root/programs'/n).read_bytes())for n in ('chastity','kaleidoscope')])
 rw,rh=map(int,a.resolution.split('x'))
 g=None
 def start(tag):return boot(a.iso,disk,out,tag,a.resolution)
 def keys(text):
  for c in text:
   g['q'].cmd('send-key',{'keys':[{'type':'qcode','data':k}for k in qcodes(c)],'hold-time':40});time.sleep(.12)
 def chord(*names):g['q'].cmd('send-key',{'keys':[{'type':'qcode','data':k}for k in names],'hold-time':80})
 def launch(command,marker):
  g['focus'](153);time.sleep(.4);s=len(g['text']());keys('run '+command);g['q'].key('ret')
  m=g['wait'](marker,s,300);target,task=int(m[1]),int(m[2]);r=g['wait'](rf'\[shell.program.launch\] target={target} task={task} surface=(\d+)',s,60)
  sid=int(r[1]);g['focus'](sid);time.sleep(.6);return target,task,sid
 def click_allow(vault,token):
  t=g['text']();m=re.search(r'\[chastity.consent.control\] token='+re.escape(str(token))+r' name=allow x=(-?\d+) y=(-?\d+) enabled=1',t)
  assert m,'Allow control missing';tile=current_tile(t,vault)
  x=int(m[1])+tile[0]+8;y=int(m[2])+tile[1]+36
  g['q'].cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.2)
  for down in (True,False):g['q'].cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.12)
 try:
  g=start('autofill-save');g['wait'](r'\[app.private.retired\] target=41 ',0,300)
  _,_,vault=launch('chastity',r'\[chastity.window\] target=(\d+) task=(\d+)')
  keys(MASTER);g['q'].key('ret');time.sleep(.5);keys(MASTER);s=len(g['text']());g['q'].key('ret');g['wait'](r'\[chastity.created\] entries=0',s,900)
  g['q'].key('a');time.sleep(.5)
  for value in ('fixture','alice',SECRET,url,'fixture notes',''):
   keys(value);s=len(g['text']());g['q'].key('ret');time.sleep(.3)
  g['wait'](r'\[chastity.saved\] entries=1 bytes=\d+',s,900);g['stop']();g=None
  g=start('autofill-live');g['wait'](r'\[app.private.retired\] target=41 ',0,300)
  _,_,vault=launch('chastity',r'\[chastity.window\] target=(\d+) task=(\d+)')
  keys(MASTER);s=len(g['text']());g['q'].key('ret');g['wait'](r'\[chastity.unlocked\] entries=1 ',s,900);g['wait'](r'\[chastity.autofill.ready\]',s,120)
  tile,resize=maximize(g,vault);g['wait'](r'\[chastity.layout\] width=\d+ height=\d+',resize,60);assert tile[2]>=816 and tile[3]>=642,tile
  target,task,browser=launch(f'+net kaleidoscope {url}',r'\[web.ready\] target=(\d+) task=(\d+)')
  g['wait'](rf'\[web.http\] method=GET status=200 bytes=\d+[^\n]*url={re.escape(url)}',0,300)
  # A pending request loses its authority on a tab switch.
  s=len(g['text']());chord('ctrl','shift','k');g['wait'](r'\[web.autofill.requested\]',s,60)
  g['wait'](rf'\[chastity.autofill.prompt\] token=\d+ domain={target} task={task}',s,120)
  chord('ctrl','t');g['wait'](r'\[web.autofill.cancelled\] reason=page_changed',s,60);g['wait'](r'\[web.autofill.cleared\]',s,120)
  chord('ctrl','pgup');g['wait'](r'\[web.tab.shown\] index=0 ',s,60)
  # A typed navigation also revokes a request before approval.
  s=len(g['text']());chord('ctrl','shift','k');g['wait'](r'\[web.autofill.requested\]',s,60)
  g['wait'](rf'\[chastity.autofill.prompt\] token=\d+ domain={target} task={task}',s,120)
  chord('ctrl','l');keys('about:blank');g['q'].key('ret')
  g['wait'](r'\[web.autofill.cancelled\] reason=page_changed',s,60);g['wait'](r'\[web.autofill.cleared\]',s,120)
  s=len(g['text']());g['q'].key('backspace');g['wait'](rf'\[web.http\] method=GET status=200 bytes=\d+[^\n]*url={re.escape(url)}',s,300)
  s=len(g['text']());chord('ctrl','shift','k');g['wait'](r'\[web.autofill.requested\]',s,60)
  prompt=g['wait'](rf'\[chastity.autofill.prompt\] token=(\d+) domain={target} task={task}',s,120)
  g['focus'](vault);time.sleep(.5);g['q'].cmd('screendump',{'filename':str(out/'browser-consent.ppm')})
  g['wait'](rf'\[chastity.consent.control\] token={prompt[1]} name=allow x=\d+ y=\d+ enabled=1',s,60)
  click_allow(vault,prompt[1]);g['wait'](rf'\[chastity.autofill.approved\] token={prompt[1]}',s,60)
  g['wait'](r'\[web.autofill.filled\] fields=2',s,120)
  g['focus'](browser);time.sleep(.5);g['q'].cmd('screendump',{'filename':str(out/'browser-password-filled.ppm')})
  deadline=time.monotonic()+120
  while not posts and time.monotonic()<deadline:time.sleep(.2)
  assert posts==[True],'HTTPS form did not receive the approved credential pair'
  text=g['text']();assert SECRET not in text and MASTER not in text,'secret appeared in serial output'
  assert '[web.fatal]'not in text and 'KERNEL PANIC'not in text
  print({'ok':True,'cancelled_on_tab_switch':True,'cancelled_on_navigation':True,'approved_fill':True,'same_origin_post':True,'secret_serial_leak':False})
 finally:
  if g:g['stop']()
if __name__=='__main__':main()
