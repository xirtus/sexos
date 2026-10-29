#!/usr/bin/env python3
"""Join sustained native alpha evidence; a short rehearsal cannot pass."""
import argparse
import hashlib
import json
import math
import pathlib
import re
from check_desktop_close_log import check as cleanup_errors
from check_document_disk import recover


def session_errors(result):
    errors=[]
    if result.get('ok') is not True or result.get('acceptance') is not True:
        errors.append('not an accepted session (rehearsals are excluded)')
    soak=result.get('soak') or {}
    if soak.get('acceptance') is not True:
        errors.append('sustained acceptance missing')
    elapsed=soak.get('elapsed_seconds',0)
    if isinstance(elapsed,bool) or not isinstance(elapsed,(int,float)) or not math.isfinite(elapsed) or elapsed<1800:
        errors.append('less than 30 minutes of sustained work')
    cycles=soak.get('cycles',0)
    if not isinstance(cycles,int) or isinstance(cycles,bool) or cycles<12:
        errors.append('insufficient active mixed-app cycles')
    samples=soak.get('samples',[])
    for kind in ['quil_input_receipt','quil_input_pixels','warm_save_receipt','warm_save_pixels','spindle_stat','linen_refresh']:
        values=[s for s in samples if s.get('kind')==kind]
        if len(values)<cycles:errors.append('missing per-cycle latency: '+kind)
        for value in values:
            n=value.get('seconds')
            if isinstance(n,bool) or not isinstance(n,(int,float)) or not math.isfinite(n) or n<0:
                errors.append('invalid observed latency: '+kind)
    if not all(soak.get(k) is True for k in ['pending_quil_close','pending_spindle_close']):
        errors.append('both pending-I/O Close cases are required')
    if soak.get('boot_successors')!=[16,17] or soak.get('cleanup_errors')!=[]:
        errors.append('fresh BOOT successors and complete cleanup required')
    docs=result.get('documents',[])
    if len(docs)!=2 or len({d.get('object') for d in docs})!=2:
        errors.append('two distinct saved documents required')
    if not result.get('keyboard') or not result.get('pointer') or not result.get('edit_during_save_dirty'):
        errors.append('actual input and immutable save revision evidence missing')
    return errors


def inspect(path):
    path=pathlib.Path(path)
    result=json.loads((path/'result.json').read_text())
    errors=session_errors(result)
    text=(path/'serial.log').read_text(errors='replace')
    errors+=cleanup_errors(text,include_successor=True,include_spindle_successor=True)
    if re.search(r'\[sexfiles.defer.drop\]|serial\.record\.overflow|KERNEL PANIC|KERNEL PAGE FAULT|DOUBLE FAULT|GP FAULT|panicked at',text):
        errors.append('fault, dropped service request or diagnostic overflow')
    manifest=json.loads((path/'manifest.json').read_text())
    binding=json.loads((path/'source-binding.json').read_text())
    digest=result.get('iso_sha256')
    if not digest or digest!=manifest.get('iso_sha256') or digest!=binding.get('iso_sha256'):
        errors.append('session/source/image bindings differ')
    disk=(path/'nvme.img').read_bytes()
    documents=[]
    for expected in result.get('documents',[]):
        found=recover(disk,expected['object'])
        payload=found.pop('payload')
        if payload!=expected['content'].encode('ascii') or len(payload)!=expected['bytes']:
            errors.append('saved document bytes differ: '+expected['name'])
        documents.append(found)
    retirements=[dict(re.findall(r'([a-z_]+)=([^\s]+)',m)) for m in re.findall(r'\[app.private.retired\] ([^\r\n]+)',text)]
    pages=sum(int(r['pages']) for r in retirements)
    return dict(ok=not errors,errors=errors,iso_sha256=digest,
                elapsed_seconds=(result.get('soak')or{}).get('elapsed_seconds'),
                cycles=(result.get('soak')or{}).get('cycles'),private_pages_zeroed_and_reused=pages,
                documents=documents,disk_sha256=hashlib.sha256(disk).hexdigest(),
                scope='real native session, exact durable payloads and five-role private cleanup; reboot checked separately')


def main():
    p=argparse.ArgumentParser();p.add_argument('session',type=pathlib.Path);a=p.parse_args()
    try:result=inspect(a.session)
    except (OSError,ValueError,KeyError,TypeError) as e:result=dict(ok=False,errors=[str(e)])
    print(json.dumps(result,indent=2));return not result['ok']
if __name__=='__main__':raise SystemExit(main())
