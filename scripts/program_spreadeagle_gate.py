#!/usr/bin/env python3
"""Spreadeagle, the spreadsheet (GridCraft's engine, built for x86_64-sex-std.json):

* `run spreadeagle` writes welcome.xlsx (hand-written SpreadsheetML, apps/spreadeagle/make_welcome.py)
  into its folder and opens it: two sheets, the Budget column A as written.
* Right shows column B with the cached formula results recalculated by the engine
  (B5 =SUM(B2:B4) = 1650, B6 =B5*12 = 19800).
* Typing 40 into B2 and Enter recalculates: B5 490, B6 5880.
* Ctrl+S writes the xlsx; Ctrl+P writes welcome.pdf beside it; the window closes.
* Ctrl+Shift+U wraps a selected cell, grows its row and draws lines in a screenshot.
* A second `run spreadeagle` reads the saved file back: B2 is 40 and the formulas still give
  490 and 5880, so GridCraft's writer and reader round-trip on SexOS.
* Shift+arrows select A1:B4; Ctrl+Shift+D sorts it by column B, Z to A, under its header
  (300, 150, 40) and the totals still read 490 and 5880.
* Ctrl+Shift+C adds a column chart of the selection at D1, drawn over the grid.
* Ctrl+Shift+E on A3 (Fun) filters the region to that row (the others read `~`);
  Ctrl+Shift+O opens the native value menu; choosing Food changes the visible row,
  Space marks Food and Fun together, then All rows restores the list.
  Ctrl+Shift+L turns the filter off.
* Ctrl+Shift+F on B2 freezes the header row and the label column; Ctrl+S saves.
* A third `run spreadeagle` finds the chart and the frozen panes kept. Ctrl+F finds "rent"
  at A4; Ctrl+C copies it and Ctrl+V pastes it into A8; Ctrl+Z takes the paste back.
  Ctrl+H replaces "food" with "meals"; Ctrl+Shift+4 shows B2:B4 as currency; dragging
  column A's header edge widens it.
  Ctrl+Page Down shows the second sheet.
* Edit B2 without saving, wait for the idle recovery copy, and cut QEMU power. On the
  same disk after reboot, Spreadeagle restores B2 and its recalculated totals; Ctrl+S
  makes that recovery the workbook's saved state, which survives another reopen.

No kernel panic, page fault or std abort anywhere in the record."""
import argparse,http.server,os,re,shutil,threading,time
from pathlib import Path
from install_program import install
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
from program_gate_geometry import current_tile
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}
COL_A=r'Item\|Rent\|Food\|Fun\|Total\|Per year\|\|'

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/spreadeagle').read_bytes();assert elf[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('spreadeagle',elf)])
 fixtures={name:(ROOT/'apps/spreadeagle/assets'/name).read_bytes() for name in ('import-warning.ods','formula-compat.ods','filter-overflow.xlsx')}
 class WarningServer(http.server.BaseHTTPRequestHandler):
  def do_GET(self):
   body=fixtures.get(self.path.lstrip('/'))
   if body is None:self.send_error(404);return
   self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  def log_message(self,*args):pass
 server=http.server.ThreadingHTTPServer(('127.0.0.1',0),WarningServer)
 threading.Thread(target=server.serve_forever,daemon=True).start()
 os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
 g=boot(a.iso,disk,a.out,'spreadeagle',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  for k in line:
   if k=='+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'equal'}],'hold-time':80})
   elif k==':':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'semicolon'}],'hold-time':80})
   elif k in '<>':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'comma' if k=='<' else 'dot'}],'hold-time':80})
   elif k=='*':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'8'}],'hold-time':80})
   else:q.key({' ':'spc','.':'dot','/':'slash','-':'minus','=':'equal'}.get(k,k))
   time.sleep(.05)
 def run(line):
  g['focus'](153);time.sleep(.5);type_line(line);q.key('ret')
 def column(col,text,start,timeout=300):
  return g['wait'](rf'\[spreadeagle\.column\] sheet=Budget col={col} text={text}\n',start,timeout)
 def opened(col_a=COL_A):
  s=len(t());run('run spreadeagle')
  o=g['wait'](r'\[spreadeagle\.open\] target=(\d+) task=(\d+) name=welcome\.xlsx sheets=Budget,Notes\n',s,900)
  g['wait'](r'\[spreadeagle\.theme\] mode=Light accent=d6336c',s,60)
  column('A',col_a,s,600)
  g['focus'](SURFACE[int(o[1])]);time.sleep(.8)
  return o
 def ctrl_shift(k,pattern):
  s=len(t());keys=('ctrl','shift',k)
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,keys),(False,keys[::-1]))for x in ks]})
  return g['wait'](pattern,s,120)
 def shift(k):
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,('shift',k)),(False,(k,'shift')))for x in ks]});time.sleep(.3)
 def close(o):
  s=len(t());q.key('f11');g['wait'](r'\[spreadeagle\.closed\]\n',s,120)
  g['wait'](rf'\[app\.private\.retired\] target={o[1]} task={o[2]} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  # The first launch creates Spreadeagle's private folder for the fixture.
  o=opened();close(o)
  s=len(t());run(f'fetch http://10.0.2.2:{server.server_address[1]}/import-warning.ods /data/spreadeagle/import-warning.ods')
  g['wait'](r'\[spindle.fetch.line\] Saved \d+ bytes',s,240)
  s=len(t());run('run spreadeagle import-warning.ods')
  warn=g['wait'](r'\[spreadeagle.warning\] (ODS data-only import:[^\n]+)',s,900)
  assert 'Source formatting' in warn[1],warn[0]
  o=g['wait'](r'\[spreadeagle.open\] target=(\d+) task=(\d+) name=import-warning.ods sheets=Harvest\n',s,120)
  g['wait'](r'\[spreadeagle.warnings.shown\] count=1',s,120)
  g['focus'](SURFACE[int(o[1])]);time.sleep(1)
  q.cmd('screendump',{'filename':str(a.out/'spreadeagle-import-warning.ppm')})
  assert (a.out/'spreadeagle-import-warning.ppm').stat().st_size>10000
  s=len(t());q.key('esc');g['wait'](r'\[spreadeagle.warnings.closed\]',s,120)
  ctrl_shift('w',r'\[spreadeagle.warnings.shown\] count=1')
  q.key('esc');close(o)
  s=len(t());run(f'fetch http://10.0.2.2:{server.server_address[1]}/formula-compat.ods /data/spreadeagle/formula-compat.ods')
  g['wait'](r'\[spindle.fetch.line\] Saved \d+ bytes',s,240)
  s=len(t());run('run spreadeagle formula-compat.ods')
  f=g['wait'](r'\[spreadeagle.open\] target=(\d+) task=(\d+) name=formula-compat.ods sheets=Formula checks,Q1Q2\n',s,900)
  g['wait'](r'\[spreadeagle.warnings.shown\] count=\d+',s,120)
  g['focus'](SURFACE[int(f[1])]);q.key('esc')
  s=len(t());q.key('right')
  g['wait'](r'\[spreadeagle.column\] sheet=Formula checks col=B text=15\|99\|7\.5\|2\|5\|',s,120)
  time.sleep(.7);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-formula-compat.ppm')})
  assert (a.out/'spreadeagle-formula-compat.ppm').stat().st_size>10000
  close(f)
  s=len(t());run(f'fetch http://10.0.2.2:{server.server_address[1]}/filter-overflow.xlsx /data/spreadeagle/filter-overflow.xlsx')
  g['wait'](r'\[spindle.fetch.line\] Saved \d+ bytes',s,240)
  s=len(t());run('run spreadeagle filter-overflow.xlsx')
  large=g['wait'](r'\[spreadeagle.open\] target=(\d+) task=(\d+) name=filter-overflow.xlsx sheets=Items\n',s,900)
  g['focus'](SURFACE[int(large[1])]);time.sleep(.8)
  ctrl_shift('l',r'\[spreadeagle.command\] data.filter\n')
  ctrl_shift('o',r'\[spreadeagle\.filter\.menu\] col=A values=1000\n')
  s=len(t());q.key('slash');type_line('Item 1100')
  g['wait'](r'\[spreadeagle\.filter\.search\] col=A query=Item 1100 matches=1 selected=1\n',s,180)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-overflow.ppm')})
  assert (a.out/'spreadeagle-filter-overflow.ppm').stat().st_size>10000
  s=len(t());q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.choice\] col=A value=Item 1100\n',s,180)
  close(large)
  o=opened()
  s=len(t());q.key('right');column('B',r'Cost\|1200\|300\|150\|1650\|19800\|\|',s)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-opened.ppm')})
  shot=(a.out/'spreadeagle-opened.ppm').read_bytes()
  assert shot.startswith(b'P6\n') and shot.count(b'\xff\xff\xff')>10000,'Light theme cell background missing'
  ctrl_shift('slash',r'\[spreadeagle\.shortcuts\] shown=1')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-shortcuts.ppm')})
  assert (a.out/'spreadeagle-shortcuts.ppm').stat().st_size>10000
  ctrl_shift('slash',r'\[spreadeagle\.shortcuts\] shown=0')
  # Search the engine's function catalog, inspect the native picker, and insert
  # a formula without changing the existing budget cells.
  s=len(t())
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'g'}],'hold-time':80});time.sleep(.3)
  type_line('D8');q.key('ret')
  g['wait'](r'\[spreadeagle\.goto\] place=D8 cell=D8\n',s,120)
  ctrl_shift('i',r'\[spreadeagle\.functions\.open\] count=\d+\n')
  type_line('sum')
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-function-picker.ppm')})
  assert (a.out/'spreadeagle-function-picker.ppm').stat().st_size>10000
  s=len(t());q.key('ret')
  g['wait'](r'\[spreadeagle\.functions\.insert\] name=SUM at=D8\n',s,120)
  type_line('B2');q.key('ret')
  g['wait'](r'\[spreadeagle\.set\] cell=D8 input==SUM\(B2\)\n',s,120)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'g'}],'hold-time':80});time.sleep(.3)
  type_line('B1');q.key('ret');g['wait'](r'\[spreadeagle\.goto\] place=B1 cell=B1\n',s,120)
  s=len(t());q.key('down');type_line('40');q.key('ret')
  g['wait'](r'\[spreadeagle\.set\] cell=B2 input=40\n',s,120)
  column('B',r'Cost\|40\|300\|150\|490\|5880\|\|',s)
  m=g['chord']('ctrl','s',r'\[spreadeagle\.saved\] name=welcome\.xlsx bytes=(\d+)\n')
  assert int(m[1])>2000,f'saved file is too small: {m[0]}'
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-edited.ppm')})
  # Ctrl+P: SexOS prints to a PDF beside the file.
  s=len(t());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,('ctrl','p')),(False,('p','ctrl')))for x in ks]})
  pdf=g['wait'](r'\[spreadeagle\.pdf\] name=welcome\.pdf bytes=(\d+)\n',s,300)
  assert int(pdf[1])>500,f'PDF is too small: {pdf[0]}'
  s=len(t());g['chord']('ctrl','g');type_line('A9');q.key('ret')
  g['wait'](r'\[spreadeagle\.goto\] place=A9 cell=A9\n',s,120)
  s=len(t());type_line('Quarterly planning notes for our team');q.key('ret')
  g['wait'](r'\[spreadeagle\.set\] cell=A9 input=Quarterly planning notes for our team\n',s,120)
  q.key('up');time.sleep(.3)
  wrapped=ctrl_shift('u',r'\[spreadeagle\.wrap\] range=A9 on=1 row_height=(\d+) cell_y=(\d+)\n')
  assert int(wrapped[1])>20,wrapped[0]
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-wrapped-cell.ppm')})
  shot=(a.out/'spreadeagle-wrapped-cell.ppm').read_bytes()
  header=re.match(rb'P6\s+(\d+)\s+(\d+)\s+255\s',shot)
  assert header and len(shot)>10000,'missing wrapped-cell screenshot'
  width=int(header[1]);height=int(header[2]);start=header.end()
  tile=current_tile(t(),SURFACE[int(o[1])]);wa=int(re.findall(r'\[spreadeagle\.columns\] A=(\d+) B=\d+\n',t())[-1])
  left=tile[0]+8+48+5;top=tile[1]+36+int(wrapped[2])+4
  def ink(y0,y1):
   return sum(1 for y in range(top+y0,min(top+y1,height)) for x in range(left,min(left+wa-10,width))
              if max(shot[start+3*(y*width+x):start+3*(y*width+x)+3])<140)
  assert ink(0,14)>10 and ink(16,30)>10,'wrapped text did not paint two visible lines'
  g['chord']('ctrl','s',r'\[spreadeagle\.saved\] name=welcome\.xlsx bytes=\d+\n')
  close(o)
  o=opened()
  s=len(t());q.key('right');column('B',r'Cost\|40\|300\|150\|490\|5880\|\|',s)
  # Sort a selection, chart it, filter, freeze.
  s=len(t());g['chord']('ctrl','home',rf'\[spreadeagle\.column\] sheet=Budget col=A text={COL_A}\n')
  for k in('down','down','down','right'):shift(k)
  ctrl_shift('d',r'\[spreadeagle\.column\] sheet=Budget col=B text=Cost\|300\|150\|40\|490\|5880\|\|\n')
  m2=ctrl_shift('c',r'\[spreadeagle\.chart\] at=D1 width=(\d+) height=(\d+)\n')
  assert int(m2[1])>200 and int(m2[2])>150,m2[0]
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-chart.ppm')})
  ctrl_shift('m',r'\[spreadeagle\.chart\.choose\]\n')
  time.sleep(.8);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-chart-chooser.ppm')})
  s=len(t());q.key('p')
  m3=g['wait'](r'\[spreadeagle\.chart\.type\] kind=pie at=D1 width=(\d+) height=(\d+)\n',s,120)
  assert int(m3[1])>200 and int(m3[2])>150,m3[0]
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-pie-chart.ppm')})
  q.key('esc');time.sleep(.3)
  s=len(t());g['chord']('ctrl','g');type_line('D1');q.key('ret')
  g['wait'](r'\[spreadeagle\.goto\] place=D1 cell=D1\n',s,120)
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('title Budget by category');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=title Budget by category\n',s,120)
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-chart-edited.ppm')})
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('type line');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=type line\n',s,120)
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-chart-type-edited.ppm')})
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('switch');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=switch\n',s,120)
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-chart-series-switched.ppm')})
  shot=(a.out/'spreadeagle-chart-series-switched.ppm').read_bytes()
  assert len(shot)>10000 and shot!=(a.out/'spreadeagle-chart-type-edited.ppm').read_bytes()
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('remove 2');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=remove 2\n',s,120)
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-chart-series-removed.ppm')})
  shot=(a.out/'spreadeagle-chart-series-removed.ppm').read_bytes()
  assert len(shot)>10000 and shot!=(a.out/'spreadeagle-chart-series-switched.ppm').read_bytes()
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('switch');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=switch\n',s,120)
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('data A1:B5');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=data A1:B5\n',s,120)
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-chart-data-expanded.ppm')})
  shot=(a.out/'spreadeagle-chart-data-expanded.ppm').read_bytes()
  assert len(shot)>10000 and shot!=(a.out/'spreadeagle-chart-type-edited.ppm').read_bytes()
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('data A1:B4');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=data A1:B4\n',s,120)
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('legend right');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=legend right\n',s,120)
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('x title Category');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=x title Category\n',s,120)
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('labels on');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=labels on\n',s,120)
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-chart-layout-edited.ppm')})
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('table on');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=table on\n',s,120)
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-chart-presentation.ppm')})
  assert (a.out/'spreadeagle-chart-presentation.ppm').stat().st_size>10000
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('type doughnut');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=type doughnut\n',s,120)
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('hole 50');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=hole 50\n',s,120)
  ctrl_shift('q',r'\[spreadeagle\.chart\.edit-prompt\] id=\d+ at=D1\n')
  s=len(t());type_line('angle 90');q.key('ret')
  g['wait'](r'\[spreadeagle\.chart\.edit\] id=\d+ action=angle 90\n',s,120)
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-doughnut-design.ppm')})
  shot=(a.out/'spreadeagle-doughnut-design.ppm').read_bytes()
  assert len(shot)>10000 and shot!=(a.out/'spreadeagle-chart-presentation.ppm').read_bytes()
  s=len(t());g['chord']('ctrl','g');type_line('B4');q.key('ret')
  g['wait'](r'\[spreadeagle\.goto\] place=B4 cell=B4\n',s,120)
  s=len(t());q.key('left');column('A',r'Item\|Food\|Fun\|Rent\|Total\|Per year\|\|',s)
  q.key('up');time.sleep(.3)
  ctrl_shift('e',r'\[spreadeagle\.column\] sheet=Budget col=A text=Item\|~\|Fun\|~\|~\|~\|\|\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter.ppm')})
  ctrl_shift('o',r'\[spreadeagle\.filter\.menu\] col=A values=5\n')
  time.sleep(.7);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-menu.ppm')})
  s=len(t());q.key('down');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.choice\] col=A value=Food\n',s,120)
  column('A',r'Item\|Food\|~\|~\|~\|~\|\|',s)
  s=len(t());ctrl_shift('o',r'\[spreadeagle\.filter\.menu\] col=A values=5\n')
  g['wait'](r'\[spreadeagle\.filter\.menu\.state\] col=A active=values selected=1\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-selected.ppm')})
  s=len(t());q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.menu\.unchanged\] col=A\n',s,120)
  column('A',r'Item\|Food\|~\|~\|~\|~\|\|',s)
  ctrl_shift('o',r'\[spreadeagle\.filter\.menu\] col=A values=5\n')
  s=len(t());q.key('slash');type_line('rent')
  g['wait'](r'\[spreadeagle\.filter\.search\] col=A query=rent matches=1 selected=1\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-search.ppm')})
  s=len(t());q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.choice\] col=A value=Rent\n',s,120)
  column('A',r'Item\|~\|~\|Rent\|~\|~\|\|',s)
  ctrl_shift('o',r'\[spreadeagle\.filter\.menu\] col=A values=5\n')
  s=len(t());q.key('spc');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.choice\] col=A value=all\n',s,120)
  column('A',r'Item\|Food\|Fun\|Rent\|Total\|Per year\|\|',s)
  ctrl_shift('o',r'\[spreadeagle\.filter\.menu\] col=A values=5\n')
  s=len(t());q.key('down');q.key('spc');q.key('down');q.key('spc');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.choice\] col=A value=2-values\n',s,120)
  column('A',r'Item\|Food\|Fun\|~\|~\|~\|\|',s)
  time.sleep(.7);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-multiple.ppm')})
  ctrl_shift('o',r'\[spreadeagle\.filter\.menu\] col=A values=5\n')
  s=len(t());q.key('spc');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.choice\] col=A value=all\n',s,120)
  column('A',r'Item\|Food\|Fun\|Rent\|Total\|Per year\|\|',s)
  ctrl_shift('k',r'\[spreadeagle\.filter\.condition-prompt\] col=A\n')
  s=len(t());type_line('contains oo');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.condition\] col=A rule=contains oo\n',s,120)
  column('A',r'Item\|Food\|~\|~\|~\|~\|\|',s)
  time.sleep(.7);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-condition.ppm')})
  ctrl_shift('k',r'\[spreadeagle\.filter\.condition-prompt\] col=A\n')
  s=len(t());type_line('starts F or contains oo');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.condition\] col=A rule=starts F or contains oo\n',s,120)
  column('A',r'Item\|Food\|Fun\|~\|~\|~\|\|',s)
  time.sleep(.7);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-compound.ppm')})
  assert (a.out/'spreadeagle-filter-compound.ppm').stat().st_size>10000
  ctrl_shift('k',r'\[spreadeagle\.filter\.condition-prompt\] col=A\n')
  s=len(t());type_line('<> Food');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.condition\] col=A rule=<> Food\n',s,120)
  column('A',r'Item\|~\|Fun\|Rent\|Total\|Per year\|\|',s)
  time.sleep(.7);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-not-equal.ppm')})
  assert (a.out/'spreadeagle-filter-not-equal.ppm').stat().st_size>10000
  ctrl_shift('k',r'\[spreadeagle\.filter\.condition-prompt\] col=A\n')
  s=len(t());type_line('= F*');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.condition\] col=A rule== F\*\n',s,120)
  column('A',r'Item\|Food\|Fun\|~\|~\|~\|\|',s)
  time.sleep(.7);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-wildcard.ppm')})
  assert (a.out/'spreadeagle-filter-wildcard.ppm').stat().st_size>10000
  s=len(t());ctrl_shift('o',r'\[spreadeagle\.filter\.menu\] col=A values=5\n')
  g['wait'](r'\[spreadeagle\.filter\.menu\.state\] col=A active=condition selected=0\n',s,120)
  s=len(t());q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.menu\.unchanged\] col=A\n',s,120)
  column('A',r'Item\|Food\|Fun\|~\|~\|~\|\|',s)
  ctrl_shift('o',r'\[spreadeagle\.filter\.menu\] col=A values=5\n')
  s=len(t());q.key('spc');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.choice\] col=A value=all\n',s,120)
  assert (a.out/'spreadeagle-filter-selected.ppm').stat().st_size>10000
  assert (a.out/'spreadeagle-filter-search.ppm').stat().st_size>10000
  column('A',r'Item\|Food\|Fun\|Rent\|Total\|Per year\|\|',s)
  s=len(t());q.key('right');column('B',r'Cost\|300\|150\|40\|490\|5880\|\|',s)
  ctrl_shift('k',r'\[spreadeagle\.filter\.condition-prompt\] col=B\n')
  s=len(t());type_line('top 2');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.condition\] col=B rule=top 2\n',s,120)
  column('B',r'Cost\|~\|~\|~\|490\|5880\|\|',s)
  time.sleep(.7);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-top-two.ppm')})
  assert (a.out/'spreadeagle-filter-top-two.ppm').stat().st_size>10000
  ctrl_shift('k',r'\[spreadeagle\.filter\.condition-prompt\] col=B\n')
  s=len(t());type_line('bottom 2');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.condition\] col=B rule=bottom 2\n',s,120)
  column('B',r'Cost\|~\|150\|40\|~\|~\|\|',s)
  time.sleep(.7);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-filter-bottom-two.ppm')})
  assert (a.out/'spreadeagle-filter-bottom-two.ppm').stat().st_size>10000
  ctrl_shift('o',r'\[spreadeagle\.filter\.menu\] col=B values=5\n')
  s=len(t());q.key('spc');q.key('ret')
  g['wait'](r'\[spreadeagle\.filter\.choice\] col=B value=all\n',s,120)
  s=len(t());q.key('left');column('A',r'Item\|Food\|Fun\|Rent\|Total\|Per year\|\|',s)
  ctrl_shift('l',r'\[spreadeagle\.column\] sheet=Budget col=A text=Item\|Food\|Fun\|Rent\|Total\|Per year\|\|\n')
  q.key('up');time.sleep(.3);q.key('right');time.sleep(.5)
  ctrl_shift('f',r'\[spreadeagle\.command\] view\.freezePanes\n')
  g['chord']('ctrl','s',r'\[spreadeagle\.saved\] name=welcome\.xlsx bytes=\d+\n')
  close(o)
  s=len(t());o=opened(r'Item\|Food\|Fun\|Rent\|Total\|Per year\|\|')
  g['wait'](r'\[spreadeagle\.view\] charts=2 frozen=1,1\n',s,60)
  # Find, copy, paste, undo.
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'f'}],'hold-time':80});time.sleep(.5)
  s=len(t());type_line('rent');q.key('ret')
  g['wait'](r'\[spreadeagle\.find\] what=rent sheet=Budget cell=A4\n',s,120)
  q.key('esc');time.sleep(.3)
  g['chord']('ctrl','c',r'\[spreadeagle\.command\] edit\.copy\n')
  for _ in range(4):q.key('down');time.sleep(.2)
  s=len(t());g['chord']('ctrl','v',r'\[spreadeagle\.paste\] at=A8\n')
  column('A',r'Item\|Food\|Fun\|Rent\|Total\|Per year\|\|Rent',s)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-paste.ppm')})
  s=len(t());g['chord']('ctrl','z',r'\[spreadeagle\.column\] sheet=Budget col=A text=Item\|Food\|Fun\|Rent\|Total\|Per year\|\|\n')
  # Replace all, a number format, a wider column.
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'h'}],'hold-time':80});time.sleep(.5)
  s=len(t());type_line('food');q.key('ret');type_line('meals');q.key('ret')
  g['wait'](r'\[spreadeagle\.replace\] what=food with=meals cells=1\n',s,120)
  column('A',r'Item\|meals\|Fun\|Rent\|Total\|Per year\|\|',s)
  q.key('esc');time.sleep(.3)
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,('ctrl','home')),(False,('home','ctrl')))for x in ks]});time.sleep(.5)
  q.key('right');time.sleep(.3);q.key('down');time.sleep(.3)
  for k in('down','down'):shift(k)
  ctrl_shift('4',r'\[spreadeagle\.column\] sheet=Budget col=B text=Cost\|\$300\.00 ?\|\$150\.00 ?\|\$40\.00 ?\|490\|5880\|\|\n')
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-currency.ppm')})
  wa=int(re.findall(r'\[spreadeagle\.columns\] A=(\d+) B=\d+\n',t())[-1])
  rw,rh=map(int,a.resolution.split('x'));tile=current_tile(t(),SURFACE[int(o[1])])
  def at(x,y):q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':round((tile[0]+8+x)*32767/(rw-1))}},{'type':'abs','data':{'axis':'y','value':round((tile[1]+36+y)*32767/(rh-1))}}]});time.sleep(.12)
  def btn(down):q.cmd('input-send-event',{'events':[{'type':'btn','data':{'down':down,'button':'left'}}]});time.sleep(.12)
  s=len(t());at(48+wa-1,41);btn(True)
  for dx in range(10,70,10):at(48+wa-1+dx,41)
  btn(False)
  wd=g['wait'](r'\[spreadeagle\.width\] col=A width=(\d+)\n',s,60)
  assert int(wd[1])>wa+30,(wa,wd[0])
  # Resize the first row from its header, then save and reopen the actual XLSX.
  ha=int(re.findall(r'\[spreadeagle\.rows\] 1=(\d+) 2=\d+\n',t())[-1])
  s=len(t());at(24,52+ha-1);btn(True)
  for dy in range(10,60,10):at(24,52+ha-1+dy)
  btn(False)
  ht=g['wait'](r'\[spreadeagle\.height\] row=1 height=(\d+)\n',s,60)
  assert int(ht[1])>ha+30,(ha,ht[0])
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-row-height.ppm')})
  assert (a.out/'spreadeagle-row-height.ppm').stat().st_size>10000
  s=len(t());g['chord']('ctrl','pgdn',r'\[spreadeagle\.column\] sheet=Notes col=A text=Welcome to Spreadeagle, the SexOS spreadsheet\.\|')
  g['chord']('ctrl','pgup',r'\[spreadeagle\.column\] sheet=Budget col=A text=Item\|meals\|Fun\|Rent\|Total\|Per year\|\|')
  g['chord']('ctrl','s',r'\[spreadeagle\.saved\] name=welcome\.xlsx bytes=\d+\n')
  close(o)
  # The previous close was orderly. This time no close or Ctrl+S is sent: only the
  # idle recovery copy can carry the edit through the power cut.
  o=opened(r'Item\|meals\|Fun\|Rent\|Total\|Per year\|\|')
  assert re.search(rf'\[spreadeagle\.rows\] 1={int(ht[1])} 2=\d+\n',t()),'resized row did not reopen'
  s=len(t());q.key('right');q.key('down');type_line('777');q.key('ret')
  g['wait'](r'\[spreadeagle\.set\] cell=B2 input=777\n',s,120)
  column('B',r'Cost\|\$777\.00 ?\|\$150\.00 ?\|\$40\.00 ?\|967\|11604\|\|',s)
  kept=g['wait'](r'\[spreadeagle\.kept\] name=welcome\.xlsx\.recovered\.xlsx\n',s,120)
  assert kept, 'idle recovery never completed'
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-unsaved.ppm')})
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  g['stop']()  # QEMU dies with the app open; no close handler gets to save.
  g=boot(a.iso,disk,a.out,'spreadeagle-reboot',a.resolution);t=g['text'];q=g['q']
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  s=len(t());o=opened(r'Item\|meals\|Fun\|Rent\|Total\|Per year\|\|')
  g['wait'](r'\[spreadeagle\.recovered\]\n',s,60)
  s=len(t());q.key('right');column('B',r'Cost\|\$777\.00 ?\|\$150\.00 ?\|\$40\.00 ?\|967\|11604\|\|',s)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-recovered.ppm')})
  g['chord']('ctrl','s',r'\[spreadeagle\.saved\] name=welcome\.xlsx bytes=\d+\n')
  close(o)
  s=len(t());o=opened(r'Item\|meals\|Fun\|Rent\|Total\|Per year\|\|')
  assert '[spreadeagle.recovered]' not in t()[s:], 'saved workbook should open normally'
  v=len(t());q.key('right');column('B',r'Cost\|\$777\.00 ?\|\$150\.00 ?\|\$40\.00 ?\|967\|11604\|\|',v)
  s=len(t());g['chord']('ctrl','g');type_line('A1:B4');q.key('ret')
  g['wait'](r'\[spreadeagle\.goto\] place=A1:B4 cell=A1\n',s,120)
  ctrl_shift('j',r'\[spreadeagle\.sort\.multi-prompt\] range=A1:B4\n')
  s=len(t());type_line('B desc, A asc');q.key('ret')
  g['wait'](r'\[spreadeagle\.sort\.multi\] range=A1:B4 keys=B desc, A asc\n',s,120)
  time.sleep(.6);q.cmd('screendump',{'filename':str(a.out/'spreadeagle-multikey-sort.ppm')})
  g['chord']('ctrl','z',r'\[spreadeagle\.column\] sheet=Budget col=A text=Item\|meals\|Fun\|Rent\|Total\|Per year\|\|\n')
  close(o)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'saved_bytes':int(m[1]),'powercut_recovered':True})
 finally:g['stop']();server.shutdown()
if __name__=='__main__':main()
