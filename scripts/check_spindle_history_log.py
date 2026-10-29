"""Join real private-history completion to SexFiles commit/close records.
Host tests establish hostile-input behavior; these joins establish one observed
same-boot exchange, never cross-reboot durability or complete-system proof.
"""
import re

def validate(text):
    errors=[]
    rows=[]
    for m in re.finditer(r'\[(files\.history\.(?:open|commit|close|retire)|spindle\.history\.receipt|app\.private\.retired|app\.service\.ack)\] ([^\r\n]*)',text):
        rows.append((m.start(),m[1],dict(re.findall(r'(\w+)=([^\s]+)',m[2]))))
    def find(tag,**wanted):return [(p,r)for p,t,r in rows if t==tag and all(r.get(k)==str(v)for k,v in wanted.items())]
    receipts=find('spindle.history.receipt')
    for target in [12,17]:
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
            else:
                matching=[x for p,x in completed if p<opens[0][0] and all(x.get(k)==r.get(k)for k in ['bytes','count','hash']) and x.get('handle')!=r.get('handle')]
                if not matching:errors.append(f'target{target}: load did not recover exact saved snapshot through fresh handle')
        observed=[int(r['seq'])for p,t,r in rows if t in {'files.history.open','files.history.commit','files.history.close'}and r.get('target')==str(target) and r.get('seq','').isdigit()]
        if observed!=sorted(set(observed)):errors.append(f'target{target}: duplicate or reversed service sequence')
        retired=find('app.private.retired',target=target)
        clean=find('files.history.retire',target=target,staged=0,pending=0,terminal=1)
        files_ack=find('app.service.ack',target=target,role=4)
        if len(retired)!=1 or len(clean)!=1 or len(files_ack)!=1 or not clean[0][0]<files_ack[0][0]<retired[0][0]:errors.append(f'target{target}: history custody not discarded before files ACK/retirement')
        if retired and any(p>retired[0][0] for p,r in find('files.history.open',target=target)+find('files.history.commit',target=target)+find('spindle.history.receipt',target=target,kind='saved')):errors.append(f'target{target}: terminal subject used history again')
    missing=find('spindle.history.receipt',target=17,kind='missing',handle=0,bytes=0,count=0)
    old=find('app.private.retired',target=12)
    saved=find('spindle.history.receipt',target=17,kind='saved')
    loaded=find('spindle.history.receipt',target=17,kind='loaded')
    if len(missing)!=1 or not old or not saved or not old[0][0]<missing[0][0]<saved[0][0]:errors.append('fresh successor did not establish its own initially missing history')
    if not loaded:errors.append('successor never restored its private snapshot')
    if any(r.get('target') not in {'12','17'}for _,r in receipts):errors.append('history receipt used non-BOOT subject')
    return errors
