#!/usr/bin/env python3
"""Observed cross-service app retirement; does not establish SMP/restart safety."""
import argparse
from pathlib import Path
import re

def validate(text):
    errors=[]
    records={}
    for m in re.finditer(r'\[([a-zA-Z0-9_.]+)\](?: ([^\[\r\n]*))?',text):
        row=dict(part.split('=',1) for part in (m[2] or '').split() if '=' in part)
        row['_pos']=m.start();records.setdefault(m[1],[]).append(row)
    def rows(name):return [r for r in records.get(name,[]) if r.get('target')=='15']
    def one(name):
        result=rows(name)
        if len(result)!=1:errors.append(f'{name}: expected one target15, got {len(result)}')
        return result[0] if result else {}
    def eq(row,**values):
        for key,value in values.items():
            if row.get(key)!=str(value):errors.append(f'{key}: expected {value}, got {row.get(key)}')
    boot=one('boot.app.resources');acquired=one('app.resources.acquired')
    task=boot.get('task')
    eq(boot,surface=300,slot=40,required=31,pager=14,coordinator=3)
    eq(acquired,surface=300,notification=1,resident=4,private_heap='elf_owned')
    if not acquired.get('handle','').isdigit() or int(acquired.get('handle',0))<=0:errors.append('no real file handle')
    accepted=[r for r in records.get('shell.app_surface.accept',[]) if r.get('sid')=='300' and r.get('caller')=='15']
    if len(accepted)!=1 or accepted[0]['_pos']>=acquired.get('_pos',-1):errors.append('live shell surface was not registered before exit')
    presented=one('display.app.presented');eq(presented,surface=300,scene=1)
    if not presented.get('_pos',10**9)<acquired.get('_pos',-1):errors.append('native app pixels were not presented before exit')
    finishes=[r for r in records.get('task.finish',[]) if r.get('pd')=='15']
    if len(finishes)!=1 or finishes[0].get('task')!=task or finishes[0].get('terminal')!='1':errors.append('missing exact original task finish')
    terminal_pos=finishes[0]['_pos'] if len(finishes)==1 else -1
    if terminal_pos<=acquired.get('_pos',10**9):errors.append('finish before live resource acquisition')
    role_rows=rows('app.service.ack')
    if {r.get('role') for r in role_rows}!={'1','2','4','8','16'} or len(role_rows)!=5:errors.append('missing/duplicate service role ACK')
    retired=one('app.private.retired');pool=one('sext.app.pool.ready')
    eq(retired,task=task,acked=31,required=31,all_flushed=1,pool_reusable=1,metadata='retained',runtime_alloc=0)
    eq(pool,task=task,ok=1,reused=1,zero_owner='sext',workspace_records=2)
    if retired.get('bytes')!=boot.get('private_bytes') or pool.get('bytes')!=retired.get('bytes') or pool.get('zeroed')!=pool.get('bytes'):errors.append('private pool not fully detached/zeroed')
    try:
        if int(retired['pages'])*4096!=int(retired['bytes']) or int(retired['pages'])<16:errors.append('private page accounting')
    except (KeyError,ValueError):errors.append('invalid private page totals')
    for name in ['shell.app.cleanup','shell.app.cleanup.ack','display.app.cleanup.prepared','display.app.cleanup.committed','display.app.cleanup.ack','files.app.cleanup','files.app.cleanup.ack','net.app.cleanup.ack','bell.app.cleanup','bell.app.cleanup.ack']:
        row=one(name)
        if row.get('task')!=task:errors.append(name+': wrong task')
        if not terminal_pos<row.get('_pos',-1)<retired.get('_pos',-1):errors.append(name+': retirement ordering')
    if any(r.get('task')!=task or not terminal_pos<r['_pos']<retired.get('_pos',-1) for r in role_rows):errors.append('role ACK identity/order')
    prepared=one('display.app.cleanup.prepared');committed=one('display.app.cleanup.committed');display_ack=one('display.app.cleanup.ack')
    if not prepared.get('_pos',10**9)<committed.get('_pos',-1)<display_ack.get('_pos',-1):errors.append('display prepare/scene commit/ACK order')
    for row in role_rows:
        if row.get('holder')!={'1':'3','2':'1','4':'11','8':'13','16':'10'}.get(row.get('role')):errors.append('wrong service ACK holder')
    closed=one('paging.target.cleanup');eq(closed,task=task,leaves=4,closed=1,all_flushed=1)
    if not terminal_pos<closed.get('_pos',10**9)<retired.get('_pos',-1)<pool.get('_pos',-1):errors.append('demand/private/zero order')
    if not any(r['_pos']>pool.get('_pos',10**9) for r in records.get('silkbar.clock.send',[])):errors.append('no desktop clock after pool reuse')
    if re.search(r'(?:\[(?:panic|paging.client.fatal|sext.fatal|paging.fault.terminal)|\bfault\.kill\b)',text):errors.append('fatal marker')
    return errors

def main():
    p=argparse.ArgumentParser();p.add_argument('log',type=Path);a=p.parse_args()
    errors=validate(a.log.read_text(errors='replace'))
    for e in errors:print('[app.resources.runtime] FAIL',e)
    if not errors:print('[app.resources.runtime] PASS roles=5 surface=300 demand_pages=4 private_pool=detached_zeroed_reused desktop=live')
    return bool(errors)
if __name__=='__main__':raise SystemExit(main())
