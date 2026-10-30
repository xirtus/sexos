#!/usr/bin/env python3
"""Join actual snapshot publication, private validation and terminal scrub."""
import argparse
import json
import re
from pathlib import Path
from check_document_views import inspect as inspect_boot


def inspect(text, required=(9,), minimum=1, require_cleanup=True):
    errors = list(inspect_boot(text)['errors'])
    pattern = r'\[(files|quil)\.document\.snapshot\] caller=(\d+) object=(\d+) identity=(\d+) sequence=(\d+) bytes=(\d+) (?:published|validated)=1'
    publications = {}
    validations = []
    for match in re.finditer(pattern, text):
        side, *raw = match.groups()
        pd, obj, identity, seq, length = map(int, raw)
        key = (pd, seq)
        payload = (obj, identity, length)
        if pd not in (9, 16, 18) or not 2 <= obj < 15 or not 0 < seq <= 0xffffff or length > 12288:
            errors.append('invalid actual snapshot identity/length')
        if side == 'files':
            if key in publications:
                errors.append('duplicate snapshot publication for accepted request')
            publications[key] = (payload, match.start())
        else:
            if key not in publications or publications[key][0] != payload or publications[key][1] >= match.start():
                errors.append('private validation lacks exact prior publication')
            if key in validations:
                errors.append('duplicate private validation')
            validations.append(key)
    if len(validations) < minimum or not set(required).issubset({pd for pd, _ in validations}):
        errors.append('required actual client snapshot validation missing')
    clears = {}
    for match in re.finditer(r'\[files\.document\.snapshot\.clear\] caller=(9|16|18) bytes=16384 scrubbed=1', text):
        clears.setdefault(int(match[1]), []).append(match.start())
    for (pd, _), (_, position) in publications.items():
        if not any(p > position for p in clears.get(pd, [])):
            errors.append('published backing never completely scrubbed')
    cleanups = 0
    for match in re.finditer(r'\[files\.app\.cleanup\] target=(9|16|18) task=\d+ ', text):
        pd = int(match[1])
        previous = max((pos for (owner, _), (_, pos) in publications.items()
                        if owner == pd and pos < match.start()), default=-1)
        if not any(previous < p < match.start() for p in clears.get(pd, [])):
            errors.append('terminal cleanup preceded complete snapshot scrub')
        cleanups += 1
    if require_cleanup and not cleanups:
        errors.append('no actual terminal document cleanup')
    return dict(ok=not errors, errors=errors, publications=len(publications), validations=len(validations),
                validated_clients=sorted({pd for pd, _ in validations}), terminal_cleanups=cleanups,
                terminal_cleanup_required=require_cleanup,
                scope='Actual immutable BOOT views, exact publication/private-copy joins and complete scrub before cleanup. Bytes/pixels/reboot and software/hostile enforcement require their separate gates.')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('log', type=Path)
    parser.add_argument('--require-clients', default='9')
    parser.add_argument('--minimum', type=int, default=1)
    parser.add_argument('--no-terminal', action='store_true')
    args = parser.parse_args()
    result = inspect(args.log.read_text(errors='replace'), tuple(int(p) for p in args.require_clients.split(',') if p), args.minimum, not args.no_terminal)
    print(json.dumps(result, indent=2))
    return not result['ok']


if __name__ == '__main__':
    raise SystemExit(main())
