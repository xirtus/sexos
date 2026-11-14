#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
"""Freezes the pure-Python standard library that apps/python carries inside the binary.

Each module of MODULES is compiled from a CPython checkout's Lib/ with this script's
own interpreter and written as marshalled code to apps/python/frozen/NAME.bin, the
same bytes CPython's Programs/_freeze_module.py would put in Python/frozen_modules/.
apps/python/build.rs turns the folder into CPython's frozen-module tables.

It also freezes POLYAMOUR, Polyamour's scripting API (apps/polyamour/python: the
`polyamour` package and its `bpy` and `mathutils` aliases), so every SexOS Python
script can `import bpy` without anything installed in its folder.

Bytecode belongs to one CPython release series, so this must run under the same
3.13 series as vendor/cpython-3.13.18 (an installed CPython 3.13's own Lib folder
works as well as a checkout: `mkdir /tmp/cp && ln -s /usr/lib/python3.13 /tmp/cp/Lib`):

    git clone --depth 1 --branch v3.13.18 https://github.com/python/cpython.git /tmp/cpython
    python3.13 scripts/python_freeze.py /tmp/cpython
"""
import marshal,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'apps/python/frozen'
VERSION=(3,13)

# importlib's bootstrap (required by the interpreter itself), what startup imports
# (encodings, site, os, io...), the traceback printer, and a working everyday set:
# json, re, collections, functools, random, datetime, decimal, typing, argparse...
MODULES='''
importlib._bootstrap importlib._bootstrap_external zipimport
abc codecs io _collections_abc _sitebuiltins genericpath ntpath posixpath os site stat
importlib importlib.util importlib.machinery importlib._abc runpy
encodings encodings.aliases encodings.utf_8 encodings.utf_8_sig encodings.ascii encodings.latin_1
__future__ _colorize _opcode_metadata _weakrefset _compat_pickle _pydecimal
argparse ast base64 bisect calendar collections contextlib copy copyreg
dataclasses datetime decimal difflib dis enum fnmatch fractions functools gettext glob
heapq inspect json json.decoder json.encoder json.scanner keyword linecache locale numbers
opcode operator pprint random re re._casefix re._compiler re._constants re._parser
reprlib shlex statistics string struct textwrap token tokenize traceback types typing
warnings weakref this
'''.split()

# Polyamour's scripting API, compiled from apps/polyamour/python rather than Lib/.
POLYAMOUR_DIR=ROOT/'apps/polyamour/python'
POLYAMOUR='polyamour polyamour.polyops polyamour.types bpy mathutils'.split()

def main():
    if sys.version_info[:2]!=VERSION:raise SystemExit(f'run this with Python {VERSION[0]}.{VERSION[1]}')
    lib=Path(sys.argv[1])/'Lib'
    for old in OUT.glob('*.bin'):old.unlink()
    manifest=[]
    for name in MODULES+POLYAMOUR:
        path=(POLYAMOUR_DIR if name in POLYAMOUR else lib).joinpath(*name.split('.'))
        package=(path/'__init__.py').exists()
        source=path/'__init__.py' if package else path.with_suffix('.py')
        code=compile(source.read_bytes(),f'<frozen {name}>','exec',optimize=0,dont_inherit=True)
        (OUT/f'{name}.bin').write_bytes(marshal.dumps(code))
        manifest.append(f'{name} {"package" if package else "module"}')
    (OUT/'MANIFEST').write_text('# name kind: written by scripts/python_freeze.py from CPython %s\n'%sys.version.split()[0]+'\n'.join(manifest)+'\n')
    print(len(manifest),'modules',sum(f.stat().st_size for f in OUT.glob('*.bin')),'bytes')

if __name__=='__main__':main()
