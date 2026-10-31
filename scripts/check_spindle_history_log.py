"""Join real private-history completion to SexFiles commit/close records.
Host tests establish hostile-input behavior; these joins establish one observed
exchange; durable mode additionally requires BOOT, barriers and readback.
Cross-boot proof uses spindle_durable_reboot_gate.py; no complete-system claim.
"""
import re
from pathlib import Path
from history_roster_evidence import verify_boot_roster,source_spindles

def validate(text,durable=False,*,expected_targets=None):
    errors=[]
    source_targets=[row["target"]for row in source_spindles(Path(__file__).resolve().parents[1])]
    # A shorter regression must explicitly name the canonical successor prefix.
    # This narrows exercised subjects, never the held BOOT roster validation.
    if expected_targets is None:
        targets=source_targets
    elif (type(expected_targets) not in (tuple,list)
          or not 2<=len(expected_targets)<=len(source_targets)
          or any(type(target) is not int for target in expected_targets)
          or list(expected_targets)!=source_targets[:len(expected_targets)]):
        return ['expected targets must be a canonical BOOT successor prefix of at least two subjects']
    else:
        targets=list(expected_targets)
    rows=[]
    for m in re.finditer(r'\[(files\.history\.(?:open|commit|close|retire|durable|recovered)|spindle\.history\.receipt|app\.private\.retired|app\.service\.ack|boot\.history\.recovery)\] ([^\r\n]*)',text):
        rows.append((m.start(),m[1],dict(re.findall(r'(\w+)=([^\s]+)',m[2]))))
    def find(tag,**wanted):return [(p,r)for p,t,r in rows if t==tag and all(r.get(k)==str(v)for k,v in wanted.items())]
    receipts=find('spindle.history.receipt')
    for target in targets:
        completed=find('spindle.history.receipt',target=target,kind='saved')
        if not completed:errors.append(f'target{target}: no save completion')
        for pos,r in completed+find('spindle.history.receipt',target=target,kind='loaded'):
            try:
                handle=int(r['handle']);size=int(r['bytes']);count=int(r['count'])
                if not 0<handle<=0xFFFFFFFF or not 24<=size<=4096 or not 0<=count<=128 or r.get('error')!='0':raise ValueError()
            except (KeyError,ValueError):errors.append('invalid completed history metadata');continue
            mode='save'if r['kind']=='saved'else'load'
            opens=[(p,x)for p,x in find('files.history.open',target=target,handle=handle,mode=mode)if p<pos]
            closes=[(p,x)for p,x in find('files.history.close',target=target,handle=handle,ok=1)if p<pos]
            if len(opens)!=1 or len(closes)!=1 or not opens[0][0]<closes[0][0]:errors.append(f'target{target}: missing exact open/close before completion');continue
            if mode=='save':
                commits=[(p,x)for p,x in find('files.history.commit',target=target,handle=handle,bytes=size,hash=r.get('hash'))if opens[0][0]<p<closes[0][0]]
                if len(commits)!=1:errors.append(f'target{target}: save lacks matching atomic commit')
                if durable and (not commits or not any(opens[0][0]<p<commits[0][0] for p,x in find('files.history.durable',target=target,bytes=size,hash=r.get('hash'),barriers=3,readback=1))):errors.append(f'target{target}: no completed durable barrier/readback before RAM publication')
            else:
                matching=[x for p,x in completed if p<opens[0][0] and all(x.get(k)==r.get(k)for k in ['bytes','count','hash']) and x.get('handle')!=r.get('handle')]
                recovered=[x for p,x in find('files.history.recovered',target=target,bytes=size,owned_hash=r.get('hash'),boot_grant=53) if p<opens[0][0]] if durable else []
                if not matching and not recovered:errors.append(f'target{target}: load did not recover exact saved snapshot through fresh handle')
        observed=[int(r['seq'])for p,t,r in rows if t in {'files.history.open','files.history.commit','files.history.close'}and r.get('target')==str(target) and r.get('seq','').isdigit()]
        if observed!=sorted(set(observed)):errors.append(f'target{target}: duplicate or reversed service sequence')
        retired=find('app.private.retired',target=target)
        clean=find('files.history.retire',target=target,staged=0,pending=0,terminal=1)
        files_ack=find('app.service.ack',target=target,role=4)
        if len(retired)!=1 or len(clean)!=1 or len(files_ack)!=1 or not clean[0][0]<files_ack[0][0]<retired[0][0]:errors.append(f'target{target}: history custody not discarded before files ACK/retirement')
        if retired and any(p>retired[0][0] for p,r in find('files.history.open',target=target)+find('files.history.commit',target=target)+find('spindle.history.receipt',target=target,kind='saved')):errors.append(f'target{target}: terminal subject used history again')
    roster=None
    grants=[]
    if durable:
        grants=find('boot.history.recovery',holder=11,slot=53,logical=source_targets[0],participants=len(source_targets),version=1,base=1808,sectors=18,held_query_verified=1,runtime_mint=0)
        try:roster=verify_boot_roster(Path(__file__).resolve().parents[1],text)
        except ValueError as error:errors.append('invalid held BOOT history roster: '+str(error))
        generations=[int(r['generation'])for _,r in find('files.history.durable')if r.get('generation','').isdigit()]
        if generations!=sorted(set(generations)):errors.append('durable generations duplicate or reverse')
    for predecessor,target in zip(targets,targets[1:]):
        missing=find('spindle.history.receipt',target=target,kind='missing',handle=0,bytes=0,count=0)
        old=find('app.private.retired',target=predecessor)
        saved=find('spindle.history.receipt',target=target,kind='saved')
        if durable:
            task=next((p['task']for p in roster['participants']if p['target']==predecessor),None)if roster else None
            recovery=find('files.history.recovered',logical=targets[0],source=predecessor,source_task=task,target=target,boot_grant=53)
            if len(grants)!=1 or len(recovery)!=1 or not old or not saved or not grants[0][0]<old[0][0]<recovery[0][0]<saved[0][0]:errors.append('missing exact BOOT-sanctioned successor data recovery')
            if recovery and not find('files.history.durable',logical=targets[0],target=predecessor,generation=recovery[0][1].get('generation'),hash=recovery[0][1].get('source_hash'),bytes=recovery[0][1].get('bytes')):errors.append('recovery did not match predecessor durable generation')
        elif len(missing)!=1 or not old or not saved or not old[0][0]<missing[0][0]<saved[0][0]:errors.append('fresh successor did not establish its own initially missing history')
        if not find('spindle.history.receipt',target=target,kind='loaded'):errors.append('successor never restored its private snapshot')
    if any(r.get('target')not in {str(target)for target in targets}for _,r in receipts):errors.append('history receipt used subject outside expected BOOT chain')
    if roster:
        tasks={str(p['target']):str(p['task'])for p in roster['participants']}
        if any(r.get('task')!=tasks.get(r.get('target'))for _,r in receipts):errors.append('history receipt task differs from held BOOT roster')
    return errors
