#!/usr/bin/env python3
"""Date (the calendar) against a CalDAV server on the host (scripts/dav_fixture.py):
it takes an account file (`run +net date @account.dav @invite.ics`), finds the
server's calendar, pulls its events, pushes one added here (the server sees it, with
the account's credentials), pulls a later server change through the sync-token,
accepts an invitation (PARTSTAT written, a METHOD:REPLY kept for the mail program),
shows the week, day and clock views, rings a timer and an event's reminder through
Bell, and on its next run finds the calendar unchanged with one PROPFIND."""
import argparse,datetime,os,re,shutil,sys,time
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(Path(__file__).resolve().parent))
from dav_fixture import Dav,serve
COLLECTION='/dav/calendars/alice/home/'
dav=Dav('/dav/calendars/alice/',COLLECTION,'cal')
PORT=serve(dav)
os.environ['SEXOS_GATE_NIC']='-netdev user,id=n0 -device e1000e,netdev=n0'
from install_program import install
from quil_autosave_gate import boot
KEYS={' ':'spc','/':'slash','.':'dot','-':'minus','+':('shift','equal'),':':('shift','semicolon'),'@':('shift','2')}
def ics(uid,start,end,summary,extra=''):
 return f'BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//fixture//EN\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261101T000000Z\r\nDTSTART:{start}\r\nDTEND:{end}\r\nSUMMARY:{summary}\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n'
