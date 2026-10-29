#!/usr/bin/env python3
"""Negative fixtures for the compiled timer PKRU boundary checker."""
import importlib.util
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location("timer_checker", Path(__file__).parents[1] / "check_timer_pkru.py")
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


def valid():
    registers = ("rax", "rbx", "rcx", "rdx", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15")
    return [
        "swapgs", "cmp byte ptr [rip + 0x20], 0x0 # <PKU_ENABLED>", "je 0x40",
        "xor ecx, ecx", "rdpkru", "mov qword ptr [rsp + 0x78], rax",
        "xor eax, eax", "xor ecx, ecx", "xor edx, edx", "wrpkru",
        "sub rsp, 0x8", "call 0x80 <timer_interrupt_handler>", "add rsp, 0x8",
        "cmp byte ptr [rip + 0x20], 0x0 # <PKU_ENABLED>", "je 0x90",
        "mov eax, dword ptr [rsp + 0x78]", "xor ecx, ecx", "xor edx, edx", "wrpkru",
        "swapgs",
    ] + ["pop " + register for register in registers] + ["add rsp, 0x8", "iretq"]


class TimerPkruChecker(unittest.TestCase):
    def test_valid_boundary(self):
        CHECKER.validate(valid())

    def rejects_replacement(self, old, new):
        code = valid()
        code[code.index(old)] = new
        with self.assertRaises(ValueError):
            CHECKER.validate(code)

    def test_missing_entry_read(self):
        self.rejects_replacement("rdpkru", "nop")

    def test_missing_restore(self):
        code = valid()
        code[18] = "nop"
        with self.assertRaises(ValueError):
            CHECKER.validate(code)

    def test_wrong_saved_slot(self):
        self.rejects_replacement("mov qword ptr [rsp + 0x78], rax", "mov qword ptr [rsp + 0x80], rax")

    def test_wrong_restore_slot(self):
        self.rejects_replacement("mov eax, dword ptr [rsp + 0x78]", "mov eax, dword ptr [rsp + 0x80]")

    def test_missing_feature_guard(self):
        self.rejects_replacement("je 0x90", "nop")

    def test_shifted_frame(self):
        code = valid()
        code[-2] = "add rsp, 0x10"
        with self.assertRaises(ValueError):
            CHECKER.validate(code)

    def test_call_after_restore(self):
        code = valid()
        code.insert(-1, "call 0x100 <unsafe_after_pkru>")
        with self.assertRaises(ValueError):
            CHECKER.validate(code)


if __name__ == "__main__":
    unittest.main()
