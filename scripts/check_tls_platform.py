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
        'rand_core': '0.6.4', 'sha2': '0.10.9', 'p256': '0.13.2',
        'webpki': '0.103.16', 'rustls-pki-types': '1.14.0'}
PACKAGE_NAMES = {'webpki': 'rustls-webpki'}
DIRECT_FEATURES = {'sha2': ['force-soft'], 'p256': ['ecdsa']}
LOCAL = {'sex-trust-config': '../sex-trust-config'}
VENDOR = 'vendor/embedded-tls-0.19.0'
VENDOR_DEP_PATH = '../../' + VENDOR
VENDOR_PROVENANCE_SHA256 = '7f2667fe7ac3587045b8a22b843eb7412afd562f212e4d7c7ba3cd690b50a4c9'
VENDOR_MANIFEST_SHA256 = '716f06505dab9054a1438a69b8848bdf7a390f652eda1c7435fbea87ddba525c'
UPSTREAM_MANIFEST_SHA256 = '2a061a6ea8ef71879f5e4985cb453c1a5f83fc18419f8d4884bb5f4e1f9a6c7a'
UPSTREAM_ARCHIVE_SHA256 = '6ff3c16c9cedcab3ef5c79fd46349152adbb98f997b937cb1ead7655f5c07e43'

EFFECTIVE_FEATURES = {
    'embedded-tls': set(), 'embedded-io': set(), 'embedded-io-async': set(),
    'rand_chacha': set(), 'rand_core': set(), 'sha2': {'force-soft'},
    'rustls-webpki': set(), 'rustls-pki-types': set(), 'sex-trust-config': set(),
    'p256': {'arithmetic', 'digest', 'ecdh', 'ecdsa', 'ecdsa-core', 'sha2', 'sha256'},
}
FORCE = {'aes_force_soft', 'polyval_force_soft'}
BLOCKED = {'libc', 'getrandom', 'rand_os', 'rustix', 'nix', 'ring', 'openssl',
           'openssl-sys', 'native-tls', 'rustls-native-certs', 'security-framework',
           'security-framework-sys', 'schannel', 'winapi', 'wasi', 'wasip2', 'wasip3',
           'r-efi', 'linux-raw-sys', 'redox_syscall', 'relibc'}


def check_manifests(tls, workspace, app, trust=None):
    errors = []
    deps = tls.get('dependencies', {})
    if set(deps) != set(PINS) | set(LOCAL):
        errors.append('TLS direct dependency set changed; review backend policy')
    for name, version in PINS.items():
        dep = deps.get(name)
        if not isinstance(dep, dict):
            errors.append('missing explicit TLS dependency policy: ' + name)
            continue
        expected = {'version': '=' + version, 'default-features': False}
        if name in DIRECT_FEATURES:
            expected['features'] = DIRECT_FEATURES[name]
        if name in PACKAGE_NAMES:
            expected['package'] = PACKAGE_NAMES[name]
        if name == 'embedded-tls':
            expected['path'] = VENDOR_DEP_PATH
        if dep != expected:
            errors.append('noncanonical TLS dependency policy: ' + name)
    for name, path in LOCAL.items():
        if deps.get(name) != {'path': path}:
            errors.append('noncanonical local TLS policy dependency: ' + name)
    for member in ['crates/sex-tls', 'crates/sex-trust-config']:
        if member not in workspace.get('workspace', {}).get('members', []):
            errors.append('TLS policy crate missing from workspace: ' + member)
    if trust is not None:
        if (trust.get('package', {}).get('name') != 'sex-trust-config'
                or trust.get('package', {}).get('version') != '0.1.0'
                or any(trust.get(key) for key in ['dependencies', 'build-dependencies',
                                                'target', 'features', 'patch', 'replace'])):
            errors.append('immutable trust config must have no provider dependencies or features')
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


