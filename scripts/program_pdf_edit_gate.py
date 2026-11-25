#!/usr/bin/env python3
"""Vellum edits a PDF and saves it: `run vellum @form.pdf` opens a one-page form; Ctrl+F
finds PARAKEET and Ctrl+H highlights it; Ctrl+N adds a typed sticky note; Ctrl+R
chooses a region with the arrows and Enter highlights it; Ctrl+L lists the three and
Delete removes the region's highlight; Ctrl+E fills the text field, Ctrl+Down moves to the
checkbox (Shell keeps Tab) and Space ticks it; Ctrl+S appends everything as an incremental update and
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
from make_pdf_fixture import form_document,choice_document,advanced_choice_document,FORM_WORD
from browser_gate_viewport import chrome_metrics,content_rect
from web_surface_geometry import latest_frame
ROOT=Path(__file__).resolve().parents[1]
FORM=form_document()
CHOICE=choice_document()
ADVANCED=advanced_choice_document()
FILES={'/form.pdf':FORM,'/choice.pdf':CHOICE,'/advanced-choice.pdf':ADVANCED}
FILES['/certificate.der']=(ROOT/'crates/vellum-edit/tests/fixtures/cms-test-cert.der').read_bytes()
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
BAR=30
def thumb_ok(data,want_w,want_h):
 w,h=int.from_bytes(data[0:2],'little'),int.from_bytes(data[2:4],'little')
 assert (w,h)==(want_w,want_h),(w,h)
 assert len(data)==4+w*h*3,(len(data),w,h)
 return w,h
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 rw,rh=map(int,a.resolution.split('x'))
 pdf=(ROOT/'iso_root/programs/vellum').read_bytes();assert pdf[:4]==b'\x7fELF'
 signer=(ROOT/'iso_root/programs/shibari-sign').read_bytes();assert signer[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 signed_fixture=a.out/'cms-signed.pdf'
 subprocess.run(['cargo','test','-p','vellum-edit','--test','edit','detached_cms_in_pdf_verifies_over_its_byte_range','--','--exact'],
                cwd=ROOT,env={**os.environ,'SEXOS_VELLUM_CMS_PDF_FIXTURE':str(signed_fixture)},check=True)
 FILES['/cms-signed.pdf']=signed_fixture.read_bytes()
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('vellum',pdf),('shibari-sign',signer)])
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
 def click(x,y):
  q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.25)
  for down in (True,False):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.2)
 def drag(x0,y0,x1,y1):
  def move(x,y):
   q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round(x*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round(y*32767/(rh-1))}}]});time.sleep(.2)
  move(x0,y0)
  q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':True,'button':'left'}}]});time.sleep(.2)
  move((x0+x1)//2,(y0+y1)//2);move(x1,y1)
  q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':False,'button':'left'}}]});time.sleep(.2)
 saved_path=None
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  s=len(t());line(f'fetch http://10.0.2.2:{PORT}/form.pdf form.pdf')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,300);assert m[1].startswith('Saved'),m[1]
  # The thumbnail first: no window, the header and the page's shape.
  s=len(t());line('run vellum --thumb @form.pdf @form.thumb 96')
  th=g['wait'](r'\[pdf\.thumb\] status=(\d+)([^\n]*)\n',s,300);assert th[1]=='0',th[0]
  tw,thh=map(int,re.search(r'w=(\d+) h=(\d+)',th[2]).groups());assert (tw,thh)==(74,96),th[0]
  thumb_ms=int(re.search(r'ms=(\d+)',th[2])[1])
  g['wait'](r'\[std\.exit\] target=\d+ task=\d+ status=0\n',s,120)
  s=len(t());line('run vellum --thumb @missing.pdf @missing.thumb 96')
  g['wait'](r'\[pdf\.thumb\] status=1 [^\n]*\n',s,300);g['wait'](r'\[std\.exit\] target=\d+ task=\d+ status=1\n',s,120)
  s=len(t());line('run vellum @form.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=form\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 width=\d+ height=\d+ ink=\d+ ms=\d+\n',s,900)
  launch=re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])
  sid=int(launch[-1]);time.sleep(1);g['focus'](sid);time.sleep(.5)
  s=len(t());chord('ctrl','shift','v')
  g['wait'](r'\[pdf\.signatures\] fields=0 signed=0 malformed=0 verified=0 content_intact=0 content_changed=0 unsupported=0\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-signature-inspection.ppm')})
  # Find the word, highlight it.
  s=len(t());chord('ctrl','f');keys(FORM_WORD.lower());q.key('ret')
  g['wait'](rf'\[pdf\.find\] term={FORM_WORD.lower()} page=1 hits=1 index=1\n',s,600)
  hl=step(lambda:chord('ctrl','h'),r'\[pdf\.annotate\] kind=highlight page=1 rects=1 from=find annots=1\n')
  q.key('esc');time.sleep(.3)
  # A sticky note, typed.
  s=len(t());chord('ctrl','n');keys(NOTE);q.key('ret')
  g['wait'](rf'\[pdf\.annotate\] kind=note page=1 chars={len(NOTE)} annots=2\n',s)
  step(lambda:chord('ctrl','z'),r'\[pdf\.annotate\.history\] action=undo page=1 annots=1\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'pdf-note-undone.ppm')})
  step(lambda:chord('ctrl','shift','z'),r'\[pdf\.annotate\.history\] action=redo page=1 annots=2\n')
  # A region: shown, moved right twice, made taller, highlighted with Enter.
  step(lambda:chord('ctrl','r'),r'\[pdf\.region\] shown=1\n')
  for k in ('right','right'):step(lambda:q.key(k),r'\[pdf\.region\] shown=1 x=')
  step(lambda:chord('shift','down'),r'\[pdf\.region\] shown=1 x=\d+ y=\d+ w=\d+ h=60\n')
  tile=latest_frame(t(),sid)
  bx,by,client_w,_=content_rect(t(),tile)
  ui=chrome_metrics(t())['scale']
  page_w=int(re.search(r'\[pdf\.page\] page=1 pages=1 width=(\d+)',t())[1])
  client_w//=ui
  left=(client_w-page_w)//2 if page_w+24<=client_w else 12
  s=len(t());drag(bx+round((left+100)*ui),by+round((BAR+12+80)*ui),
                   bx+round((left+260)*ui),by+round((BAR+12+150)*ui))
  g['wait'](r'\[pdf\.region\.drag\] page=1 x=\d+ y=\d+ w=\d+ h=\d+\n',s)
  q.cmd('screendump',{'filename':str(a.out/'pdf-region-drag.ppm')})
  assert (a.out/'pdf-region-drag.ppm').stat().st_size>10000
  step(lambda:q.key('ret'),r'\[pdf\.annotate\] kind=highlight page=1 rects=1 from=region annots=3\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'pdf-marked.ppm')})
  # The list: three, in the page's order; the last (the region) deleted.
  step(lambda:chord('ctrl','l'),r'\[pdf\.annots\] page=1 count=3 kinds=Highlight,Text,Highlight\n')
  q.key('down');time.sleep(.2);q.key('down');time.sleep(.2)
  step(lambda:q.key('delete'),r'\[pdf\.annotate\] kind=delete page=1 was=Highlight annots=2\n')
  step(lambda:chord('ctrl','z'),r'\[pdf\.annotate\.history\] action=undo page=1 annots=3\n')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-deletion-undone.ppm')})
  step(lambda:chord('ctrl','shift','z'),r'\[pdf\.annotate\.history\] action=redo page=1 annots=2\n')
  q.key('esc');time.sleep(.3)
  # Edit inside the name, including a forward delete and an end-of-field edit.
  # The committed value remains NAME after these caret operations.
  step(lambda:chord('ctrl','e'),r'\[pdf\.form\] fields=2 first=name\n')
  keys('Ada Lvelace')
  step(lambda:q.key('home'),r'\[pdf\.form\.caret\] at=0 chars=11\n')
  s=len(t())
  for _ in range(5):q.key('right');time.sleep(.1)
  g['wait'](r'\[pdf\.form\.caret\] at=5 chars=11\n',s)
  keys('o')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-form-caret.ppm')})
  assert (a.out/'pdf-form-caret.ppm').stat().st_size>10000
  q.key('left');q.key('delete');keys('o')
  step(lambda:q.key('end'),r'\[pdf\.form\.caret\] at=12 chars=12\n')
  q.key('backspace');keys('e')
  q.key('home');time.sleep(.2)
  for _ in range(4):q.key('right');time.sleep(.1)
  s=len(t())
  for _ in range(8):chord('shift','right')
  g['wait'](r'\[pdf\.form\.selection\] start=4 end=12 chars=12\n',s)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-form-selection.ppm')})
  assert (a.out/'pdf-form-selection.ppm').stat().st_size>10000
  keys('Lovelace')
  tile=latest_frame(t(),sid)
  bx,by,_,_=content_rect(t(),tile)
  ui=chrome_metrics(t())['scale']
  name_vx=8+(len('name')+2)*9
  s=len(t());click(bx+round((name_vx+5*9+2)*ui),by+round(BAR//2*ui))
  g['wait'](r'\[pdf\.form\.pointer\] name=name at=5 chars=12\n',s)
  keys('X')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-form-pointer-caret.ppm')})
  assert (a.out/'pdf-form-pointer-caret.ppm').stat().st_size>10000
  q.key('backspace');time.sleep(.2)
  # Shell keeps Tab (it moves focus between windows): Ctrl+Down is the next field.
  s=len(t());chord('ctrl','down')
  g['wait'](rf'\[pdf\.field\] name=name kind=text value={NAME}\n',s);g['wait'](r'\[pdf\.field\.focus\] name=agree index=2 page=1\n',s)
  step(lambda:q.key('spc'),r'\[pdf\.field\] name=agree kind=checkbox value=Yes\n')
  step(lambda:chord('ctrl','z'),r'\[pdf\.field\.history\] action=undo name=agree\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'pdf-form-undone.ppm')})
  step(lambda:chord('ctrl','shift','z'),r'\[pdf\.field\.history\] action=redo name=agree\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'pdf-filled.ppm')})
  step(lambda:q.key('esc'),r'\[pdf\.form\] done=1\n')
  # A typed visual signature is visible markup, explicitly not a digital signature.
  s=len(t());chord('ctrl','shift','g');keys(NAME);q.key('ret')
  g['wait'](rf'\[pdf\.sign\.visual\] page=1 chars={len(NAME)} annots=3\n',s)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'pdf-visual-signature.ppm')})
  # Save: an update after the original, read back, and drawn again from what was read.
  s=len(t());chord('ctrl','s')
  g['wait'](r'\[pdf\.save\.synced\] path=@/form\.pdf bytes=\d+\n',s,300)
  sv=g['wait'](r'\[pdf\.saved\] path=(\S+) bytes=(\d+) original=(\d+) prefix=1 same=1 annots=3 filled=2 ms=(\d+)\n',s,300)
  trace=t()[s:]
  assert trace.find('[pdf.save.synced]')<trace.find('[pdf.saved]'), 'save reported success before Files sync'
  saved_path=sv[1];assert int(sv[3])==len(FORM) and int(sv[2])>len(FORM),sv[0]
  g['wait'](r'\[pdf\.page\] page=1 pages=1 width=\d+ height=\d+ ink=\d+ ms=\d+\n',s,600)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'pdf-saved.ppm')})
  q.cmd('screendump',{'filename':str(a.out/'pdf-form-sync-confirmed.ppm')})
  assert (a.out/'pdf-form-sync-confirmed.ppm').stat().st_size>10000
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  g['wait'](rf'\[app\.private\.retired\] target={o[1]} task={o[2]} ',s,180)
  # Opened again from the disk: all three annotations are there.
  s=len(t());line('run vellum @form.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=form\.pdf pages=1\n',s,600)
  form_page=g['wait'](r'\[pdf\.page\] page=1 pages=1 width=(\d+) height=(\d+) ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid);time.sleep(.5)
  tile=latest_frame(t(),sid);bx,by,client_w,_=content_rect(t(),tile)
  ui=chrome_metrics(t())['scale'];page_w=int(form_page[1]);scale=page_w/612
  client_w//=ui;left=(client_w-page_w)//2 if page_w+24<=client_w else 12
  s=len(t());click(bx+round((left+81*scale)*ui),by+round((12+225*scale)*ui))
  g['wait'](r'\[pdf\.widget\.pointer\] name=agree kind=checkbox page=1\n',s,120)
  g['wait'](r'\[pdf\.field\] name=agree kind=checkbox value=Off\n',s,120)
  q.cmd('screendump',{'filename':str(a.out/'pdf-widget-checkbox.ppm')})
  assert (a.out/'pdf-widget-checkbox.ppm').stat().st_size>10000
  step(lambda:chord('ctrl','z'),r'\[pdf\.field\.history\] action=undo name=agree\n')
  step(lambda:chord('ctrl','l'),r'\[pdf\.annots\] page=1 count=3 kinds=Highlight,Text,FreeText\n')
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  # A real AcroForm combo and list keep their export values through native
  # keyboard selection, incremental save, and a fresh guest reopen.
  s=len(t());line(f'fetch http://10.0.2.2:{PORT}/choice.pdf choice.pdf')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,300);assert m[1].startswith('Saved'),m[1]
  s=len(t());line('run vellum @choice.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=choice\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid)
  step(lambda:chord('ctrl','e'),r'\[pdf\.form\] fields=2 first=size\n')
  step(lambda:q.key('down'),r'\[pdf\.choice\] name=size selected=2 count=3 display=Medium\n')
  s=len(t());q.key('ret')
  g['wait'](r'\[pdf\.field\] name=size kind=choice value=Medium\n',s,120)
  g['wait'](r'\[pdf\.choice\.focus\] name=team selected=1 count=3\n',s,120)
  step(lambda:q.key('down'),r'\[pdf\.choice\] name=team selected=2 count=3 display=Engineering\n')
  s=len(t());q.key('ret')
  g['wait'](r'\[pdf\.field\] name=team kind=choice value=Engineering\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-choice-fields.ppm')})
  assert (a.out/'pdf-choice-fields.ppm').stat().st_size>10000
  step(lambda:q.key('esc'),r'\[pdf\.form\] done=1\n')
  s=len(t());chord('ctrl','s')
  g['wait'](r'\[pdf\.save\.synced\] path=@/choice\.pdf bytes=\d+\n',s,300)
  g['wait'](r'\[pdf\.saved\] path=@/choice\.pdf bytes=\d+ original=\d+ prefix=1 same=1 annots=0 filled=2 ms=\d+\n',s,300)
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  s=len(t());line('run vellum @choice.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=choice\.pdf pages=1\n',s,600)
  page_receipt=g['wait'](r'\[pdf\.page\] page=1 pages=1 width=(\d+) height=(\d+) ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid)
  tile=latest_frame(t(),sid)
  bx,by,client_w,client_h=content_rect(t(),tile)
  ui=chrome_metrics(t())['scale']
  page_w,page_h=map(int,page_receipt[1:3]);scale=page_w/612
  client_w//=ui;client_h//=ui
  left=(client_w-page_w)//2 if page_w+24<=client_w else 12
  widget_x=bx+round((left+240*scale)*ui)
  widget_y=by+round((12+124*scale)*ui)
  s=len(t());click(widget_x,widget_y)
  g['wait'](r'\[pdf\.widget\.pointer\] name=size kind=choice page=1\n',s,120)
  g['wait'](r'\[pdf\.choice\.focus\] name=size selected=2 count=3\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-choice-reopened.ppm')})
  assert (a.out/'pdf-choice-reopened.ppm').stat().st_size>10000
  q.cmd('screendump',{'filename':str(a.out/'pdf-choice-pointer-menu.ppm')})
  assert (a.out/'pdf-choice-pointer-menu.ppm').stat().st_size>10000
  s=len(t());step(lambda:click(bx+100*ui,by+(BAR+2*20+10)*ui),r'\[pdf\.choice\.pointer\] name=size selected=3 count=3\n')
  g['wait'](r'\[pdf\.field\] name=size kind=choice value=Large\n',s,120)
  ah=client_h-BAR
  scroll=max(0,min(int(111*scale)+12-ah//4,max(0,page_h+24-ah)))
  team_y=by+round((BAR+12+213*scale-scroll)*ui)
  s=len(t());click(widget_x,team_y)
  g['wait'](r'\[pdf\.widget\.pointer\] name=team kind=choice page=1\n',s,120)
  g['wait'](r'\[pdf\.choice\.focus\] name=team selected=2 count=3\n',s,120)
  q.cmd('screendump',{'filename':str(a.out/'pdf-choice-pointer-selected.ppm')})
  assert (a.out/'pdf-choice-pointer-selected.ppm').stat().st_size>10000
  step(lambda:q.key('esc'),r'\[pdf\.form\] done=1\n')
  s=len(t());chord('ctrl','s')
  g['wait'](r'\[pdf\.save\.synced\] path=@/choice\.pdf bytes=\d+\n',s,300)
  g['wait'](r'\[pdf\.saved\] path=@/choice\.pdf bytes=\d+ original=\d+ prefix=1 same=1 annots=0 filled=2 ms=\d+\n',s,300)
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  # Editable combo text and multi-select list values survive a guest save.
  s=len(t());line(f'fetch http://10.0.2.2:{PORT}/advanced-choice.pdf advanced-choice.pdf')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,300);assert m[1].startswith('Saved'),m[1]
  s=len(t());line('run vellum @advanced-choice.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=advanced-choice\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid)
  step(lambda:chord('ctrl','e'),r'\[pdf\.form\] fields=2 first=project\n')
  g['wait'](r'\[pdf\.choice\.focus\] name=project selected=0 count=3\n',s,120)
  chord('ctrl','a');keys('Orbit')
  s=len(t());q.key('ret')
  g['wait'](r'\[pdf\.field\] name=project kind=editable-choice value=Orbit\n',s,120)
  g['wait'](r'\[pdf\.choice\.focus\] name=teams selected=1 count=3\n',s,120)
  step(lambda:q.key('down'),r'\[pdf\.choice\] name=teams selected=2 count=3 display=Engineering\n')
  step(lambda:q.key('spc'),r'\[pdf\.field\] name=teams kind=multi-choice value=3 selected\n')
  step(lambda:q.key('up'),r'\[pdf\.choice\] name=teams selected=1 count=3 display=Accounting\n')
  step(lambda:q.key('spc'),r'\[pdf\.field\] name=teams kind=multi-choice value=2 selected\n')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-advanced-choice-fields.ppm')})
  assert (a.out/'pdf-advanced-choice-fields.ppm').stat().st_size>10000
  step(lambda:q.key('esc'),r'\[pdf\.form\] done=1\n')
  s=len(t());chord('ctrl','s')
  g['wait'](r'\[pdf\.save\.synced\] path=@/advanced-choice\.pdf bytes=\d+\n',s,300)
  g['wait'](r'\[pdf\.saved\] path=@/advanced-choice\.pdf bytes=\d+ original=\d+ prefix=1 same=1 annots=0 filled=2 ms=\d+\n',s,300)
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  s=len(t());line('run vellum @advanced-choice.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=advanced-choice\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid)
  s=len(t());chord('ctrl','e')
  g['wait'](r'\[pdf\.choice\.focus\] name=project selected=0 count=3\n',s,120)
  step(lambda:chord('ctrl','down'),r'\[pdf\.choice\.focus\] name=teams selected=2 count=3\n')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-advanced-choice-reopened.ppm')})
  assert (a.out/'pdf-advanced-choice-reopened.ppm').stat().st_size>10000
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  # This fixture was signed by vellum-edit::cms in the host test above. The
  # guest opens those exact bytes and shows its deliberately unverified status.
  s=len(t());line(f'fetch http://10.0.2.2:{PORT}/cms-signed.pdf cms-signed.pdf')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,300);assert m[1].startswith('Saved'),m[1]
  s=len(t());line('run vellum @cms-signed.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=cms-signed\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid);time.sleep(.5)
  s=len(t());chord('ctrl','shift','v')
  g['wait'](r'\[pdf\.signatures\] fields=1 signed=1 malformed=0 verified=0 content_intact=1 content_changed=0 unsupported=0\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-cms-signed-inspection.ppm')})
  q.cmd('screendump',{'filename':str(a.out/'pdf-cms-content-intact.ppm')})
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  # Place a real empty signature widget from the Vellum window, save it,
  # and reopen the persisted PDF before reporting the field as available.
  s=len(t());line(f'fetch http://10.0.2.2:{PORT}/form.pdf signature-field.pdf')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,300);assert m[1].startswith('Saved'),m[1]
  s=len(t());line('run vellum @signature-field.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=signature-field\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid)
  step(lambda:chord('ctrl','shift','d'),r'\[pdf\.sign\.field\] page=1 name=Vellum signature 1 empty=1\n')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-signature-field.ppm')})
  s=len(t());chord('ctrl','s')
  g['wait'](r'\[pdf\.saved\] path=@/signature-field\.pdf bytes=\d+ original=\d+ prefix=1 same=1 annots=0 filled=0 ms=\d+\n',s,300)
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  s=len(t());line('run vellum @signature-field.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=signature-field\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid)
  s=len(t());chord('ctrl','shift','v')
  g['wait'](r'\[pdf\.signatures\] fields=1 signed=0 malformed=0 verified=0 content_intact=0 content_changed=0 unsupported=0\n',s,120)
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  # Shibari imports a public certificate and the owner types the matching
  # scalar into its own window. No key bytes travel through Shell or Vellum.
  s=len(t());line(f'fetch http://10.0.2.2:{PORT}/certificate.der certificate.der')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,300);assert m[1].startswith('Saved'),m[1]
  s=len(t());line('run shibari-sign @certificate.der')
  launched=g['wait'](r'\[shell\.program\.launch\] target=(\d+) task=(\d+) surface=(\d+)',s,300)
  signer_sid=int(launched[3]);g['focus'](signer_sid);time.sleep(.5)
  # This is a published test key for the repository's synthetic certificate.
  fixture_scalar='15b6eb1d2fa7ec84fc8d80ef31bc5f6c37db2e8887945932ee00c7322c42a7ce'
  s=len(t());keys(fixture_scalar);q.key('ret')
  g['wait'](r'\[shibari\.pdf\.ready\] row=\d+ certificate_bytes=\d+\n',s,120)
  s=len(t());line('run vellum @signature-field.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=signature-field\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(.5);g['focus'](sid)
  step(lambda:chord('ctrl','shift','b'),r'\[pdf\.sign\.choice\] field=Vellum signature 1 certificate_bytes=\d+ confirmed=0\n')
  time.sleep(.4);q.cmd('screendump',{'filename':str(a.out/'pdf-digital-signature-consent.ppm')})
  step(lambda:q.key('esc'),r'\[pdf\.sign\.cancelled\] stage=viewer\n')
  step(lambda:chord('ctrl','shift','b'),r'\[pdf\.sign\.choice\] field=Vellum signature 1 certificate_bytes=\d+ confirmed=0\n')
  s=len(t());q.key('ret')
  g['wait'](r'\[pdf\.sign\.request\] client=\d+ signer=\d+ attrs=\d+\n',s,120)
  # Vellum stays responsive while Shibari waits for a decision. Cancel the
  # already submitted request, then retry with the same signer run.
  s=len(t());q.key('esc')
  g['wait'](r'\[pdf\.sign\.cancelled\] stage=approval\n',s,120)
  step(lambda:chord('ctrl','shift','b'),r'\[pdf\.sign\.choice\] field=Vellum signature 1 certificate_bytes=\d+ confirmed=0\n')
  s=len(t());q.key('ret')
  g['wait'](r'\[pdf\.sign\.request\] client=\d+ signer=\d+ attrs=\d+\n',s,120)
  g['focus'](signer_sid);time.sleep(.5);q.key('n')
  g['wait'](r'\[shibari\.pdf\.refused\]\n',s,120)
  g['wait'](r'\[pdf\.sign\.failed\] stage=approval_or_cms\n',s,120)
  g['focus'](sid);time.sleep(.5)
  step(lambda:chord('ctrl','shift','b'),r'\[pdf\.sign\.choice\] field=Vellum signature 1 certificate_bytes=\d+ confirmed=0\n')
  s=len(t());q.key('ret')
  g['wait'](r'\[pdf\.sign\.request\] client=\d+ signer=\d+ attrs=\d+\n',s,120)
  g['focus'](signer_sid);time.sleep(.5);q.key('y')
  g['wait'](r'\[shibari\.pdf\.approved\] ok=1\n',s,120)
  g['wait'](r'\[pdf\.sign\.saved\] field=Vellum signature 1 bytes=\d+ content_intact=1 verified=0\n',s,300)
  g['focus'](sid);time.sleep(.5)
  step(lambda:chord('ctrl','shift','v'),r'\[pdf\.signatures\] fields=1 signed=1 malformed=0 verified=0 content_intact=1 content_changed=0 unsupported=0\n')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-digital-signature.ppm')})
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  # A separate source stays untouched while a complete unsaved PDF is kept.
  s=len(t());line(f'fetch http://10.0.2.2:{PORT}/form.pdf recover.pdf')
  m=g['wait'](r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',s,300);assert m[1].startswith('Saved'),m[1]
  s=len(t());line('run vellum @recover.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=recover\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid)
  s=len(t());chord('ctrl','n');keys('crash note');q.key('ret')
  g['wait'](r'\[pdf\.annotate\] kind=note page=1 chars=10 annots=1\n',s)
  g['wait'](r'\[pdf\.recovery\.kept\] name=vellum-recovered-[0-9a-f]{16}\.pdf bytes=\d+\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'pdf-recovery-unsaved.ppm')})
  assert 'KERNEL PANIC' not in t() and '[std.abort]' not in t()
  g['stop']()  # No Close or Ctrl+S reaches Vellum.
  g=boot(a.iso,disk,a.out,'pdf-recovery-reboot',a.resolution);t=g['text'];q=g['q']
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  s=len(t());line('run vellum @recover.pdf')
  o=g['wait'](r'\[pdf\.open\] target=(\d+) task=(\d+) name=recover\.pdf pages=1\n',s,600)
  g['wait'](r'\[pdf\.recovery\.restored\] name=vellum-recovered-[0-9a-f]{16}\.pdf bytes=\d+\n',s,60)
  g['wait'](r'\[pdf\.page\] page=1 pages=1 ',s,900)
  sid=int(re.findall(rf'\[shell.program.launch\] target={o[1]} task={o[2]} surface=(\d+)',t()[s:])[-1]);time.sleep(1);g['focus'](sid)
  step(lambda:chord('ctrl','l'),r'\[pdf\.annots\] page=1 count=1 kinds=Text\n')
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'pdf-recovery-restored.ppm')})
  s=len(t());chord('ctrl','s')
  g['wait'](r'\[pdf\.saved\] path=@/recover\.pdf bytes=\d+ original=\d+ prefix=1 same=1 annots=1 filled=0 ms=\d+\n',s,300)
  s=len(t());q.key('f11');g['wait'](r'\[pdf\.closed\]\n',s,60)
  assert 'KERNEL PANIC' not in t() and '[std.abort]' not in t() and 'panicked' not in t()
 finally:g['stop']()
 # On the host, from the disk the system wrote.
 assert saved_path and saved_path.startswith('@/'),saved_path
 out_pdf=a.out/'form-saved.pdf';subprocess.run([str(extractor),str(disk),saved_path[1:],str(out_pdf)],check=True)
 data=out_pdf.read_bytes();assert data.startswith(FORM) and len(data)>len(FORM)
 update=data[len(FORM):]
 for want in (b'/Subtype /Highlight',b'/Subtype /Text',b'/Subtype /FreeText',b'/QuadPoints',b'/V /Yes',b'/Prev '):assert want in update,want
 assert NOTE.encode() in update and NAME.encode() in update
 recovered_pdf=a.out/'recover-saved.pdf';subprocess.run([str(extractor),str(disk),saved_path[1:].rsplit('/',1)[0]+'/recover.pdf',str(recovered_pdf)],check=True)
 recovered_data=recovered_pdf.read_bytes();assert recovered_data.startswith(FORM) and b'crash note' in recovered_data[len(FORM):]
 signed_pdf=a.out/'signature-field-signed.pdf'
 subprocess.run([str(extractor),str(disk),saved_path[1:].rsplit('/',1)[0]+'/signature-field.pdf',str(signed_pdf)],check=True)
 signed_data=signed_pdf.read_bytes()
 assert signed_data.startswith(FORM) and b'/ByteRange' in signed_data[len(FORM):] and b'/Contents <' in signed_data[len(FORM):]
 advanced_pdf=a.out/'advanced-choice-saved.pdf'
 subprocess.run([str(extractor),str(disk),saved_path[1:].rsplit('/',1)[0]+'/advanced-choice.pdf',str(advanced_pdf)],check=True)
 subprocess.run(['cargo','test','-p','vellum-edit','--test','edit','guest_advanced_choices_survive_guest_save','--','--exact'],
                cwd=ROOT,env={**os.environ,'SEXOS_VELLUM_GUEST_ADVANCED_PDF':str(advanced_pdf)},check=True)
 pointer_pdf=a.out/'choice-pointer-saved.pdf'
 subprocess.run([str(extractor),str(disk),saved_path[1:].rsplit('/',1)[0]+'/choice.pdf',str(pointer_pdf)],check=True)
 subprocess.run(['cargo','test','-p','vellum-edit','--test','edit','guest_pointer_choice_survives_guest_save','--','--exact'],
                cwd=ROOT,env={**os.environ,'SEXOS_VELLUM_GUEST_POINTER_PDF':str(pointer_pdf)},check=True)
 subprocess.run(['cargo','test','-p','vellum-edit','--test','edit','guest_signed_pdf_verifies_independently_when_supplied','--','--exact'],
                cwd=ROOT,env={**os.environ,'SEXOS_VELLUM_GUEST_SIGNED_PDF':str(signed_pdf)},check=True)
 checked='bytes'
 try:import pypdf
 except ImportError:pypdf=None
 if pypdf:
  r=pypdf.PdfReader(io.BytesIO(data));annots=[x.get_object() for x in r.pages[0]['/Annots']]
  kinds=sorted(str(x['/Subtype']) for x in annots)
  assert kinds.count('/Highlight')==1 and kinds.count('/Text')==1 and kinds.count('/FreeText')==1 and kinds.count('/Widget')==2,kinds
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
