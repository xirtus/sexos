#!/usr/bin/env python3
"""Bodice, the e-book reader on sex-app: the fixture EPUB (crates/sex-ebook/tests/
tiny.epub, made by make_fixtures.py) is fetched into Spindle's folder and given to
`run bodice @tiny.epub`, which copies it into its library and opens it on page 1.
Pages turn by keys and by a click, the contents (EPUB 3 nav) jump to a chapter, night
mode turns the page dark (checked in a screenshot's pixels), and closing keeps the
place. The next run lists the book in its library and opens it on that page, still in
night mode."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
EPUB=(ROOT/'crates/sex-ebook/tests/tiny.epub').read_bytes()
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path!='/tiny.epub':self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(EPUB)));self.end_headers();self.wfile.write(EPUB)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from install_program import install
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus',':':('shift','semicolon'),'@':('shift','2')}
DAY=(0xF4,0xEF,0xE6);NIGHT=(0x10,0x11,0x14)
def ppm(path):
 data=path.read_bytes();parts=data.split(maxsplit=4);assert parts[0]==b'P6',parts[0]
 w,h=int(parts[1]),int(parts[2]);px=parts[4][:w*h*3];return w,h,px
def count(path,rgb):
 _,_,px=ppm(path);want=bytes(rgb);return sum(1 for i in range(0,len(px),3) if px[i:i+3]==want)
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('bodice',(ROOT/'iso_root/programs/bodice').read_bytes())])
 g=boot(a.iso,disk,a.out,'bodice',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   v=KEYS.get(k,k)
   if isinstance(v,tuple):q.cmd('send-key',{'keys':[{'type':'qcode','data':x}for x in v],'hold-time':80})
   else:q.key(v)
   time.sleep(.08)
 def spindle(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def start(args):
  s=len(t());spindle(f'run bodice{args}',r'\[shell.program.launch\]')
  ready=g['wait'](r'\[bodice.ready\] target=(\d+) task=(\d+) books=(\d+) night=(\d) font=(\d+)',s,120)
  launch=g['wait'](rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)',s,60)
  g['wait'](rf'\[display.app.pixels\] target={ready[1]} ',s,60);time.sleep(2)
  g['focus'](int(launch[1]));time.sleep(.5)
  return ready,int(launch[1]),s
 def close(ready):
  s=len(t());q.key('f11');g['wait'](rf'\[bodice.closed\] target={ready[1]} task={ready[2]}',s,60);g['wait'](rf'\[app.private.retired\] target={ready[1]} task={ready[2]} ',s,90);return s
 def shot(name):
  path=a.out/name;q.cmd('screendump',{'filename':str(path)});time.sleep(1);return path
 def page(key,n):
  s=len(t());q.key(key);return g['wait'](rf'\[bodice.page\] book=tiny.epub page={n} of=\d+ chapter=(\d+) offset=(\d+)\n',s,30)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=spindle(f'fetch http://10.0.2.2:{PORT}/tiny.epub tiny.epub',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  # First run: the book is copied into the library and opens on page 1.
  ready,sid,s=start(' @tiny.epub')
  g['wait'](rf'\[bodice.imported\] book=tiny.epub bytes={len(EPUB)}\n',s,60)
  opened=g['wait'](r'\[bodice.opened\] book=tiny.epub format=epub title=A Tiny Book author=Bodice Fixture chapters=3 toc=3 pages=(\d+) page=1 saved=0\n',s,60)
  assert int(opened[1])>=4,opened[0]
  # The whole desktop for Bodice (Super+M), so clicks land on its page.
  s2=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'meta_l'},{'type':'qcode','data':'m'}],'hold-time':80})
  g['wait'](r'\[shell.wm.action\] action=ToggleMonocle done=1',s2,30);g['wait'](rf'\[bodice.layout\] book=tiny.epub cols=\d+ rows=\d+ pages=(\d+) page=1\n',s2,30)
  time.sleep(2);g['focus'](sid);time.sleep(.5)
  pages=int(re.findall(r'\[bodice.layout\] book=tiny.epub cols=\d+ rows=\d+ pages=(\d+)',t())[-1])
  day=shot('bodice-day.ppm');assert count(day,DAY)>300000,'the page is not paper-coloured'
  # Two pages forward by keys, one back by a click on the left of the page, one forward
  # by a click on its right.
  page('right',2);page('pgdn',3)
  def click(fx):
   q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':int(32767*fx)}},{'type':'abs','data':{'axis':'y','value':16384}}]});time.sleep(.3)
   for down in (True,False):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.1)
  s=len(t());click(.12);g['wait'](r'\[bodice.page\] book=tiny.epub page=2 ',s,30)
  s=len(t());click(.88);g['wait'](r'\[bodice.page\] book=tiny.epub page=3 ',s,30)
  # The contents list the three chapters; Down, Down, Enter jumps to the third.
  s=len(t());q.key('t');g['wait'](r'\[bodice.contents\] entries=3\n',s,30)
  for n in (1,2,3):g['wait'](rf'\[bodice.contents.entry\] chapter={n-1} depth=0 label=Chapter {n}\n',s,30)
  for _ in range(2):q.key('down');time.sleep(.15)
  s=len(t());q.key('ret');jump=g['wait'](r'\[bodice.jump\] book=tiny.epub entry=Chapter 3 chapter=2 page=(\d+)\n',s,30)
  target=int(jump[1]);assert 1<target<pages,(target,pages)
  # Back one page into chapter 2: the place to come back to.
  left=page('left',target-1);assert left[1]=='1',left[0]
  saved=target-1
  s=len(t());q.key('n');g['wait'](r'\[bodice.night\] on=1\n',s,30);time.sleep(1.5)
  night=shot('bodice-night.ppm');assert count(night,NIGHT)>300000,'night mode did not darken the page'
  s=close(ready);g['wait'](rf'\[bodice.saved\] book=tiny.epub page={saved} chapter=1 offset={left[2]}\n',s,30)
  # Second run: the library lists the book; Enter opens it where it was left.
  ready,sid,s=start('')
  assert ready[3]=='1' and ready[4]=='1',ready[0]
  g['wait'](r'\[bodice.library\] books=1\n',s,60);g['wait'](r'\[bodice.library.book\] name=tiny.epub\n',s,60)
  s=len(t());q.key('ret')
  re_opened=g['wait'](r'\[bodice.opened\] book=tiny.epub format=epub title=A Tiny Book author=Bodice Fixture chapters=3 toc=3 pages=(\d+) page=(\d+) saved=1\n',s,60)
  if int(re_opened[1])!=pages:
   # A window of another size: the whole desktop again, where the saved page is the same page.
   s2=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'meta_l'},{'type':'qcode','data':'m'}],'hold-time':80})
   g['wait'](rf'\[bodice.layout\] book=tiny.epub cols=\d+ rows=\d+ pages={pages} page={saved}\n',s2,30);time.sleep(1);g['focus'](sid);time.sleep(.5)
  else:assert int(re_opened[2])==saved,re_opened[0]
  time.sleep(1.5);again=shot('bodice-reopened.ppm');assert count(again,NIGHT)>300000,'night mode was not kept'
  # Esc back to the library saves the place again; then close.
  s=len(t());q.key('esc');g['wait'](rf'\[bodice.saved\] book=tiny.epub page={saved} ',s,30)
  close(ready)
  text=t();assert 'KERNEL PANIC' not in text and '[bodice.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'pages':pages,'saved_page':saved})
 finally:g['stop']()
if __name__=='__main__':main()