def check_vendor(root):
    """Exact reviewed published inputs plus tiny recorded patches, no broad fork."""
    root = Path(root).resolve()
    base = root / VENDOR
    errors, inputs = [], {}
    if base.is_symlink() or base.resolve() != base:
        return ['TLS vendor path must be a canonical repository directory'], inputs
    provenance_path = base / 'UPSTREAM.toml'
    raw = provenance_path.read_bytes()
    inputs[VENDOR + '/UPSTREAM.toml'] = hashlib.sha256(raw).hexdigest()
    if inputs[VENDOR + '/UPSTREAM.toml'] != VENDOR_PROVENANCE_SHA256:
        return ['TLS vendor provenance changed; explicit source review required'], inputs
    provenance = tomllib.loads(raw.decode())
    if (provenance.get('archive_sha256') != UPSTREAM_ARCHIVE_SHA256
            or provenance.get('name') != 'embedded-tls' or provenance.get('version') != '0.19.0'
            or provenance.get('original_members') != 96 or provenance.get('schema') != 1):
        errors.append('noncanonical TLS upstream provenance')
    expected = {'UPSTREAM.toml'}
    originals = 0
    for record in provenance.get('files', []):
        name = record.get('path', '')
        relative = Path(name)
        if not name or relative.is_absolute() or '..' in relative.parts or name in expected:
            errors.append('noncanonical or duplicate TLS vendor source path')
            continue
        expected.add(name)
        path = base / relative
        if path.is_symlink() or path.resolve() != path:
            errors.append('TLS vendor source symlink is not reviewed: ' + name)
            continue
        payload = path.read_bytes()
        digest = hashlib.sha256(payload).hexdigest()
        inputs[VENDOR + '/' + name] = digest
        if digest != record.get('sha256') or len(payload) != record.get('bytes'):
            errors.append('TLS vendor source differs from reviewed patch: ' + name)
        originals += 'upstream_sha256' in record
    actual = {p.relative_to(base).as_posix() for p in base.rglob('*')
              if p.is_file() or p.is_symlink()}
    if actual != expected or originals != 96:
        errors.append('TLS vendor source closure changed; no extra or missing files allowed')
    for name, digest in [('Cargo.toml', VENDOR_MANIFEST_SHA256),
                         ('Cargo.toml.upstream', UPSTREAM_MANIFEST_SHA256)]:
        if inputs.get(VENDOR + '/' + name) != digest:
            errors.append('TLS vendor manifest changed: ' + name)
    effective = tomllib.loads((base / 'Cargo.toml').read_text())
    original = tomllib.loads((base / 'Cargo.toml.upstream').read_text())
    original.pop('test', None)
    original.pop('dev-dependencies', None)
    if effective != original:
        errors.append('TLS vendor normalization altered runtime dependencies or features')
    return errors, inputs


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


def check_tree(output, *, root=None):
    errors, packages = [], {}
    for line in output.splitlines():
        match = re.fullmatch(r'([A-Za-z0-9_-]+) v([^ ]+)(?: \(([^\n]*?)\))?(?: (.*))?', line.strip())
        if not match:
            errors.append('malformed effective Cargo graph line')
            continue
        name, version, location, raw = match.groups()
        if name == 'embedded-tls' and root is not None:
            if location != str((Path(root).resolve() / VENDOR).resolve()):
                errors.append('effective TLS graph must use the exact reviewed vendor path')
        features = set((raw or '').removesuffix('(*)').strip().split(',')) - {''}
        entry = packages.setdefault(name, {'versions': set(), 'features': set()})
        entry['versions'].add(version)
        entry['features'].update(features)
        if name in BLOCKED or name.startswith(('windows-', 'aws-lc-', 'openssl-')):
            errors.append('OS or unreviewed crypto provider in effective graph: ' + name)
        if features & {'std', 'alloc', 'getrandom', 'os_rng', 'system-verification', 'native-certs'}:
            errors.append('OS/provider feature in effective graph: ' + name)
    for alias, version in dict(PINS, **{'sex-trust-config': '0.1.0'}).items():
        name = PACKAGE_NAMES.get(alias, alias)
        if packages.get(name, {}).get('versions') != {version}:
            errors.append('effective dependency version mismatch: ' + name)
    if 'sex-tls' not in packages:
        errors.append('missing TLS graph root')
    for name, expected in EFFECTIVE_FEATURES.items():
        if packages.get(name, {}).get('features') != expected:
            errors.append('unexpected effective provider features: ' + name)
    summary = {name: {'versions': sorted(p['versions']), 'features': sorted(p['features'])}
               for name, p in sorted(packages.items())}
    return errors, summary


def inspect(root, *, tree_output=None):
    root = Path(root).resolve()
    report = {'ok': False, 'scope': SCOPE}
    errors = []
    try:
        paths = ['crates/sex-tls/Cargo.toml', 'Cargo.toml', 'apps/kaleidoscope/Cargo.toml',
                 'sexos_build_spec.toml', 'Cargo.lock', 'x86_64-sex.json',
                 'crates/sex-trust-config/Cargo.toml', 'crates/sex-trust-config/src/lib.rs']
        raw = {p: (root / p).read_bytes() for p in paths}
        report['inputs'] = {p: hashlib.sha256(b).hexdigest() for p, b in raw.items()}
        parsed = [tomllib.loads(raw[p].decode()) for p in paths[:4]]
        trust = tomllib.loads(raw['crates/sex-trust-config/Cargo.toml'].decode())
        errors.extend(check_manifests(*parsed[:3], trust=trust))
        errors.extend(check_flags(parsed[3]))
        vendor_errors, vendor_inputs = check_vendor(root)
        errors.extend(vendor_errors)
        report['inputs'].update(vendor_inputs)
        if not errors:
            command = ['cargo', 'tree', '--locked', '-p', 'sex-tls', '--target',
                       str(root / 'x86_64-sex.json'), '-e', 'normal,build',
                       '--prefix', 'none', '--format', '{p} {f}']
            if tree_output is None:
                env = dict(os.environ)
                env.pop('RUSTUP_TOOLCHAIN', None)
                tree_output = subprocess.run(command, cwd=root, env=env, check=True,
                                             capture_output=True, text=True, timeout=60).stdout
            tree_errors, packages = check_tree(tree_output, root=root)
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
