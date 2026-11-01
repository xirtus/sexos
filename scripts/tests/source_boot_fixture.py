"""Complete synthetic current BOOT metadata, for positive fixture construction.

Apply this decorator before test mutations. It is never an evidence validator.
"""
from functools import wraps
import re
import sys
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import browser_source_evidence as e
from test_browser_source_evidence import inventory

TAGS=('boot.entropy.source','boot.clock.source','boot.trust.policy','boot.silkbar.clock.source','boot.browser.sources.readback','net.source.binding.held')

def complete(text):
    data=inventory(e.FUTURE_PLANS,observed=True)
    for old,new in ((150,15),(300,30),(310,31),(130,13)):
        data=re.sub(r'(?<=task=)'+str(old)+r'\b',str(new),data)
    ordered=[]
    for line in data.splitlines(keepends=True):
        tag=line.split(']')[0]+']'
        if tag in ('[boot.entropy.source]','[boot.clock.source]','[boot.trust.policy]'):
            fields=dict(token.split('=')for token in line.split()[1:])
            keys=['holder','task','slot','consumer','consumer_task']
            keys+=['profile','root_bytes']if tag=='[boot.trust.policy]'else['source','attempts']+(['precision']if tag=='[boot.clock.source]'else[])
            keys+=['runtime_mint']
            line=tag+' '+' '.join(key+'='+fields[key]for key in keys)+'\n'
        ordered.append(line)
    data=''.join(ordered)
    kept=[]
    for line in text.splitlines(keepends=True):
        if any('['+tag+']'in line for tag in TAGS):continue
        if '[boot.app.catalog]'in line and re.search(r'\btarget=(15|30|31)\b',line):continue
        kept.append(line)
    if '[boot.network.tls]'in text:
        kept=[line for line in kept if '[boot.network.tls]'not in line]
        data+=''.join(f'[boot.network.tls] holder=13 task=13 consumer={target} consumer_task={target} profile=1 peer_port=46080 origin=immutable_boot_profile runtime_mint=0\n'for target in (15,30,31))
    body=''.join(kept)
    # All source bindings have startup INFO observations in the actual program.
    for tag in ('sexnet.entropy.source','sexnet.clock.source'):
        lines=[line for line in body.splitlines(keepends=True)if '['+tag+']'in line]
        if len(lines)==1:body=body.replace(lines[0],lines[0]*3)
    return data+body

def with_inventory(function):
    @wraps(function)
    def wrapped(*args,**kwargs):return complete(function(*args,**kwargs))
    return wrapped
