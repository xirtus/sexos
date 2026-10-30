#!/usr/bin/env python3
"""Check pinned TLS dependency selection and the actual bare-metal Cargo graph."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import tomllib

SCOPE = ('Dependency metadata, effective bare-metal Cargo graph and declared scalar '
         'backend flags only. No instruction-level crypto, certificate trust, entropy '
         'quality, runtime enforcement or authenticated-connection proof.')
PINS = {'embedded-tls': '0.19.0', 'embedded-io': '0.7.1',
        'embedded-io-async': '0.7.0', 'rand_chacha': '0.3.1',
        'rand_core': '0.6.4', 'sha2': '0.10.9'}
FORCE = {'aes_force_soft', 'polyval_force_soft'}
BLOCKED = {'libc', 'getrandom', 'rand_os', 'rustix', 'nix', 'ring', 'openssl',
           'openssl-sys', 'native-tls', 'rustls-native-certs', 'security-framework',
           'security-framework-sys', 'schannel', 'winapi', 'wasi', 'wasip2', 'wasip3',
           'r-efi', 'linux-raw-sys', 'redox_syscall', 'relibc'}


def check_manifests(tls, workspace, app):
    errors = []
    deps = tls.get('dependencies', {})
    if set(deps) != set(PINS):
        errors.append('TLS direct dependency set changed; review backend policy')
    for name, version in PINS.items():
        dep = deps.get(name)
        if not isinstance(dep, dict):
            errors.append('missing explicit TLS dependency policy: ' + name)
            continue
        expected_features = ['force-soft'] if name == 'sha2' else []
        if (dep.get('version') != '=' + version or dep.get('default-features') is not False
                or dep.get('features', []) != expected_features
                or set(dep) - {'version', 'default-features', 'features'}):
            errors.append('noncanonical TLS dependency policy: ' + name)
    if 'crates/sex-tls' not in workspace.get('workspace', {}).get('members', []):
        errors.append('TLS crate missing from workspace')
    if app.get('dependencies', {}).get('sex-tls') != {'path': '../../crates/sex-tls'}:
        errors.append('Kaleidoscope must consume the reviewed workspace TLS crate')
    # Target-specific additions and workspace replacements would evade the
    # direct manifest policy; a future provider change needs an explicit review.
    if tls.get('target'):
        errors.append('unreviewed target-specific TLS dependencies')
    for key in ['patch', 'replace']:
        if workspace.get(key):
            errors.append('workspace dependency replacement needs TLS policy review: ' + key)
    return errors


def check_flags(spec):
    errors = []
    stages = [s for s in spec.get('stage', []) if s.get('id') == 'build_kaleidoscope']
    if len(stages) != 1:
        return ['missing or duplicate Kaleidoscope build stage']
    stage = stages[0]
    if stage.get('action') != 'cargo_manifest' or stage.get('manifest') != 'apps/kaleidoscope/Cargo.toml':
        errors.append('unexpected Kaleidoscope build action or manifest')
    try:
        tokens = shlex.split(stage.get('rustflags', ''))
    except (ValueError, TypeError):
        return errors + ['malformed Kaleidoscope Rust flags']
    cfg, codegen = [], []
    i = 0
    while i < len(tokens):
        token = tokens[i]
        if token in ('--cfg', '-C', '--codegen'):
            if i + 1 == len(tokens):
                errors.append('missing Rust flag argument')
                break
            i += 1
            (cfg if token == '--cfg' else codegen).append(tokens[i])
        elif token.startswith('--cfg='):
            cfg.append(token[len('--cfg='):])
        elif token.startswith('-C'):
            codegen.append(token[2:])
        elif token.startswith('--codegen='):
            codegen.append(token[len('--codegen='):])
        i += 1
    for name in FORCE:
        if cfg.count(name) != 1:
            errors.append('missing or duplicate scalar crypto cfg: ' + name)
    for setting in codegen:
        if setting.startswith('target-cpu=') and setting != 'target-cpu=generic':
            errors.append('CPU override may enable an unpreserved vector backend')
        if setting.startswith('target-feature='):
            features = setting[len('target-feature='):].split(',')
            if any(f.startswith(('+sse', '+avx', '+aes', '+pclmul', '+sha', '+mmx', '+fma')) for f in features):
                errors.append('enabled vector/crypto instruction override')
    return errors


def check_tree(output):
    errors, packages = [], {}
    for line in output.splitlines():
        match = re.fullmatch(r'([A-Za-z0-9_-]+) v([^ ]+)(?: \([^\n]+\))?(?: (.*))?', line.strip())
        if not match:
            errors.append('malformed effective Cargo graph line')
            continue
        name, version, raw = match.groups()
        features = set((raw or '').removesuffix('(*)').strip().split(',')) - {''}
        entry = packages.setdefault(name, {'versions': set(), 'features': set()})
        entry['versions'].add(version)
        entry['features'].update(features)
        if name in BLOCKED or name.startswith(('windows-', 'aws-lc-', 'openssl-')):
            errors.append('OS or unreviewed crypto provider in effective graph: ' + name)
        if features & {'std', 'getrandom', 'os_rng', 'system-verification', 'native-certs'}:
            errors.append('OS/provider feature in effective graph: ' + name)
    for name, version in PINS.items():
        if packages.get(name, {}).get('versions') != {version}:
            errors.append('effective dependency version mismatch: ' + name)
    if 'sex-tls' not in packages:
        errors.append('missing TLS graph root')
    for name in ['embedded-tls', 'rand_chacha', 'rand_core']:
        if packages.get(name, {}).get('features'):
            errors.append('unexpected effective provider features: ' + name)
    if packages.get('sha2', {}).get('features') != {'force-soft'}:
        errors.append('effective SHA2 backend must be force-soft only')
    summary = {name: {'versions': sorted(p['versions']), 'features': sorted(p['features'])}
               for name, p in sorted(packages.items())}
    return errors, summary


def inspect(root, *, tree_output=None):
    root = Path(root).resolve()
    report = {'ok': False, 'scope': SCOPE}
    errors = []
    try:
        paths = ['crates/sex-tls/Cargo.toml', 'Cargo.toml', 'apps/kaleidoscope/Cargo.toml',
                 'sexos_build_spec.toml', 'Cargo.lock', 'x86_64-sex.json']
        raw = {p: (root / p).read_bytes() for p in paths}
        report['inputs'] = {p: hashlib.sha256(b).hexdigest() for p, b in raw.items()}
        parsed = [tomllib.loads(raw[p].decode()) for p in paths[:4]]
        errors.extend(check_manifests(*parsed[:3]))
        errors.extend(check_flags(parsed[3]))
        if not errors:
            command = ['cargo', 'tree', '--locked', '-p', 'sex-tls', '--target',
                       str(root / 'x86_64-sex.json'), '-e', 'normal,build',
                       '--prefix', 'none', '--format', '{p} {f}']
            if tree_output is None:
                env = dict(os.environ)
                env.pop('RUSTUP_TOOLCHAIN', None)
                tree_output = subprocess.run(command, cwd=root, env=env, check=True,
                                             capture_output=True, text=True, timeout=60).stdout
            tree_errors, packages = check_tree(tree_output)
            errors.extend(tree_errors)
            report.update(command=command, packages=packages,
                          graph_sha256=hashlib.sha256(tree_output.encode()).hexdigest())
    except (OSError, ValueError, TypeError, subprocess.SubprocessError) as exc:
        errors.append(type(exc).__name__ + ': ' + str(exc)[:500])
    report.update(ok=not errors, errors=errors)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    report = inspect(parser.parse_args().root)
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0 if report['ok'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
