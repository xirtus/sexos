#!/usr/bin/env python3
"""Cold-open the last saved document after a complete BOOT Quil reserve chain.

Never builds. --check reads immutable evidence and starts no guest.
"""
import argparse
import gzip
import json
from pathlib import Path

import alpha_evidence as alpha
import pending_io_cold_gate as cold
from https_asset_gate import script_dependencies
from quil_reserve_catalog import quil_catalog, verify_boot_quil

FRAME_NAME = 'reserve-cold-recovered'


def contract(root):
    result = alpha.contract(root)
    names = script_dependencies(root, ('scripts/quil_reserve_cold_gate.py',
                                       'scripts/tests/test_quil_reserve_cold_gate.py'))
    result['source_sha256'].update({name: alpha.sha(Path(root) / name) for name in names})
    return result


def terminal_document(seed, manifest, result, rows, session):
    """Join the complete source roster, last save and independently parsed disk."""
    if result.get('ok') is not True or not manifest.get('options', {}).get('third_quil'):
        raise ValueError('accepted complete-reserve workflow required')
    third = (result.get('soak') or {}).get('third_quil') or {}
    expected = [row['target'] for row in rows]
    steps = third.get('chain', [])
    if len(rows) < 3 or third.get('catalog') != rows or [step['target'] for step in steps] != expected[2:]:
        raise ValueError('terminal session skipped or substituted a source incarnation')
    if third.get('final_target') != expected[-1] or third.get('append_count') != len(rows) - 2:
        raise ValueError('terminal session did not finish its entire reserve')
    if cold.digest(seed) != manifest['final_disk_sha256']:
        raise ValueError('terminal reserve disk hash differs')
    last = steps[-1]
    saved = last['saved']
    payload = saved['content'].encode('ascii')
    found = cold.recover(seed, saved['object'])
    if found.pop('payload') != payload or (found['bytes'], found['hash']) != (saved['bytes'], saved['hash']):
        raise ValueError('last incarnation save differs from terminal journal')
    if len(payload) < 8 or len(payload) != saved['bytes'] or hex(cold.fnv(payload)) != saved['hash']:
        raise ValueError('invalid last saved payload')
    chain = dict(session=str(Path(session).resolve()), seed_sha256=cold.digest(seed),
                 targets=expected, final_target=expected[-1], document=saved['object'],
                 recovered=found, bytes=len(payload), hash=hex(cold.fnv(payload)))
    return seed, payload, chain


def accepted_session(root, iso, binding, session):
    alpha.replay(root, iso, binding, session)
    session = Path(session).resolve()
    manifest = json.loads((session / 'manifest.json').read_text())
    result = json.loads((session / 'result.json').read_text())
    seed = gzip.decompress((session / 'nvme.img.gz').read_bytes())
    seed, payload, chain = terminal_document(seed, manifest, result, quil_catalog(root), session)
    chain.update(manifest_sha256=alpha.sha(session / 'manifest.json'),
                 result_sha256=alpha.sha(session / 'result.json'))
    return seed, payload, chain


class ReservePolicy:
    frame_name = FRAME_NAME
    accepted_session = staticmethod(accepted_session)
    contract = staticmethod(contract)

    @staticmethod
    def expected(payload):
        # The exact whole payload is checked against native load and journal;
        # its trailing glyphs must also appear in the completed viewport.
        return payload.decode('ascii')[-8:]


POLICY = ReservePolicy()


def replay(root, iso, binding, session, out):
    result = cold.replay(root, iso, binding, session, out, policy=POLICY)
    verify_boot_quil(alpha.serial_text(Path(out) / 'serial.log'), quil_catalog(root))
    result['scope'] = ('Cold boot of the exact complete-reserve terminal disk; fresh Quil9 '
                       'loads the last incarnation save with exact journal metadata/payload '
                       'and completed viewport/clock pixels. Not power-loss qualification.')
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('iso', 'binding', 'session', 'out'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--resolution', choices=('1280x800', '1024x768'), default='1280x800')
    parser.add_argument('--model', choices=('e1000', 'e1000e'), default='e1000e')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parents[1]
    values = (root, args.iso.resolve(strict=True), args.binding.resolve(strict=True),
              args.session.resolve(strict=True), args.out.resolve())
    if not args.check:
        cold.run(*values, args.resolution, args.model, policy=POLICY)
    result = replay(*values)
    print(json.dumps(result, indent=2))
    return result


if __name__ == '__main__':
    main()
