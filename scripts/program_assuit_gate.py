#!/usr/bin/env python3
"""Assuit, the word processor (WordCraft's engine, built for x86_64-sex-std.json):

* `run assuit` writes welcome.docx (hand-written WordprocessingML, apps/assuit/make_welcome.py)
  into its folder, opens it and draws page 1 with ink on it.
* End, then typing " today" adds to the title; the record shows the new first line.
* Shift+Home selects the title line (one highlight rectangle) and Ctrl+B runs format.bold.
* Ctrl+S writes the docx; Ctrl+P writes welcome.pdf beside it; the window closes.
* A second `run assuit` reads the saved file back with the edit in its first line; Ctrl+End
  and fifty Enters push the text onto page 2, which the view draws below page 1.
* Before that, Ctrl+F and "assuit" with Enter select a match and Enter again the next one;
  Esc closes the prompt and Ctrl+C copies the match, which Ctrl+V pastes on page 2.
* Ctrl+H replaces every "assuit" with "assuit writer"; Ctrl+Home and Ctrl+Shift+S open the
  style list on the title's style, Up and Enter make the title a Heading 1.

No kernel panic, page fault or std abort anywhere in the record."""
import argparse,os,re,shutil,time
from pathlib import Path
from install_program import install
os.environ.setdefault('SEXOS_GATE_BOUND','1500')
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
SURFACE={int(a):int(b)for a,b in re.findall(r'app: App::Program, ordinal: \d+, target: (\d+), surface: (\d+)',(ROOT/'crates/sex-pdx/src/app_catalog.rs').read_text())}

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 elf=(ROOT/'iso_root/programs/assuit').read_bytes();assert elf[:4]==b'\x7fELF'
 jot=(ROOT/'iso_root/programs/jot').read_bytes();assert jot[:4]==b'\x7fELF'
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 install(disk,[('assuit',elf),('jot',jot)])
 g=boot(a.iso,disk,a.out,'assuit',a.resolution);t=g['text'];q=g['q']
 def type_line(line):
  for k in line:
   q.key({' ':'spc','.':'dot'}.get(k,k));time.sleep(.05)
 def opened():
  s=len(t());g['focus'](153);time.sleep(.5);type_line('run assuit');q.key('ret')
  o=g['wait'](r'\[assuit\.open\] target=(\d+) task=(\d+) name=welcome\.docx pages=1\n',s,900)
  g['wait'](r'\[assuit\.theme\] mode=Light accent=d6336c',s,60)
  m=g['wait'](r'\[assuit\.page\] page=1 pages=1 width=(\d+) height=(\d+) ink=(\d+) ms=(\d+)\n',s,900)
  assert int(m[3])>3000,f'page 1 looks empty: {m[0]}'
  g['focus'](SURFACE[int(o[1])]);time.sleep(.8)
  return o
 def press(*keys):
  # Down in order, up in reverse, as separate events (send-key can leave Shift held here).
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,keys),(False,keys[::-1]))for x in ks]});time.sleep(.3)
 def close(o):
  s=len(t());q.key('f11');g['wait'](r'\[assuit\.closed\]\n',s,120)
  g['wait'](rf'\[app\.private\.retired\] target={o[1]} task={o[2]} ',s,180)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  o=opened()
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-opened.ppm')})
  shot=(a.out/'assuit-opened.ppm').read_bytes()
  assert shot.startswith(b'P6\n') and shot.count(bytes((0xF6,0xF5,0xF7)))>10000,'Light theme desk missing'
  s=len(t());press('ctrl','shift','slash')
  g['wait'](r'\[assuit\.shortcuts\] shown=1',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-shortcuts.ppm')})
  assert (a.out/'assuit-shortcuts.ppm').stat().st_size>10000
  s=len(t());press('ctrl','shift','slash');g['wait'](r'\[assuit\.shortcuts\] shown=0',s,60)
  s=len(t());q.key('end');type_line(' today')
  g['wait'](r'\[assuit\.text\] chars=\d+ first=Welcome to Assuit today\n',s,300)
  s=len(t());press('shift','home')
  g['wait'](r'\[assuit\.selection\] rects=1 pages=1\n',s,120)
  g['chord']('ctrl','b',r'\[assuit\.command\] format\.bold\n')
  m=g['chord']('ctrl','s',r'\[assuit\.saved\] name=welcome\.docx bytes=(\d+)\n')
  assert int(m[1])>2000,f'saved file is too small: {m[0]}'
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-edited.ppm')})
  # Ctrl+P: SexOS prints to a PDF beside the file.
  s=len(t());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,('ctrl','p')),(False,('p','ctrl')))for x in ks]})
  pdf=g['wait'](r'\[assuit\.pdf\] name=welcome\.pdf bytes=(\d+)\n',s,300)
  assert int(pdf[1])>500,f'PDF is too small: {pdf[0]}'
  s=len(t());press('ctrl','shift','p')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-print-range.ppm')})
  assert (a.out/'assuit-print-range.ppm').stat().st_size>10000
  type_line('1');q.key('ret')
  selected=g['wait'](r'\[assuit\.pdf-range\] pages=1 name=welcome\.pages\.pdf bytes=(\d+)\n',s,300)
  assert int(selected[1])>500,f'page-range PDF is too small: {selected[0]}'
  close(o)
  o=opened()
  s=len(t());q.key('end');press('shift','1')
  g['wait'](r'\[assuit\.text\] chars=\d+ first=Welcome to Assuit today!\n',s,300)
  # Find, then copy the match.
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'f'}],'hold-time':80});time.sleep(.5)
  s=len(t());type_line('assuit');q.key('ret')
  f1=g['wait'](r'\[assuit\.find\] what=assuit match=(\d+) of=(\d+)\n',s,120)
  assert int(f1[2])>=2,f1[0]
  s=len(t());q.key('ret')
  f2=g['wait'](r'\[assuit\.find\] what=assuit match=(\d+) of=(\d+)\n',s,120)
  assert f2[1]!=f1[1],(f1[0],f2[0])
  q.key('esc');time.sleep(.3)
  g['wait'](r'\[assuit\.selection\] rects=1 pages=1\n',s,60)
  g['chord']('ctrl','c',r'\[assuit\.command\] edit\.copy\n')
  copied=g['wait'](r'\[assuit\.clipboard\.copy\] bytes=(\d+) cut=0 ok=1\n',s,60)
  assert int(copied[1])==len('Assuit'),copied[0]
  # Another native app pastes the exact text Assuit published through Shell.
  s=len(t());g['focus'](153);time.sleep(.5);type_line('run jot');q.key('ret')
  jot_ready=g['wait'](r'\[jot\.ready\] target=(\d+) task=(\d+)',s,120)
  jot_launch=g['wait'](rf'\[shell\.program\.launch\] target={jot_ready[1]} task={jot_ready[2]} surface=(\d+)',s,60)
  g['focus'](int(jot_launch[1]));time.sleep(.8)
  s=len(t());press('ctrl','v')
  g['wait'](r'\[jot\.clip\.paste\] bytes=6 system=1 err=None',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-clipboard-in-jot.ppm')})
  assert (a.out/'assuit-clipboard-in-jot.ppm').stat().st_size>10000
  s=len(t());q.key('f11');g['wait'](r'\[jot\.closed\]',s,60)
  g['focus'](SURFACE[int(o[1])]);time.sleep(.5)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'end'}],'hold-time':80});time.sleep(.5)
  for _ in range(50):q.key('ret');time.sleep(.08)
  g['wait'](r'\[assuit\.page\] page=2 pages=2 width=\d+ height=\d+ ink=\d+ ms=\d+\n',s,300)
  g['chord']('ctrl','v',r'\[assuit\.command\] edit\.paste\n')
  g['wait'](r'\[assuit\.clipboard\.paste\] bytes=6 ok=1\n',s,60)
  # Preview uses WordCraft's rendered paper but does not move the editing caret.
  s=len(t());press('ctrl','alt','p')
  g['wait'](r'\[assuit\.preview\] page=2 pages=2 width=\d+ height=\d+',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-print-preview-page2.ppm')})
  assert (a.out/'assuit-print-preview-page2.ppm').stat().st_size>10000
  s=len(t());q.key('left')
  g['wait'](r'\[assuit\.preview\] page=1 pages=2 width=\d+ height=\d+',s,120)
  s=len(t());q.key('esc');g['wait'](r'\[assuit\.preview\.closed\]',s,60)
  # Replace all, then a paragraph style.
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'h'}],'hold-time':80});time.sleep(.5)
  s=len(t());type_line('assuit');q.key('ret');type_line('assuit writer');q.key('ret')
  r=g['wait'](r'\[assuit\.replace\] what=assuit with=assuit writer count=(\d+)\n',s,120)
  assert int(r[1])>=3,r[0]
  q.key('esc');time.sleep(.3)
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'home'}],'hold-time':80});time.sleep(.5)
  s=len(t());keys=('ctrl','shift','s')
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,keys),(False,keys[::-1]))for x in ks]})
  g['wait'](r'\[assuit\.styles\] count=\d+ current=Title\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-styles.ppm')})
  s=len(t());q.key('up');time.sleep(.3);q.key('ret')
  g['wait'](r'\[assuit\.style\] style=Heading 1\n',s,120)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-pages.ppm')})
  # The native page view inserts an editable table through its dimensions prompt.
  q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':'t'}],'hold-time':80});time.sleep(.5)
  s=len(t());type_line('2x2');q.key('ret')
  g['wait'](r'\[assuit\.table\] rows=2 cols=2\n',s,120)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-table.ppm')})
  # Edit the table at its insertion caret and capture the native result.
  s=len(t());press('ctrl','shift','t')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-table-edit-prompt.ppm')})
  assert (a.out/'assuit-table-edit-prompt.ppm').stat().st_size>10000
  type_line('header on');q.key('ret')
  g['wait'](r'\[assuit\.table-edit\] action=header on\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-table-repeat-header.ppm')})
  assert (a.out/'assuit-table-repeat-header.ppm').stat().st_size>10000
  s=len(t());press('ctrl','shift','t');type_line('row below');q.key('ret')
  g['wait'](r'\[assuit\.table-edit\] action=row below\n',s,120)
  s=len(t());press('ctrl','shift','t');type_line('column right');q.key('ret')
  g['wait'](r'\[assuit\.table-edit\] action=column right\n',s,120)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-table-edited.ppm')})
  assert (a.out/'assuit-table-edited.ppm').stat().st_size>10000
  s=len(t());press('ctrl','shift','t');type_line('select row');q.key('ret')
  g['wait'](r'\[assuit\.table-edit\] action=select row\n',s,120)
  s=len(t());press('ctrl','shift','t');type_line('merge cells');q.key('ret')
  g['wait'](r'\[assuit\.table-edit\] action=merge cells\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-table-merged.ppm')})
  s=len(t());press('ctrl','shift','t');type_line('split 2');q.key('ret')
  g['wait'](r'\[assuit\.table-edit\] action=split 2\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-table-split.ppm')})
  s=len(t());press('ctrl','shift','t');type_line('shade ffe8b0');q.key('ret')
  g['wait'](r'\[assuit\.table-edit\] action=shade ffe8b0\n',s,120)
  s=len(t());press('ctrl','shift','t');type_line('border all');q.key('ret')
  g['wait'](r'\[assuit\.table-edit\] action=border all\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-table-styled.ppm')})
  s=len(t());press('ctrl','shift','t');type_line('style grid table 1 light');q.key('ret')
  g['wait'](r'\[assuit\.table-edit\] action=style grid table 1 light\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-table-named-style.ppm')})
  s=len(t());press('ctrl','shift','t');type_line('style');q.key('ret')
  g['wait'](r'\[assuit\.table-gallery.open\] styles=\d+\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-table-gallery.ppm')})
  assert (a.out/'assuit-table-gallery.ppm').stat().st_size>10000
  s=len(t());q.key('down');q.key('ret')
  g['wait'](r'\[assuit\.table-gallery.applied\] name=[^\n]+\n',s,120)
  keys=('ctrl','shift','i')
  q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,keys),(False,keys[::-1]))for x in ks]});time.sleep(.5)
  s=len(t());type_line('assuitmark.png');q.key('ret')
  g['wait'](r'\[assuit\.picture\] path=assuitmark\.png\n',s,120)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-picture.ppm')})
  s=len(t());press('ctrl','shift','r')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-picture-alt-prompt.ppm')})
  type_line('SexOS lips badge');q.key('ret')
  g['wait'](r'\[assuit\.picture-alt\] chars=16\n',s,120)
  s=len(t());press('ctrl','shift','o');type_line('2');q.key('ret')
  g['wait'](r'\[assuit\.picture-width\] inches=2\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-picture-sized.ppm')})
  s=len(t());press('ctrl','shift','c');type_line('10 0 5 0');q.key('ret')
  g['wait'](r'\[assuit\.picture-crop\] values=10 0 5 0\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-picture-cropped.ppm')})
  type_line(' helo');q.key('home')
  s=len(t());press('ctrl','shift','u')
  g['wait'](r'\[assuit\.proofing\.marks\] enabled=0\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-proofing-hidden.ppm')})
  s=len(t());press('ctrl','shift','u')
  g['wait'](r'\[assuit\.proofing\.marks\] enabled=1\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-proofing-marks.ppm')})
  assert (a.out/'assuit-proofing-marks.ppm').stat().st_size>10000
  assert (a.out/'assuit-proofing-hidden.ppm').read_bytes()!=(a.out/'assuit-proofing-marks.ppm').read_bytes()
  keys=('ctrl','shift','y')
  s=len(t());q.cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':x}}}for dn,ks in((True,keys),(False,keys[::-1]))for x in ks]})
  g['wait'](r'\[assuit\.spelling\] word=helo suggestions=\d+\n',s,120)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-spelling.ppm')})
  s=len(t());press('a')
  g['wait'](r'\[assuit\.dictionary\.added\] word=helo\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-dictionary-added.ppm')})
  assert (a.out/'assuit-dictionary-added.ppm').stat().st_size>10000
  s=len(t());press('ctrl','shift','d')
  g['wait'](r'\[assuit\.dictionary\.open\] words=1\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-dictionary-manager.ppm')})
  assert (a.out/'assuit-dictionary-manager.ppm').stat().st_size>10000
  s=len(t());q.key('delete')
  g['wait'](r'\[assuit\.dictionary\.manager\.removed\] word=helo\n',s,120)
  s=len(t());q.key('a');type_line('helo');q.key('ret')
  g['wait'](r'\[assuit\.dictionary\.manager\.added\] word=helo\n',s,120)
  q.key('esc')
  s=len(t());press('ctrl','shift','k')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-proofing-language-prompt.ppm')})
  assert (a.out/'assuit-proofing-language-prompt.ppm').stat().st_size>10000
  type_line('en-GB');q.key('ret')
  g['wait'](r'\[assuit\.proofing-language\] en-GB\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-proofing-language.ppm')})
  s=len(t());press('ctrl','shift','l')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-paper-prompt.ppm')})
  assert (a.out/'assuit-paper-prompt.ppm').stat().st_size>10000
  type_line('a4 landscape');q.key('ret')
  g['wait'](r'\[assuit\.paper\] a4 landscape\n',s,120)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-paper.ppm')})
  assert (a.out/'assuit-paper.ppm').stat().st_size>10000
  s=len(t());press('ctrl','shift','m')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-margins-prompt.ppm')})
  assert (a.out/'assuit-margins-prompt.ppm').stat().st_size>10000
  type_line('narrow');q.key('ret')
  g['wait'](r'\[assuit\.margins\] narrow\n',s,120)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-margins.ppm')})
  assert (a.out/'assuit-margins.ppm').stat().st_size>10000
  s=len(t());press('ctrl','alt','s')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-paragraph-spacing-prompt.ppm')})
  prompt=(a.out/'assuit-paragraph-spacing-prompt.ppm').read_bytes()
  assert prompt.startswith(b'P6\n') and len(prompt)>10000
  type_line('1.5 0 8');q.key('ret')
  g['wait'](r'\[assuit\.paragraph-spacing\] 1\.5 0 8\n',s,120)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-paragraph-spacing.ppm')})
  assert (a.out/'assuit-paragraph-spacing.ppm').stat().st_size>10000
  # A native list action must change the page, and the edited DOCX must save.
  press('ctrl','end');q.key('ret')
  s=len(t());press('ctrl','alt','l')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'assuit-list-prompt.ppm')})
  prompt=(a.out/'assuit-list-prompt.ppm').read_bytes()
  assert prompt.startswith(b'P6\n') and len(prompt)>10000
  type_line('numbered');q.key('ret')
  g['wait'](r'\[assuit\.list\] action=numbered\n',s,120)
  type_line('First item');q.key('ret');type_line('Second item')
  g['wait'](r'\[assuit\.text\] chars=\d+ first=Welcome to Assuit today!\n',s,120)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-numbered-list.ppm')})
  numbered=(a.out/'assuit-numbered-list.ppm').read_bytes()
  assert numbered.startswith(b'P6\n') and numbered!=prompt,'List edit did not change the document view'
  s=len(t());press('ctrl','s')
  g['wait'](r'\[assuit\.saved\] name=welcome\.docx bytes=\d+\n',s,300)
  # The second item must move to a nested list level on the rendered page.
  s=len(t());press('ctrl','alt','l');type_line('level 2');q.key('ret')
  g['wait'](r'\[assuit\.list\] action=level 2\n',s,120)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'assuit-nested-list.ppm')})
  nested=(a.out/'assuit-nested-list.ppm').read_bytes()
  assert nested.startswith(b'P6\n') and nested!=numbered,'Nested level did not change the document view'
  s=len(t());press('ctrl','s')
  g['wait'](r'\[assuit\.saved\] name=welcome\.docx bytes=\d+\n',s,300)
  close(o)
  text=t();assert 'KERNEL PANIC' not in text and '[std.abort]' not in text and 'KERNEL PAGE FAULT' not in text
  print({'ok':True,'saved_bytes':int(m[1])})
 finally:g['stop']()
if __name__=='__main__':main()
