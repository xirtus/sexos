#!/usr/bin/env python3
"""Spindle `calc`: typed expressions give exact decimal answers on screen."""
import argparse,hashlib,json,re,shutil
from pathlib import Path
from network_contention_gate import frozen_binding
from quil_autosave_gate import boot,settle

SHIFTED={'*':'8','(':'9',')':'0','+':'equal','%':'5'}
PLAIN={' ':'spc','-':'minus','/':'slash','.':'dot'}
CASES=[('calc ans',r'\[spindle.calc\] expr_bytes=3 error=NoAnswer ok=0'),
       ('calc 2*(3+4)/5',r'\[spindle.calc\] expr_bytes=9 result== 2\.8 ok=1'),
       ('calc ans*10',r'\[spindle.calc\] expr_bytes=6 result== 28 ok=1'),
       ('calc 0.1+0.2',r'\[spindle.calc\] expr_bytes=7 result== 0\.3 ok=1'),
       ('calc 1/3',r'\[spindle.calc\] expr_bytes=3 result=~ 0\.333333333 ok=1'),
       ('calc 2**10',r'\[spindle.calc\] expr_bytes=5 result== 1024 ok=1'),
       ('calc sqrt(2)',r'\[spindle.calc\] expr_bytes=7 result=~ 1\.414213562 ok=1'),
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
  g['q'].cmd('screendump',{'filename':str(out/'calc.ppm')})
 except Exception as exc:failure=repr(exc)
 finally:
  if g:g['stop']()
 result=dict(ok=failure is None,error=failure,binding=binding,resolution=a.resolution,cases=rows);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if failure:raise RuntimeError(failure)
 print(json.dumps(dict(ok=True,cases=len(rows))))

if __name__=='__main__':main()
