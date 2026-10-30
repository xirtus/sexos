#!/usr/bin/env python3
"""Join actual BOOT inventory, timer park/expiry and Ring3 return receipts."""
import argparse,json,re
from pathlib import Path

def inspect(text,active=True):
 errors=[]
 seals=list(re.finditer(r'\[boot.self_wait.sealed\] tasks=(\d+) capacity=32 max_ticks=64 runtime_registration=0',text))
 authority=list(re.finditer(r'\[sexnet.wait.authority\] invalid_intervals=3 spare_arguments=4 ok=1',text))
 parks=list(re.finditer(r'\[scheduler.self_wait.park\] task=(\d+) ticks=(\d+) deadline=(\d+) count=(\d+)',text))
 expiries=list(re.finditer(r'\[scheduler.self_wait.expired\] task=(\d+) deadline=(\d+) now=(\d+) count=(\d+)',text))
 returns=list(re.finditer(r'\[sexnet.receive.wait\] before=(\d+) after=(\d+) ticks=2 count=(\d+) kind=timer_assisted_poll irq=0',text))
 if len(seals)!=1 or not 1<=int(seals[0][1])<=32:errors.append('missing/invalid unique BOOT timer inventory')
 if len(authority)!=1:errors.append('missing unique real Ring3 invalid-argument probes')
 if seals and any(m.start()<seals[0].start()for m in parks+expiries+authority):errors.append('timed wait precedes BOOT seal')
 for p in parks:
  if int(p[1])not in(13,15) or int(p[2])!=2 or int(p[3])==0:errors.append('unexpected timed-poll task/interval/deadline')
 for e in expiries:
  if int(e[1])not in(13,15) or int(e[3])<int(e[2]):errors.append('foreign task or early timer expiry')
 for r in returns:
  if int(r[2])<int(r[1]):errors.append('backward Ring3 clock')
 for group,index in [(parks,4),(expiries,4),(returns,3)]:
  counts=[int(x[index])for x in group]
  if any(n<=0 for n in counts)or any(a>=b for a,b in zip(counts,counts[1:])):errors.append('repeated/backward diagnostic ordinal')
 joined=[]
 for e in expiries:
  if int(e[1])!=13:continue
  matching=next((p for p in parks if p.start()<e.start()and p[1]==e[1]and p[3]==e[2]),None)
  returned=next((r for r in returns if r.start()>e.start()and int(r[1])<=int(e[2])-2 and int(r[2])>=int(e[3])),None)
  if matching and returned:joined.append(dict(task=int(e[1]),deadline=int(e[2]),expired_at=int(e[3]),returned_at=int(returned[2])))
 if active and(not parks or not expiries or not returns or not joined):errors.append('missing actual park→timer expiry→user return join')
 if not active and(any(int(p[1])==13 for p in parks)or any(int(e[1])==13 for e in expiries)or returns):
  if not returns or any(not text[r.end():].startswith(' device=0 reply_pending=1')for r in returns):errors.append('unsupported NIC unexpectedly entered device polling without retained reply work')
 if re.search(r'TIMED_WAIT_WITHOUT_ATTACHED_OWNER|BOOT_SELF_WAIT_AUTHORITY|SELF_WAIT_FRONTEND_PROBE|KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|panicked at',text):errors.append('guest self-wait failure')
 return dict(ok=not errors,errors=errors,boot_tasks=int(seals[0][1])if len(seals)==1 else None,parks_observed=len(parks),expiries_observed=len(expiries),returns_observed=len(returns),timer_user_joins=joined,scope='Sampled real BOOT/main-task/park/timer/Ring3 receipts; native peer and desktop gates separately establish useful progress. Nominal timer-assisted polling, not NIC IRQ, CPU/power bound, physical or SMP proof.')

def main():
 p=argparse.ArgumentParser();p.add_argument('log',type=Path);p.add_argument('--inactive',action='store_true');a=p.parse_args();r=inspect(a.log.read_text(errors='replace'),not a.inactive);print(json.dumps(r,indent=2));raise SystemExit(0 if r['ok']else 1)
if __name__=='__main__':main()
