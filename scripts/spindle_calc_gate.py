#!/usr/bin/env python3
"""Spindle `calc`: typed expressions give exact decimal answers on screen."""
import argparse,hashlib,json,re,shutil
from pathlib import Path
from network_contention_gate import frozen_binding
from quil_autosave_gate import boot,settle

SHIFTED={'*':'8','(':'9',')':'0','+':'equal','%':'5'}
PLAIN={' ':'spc','-':'minus','/':'slash','.':'dot',',':'comma','=':'equal'}
CASES=[('calc ans',r'\[spindle.calc\] expr_bytes=3 error=NoAnswer ok=0'),
       ('calc 2*(3+4)/5',r'\[spindle.calc\] expr_bytes=9 result== 2\.8 ok=1'),
       ('calc ans*10',r'\[spindle.calc\] expr_bytes=6 result== 28 ok=1'),
       ('calc 0.1+0.2',r'\[spindle.calc\] expr_bytes=7 result== 0\.3 ok=1'),
       ('calc 1/3',r'\[spindle.calc\] expr_bytes=3 result=~ 0\.333333333 ok=1'),
       ('calc 2**10',r'\[spindle.calc\] expr_bytes=5 result== 1024 ok=1'),
       ('calc sqrt(2)',r'\[spindle.calc\] expr_bytes=7 result=~ 1\.414213562 ok=1'),
       ('calc max(2,9)',r'\[spindle.calc\] expr_bytes=8 result== 9 ok=1'),
       ('calc 2*pi',r'\[spindle.calc\] expr_bytes=4 result=~ 6\.283185308 ok=1'),
       ('calc r = 3',r'\[spindle.calc\] expr_bytes=5 result== 3 ok=1'),
       ('calc r*r+1',r'\[spindle.calc\] expr_bytes=5 result== 10 ok=1'),
       ('calc 0xff+1',r'\[spindle.calc\] expr_bytes=6 result== 256 ok=1'),
       ('calc ans in hex',r'\[spindle.calc\] expr_bytes=10 result== 0x100 ok=1'),
       ('calc 1/2 in bin',r'\[spindle.calc\] expr_bytes=10 error=NotWhole ok=0'),
       ('calc ln(2)',r'\[spindle.calc\] expr_bytes=5 result=~ 0\.693147181 ok=1'),
       ('calc 2**0.5',r'\[spindle.calc\] expr_bytes=6 result=~ 1\.414213562 ok=1'),
       ('calc 7%0',r'\[spindle.calc\] expr_bytes=3 error=DivideByZero ok=0')]

def type_line(g,value):
 for i,c in enumerate(value,1):
  keys=('shift',SHIFTED[c])if c in SHIFTED else(PLAIN.get(c,c),)
  s=len(g['text']());g['q'].cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':k}}}for dn,ks in((True,keys),(False,keys[::-1]))for k in ks]})
  g['wait'](rf'\[spindle.cursor.view\] target=12 sequence=\d+ pos={i} len={i}(?:\s|$)',s)

def main():
 p=argparse.ArgumentParser();[p.add_argument(f'--{n}',required=True,type=Path)for n in('iso','source-disk','binding','out')];p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 iso=a.iso.resolve(strict=True);binding=frozen_binding(root,iso,a.binding.resolve(strict=True));held=json.loads(a.binding.read_text())
 for name in('scripts/spindle_calc_gate.py','scripts/quil_autosave_gate.py'):
  if held['runner_sha256'].get(name)!=hashlib.sha256((root/name).read_bytes()).hexdigest():raise RuntimeError(name+' absent from frozen binding')
 disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);rows=[];failure=None;g=None
 try:
  g=boot(iso,disk,out,'boot',a.resolution);g['focus'](153)
  for line,pattern in CASES:
   type_line(g,line);s=len(g['text']());g['q'].key('ret');m=g['wait'](pattern,s);rows.append(dict(input=line,record=m[0]))
  # Tab completes a command name: unique (`sext` -> `sextime `), then shared
  # prefix (`proj` -> `project-scene-`), then a second Tab lists both.
  type_line(g,'sext');s=len(g['text']());g['q'].key('tab');g['wait'](r'\[spindle.complete\] matches=1 added=4 len=8',s)
  s=len(g['text']());g['q'].key('0');g['wait'](r'\[spindle.cursor.view\] target=12 sequence=\d+ pos=9 len=9(?:\s|$)',s)
  s=len(g['text']());g['q'].key('ret');rows.append(dict(input='sext<Tab>0',record=g['wait'](r'\[spindle.sextime\] [^\n]* ok=1',s)[0]))
  type_line(g,'proj');s=len(g['text']());g['q'].key('tab');rows.append(dict(input='proj<Tab>',record=g['wait'](r'\[spindle.complete\] matches=2 added=10 len=14',s)[0]))
  s=len(g['text']());g['q'].key('tab');g['wait'](r'\[spindle.complete\] matches=2 added=0 len=14',s)
  for _ in range(14):
   s=len(g['text']());g['q'].key('backspace');g['wait'](r'\[spindle.line.backspace\]',s)
  # Ctrl editing: Ctrl+W drops a word, Ctrl+U the line, Ctrl+R recalls history.
  type_line(g,'echo ab cd');g['chord']('ctrl','w',r'\[spindle.line.ctrl\] key=w len=8 cur=8')
  g['chord']('ctrl','u',r'\[spindle.line.ctrl\] key=u len=0 cur=0')
  type_line(g,'0xff');g['chord']('ctrl','r',r'\[spindle.line.ctrl\] key=r len=11 cur=11')
  s=len(g['text']());g['q'].key('ret');rows.append(dict(input='0xff<Ctrl+R>',record=g['wait'](r'\[spindle.calc\] expr_bytes=6 result== 256 ok=1',s)[0]))
  # `history ln` lists the one calc line containing "ln".
  type_line(g,'history ln');s=len(g['text']());g['q'].key('ret');rows.append(dict(input='history ln',record=g['wait'](r'\[spindle.history.search\] bytes=2 found=1',s)[0]))
  # Spindle's display outbox must drain: its last batch is admitted after the last command.
  last=len(g['text']());import time as _t;d=_t.monotonic()+30
  while _t.monotonic()<d and not re.search(r'\[spindle.display.batch.admitted\]',g['text']()[last-20000 if last>20000 else 0:]):_t.sleep(.2)
  _t.sleep(2)
  g['q'].cmd('screendump',{'filename':str(out/'calc.ppm')})
 except Exception as exc:failure=repr(exc)
 finally:
  if g:g['stop']()
 result=dict(ok=failure is None,error=failure,binding=binding,resolution=a.resolution,cases=rows);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if failure:raise RuntimeError(failure)
 print(json.dumps(dict(ok=True,cases=len(rows))))

if __name__=='__main__':main()
