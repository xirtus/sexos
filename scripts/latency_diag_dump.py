#!/usr/bin/env python3
"""Temporary (stream 8): print what a failed latency_gate run's serial log says, since
CI artifacts cannot be fetched from the cloud session."""
import re,sys
t=open(sys.argv[1],errors='replace').read()
keys=('smp.online','smp.core.first_task','bench.','files.program','spindle.run','program.','app.private','sched.latency','PANIC','FAULT','panicked','refused','denied','retry','stall','timeout','tlb','shootdown')
noise=('view.bytes','view.text','clock.source.snapshot','silkbar.civil','display.heartbeat','display.text.committed')
s=t.find('run bench spin')
start=t.rfind('[app.private.retired] target=41 ')
lines=t[max(0,start):].splitlines()
print(f'--- dump: {len(t)} bytes, {len(lines)} lines after Spindle ready ---')
hits=[l for l in lines if any(k in l for k in keys) and not any(n in l for n in noise)]
for l in hits[-250:]:print(l[:300])
print('--- last 120 lines (noise removed) ---')
for l in [l for l in lines if not any(n in l for n in noise)][-120:]:print(l[:300])
