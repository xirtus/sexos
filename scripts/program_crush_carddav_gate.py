#!/usr/bin/env python3
"""Crush (contacts) against a CardDAV server on the host (scripts/dav_fixture.py): it
imports a vCard file from a phone (3.0, 4.0 and 2.1 cards), takes an account file,
finds the server's address book, pulls its cards and pushes the imported ones, finds
a card by typing, adds one with the form and pushes it (the server sees the vCard),
pulls a card changed on the server, removes one here and the server deletes it,
exports every card, and on its next run has them all and nothing to sync."""
import argparse,os,re,shutil,sys,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(Path(__file__).resolve().parent))
from dav_fixture import Dav,serve
BOOK='/dav/addressbooks/alice/contacts/'
dav=Dav('/dav/addressbooks/alice/',BOOK,'card',name='Contacts')
PORT=serve(dav)
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from install_program import install
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','+':('shift','equal'),':':('shift','semicolon'),'@':('shift','2')}
def card(uid,fn,email,extra=''):return f'BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:{fn}\r\nEMAIL;TYPE=work:{email}\r\n{extra}END:VCARD\r\n'
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('crush',(ROOT/'iso_root/programs/crush').read_bytes())])
 dav.files['/account.dav']=f'url=http://10.0.2.2:{PORT}/dav/addressbooks/alice/\nuser=alice\npassword=wonderland\n'.encode()
 dav.files['/phone.vcf']=(ROOT/'crates/sex-vcard/tests/fixture.vcf').read_bytes()
 dav.put('alan.vcf',card('alan@fixture','Alan Turing','alan@bletchley.example','TEL;TYPE=cell:+44 1908 640404\r\n'))
 dav.put('emmy.vcf',card('emmy@fixture','Emmy Noether','emmy@goettingen.example'))
 g=boot(a.iso,disk,a.out,'crush',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   v=KEYS.get(k,k)
   if k.isupper():v=('shift',k.lower())
   if isinstance(v,tuple):q.cmd('send-key',{'keys':[{'type':'qcode','data':x}for x in v],'hold-time':80})
   else:q.key(v)
   time.sleep(.08)
 def ctrl(k):q.cmd('send-key',{'keys':[{'type':'qcode','data':'ctrl'},{'type':'qcode','data':k}],'hold-time':80})
 def spindle(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def start(args):
  s=len(t());spindle(f'run +net crush{args}',r'\[shell.program.launch\]')
  ready=g['wait'](r'\[crush.ready\] target=(\d+) task=(\d+) contacts=(\d+)',s,120)
  launch=g['wait'](rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)',s,60)
  g['wait'](rf'\[display.app.pixels\] target={ready[1]} ',s,60);time.sleep(2)
  g['focus'](int(launch[1]));time.sleep(.5)
  return ready[1],ready[2],s
 def close(target,task):
  s=len(t());q.key('f11');g['wait'](rf'\[crush.closed\] target={target} task={task}',s,60);g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
 def sync(pattern):s=len(t());ctrl('s');return g['wait'](pattern,s,120)
 def shot(name):time.sleep(1);q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')});time.sleep(.5)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  for f in ['account.dav','phone.vcf']:
   m=spindle(f'fetch http://10.0.2.2:{PORT}/{f} {f}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  target,task,s=start(' @account.dav @phone.vcf')
  g['wait'](r'\[crush.account\] url=http://10.0.2.2:\d+/dav/addressbooks/alice/ user=alice\n',s,60)
  g['wait'](r'\[crush.imported\] cards=3 added=3\n',s,60)
  # First sync: the address book is found, two cards come down, three go up.
  s=len(t());ctrl('s')
  g['wait'](rf'\[crush.sync.found\] books=1 path={BOOK}\n',s,120)
  g['wait'](r'\[crush.sync\] unchanged=0 incremental=0 pulled=2 removed=0 pushed=3 deleted=0 conflicts=0 failed=0 contacts=5\n',s,120)
  assert len(dav.items)==5,dav.items.keys()
  ada=[d for p,(n,d) in dav.items.items() if 'Ada Lovelace' in d];assert ada and 'ada@engines.example' in ada[0] and 'X-SEXOS' not in ada[0],ada
  assert all(r[2].get('Authorization')==dav.auth for r in dav.requests if r[1].startswith('/dav/'))
  # Search by typing.
  s=len(t());keys('turing');g['wait'](r'\[crush.search\] query=turing matches=1\n',s,30);shot('crush-search')
  q.key('esc');time.sleep(.3)
  s=len(t());keys('1908');g['wait'](r'\[crush.search\] query=1908 matches=1\n',s,30);q.key('esc');time.sleep(.3)
  # A new card with the form: name, Down, email, Down, phone, Enter (Tab belongs to the desktop).
  s=len(t());ctrl('n');time.sleep(.3);keys('Rosalind Franklin');q.key('down');keys('rosalind@kings.example');q.key('down');keys('+44 20 7836 5454');time.sleep(.2);shot('crush-form');q.key('ret')
  g['wait'](r'\[crush.contact.saved\] name=Rosalind Franklin email=rosalind@kings.example phone=\+44 20 7836 5454 new=1\n',s,30)
  # The server changes Alan's card meanwhile; one sync both ways.
  dav.put('alan.vcf',card('alan@fixture','Alan M. Turing','alan@bletchley.example'))
  sync(r'\[crush.sync\] unchanged=0 incremental=1 pulled=1 removed=0 pushed=1 deleted=0 conflicts=0 failed=0 contacts=6\n')
  ros=[d for p,(n,d) in dav.items.items() if 'Rosalind' in d];assert ros and 'EMAIL:rosalind@kings.example' in ros[0] and 'TEL:+44 20 7836 5454' in ros[0],ros
  # Remove Emmy here: the server deletes her card.
  s=len(t());keys('noether');g['wait'](r'\[crush.search\] query=noether matches=1\n',s,30)
  s=len(t());q.key('delete');g['wait'](r'\[crush.removed\] name=Emmy Noether\n',s,30);q.key('esc');time.sleep(.3)
  sync(r'\[crush.sync\] unchanged=0 incremental=\d pulled=0 removed=0 pushed=0 deleted=1 conflicts=0 failed=0 contacts=5\n')
  assert not any('Emmy' in d for n,d in dav.items.values()) and len(dav.items)==5,dav.items.keys()
  # Our own writes moved the ctag: one more look settles it.
  sync(r'\[crush.sync\] unchanged=0 incremental=1 pulled=0 removed=0 pushed=0 deleted=0 conflicts=0 failed=0 contacts=5\n')
  s=len(t());ctrl('e');g['wait'](r'\[crush.exported\] cards=5 bytes=\d+\n',s,30);shot('crush-list')
  close(target,task)
  s0=len(t());target,task,s=start('')
  g['wait'](r'\[crush.loaded\] contacts=5\n',s0,60)
  sync(r'\[crush.sync\] unchanged=1 ')
  close(target,task)
  text=t();assert 'KERNEL PANIC' not in text and '[crush.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'server_cards':len(dav.items),'requests':len(dav.requests)})
 finally:g['stop']()
if __name__=='__main__':main()
