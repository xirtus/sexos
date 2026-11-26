#!/usr/bin/env python3
"""The calendar, a program built on sex-app: it imports an iCalendar file it was given
(`run date @cal.ics`, the host-test fixture: folded lines, escapes, an alarm and a
to-do to skip, weekly/daily/monthly/yearly repeats with COUNT, UNTIL and INTERVAL),
opens on today's date by the machine's clock, moves by month and day from the keyboard,
shows a day's events with repeats expanded onto it, adds a quick event with `n`, keeps
it in its own folder and finds it again on its next run. A final two-boot phase
cuts QEMU power after an event save and checks the same disk after reboot."""
import argparse,calendar,datetime,http.server,os,re,shutil,threading,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
ICS=(ROOT/'crates/sex-calendar/tests/fixture.ics').read_bytes()
def due_ics():
 # Leave at least one minute for fetch/import/launch before the reminder.
 now=datetime.datetime.now(datetime.timezone.utc)
 due=now.replace(second=0,microsecond=0)+datetime.timedelta(minutes=2)
 start=due+datetime.timedelta(minutes=1)
 end=start+datetime.timedelta(minutes=30)
 lines=['BEGIN:VCALENDAR','VERSION:2.0','PRODID:-//SexOS//Date keyed reminder gate//EN',
  'BEGIN:VEVENT','UID:keyed-reminder-gate@sexos.test',f'DTSTART:{start:%Y%m%dT%H%M%SZ}',
  f'DTEND:{end:%Y%m%dT%H%M%SZ}','SUMMARY:Keyed reminder proof',
  'BEGIN:VALARM','TRIGGER:-PT1M','ACTION:DISPLAY','END:VALARM',
  'END:VEVENT','END:VCALENDAR','']
 return '\r\n'.join(lines).encode()
