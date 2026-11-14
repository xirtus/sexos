#!/usr/bin/env python3
"""The calendar, a program built on sex-app: it imports an iCalendar file it was given
(`run calendar @cal.ics`, the host-test fixture: folded lines, escapes, an alarm and a
to-do to skip, weekly/daily/monthly/yearly repeats with COUNT, UNTIL and INTERVAL),
opens on today's date by the machine's clock, moves by month and day from the keyboard,
shows a day's events with repeats expanded onto it, adds a quick event with `n`, keeps
it in its own folder and finds it again on its next run."""
import argparse,calendar,datetime,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
ICS=(ROOT/'crates/sex-calendar/tests/fixture.ics').read_bytes()
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path!='/cal.ics':self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(ICS)));self.end_headers();self.wfile.write(ICS)
 def log_message(self,*a):pass
class Quiet(http.server.ThreadingHTTPServer):daemon_threads=True
server=Quiet(('127.0.0.1',0),Handler);PORT=server.server_address[1]
threading.Thread(target=server.serve_forever,daemon=True).start()
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from install_program import install
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus',':':('shift','semicolon'),'@':('shift','2')}
TARGET=datetime.date(2026,3,16)
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('calendar',(ROOT/'iso_root/programs/calendar').read_bytes())])
 g=boot(a.iso,disk,a.out,'calendar',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   v=KEYS.get(k,k)
   if isinstance(v,tuple):q.cmd('send-key',{'keys':[{'type':'qcode','data':x}for x in v],'hold-time':80})
   else:q.key(v)
   time.sleep(.08)
 def spindle(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def start(args):
  s=len(t());spindle(f'run calendar{args}',r'\[shell.program.launch\]')
  ready=g['wait'](r'\[calendar.ready\] target=(\d+) task=(\d+) today=(\d{4}-\d\d-\d\d) month=',s,120)
  launch=g['wait'](rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)',s,60)
  g['wait'](rf'\[display.app.pixels\] target={ready[1]} ',s,60);time.sleep(2)
  g['focus'](int(launch[1]));time.sleep(.5)
  return ready[1],ready[2],int(launch[1]),datetime.date.fromisoformat(ready[3]),s
 def close(target,task):
  s=len(t());q.key('f11');g['wait'](rf'\[calendar.closed\] target={target} task={task}',s,60);g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  m=spindle(f'fetch http://10.0.2.2:{PORT}/cal.ics cal.ics',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  target,task,sid,today,s=start(' @cal.ics')
  g['wait'](r'\[calendar.imported\] events=6 added=6\n',s,60);g['wait'](r'\[calendar.saved\] events=6 ',s,60)
  # The whole desktop for the calendar (Super+M), so a month shows its titles.
  s2=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'meta_l'},{'type':'qcode','data':'m'}],'hold-time':80})
  g['wait'](r'\[shell.wm.action\] action=ToggleMonocle done=1',s2,30);g['wait'](rf'\[shell.program.resize\] pd={target} task={task} w=\d+ h=\d+ status=0',s2,30);time.sleep(2)
  g['focus'](sid);time.sleep(.5)
  # Today is the machine's clock (QEMU's RTC follows this host, in UTC).
  host=datetime.datetime.now(datetime.timezone.utc).date()
  assert abs((today-host).days)<=1,(today,host)
  # PageUp/PageDown to March 2026, keeping the day of the month; then arrows to the 16th.
  months=(TARGET.year*12+TARGET.month)-(today.year*12+today.month)
  for _ in range(abs(months)):q.key('pgdn' if months>0 else 'pgup');time.sleep(.15)
  if months:g['wait'](r'\[calendar.month\] month=2026-03\n',s,30)
  day=min(today.day,calendar.monthrange(TARGET.year,TARGET.month)[1])
  for _ in range(abs(TARGET.day-day)):q.key('right' if TARGET.day>day else 'left');time.sleep(.12)
  time.sleep(.5);s=len(t());q.key('ret')
  g['wait'](r'\[calendar.day\] date=2026-03-16 events=4\n',s,30)
  for when,title in [('all day','Birthday'),('all day','Conference'),('09:00-09:30 UTC','Team sync'),('10:00-10:15','Standup')]:
   g['wait'](rf'\[calendar.event\] date=2026-03-16 when={re.escape(when)} title={re.escape(title)}\n',s,30)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'calendar-day.ppm')});time.sleep(.5)
  # Back to the month; `n` adds an event at a time on the chosen day.
  q.key('ret');time.sleep(.3);s=len(t());q.key('n');time.sleep(.3);keys('09:30 dentist');q.key('ret')
  g['wait'](r'\[calendar.added\] date=2026-03-16 when=09:30-10:30 title=dentist\n',s,30);g['wait'](r'\[calendar.saved\] events=7 ',s,60)
  s=len(t());q.key('ret');g['wait'](r'\[calendar.day\] date=2026-03-16 events=5\n',s,30)
  # The 23rd is the weekly meeting's last (COUNT=4) and the 20th has only lunch: the
  # daily standup's UNTIL is that midnight.
  q.key('ret');time.sleep(.3)
  for _ in range(7):q.key('right');time.sleep(.12)
  s=len(t());q.key('ret');g['wait'](r'\[calendar.day\] date=2026-03-23 events=1\n',s,30);g['wait'](r'\[calendar.event\] date=2026-03-23 when=09:00-09:30 UTC title=Team sync\n',s,30)
  q.key('ret');time.sleep(.3)
  for _ in range(3):q.key('left');time.sleep(.12)
  s=len(t());q.key('ret');g['wait'](r'\[calendar.day\] date=2026-03-20 events=1\n',s,30);g['wait'](r'\[calendar.event\] date=2026-03-20 when=12:30-14:00 title=Lunch with the design team, then a long walk',s,30)
  q.key('ret');time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'calendar-month.ppm')});time.sleep(.5)
  close(target,task)
  # The next run finds all seven in its own folder; the same file again adds nothing.
  s0=len(t());target,task,sid,_,s=start(' @cal.ics')
  g['wait'](r'\[calendar.loaded\] events=7\n',s0,60);g['wait'](r'\[calendar.imported\] events=6 added=0\n',s,60)
  close(target,task)
  text=t();assert 'KERNEL PANIC' not in text and '[calendar.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'today':str(today),'months_moved':months})
 finally:g['stop']()
if __name__=='__main__':main()
