#!/usr/bin/env python3
"""Network time (B5): the machine's clock starts far off (QEMU's RTC set to March 2024)
and comes right from the network. Once a program brings the network up, sexnet asks
pool.ntp.org (SNTP, over IPv6 to the peer of program_ipv6_gate, whose clock is the
host's) and sets civil time; it must land within 1 s of the host. A second check, after
the clock is right, takes no step."""
import argparse,os,re,shutil,time
from pathlib import Path
os.environ['SEXOS_GATE_RTC']='base=2024-03-01T00:00:00,clock=host,driftfix=none'
import program_ipv6_gate as six
from install_program import install
from quil_autosave_gate import boot
ROOT=Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('flirt',(ROOT/'iso_root/programs/flirt').read_bytes())])
 g=boot(a.iso,disk,a.out,'network-time',a.resolution);t=g['text'];q=g['q']
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  g['focus'](153);time.sleep(.5);s=len(t())
  for k in f'run +net flirt check {six.text6(six.PEER)} 7':
   if k in ':+':q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':{':':'semicolon','+':'equal'}[k]}],'hold-time':80})
   else:q.key(six.KEYS.get(k,k))
   time.sleep(.06)
  q.key('ret')
  m=g['wait'](r'\[clock.civil.set\] pd=\d+ ms=(\d+) step_ms=(-?\d+) offset_ms=(-?\d+)',s,180)
  host=time.time()*1000;set_ms=int(m[1])
  assert abs(set_ms-host)<1000,f'set to {set_ms}, host {host:.0f}: off by {set_ms-host:.0f} ms'
  assert int(m[2])>60*86400*1000,f'step {m[2]} ms is not the skew'
  r=g['wait'](r'\[net.time.synced\] offset_ms=(-?\d+) delay_ms=(\d+) stratum=2 stepped=1',s,30)
  assert six.peer.ntp,'the peer saw no SNTP request'
  assert (six.TIME_SERVER,28) in six.peer.dns,six.peer.dns
  text=t()
  assert 'KERNEL PANIC' not in text and 'panicked' not in text and '[net.time.failed]' not in text[s:],'failure'
  print({'ok':True,'step_ms':int(m[2]),'error_ms':round(set_ms-host),'delay_ms':int(r[2]),'requests':len(six.peer.ntp)})
 finally:g['stop']()
if __name__=='__main__':main()
