"""Strict source-evidence joins; metadata is neither a grant nor native proof.

Pass the frozen image's explicit Plan tuple. FUTURE_PLANS are host fixtures until
that image actually contains them. Full global streams must precede filtering.
Trust version is absent from old records. It is observed only when both actual
BOOT readback and Net binding diagnostics contain exact typed trust fields;
require_version=True refuses old records. Expected constants are not observations.
"""
from dataclasses import dataclass
import re

@dataclass(frozen=True)
class Plan:
    target:int
    surface:int
    lifecycle:int
    entropy:int
    clock:int
    trust:int

CURRENT_PLANS=(Plan(15,300,40,60,61,62),)
FUTURE_PLANS=CURRENT_PLANS+(Plan(30,343,154,70,71,72),Plan(31,344,158,73,74,75))
def plans_from_catalog(root):
    """Read the bound SDK's literal catalog and source-slot selector, never logs."""
    from pathlib import Path
    root=Path(root)
    catalog=(root/'crates/sex-pdx/src/app_catalog.rs').read_text()
    source=(root/'crates/sex-pdx/src/browser_sources.rs').read_text()
    rows=re.findall(r'Incarnation \{ app: App::Browser, ordinal: ([0-9]+), target: ([0-9]+), surface: ([0-9]+), shell_surface: None, lifecycle_slot: ([0-9]+),',catalog)
    if len(rows)!=catalog.count('Incarnation { app: App::Browser,'):raise ValueError('unsupported Browser catalog syntax')
    body=re.search(r'const fn slots\(ordinal:u8\)->\[u64;3\]\{\s*match ordinal \{(.*?)\n    \}',source,re.S)
    if body is None:raise ValueError('unsupported source slot selector')
    clean=re.sub(r'//[^\n]*','',body[1])
    arms=re.findall(r'([0-9]+)=>\[([0-9]+),([0-9]+),([0-9]+)\],',clean)
    remainder=re.sub(r'[0-9]+=>\[[0-9]+,[0-9]+,[0-9]+\],','',clean)
    if re.sub(r'\s+','',remainder)!='_=>panic!("unsupportedBrowsersource-planordinal"),':raise ValueError('unreviewed slot expression')
    slots={int(a):tuple(map(int,(b,c,d)))for a,b,c,d in arms}
    if len(slots)!=len(arms)or slots!={0:(60,61,62),1:(70,71,72),2:(73,74,75)}:raise ValueError('unreviewed source slots')
    plans=[]
    for index,row in enumerate(rows):
        ordinal,target,surface,lifecycle=map(int,row)
        if ordinal!=index or ordinal not in slots:raise ValueError('noncanonical Browser ordinal')
        plans.append(Plan(target,surface,lifecycle,*slots[ordinal]))
    result=tuple(plans)
    if result not in (CURRENT_PLANS,FUTURE_PLANS):raise ValueError('unreviewed bound catalog')
    return result


MIN_SECONDS=1577836800
MAX_SECONDS=4102444799


def records(trace,tag):
    marker='['+tag+']'
    pattern=re.compile(re.escape(marker)+r' (?P<body>[^\r\n]+)\r?$',re.M)
    found=list(pattern.finditer(trace))
    if len(found)!=trace.count(marker):raise ValueError(tag+': malformed marker')
    result=[]
    for match in found:
        fields={}
        for token in match['body'].split(' '):
            if not re.fullmatch(r'[a-z_]+=[A-Za-z0-9_]+',token):raise ValueError(tag+': malformed field')
            key,value=token.split('=')
            if key in fields:raise ValueError(tag+': duplicate field')
            fields[key]=value
        result.append((match.start(),fields))
    return result


def number(fields,key,maximum=(1<<64)-1):
    if key not in fields:raise ValueError('missing '+key)
    value=fields[key]
    if not re.fullmatch(r'0|[1-9][0-9]*',value):raise ValueError('noncanonical '+key)
    n=int(value)
    if n>maximum:raise ValueError('overflow '+key)
    return n


def exact(fields,expected):
    if fields!={key:str(value)for key,value in expected.items()}:raise ValueError('unexpected source fields: '+repr(fields))


