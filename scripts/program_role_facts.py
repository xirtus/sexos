"""Ordinary program candidates from complete sealed BOOT/catalog observations.

A source constraint is observed, never inferred from a filename or bypassed. These
facts select test subjects; they grant no runtime ability to alter a BOOT grant.
"""
import re

FIELDS = {'logical','incarnation','predecessor','successor','task','surface','key',
          'dormant','leaves','runtime_mint','source_pinned'}
SEAL = '[boot.memory.authority.sealed] runtime_alloc=0 runtime_map=capability_leaf runtime_mint=0'


def program_rows(root, serial):
    catalog = {}
    by_ordinal = {}
    source = (root/'crates/sex-pdx/src/app_catalog.rs').read_text()
    rows = re.findall(r'app:\s*App::(\w+),\s*ordinal:\s*(\d+),\s*target:\s*(\d+),\s*surface:\s*(\d+)', source)
    # Quil uses the catalog's fixed constructor; its first three arguments are
    # ordinal, target and surface. Do not silently omit those actual subjects.
    rows += [('Quil', *match) for match in re.findall(r'^\s*quil\(\s*(\d+),\s*(\d+),\s*(\d+),', source, re.M)]
    declared = re.search(r'pub const INCARNATIONS:\s*\[Incarnation;\s*(\d+)\]', source)
    if declared: assert len(rows)==int(declared[1]), 'unparsed catalog row or duplicate constructor'
    for app, ordinal, target, surface in rows:
        row = dict(app=app, ordinal=int(ordinal), target=int(target), surface=int(surface))
        assert row['target'] not in catalog and (app,row['ordinal']) not in by_ordinal, 'duplicate source catalog subject'
        catalog[row['target']] = row
        by_ordinal[app,row['ordinal']] = row
    assert catalog and any(row['app']=='Program' for row in catalog.values()), 'missing program catalog'
    complete = serial[:serial.rfind('\n')+1]
    seals = list(re.finditer(r'^'+re.escape(SEAL)+r'\r?\n', complete, re.M))
    assert len(seals)==1, 'missing/ambiguous complete BOOT seal'
    # User tasks run only after this seal. Never let a later serial line change
    # the observed source constraint of a BOOT activation grant.
    prefix = complete[:seals[0].start()]
    observed = {}
    for line in prefix.splitlines():
        if not line.startswith('[boot.app.successor]'):continue
        assert line.startswith('[boot.app.successor] ') and len(line)>len('[boot.app.successor] '), ('malformed successor record',line)
        payload=line[len('[boot.app.successor] '):]
        fields = {}
        for token in payload.split():
            match = re.fullmatch(r'([a-z_]+)=(0|[1-9][0-9]*)', token)
            assert match and match[1] not in fields, ('malformed successor field', token)
            fields[match[1]] = int(match[2])
        assert fields.keys()==FIELDS, ('successor field set changed',fields)
        target = fields['successor']
        assert target in catalog and catalog[target]['ordinal']>0, ('unknown successor',target)
        assert target not in observed, ('duplicate successor',target)
        row = catalog[target]
        previous = by_ordinal.get((row['app'],row['ordinal']-1))
        first = by_ordinal.get((row['app'],0))
        assert previous and first, ('broken catalog chain',row)
        assert fields['logical']==first['target'] and fields['incarnation']==row['ordinal']+1
        assert fields['predecessor']==previous['target'] and fields['task']==target
        assert fields['surface']==row['surface'] and 0<fields['key']<16
        assert fields['dormant']==1 and fields['leaves']>0 and fields['runtime_mint']==0
        assert fields['source_pinned'] in (0,1), ('invalid source constraint flag',fields)
        observed[target] = fields
    expected = {row['target'] for row in catalog.values() if row['ordinal']>0}
    assert observed.keys()==expected, ('missing BOOT successor facts',sorted(expected-observed.keys()))
    return [dict(row, key=observed[row['target']]['key'], source_pinned=bool(observed[row['target']]['source_pinned']))
            for row in sorted(catalog.values(),key=lambda row:row['ordinal'])
            if row['app']=='Program' and row['ordinal']>0]


def ordinary_rows(root, serial):
    rows = [row for row in program_rows(root,serial) if not row['source_pinned']]
    assert rows, 'no ordinary reusable program row'
    return rows


def assert_ordinary_launch(root, serial, target, task, surface):
    rows = {row['target']:row for row in ordinary_rows(root,serial)}
    assert target in rows, ('ordinary launch used source-constrained/unknown row',target)
    assert 0<task<=0xffff_ffff and surface==rows[target]['surface'], ('launch identity mismatch',target,task,surface)
    return rows[target]


def assert_private_ordinary_launch(root, serial, target, task, surface):
    row = assert_ordinary_launch(root, serial, target, task, surface)
    # A private key appears once among ALL reusable rows, including constrained
    # ones. Removing a constrained row must not make a shared key look private.
    assert sum(other['key']==row['key'] for other in program_rows(root,serial))==1, ('JIT subject shares a program key',row)
    return row


def check_ordinary_launches(root, serial, start=0):
    complete = serial[:serial.rfind('\n')+1]
    for line in complete[start:].splitlines():
        if not line.startswith('[shell.program.launch]'):continue
        match = re.fullmatch(r'\[shell.program.launch\] target=(0|[1-9][0-9]*) task=(0|[1-9][0-9]*) surface=(0|[1-9][0-9]*)',line)
        assert match, ('malformed ordinary launch receipt',line)
        assert_ordinary_launch(root,complete,*map(int,match.groups()))
    return len(complete)
