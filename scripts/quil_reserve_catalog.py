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


def surface_families(root):
 """Closed literal catalog view for visual proof only; grants remain separate."""
 text=(Path(root)/'crates/sex-pdx/src/app_catalog.rs').read_text()
 head=re.search(r'pub const INCARNATIONS: \[Incarnation; (\d+)\] = \[',text)
 if not head:raise ValueError('catalog visual inventory declaration missing')
 body=text[head.end():text.index('\n];',head.end())];families={};count=0
 def add(family,surfaces):
  for sid in set(surfaces):
   if sid==200 or sid in families:raise ValueError('ambiguous app visual surface')
   families[sid]=family
 for row in quil_catalog(root):add('Quil',[row['surface'],row['frame']]);count+=1
 for line in body.splitlines():
  if 'Incarnation'not in line:continue
  family=re.search(r'app\s*:\s*App::(\w+)',line)
  surface=re.search(r'\bsurface\s*:\s*(\d+)',line)
  frame=re.search(r'\bshell_surface\s*:\s*(None|Some\((\d+)\))',line)
  children=re.search(r'\bchildren\s*:\s*&\[([\d,\s]*)\]',line)
  if not all([family,surface,frame,children])or family[1]not in ['Spindle','Browser']:raise ValueError('unparsed app visual row')
  sids=[int(surface[1])]+([int(frame[2])]if frame[2]else[])+[int(value)for value in children[1].split(',')if value.strip()]
  add(family[1],sids);count+=1
 if count!=int(head[1]):raise ValueError('incomplete app visual inventory')
 return families