def validate_inventory(trace,plans=CURRENT_PLANS,*,profile=1,root_bytes=376,version=1,require_version=False):
    plans=tuple(plans)
    # Explicitly bounded reviewed plans: never discover authority from logs.
    if plans not in (CURRENT_PLANS,FUTURE_PLANS):raise ValueError('unreviewed source plan')
    if (profile,root_bytes,version)!=(1,376,1):raise ValueError('unreviewed trust contract')
    by_target={p.target:p for p in plans};tasks={};catalog_at={}
    catalog=records(trace,'boot.app.catalog')
    seen=set()
    for at,f in catalog:
        target=number(f,'target',2**32-1)
        if target in seen:raise ValueError('duplicate catalog target')
        seen.add(target)
        if target not in by_target:
            if target in {p.target for p in FUTURE_PLANS}:raise ValueError('unplanned Browser catalog row')
            continue
        p=by_target[target];task=number(f,'task',2**32-1)
        if not task:raise ValueError('zero catalog task')
        # Other catalog data is preserved by its own gate, not trusted as a route.
        required={'target','task','surface','logical','incarnation','predecessor','lifecycle','close','route','dormant','grants_verified','data_only'}
        if set(f)!=required:raise ValueError('catalog shape changed')
        if (number(f,'surface'),number(f,'lifecycle'),f['grants_verified'],f['data_only'])!=(p.surface,p.lifecycle,'1','1'):raise ValueError('wrong catalog association')
        index=plans.index(p)
        close,route=((43,43),(155,157),(159,161))[index]
        exact(f,dict(target=target,task=task,surface=p.surface,logical=15,incarnation=index+1,predecessor=0 if index==0 else plans[index-1].target,lifecycle=p.lifecycle,close=close,route=route,dormant=int(index!=0),grants_verified=1,data_only=1))
        tasks[target]=task;catalog_at[target]=at
    if set(tasks)!=set(by_target):raise ValueError('missing planned catalog identity')
    holder_task=None;slots=set();clock_allowed={};entropy_allowed={};grant_at={}
    kinds=(('entropy','boot.entropy.source'),('clock','boot.clock.source'),('trust','boot.trust.policy'))
    roster={p.target:dict(plan=p,task=tasks[p.target])for p in plans}
    for kind,tag in kinds:
        entries=records(trace,tag);seen=set()
        if len(entries)!=len(plans):raise ValueError(tag+': missing/extra grants')
        for at,f in entries:
            target=number(f,'consumer',2**32-1)
            if target not in by_target or target in seen:raise ValueError('foreign/duplicate consumer')
            seen.add(target);p=by_target[target];ht=number(f,'task',2**32-1)
            if not ht or holder_task not in (None,ht):raise ValueError('inconsistent provider task')
            holder_task=ht;slot=getattr(p,kind)
            base=dict(holder=13,task=ht,slot=slot,consumer=target,consumer_task=tasks[target],runtime_mint=0)
            if kind=='entropy':base.update(source='rdseed',attempts=8)
            elif kind=='clock':base.update(source='q35_cmos_utc',attempts=4,precision=1)
            else:
                base.update(profile=profile,root_bytes=root_bytes)
                if 'version'in f:base['version']=version
                # Old authority record intentionally keeps its byte format.
            exact(f,base)
            key=(13,slot)
            if key in slots:raise ValueError('holder-local slot collision')
            slots.add(key);grant_at[(kind,target)]=at
            if kind=='clock':clock_allowed[(13,ht,slot)]=(target,tasks[target],at)
            if kind=='entropy':entropy_allowed[(13,ht,slot)]=(target,tasks[target],at)
    tls=records(trace,'boot.network.tls')
    if tls:
        seen=set()
        if len(tls)!=len(plans):raise ValueError('missing/extra BOOT TLS consumers')
        for _,fields in tls:
            target=number(fields,'consumer',2**32-1)
            if target not in by_target or target in seen:raise ValueError('foreign/duplicate BOOT TLS consumer')
            seen.add(target)
            exact(fields,dict(holder=13,task=holder_task,consumer=target,consumer_task=tasks[target],profile=1,peer_port=46080,origin='immutable_boot_profile',runtime_mint=0))
    bars=records(trace,'boot.silkbar.clock.source')
    if len(bars)!=1:raise ValueError('missing/duplicate SilkBar grant')
    at,f=bars[0]
    # Current direct civil-role contract is explicitly6/6, independent of Net.
    exact(f,dict(holder=6,task=6,slot=61,consumer=6,consumer_task=6,source='q35_cmos_utc',attempts=4,precision=1,runtime_mint=0))
    clock_allowed[(6,6,61)]=(6,6,at)
    readbacks=records(trace,'boot.browser.sources.readback');readback_at={};observed_readback=set()
    if require_version and len(readbacks)!=len(plans):raise ValueError('missing typed readback inventory')
    for at,f in readbacks:
        target=number(f,'target',2**32-1)
        if target not in by_target or target in readback_at:raise ValueError('foreign/duplicate readback')
        p=by_target[target]
        expected=dict(holder=13,target=target,entropy=p.entropy,clock=p.clock,trust=p.trust,late_before_seal=1,typed_info=1,runtime_context=0)
        if any(k in f for k in ('trust_profile','trust_root_bytes','trust_version')):
            expected.update(trust_profile=profile,trust_root_bytes=root_bytes,trust_version=version);observed_readback.add(target)
        elif require_version:raise ValueError('readback trust version unobserved')
        exact(f,expected)
        if at<=max(catalog_at[target],*(grant_at[(kind,target)]for kind,_ in kinds)):raise ValueError('readback precedes grants/catalog')
        readback_at[target]=at
    bindings=records(trace,'net.source.binding.held');seen=set();binding_at={};observed_binding=set()
    if records(trace,'net.source.binding.unavailable'):raise ValueError('source binding unavailable')
    if len(bindings)!=len(plans):raise ValueError('missing/extra runtime binding')
    for at,f in bindings:
        target=number(f,'target',2**32-1)
        if target not in by_target or target in seen:raise ValueError('foreign/duplicate binding')
        seen.add(target);p=by_target[target]
        expected=dict(target=target,task=tasks[target],surface=p.surface,lifecycle_slot=p.lifecycle,entropy_slot=p.entropy,clock_slot=p.clock,trust_slot=p.trust,before_dispatch=1)
        if any(k in f for k in ('trust_profile','trust_root_bytes','trust_version')):
            expected.update(trust_profile=profile,trust_root_bytes=root_bytes,trust_version=version);observed_binding.add(target)
        elif require_version:raise ValueError('binding trust version unobserved')
        exact(f,expected)
        if target in readback_at and at<=readback_at[target]:raise ValueError('binding precedes readback')
        if at<=max(catalog_at[target],*(grant_at[(kind,target)]for kind,_ in kinds)):raise ValueError('binding precedes BOOT identity/grants')
        binding_at[target]=at
    return dict(roster=roster,holder=(13,holder_task),clock_allowed=clock_allowed,entropy_allowed=entropy_allowed,binding_at=binding_at,trust_version=version,trust_version_observed=observed_readback==set(by_target) and observed_binding==set(by_target),scope='validated_log_metadata_not_capability_genesis_or_native_proof')


