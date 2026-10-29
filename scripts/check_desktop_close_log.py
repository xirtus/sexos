#!/usr/bin/env python3
"""Strict joins for three cooperative GUI Close operations in one real boot."""
import argparse
import pathlib
import re

def records_from(text):
    return [(m.start(),m[1],dict(re.findall(r'([a-z_]+)=([^\s]+)',m[2])))
        for m in re.finditer(r'\[([A-Za-z0-9_.]+)\] ([^\[\r\n]*)',text)]

def responsiveness(text,after=0):
    records=records_from(text)
    starts=[i for i,n,f in records if n=='shell.linen.rescan.sent' and i>after]
    evidence=[]
    for offset,start in enumerate(starts):
        end=starts[offset+1] if offset+1<len(starts) else len(text)
        previous_ids=set();staged=set()
        for i,n,f in records:
            if i>=start:break
            if n=='linen.remote.entry':staged.add(f.get('id'))
            if n=='linen.remote.snapshot.ok':previous_ids=staged;staged=set()
        backend=[i for i,n,f in records if start<i<end and n in {'linen.disk.rescan.ok','linen.disk.rescan.skip'}]
        if not backend:continue
        commits=[(i,f) for i,n,f in records if backend[0]<i<end and n=='linen.remote.snapshot.ok']
        if not commits:continue
        commit,fields=commits[0]
        actions=[(i,f) for i,n,f in records if start<i<commit and n=='shell.action.select_next_linen']
        currents=[(i,f) for i,n,f in records if commit<i<end and n=='linen.object_select.current']
        if not actions or not currents:continue
        selected=actions[-1][1].get('id')
        if selected not in previous_ids or fields.get('selection_preserved')!='1' or currents[0][1].get('id')!=selected:continue
        evidence.append({'start':start,'backend':backend[0],'action':actions[-1][0],'commit':commit,'selected':selected,'current':currents[0][0]})
    return evidence