def z(t):return t.strftime('%Y%m%dT%H%M%SZ')
def main():
 p=argparse.ArgumentParser();p.add_argument('--iso',required=True,type=Path);p.add_argument('--source-disk',required=True,type=Path);p.add_argument('--binding',type=Path);p.add_argument('--out',required=True,type=Path);p.add_argument('--resolution',default='1280x800');a=p.parse_args()
 a.out.mkdir(parents=True,exist_ok=True);disk=a.out/'nvme.img';shutil.copyfile(a.source_disk,disk)
 with disk.open('r+b') as f:f.truncate(1024**3)
 install(disk,[('date',(ROOT/'iso_root/programs/calendar').read_bytes())])
 today=datetime.datetime.now(datetime.timezone.utc).date()
 invite_day=today+datetime.timedelta(days=2)
 dav.files['/account.dav']=f'url=http://10.0.2.2:{PORT}/dav/calendars/alice/\nuser=alice\npassword=wonderland\nemail=alice@example.org\n'.encode()
 dav.files['/invite.ics']=('BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:review-invite@example.org\r\nDTSTAMP:20261101T000000Z\r\n'
  f'DTSTART:{invite_day:%Y%m%d}T150000Z\r\nDTEND:{invite_day:%Y%m%d}T160000Z\r\nSUMMARY:Design review\r\nORGANIZER;CN=Bob:mailto:bob@example.org\r\n'
  'ATTENDEE;CN=Bob;PARTSTAT=ACCEPTED:mailto:bob@example.org\r\nATTENDEE;CN=Alice;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:alice@example.org\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n').encode()
 # A weekly event the server already has: Mondays, Wednesdays and Fridays, one skipped.
 dav.put('gym.ics',ics('gym@fixture','20261102T170000Z','20261102T180000Z','Gym','RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR\r\nEXDATE:20261104T170000Z\r\n'))
 g=boot(a.iso,disk,a.out,'date',a.resolution);t=g['text'];q=g['q']
 def keys(text):
  for k in text:
   v=KEYS.get(k,k)
   if isinstance(v,tuple):q.cmd('send-key',{'keys':[{'type':'qcode','data':x}for x in v],'hold-time':80})
   else:q.key(v)
   time.sleep(.08)
 def spindle(text,pattern,timeout=120):
  g['focus'](153);time.sleep(.5);s=len(t());keys(text);q.key('ret');return g['wait'](pattern,s,timeout)
 def start(args):
  s=len(t());spindle(f'run +net date{args}',r'\[shell.program.launch\]')
  ready=g['wait'](r'\[calendar.ready\] target=(\d+) task=(\d+) today=(\d{4}-\d\d-\d\d) month=',s,120)
  launch=g['wait'](rf'\[shell.program.launch\] target={ready[1]} task={ready[2]} surface=(\d+)',s,60)
  g['wait'](rf'\[display.app.pixels\] target={ready[1]} ',s,60);time.sleep(2)
  g['focus'](int(launch[1]));time.sleep(.5)
  return ready[1],ready[2],int(launch[1]),s
 def close(target,task):
  s=len(t());q.key('f11');g['wait'](rf'\[calendar.closed\] target={target} task={task}',s,60);g['wait'](rf'\[app.private.retired\] target={target} task={task} ',s,90)
 def press(k,pattern,timeout=60):
  s=len(t());keys(k) if len(k)==1 else q.key(k);return g['wait'](pattern,s,timeout)
 def shot(name):time.sleep(1);q.cmd('screendump',{'filename':str(a.out/f'{name}.ppm')});time.sleep(.5)
 try:
  g['wait'](r'\[app.private.retired\] target=41 ',0,120);time.sleep(2)
  for f in ['account.dav','invite.ics']:
   m=spindle(f'fetch http://10.0.2.2:{PORT}/{f} {f}',r'\[spindle.fetch.line\] (Saved \d+ bytes|fetch: [^\n]*)',240);assert m[1].startswith('Saved'),m[1]
  target,task,sid,s=start(' @account.dav @invite.ics')
  g['wait'](r'\[date.account\] url=http://10.0.2.2:\d+/dav/calendars/alice/ user=alice\n',s,60)
  g['wait'](r'\[calendar.imported\] events=1 added=1\n',s,60);g['wait'](r'\[date.invitations\] count=1\n',s,60)
  s2=len(t());q.cmd('send-key',{'keys':[{'type':'qcode','data':'meta_l'},{'type':'qcode','data':'m'}],'hold-time':80})
  g['wait'](r'\[shell.wm.action\] action=ToggleMonocle done=1',s2,30);time.sleep(2);g['focus'](sid);time.sleep(.5)
  # The server's event starts in three minutes and reminds one minute before.
  start_at=(datetime.datetime.now(datetime.timezone.utc)+datetime.timedelta(minutes=3)).replace(second=0,microsecond=0)
  dav.put('planning.ics',ics('planning@fixture',z(start_at),z(start_at+datetime.timedelta(minutes=30)),'Server planning','BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Server planning\r\nTRIGGER:-PT1M\r\nEND:VALARM\r\n'))
  # Pull: discovery, then everything (no sync-token yet).
  s=len(t());keys('s')
  g['wait'](rf'\[date.sync.found\] id=1 name=Home path={COLLECTION}\n',s,120)
  g['wait'](r'\[date.sync\] calendar=Home unchanged=0 incremental=0 pulled=2 removed=0 pushed=0 deleted=0 conflicts=0 failed=0\n',s,120)
  g['wait'](r'\[date.sync.done\] events=3 ',s,60)
  # Push: pick the server's calendar for new events, add one, sync.
  press('l',r'\[date.view\] view=Calendars');press('down',r'\[date.calendar.target\] id=1 name=Home\n');press('m',r'\[date.view\] view=Month')
  s=len(t());keys('n');time.sleep(.3);keys('11:15 pushed here');q.key('ret')
  g['wait'](rf'\[calendar.added\] date={today} when=11:15-12:15 title=pushed here\n',s,30)
  s=len(t());keys('s')
  g['wait'](r'\[date.sync\] calendar=Home unchanged=0 incremental=0 pulled=0 removed=0 pushed=1 deleted=0 conflicts=0 failed=0\n',s,120)
  puts=[r for r in dav.requests if r[0]=='PUT']
  assert len(puts)==1 and puts[0][1].startswith(COLLECTION) and puts[0][2].get('If-None-Match')=='*',puts
  assert b'SUMMARY:pushed here' in puts[0][3] and f'DTSTART:{today:%Y%m%d}T111500'.encode() in puts[0][3],puts[0][3]
  stored=[d for p,(n,d) in dav.items.items() if 'pushed here' in d];assert len(stored)==1,dav.items.keys()
  # A change on the server comes down through the sync-token.
  dav.put('late.ics',ics('late@fixture',f'{today:%Y%m%d}T200000Z',f'{today:%Y%m%d}T203000Z','Added on server'))
  s=len(t());keys('s')
  g['wait'](r'\[date.sync\] calendar=Home unchanged=0 incremental=1 pulled=1 removed=0 pushed=0 deleted=0 conflicts=0 failed=0\n',s,120)
  # Every request carried the account's credentials.
  assert all(r[2].get('Authorization')==dav.auth for r in dav.requests if r[1].startswith('/dav/')),'credentials'
  # The invitation two days on: open the day, accept.
  for _ in range(2):q.key('right');time.sleep(.15)
  s=len(t());q.key('ret');g['wait'](rf'\[calendar.day\] date={invite_day} events=\d+\n',s,30)
  g['wait'](rf'\[calendar.event\] date={invite_day} when=15:00-16:00 UTC title=Design review\n',s,30)
  # Gym (17:00) may share the day; the review at 15:00 sorts first.
  m=press('a',r'\[date.invite.answered\] uid=review-invite@example.org partstat=ACCEPTED reply=(\S+) bytes=(\d+)\n')
  shot('date-invitation');q.key('esc');time.sleep(.3)
  # Week and day views, mouse on a tab too.
  press('w',r'\[date.view\] view=Week');shot('date-week')
  press('d',r'\[date.view\] view=Day');shot('date-day')
  press('c',r'\[date.view\] view=Clock')
  s=len(t());keys('r');time.sleep(.3);keys('5s tea');q.key('ret')
  g['wait'](r'\[date.timer.started\] seconds=5 label=tea\n',s,30)
  g['wait'](r'\[date.alarm.fired\] kind=timer title=tea notified=1\n',s,60)
  g['wait'](r'\[bell.post.ok\] caller_pd=\d+ ',s,30);shot('date-clock')
  # The reminder rings a minute before the server's event.
  m=g['wait'](r'\[date.alarm.fired\] kind=reminder title=Server planning notified=1\n',s,300)
  rang=datetime.datetime.now(datetime.timezone.utc)
  assert rang>=start_at-datetime.timedelta(minutes=1,seconds=5),(rang,start_at)
  shot('date-reminder')
  press('m',r'\[date.view\] view=Month');press('e',r'\[date.exported\] events=(\d+) bytes=\d+\n')
  close(target,task)
  # The next run keeps everything; the server has not changed since: unchanged.
  s0=len(t());target,task,sid,s=start('')
  g['wait'](r'\[calendar.loaded\] events=5\n',s0,60)
  s=len(t());keys('s');g['wait'](r'\[date.sync\] calendar=Home unchanged=1 ',s,120)
  close(target,task)
  text=t();assert 'KERNEL PANIC' not in text and '[calendar.fatal]' not in text and 'panicked' not in text
  print({'ok':True,'requests':len(dav.requests),'today':str(today)})
 finally:g['stop']()
if __name__=='__main__':main()
