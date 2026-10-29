"""Real bounded input bursts with separate input, admission, application and pixel observations."""
import json
import re
import time
from check_quil_ui_pixels import wait_view


def run(c, bursts):
    if not 1 <= bursts <= 100:
        raise ValueError('typing bursts must be 1..100')
    q,text,wait,capture=(c[k]for k in ['q','text','wait','capture'])
    out,second=c['out'],c['second']
    c['park_pointer']()
    capture('typing-start','#')
    samples=[]
    for index in range(bursts):
        # <=10 HID events per submission fits the existing bounded input lane.
        added='x'*(1 if index%2==0 else 5)
        expected=len(second['content'])+len(added)
        start=len(text());sent=time.monotonic()
        q.key_sequence(list(added))
        wait(r'\[quil.document.edit\].*bytes='+str(expected)+r'(?:\s|$)',start)
        input_receipt=time.monotonic()-sent
        second['content']+=added
        wait(r'\[quil.ui.view\].*cursor='+str(expected)+r' ',start)
        v=wait_view(text)
        if v['cursor']!=expected:raise RuntimeError('input burst cursor differs: '+str(v['cursor']))
        admitted=time.monotonic()-sent
        wait(r'\[quil.ui.applied\] paint='+str(v['paint'])+r' bytes=\d+ pixels=unconfirmed',start)
        applied=time.monotonic()-sent
        capture('typing-'+str(index).zfill(3),'#')
        samples.append(dict(keys=len(added),cursor=expected,paint=v['paint'],
                            input_receipt_seconds=input_receipt,admitted_seconds=admitted,
                            applied_seconds=applied,pixel_seconds=time.monotonic()-sent))
        (out/'input-latency.json').write_text(json.dumps(samples,indent=2)+'\n')
    start=len(text());c['ctrl']('s',r'\[quil.document.saved\].*dirty=0')
    m=re.findall(r'\[quil.persist.save.ok\] bytes=(\d+) hash=(0x[0-9a-f]+) doc=(\d+)',text()[start:])[-1]
    if int(m[0])!=len(second['content']) or int(m[2])!=second['object']:raise RuntimeError('burst save binding differs')
    second.update(bytes=int(m[0]),hash=m[1]);capture('typing-saved','uibeta')
    return dict(bursts=bursts,keys=sum(s['keys']for s in samples),samples=samples,
                scope='real bounded HID bursts; exact latest text/caret glyphs and durable final payload; host observations include polling and framebuffer capture')
