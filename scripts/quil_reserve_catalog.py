"""Source-bound host descriptors for BOOT Quil test profiles; never authority."""
import re
from pathlib import Path


def quil_catalog(root):
 text=(Path(root)/'crates/sex-pdx/src/app_catalog.rs').read_text()
 pattern=r'^\s*quil\((\d+),\s*(\d+),\s*(\d+),\s*(None|Some\(\d+\)),\s*(\d+),\s*(\d+),\s*(None|Some\(\d+\)),\s*(\d+)\),\s*$'
 rows=[]
 for match in re.finditer(pattern,text,re.M):
  ordinal,target,surface,frame,life,close,activation,route=match.groups();rows.append(dict(ordinal=int(ordinal),target=int(target),surface=int(surface),frame=int(re.search(r'\d+',frame)[0])if frame!='None'else int(surface),lifecycle=int(life),close=int(close),activation=None if activation=='None'else int(re.search(r'\d+',activation)[0]),route=int(route)))
 if not rows or [r['ordinal']for r in rows]!=list(range(len(rows)))or len({r['target']for r in rows})!=len(rows):raise ValueError('source Quil catalog malformed or incomplete')
 return rows


def verify_boot_quil(text,rows):
 found=[]
 for m in re.finditer(r'^\[boot.app.catalog\] (.+)$',text,re.M):
  d={k:int(v)for k,v in re.findall(r'(\w+)=(\d+)',m[1])}
  if d.get('logical')==rows[0]['target']:found.append(d)
 if len(found)!=len(rows):raise ValueError('missing/duplicate BOOT Quil audit row')
 for index,(r,d)in enumerate(zip(rows,found)):
  wanted=dict(target=r['target'],task=r['target'],surface=r['surface'],logical=rows[0]['target'],incarnation=index+1,predecessor=rows[index-1]['target']if index else 0,lifecycle=r['lifecycle'],close=r['close'],route=r['route'],dormant=int(index!=0),grants_verified=1,data_only=1)
  if d!=wanted:raise ValueError('BOOT Quil tuple differs from sealed source catalog')
