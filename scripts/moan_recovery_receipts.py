# SPDX-License-Identifier: GPL-3.0-only
"""Exact metadata joins for benign public-SDK restricted-helper integration.
These joins qualify observed admission ordering, not unexecuted attack scenarios.
"""
import re
SERVICES=(11,1,13,10)
def admission(text,pd,task,since=0):
 segment=text[since:]
 def exact(pattern):
  found=list(re.finditer(pattern,segment))
  if not found:raise ValueError('missing admission receipt: '+pattern)
  return found[0]
 copied=exact(rf'\[moan.role.copied\] pd={pd} task={task} generation=(\d+)\r?\n');generation=int(copied[1])
 reserved=exact(rf'\[moan.role.reserved\] pd={pd} task=(\d+) generation={generation}\r?\n');previous=int(reserved[1])
 active=exact(rf'\[moan.role.active\] pd={pd} task={task} generation={generation}\r?\n')
 ready=exact(rf'\[moan.role.ready\] pd={pd} task={task}\r?\n')
 bootstrap=exact(rf'\[moanplug.bootstrap\] pd={pd} task={task} scrubbed=1 hashes=1\r?\n')
 native=exact(rf'\[moanplug.native\] pd={pd} task={task} ready=1\r?\n')
 # Shell logs active/ready AFTER their committing syscalls. The child may run
 # before those post-commit log lines; preserve only actual causal/source edges.
 if not (reserved.start()<copied.start()<active.start()<ready.start() and copied.start()<bootstrap.start()<native.start() and bootstrap.start()<ready.start()):raise ValueError('copy/activation/bootstrap/ready/native causal ordering')
 receipts={}
 for service in SERVICES:
  for phase in range(1,12):
   expected_task=previous if phase<=9 else task
   pattern=rf'\[moan.role.policy\] service={service} phase={phase} pd={pd} task=(\d+) generation={generation} complete=1\r?\n'
   found=list(re.finditer(pattern,segment))
   if not found or any(int(m[1])!=expected_task for m in found):raise ValueError(f'wrong/missing policy service={service} phase={phase}')
   m=found[0]
   if not reserved.start()<m.start()<min(active.start(),bootstrap.start(),native.start()):raise ValueError('policy outside reservation/activation/native fence')
   if phase<=9 and m.start()>copied.start():raise ValueError('pre-copy policy after copy')
   if phase>=10 and m.start()<copied.start():raise ValueError('post-copy policy before copy')
   receipts[service,phase]=m.start()
  if any(receipts[service,p]>=receipts[service,p+1]for p in range(1,11)):raise ValueError('provider phases out of order')
 return dict(pd=pd,task=task,previous_task=previous,generation=generation,complete_policy_joins=len(receipts),native_offset=since+native.start())
