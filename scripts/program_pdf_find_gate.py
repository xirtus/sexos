#!/usr/bin/env python3
"""PDF finds text: `run pdf @find10.pdf` opens a ten-page document the person gave it;
Ctrl+F, typing ZANZIBAR and Enter turns to page 7, the only page holding the word, and
lights it up; Ctrl+G comes back round to the same single hit. The outline panel lists the
document's four bookmarks and Enter on one turns to its page; the thumbnail strip
renders small pages; F fits the whole page and + zooms in from there.

The document is scripts/make_pdf_fixture.py's, served to the system over HTTP and
fetched into the person's folder; the viewer is installed in the disk program store."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from install_program import install
from make_pdf_fixture import document,UNIQUE,UNIQUE_PAGE,PAGES
ROOT=Path(__file__).resolve().parents[1]
FILES={'/find10.pdf':document()}
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  body=FILES.get(self.path)
  if body is None:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from quil_autosave_gate import boot
# Ten pages' text read and several pages rendered under emulation.
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 pdf=(ROOT/'iso_root/programs/pdf').read_bytes();assert pdf[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('pdf',pdf)])
 g=boot(a.iso,disk,a.out,'pdf-find',a.resolution);t=g['text'];q=g['q']
 def shifted(k):q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':k}],'hold-time':80})
 def keys(text):
  for k in text:
   if k=='@':shifted('2')
   elif k==':':shifted('semicolon')
   elif k.isupper():shifted(k.lower())
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def line(text):
  g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def press(k,pattern,timeout=300):
  s=len(t());q.key(k);return g['wait'](pattern,s,timeout)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  s=len(t());line(f'fetch http://10.0.2.2:{PORT}/find10.pdf find10.pdf')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,300);assert m[1].startswith('Saved'),m[1]
  s=len(t());line('run pdf @find10.pdf')
  o=g['wait'](rf'\[pdf\.open\] target=(\d+) task=(\d+) name=find10\.pdf pages={PAGES}\n',s,600)
  g['wait'](rf'\[pdf\.page\] page=1 pages={PAGES} width=(\d+) height=(\d+) ink=(\d+) ms=(\d+)\n',s,900)
  launch=re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])
  sid=int(launch[-1]) if launch else SURFACE[int(o[1])]
  time.sleep(1);g['focus'](sid);time.sleep(.5)
  # Find: Ctrl+F, the word in lower case (search ignores case), Enter.
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'f'}],'hold-time':80});time.sleep(.5)
  keys(UNIQUE.lower());q.key('ret')
  text=g['wait'](rf'\[pdf\.text\] pages={PAGES} chars=(\d+) ms=(\d+)\n',s,900)
  assert int(text[1])>PAGES*100,f'too little text read: {text[0]}'
  hit=g['wait'](rf'\[pdf\.find\] term={UNIQUE.lower()} page={UNIQUE_PAGE} hits=1 index=1\n',s,300)
  g['wait'](rf'\[pdf\.page\] page={UNIQUE_PAGE} pages={PAGES} width=\d+ height=\d+ ink=(\d+) ms=\d+\n',s,900)
  time.sleep(1.5);q.cmd('screendump',{'filename':str(a.out/'pdf-find.ppm')})
  # Ctrl+G (Shell keeps F3): the only hit again, after going round.
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'g'}],'hold-time':80})
  g['wait'](rf'\[pdf\.find\] term={UNIQUE.lower()} page={UNIQUE_PAGE} hits=1 index=1\n',s,300)
  # A word on every page: Enter steps to the next page's hit.
  s=len(t());q.key('esc');time.sleep(.3);q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'f'}],'hold-time':80})
  for _ in range(len(UNIQUE)):q.key('backspace');time.sleep(.04)
  keys('lantern');q.key('ret')
  g['wait'](rf'\[pdf\.find\] term=lantern page={UNIQUE_PAGE} hits={PAGES} index={UNIQUE_PAGE}\n',s,300)
  press('ret',rf'\[pdf\.find\] term=lantern page={UNIQUE_PAGE+1} hits={PAGES} index={UNIQUE_PAGE+1}\n')
  s=len(t());q.key('esc');time.sleep(.3)
  # The outline: four entries; Up to the first, Enter turns to page 1.
  press('o',r'\[pdf\.outline\] shown=1 entries=4\n')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-outline.ppm')})
  for _ in range(4):q.key('up');time.sleep(.1)
  press('ret',r'\[pdf\.goto\] page=1 via=outline title=Introduction\n')
  g['wait'](rf'\[pdf\.page\] page=1 pages={PAGES} ',s,600)
  q.key('o');time.sleep(.3)
  # Thumbnails: the strip renders the pages that fit it.
  thumbs=press('t',r'\[pdf\.thumbs\] rendered=(\d+) cached=(\d+) first=1\n',600)
  assert int(thumbs[1])>=2,thumbs[0]
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-thumbs.ppm')})
  q.key('t');time.sleep(.3)
  # Fit the page, then one zoom step up from that scale.
  fit=press('f',r'\[pdf\.zoom\] mode=page pct=(\d+)\n')
  zoomed=press('equal',r'\[pdf\.zoom\] mode=fixed pct=(\d+)\n')
  assert int(zoomed[1])>int(fit[1]),(fit[0],zoomed[0])
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  g['wait'](rf'\[app\.private\.retired\] target={o[1]} task={o[2]} ',s,180)
  assert 'KERNEL PANIC' not in t() and '[std.abort]' not in t() and 'panicked' not in t()
  print({'ok':True,'chars':int(text[1]),'text_ms':int(text[2]),'thumbs':int(thumbs[1]),'fit_pct':int(fit[1]),'zoom_pct':int(zoomed[1])})
 finally:g['stop']()
if __name__=='__main__':main()
