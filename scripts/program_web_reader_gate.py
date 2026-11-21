#!/usr/bin/env python3
"""Kaleidoscope reader view (N5): a news-style page (scripts/web_site/reader/article.html)
opens in Kaleidoscope; Ctrl+Shift+R shows its article on a quiet page of its own, with
its picture from the site and without the menu, side bar, share box, comments, footer
or script; Ctrl+Shift+R again goes back to the page."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from PIL import Image
ROOT=Path(__file__).resolve().parents[1]
SITE=ROOT/'scripts/web_site'
hits=[]
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  hits.append(self.path)
  if self.path=='/web':body=(ROOT/'iso_root/programs/kaleidoscope').read_bytes()
  else:
   f=SITE/self.path.removeprefix('/site/') if self.path.startswith('/site/') else None
   if f is None or not f.is_file():self.send_response(404);self.send_header('Content-Length','0');self.end_headers();return
   body=f.read_bytes()
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','=':'equal'}
SHIFTED={':':'semicolon','?':'slash','+':'equal','_':'minus'}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 base=f'http://10.0.2.2:{PORT}';page=f'{base}/site/reader/article.html'
 g=boot(a.iso,disk,a.out,'reader',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   if k in SHIFTED:q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':SHIFTED[k]}],'hold-time':80})
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def reader_key():
  s=len(t());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}} for dn,ks in((True,('ctrl','shift','r')),(False,('r','shift','ctrl'))) for x in ks]});return s
 def shot(name):
  f=a.out/f'{name}.ppm';q.cmd('screendump',{'filename':str(f)});time.sleep(.5)
  img=Image.open(f).convert('RGB');img.save(a.out/f'{name}.png');return img
 loaded=r'\[web.loaded\] bytes_ok=1 '
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'fetch {base}/web /bin/kaleidoscope');q.key('ret')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,240);assert m[1].startswith('Saved'),m[1]
  g['focus'](153);time.sleep(.5);s=len(t());keys(f'run +net kaleidoscope {page}');q.key('ret')
  g['wait'](loaded,s,1200);g['wait'](r'\[web.ready\] target=(\d+) task=(\d+)',s,60);time.sleep(2)
  hits.clear();s=reader_key()
  m=g['wait'](r'\[web.reader\] words=(\d+) title=([^\n]*) url=(\S+)',s,300)
  words,title,url=int(m[1]),m[2].strip(),m[3]
  assert 60<words<120 and title=='Why moths fly towards light' and url==page,(words,title,url)
  g['wait'](r'\[web.loaded\] bytes_ok=1 [^\n]*images=1 ',s,120);time.sleep(2)
  assert '/site/reader/moth.png' in hits,hits
  img=shot('reader')
  # The reader page's warm background fills the view.
  cream=sum(1 for px in img.get_flattened_data() if px==(251,248,241));assert cream>50000,cream
  s=reader_key();g['wait'](loaded,s,300)
  back=t()[s:];assert '[web.reader]' not in back
  time.sleep(2);img=shot('page');assert sum(1 for px in img.get_flattened_data() if px==(251,248,241))<cream/10
  text=t();assert 'KERNEL PANIC' not in text and '[web.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'words':words,'title':title,'cream_pixels':cream})
 finally:g['stop']()
if __name__=='__main__':main()
