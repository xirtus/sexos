#!/usr/bin/env python3
"""Active target metadata/compiler cfg evidence, not OS authority or enforcement."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tomllib

SCOPE = ('Active x86_64-sex.json and pinned compiler default cfg only; legacy '
         'sex-src/targets is historical. No capability, containment, SMP, runtime '
         'or whole-workspace proof; downstream codegen overrides require separate checks.')


def check_source(spec):
    errors = []
    expected = {'llvm-target': 'x86_64-unknown-none', 'arch': 'x86_64',
                'target-endian': 'little', 'target-pointer-width': 64,
                'target-c-int-width': 32, 'os': 'none', 'env': '',
                'target-family': [], 'rustc-abi': 'softfloat',
                'disable-redzone': True, 'panic-strategy': 'abort',
                'relocation-model': 'static', 'has-rpath': False,
                'executables': True, 'linker-flavor': 'ld.lld',
                'linker': 'rust-lld', 'code-model': 'kernel'}
    if not isinstance(spec, dict):
        return ['target JSON must be an object']
    for key, value in expected.items():
        if type(spec.get(key)) is not type(value) or spec.get(key) != value:
            errors.append('noncanonical target field: ' + key)
    features = spec.get('features')
    if not isinstance(features, str):
        errors.append('missing target feature policy')
    else:
        tokens = features.split(',')
        if len(tokens) != len(set(tokens)) or any(not re.fullmatch(r'[+-][a-zA-Z0-9_.-]+', t) for t in tokens):
            errors.append('malformed or duplicate target features')
        if not {'-mmx', '-sse', '-sse2', '+soft-float'}.issubset(tokens):
            errors.append('missing scalar softfloat baseline')
        if any(t.startswith(('+sse', '+avx')) for t in tokens):
            errors.append('enabled SSE/AVX target feature')
    if not isinstance(spec.get('data-layout'), str) or 'n8:16:32:64' not in spec['data-layout']:
        errors.append('missing 64-bit integer data layout')
    return errors


def check_cfg(text):
    errors, flags, values = [], set(), {}
    for line in text.splitlines():
        if re.fullmatch(r'[a-zA-Z_][a-zA-Z0-9_]*', line):
            flags.add(line)
            continue
        match = re.fullmatch(r'([a-zA-Z_][a-zA-Z0-9_]*)=("(?:[^"\\]|\\.)*")', line)
        if not match:
            errors.append('malformed compiler cfg line')
            continue
        try:
            value = json.loads(match[2])
        except ValueError:
            errors.append('invalid compiler cfg value')
            continue
        values.setdefault(match[1], []).append(value)
    for key, expected in [('target_os', 'none'), ('target_env', ''),
                          ('target_arch', 'x86_64'), ('target_pointer_width', '64'),
                          ('target_endian', 'little'), ('panic', 'abort')]:
        if values.get(key) != [expected]:
            errors.append('compiler cfg mismatch: ' + key)
    if 'unix' in flags or 'windows' in flags or values.get('target_family'):
        errors.append('compiler exposes an OS target family')
    features = values.get('target_feature', [])
    if any(feature.startswith(('sse', 'avx')) for feature in features):
        errors.append('compiler enables SSE/AVX')
    return errors, {'os': values.get('target_os'), 'env': values.get('target_env'),
                    'arch': values.get('target_arch'), 'pointer_width': values.get('target_pointer_width'),
                    'families': values.get('target_family', []), 'features': sorted(features),
                    'unix': 'unix' in flags, 'sha256': hashlib.sha256(text.encode()).hexdigest()}


def _unique(pairs):
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError('duplicate target JSON key: ' + key)
        obj[key] = value
    return obj


def inspect(root, *, cfg_output=None, compiler_output=None):
    """Optional outputs permit independent adverse cfg tests without root edits."""
    root = Path(root).resolve()
    target = root / 'x86_64-sex.json'
    errors, report = [], {'ok': False, 'scope': SCOPE, 'target': str(target)}
    try:
        raw = target.read_bytes()
        report['target_sha256'] = hashlib.sha256(raw).hexdigest()
        errors.extend(check_source(json.loads(raw, object_pairs_hook=_unique)))
        channel = tomllib.loads((root / 'rust-toolchain.toml').read_text())['toolchain']['channel']
        if not isinstance(channel, str) or not channel:
            raise ValueError('missing pinned toolchain channel')
        report['toolchain'] = channel
        if not errors:
            env = dict(os.environ)
            env.pop('RUSTUP_TOOLCHAIN', None)  # cwd selects the repository pin.
            command = ['rustc', '-Z', 'unstable-options', '--print', 'cfg', '--target', str(target)]
            if cfg_output is None:
                cfg_output = subprocess.run(command, cwd=root, env=env, check=True,
                                            capture_output=True, text=True, timeout=60).stdout
            if compiler_output is None:
                compiler_output = subprocess.run(['rustc', '--version', '--verbose'], cwd=root, env=env,
                                                 check=True, capture_output=True, text=True, timeout=60).stdout
            if not compiler_output.strip():
                errors.append('missing compiler identity')
            cfg_errors, cfg = check_cfg(cfg_output)
            errors.extend(cfg_errors)
            report.update(command=command, compiler=compiler_output.strip(), cfg=cfg)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as exc:
        errors.append(type(exc).__name__ + ': ' + str(exc)[:500])
    report.update(ok=not errors, errors=errors)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    report = inspect(args.root)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
