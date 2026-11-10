#!/usr/bin/env python3
"""The screen shows how a start goes and why it stopped, and the next start says so.

Needs an ISO built with SEXOS_CRASH_PROBE=1 SEXOS_STOP_RESTART_SECONDS=8.

Start 1: the boot log is drawn on the framebuffer before Display starts. In Spindle,
`crash Network` ends the network service as a fault would: the kernel records the fault
and the restart, and Files writes both to the reserved ring on disk. `crash kernel` stops
the kernel: the stop screen (red, with the reason and the log's last lines) fills the
framebuffer, and after the countdown the machine restarts itself.
Start 2 (warm restart, same RAM): the kernel finds the kept stop, pins it at the top of
the boot log, and Files writes it to disk next to the two earlier records.
Start 3 (cold, QEMU started again, RAM gone): Files reads all three back from disk and
`crashes` in Spindle lists them as from an earlier start. Then Shell ends: Shell and
Display start again in place, the running apps keep their windows, the layout and
every window's place come back, and Spindle works.
Start 1 also ends Display alone: Shell puts the same desktop back on the screen.
"""
import argparse,os,re,shutil,subprocess,time
from pathlib import Path
from desktop_close_gate import QMP
from desktop_focus import focus_surface
from check_disk_persistence import ppm_rgb
from certificate_time_gate import RTC_NORMAL

KEYS={' ':'spc','/':'slash','.':'dot','-':'minus'}
SPINDLE=153
BOOT_BG=(0x10,0x14,0x1a);BAR_BG=(0x2a,0x3a,0x5a);NOTE_BG=(0x5a,0x3a,0x10);STOP_BG=(0x4a,0x0c,0x14);INK=(0xc8,0xd0,0xdc)

def start(iso,disk,out,tag):
 serial=out/f'{tag}-serial.log';sock=Path('/tmp')/f'sex-crash-{os.getpid()}-{tag}.sock'
 # No -no-reboot: the stop screen restarts the machine and the gate watches the next start.
 cmd=['qemu-system-x86_64','-M','q35','-cpu','max,+pku,+rdseed','-rtc',RTC_NORMAL,'-smp',os.environ.get('SEXOS_GATE_CORES','1'),'-m',os.environ.get('SEXOS_GATE_MEMORY','512M'),'-vga','none','-device','VGA,xres=1280,yres=800','-device','nec-usb-xhci,id=xhci','-device','usb-tablet,bus=xhci.0','-cdrom',str(iso),*(['-drive','if=pflash,format=raw,readonly=on,file='+os.environ['SEXOS_GATE_FIRMWARE']]if os.environ.get('SEXOS_GATE_FIRMWARE')else[]),'-drive',f'if=none,id=nvm,file={disk},format=raw','-device','nvme,serial=sexos01,drive=nvm','-serial',f'file:{serial}','-qmp',f'unix:{sock},server=on,wait=off','-display','none','-nic','none']
 proc=subprocess.Popen(cmd,stdout=(out/f'{tag}-qemu.log').open('w'),stderr=subprocess.STDOUT);began=time.monotonic()
 def text():return serial.read_text(errors='replace') if serial.exists() else ''
 def wait(pattern,s=0,timeout=180):
  d=time.monotonic()+timeout
  while time.monotonic()<d:
   if proc.poll() is not None:raise RuntimeError(tag+' guest exited')
   if time.monotonic()-began>int(os.environ.get('SEXOS_GATE_BOUND','600')):raise TimeoutError(tag+' over its time bound')
   m=re.search(pattern,text()[s:])
   if m:return m
   time.sleep(.02)
  raise TimeoutError(f'{tag}: {pattern}')
 d=time.monotonic()+15
 while not sock.exists():
  if time.monotonic()>d:raise TimeoutError('QMP unavailable')
  time.sleep(.05)
 q=QMP(sock)
 def key(name,pattern):s=len(text());q.key(name);return wait(pattern,s)
 def stop():
  if proc.poll() is None:proc.kill();proc.wait()
  sock.unlink(missing_ok=True)
 return dict(q=q,text=text,wait=wait,key=key,stop=stop,focus=lambda sid:focus_surface(sid,text,key))

def shot(g,path):
 g['q'].cmd('screendump',{'filename':str(path)});time.sleep(.3)
 w,h,px=ppm_rgb(path.read_bytes(),min_width=1,min_height=1)
 return w,h,px
