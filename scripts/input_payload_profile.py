#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Declare an UNSAFE keyboard-payload proof profile; never for private input.

This changes only reviewed userspace stage flags. Build it through the canonical
entrypoint, preserve its manifest/trace, then restore and rebuild normal artifacts.
Kernel diagnostics remain a separately reviewed/authorized source change.
"""
import argparse,json,re,tomllib
from pathlib import Path
FLAG='--cfg sexos_input_payload_proof'
STAGES=('build_silk_shell','build_sexinput')

def diagnostic_spec(text):
    parsed=tomllib.loads(text);rows={row['id']:row for row in parsed['stage']}
    if any(FLAG in row.get('rustflags','') for row in parsed['stage']):
        raise ValueError('input profile must start from a normal declaration')
    for stage in STAGES:
        if stage not in rows:raise ValueError('missing input stage '+stage)
    pattern=r'^\[\[stage\]\]\s*\n.*?(?=^\[\[stage\]\]|\Z)'
    seen=set()
    def replace(match):
        block=match.group();row=tomllib.loads(block)['stage'][0]
        if row['id'] not in STAGES:return block
        if row['id'] in seen:raise ValueError('duplicate stage '+row['id'])
        seen.add(row['id']);flags=' '.join(filter(None,[row.get('rustflags',''),FLAG]))
        line='rustflags = '+json.dumps(flags)
        if 'rustflags' in row:
            return re.sub(r'^rustflags\s*=.*$',lambda _:line,block,count=1,flags=re.M)
        return block.rstrip()+'\n'+line+'\n\n'
    result=re.sub(pattern,replace,text,flags=re.M|re.S)
    if seen!=set(STAGES):raise ValueError('stage profile incomplete')
    # Parse the result too; every other stage and top-level value must be intact.
    after=tomllib.loads(result)
    if len(after['stage'])!=len(parsed['stage']) or {k:v for k,v in after.items() if k!='stage'}!={k:v for k,v in parsed.items() if k!='stage'}:raise ValueError('profile changed an unrelated declaration')
    for a,b in zip(parsed['stage'],after['stage']):
        wanted=dict(a)
        if a['id'] in STAGES:wanted['rustflags']=' '.join(filter(None,[a.get('rustflags',''),FLAG]))
        if b!=wanted:raise ValueError('profile changed an unrelated declaration')
    return result

PROOF_RECEIPTS=''.join(f'[input.payload.mode] component={component} mode=proof\n' for component in ('sexinput','shell'))
def assert_payload_proof(trace):
    for component in ('sexinput','shell'):
        values=re.findall(rf'\[input.payload.mode\] component={component} mode=(proof|redacted)(?=\s|$)',trace)
        if values!=['proof']:raise ValueError('strict physical-key evidence requires one explicitly declared diagnostic receipt for '+component)

def require_payload_proof(g):
    for component in ('sexinput','shell'):
        g['wait'](rf'\[input.payload.mode\] component={component} mode=(?:proof|redacted)',0,120)
    assert_payload_proof(g['text']())

def wait_releases(wait,start,count=2):
    """Exclusive QMP chord: actual app ACK plus all key-up admissions, no key values."""
    pattern=r'\[input.keyboard.keyup.ok\][^\n]*source=ps2(?=\s|\n|$)'
    cursor=start
    for _ in range(count):
        match=wait(pattern,cursor)
        cursor+=match.end()
    return cursor

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--spec',type=Path,default=Path('sexos_build_spec.toml'));parser.add_argument('--output',type=Path,required=True);args=parser.parse_args()
    args.output.write_text(diagnostic_spec(args.spec.read_text()))
