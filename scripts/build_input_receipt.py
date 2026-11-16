#!/usr/bin/env python3
"""Bind declared generated build inputs to their consuming staged artifact.

This records the sealed host build, not an attestation against a hostile compiler
or host. Inputs must be stable across the consuming stage and at final freeze.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tomllib

RECEIPTS = Path('target/build-consumed-inputs')
MAX_BYTES = 64 * 1024 * 1024


def file_record(root, name):
    if not isinstance(name, str) or not name or Path(name).is_absolute() or Path(name).as_posix() != name or '..' in Path(name).parts:
        raise ValueError('invalid consumed artifact path')
    path = (root / name).resolve(strict=True)
    if not path.is_relative_to(root) or not path.is_file() or not 0 < path.stat().st_size <= MAX_BYTES:
        raise ValueError('invalid consumed artifact: ' + name)
    data = path.read_bytes()
    if not 0 < len(data) <= MAX_BYTES:
        raise ValueError('consumed artifact exceeds bound: ' + name)
    return dict(bytes=len(data), sha256=hashlib.sha256(data).hexdigest())


def declaration(stage):
    names = stage.get('consumed_inputs', [])
    if not isinstance(names, list) or len(names) > 16 or any(not isinstance(n, str) for n in names) or len(set(names)) != len(names):
        raise ValueError('invalid consumed_inputs declaration')
    if not names:
        return None
    sid = stage.get('id')
    if not isinstance(sid, str) or not re.fullmatch('[a-z][a-z0-9_]{0,95}', sid) or stage.get('action') != 'cargo_manifest':
        raise ValueError('consumed inputs require a named cargo_manifest stage')
    return dict(stage=sid, manifest=stage['manifest'], output=stage['dest_artifact'], names=names)


def begin(root, stage):
    declared = declaration(stage)
    if declared is None:
        return
    directory = root / RECEIPTS
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / (declared['stage'] + '.json')
    if path.exists():
        raise ValueError('consumed input receipt already exists')
    record = dict(stage=declared['stage'], manifest=declared['manifest'], output=declared['output'],
                  inputs={name: file_record(root, name) for name in declared['names']}, complete=False)
    path.write_text(json.dumps(record, sort_keys=True, indent=2) + '\n')


def finish(root, stage):
    declared = declaration(stage)
    if declared is None:
        return
    path = root / RECEIPTS / (declared['stage'] + '.json')
    record = json.loads(path.read_text())
    expected = dict(stage=declared['stage'], manifest=declared['manifest'], output=declared['output'],
                    inputs={name: file_record(root, name) for name in declared['names']}, complete=False)
    if record != expected:
        raise ValueError('consumed input changed during build stage')
    record.update(complete=True, artifact=file_record(root, declared['output']))
    path.write_text(json.dumps(record, sort_keys=True, indent=2) + '\n')


def collect(root, spec):
    result = {}
    for stage in spec.get('stage', []):
        declared = declaration(stage)
        if declared is None:
            continue
        sid = declared['stage']
        if sid in result:
            raise ValueError('duplicate consuming stage')
        record = json.loads((root / RECEIPTS / (sid + '.json')).read_text())
        expected = dict(stage=sid, manifest=declared['manifest'], output=declared['output'],
                        inputs={name: file_record(root, name) for name in declared['names']},
                        complete=True, artifact=file_record(root, declared['output']))
        if record != expected:
            raise ValueError('consumed artifact receipt differs from sealed build: ' + sid)
        result[sid] = record
    return result


def validate(root, spec, records):
    """Verify archived receipts without depending on mutable target receipts."""
    if not isinstance(records, dict):
        raise ValueError('consumed artifact records must be a map')
    expected = {}
    for stage in spec.get('stage', []):
        declared = declaration(stage)
        if declared is None:
            continue
        sid = declared['stage']
        if sid in expected:
            raise ValueError('duplicate consuming stage')
        expected[sid] = dict(stage=sid, manifest=declared['manifest'], output=declared['output'],
                             inputs={name: file_record(root, name) for name in declared['names']},
                             complete=True, artifact=file_record(root, declared['output']))
    if records != expected:
        raise ValueError('consumed artifacts differ from declared sealed build')
    return {name: data for record in expected.values()
            for name, data in [*record['inputs'].items(), (record['output'], record['artifact'])]}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('mode', choices=['begin', 'finish', 'collect'])
    parser.add_argument('spec', type=Path)
    parser.add_argument('--out', type=Path)
    args = parser.parse_args()
    root = Path.cwd().resolve()
    spec = tomllib.loads(args.spec.read_text())
    if args.mode == 'collect':
        result = collect(root, spec)
        if args.out:
            args.out.write_text(json.dumps(result, sort_keys=True, indent=2) + '\n')
    else:
        if len(spec['stage']) != 1:
            raise ValueError('one stage required')
        (begin if args.mode == 'begin' else finish)(root, spec['stage'][0])


if __name__ == '__main__':
    main()