def check(text, include_successor=False, include_spindle_successor=False, include_third_quil=False):
    errors=[]
    records=records_from(text)
    def rows(name,target):
        return [(i,f) for i,n,f in records if n==name and f.get('target')==str(target)]
    def one(name,target):
        found=rows(name,target)
        if len(found)!=1:errors.append(f'{name} target={target}: expected one, got {len(found)}');return None
        return found[0]
    if re.search(r'KERNEL PAGE FAULT|DOUBLE FAULT|KERNEL PANIC|GP FAULT|fault\.kill|panicked at',text):errors.append('unexpected fatal fault')
    presented=rows('display.app.presented',15)
    if not presented or not any(f.get('surface')=='300' and f.get('scene')=='1' for _,f in presented):errors.append('native300 never presented')
    final_pool=-1
    for target,sid in [(15,300),(9,201),(12,153)]+([(16,301)] if include_successor else [])+([(17,306)] if include_spindle_successor else [])+([(18,307)] if include_third_quil else []):
        close=one('shell.app.close.request',target)
        # Kaleido's authenticated close handler directly finishes; only Quil
        # and Spindle emit the additional receiver diagnostic.
        accepted=one('app.close.accepted',target) if target!=15 else None
        prepared=one('display.app.cleanup.prepared',target)
        committed=one('display.app.cleanup.committed',target)
        shell=one('shell.app.cleanup',target)
        retired=one('app.private.retired',target)
        pool=one('sext.app.pool.ready',target)
        if not all([close,prepared,committed,shell,retired,pool]) or (target!=15 and not accepted):continue
        task=close[1].get('task')
        if not task or not task.isdecimal() or int(task)==0:errors.append(f'target{target}: invalid task');continue
        if close[1].get('surface')!=str(sid):errors.append(f'target{target}: wrong close surface')
        for label,row in [('accepted',accepted),('prepared',prepared),('committed',committed),('shell',shell),('retired',retired),('pool',pool)]:
            if row and row[1].get('task')!=task:errors.append(f'target{target}: {label} task mismatch')
        sends=[(i,f) for i,f in rows('shell.app.close.send',target) if f.get('task')==task and f.get('accepted')=='1' and f.get('slot')==str({9:41,12:42,15:43,16:46,17:51,18:56}[target])]
        if not sends or not any(close[0]<i<retired[0] for i,_ in sends):errors.append(f'target{target}: no accepted close delivery')
        finish=[(i,f) for i,n,f in records if n=='task.finish' and f.get('pd')==str(target) and f.get('task')==task and f.get('terminal')=='1']
        if len(finish)!=1:errors.append(f'target{target}: missing/duplicate exact task finish')
        elif not close[0]<finish[0][0]<prepared[0]<=committed[0]<retired[0]<pool[0]:errors.append(f'target{target}: invalid teardown order')
        elif accepted and not close[0]<accepted[0]<finish[0][0]:errors.append(f'target{target}: invalid receiver acceptance order')
        if target==15 and presented and not min(i for i,_ in presented)<close[0]:errors.append('native300 close preceded presentation')
        if shell[1].get('retired')!='1' or shell[1].get('surface')!=str({15:300,9:156,12:154,16:301,17:302,18:307}[target]) or shell[1].get('focus') in {str(sid),str({15:300,9:156,12:154,16:301,17:302,18:307}[target])}:errors.append(f'target{target}: stale shell focus/retirement')
        if accepted and accepted[1].get('coordinator')!='3':errors.append(f'target{target}: close accepted from wrong coordinator')
        if not prepared[1].get('removed','').isdecimal() or int(prepared[1].get('removed','0'))>16:errors.append(f'target{target}: invalid display resource count')
        roles=set()
        for i,f in rows('app.service.ack',target):
            if f.get('task')!=task or f.get('required')!='31':errors.append(f'target{target}: mismatched ACK identity');continue
            try:role=int(f.get('role',''));holder=int(f.get('holder',''))
            except ValueError:errors.append('malformed role');continue
            if {1:3,2:1,4:11,8:13,16:10}.get(role)!=holder:errors.append(f'target{target}: wrong ACK holder')
            if not close[0]<i<retired[0]:errors.append(f'target{target}: ACK outside transaction')
            if role==1 and i<shell[0]:errors.append(f'target{target}: shell ACK before publication')
            if role==2 and i<committed[0]:errors.append(f'target{target}: display ACK before scene commit')
            if role in roles:errors.append(f'target{target}: duplicate service ACK')
            if finish and i<=finish[0][0]:errors.append(f'target{target}: ACK before task finish')
            roles.add(role)
        if roles!={1,2,4,8,16}:errors.append(f'target{target}: incomplete service ACKs {roles}')
        r=retired[1];p=pool[1]
        try:
            size=int(r.get('bytes',''));pages=int(r.get('pages',''))
            if size<=0 or size!=pages*4096 or int(p.get('bytes',''))!=size or int(p.get('zeroed',''))!=size:raise ValueError()
        except ValueError:errors.append(f'target{target}: invalid private pool accounting')
        if any(r.get(k)!=v for k,v in {'acked':'31','required':'31','all_flushed':'1','pool_reusable':'1','metadata':'retained','runtime_alloc':'0'}.items()):errors.append(f'target{target}: incomplete private retirement')
        if any(p.get(k)!=v for k,v in {'ok':'1','reused':'1','zero_owner':'sext'}.items()):errors.append(f'target{target}: pool not safely reused')
        retired_surfaces={str(sid),str({15:300,9:156,12:154,16:301,17:302,18:307}[target])}
        for i,name,fields in records:
            if i<=retired[0]:continue
            if name=='shell.focus.set' and fields.get('id') in retired_surfaces:errors.append(f'target{target}: retired surface regained focus')
            if name=='shell.app_surface.accept' and fields.get('caller')==str(target):errors.append(f'target{target}: retired app registered again')
        final_pool=max(final_pool,pool[0])
    # Survivor liveness must occur after every native app has retired.
    snapshots=[i for i,n,f in records if n=='linen.remote.snapshot.ok' and f.get('complete')=='1' and f.get('unique')=='1' and f.get('count','0').isdigit() and int(f['count'])>0 and i>final_pool]
    selections=[i for i,n,f in records if n=='shell.action.select_next_linen' and f.get('id','0').isdigit() and int(f['id'])>0 and i>final_pool]
    if not snapshots or not selections or max(selections)<=min(snapshots):errors.append('missing surviving Linen refresh + input selection after all cleanup')
    if not responsiveness(text,final_pool):errors.append('no input progressed during a real rescan with old-view selection preserved')
    return errors

def main():
    parser=argparse.ArgumentParser();parser.add_argument('log',type=pathlib.Path);args=parser.parse_args()
    try:errors=check(args.log.read_text(errors='replace'))
    except OSError as exc:errors=[str(exc)]
    for error in errors:print('FAIL:',error)
    print('[desktop.close.gate.result] '+('FAIL' if errors else 'PASS')+' scope=three_native_GUI_close_private_cleanup_surviving_input')
    return bool(errors)
if __name__=='__main__':raise SystemExit(main())