def count(px,colour):
 c=bytes(colour);return sum(1 for i in range(0,len(px),3*7) if px[i:i+3]==c)
def frac(px,colour):return count(px,colour)/(len(px)//(3*7))

def same(a,b):
 _,_,x=ppm_rgb(a.read_bytes(),min_width=1,min_height=1);_,_,y=ppm_rgb(b.read_bytes(),min_width=1,min_height=1)
 # Every 7th pixel; the bar's clock and the cursor may differ.
 n=len(x)//(3*7);return sum(1 for i in range(0,len(x),3*7) if x[i:i+3]==y[i:i+3])/n

def desktop(g,s=0):
 g['wait'](r'\[shell.app.registered.reply\] target=15 .*committed=1',s,240);g['wait'](r'\[linen.disk.publish.done\]',s,240)
 g['wait'](r'\[app.private.retired\] target=41 ',s,240);time.sleep(2)

def chord(g,keys,action):
 # Press in order, release in reverse; the Shell records the action it took.
 send=lambda down,k:g['q'].cmd('input-send-event',{'events':[{'type':'key','data':{'down':down,'key':{'type':'qcode','data':k}}}]})
 s=len(g['text']())
 for k in keys:send(True,k);time.sleep(.08)
 for k in reversed(keys):send(False,k);time.sleep(.08)
 return g['wait'](r'\[shell.wm.action\] action='+re.escape(action)+r' done=1 scene=\d->\d focus=\d+->\d+ layout=(\w+)',s,30)

def spindle(g,command,pattern,timeout=60):
 g['focus'](SPINDLE);time.sleep(.5);s=len(g['text']())
 for k in command:g['q'].key(KEYS.get(k,k));time.sleep(.05)
 g['q'].key('ret');return g['wait'](pattern,s,timeout)

def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--out',required=True,type=Path);a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(max(f.seek(0,2),1024**3))
 evidence={}
 g=start(a.iso,disk,a.out,'warm')
 try:
  # The boot log, before Display has the screen.
  g['wait'](r'\[crash.pstore\] available=1 found=0',0,120);time.sleep(.3)
  _,_,px=shot(g,a.out/'boot-log.ppm')
  evidence['boot_log']=dict(background=round(frac(px,BOOT_BG),3),bar=round(frac(px,BAR_BG),3),ink=count(px,INK))
  assert evidence['boot_log']['background']>.5 and evidence['boot_log']['bar']>.01 and evidence['boot_log']['ink']>100,evidence
  desktop(g)
  g['wait'](r'\[files.crash.ring\] readable=1 kept=0',0,60)
  # A service fault: recorded, restarted, kept on disk.
  spindle(g,'crash network',r'\[crash.probe.service\] pd=13')
  g['wait'](r'\[crash.record\] seq=\d+ kind=2 pd=13 ',0,60);g['wait'](r'\[service.restarted\] pd=13 ',0,60)
  g['wait'](r'\[files.crash.kept\] slot=\d+ index=\d+ seq=\d+ kind=4 pd=13',0,60)
  m=spindle(g,'crashes',r'\[spindle.crashes\] count=(\d+)');assert int(m[1])>=2,m[0]
  # Input and Bell start again in place: keys still reach Spindle, and a beep plays.
  spindle(g,'crash input',r'\[service.restarted\] pd=4 ');g['wait'](r'\[sexinput.restarted\] restarts=1',0,60);time.sleep(1)
  spindle(g,'crash bell',r'\[service.restarted\] pd=10 ');g['wait'](r'\[sexbell.restarted\] restarts=1',0,60);time.sleep(1)
  spindle(g,'beep',r'\[spindle.beep\] hz=880 ms=200 volume=30 status=0')
  # Display starts again in place: Shell shows the desktop again and the windows come back.
  _,_,px=shot(g,a.out/'before-display.ppm');before=frac(px,BOOT_BG)
  s=len(g['text']());spindle(g,'crash display',r'\[service.restarted\] pd=1 ')
  m=g['wait'](r'\[shell.display.back\] times=1 windows=(\d+)',s,60);assert int(m[1])>=3,m[0]
  time.sleep(4);_,_,px=shot(g,a.out/'after-display.ppm')
  evidence['display_restart']=dict(windows=int(m[1]),same_pixels=round(same(a.out/'before-display.ppm',a.out/'after-display.ppm'),3))
  assert evidence['display_restart']['same_pixels']>.9,evidence
  spindle(g,'crashes',r'\[spindle.crashes\] count=\d+')
  # The kernel stops: the stop screen, then a restart by itself.
  s=len(g['text']());spindle(g,'crash kernel',r'\[crash.pstore.kept\]')
  time.sleep(1.5);_,_,px=shot(g,a.out/'stop-screen.ppm')
  evidence['stop']=dict(background=round(frac(px,STOP_BG),3),white=count(px,(255,255,255)))
  assert evidence['stop']['background']>.5 and evidence['stop']['white']>200,evidence
  assert 'crash probe: the kernel was asked to stop' in g['text']()[s:]
  g['wait'](r'\[console.stop.restart\]',s,60)
  # The warm start shows what stopped and keeps it.
  m=g['wait'](r'\[crash.pstore.found\] boot=\S+ seq=\d+ kind=1 tail=1',s,120)
  g['wait'](r'\[crash.boot.shown\]',s,30);time.sleep(.5)
  _,_,px=shot(g,a.out/'boot-after-stop.ppm')
  evidence['boot_note']=round(frac(px,NOTE_BG),3)
  assert evidence['boot_note']>.03,evidence
  desktop(g,s)
  m=g['wait'](r'\[files.crash.ring\] readable=1 kept=(\d+)',s,60);assert int(m[1])>=2,m[0]
  g['wait'](r'\[files.crash.kept\] slot=\d+ index=\d+ seq=\d+ kind=1 pd=',s,60)
  spindle(g,'crashes',r'\[spindle.crashes.row\] seq=\d+ kind=1 pd=\d+ earlier=1')
  assert 'KERNEL PANIC' not in g['text']().split('[console.stop.restart]',1)[1]
 finally:g['stop']()
 # Power off: only the disk remembers.
 g=start(a.iso,disk,a.out,'cold')
 try:
  g['wait'](r'\[crash.pstore\] available=1 found=0',0,120)
  desktop(g)
  m=g['wait'](r'\[files.crash.ring\] readable=1 kept=(\d+)',0,60);assert int(m[1])>=3,m[0]
  spindle(g,'crashes',r'\[spindle.crashes.row\] seq=\d+ kind=1 pd=\d+ earlier=1')
  assert 'KERNEL PANIC' not in g['text']()
  evidence['cold_kept']=int(m[1])
  # Shell starts again in place: Display starts with it, the running apps keep their
  # windows, and Spindle (which kept running) still takes commands.
  # Shell keeps the desktop across its own restart: change the layout first.
  m=chord(g,['meta_l','spc'],'CycleLayout');layout=m[1];assert layout!='master',m[0]
  time.sleep(2);shot(g,a.out/'before-shell.ppm')
  s=len(g['text']());spindle(g,'crash shell',r'\[crash.probe.service\] pd=3')
  g['wait'](r'\[service.restarted\] pd=3 ',s,60);g['wait'](r'\[service.restarted\] pd=1 ',s,60)
  m=g['wait'](r'\[shell.restarted\] restarts=1 adopted=(\d+) windows=(\d+)',s,60);assert int(m[1])>=1 and int(m[2])>=3,m[0]
  time.sleep(4);_,_,px=shot(g,a.out/'after-shell.ppm');assert frac(px,STOP_BG)<.01
  r=g['wait'](r'\[shell.session.restored\] windows=(\d+) minimized=\d+ shown=(\d+) scene=0 ',s,60);assert int(r[1])>=3,r[0]
  g['wait'](r'\[shell.wm.layout\] scene=0 layout='+re.escape(layout)+' ',s,30)
  kept=round(same(a.out/'before-shell.ppm',a.out/'after-shell.ppm'),3);assert kept>.9,kept
  # Focus as the new Shell reports it, not the old run's last record.
  g['focus']=lambda sid:focus_surface(sid,lambda:g['text']()[s:],g['key'])
  spindle(g,'crashes',r'\[spindle.crashes.row\] seq=\d+ kind=4 pd=3 earlier=0')
  evidence['shell_restart']=dict(adopted=int(m[1]),windows=int(m[2]),restored=int(r[1]),layout=layout,same_pixels=kept)
  assert 'KERNEL PANIC' not in g['text']()[s:]
 finally:g['stop']()
 print({'ok':True,**evidence})
if __name__=='__main__':main()
