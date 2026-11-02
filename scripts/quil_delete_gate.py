#!/usr/bin/env python3
"""Quil Ctrl+D deletes the open saved document, confirmed by a second press.

Save a new document under a fresh name, press Ctrl+D (asks), type nothing,
press Ctrl+D again: sexfiles deletes the object through the held document
route, Quil keeps the text as an unsaved UNTITLED buffer, and Linen no longer
lists the name, also after a power cut and reboot.
"""
import argparse,hashlib,json,re,shutil,time
from pathlib import Path
from network_contention_gate import frozen_binding
from check_desktop_pixels import final_view
from quil_autosave_gate import boot,type_keys,settle

NAME='gone'
SCOPE='Native QEMU delete of a Quil-saved object through the held document route; Linen listing checked live and after a power cut.'

def linen_labels(g):
 g['focus'](201);g['chord']('ctrl','o',r'\[shell.document.open\] target=9 linen=200');g['focus'](200)
 view,labels=final_view(g['text']());return [label for _,_,label in labels]

def main():
 p=argparse.ArgumentParser();[p.add_argument(f'--{n}',required=True,type=Path)for n in('iso','source-disk','binding','out')];p.add_argument('--resolution',choices=('1280x800','1024x768'),default='1280x800');a=p.parse_args()
 root=Path(__file__).resolve().parents[1];out=a.out.resolve();out.mkdir(parents=True,exist_ok=True)
 if any(out.iterdir()):raise RuntimeError('fresh evidence directory required')
 iso=a.iso.resolve(strict=True);binding=frozen_binding(root,iso,a.binding.resolve(strict=True));held=json.loads(a.binding.read_text())
 for name in('scripts/quil_delete_gate.py','scripts/quil_autosave_gate.py'):
  if held['runner_sha256'].get(name)!=hashlib.sha256((root/name).read_bytes()).hexdigest():raise RuntimeError(name+' absent from frozen binding')
 disk=out/'nvme.img';shutil.copyfile(a.source_disk,disk);result=dict(binding=binding,resolution=a.resolution,scope=SCOPE);failure=None;g=None
 try:
  g=boot(iso,disk,out,'boot1',a.resolution);t=g['text']
  g['focus'](201);g['chord']('ctrl','n',r'\[quil.new.ok\] bytes=0');type_keys(g,['d','e','l','spc','m','e'])
  s=len(t());g['q'].cmd('input-send-event',{'events':[{'type':'key','data':{'down':dn,'key':{'type':'qcode','data':k}}}for dn,ks in((True,('ctrl','shift','s')),(False,('s','shift','ctrl')))for k in ks]})
  g['wait'](r'\[quil.name.mode\] on=1',s)
  s=len(t());g['q'].key_sequence(list(NAME));g['wait'](r'\[quil.text.view\][^\n]*status=Name: '+NAME,s)
  saved=g['key']('ret',r'\[quil.persist.save.ok\] bytes=6 hash=(0x[0-9a-f]+) doc=(\d+)');obj=int(saved[2])
  g['wait'](r'\[quil.document.saved\][^\n]*dirty=0',0)
  before=linen_labels(g)
  if NAME not in before:raise RuntimeError(f'saved name not listed: {before}')
  g['focus'](201)
  g['chord']('ctrl','d',rf'\[quil.document.delete.ask\] object={obj} revision=\d+')
  ask=len(t());settle(g,60);g['q'].cmd('screendump',{'filename':str(out/'delete-ask.ppm')})
  if re.search(r'\[files.document.deleted\]',t()):raise RuntimeError('first Ctrl+D deleted without confirmation')
  g['chord']('ctrl','d',rf'\[files.document.deleted\] target=9 object={obj} generation=\d+')
  g['wait'](rf'\[quil.document.deleted\] object={obj} current=1 text_kept=6',ask)
  g['wait'](r'\[quil.text.view\][^\n]*DELETED / TEXT KEPT UNSAVED',ask);settle(g,60);g['q'].cmd('screendump',{'filename':str(out/'deleted.ppm')})
  time.sleep(2);after=linen_labels(g)
  if NAME in after:raise RuntimeError(f'deleted name still listed: {after}')
  result['boot1']=dict(object=obj,saved_hash=saved[1],listed_before=before,listed_after=after)
  g['stop']();g=None
  g=boot(iso,disk,out,'boot2',a.resolution)
  rebooted=linen_labels(g)
  if NAME in rebooted:raise RuntimeError(f'deleted name back after reboot: {rebooted}')
  result['boot2']=dict(listed=rebooted)
 except Exception as exc:failure=repr(exc)
 finally:
  if g:g['stop']()
 result.update(ok=failure is None,error=failure);(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
 if failure:raise RuntimeError(failure)
 print(json.dumps(dict(ok=True,object=result['boot1']['object'],after=result['boot1']['listed_after'],rebooted=result['boot2']['listed'])))

if __name__=='__main__':main()
