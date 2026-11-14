#!/usr/bin/env python3
"""Vellum edits a PDF and saves it: `run pdf @form.pdf` opens a one-page form; Ctrl+F
finds PARAKEET and Ctrl+H highlights it; Ctrl+N adds a typed sticky note; Ctrl+R
chooses a region with the arrows and Enter highlights it; Ctrl+L lists the three and
Delete removes the region's highlight; Ctrl+E fills the text field, Tab moves to the
checkbox and Space ticks it; Ctrl+S appends everything as an incremental update and
shows the file as read back. Opened again, the file still holds two annotations.

`pdf --thumb @form.pdf @form.thumb 96` writes the first page as a thumbnail with no
window (u16 LE width and height, then RGB rows); a missing input fails with no file.

After the system stops, the saved PDF and the thumbnail are read from the SexFS disk
on the host: the original bytes are the saved file's prefix, the update holds the
highlight, the note and the field values (checked with pypdf when it is installed),
and the thumbnail's header, size and white paper are right."""
import argparse,http.server,io,os,re,shutil,subprocess,threading,time
from pathlib import Path
from install_program import install
from make_pdf_fixture import form_document,FORM_WORD
ROOT=Path(__file__).resolve().parents[1]
FORM=form_document()
FILES={'/form.pdf':FORM}
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
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
NOTE='check this'
NAME='Ada Lovelace'
def thumb_ok(data,want_w,want_h):
 w,h=int.from_bytes(data[0:2],'little'),int.from_bytes(data[2:4],'little')
 assert (w,h)==(want_w,want_h),(w,h)
 assert len(data)==4+w*h*3,(len(data),w,h)
 return w,h
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 pdf=(ROOT/'iso_root/programs/pdf').read_bytes();assert pdf[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('pdf',pdf)])
 extractor=a.out/'sexfs-extract';subprocess.run(['rustc','--edition=2021','-O','-A','dead_code','-A','unused_attributes',str(ROOT/'scripts/sexfs_extract.rs'),'-o',str(extractor)],check=True)
 g=boot(a.iso,disk,a.out,'pdf-edit',a.resolution);t=g['text'];q=g['q']
 def chord(*k):q.cmd('send-key',{'keys':[{'type':'qcode','data':x} for x in k],'hold-time':80});time.sleep(.15)
 def keys(text):
  for k in text:
   if k=='@':chord('shift','2')
   elif k==':':chord('shift','semicolon')
   elif k.isupper():chord('shift',k.lower())
   else:q.key(KEYS.get(k,k))
   time.sleep(.06)
 def line(text):
  g['focus'](153);time.sleep(.5);keys(text);q.key('ret')
 def step(action,pattern,timeout=300):
  s=len(t());action();return g['wait'](pattern,s,timeout)
 saved_path=None
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  s=len(t());line(f'fetch http://10.0.2.2:{PORT}/form.pdf form.pdf')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,300);assert m[1].startswith('Saved'),m[1]
  # The thumbnail first: no window, the header and the page's shape.
  s=len(t());line('run pdf --thumb @form.pdf @form.thumb 96')
  th=g['wait'](r'\[pdf\.thumb\] status=(\d+)([^\n]*)\n',s,300);assert th[1]=='0',th[0]
  tw,thh=map(int,re.search(r'w=(\d+) h=(\d+)',th[2]).groups());assert (tw,thh)==(74,96),th[0]
  thumb_ms=int(re.search(r'ms=(\d+)',th[2])[1])
  g['wait'](r'\[std\.exit\] target=\d+ task=\d+ status=0\n',s,120)
  s=len(t());line('run pdf --thumb @missing.pdf @missing.thumb 96')
  g['wait'](r'\[pdf\.thumb\] status=1 [^\n]*\n',s,300);g['wait'](r'\[std\.exit\] target=\d+ task=\d+ status=1\n',s,120)
  s=len(t());line('run pdf @form.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=form\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 width=\d+ height=\d+ ink=\d+ ms=\d+\n',s,900)
  launch=re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])
  sid=int(launch[-1]);time.sleep(1);g['focus'](sid);time.sleep(.5)
  # Find the word, highlight it.
  s=len(t());chord('ctrl','f');keys(FORM_WORD.lower());q.key('ret')
  g['wait'](rf'\[pdf\.find\] term={FORM_WORD.lower()} page=1 hits=1 index=1\n',s,600)
  hl=step(lambda:chord('ctrl','h'),r'\[pdf\.annotate\] kind=highlight page=1 rects=1 from=find annots=1\n')
  q.key('esc');time.sleep(.3)
  # A sticky note, typed.
  s=len(t());chord('ctrl','n');keys(NOTE);q.key('ret')
  g['wait'](rf'\[pdf\.annotate\] kind=note page=1 chars={len(NOTE)} annots=2\n',s)
  # A region: shown, moved right twice, made taller, highlighted with Enter.
  step(lambda:chord('ctrl','r'),r'\[pdf\.region\] shown=1\n')
  for k in ('right','right'):step(lambda:q.key(k),r'\[pdf\.region\] shown=1 x=')
  step(lambda:chord('shift','down'),r'\[pdf\.region\] shown=1 x=\d+ y=\d+ w=\d+ h=60\n')
  step(lambda:q.key('ret'),r'\[pdf\.annotate\] kind=highlight page=1 rects=1 from=region annots=3\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'pdf-marked.ppm')})
  # The list: three, in the page's order; the last (the region) deleted.
  step(lambda:chord('ctrl','l'),r'\[pdf\.annots\] page=1 count=3 kinds=Highlight,Text,Highlight\n')
  q.key('down');time.sleep(.2);q.key('down');time.sleep(.2)
  step(lambda:q.key('delete'),r'\[pdf\.annotate\] kind=delete page=1 was=Highlight annots=2\n')
  q.key('esc');time.sleep(.3)
  # The form: type the name, Tab to the checkbox, Space ticks it, Escape ends.
  step(lambda:chord('ctrl','e'),r'\[pdf\.form\] fields=2 first=name\n')
  keys(NAME)
  s=len(t());q.key('tab')
  g['wait'](rf'\[pdf\.field\] name=name kind=text value={NAME}\n',s);g['wait'](r'\[pdf\.field\.focus\] name=agree index=2 page=1\n',s)
  step(lambda:q.key('spc'),r'\[pdf\.field\] name=agree kind=checkbox value=Yes\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'pdf-filled.ppm')})
  step(lambda:q.key('esc'),r'\[pdf\.form\] done=1\n')
  # Save: an update after the original, read back, and drawn again from what was read.
  s=len(t());chord('ctrl','s')
  sv=g['wait'](r'\[pdf\.saved\] path=(\S+) bytes=(\d+) original=(\d+) prefix=1 same=1 annots=2 filled=2 ms=(\d+)\n',s,300)
  saved_path=sv[1];assert int(sv[3])==len(FORM) and int(sv[2])>len(FORM),sv[0]
  g['wait'](r'\[pdf\.page\] page=1 pages=1 width=\d+ height=\d+ ink=\d+ ms=\d+\n',s,600)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'pdf-saved.ppm')})
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  g['wait'](rf'\[app\.private\.retired\] target={o[1]} task={o[2]} ',s,180)
  # Opened again from the disk: the two annotations are there.
  s=len(t());line('run pdf @form.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=form\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid);time.sleep(.5)
  step(lambda:chord('ctrl','l'),r'\[pdf\.annots\] page=1 count=2 kinds=Highlight,Text\n')
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  assert 'KERNEL PANIC' not in t() and '[std.abort]' not in t() and 'panicked' not in t()
 finally:g['stop']()
 # On the host, from the disk the system wrote.
 assert saved_path and saved_path.startswith('@/'),saved_path
 out_pdf=a.out/'form-saved.pdf';subprocess.run([str(extractor),str(disk),saved_path[1:],str(out_pdf)],check=True)
 data=out_pdf.read_bytes();assert data.startswith(FORM) and len(data)>len(FORM)
 update=data[len(FORM):]
 for want in (b'/Subtype /Highlight',b'/Subtype /Text',b'/QuadPoints',b'/V /Yes',b'/Prev '):assert want in update,want
 assert NOTE.encode() in update and NAME.encode() in update
 checked='bytes'
 try:import pypdf
 except ImportError:pypdf=None
 if pypdf:
  r=pypdf.PdfReader(io.BytesIO(data));annots=[x.get_object() for x in r.pages[0]['/Annots']]
  kinds=sorted(str(x['/Subtype']) for x in annots)
  assert kinds.count('/Highlight')==1 and kinds.count('/Text')==1 and kinds.count('/Widget')==2,kinds
  assert any(str(x.get('/Contents',''))==NOTE for x in annots)
  fields=r.get_fields();assert str(fields['name'].get('/V'))==NAME and str(fields['agree'].get('/V'))=='/Yes',fields
  checked='pypdf'
 out_thumb=a.out/'form.thumb';subprocess.run([str(extractor),str(disk),saved_path[1:].rsplit('/',1)[0]+'/form.thumb',str(out_thumb)],check=True)
 tdata=out_thumb.read_bytes();thumb_ok(tdata,74,96)
 white=sum(1 for i in range(4,len(tdata),3) if min(tdata[i:i+3])>=240);assert white>74*96*0.8,white
 missing=subprocess.run([str(extractor),str(disk),saved_path[1:].rsplit('/',1)[0]+'/missing.thumb',str(a.out/'missing.thumb')],capture_output=True)
 assert missing.returncode!=0,'a failed thumbnail left a file'
 print({'ok':True,'saved_bytes':len(data),'original_bytes':len(FORM),'checked_with':checked,'thumb':(tw,thh),'thumb_ms':thumb_ms})
if __name__=='__main__':main()
