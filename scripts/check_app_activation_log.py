"""Bounded first-Quil-successor evidence. Log checks do not prove pixels."""
import re

def validate(text):
    errors=[]
    def rows(tag):
        return [(m.start(),dict(re.findall(r'(\w+)=([^\s\]]+)',m[1]))) for m in re.finditer(r'\['+re.escape(tag)+r'\]([^\r\n]*)',text)]
    def one(tag,**expected):
        found=[(pos,row) for pos,row in rows(tag) if all(row.get(k)==str(v) for k,v in expected.items())]
        if len(found)!=1:errors.append(f'{tag}: expected one matching record, got {len(found)}')
        return found[0] if len(found)==1 else (-1,{})
    reserve=one('boot.app.successor',logical=9,incarnation=2,predecessor=9,successor=16,task=16,surface=301,key=9,dormant=1,runtime_mint=0)
    early=one('app.activation.negative',dormant_route='denied',early_activation='retry',ok=1)
    old=one('app.private.retired',target=9,task=9,acked=31,required=31,all_flushed=1,pool_reusable=1)
    commit=one('app.activation.committed',logical=9,incarnation=2,predecessor=9,successor=16,task=16,surface=301,runtime_alloc=0,runtime_mint=0,bsp=1)
    replay=one('app.activation.negative',stale_predecessor_route='denied',duplicate_activation='receipt',ok=1)
    start=one('quil.incarnation.start',target=16,task=16,surface=301,fresh=1)
    accepted=one('shell.app_surface.accept',sid=301,app_id=16,caller=16)
    presented=one('app.surface.presented',target=16,surface=301,sequence=1,ok=1)
    retired=one('app.private.retired',target=16,task=16,acked=31,required=31,all_flushed=1,pool_reusable=1)
    pool=one('sext.app.pool.ready',target=16,task=16,reused=1,zero_owner='sext',ok=1)
    denied=one('launcher.execute.denied',idx=1,reason='boot_reserve_exhausted')
    order=[reserve[0],early[0],old[0],commit[0],start[0],accepted[0],presented[0],retired[0],pool[0],denied[0]]
    if any(p<0 for p in order) or order!=sorted(order) or len(set(order))!=len(order):errors.append('activation/registration/presentation/retirement/exhaustion order invalid')
    if not commit[0]<replay[0]<retired[0]:errors.append('replay/stale-route probe outside active incarnation')
    for target,before in [(9,old[0]),(16,retired[0])]:
        acks=[(pos,row) for pos,row in rows('app.service.ack') if row.get('target')==str(target)]
        if {r.get('role') for _,r in acks}!={'1','2','4','8','16'} or any(pos>=before for pos,_ in acks):errors.append(f'target{target}: five service roles not complete before retirement')
    running=[(m.start(),dict(re.findall(r'(\w+)=([^\s]+)',m[1]))) for m in re.finditer(r'\btask\.running ([^\r\n]*)',text)]
    for pos,row in running:
        if row.get('pd_id')=='16' and pos<commit[0]:errors.append('successor ran before authorized activation')
    for pos,row in rows('quil.incarnation.start'):
        if row.get('target')=='9' and pos>commit[0]:errors.append('predecessor restarted')
    try:
        if int(reserve[1]['leaves'])!=int(commit[1]['pages']) or int(commit[1]['pages'])!=int(retired[1]['pages']):errors.append('reserved/restored/retired page counts mismatch')
        if int(retired[1]['bytes'])!=int(pool[1]['bytes']) or pool[1]['bytes']!=pool[1]['zeroed']:errors.append('successor pool not fully scrubbed')
    except (KeyError,ValueError):errors.append('missing or invalid reserved page accounting')
    if re.search(r'KERNEL PAGE FAULT|DOUBLE FAULT|KERNEL PANIC|GP FAULT|panicked at',text):errors.append('guest fault')
    return errors
