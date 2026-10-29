#!/usr/bin/env python3
"""Check the sealed kernel's timer PKRU save/return boundary without building."""
import argparse
import hashlib
from pathlib import Path
import re
import shutil
import subprocess
import sys


def timer_instructions(disassembly):
    functions = []
    active = None
    for line in disassembly.splitlines():
        header = re.match(r"^\s*[0-9a-f]+ <(.+)>:\s*$", line)
        if header:
            active = [] if header[1].endswith("::timer_interrupt_stub") else None
            if active is not None:
                functions.append(active)
            continue
        instruction = re.match(r"^\s*[0-9a-f]+:\s+(.+?)\s*$", line)
        if active is not None and instruction:
            active.append(re.sub(r"\s+", " ", instruction[1].lower()))
    if len(functions) != 1 or not functions[0]:
        raise ValueError("expected one timer_interrupt_stub")
    return functions[0]


def validate(instructions):
    code = [line.lower().replace(" ", "") for line in instructions]

    def one(predicate, label):
        matches = [i for i, item in enumerate(code) if predicate(item)]
        if len(matches) != 1:
            raise ValueError(f"expected one {label}; found {len(matches)}")
        return matches[0]

    read = one(lambda x: x == "rdpkru", "entry PKRU read")
    save = one(lambda x: x in ("movqwordptr[rsp+0x78],rax", "mov[rsp+0x78],rax"),
               "PKRU save in existing dummy qword")
    load = one(lambda x: x in ("moveax,dwordptr[rsp+0x78]", "moveax,[rsp+0x78]"),
               "PKRU reload from existing dummy qword")
    call = one(lambda x: x.startswith("call") and "<timer_interrupt_handler>" in x,
               "timer handler call")
    writes = [i for i, item in enumerate(code) if item == "wrpkru"]
    if len(writes) != 2:
        raise ValueError("expected kernel-open and return-restore WRPKRU")
    opening, restore = writes
    if not read < save < opening < call < load < restore:
        raise ValueError("entry save and return restore order violated")
    if code[read - 1] != "xorecx,ecx":
        raise ValueError("RDPKRU requires ECX=0")
    if code[opening - 3:opening] != ["xoreax,eax", "xorecx,ecx", "xoredx,edx"]:
        raise ValueError("kernel entry must open PKRU with ECX=EDX=0")
    if code[restore - 2:restore] != ["xorecx,ecx", "xoredx,edx"] or restore != load + 3:
        raise ValueError("return restore requires saved EAX and ECX=EDX=0")
    guards = [i for i, item in enumerate(code) if item.startswith("cmp") and "<pku_enabled>" in item]
    if len(guards) != 2 or not guards[0] < read < call < guards[1] < load:
        raise ValueError("entry and return each require PKU capability guard")
    if any(not code[i + 1].startswith("je") for i in guards):
        raise ValueError("disabled PKU must skip each PKRU operation block")
    pops = [item for item in code if item.startswith("pop")]
    registers = ("rax", "rbx", "rcx", "rdx", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15")
    if pops != ["pop" + register for register in registers]:
        raise ValueError("saved GPR restore layout changed")
    iret = one(lambda x: x == "iretq", "IRETQ")
    swaps = [i for i, item in enumerate(code) if item == "swapgs"]
    if len(swaps) != 2 or not restore < swaps[1] < code.index("poprax") < iret:
        raise ValueError("PKRU restore must precede user GS/GPR restoration")
    if code[iret - 1] != "addrsp,0x8":
        raise ValueError("return must discard the existing single dummy qword")
    if any(item.startswith("call") for item in code[restore + 1:]):
        raise ValueError("call after restoring restricted PKRU")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kernel", type=Path, default=Path("target/x86_64-sex/release/sex-kernel"))
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    objdump = shutil.which("llvm-objdump") or shutil.which("objdump")
    if not objdump:
        parser.error("llvm-objdump or objdump required")
    result = subprocess.run([objdump, "-d", "-C", "-M", "intel", "--no-show-raw-insn", str(args.kernel)],
                            capture_output=True, text=True)
    if result.returncode:
        print(result.stderr, file=sys.stderr)
        return 1
    try:
        instructions = timer_instructions(result.stdout)
        validate(instructions)
    except ValueError as error:
        print(f"[timer.pkru.compiled] FAIL {error}", file=sys.stderr)
        return 1
    if args.evidence:
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        args.evidence.write_text("\n".join(instructions) + "\n")
    print(f"[timer.pkru.compiled] PASS kernel_sha256={hashlib.sha256(args.kernel.read_bytes()).hexdigest()}")
    print("scope=compiled_timer_return_not_native_WRPKRU_containment")
    return 0


if __name__ == "__main__":
    sys.exit(main())