def validate_clock_stream(trace,inventory,*,service_semantics=True):
    """Validate ALL holders/slots/global ordinals before any consumer selection.

    Net's one high-water/poison spans every Net clock slot; SilkBar shares kernel
    ordinals but not Net service high-water. Lower kernel samples are legal only
    with exact Net rollback evidence; no later Net source sample is permitted.
    """
    output=[];events=[]
    for ordinal,(at,f)in enumerate(records(trace,'clock.source.snapshot'),1):
        key=tuple(number(f,k,2**32-1 if k!='slot'else 1024)for k in ('pd','task','slot'))
        if key not in inventory['clock_allowed']:raise ValueError('ungranted clock tuple')
        target,task,grant_at=inventory['clock_allowed'][key]
        seconds=number(f,'seconds');attempts=number(f,'attempts')
        if not MIN_SECONDS<=seconds<=MAX_SECONDS or not 1<=attempts<=4:raise ValueError('clock bounds')
        exact(f,dict(pd=key[0],task=key[1],slot=key[2],seconds=seconds,attempts=attempts,precision=1,profile='q35_cmos_utc',ordinal=ordinal))
        if at<=grant_at or (key[0]==13 and at<=inventory['binding_at'][target]):raise ValueError('sample before grant/binding')
        row=dict(offset=at,holder=key,consumer=(target,task),seconds=seconds,attempts=attempts,ordinal=ordinal)
        output.append(row)
        if key[0]==13:events.append((at,'source',row))
    if service_semantics:
        for kind,tag in [('success','certificate.time.snapshot'),('rollback','certificate.time.rollback')]:
            for at,f in records(trace,tag):events.append((at,kind,f))
        high=None;poison=False;pending=None
        for at,kind,r in sorted(events,key=lambda e:e[0]):
            if kind=='source':
                if poison or pending is not None:raise ValueError('source after poison or unmatched source')
                pending=r;continue
            if pending is None:raise ValueError('service result lacks fresh Net sample')
            target,task=pending['consumer'];seq=number(r,'seq',0xffffff)
            if not seq:raise ValueError('zero service sequence')
            backwards=high is not None and pending['seconds']<high
            if kind=='success':
                exact(r,dict(owner=target,task=task,seq=seq,seconds=pending['seconds'],source='q35_cmos_utc',precision=1,receipt='retained'))
                if backwards:raise ValueError('rollback became success')
                high=pending['seconds']
            else:
                exact(r,dict(owner=target,task=task,seq=seq,blocked_until_boot=1,last_good_fallback=0))
                if not backwards:raise ValueError('unsubstantiated rollback')
                poison=True
            pending=None
        if pending is not None:raise ValueError('truncated Net clock result')
    return output


def consumer_clock_sources(trace,inventory,target,task,*,service_semantics=True):
    all_sources=validate_clock_stream(trace,inventory,service_semantics=service_semantics)
    return [row for row in all_sources if row['consumer']==(target,task)]


def validate_entropy_stream(trace,inventory):
    """Kernel SUCCESS is global and logs first4 then powers2, not each word."""
    output=[];expected=1
    for at,f in records(trace,'entropy.source.sample'):
        key=tuple(number(f,k,2**32-1 if k!='slot'else 1024)for k in ('pd','task','slot'))
        if key not in inventory['entropy_allowed']:raise ValueError('ungranted entropy tuple')
        target,task,grant_at=inventory['entropy_allowed'][key];attempts=number(f,'attempts');count=number(f,'count')
        if not 1<=attempts<=8 or count!=expected:raise ValueError('entropy sparse counter/attempt bounds')
        exact(f,dict(pd=key[0],task=key[1],slot=key[2],source='rdseed',attempts=attempts,count=count,private=1))
        if at<=max(grant_at,inventory['binding_at'][target]):raise ValueError('entropy before binding/grant')
        output.append(dict(offset=at,holder=key,consumer=(target,task),count=count,attempts=attempts))
        expected=count+1 if count<4 else count*2
    return output
