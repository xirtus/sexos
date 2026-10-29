#!/usr/bin/env python3
"""Inspect the built x86 switch boundary. Never compiles or packages an image."""
import argparse
import hashlib
from pathlib import Path
import re
import shutil
import subprocess
import sys


def switch_instructions(disassembly):
    functions = []
    active = None
    for line in disassembly.splitlines():
        header = re.match(r"^\s*[0-9a-f]+ <(.+)>:\s*$", line)
        if header:
            active = [] if "Scheduler" in header[1] and "switch_to" in header[1] else None
            if active is not None:
                functions.append(active)
            continue
        inst = re.match(r"^\s*[0-9a-f]+:\s+(.+?)\s*$", line)
        if active is not None and inst:
            active.append(re.sub(r"\s+", " ", inst[1].lower()))
    if len(functions) != 1 or not functions[0]:
        raise ValueError("expected exactly one nonempty Scheduler::switch_to disassembly")
    return functions[0]


def validate(instructions):
    compact = [line.replace(" ", "") for line in instructions]

    def one(predicate, label):
        matches = [i for i, line in enumerate(compact) if predicate(line)]
        if len(matches) != 1:
            raise ValueError(f"expected exactly one {label}, found {len(matches)}")
        return matches[0]

    cli = one(lambda x: x == "cli", "CLI")
    capture = one(lambda x: x == "movr12,rsi", "callee-saved next-context capture")
    callback = one(lambda x: x.startswith("call") and "<scheduler_finish_switch>" in x,
                   "post-stack-switch callback")
    pivots = [i for i, line in enumerate(compact)
              if re.fullmatch(r"movrsp,(?:qwordptr)?\[(?:rsi|r12)\+0xc0\]", line)]
    if len(pivots) != 2:
        raise ValueError("expected initial stack pivot and saved-frame reload")
    pivot, reload = pivots
    if not cli < capture < pivot < callback < reload:
        raise ValueError("outgoing release must follow RSP pivot and precede frame reload")
    alignment = [i for i, line in enumerate(compact)
                 if line in ("andrsp,-0x10", "andrsp,0xfffffffffffffff0", "subrsp,0x8")]
    if not any(pivot < i < callback for i in alignment):
        raise ValueError("callback stack lacks downward SysV alignment")
    pkru = one(lambda x: x == "wrpkru", "PKRU restore")
    if not reload < pkru:
        raise ValueError("callback must execute before narrowing PKRU")
    if any(x.startswith("call") for x in compact[pkru + 1:]):
        raise ValueError("call after narrowing PKRU")
    pops = [x for x in compact if x.startswith("pop")]
    expected = ["pop" + name for name in
                ("rax", "rbx", "rcx", "rdx", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15")]
    if pops != expected:
        raise ValueError("saved GPR restore layout changed")
    iret = one(lambda x: x == "iretq", "IRETQ")
    if compact[iret - 1] != "addrsp,0x8":
        raise ValueError("IRETQ must discard exactly the dummy error qword")
    if not pkru < one(lambda x: x == "swapgs", "SWAPGS") < iret:
        raise ValueError("GS/PKRU restore ordering changed")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kernel", type=Path, default=Path("target/x86_64-sex/release/sex-kernel"))
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    objdump = shutil.which("llvm-objdump") or shutil.which("objdump")
    if objdump is None:
        parser.error("llvm-objdump or objdump required")
    result = subprocess.run([objdump, "-d", "-C", "-M", "intel", "--no-show-raw-insn", str(args.kernel)],
                            capture_output=True, text=True)
    if result.returncode != 0:
        print(result.stderr, file=sys.stderr)
        return 1
    try:
        instructions = switch_instructions(result.stdout)
        validate(instructions)
    except ValueError as exc:
        print(f"[scheduler.switch.compiled] FAIL {exc}", file=sys.stderr)
        return 1
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text("\n".join(instructions) + "\n")
    print(f"[scheduler.switch.compiled] PASS kernel_sha256={hashlib.sha256(args.kernel.read_bytes()).hexdigest()}")
    print("scope=compiled_instruction_order_not_full_SMP_execution")
    return 0


if __name__ == "__main__":
    sys.exit(main())
