#!/usr/bin/env python3
"""Join actual BOOT inventory, timer park/expiry and Ring3 return receipts."""
import argparse,json,re
from pathlib import Path

# Independent qualification bound. A larger logged value cannot authorize a
# larger BOOT inventory; changing this profile needs an explicit checker change.
EXPECTED_BOOT_MAIN_CAPACITY=64

def inspect(text,active=True,*,browser_tasks=(15,)):
 errors=[]
 # IDs are observed task identities, not catalog PD numbers or indices into
 # the 64-entry inventory. A caller must join its held source records first.
 valid_browser_tasks=isinstance(browser_tasks,(tuple,list))and 1<=len(browser_tasks)<=EXPECTED_BOOT_MAIN_CAPACITY
 valid_browser_tasks=valid_browser_tasks and all(type(task)is int and 0<task<=0xffff_ffff for task in browser_tasks)
 valid_browser_tasks=valid_browser_tasks and len(set(browser_tasks))==len(browser_tasks)
 selected=tuple(browser_tasks)if valid_browser_tasks else()
 if not valid_browser_tasks:errors.append('invalid explicit Browser task IDs: require 1..64 unique positive u32 identities')
 markers=list(re.finditer(re.escape('[boot.self_wait.sealed'),text))
 seals=list(re.finditer(r'^\[boot.self_wait.sealed\] tasks=([1-9][0-9]?) capacity='+str(EXPECTED_BOOT_MAIN_CAPACITY)+r' max_ticks=64 runtime_registration=0\r?\n',text,re.M))
 authority=list(re.finditer(r'\[sexnet.wait.authority\] invalid_intervals=3 spare_arguments=4 ok=1',text))
 parks=list(re.finditer(r'\[scheduler.self_wait.park\] task=(\d+) ticks=(\d+) deadline=(\d+) count=(\d+)',text))
 expiries=list(re.finditer(r'\[scheduler.self_wait.expired\] task=(\d+) deadline=(\d+) now=(\d+) count=(\d+)',text))
 returns=list(re.finditer(r'\[sexnet.receive.wait\] before=(\d+) after=(\d+) ticks=2 count=(\d+) kind=timer_assisted_poll irq=0',text))
 valid_seal=len(markers)==len(seals)==1 and 1<=int(seals[0][1])<=EXPECTED_BOOT_MAIN_CAPACITY
 if not valid_seal:errors.append('missing/invalid unique complete BOOT timer inventory (capacity must be 64)')
 if len(authority)!=1:errors.append('missing unique real Ring3 invalid-argument probes')
 if seals and any(m.start()<seals[0].start()for m in parks+expiries+authority):errors.append('timed wait precedes BOOT seal')
 for p in parks:
  # SexNet (13) and explicitly selected Browser tasks poll every 2 ticks;
  # since round 105 the
  # sexdrive main task (2) waits 1 tick for NVMe completions. Park records
  # are sampled (first four, then powers of two), so either may appear.
  if (int(p[1]),int(p[2]))not in((13,2),(2,1))+tuple((task,2)for task in selected) or int(p[3])==0:errors.append('unexpected timed-poll task/interval/deadline')
 for e in expiries:
  if int(e[1])not in(13,2)+selected or int(e[3])<int(e[2]):errors.append('foreign task or early timer expiry')
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
 return dict(ok=not errors,errors=errors,browser_tasks=list(selected),boot_tasks=int(seals[0][1])if valid_seal else None,parks_observed=len(parks),expiries_observed=len(expiries),returns_observed=len(returns),timer_user_joins=joined,scope='Sampled real BOOT/main-task/park/timer/Ring3 receipts with explicitly caller-selected observed Browser task IDs; no identity autodiscovery or authority grant. Native peer and desktop gates separately establish useful progress. Nominal timer-assisted polling, not NIC IRQ, CPU/power bound, physical or SMP proof.')

def main():
 p=argparse.ArgumentParser();p.add_argument('log',type=Path);p.add_argument('--inactive',action='store_true');a=p.parse_args();r=inspect(a.log.read_text(errors='replace'),not a.inactive);print(json.dumps(r,indent=2));raise SystemExit(0 if r['ok']else 1)
if __name__=='__main__':main()
