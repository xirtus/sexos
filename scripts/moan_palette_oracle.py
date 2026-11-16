#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Build exact independent full-AA palette label patches using pinned local Weft."""
import argparse, hashlib, json, shutil, subprocess
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--toolchain', default='stable')
    args = parser.parse_args()
    out = args.out.resolve(); out.mkdir(parents=True, exist_ok=False)
    inputs = {}
    for directory in ('crates/weft', 'crates/sex-font', 'assets/fonts'):
        for path in (ROOT / directory).rglob('*'):
            if path.is_file() and '__pycache__' not in path.parts and 'target' not in path.relative_to(ROOT / directory).parts:
                inputs[str(path)] = sha(path)
    source = ROOT / 'scripts/moan_palette_label_oracle.rs'
    inputs[str(source)] = sha(source)
    (out / 'src').mkdir(); shutil.copyfile(source, out / 'src/main.rs')
    (out / 'Cargo.toml').write_text('[package]\nname="moan-palette-label-oracle"\nversion="0.1.0"\nedition="2021"\n[workspace]\n[dependencies]\nweft={path=' + json.dumps(str(ROOT / 'crates/weft')) + '}\n')
    command = ['cargo', '+' + args.toolchain, 'run', '--offline', '--manifest-path', str(out / 'Cargo.toml')]
    with (out / 'expected.jsonl').open('w') as output, (out / 'build.log').open('w') as log:
        subprocess.run(command, stdout=output, stderr=log, check=True)
    assert all(sha(Path(path)) == digest for path, digest in inputs.items()), 'font/source changed during oracle generation'
    receipt = dict(scope=__doc__, command=command, inputs=inputs,
                   expected_sha256=sha(out / 'expected.jsonl'),
                   manifest_sha256=sha(out / 'Cargo.toml'), lock_sha256=sha(out / 'Cargo.lock'),
                   source_sha256=sha(out / 'src/main.rs'),
                   rustc=subprocess.check_output(['rustc', '+' + args.toolchain, '--version'], text=True).strip())
    (out / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    print(json.dumps(dict(ok=True, patches=8, expected_sha256=receipt['expected_sha256'])))
if __name__ == '__main__': main()