class Handler(http.server.BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/cal.ics':data=ICS
  elif self.path=='/due.ics':data=due_ics()
  else:self.send_response(404);self.end_headers();return
  self.send_response(200);self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
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
 install(disk,[('date',(ROOT/'iso_root/programs/date').read_bytes())])
 g=boot(a.iso,disk,a.out,'calendar',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   v=KEYS.get(k,('shift',k.lower()) if k.isupper() else k)
   if isinstance(v,tuple):q.cmd('send-key',{'keys':[{'type':'qcode','data':x}for x in v],'hold-time':80})
   else:q.key(v)
   time.sleep(.08)
 def spindle(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def start(args):
  s=len(t());spindle(f'run date{args}',r'\[shell.program.launch\]')
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
  g['wait'](r'\[date.notice.cursor.initialized\] owner=visible minute=\d+',s,60)
  g['wait'](r'\[calendar.imported\] events=6 added=6\n',s,60);g['wait'](r'\[calendar.saved\] events=6 ',s,60)
  g['wait'](r'\[date.save.committed\] file=events.txt bytes=\d+',s,60)
  # The whole desktop for the calendar (Super+M), so a month shows its titles.
  s2=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'meta_l'},{'type':'qcode','data':'m'}],'hold-time':80})
  g['wait'](r'\[shell.wm.action\] action=ToggleMonocle done=1',s2,30);g['wait'](rf'\[shell.program.resize\] pd={target} task={task} w=\d+ h=\d+ status=0',s2,30);time.sleep(2)
  g['focus'](sid);time.sleep(.5)
  g['wait'](r'\[date.theme\] mode=Light accent=d6336c',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-light.ppm')});time.sleep(.5)
  light=(a.out/'date-light.ppm').read_bytes()
  assert light.startswith(b'P6\n') and light.count(bytes((0xF6,0xF5,0xF7)))>10000,'Date did not paint the light theme background'
  # In-app reference covers the calendar, clock, invitation and sync keys.
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'slash'}],'hold-time':80})
  g['wait'](r'\[date.help\] open=1 offset=0',s,30)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'date-shortcuts.ppm')});time.sleep(.5)
  shot=(a.out/'date-shortcuts.ppm').read_bytes();assert shot.startswith(b'P6\n') and len(shot)>100000
  s=len(t());q.key('down');g['wait'](r'\[date.help\] open=1 offset=1',s,30)
  s=len(t());q.key('esc');g['wait'](r'\[date.help\] open=0 offset=0',s,30)
  # A Bell notice can be snoozed from its banner. The replacement timer is
  # committed before the success status is drawn and survives a Date restart.
  q.key('c');time.sleep(.3);s=len(t());q.key('r');time.sleep(.3);keys('2s snooze me');q.key('ret')
  g['wait'](r'\[date.timer.started\] seconds=2 label=snooze me',s,30)
  g['wait'](r'\[date.alarm.fired\] kind=timer title=snooze me notified=1',s,45)
  g['wait'](r'\[shell.notify.handled\] event_id=\d+ ticket=\d+ queued=1 shown=1 muted=0',s,30)
  g['wait'](r'\[bell.post.receipt\] caller_pd=\d+ ticket=\d+ accepted=1',s,30)
  g['wait'](r'\[bell.post.ok\] caller_pd=\d+ event_id=\d+ lane=\d+ len=\d+ delivered=1',s,30)
  q.cmd('screendump',{'filename':str(a.out/'date-timer-delivered.ppm')})
  delivered=(a.out/'date-timer-delivered.ppm').read_bytes()
  assert delivered.startswith(b'P6\n') and len(delivered)>100000
  time.sleep(1)
  assert len(re.findall(r'\[date.alarm.fired\] kind=timer title=snooze me notified=1',t()[s:]))==1
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'s'}],'hold-time':80})
  g['wait'](r'\[date.notice.snoozed\] seconds=300 saved=1',s,30)
  g['wait'](r'\[date.save.committed\] file=clock.txt bytes=\d+',s,30)
  time.sleep(1);q.cmd('screendump',{'filename':str(a.out/'date-notice-snoozed.ppm')});time.sleep(.5)
  notice=(a.out/'date-notice-snoozed.ppm').read_bytes();assert notice.startswith(b'P6\n') and len(notice)>100000
  s=len(t());q.key('z');time.sleep(.3);keys('chicago');q.key('ret')
  g['wait'](r'\[date.zone\] zone=America/Chicago home=0',s,30)
  s=len(t());g['chord']('ctrl','z',r'\[date.zone.undo\] home= zones=0')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-zone-undone.ppm')})
  s=len(t());g['chord']('ctrl','y',r'\[date.zone.redo\] home= zones=8')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-zone-redone.ppm')})
  s=len(t());q.key('n');time.sleep(.3);keys('12:34 Review');q.key('ret')
  g['wait'](r'\[date.alarm.added\] time=12:34 label=Review',s,30)
  s=len(t());g['chord']('ctrl','z',r'\[date.alarm.undo\] alarms=0')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-alarm-undone.ppm')})
  s=len(t());g['chord']('ctrl','y',r'\[date.alarm.redo\] alarms=1')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-alarm-redone.ppm')})
  g['chord']('ctrl','z',r'\[date.alarm.undo\] alarms=0')
  s=len(t());q.key('r');time.sleep(.3);keys('1h Undo demo');q.key('ret')
  started=g['wait'](r'\[date.timer.started\] seconds=3600 label=Undo demo timers=(\d+)',s,30)
  timer_count=int(started[1]);assert timer_count>=1
  saved=g['wait'](r'\[date.save.committed\] file=clock.txt bytes=(\d+)',s,30)
  s=len(t());g['chord']('ctrl','z',rf'\[date.timer.undo\] timers={timer_count-1} last=[^\n]*\n')
  undone=g['wait'](r'\[date.save.committed\] file=clock.txt bytes=(\d+)',s,30)
  assert int(undone[1])<int(saved[1]),'undo did not remove the timer from the saved clock'
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-timer-undone.ppm')})
  s=len(t());g['chord']('ctrl','y',rf'\[date.timer.redo\] timers={timer_count} last=Undo demo\n')
  redone=g['wait'](r'\[date.save.committed\] file=clock.txt bytes=(\d+)',s,30)
  assert redone[1]==saved[1],'redo did not restore the timer in the saved clock'
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-timer-redone.ppm')})
  restored=(a.out/'date-timer-redone.ppm').read_bytes()
  removed=(a.out/'date-timer-undone.ppm').read_bytes()
  assert restored.startswith(b'P6\n') and len(restored)>100000 and restored!=removed,'redone timer did not change the Clock screenshot'
  q.key('m');time.sleep(.3)
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
  g['wait'](r'\[date.save.committed\] file=events.txt bytes=\d+',s,60)
  # Event history is saved at each step, and redo restores the same event.
  s=len(t());g['chord']('ctrl','z',r'\[date.undo\] events=6\n')
  g['wait'](r'\[calendar.saved\] events=6 ',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-event-undo.ppm')})
  s=len(t());g['chord']('ctrl','y',r'\[date.redo\] events=7\n')
  g['wait'](r'\[calendar.saved\] events=7 ',s,60)
  s=len(t());q.key('ret');g['wait'](r'\[calendar.day\] date=2026-03-16 events=5\n',s,30)
  s=len(t());q.key('delete')
  removed=g['wait'](r'\[calendar.removed\] date=2026-03-16 title=([^\n]+)\n',s,30)
  assert removed[1],removed[0]
  g['chord']('ctrl','z',r'\[date.undo\] events=7\n')
  g['chord']('ctrl','y',r'\[date.redo\] events=6\n')
  g['chord']('ctrl','z',r'\[date.undo\] events=7\n')
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-event-restored.ppm')})
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
  target,task,sid,today,s=start('')
  g['wait'](r'\[calendar.loaded\] events=7\n',s,60)
  s=len(t());q.key('n');time.sleep(.3);keys('10:20 power cut');q.key('ret')
  g['wait'](rf'\[calendar.added\] date={today} when=10:20-11:20 title=power cut\n',s,60)
  g['wait'](r'\[calendar.saved\] events=8 ',s,60)
  g['wait'](r'\[date.save.committed\] file=events.txt bytes=\d+',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-before-powercut.ppm')})
  assert 'KERNEL PANIC' not in t() and '[calendar.fatal]' not in t()
  g['stop']()  # No Close event reaches Date after the completed staged save.
  g=boot(a.iso,disk,a.out,'date-reboot',a.resolution);t=g['text'];q=g['q']
  g['wait'](r'\[app.private.retired\] target=41 ',0,120)
  target,task,sid,_,s=start('')
  g['wait'](r'\[calendar.loaded\] events=8\n',s,60)
  s=len(t());q.key('ret')
  g['wait'](rf'\[calendar.day\] date={today} events=\d+\n',s,60)
  g['wait'](rf'\[calendar.event\] date={today} when=10:20-11:20 title=power cut\n',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-after-powercut.ppm')})
  # Copy the picked occurrence to tomorrow. It is a separate saved event and one
  # undo step; the new agenda is visibly different from today's source agenda.
  tomorrow=today+datetime.timedelta(days=1)
  s=len(t());q.key('d')
  g['wait'](rf'\[date.event.copied\] from={today} to={tomorrow} title=power cut linked=0',s,60)
  g['wait'](r'\[calendar.saved\] events=9 ',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-event-copied.ppm')})
  copied_pixels=(a.out/'date-event-copied.ppm').read_bytes()
  assert copied_pixels.startswith(b'P6\n') and copied_pixels!=(a.out/'date-after-powercut.ppm').read_bytes()
  s=len(t());g['chord']('ctrl','z',r'\[date.undo\] events=8\n')
  g['wait'](r'\[calendar.saved\] events=8 ',s,60)
  s=len(t());g['chord']('ctrl','y',r'\[date.redo\] events=9\n')
  g['wait'](r'\[calendar.saved\] events=9 ',s,60)
  close(target,task)
  target,task,sid,_,s=start('')
  g['wait'](r'\[calendar.loaded\] events=9\n',s,60)
  q.key('right');time.sleep(.2);s=len(t());q.key('ret')
  g['wait'](rf'\[calendar.day\] date={tomorrow} events=1\n',s,60)
  g['wait'](rf'\[calendar.event\] date={tomorrow} when=10:20-11:20 title=power cut\n',s,60)
  close(target,task)
  # Two visible Date windows share one Bell owner. The first window reloads a
  # timer created before the second opens; only one notice may be posted.
  first_target,first_task,first_sid,_,first_s=start('')
  q.key('c');time.sleep(.3);s=len(t());q.key('r');time.sleep(.3);keys('20s two windows');q.key('ret')
  g['wait'](r'\[date.timer.started\] seconds=20 label=two windows',s,30)
  second_target,second_task,second_sid,_,second_s=start('')
  g['wait'](rf'\[date.notice.ui.owner\] task={first_task} owns=1',first_s,30)
  g['wait'](rf'\[date.notice.ui.owner\] task={second_task} owns=0',second_s,30)
  # Keep the non-owner busy with real key events while the first window's timer
  # comes due. A key must not bypass the visible Bell-owner election.
  g['focus'](second_sid)
  busy_until=time.monotonic()+22
  while time.monotonic()<busy_until:q.key('f2');time.sleep(.08)
  g['wait'](r'\[date.alarm.fired\] kind=timer title=two windows notified=1',s,45)
  time.sleep(2)
  assert len(re.findall(r'\[date.alarm.fired\] kind=timer title=two windows notified=1',t()[s:]))==1,'two Date windows posted duplicate Bell notices'
  q.cmd('screendump',{'filename':str(a.out/'date-two-windows.ppm')})
  assert (a.out/'date-two-windows.ppm').read_bytes().startswith(b'P6\n')
  # Pointer movement is ignored by the calendar UI, but it must not starve the
  # elected owner's periodic due check or its banner.
  g['focus'](first_sid);time.sleep(.3);q.key('c');time.sleep(.2);s=len(t());q.key('r');time.sleep(.2);keys('4s busy owner');q.key('ret')
  g['wait'](r'\[date.timer.started\] seconds=4 label=busy owner',s,30)
  began=time.monotonic();posted_during_input=False
  while time.monotonic()-began<6:
   x=15000 if int((time.monotonic()-began)*20)%2 else 16000
   q.cmd('input-send-event',{'events':[{'type':'abs','data':{'axis':'x','value':x}},{'type':'abs','data':{'axis':'y','value':15000}}]})
   if '[date.alarm.fired] kind=timer title=busy owner notified=1' in t()[s:]:posted_during_input=True
   time.sleep(.05)
  assert posted_during_input,'continuous pointer input postponed the elected Date owner\'s Bell notice'
  q.cmd('screendump',{'filename':str(a.out/'date-owner-under-input.ppm')})
  assert (a.out/'date-owner-under-input.ppm').read_bytes().startswith(b'P6\n')
  g['focus'](first_sid);close(first_target,first_task)
  g['focus'](second_sid);time.sleep(.5);s=len(t());q.key('c');time.sleep(.3);q.key('r');time.sleep(.3);keys('2s new owner');q.key('ret')
  g['wait'](rf'\[date.notice.ui.owner\] task={second_task} owns=1',s,30)
  g['wait'](r'\[date.alarm.fired\] kind=timer title=new owner notified=1',s,30)
  time.sleep(2)
  assert len(re.findall(r'\[date.alarm.fired\] kind=timer title=new owner notified=1',t()[s:]))==1,'handoff posted duplicate Bell notices'
  close(second_target,second_task)
  # A clock alarm becomes due while Date has no window. The hidden Date worker
  # must ring Bell before another visible Date run starts.
  target,task,sid,_,s=start('')
  g['wait'](r'\[date.notice.cursor.loaded\] minute=\d+ now=\d+ catchup=[01]',s,60)
  q.key('c');time.sleep(.3)
  now=datetime.datetime.now(datetime.timezone.utc)
  due=now.replace(second=0,microsecond=0)+datetime.timedelta(minutes=1 if now.second<45 else 2)
  label='after close'
  s=len(t());q.key('n');time.sleep(.3);keys(due.strftime('%H:%M')+' '+label);q.key('ret')
  g['wait'](rf'\[date.alarm.added\] time={due:%H:%M} label={label}',s,60)
  s=len(t())
  close(target,task)
  g['wait'](r'\[date.notice.worker.resumed\]',s,60)
  time.sleep(max(0,(due-datetime.datetime.now(datetime.timezone.utc)).total_seconds())+3)
  g['wait'](r'\[date.alarm.fired\] kind=alarm title=after close notified=1',s,15)
  q.cmd('screendump',{'filename':str(a.out/'date-background-alarm.ppm')})
  assert (a.out/'date-background-alarm.ppm').read_bytes().startswith(b'P6\n')
  s=len(t())
  target,task,sid,_,s=start('')
  g['wait'](r'\[date.notice.worker.handoff\]',s,60)
  g['wait'](r'\[date.notice.cursor.loaded\] minute=\d+ now=\d+ catchup=[01]',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-overdue-alarm.ppm')})
  # Choose an arbitrary destination for a personal copy, then move that copy.
  # Both actions persist, and moving is a single undo/redo step.
  copy_day=today+datetime.timedelta(days=3)
  move_day=today+datetime.timedelta(days=5)
  s=len(t());q.key('ret')
  g['wait'](rf'\[calendar.event\] date={today} when=10:20-11:20 title=power cut\n',s,60)
  s=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'shift'},{'type':'qcode','data':'d'}],'hold-time':80})
  time.sleep(.3);keys(str(copy_day));q.key('ret')
  g['wait'](rf'\[date.event.copied\] from={today} to={copy_day} title=power cut linked=0',s,60)
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-event-copy-date.ppm')})
  assert (a.out/'date-event-copy-date.ppm').read_bytes().startswith(b'P6\n')
  s=len(t());q.key('m');time.sleep(.3);keys(str(move_day));q.key('ret')
  g['wait'](rf'\[date.event.moved\] from={copy_day} to={move_day} title=power cut linked=0',s,60)
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-event-move-date.ppm')})
  moved_pixels=(a.out/'date-event-move-date.ppm').read_bytes()
  assert moved_pixels.startswith(b'P6\n') and moved_pixels!=(a.out/'date-event-copy-date.ppm').read_bytes()
  s=len(t());g['chord']('ctrl','z',r'\[date.undo\] events=10\n')
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  s=len(t());g['chord']('ctrl','y',r'\[date.redo\] events=10\n')
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  s=len(t());q.key('e');time.sleep(.3);keys('rescheduled power cut');q.key('ret')
  g['wait'](rf'\[date.event.retitled\] date={move_day} from=power cut to=rescheduled power cut linked=0',s,60)
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-event-title-edited.ppm')})
  title_pixels=(a.out/'date-event-title-edited.ppm').read_bytes()
  assert title_pixels.startswith(b'P6\n') and title_pixels!=moved_pixels
  s=len(t());g['chord']('ctrl','z',r'\[date.undo\] events=10\n')
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  s=len(t());g['chord']('ctrl','y',r'\[date.redo\] events=10\n')
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  s=len(t());q.key('l');time.sleep(.3);keys('North room');q.key('ret')
  g['wait'](rf'\[date.event.location\] date={move_day} title=rescheduled power cut from= to=North room linked=0',s,60)
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-event-location-edited.ppm')})
  location_pixels=(a.out/'date-event-location-edited.ppm').read_bytes()
  assert location_pixels.startswith(b'P6\n') and location_pixels!=title_pixels
  s=len(t());g['chord']('ctrl','z',r'\[date.undo\] events=10\n')
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  s=len(t());g['chord']('ctrl','y',r'\[date.redo\] events=10\n')
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  s=len(t());q.key('l');time.sleep(.3);q.key('ret')
  g['wait'](rf'\[date.event.location\] date={move_day} title=rescheduled power cut from=North room to= linked=0',s,60)
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  s=len(t());q.key('t');time.sleep(.3);keys('11:30-12:15');q.key('ret')
  g['wait'](rf'\[date.event.timed\] date={move_day} title=rescheduled power cut from=10:20-11:20 to=11:30-12:15 linked=0',s,60)
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-event-time-edited.ppm')})
  time_pixels=(a.out/'date-event-time-edited.ppm').read_bytes()
  assert time_pixels.startswith(b'P6\n') and time_pixels!=location_pixels
  s=len(t());g['chord']('ctrl','z',r'\[date.undo\] events=10\n')
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  s=len(t());g['chord']('ctrl','y',r'\[date.redo\] events=10\n')
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  s=len(t());q.key('t');time.sleep(.3);keys('all day');q.key('ret')
  g['wait'](rf'\[date.event.timed\] date={move_day} title=rescheduled power cut from=11:30-12:15 to=all day linked=0',s,60)
  g['wait'](r'\[calendar.saved\] events=10 ',s,60)
  close(target,task)
  # A fresh UID-backed event must take the keyed Date -> Bell -> Shell route.
  m=spindle(f'fetch http://10.0.2.2:{PORT}/due.ics due.ics',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240)
  assert m[1].startswith('Saved'),m[1]
  target,task,sid,_,s=start(' @due.ics')
  g['wait'](r'\[calendar.imported\] events=1 added=1\n',s,60)
  q.cmd('screendump',{'filename':str(a.out/'date-keyed-before.ppm')})
  semantic=g['wait'](r'\[shell.notify.semantic\] event_id=(\d+) key_low=(\d+) key_high=(\d+)',s,180)
  g['wait'](rf'\[shell.notify.handled\] event_id={semantic[1]} ticket=\d+ queued=1 shown=1 muted=0',s,30)
  g['wait'](r'\[date.alarm.fired\] kind=reminder title=Keyed reminder proof notified=1 settled=1',s,30)
  time.sleep(.5);q.cmd('screendump',{'filename':str(a.out/'date-keyed-reminder.ppm')})
  keyed=(a.out/'date-keyed-reminder.ppm').read_bytes()
  assert keyed.startswith(b'P6\n') and len(keyed)>100000 and keyed!=(a.out/'date-keyed-before.ppm').read_bytes()
  close(target,task)
  # The Date-owned receipt file must survive a run restart. The host test
  # verifies that a parsed journal recognizes the recorded key.
  target,task,sid,_,s=start('')
  g['wait'](r'\[date.notice.receipts.loaded\] owner=visible count=[1-9]\d*',s,60)
  q.cmd('screendump',{'filename':str(a.out/'date-keyed-reopened.ppm')})
  reopened=(a.out/'date-keyed-reopened.ppm').read_bytes()
  assert reopened.startswith(b'P6\n') and len(reopened)>100000
  close(target,task)
  text=t();assert 'KERNEL PANIC' not in text and '[calendar.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'today':str(today),'months_moved':months,'powercut_event':True})
 finally:g['stop']()
if __name__=='__main__':main()
