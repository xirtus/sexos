#!/usr/bin/env python3
"""Reject compiled memory primitives calling/tail-jumping to memory intrinsics."""
import re
import subprocess
import sys

binary = sys.argv[1]
objdump = sys.argv[2] if len(sys.argv) > 2 else 'objdump'
text = subprocess.check_output([objdump, '-dr', binary], text=True)
symbols = subprocess.check_output([objdump, '-t', binary], text=True)
addresses = {}
for line in symbols.splitlines():
    match = re.match(r'^([0-9a-fA-F]+)\s+.*\s_?(memcpy|memset)$', line)
    if match: addresses[int(match[1],16)] = match[2]
functions = {}
current = None
for line in text.splitlines():
    match = re.match(r'\s*([0-9a-fA-F]+)\s+<([^>]+)>:', line)
    if match:
        current = match[2].lstrip('_') if match[2].lstrip('_') in ['memcpy','memset'] else addresses.get(int(match[1],16))
        if current: functions[current] = []
    elif current:
        functions[current].append(line)
for name in ['memcpy', 'memset']:
    assert name in functions, f'{binary}: missing {name}'
    lines = functions[name]
    for line in lines:
        # Local branches to interior labels are ordinary byte-loop control flow.
        # A branch to the entry, or relocation to a memory intrinsic, recurses.
        direct = re.search(r'\b(?:callq?|jmpq?|b|bl)\s+(?:0x)?([0-9a-fA-F]+)\s+<[^>]+>', line)
        direct = direct and int(direct[1],16) in addresses
        relocated = re.search(r'(?:RELOC|R_X86_64|BRANCH).*\b_?(?:memcpy|memset)\b', line)
        assert not (direct or relocated), f'{binary}: recursive/intrinsic lowering in {name}: {line.strip()}'
    assert any(re.search(r'\bretq?\b', line) for line in lines), f'{name}: no return instruction'
print('[gate.silk_memory.codegen] PASS optimized_memcpy_memset_no_intrinsic_recursion')
